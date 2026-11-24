// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host tests' model of the RK3588 HDMI0 display path (tests only), at
//! the real addresses: the CRU and the PMU1 CRU, the PMU's power domains,
//! sys_grf, vop_grf, vo1_grf, the HDPTX PHY0 and its GRF, the DW HDMI QP
//! TX0, VOP2 and its two `vop_mmu` register sets. It is the union of the
//! single-block models of `soc::rk3588_display` (D4), `ip::hdptx` (D5),
//! `ip::dw_hdmi_qp` (D6), `ip::vop2` and `ip::rk_iommu` (D1), with what
//! links the blocks: the PHY's PLL is VP0's pixel clock through the
//! DCLK_VOP0 mux, and the controller's clock monitor sees the link and the
//! video clock (driver/docs/nanopc-t6-display.md, sections 1 and 7 row D7).
//!
//! As [`crate::model_hdmi`] does, it restates every offset and bit from the
//! design note and the sources it cites, never from the code under test, so
//! a wrong constant cannot agree with itself. It records every write in
//! order, answers the polls, takes injected faults ([`Faults`]) and refuses
//! an access the silicon would not take, the way the part models do:
//!
//! - CRU / PMU (D4's model): control words take the write mask in 31:16;
//!   the PMU status words follow the control words `lag` reads after the
//!   last write; a domain switched with one of its switching gates closed
//!   is refused; a domain seen off loses its blocks' registers.
//! - A block access with its domain not usable, a gate closed or a reset
//!   asserted is refused (logged, a write dropped, a read 0). vo1_grf
//!   needs `PCLK_VO1GRF` (59/12, clk-rk3588.c:2436); the other GRFs are not
//!   gated here (their clocks are not in the design note).
//! - Asserting a block's reset returns its registers to their reset values.
//!   For the PHY (no power domain) that is `SRST_P_HDPTX0`: an assumption
//!   of this model (Linux pulses it before every table write, hdptx.c fact
//!   4 of `ip::hdptx`; what it clears is not documented).
//! - PHY (D5's model): accesses only inside Linux's ranges; `STATUS`
//!   `PHY_CLK_RDY` once `CON0` has PLL, bias and band gap on with init and
//!   cmn released and the reference clock (PMU1 CRU 4/7) open;
//!   `PHY_RDY` and `PLL_LOCK_DONE` once lane is released too.
//! - Controller (D6's model): request words never read, the I2C master
//!   never touched; 0x10, 0x804 and 0x80c never touched (their reads end
//!   in a bus error on the T6: the D0 dump's record, 2026-11-12); `CORE_ID` reads [`Rk3588Display::core_id`];
//!   `CMU_STATUS` reads the locked value 0x15 while the PHY is locked
//!   (link clock) and VP0 runs (video clock); otherwise `IPI_CLK_OFF`
//!   (bit 1) and `VIDQPCLK_OFF` (bit 3) while VP0 does not run and
//!   `LINKQPCLK_OFF` (bit 5) while the PHY is not locked (qp.h:47-49; which
//!   bits the silicon sets for which missing clock is an assumption of this
//!   model: the board read 0x00000200, no clock reported off, right after
//!   VP0 started, issue #11856). `HDCP2LOGIC_CONFIG0` (0x8e0) and
//!   `LINK_CONFIG0` (0x968), the words the controller's enable writes,
//!   answer only while VP0 runs: on the T6 an access to 0x8e0 before VP0
//!   started ended in a bus error (issue #11856); 0x968 is taken to behave
//!   the same (not measured: the enable stopped at 0x8e0).
//! - VOP2 (D1's model): ports run frames while STANDBY is clear and their
//!   pixel clock runs. VP0-VP2's come from the CRU (clk-rk3588.c:2091-2111):
//!   `DCLK_VOPn` gate 52/13, 53/0, 53/1 open (and for VP0 `SRST_D_VOP0`
//!   52/13 released), its mux (CLKSEL_CON(112) bits 8:7, 10:9, 12:11) on
//!   either `dclk_vopn_src` (0; its gate 52/10, 52/11, 52/12 open, its PLL
//!   taken as running) or the PHY0's pixel clock (1; the PHY's clock
//!   ready); a mux on PHY1 (2) gives no clock here. VP3's pixel clock is
//!   [`Rk3588Display::set_vp_clock`]. At a port's frame start its pending
//!   `REG_CFG_DONE` is taken: the written port select comes into force and
//!   the window enables written become the ones that fetch (review R3-S2:
//!   the model shadows the enables; the other window registers read as
//!   written). A fetching window reads through the IOMMU while paging is on;
//!   one whose address is in [`Rk3588Display::iova_windows`] (an IOVA of
//!   the system before us) and fetches with paging off raises a bus error.
//! - vop_mmu (D1's model): a stall becomes active only while paging, the
//!   commands act at once.

use crate::env::{Env, Level};
use crate::mmio::Mmio;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};

// Bases (design note section 1).
pub const CRU: usize = 0xfd7c_0000;
pub const PMU1CRU: usize = 0xfd7f_0000;
pub const PMU: usize = 0xfd8d_8000;
pub const SYS_GRF: usize = 0xfd58_c000;
pub const VOP_GRF: usize = 0xfd5a_4000;
pub const VO1_GRF: usize = 0xfd5a_8000;
pub const PHY_GRF: usize = 0xfd5e_0000;
pub const VOP: usize = 0xfdd9_0000;
pub const MMU: [usize; 2] = [0xfdd9_7e00, 0xfdd9_7f00];
pub const HDMI: usize = 0xfde8_0000;
pub const PHY: usize = 0xfed6_0000;

// PMU (pm-domains.c:1493-1506, as D4's model).
const PWR: usize = PMU + 0x14c;
const REQ: usize = PMU + 0x10c;
const ACK: usize = PMU + 0x118;
const IDLE: usize = PMU + 0x120;
const STATUS: usize = PMU + 0x180;
const MEM_PWR: usize = PMU + 0x1a0;
const CHAIN: usize = PMU + 0x1f0;
const MEM_ST: usize = PMU + 0x1f8;
const REPAIR: usize = PMU + 0x290;

pub const fn gate_addr(part: usize, con: usize) -> usize {
    CRU + part + 0x800 + 4 * con
}
pub const fn rst_addr(part: usize, con: usize) -> usize {
    CRU + part + 0xa00 + 4 * con
}
pub const fn sel_addr(part: usize, con: usize) -> usize {
    CRU + part + 0x300 + 4 * con
}
const P1: usize = 0x3_0000;

/// A modelled domain (D4's model): name, power word/bit, memory bit, repair
/// bit, request word/bits, idle bits, the gates it needs while it switches.
struct Dom {
    name: &'static str,
    pwr: (usize, u32),
    mem: u32,
    repair: u32,
    req: (usize, u32),
    idle: u32,
    switch_gates: &'static [(usize, u32)],
}
const DOMS: [Dom; 3] = [
    Dom {
        name: "vop",
        pwr: (PWR + 4, 1 << 1),
        mem: 1 << 25,
        repair: 1 << 16,
        req: (REQ, 3 << 13),
        idle: 3 << 13,
        switch_gates: &[(52, 0), (52, 2), (52, 3), (52, 9)],
    },
    Dom {
        name: "vo0",
        pwr: (PWR + 4, 1 << 2),
        mem: 1 << 26,
        repair: 1 << 17,
        req: (REQ, 1 << 15),
        idle: 1 << 15,
        switch_gates: &[(55, 0), (55, 3), (55, 4), (55, 2), (55, 13), (55, 12), (52, 2)],
    },
    Dom {
        name: "vo1",
        pwr: (PWR + 4, 1 << 3),
        mem: 1 << 27,
        repair: 1 << 18,
        req: (REQ + 4, 1 << 0),
        idle: 1 << 16,
        switch_gates: &[(59, 4), (59, 5), (59, 3), (60, 6), (60, 5), (59, 1), (74, 2)],
    },
];

/// Block names, as [`Rk3588Display::block_of`] gives them.
pub const B_CRU: &str = "cru";
pub const B_PMU: &str = "pmu";
pub const B_SYS_GRF: &str = "sys_grf";
pub const B_VOP_GRF: &str = "vop_grf";
pub const B_VO1_GRF: &str = "vo1_grf";
pub const B_PHY_GRF: &str = "hdptxphy0_grf";
pub const B_VOP: &str = "vop";
pub const B_MMU: &str = "vop_mmu";
pub const B_HDMI: &str = "hdmi-tx0";
pub const B_PHY: &str = "hdptx0";

/// A modelled block: window, domain, gates (part, CLKGATE_CON, bit) and
/// resets (part, SOFTRST_CON, bit) its registers need (D4's model, plus
/// vo1_grf and vop_mmu).
struct Blk {
    name: &'static str,
    base: usize,
    size: usize,
    dom: Option<&'static str>,
    gates: &'static [(usize, usize, u32)],
    resets: &'static [(usize, usize, u32)],
}
const VOP_GATES: &[(usize, usize, u32)] = &[(0, 52, 8), (0, 52, 9), (0, 52, 3)];
const VOP_RESETS: &[(usize, usize, u32)] = &[(0, 52, 8), (0, 52, 9)];
const BLKS: [Blk; 10] = [
    Blk { name: B_VOP, base: VOP, size: 0x4200, dom: Some("vop"), gates: VOP_GATES, resets: VOP_RESETS },
    Blk { name: B_MMU, base: 0xfdd9_7e00, size: 0x200, dom: Some("vop"), gates: VOP_GATES, resets: VOP_RESETS },
    Blk {
        name: B_HDMI,
        base: HDMI,
        size: 0x2_0000,
        dom: Some("vo1"),
        gates: &[(0, 60, 11), (0, 59, 9), (0, 61, 0), (0, 73, 12)],
        resets: &[(0, 60, 11), (0, 61, 0), (0, 73, 12)],
    },
    Blk { name: B_PHY, base: PHY, size: 0x2000, dom: None, gates: &[(0, 72, 5)], resets: &[(0, 72, 5)] },
    Blk { name: B_VO1_GRF, base: VO1_GRF, size: 0x100, dom: None, gates: &[(0, 59, 12)], resets: &[] },
    Blk { name: B_SYS_GRF, base: SYS_GRF, size: 0x1000, dom: None, gates: &[], resets: &[] },
    Blk { name: B_VOP_GRF, base: VOP_GRF, size: 0x100, dom: None, gates: &[], resets: &[] },
    Blk { name: B_PHY_GRF, base: PHY_GRF, size: 0x100, dom: None, gates: &[], resets: &[] },
    Blk { name: B_CRU, base: CRU, size: 0x3_1000, dom: None, gates: &[], resets: &[] },
    Blk { name: B_PMU, base: PMU, size: 0x1000, dom: None, gates: &[], resets: &[] },
];

/// PHY register ranges Linux reads and writes (hdptx.c:776-799).
const PHY_RANGES: [(usize, usize); 7] =
    [(0x000, 0x0a7), (0x100, 0x129), (0x200, 0x229), (0x300, 0x32d), (0x400, 0x42d), (0x500, 0x52d), (0x600, 0x62d)];

// VOP2 offsets (vop2.h, as the design note and D1's facts).
const CFG_DONE: usize = 0x000;
const VERSION: usize = 0x004;
const PD_CTRL: usize = 0x034;
const SYS0_INT_EN: usize = 0x080;
const SYS0_INT_CLR: usize = 0x084;
const SYS0_INT_STATUS: usize = 0x088;
const SYS1_INT_STATUS: usize = 0x098;
const LAYER_SEL: usize = 0x604;
const PORT_SEL: usize = 0x608;
pub const VERSION_RK3588: u32 = 0x4017_6786;
const fn dsp_ctrl(vp: usize) -> usize {
    0xc00 + 0x100 * vp
}
const fn int_clr(vp: usize) -> usize {
    0x0a4 + 0x10 * vp
}
const fn int_raw(vp: usize) -> usize {
    0x0ac + 0x10 * vp
}
/// (enable register, its bit) of each window: Cluster0-3 (`WIN_CTRL0`),
/// Esmart0-3 (`REGION0_CTRL`); bit n Cluster n, bit 4 + n Esmart n.
const fn win_en(w: usize) -> usize {
    if w < 4 {
        0x1000 + 0x200 * w
    } else {
        0x1810 + 0x200 * (w - 4)
    }
}
const fn win_mst(w: usize) -> Option<usize> {
    if w < 4 {
        None
    } else {
        Some(0x1814 + 0x200 * (w - 4))
    }
}
/// The window's port field in `OVL_PORT_SEL`: Cluster n at 17:16 + 2n,
/// Esmart n at 25:24 + 2n.
const fn win_port(sel: u32, w: usize) -> usize {
    let shift = if w < 4 { 16 + 2 * w } else { 24 + 2 * (w - 4) };
    ((sel >> shift) & 3) as usize
}

/// Injected faults.
#[derive(Clone, Copy, Debug, Default)]
pub struct Faults {
    /// This domain never reads powered.
    pub dead: Option<&'static str>,
    /// vop_grf ignores every write.
    pub grf_stuck: bool,
    /// The PHY never reports `PHY_CLK_RDY`.
    pub phy_no_clk: bool,
    /// The PHY never reports `PHY_RDY` and `PLL_LOCK_DONE`.
    pub phy_no_lock: bool,
    /// CLKSEL_CON(112) ignores writes (DCLK_VOP0's mux stays where it is).
    pub dclk_mux_stuck: bool,
    /// VOP2's `VERSION_INFO` reads this instead.
    pub vop_version: Option<u32>,
    /// A fetching window underflows every frame.
    pub underflow: bool,
    /// A fetching window's AXI read fails every frame (SYS0 bus error).
    pub bus_error: bool,
    /// A running port ignores STANDBY: it goes on running frames and raises
    /// no DSP_HOLD_VALID.
    pub standby_ignored: bool,
    /// Esmart0's `REGION0_YRGB_MST` ignores writes.
    pub mst_stuck: bool,
    /// The controller's clock monitor never reads locked.
    pub cmu_unlocked: bool,
    /// The controller's clock monitor reports the video clock off
    /// (`VIDQPCLK_OFF`) also while VP0 runs.
    pub cmu_video_off: bool,
    /// The PHY's lanes report not ready (`PHY_RDY` 0) once VP0 runs; the
    /// PLL stays locked.
    pub phy_rdy_lost: bool,
    /// The ports' `REG_CFG_DONE` bits read 1 after the update is taken (the
    /// other answer to the design note's U8).
    pub cfg_done_sticky: bool,
    /// These ports (bit n = VPn) time no frame although every clock input
    /// the CRU and the PHY show reads as running.
    pub no_frames: u32,
    /// SRST_A_VOP (52/9) ignores a release: the VOP stays in reset.
    pub vop_reset_stuck: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct MmuSet {
    status: u32,
    dte: u32,
    int_mask: u32,
}

/// The VOP's state beyond its plain registers.
#[derive(Default)]
struct VopState {
    /// Registers as written (absent: the reset value).
    regs: BTreeMap<usize, u32>,
    port_sel_written: Option<u32>,
    pending: [bool; 4],
    raw: [u32; 4],
    sys_raw: [u32; 2],
    stopping: [bool; 4],
    /// The port was seen active (STANDBY clear or stopping) without its
    /// pixel clock: the frame it was in is not over.
    unclocked_seen: [bool; 4],
    next_frame_us: [u64; 4],
    /// The windows fetching ([`win_en`] numbering) and their addresses.
    fetching: u32,
    fetch_addr: [u32; 8],
    mmu: [MmuSet; 2],
}

/// The combined chip; see the module text.
pub struct Rk3588Display {
    regs: RefCell<HashMap<usize, u32>>,
    blocks: RefCell<BTreeMap<usize, u32>>,
    vop: RefCell<VopState>,
    writes: RefCell<Vec<(usize, u32)>>,
    refused: RefCell<Vec<String>>,
    reads_since: RefCell<u32>,
    now_us: RefCell<u64>,
    lines: RefCell<Vec<(Level, String)>>,
    /// PMU status lag in reads (D4's model).
    pub lag: u32,
    pub faults: Faults,
    /// `CORE_ID` of the controller (a test value; the board's is the dump's).
    pub core_id: u32,
    /// The HPD level in sys_grf `SOC_STATUS1` bit 16.
    pub hpd: bool,
    /// VP3 has a pixel clock (VP0-VP2's follow the CRU).
    vp_clock: Cell<[bool; 4]>,
    /// Addresses that are IOVAs of the system before us.
    iova_windows: RefCell<Vec<u32>>,
    /// A frame, µs.
    pub frame_us: u64,
}

/// The controller identity the tests use (not the board's).
pub const TEST_CORE_ID: u32 = 0x1234_5678;

impl Rk3588Display {
    /// Everything gated and in reset, the display domains off; the CPLL at
    /// 1500 MHz, GPLL 1188, NPLL 850, all in normal mode (D4's `cold`).
    pub fn cold() -> Rk3588Display {
        Self::with_faults(Faults::default())
    }

    pub fn with_faults(faults: Faults) -> Rk3588Display {
        let mut r = HashMap::new();
        for con in 0..78 {
            r.insert(gate_addr(0, con), 0xffff);
            r.insert(rst_addr(0, con), 0xffff);
        }
        for con in 0..180 {
            r.insert(sel_addr(0, con), 0);
        }
        for con in 0..6 {
            r.insert(gate_addr(P1, con), 0xffff);
            r.insert(rst_addr(P1, con), 0xffff);
        }
        r.insert(sel_addr(P1, 14), 0);
        for (a, v) in [
            (CRU + 0x280, 0x545),
            (CRU + 4 * 104, 0xfa),
            (CRU + 4 * 105, 0x42),
            (CRU + 4 * 106, 0),
            (CRU + 4 * 112, 0xc6),
            (CRU + 4 * 113, 0x42),
            (CRU + 4 * 114, 0),
            (CRU + 4 * 120, 0x1a9),
            (CRU + 4 * 121, 0x83),
            (CRU + 4 * 122, 0),
            (PWR, 0xffff),
            (PWR + 4, 0xffff),
            (REQ, 0),
            (REQ + 4, 0),
            (ACK, 0),
            (IDLE, 0),
            (STATUS, 0),
            (MEM_PWR, 0),
            (MEM_PWR + 4, 0),
            (CHAIN, 0),
            (MEM_ST, 0),
            (REPAIR, 0),
        ] {
            r.insert(a, v);
        }
        let m = Rk3588Display {
            regs: RefCell::new(r),
            blocks: RefCell::new(BTreeMap::new()),
            vop: RefCell::new(VopState::default()),
            writes: RefCell::new(Vec::new()),
            refused: RefCell::new(Vec::new()),
            reads_since: RefCell::new(0),
            now_us: RefCell::new(0),
            lines: RefCell::new(Vec::new()),
            lag: 2,
            faults,
            core_id: TEST_CORE_ID,
            hpd: false,
            vp_clock: Cell::new([false; 4]),
            iova_windows: RefCell::new(Vec::new()),
            frame_us: 16_667,
        };
        m.settle();
        m
    }

    /// The block an address is in.
    pub fn block_of(addr: usize) -> Option<&'static str> {
        Self::blk(addr).map(|b| b.name)
    }

    fn blk(addr: usize) -> Option<&'static Blk> {
        BLKS.iter().find(|b| (b.base..b.base + b.size).contains(&addr))
    }

    fn get(&self, a: usize) -> u32 {
        *self.regs.borrow().get(&a).unwrap_or(&0)
    }

    /// The register image at `addr`, as a read would give it but without
    /// logging, settling or refusing: what the golden test compares.
    pub fn peek(&self, addr: usize) -> u32 {
        match Self::block_of(addr) {
            Some(B_VOP) | Some(B_MMU) => self.vop_read(addr),
            Some(B_HDMI) => self.hdmi_read(addr - HDMI),
            Some(B_PHY) => *self.blocks.borrow().get(&addr).unwrap_or(&0),
            Some(B_PHY_GRF) if addr - PHY_GRF == 0x80 => self.phy_status(),
            Some(B_SYS_GRF) if addr - SYS_GRF == 0x384 => self.sys_status1(),
            _ => self.get(addr),
        }
    }

    /// Set a register without logging (a test's preset state).
    pub fn poke(&self, addr: usize, v: u32) {
        match Self::block_of(addr) {
            Some(B_VOP) => {
                self.vop.borrow_mut().regs.insert(addr - VOP, v);
            }
            Some(B_HDMI) | Some(B_PHY) => {
                self.blocks.borrow_mut().insert(addr, v);
            }
            _ => {
                self.regs.borrow_mut().insert(addr, v);
            }
        }
    }

    pub fn writes(&self) -> Vec<(usize, u32)> {
        self.writes.borrow().clone()
    }
    /// The index of the first write into block `name`, if any.
    pub fn first_write(&self, name: &str) -> Option<usize> {
        self.writes.borrow().iter().position(|(a, _)| Self::block_of(*a) == Some(name))
    }
    pub fn wrote_to(&self, name: &str) -> bool {
        self.first_write(name).is_some()
    }
    pub fn refused(&self) -> Vec<String> {
        self.refused.borrow().clone()
    }
    pub fn clear_log(&self) {
        self.writes.borrow_mut().clear();
        self.refused.borrow_mut().clear();
    }
    pub fn lines(&self) -> Vec<(Level, String)> {
        self.lines.borrow().clone()
    }
    /// Forget the log lines (the writes stay: [`Self::clear_log`]).
    pub fn clear_lines(&self) {
        self.lines.borrow_mut().clear();
    }
    pub fn now_us(&self) -> u64 {
        *self.now_us.borrow()
    }

    fn powered(&self, d: &Dom) -> bool {
        self.get(REPAIR) & d.repair != 0
    }

    pub fn usable(&self, name: &str) -> bool {
        let d = DOMS.iter().find(|d| d.name == name).unwrap();
        self.powered(d) && self.get(d.req.0) & d.req.1 == 0 && (self.get(ACK) | self.get(IDLE)) & d.idle == 0
    }

    fn gate_open(&self, part: usize, con: usize, bit: u32) -> bool {
        self.get(gate_addr(part, con)) & (1 << bit) == 0
    }
    fn rst_released(&self, part: usize, con: usize, bit: u32) -> bool {
        self.get(rst_addr(part, con)) & (1 << bit) == 0
    }

    /// Let the PMU status words follow the control words (D4's model); a
    /// domain off loses its blocks.
    fn settle(&self) {
        let mut off = Vec::new();
        {
            let mut r = self.regs.borrow_mut();
            for d in &DOMS {
                let pwr_on = r[&d.pwr.0] & d.pwr.1 == 0;
                let on = pwr_on && self.faults.dead != Some(d.name);
                let set = |r: &mut HashMap<usize, u32>, a: usize, bits: u32, v: bool| {
                    let w = r.get_mut(&a).unwrap();
                    *w = if v { *w | bits } else { *w & !bits };
                };
                set(&mut r, REPAIR, d.repair, on);
                set(&mut r, CHAIN, d.mem, pwr_on);
                let mem_off = r[&(MEM_PWR + d.pwr.0 - PWR)] & d.pwr.1 != 0;
                set(&mut r, MEM_ST, d.mem, mem_off);
                let idle = !on || r[&d.req.0] & d.req.1 != 0;
                set(&mut r, ACK, d.idle, idle);
                set(&mut r, IDLE, d.idle, idle);
                if !on {
                    off.push(d.name);
                }
            }
        }
        for name in off {
            for b in BLKS.iter().filter(|b| b.dom == Some(name)) {
                self.reset_block(b);
            }
        }
    }

    fn reset_block(&self, b: &Blk) {
        match b.name {
            B_VOP | B_MMU => *self.vop.borrow_mut() = VopState::default(),
            _ => self.blocks.borrow_mut().retain(|a, _| !(b.base..b.base + b.size).contains(a)),
        }
    }

    fn refusal(&self, b: &Blk) -> Option<String> {
        if let Some(d) = b.dom {
            if !self.usable(d) {
                return Some(format!("{}: domain {d} not usable", b.name));
            }
        }
        for (p, con, bit) in b.gates {
            if !self.gate_open(*p, *con, *bit) {
                return Some(format!("{}: gate {con}/{bit} closed", b.name));
            }
        }
        for (p, con, bit) in b.resets {
            if !self.rst_released(*p, *con, *bit) {
                return Some(format!("{}: reset {con}/{bit} asserted", b.name));
            }
        }
        None
    }

    /// A PMU write switching a domain whose switching gates are not open.
    fn check_switch(&self, addr: usize, val: u32) {
        for d in &DOMS {
            let hits = (addr == d.pwr.0 && (val >> 16) & d.pwr.1 != 0)
                || (addr == d.req.0 && (val >> 16) & d.req.1 != 0)
                || (addr == MEM_PWR + d.pwr.0 - PWR && (val >> 16) & d.pwr.1 != 0);
            if !hits {
                continue;
            }
            for (con, bit) in d.switch_gates {
                if !self.gate_open(0, *con, *bit) {
                    self.refused.borrow_mut().push(format!("{}: switched with gate {con}/{bit} closed", d.name));
                }
            }
        }
    }

    // ---- the PHY ----

    fn phy_con0(&self) -> u32 {
        self.get(PHY_GRF)
    }

    /// hdptxphy0_grf `STATUS` from the state (module text).
    fn phy_status(&self) -> u32 {
        let on = self.phy_con0() & 0xe0 == 0xe0;
        let init = self.rst_released(P1, 3, 11);
        let cmn = self.rst_released(P1, 3, 12);
        let lane = self.rst_released(P1, 3, 13);
        let refclk = self.gate_open(P1, 4, 7);
        let mut st = 0;
        if on && init && cmn && refclk && !self.faults.phy_no_clk {
            st |= 1 << 2;
            if lane && !self.faults.phy_no_lock {
                st |= 1 << 3;
                // (no borrow: called from inside the VOP's own update, where
                // only the clock bit is read)
                let vp0 = self.vop.try_borrow().map(|v| Self::active(&v, 0)).unwrap_or(false);
                if !(self.faults.phy_rdy_lost && vp0 && self.gate_open(0, 52, 13)) {
                    st |= 1 << 1;
                }
            }
        }
        st
    }

    fn phy_clock(&self) -> bool {
        self.phy_status() & (1 << 2) != 0
    }
    fn phy_check(&self, addr: usize) -> bool {
        let off = addr - PHY;
        let n = off / 4;
        if !off.is_multiple_of(4) || !PHY_RANGES.iter().any(|&(lo, hi)| (lo..=hi).contains(&n)) {
            self.refused.borrow_mut().push(format!("PHY register {n:#x} outside Linux's ranges"));
            return false;
        }
        true
    }

    // ---- the controller ----

    fn hdmi_read(&self, off: usize) -> u32 {
        match off {
            0x0 => self.core_id,
            0xb0 => {
                let link = self.phy_status() & (1 << 3) != 0;
                let video = self.vp_running(&self.vop.borrow(), 0);
                if self.faults.cmu_video_off {
                    1 << 3
                } else if link && video {
                    if self.faults.cmu_unlocked {
                        0
                    } else {
                        0x15
                    }
                } else {
                    (if video { 0 } else { 1 << 1 | 1 << 3 }) | if link { 0 } else { 1 << 5 }
                }
            }
            _ => *self.blocks.borrow().get(&(HDMI + off)).unwrap_or(&0),
        }
    }

    fn hdmi_check(&self, off: usize, read: bool) -> bool {
        let bad = if !off.is_multiple_of(4) || off > 0x482c {
            Some("outside the controller's registers")
        } else if (0xe0..=0x118).contains(&off) {
            Some("I2C master (D10)")
        } else if read && [0x40, 0x974, 0xab0].contains(&off) {
            Some("a request word read")
        } else if [0x10, 0x804, 0x80c].contains(&off) {
            Some("a word this controller does not answer (a bus error on the T6, D0 record)")
        } else if [0x8e0, 0x968].contains(&off) && !self.vp_running(&self.vop.borrow(), 0) {
            Some("an enable word without the video clock: VP0 does not run (a bus error on the T6, #11856)")
        } else {
            None
        };
        if let Some(why) = bad {
            self.refused.borrow_mut().push(format!("controller {off:#x}: {why}"));
        }
        bad.is_none()
    }

    fn sys_status1(&self) -> u32 {
        if self.hpd {
            1 << 16
        } else {
            0
        }
    }

    // ---- VOP2 ----

    /// The pixel clock of port `p` runs (module text).
    fn vp_clocked(&self, p: usize) -> bool {
        if p == 3 {
            return self.vp_clock.get()[p];
        }
        let (gate, src_gate) = [((52, 13), 10), ((53, 0), 11), ((53, 1), 12)][p];
        let mux = (self.get(sel_addr(0, 112)) >> (7 + 2 * p)) & 3;
        self.gate_open(0, gate.0, gate.1)
            && (p != 0 || self.rst_released(0, 52, 13))
            && match mux {
                0 => self.gate_open(0, 52, src_gate),
                1 => self.phy_clock(),
                _ => false,
            }
    }

    fn vop_reg(s: &VopState, off: usize) -> u32 {
        if let Some(v) = s.regs.get(&off) {
            return *v;
        }
        // the reset values D1's model assumes (not in any source): every
        // internal domain off, every port in standby, layer n = window n,
        // no layer on any port
        match off {
            PD_CTRL => 0xef,
            0xc00 | 0xd00 | 0xe00 | 0xf00 => 1 << 31,
            LAYER_SEL => 0x7654_3210,
            PORT_SEL => 0x8000_8888,
            _ => 0,
        }
    }

    /// STANDBY clear, or set but the frame not ended yet.
    fn active(s: &VopState, p: usize) -> bool {
        s.stopping[p] || Self::vop_reg(s, dsp_ctrl(p)) & 1 << 31 == 0
    }

    fn vp_running(&self, s: &VopState, p: usize) -> bool {
        Self::active(s, p) && self.vp_clocked(p)
    }

    /// The address a window fetches is wrong for the IOMMU's state: with
    /// paging off an IOVA of the system before us is read as a physical
    /// address; with paging on through the tables of the system before us,
    /// our physical buffer is read as an IOVA (not mapped there; an
    /// assumption of this model). Either is a bus error.
    fn bad_fetch(&self, s: &VopState, w: usize) -> bool {
        let iova = self.iova_windows.borrow().contains(&s.fetch_addr[w]);
        let paging = s.mmu[0].status & 1 != 0;
        if paging {
            !iova && w >= 4 && !self.iova_windows.borrow().is_empty()
        } else {
            iova
        }
    }

    /// Paging just went off: a window still fetching on a running port
    /// (its disable pending, or none written) reads its IOVA as a physical
    /// address for the rest of its frame.
    fn paging_went_off(&self) {
        let mut s = self.vop.borrow_mut();
        let sel = Self::vop_reg(&s, PORT_SEL);
        for w in 0..8 {
            let p = win_port(sel, w);
            if s.fetching & 1 << w != 0 && self.vp_running(&s, p) && self.bad_fetch(&s, w) {
                s.sys_raw[0] |= 1 << 1;
            }
        }
    }

    fn vop_read(&self, addr: usize) -> u32 {
        let s = self.vop.borrow();
        if addr >= MMU[0] {
            let i = usize::from(addr >= MMU[1]);
            let set = s.mmu[i];
            return match addr & 0xff {
                0x00 => set.dte,
                0x04 => set.status,
                0x1c => set.int_mask,
                // COMMAND: write-only commands; the D0 dump reads it as 0
                0x08 => 0,
                o => panic!("read of vop_mmu offset {o:#x} (rockchip-iommu.c:33-42 names 0x00, 0x04, 0x08, 0x1c)"),
            };
        }
        let off = addr - VOP;
        assert!(off < 0x2000, "VOP access at {off:#x}: the HDR and LUT registers from 0x2000 are never touched");
        let raw = (0..4).find(|p| int_raw(*p) == off);
        match off {
            VERSION => self.faults.vop_version.unwrap_or(VERSION_RK3588),
            CFG_DONE => {
                let pend = (0..4).fold(0, |a, p| a | u32::from(s.pending[p] || self.faults.cfg_done_sticky) << p);
                Self::vop_reg(&s, CFG_DONE) & 1 << 15 | pend
            }
            SYS0_INT_STATUS => s.sys_raw[0] & Self::vop_reg(&s, SYS0_INT_EN),
            SYS1_INT_STATUS => s.sys_raw[1] & Self::vop_reg(&s, SYS0_INT_EN + 0x10),
            _ if raw.is_some() => s.raw[raw.unwrap()],
            _ => Self::vop_reg(&s, off),
        }
    }

    fn vop_write(&self, addr: usize, val: u32) {
        let mut s = self.vop.borrow_mut();
        if addr >= MMU[0] {
            let paging = s.mmu.iter().any(|x| x.status & 1 != 0);
            let i = usize::from(addr >= MMU[1]);
            let set = &mut s.mmu[i];
            match (addr & 0xff, val) {
                (0x00, v) => set.dte = v,
                (0x1c, v) => set.int_mask = v,
                (0x08, 1) => set.status &= !1,
                (0x08, 2) if set.status & 1 != 0 => set.status |= 4,
                (0x08, 3) => set.status &= !4,
                (0x08, 2) => {}
                (o, v) => panic!("write {v:#x} to vop_mmu offset {o:#x}"),
            }
            if paging && s.mmu.iter().all(|x| x.status & 1 == 0) {
                drop(s);
                self.paging_went_off();
            }
            return;
        }
        let off = addr - VOP;
        assert!(off < 0x2000, "VOP access at {off:#x}: the HDR and LUT registers from 0x2000 are never touched");
        assert_ne!(off, VERSION, "VERSION_INFO is read only");
        let clr = (0..4).find(|p| int_clr(*p) == off);
        let ctrl = (0..4).find(|p| dsp_ctrl(*p) == off);
        if off == CFG_DONE {
            s.regs.insert(CFG_DONE, val & 1 << 15);
            for p in 0..4 {
                if val & 1 << p != 0 && val & 1 << (p + 16) != 0 {
                    s.pending[p] = true;
                }
            }
        } else if let Some(p) = clr {
            s.raw[p] &= !(val & (val >> 16) & 0xffff);
        } else if off == SYS0_INT_CLR || off == SYS0_INT_CLR + 0x10 {
            let i = usize::from(off != SYS0_INT_CLR);
            s.sys_raw[i] &= !(val & (val >> 16) & 0xffff);
        } else if off == SYS0_INT_EN || off == SYS0_INT_EN + 0x10 || (0..4).any(|p| off == 0x0a0 + 0x10 * p) {
            // interrupt enables take their write mask in 31:16 (vop2.c:746-747)
            let mask = val >> 16;
            let old = Self::vop_reg(&s, off);
            s.regs.insert(off, (old & !mask) | (val & mask & 0xffff));
        } else if off == 0x1814 && self.faults.mst_stuck {
        } else if off == PORT_SEL {
            s.port_sel_written = Some(val);
        } else if let Some(p) = ctrl {
            // STANDBY takes effect at the end of the frame (vop2 fact 16),
            // whether or not the pixel clock runs now: a port without it
            // ends its frame when the clock comes back (D1's model)
            let was_active = Self::active(&s, p);
            let was = self.vp_running(&s, p);
            if was && val & 1 << 31 != 0 && self.faults.standby_ignored {
                return;
            }
            s.regs.insert(off, val);
            if !was && self.vp_running(&s, p) {
                s.next_frame_us[p] = *self.now_us.borrow() + self.frame_us;
            }
            if was_active && val & 1 << 31 != 0 {
                s.stopping[p] = true;
            }
        } else {
            s.regs.insert(off, val);
        }
    }

    /// Let `us` pass: the ports run their frames.
    fn advance(&self, us: u64) {
        let end = *self.now_us.borrow() + us;
        let mut s = self.vop.borrow_mut();
        for p in 0..4 {
            if Self::active(&s, p) && !self.vp_clocked(p) {
                s.unclocked_seen[p] = true;
            }
            if !self.vp_running(&s, p) || self.faults.no_frames & 1 << p != 0 {
                continue;
            }
            if s.next_frame_us[p] == 0 {
                s.next_frame_us[p] = *self.now_us.borrow() + self.frame_us / 2;
            }
            while self.vp_running(&s, p) && s.next_frame_us[p] <= end {
                s.next_frame_us[p] += self.frame_us;
                if s.unclocked_seen[p] {
                    // the frame its clock was taken from ends now, with the
                    // windows of that frame
                    s.unclocked_seen[p] = false;
                    let sel = Self::vop_reg(&s, PORT_SEL);
                    let iova = self.iova_windows.borrow();
                    for w in (0..8).filter(|w| s.fetching & 1 << w != 0 && win_port(sel, *w) == p) {
                        if iova.contains(&s.fetch_addr[w]) {
                            self.refused.borrow_mut().push(format!(
                                "VP{p} got its pixel clock back with window {w} fetching {:#x}, an IOVA of the system before us",
                                s.fetch_addr[w]
                            ));
                        }
                    }
                }
                if s.stopping[p] {
                    s.stopping[p] = false;
                    s.raw[p] |= 1 << 6;
                    break;
                }
                s.raw[p] |= 1;
                if s.pending[p] {
                    s.pending[p] = false;
                    if let Some(sel) = s.port_sel_written.take() {
                        s.regs.insert(PORT_SEL, sel);
                    }
                    let sel = Self::vop_reg(&s, PORT_SEL);
                    for w in 0..8 {
                        if win_port(sel, w) != p {
                            continue;
                        }
                        let mut en = Self::vop_reg(&s, win_en(w)) & 1 != 0;
                        if w < 4 {
                            en = en && Self::vop_reg(&s, win_en(w) + 0x100) & 1 != 0;
                        }
                        if en {
                            s.fetching |= 1 << w;
                        } else {
                            s.fetching &= !(1 << w);
                        }
                        if let Some(mst) = win_mst(w) {
                            s.fetch_addr[w] = Self::vop_reg(&s, mst);
                        }
                    }
                }
                let sel = Self::vop_reg(&s, PORT_SEL);
                let fetching = s.fetching;
                for w in (0..8).filter(|w| fetching & 1 << w != 0 && win_port(sel, *w) == p) {
                    if self.faults.bus_error || self.bad_fetch(&s, w) {
                        s.sys_raw[0] |= 1 << 1;
                    }
                    if self.faults.underflow {
                        s.raw[p] |= 1 << 4;
                    }
                }
            }
        }
        *self.now_us.borrow_mut() = end;
    }

    /// The windows fetching now ([`win_en`] numbering).
    pub fn fetching(&self) -> u32 {
        self.vop.borrow().fetching
    }

    /// Port `p` runs frames now.
    pub fn port_runs(&self, p: usize) -> bool {
        self.vp_running(&self.vop.borrow(), p)
    }

    /// A system before us left this: the chain running is the caller's
    /// (it runs a bring-up first); this adds Linux's picture on Cluster0 of
    /// VP0 at an IOVA with both vop_mmu sets paging, and, when `vp1`, VP1
    /// running a Cluster1 window too (the T6's second HDMI output).
    pub fn leave_linux_running(&self, vp1: bool) {
        const IOVA: u32 = 0xffc0_0000;
        self.add_iova(IOVA);
        let mut s = self.vop.borrow_mut();
        let mut sel = Self::vop_reg(&s, PORT_SEL) & !(0x3 << 16 | 0x3 << 18);
        if vp1 {
            sel |= 1 << 18;
        }
        s.regs.insert(PORT_SEL, sel);
        for (w, on) in [(0, true), (1, vp1)] {
            if on {
                s.regs.insert(win_en(w), 1);
                s.regs.insert(win_en(w) + 0x100, 1);
                s.fetching |= 1 << w;
                s.fetch_addr[w] = IOVA;
            }
        }
        s.fetching &= !(1 << 4);
        s.regs.insert(win_en(4), 0);
        for set in s.mmu.iter_mut() {
            *set = MmuSet { status: 1, dte: 0x0123_4000, int_mask: 3 };
        }
        if vp1 {
            s.regs.insert(dsp_ctrl(1), 0xf);
        }
        drop(s);
        if vp1 {
            // VP1 from dclk_vop1_src, as the D0 dump reads it: DCLK_VOP1's
            // mux 0, its gate 53/0 and the source's gate 52/11 open
            self.poke(sel_addr(0, 112), self.get(sel_addr(0, 112)) & !(3 << 9));
            self.poke(gate_addr(0, 53), self.get(gate_addr(0, 53)) & !1);
            self.poke(gate_addr(0, 52), self.get(gate_addr(0, 52)) & !(1 << 11));
        }
    }

    /// Give VP3 a pixel clock or take it away (`p` = 3).
    pub fn set_vp_clock(&self, p: usize, on: bool) {
        let mut c = self.vp_clock.get();
        c[p] = on;
        self.vp_clock.set(c);
    }

    /// Name `addr` an IOVA of the system before us.
    pub fn add_iova(&self, addr: u32) {
        self.iova_windows.borrow_mut().push(addr);
    }
}

impl Rk3588Display {
    fn write32_inner(&self, addr: usize, val: u32) {
        self.writes.borrow_mut().push((addr, val));
        *self.reads_since.borrow_mut() = 0;
        let Some(b) = Self::blk(addr) else {
            panic!("write to an unmodelled address {addr:#x}");
        };
        if let Some(why) = self.refusal(b) {
            self.refused.borrow_mut().push(format!("write {addr:#x}: {why}"));
            return;
        }
        match b.name {
            B_VOP | B_MMU => self.vop_write(addr, val),
            B_HDMI => {
                if self.hdmi_check(addr - HDMI, false) {
                    self.blocks.borrow_mut().insert(addr, val);
                }
            }
            B_PHY => {
                if self.phy_check(addr) {
                    self.blocks.borrow_mut().insert(addr, val);
                }
            }
            B_CRU | B_PMU | B_SYS_GRF | B_VOP_GRF | B_VO1_GRF | B_PHY_GRF => {
                if b.name == B_PMU {
                    self.check_switch(addr, val);
                    // the VOP's bus clocks must not stop before a port's
                    // STANDBY has taken effect (DSP_HOLD_VALID; design note
                    // section 1, rockchip_drm_vop2.c:950-969)
                    let vop_off = addr == PWR + 4 && val & (val >> 16) & (1 << 1) != 0;
                    let s = self.vop.borrow();
                    if vop_off && (0..4).any(|p| self.vp_running(&s, p)) {
                        self.refused.borrow_mut().push("PD_VOP switched off under a running port".into());
                    }
                }
                let mask = val >> 16;
                if matches!(b.name, B_SYS_GRF | B_VOP_GRF | B_VO1_GRF | B_PHY_GRF) && val & 0xffff & !mask != 0 {
                    self.refused.borrow_mut().push(format!("GRF write {addr:#x} = {val:#x}: a bit outside its mask"));
                }
                if b.name == B_VOP_GRF && self.faults.grf_stuck {
                    return;
                }
                if addr == sel_addr(0, 112) && self.faults.dclk_mux_stuck {
                    return;
                }
                if addr == rst_addr(0, 52) && self.faults.vop_reset_stuck && (val >> 16) & !val & (1 << 9) != 0 {
                    return;
                }
                let resets_before: Vec<bool> = if b.name == B_CRU { self.reset_states() } else { Vec::new() };
                {
                    let mut r = self.regs.borrow_mut();
                    let e = if matches!(b.name, B_CRU | B_PMU) {
                        r.get_mut(&addr).unwrap_or_else(|| panic!("write to an unmodelled register {addr:#x}"))
                    } else {
                        r.entry(addr).or_insert(0)
                    };
                    *e = (*e & !mask) | (val & mask);
                }
                if b.name == B_CRU {
                    self.after_reset_write(&resets_before);
                }
            }
            _ => unreachable!(),
        }
    }
}

impl Mmio for Rk3588Display {
    fn read32(&self, addr: usize) -> u32 {
        *self.reads_since.borrow_mut() += 1;
        if *self.reads_since.borrow() > self.lag {
            self.settle();
        }
        let Some(b) = Self::blk(addr) else {
            panic!("read of an unmodelled address {addr:#x}");
        };
        if let Some(why) = self.refusal(b) {
            self.refused.borrow_mut().push(format!("read {addr:#x}: {why}"));
            return 0;
        }
        match b.name {
            B_PHY if !self.phy_check(addr) => 0,
            B_HDMI if !self.hdmi_check(addr - HDMI, true) => 0,
            B_CRU | B_PMU => {
                *self.regs.borrow().get(&addr).unwrap_or_else(|| panic!("read of an unmodelled register {addr:#x}"))
            }
            _ => self.peek(addr),
        }
    }

    fn write32(&self, addr: usize, val: u32) {
        let vp0_running = self.vp_running(&self.vop.borrow(), 0);
        self.write32_inner(addr, val);
        // VP0's pixel clock (the PHY's PLL, the DCLK_VOP0 gate, mux or
        // reset) taken away while VP0 runs and its STANDBY has not taken
        // effect: its frame stops in the middle. Linux stops a port first; the PLL goes
        // off only when its last user, the VOP's dclk, lets it go
        // (hdptx.c:1134-1164, rockchip_drm_vop2.c:950-969).
        if vp0_running && !self.vp_clocked(0) && Self::active(&self.vop.borrow(), 0) {
            self.refused.borrow_mut().push(format!("write {addr:#x} = {val:#x}: VP0's pixel clock stopped under it"));
        }
    }

    fn read8(&self, _: usize) -> u8 {
        unreachable!("every block of the RK3588 display path is accessed 32 bits wide")
    }
    fn write8(&self, _: usize, _: u8) {
        unreachable!("every block of the RK3588 display path is accessed 32 bits wide")
    }
}

impl Rk3588Display {
    /// Asserted state of each block's resets, in [`BLKS`] order.
    fn reset_states(&self) -> Vec<bool> {
        BLKS.iter().map(|b| b.resets.iter().any(|(p, c, bit)| !self.rst_released(*p, *c, *bit))).collect()
    }
    /// A block whose reset went from released to asserted loses its
    /// registers (module text).
    fn after_reset_write(&self, before: &[bool]) {
        let now = self.reset_states();
        for (i, b) in BLKS.iter().enumerate() {
            if !before[i] && now[i] {
                self.reset_block(b);
            }
        }
    }
}

impl Env for Rk3588Display {
    fn busy_wait_us(&self, us: u32) {
        self.advance(u64::from(us));
    }
    fn sleep_ms(&self, ms: u32) {
        self.advance(u64::from(ms) * 1000);
    }
    fn log_str(&self, level: Level, line: &str) {
        self.lines.borrow_mut().push((level, line.to_string()));
    }
    fn i2c_write(&self, _: u32, _: &[u8]) -> Result<(), i32> {
        Err(-crate::errno::ENOTSUP)
    }
    fn i2c_write_read(&self, _: u32, _: &[u8], _: &mut [u8]) -> Result<(), i32> {
        Err(-crate::errno::ENOTSUP)
    }
    fn gpio_set(&self, _: u32, _: bool) -> Result<(), i32> {
        Err(-crate::errno::ENOTSUP)
    }
}

/// The blocks as a Zephyr image maps them: each one at its own virtual
/// address, none at its physical one (on the T6, `device_map()` returns
/// addresses from 0x10000000 up). One window per mapping the display
/// probes make, page offsets kept (the two `vop_mmu` sets share a page).
/// A test that passes [`virt`] bases through [`Mapped`] fails when the code
/// compares a base with a physical constant, or reaches a block from
/// another block's base.
const WINDOWS: [(usize, usize); 10] = [
    (VOP, 0x5000),
    (0xfdd9_7000, 0x1000),
    (PMU, 0x1000),
    (CRU, 0x3_1000),
    (SYS_GRF, 0x1000),
    (VOP_GRF, 0x1000),
    (VO1_GRF, 0x1000),
    (PHY_GRF, 0x1000),
    (HDMI, 0x2_0000),
    (PHY, 0x2000),
];

/// The first window's virtual address; window i is 1 MiB after window i - 1.
const VIRT_BASE: usize = 0x4000_0000;

/// The virtual address of `phys` in [`WINDOWS`]. Panics for an address no
/// window holds.
pub const fn virt(phys: usize) -> usize {
    let mut i = 0;
    while i < WINDOWS.len() {
        let (base, size) = WINDOWS[i];
        if phys >= base && phys < base + size {
            return VIRT_BASE + i * 0x10_0000 + (phys - base);
        }
        i += 1;
    }
    panic!("no mapping holds this physical address");
}

/// The physical address of `va`, or `None` outside every window.
fn phys(va: usize) -> Option<usize> {
    let i = va.checked_sub(VIRT_BASE)? / 0x10_0000;
    let (base, size) = *WINDOWS.get(i)?;
    let off = va - VIRT_BASE - i * 0x10_0000;
    (off < size).then_some(base + off)
}

/// `m` seen through [`WINDOWS`]: every access is translated to its physical
/// address; one outside every window panics (on the board, a fault).
pub struct Mapped<'a, M: Mmio>(pub &'a M);

impl<M: Mmio> Mapped<'_, M> {
    fn at(va: usize) -> usize {
        phys(va).unwrap_or_else(|| panic!("access at {va:#x}: no mapping holds it (a physical address?)"))
    }
}

impl<M: Mmio> Mmio for Mapped<'_, M> {
    fn read32(&self, addr: usize) -> u32 {
        self.0.read32(Self::at(addr))
    }
    fn write32(&self, addr: usize, val: u32) {
        self.0.write32(Self::at(addr), val)
    }
    fn read8(&self, addr: usize) -> u8 {
        self.0.read8(Self::at(addr))
    }
    fn write8(&self, addr: usize, val: u8) {
        self.0.write8(Self::at(addr), val)
    }
}

/// The chain's bases as the probe passes them: [`virt`] of the physical ones.
pub const fn chain_bases() -> crate::soc::rk3588_display_chain::Bases {
    use crate::soc::rk3588_display_chain::{Bases, RK3588 as P};
    Bases {
        cru: virt(P.cru),
        pmu: virt(P.pmu),
        sys_grf: virt(P.sys_grf),
        vop_grf: virt(P.vop_grf),
        vo1_grf: virt(P.vo1_grf),
        phy: crate::ip::hdptx::Bases { phy: virt(P.phy.phy), grf: virt(P.phy.grf) },
        hdmi: virt(P.hdmi),
        vop: virt(P.vop),
        vop_mmu: [virt(P.vop_mmu[0]), virt(P.vop_mmu[1])],
    }
}

#[test]
fn every_mapped_address_differs_from_its_physical_one_and_maps_back() {
    for (base, size) in WINDOWS {
        for off in [0, size - 4] {
            let va = virt(base + off);
            assert_ne!(va, base + off);
            assert_eq!(phys(va), Some(base + off));
        }
    }
    assert_eq!(phys(VIRT_BASE - 4), None);
    assert_eq!(phys(virt(PMU) + 0x1000), None, "past the PMU's page");
}
