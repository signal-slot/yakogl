// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host tests' stand-in for the hardware and the OS (tests only).
//!
//! [`Model`] is an [`Mmio`] over a few register blocks at fake addresses. It
//! records every access as (block, offset, value) in order, so a test can
//! assert exactly what a driver wrote and in what order, and it answers the
//! things drivers poll or count -- only those, and only as far as the
//! documentation in the driver says the hardware does. It proves a driver
//! follows the sequence it claims; whether the silicon behaves as modelled is
//! for the board to say. Failures are injected through [`Faults`].
//!
//! The i.MX 8M Plus media blocks are modelled from IMX8MPRM Rev. 3:
//! - CCM: every register at 0x4000 and above has SET (+4), CLR (+8) and TOG
//!   (+0xC) aliases, and all four read the register (5.1.6.1).
//! - GPC: a PU power-up / power-down trigger self-clears (5.2.9.4); the
//!   power-up takes effect only for a domain mapped to the A53 (5.2.8, rule
//!   2), otherwise A53_PU_PGC_PUP_STATUS0 records the refusal (5.2.10.45); a
//!   power-down switches the power off only with the PGC's PCR set
//!   (5.2.11.9) and then sets PSR (5.2.11.12), which a write of 1 clears; the
//!   MEDIAMIX ADB400 acknowledge follows its request while MEDIAMIX is
//!   powered (5.2.10.57, read the other way round for power-up: the RM does
//!   not state it).
//! - ANATOP: VIDEO_PLL1's PLL_LOCK reads 1 a few reads after PLL_RST rises,
//!   0 while PLL_RST is low (5.1.8.11; the delay is the model's).
//! - MEDIA_BLK_CTRL: reset to zero when MEDIAMIX powers down (its registers
//!   are inside the domain); any access while MEDIAMIX is off, or while
//!   MEDIA_APB_CLK_ROOT or the MEDIA clock gate (CCGR93) is off, fails the
//!   test (the block's registers sit behind media_apb, Table 5-1 / 5-9).
//!
//! - MIPI_DSI (13.6): reachable only with MEDIAMIX powered and the block's
//!   APB clock enabled and out of reset in MEDIA_BLK_CTRL; registers reset to
//!   the 13.6.9.1.1 memory map's values; `PllStable` sets a few reads after
//!   `PllEn` only while the PLL has its reference (MEDIA_MIPI_PHY1_REF root
//!   enabled, MIPI_PHY1 powered, the block's CLKREF clock on and the D-PHY
//!   master reset `GPR_MIPI_M_RESETN` released); `SwRst` completes only with
//!   the PLL stable (Table 13-26 case 16: a synchronous reset), clears itself
//!   and resets every register but STATUS, SWRST, CLKCTRL, PLLCTRL and
//!   PLLTMR (13.6.9.1.5); lane stop states and `TxReadyHsClk` follow the lane
//!   enables, the byte clock and the HS request; INTSRC is write-1-to-clear.
//! - LCDIF1 (13.3): reachable with MEDIAMIX powered and its APB clock on and
//!   out of reset; `CTRL` resets to `SW_RESET` set and has SET/CLR/TOG
//!   aliases; `SHADOW_LOAD_EN` clears on the next read while the pixel clock
//!   runs (MEDIA_DISP1_PIX root enabled, `LCDIF_PIXEL` clock on and out of
//!   reset, `DISP_ON`); INT_STATUS_D0 is write-1-to-clear.
//! - The GPUMIX handshake (PU_PWRHSK request 9 / acknowledge 25) is modelled
//!   like MEDIAMIX's, with GPU_SHARE_LOGIC powered.
//! - GC (the Vivante GC7000, gpu::vivante): IDLE_STATE all idle,
//!   CLOCK_CONTROL with IDLE_3D/IDLE_2D, INTR_ACKNOWLEDGE read-to-clear, and
//!   the board's two odd answers, MMUv2_AHB_CONTROL reading the
//!   absent-address pattern and AXI_CONFIG reading 0. With
//!   [`Model::require_gpu_power`] any GC access before GPUMIX, GPU3D and the
//!   GPU clocks are up fails the test.
//!
//! [`TestEnv`] is an [`Env`] that records log lines and waits, and fails the
//! I2C and GPIO calls on request.

use crate::env::{Env, Level};
use crate::mmio::Mmio;
use std::cell::RefCell;
use std::collections::BTreeMap;

pub const PWM: &str = "pwm";
pub const CCM: &str = "ccm";
/// The PowerVR register bank (gpu::pvr): plain storage, no behaviour.
pub const GPU: &str = "gpu";
pub const GPC: &str = "gpc";
pub const ANATOP: &str = "anatop";
pub const BLK: &str = "blk";
pub const DSIM: &str = "dsim";
pub const LCDIF1: &str = "lcdif1";

/// The Vivante GC7000 register block (gpu::vivante).
pub const GC: &str = "gc";

const BLOCKS: [(&str, usize, usize); 9] = [
    (PWM, 0x1000_0000, 0x1_0000),
    (CCM, 0x2000_0000, 0x1_0000),
    (GPC, 0x3000_0000, 0x1_0000),
    (ANATOP, 0x4000_0000, 0x1_0000),
    (BLK, 0x5000_0000, 0x1_0000),
    (GPU, 0x6000_0000, 0x1_0000),
    (DSIM, 0x7000_0000, 0x1000),
    (LCDIF1, 0x8000_0000, 0x1_0000),
    (GC, 0x9000_0000, 0x1000),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    R(&'static str, u32, u32),
    W(&'static str, u32, u32),
}

/// Injected failures.
#[derive(Default, Clone, Copy)]
pub struct Faults {
    /// PWMCR.SWR never clears.
    pub pwm_swr_stuck: bool,
    /// A GPC PU power-up trigger never self-clears.
    pub gpc_pup_stuck: bool,
    /// A GPC PU power-up trigger clears but is refused (STATUS0 set).
    pub gpc_pup_refused: bool,
    /// A GPC PU power-down trigger never self-clears.
    pub gpc_pdn_stuck: bool,
    /// The MEDIAMIX ADB400 never acknowledges.
    pub adb_no_ack: bool,
    /// VIDEO_PLL1 never locks.
    pub pll_no_lock: bool,
    /// The DSIM's PllStable never sets.
    pub dsim_pll_no_lock: bool,
    /// The DSIM's software reset never releases.
    pub dsim_swrst_stuck: bool,
    /// The DSIM's lanes never reach stop state.
    pub dsim_lanes_stuck: bool,
    /// LCDIF1 never reaches a frame boundary (SHADOW_LOAD_EN stays set).
    pub lcdif_no_vsync: bool,
    /// The GPUMIX ADB400 never acknowledges.
    pub gpumix_adb_no_ack: bool,
    /// The GC7000 never reports idle (IDLE_STATE FE, PE and SH busy).
    pub gc_never_idle: bool,
    /// The GC7000's MMUv2_SEC_CONTROL.ENABLE stays set through a reset.
    pub gc_mmu_stuck: bool,
}

#[derive(Default)]
struct State {
    regs: BTreeMap<(&'static str, u32), u32>,
    log: Vec<Access>,
    faults: Faults,
    /// Reads of PWMCR left until a software reset completes.
    swr_reads_left: u32,
    /// Sample words in the PWM FIFO (PWMSR.FIFOAV).
    pwm_fifo: u32,
    /// PU domains powered, as PU_PGC_SW_PUP_REQ bits.
    powered: u32,
    /// Pending GPC triggers and the reads until they complete.
    pup_pending: u32,
    pup_reads_left: u32,
    pdn_pending: u32,
    pdn_reads_left: u32,
    /// Reads of VIDEO_PLL1_GEN_CTRL until PLL_LOCK.
    lock_reads_left: u32,
    /// DSIM: reads of STATUS until PllStable (None: PLL off or no reference).
    dsim_pll_reads_left: Option<u32>,
    /// DSIM: reads until the software reset releases (None: not in reset).
    dsim_swrst_reads_left: Option<u32>,
    /// DSIM: FIFO pointer initialisations seen (FIFOCTRL bits 4:0 low, then high).
    dsim_fifo_inits: u32,
    /// GC accesses require GPUMIX and GPU3D powered and their clocks on.
    gc_needs_power: bool,
}

#[derive(Default)]
pub struct Model(RefCell<State>);

// PERI_PWM.h offsets, repeated here on purpose: the model must not share the
// driver's view of the hardware, or a wrong constant would agree with itself.
const PWMCR: u32 = 0x0;
const PWMSR: u32 = 0x4;
const PWMSAR: u32 = 0xc;
const SWR: u32 = 1 << 3;

// The same for the media blocks (IMX8MPRM Rev. 3 register maps).
const GPC_PUP: u32 = 0xd8;
const GPC_PDN: u32 = 0xe4;
const GPC_PUP_STATUS0: u32 = 0x124;
const GPC_PWRHSK: u32 = 0x190;
const GPC_MAP: u32 = 0x1cc;
const MEDIMIX: u32 = 1 << 10;
const MEDIAMIX_ADB_REQ: u32 = 1 << 14;
const MEDIAMIX_ADB_ACK: u32 = 1 << 30;
/// PU trigger bit -> A53 mapping bit (5.2.10.37 / 5.2.10.72).
const PU_MAP: [(u32, u32); 5] =
    [(1 << 0, 1 << 2), (1 << 6, 1 << 8), (1 << 7, 1 << 9), (1 << 9, 1 << 11), (1 << 10, 1 << 12)];
const fn pu_ctrl(bit: u32) -> u32 {
    0xb00 + bit.trailing_zeros() * 0x40
}
const fn pu_sr(bit: u32) -> u32 {
    pu_ctrl(bit) + 0xc
}
const VIDEO_PLL1_GEN: u32 = 0x28;
const PLL_RST: u32 = 1 << 9;
const PLL_LOCK: u32 = 1 << 31;
const CCM_MEDIA_APB_ROOT: u32 = 0x8a80;
const CCM_CCGR93: u32 = 0x45d0;
const CCM_MIPI_PHY1_REF_ROOT: u32 = 0xbd80;
const CCM_DISP1_PIX_ROOT: u32 = 0xbe00;
const ROOT_ENABLE: u32 = 1 << 28;
const PU_MIPI_PHY1: u32 = 1 << 0;
// MEDIA_BLK_CTRL (13.2.3.1.2-.4)
const BLK_SFT_RSTN: u32 = 0x0;
const BLK_CLK_EN: u32 = 0x4;
const BLK_MIPI_RESET_DIV: u32 = 0x8;
const BLK_DSI_PCLK: u32 = 1 << 0;
const BLK_DSI_CLKREF: u32 = 1 << 1;
const BLK_LCDIF_PIXEL: u32 = 1 << 4;
const BLK_LCDIF_APB: u32 = 1 << 5;
const BLK_MIPI_M_RESETN: u32 = 1 << 17;
// MIPI_DSI (13.6.9.1)
const D_STATUS: u32 = 0x4;
const D_SWRST: u32 = 0xc;
const D_CLKCTRL: u32 = 0x10;
const D_CONFIG: u32 = 0x18;
const D_INTSRC: u32 = 0x34;
const D_FIFOCTRL: u32 = 0x4c;
const D_PLLCTRL: u32 = 0x94;
const D_PLLTMR: u32 = 0xa0;
const D_PLLEN: u32 = 1 << 23;
const D_SWRST_BIT: u32 = 1 << 0;
const D_TXREQHS: u32 = 1 << 31;
const D_BYTECLKEN: u32 = 1 << 24;
/// The memory map's reset values (13.6.9.1.1); 0 for the rest.
const DSIM_RESET: [(u32, u32); 14] = [
    (0x00, 0x0106_0100),
    (0x08, 0x0000_0001),
    (0x10, 0x0000_ffff),
    (0x14, 0x00ff_ffff),
    (0x18, 0x0200_0000),
    (0x20, 0x0300_0400),
    (0x24, 0xf000_0000),
    (0x30, 0x0300_0400),
    (0x38, 0xbb37_ffff),
    (0x48, 0x0000_01ff),
    (0x4c, 0x0155_551f),
    (0x50, 0x0000_4040),
    (0x78, 0x0000_0002),
    (0xa0, 0xffff_ffff),
];
/// Registers a software reset leaves alone (13.6.9.1.5).
const DSIM_SWRST_KEEPS: [u32; 5] = [D_STATUS, D_SWRST, D_CLKCTRL, D_PLLCTRL, D_PLLTMR];
// LCDIFv3 (13.3.3.1)
const L_DISP_PARA: u32 = 0x10;
const L_INT_STATUS_D0: u32 = 0x24;
const L_DESC5: u32 = 0x214;
const GPU_SHARE_LOGIC: u32 = 1 << 7;
const GPU3D: u32 = 1 << 9;
const GPUMIX_ADB_REQ: u32 = 1 << 9;
const GPUMIX_ADB_ACK: u32 = 1 << 25;
/// GPU3D_CORE / GPU_AXI roots and the CCGR70 / CCGR87 gates (Table 5-1, 5-9).
const CCM_GPU3D_CORE_ROOT: u32 = 0x8180;
const CCM_GPU_AXI_ROOT: u32 = 0x8c00;
const CCM_CCGR70: u32 = 0x4460;
const CCM_CCGR87: u32 = 0x4570;
// The GC7000 registers the model answers (rnndb state_hi.xml.h), and what
// it answers: IDLE_STATE all idle, CLOCK_CONTROL with IDLE_3D/IDLE_2D,
// INTR_ACKNOWLEDGE read-to-clear, MMUv2_AHB_CONTROL the absent pattern and
// AXI_CONFIG 0 (both measured on the board, gpu::vivante::hw) -- the same as
// the register file of driver/tools/gclab/kerneltest (deleted with the C,
// 4c25beb0) that recorded gpu::vivante's c_golden.rs, so the traces compare.
const GC_CLOCK_CONTROL: u32 = 0x0;
const GC_IDLE_STATE: u32 = 0x4;
const GC_AXI_CONFIG: u32 = 0x8;
const GC_INTR_ACK: u32 = 0x10;
const GC_SEC_CONTROL: u32 = 0x388;
const GC_AHB_CONTROL: u32 = 0x3a8;

impl Model {
    pub fn new() -> Model {
        Model::default()
    }

    pub fn with_faults(f: Faults) -> Model {
        let m = Model::new();
        m.0.borrow_mut().faults = f;
        m
    }

    pub fn base(&self, block: &str) -> usize {
        BLOCKS.iter().find(|b| b.0 == block).expect("block").1
    }

    /// Set a register without logging (the state a test starts from).
    pub fn poke(&self, block: &'static str, off: u32, val: u32) {
        let mut s = self.0.borrow_mut();
        if block == PWM && off == PWMSR {
            s.pwm_fifo = val & 7;
        }
        if block == ANATOP && off == VIDEO_PLL1_GEN {
            s.lock_reads_left = 0;
        }
        s.regs.insert((block, off), val);
    }

    pub fn peek(&self, block: &'static str, off: u32) -> u32 {
        let s = self.0.borrow();
        Model::value(&s, block, off)
    }

    fn value(s: &State, block: &'static str, off: u32) -> u32 {
        let raw = |o| s.regs.get(&(block, o)).copied().unwrap_or(0);
        match (block, off) {
            (PWM, PWMSR) => (raw(off) & !7) | s.pwm_fifo,
            (CCM, o) if o >= 0x4000 => raw(o & !0xf),
            (GPC, GPC_PWRHSK) => {
                let ack = raw(off) & MEDIAMIX_ADB_REQ != 0 && s.powered & MEDIMIX != 0 && !s.faults.adb_no_ack;
                let gack =
                    raw(off) & GPUMIX_ADB_REQ != 0 && s.powered & GPU_SHARE_LOGIC != 0 && !s.faults.gpumix_adb_no_ack;
                (raw(off) & !(MEDIAMIX_ADB_ACK | GPUMIX_ADB_ACK))
                    | if ack { MEDIAMIX_ADB_ACK } else { 0 }
                    | if gack { GPUMIX_ADB_ACK } else { 0 }
            }
            (GC, GC_IDLE_STATE) => {
                if s.faults.gc_never_idle {
                    0x7fff_fff2
                } else {
                    0x7fff_ffff
                }
            }
            (GC, GC_CLOCK_CONTROL) => raw(off) | if s.faults.gc_never_idle { 0 } else { (1 << 16) | (1 << 17) },
            (GC, GC_AHB_CONTROL) => 0xdead_de42,
            (GC, GC_AXI_CONFIG) => 0,
            (GC, GC_SEC_CONTROL) => raw(off) | u32::from(s.faults.gc_mmu_stuck),
            (ANATOP, VIDEO_PLL1_GEN) => {
                let v = raw(off);
                let locked = v & PLL_RST != 0 && !s.faults.pll_no_lock && s.lock_reads_left == 0;
                (v & !PLL_LOCK) | if locked { PLL_LOCK } else { 0 }
            }
            (DSIM, D_STATUS) => Model::dsim_status(s),
            (DSIM, D_SWRST) => {
                if s.dsim_swrst_reads_left.is_some() {
                    D_SWRST_BIT
                } else {
                    0
                }
            }
            (DSIM, o) => Model::dsim_reg(s, o),
            (LCDIF1, o) if o < 0x10 => s.regs.get(&(LCDIF1, 0)).copied().unwrap_or(0x8000_0000),
            _ => raw(off),
        }
    }

    fn dsim_reset(off: u32) -> u32 {
        DSIM_RESET.iter().find(|r| r.0 == off).map_or(0, |r| r.1)
    }

    fn dsim_reg(s: &State, off: u32) -> u32 {
        s.regs.get(&(DSIM, off)).copied().unwrap_or_else(|| Model::dsim_reset(off))
    }

    fn dsim_pll_stable(s: &State) -> bool {
        s.dsim_pll_reads_left == Some(0)
    }

    /// STATUS from the state: PllStable, SwRstRls, the lanes.
    fn dsim_status(s: &State) -> u32 {
        let mut v = 0;
        let pll = Model::dsim_pll_stable(s);
        if pll {
            v |= 1 << 31;
        }
        if s.dsim_swrst_reads_left.is_none() {
            v |= 1 << 20;
        }
        let cfg = Model::dsim_reg(s, D_CONFIG);
        let clk = Model::dsim_reg(s, D_CLKCTRL);
        let running = pll && clk & D_BYTECLKEN != 0 && !s.faults.dsim_lanes_stuck;
        if running {
            v |= (cfg >> 1) & 0xf; // StopstateDat follows LaneEn[4:1]
            if cfg & 1 != 0 {
                v |= if clk & D_TXREQHS != 0 { 1 << 10 } else { 1 << 8 };
            }
        }
        v
    }

    /// What the DSIM PLL needs to lock: its reference root, MIPI_PHY1's
    /// power, the block's CLKREF clock and the D-PHY master reset released.
    fn dsim_pll_has_reference(s: &State) -> bool {
        let blk = |o| s.regs.get(&(BLK, o)).copied().unwrap_or(0);
        Model::value(s, CCM, CCM_MIPI_PHY1_REF_ROOT) & ROOT_ENABLE != 0
            && s.powered & PU_MIPI_PHY1 != 0
            && blk(BLK_CLK_EN) & BLK_DSI_CLKREF != 0
            && blk(BLK_SFT_RSTN) & BLK_DSI_CLKREF != 0
            && blk(BLK_MIPI_RESET_DIV) & BLK_MIPI_M_RESETN != 0
    }

    /// LCDIF1's pixel clock runs and the display is on: frames are timed.
    fn lcdif_frames(s: &State) -> bool {
        let blk = |o| s.regs.get(&(BLK, o)).copied().unwrap_or(0);
        Model::value(s, CCM, CCM_DISP1_PIX_ROOT) & ROOT_ENABLE != 0
            && blk(BLK_CLK_EN) & BLK_LCDIF_PIXEL != 0
            && blk(BLK_SFT_RSTN) & BLK_LCDIF_PIXEL != 0
            && s.regs.get(&(LCDIF1, L_DISP_PARA)).copied().unwrap_or(0) & (1 << 31) != 0
            && !s.faults.lcdif_no_vsync
    }

    /// DSIM and LCDIF1 registers sit inside MEDIAMIX behind their APB clocks.
    fn check_media_ip(s: &State, blk: &str, what: &str, off: u32) {
        let (clk, name) = if blk == DSIM { (BLK_DSI_PCLK, "MIPI_DSI_PCLK") } else { (BLK_LCDIF_APB, "LCDIF_APB_CLK") };
        let reg = |o| s.regs.get(&(BLK, o)).copied().unwrap_or(0);
        assert!(s.powered & MEDIMIX != 0, "{blk} {what} {off:#x} with MEDIAMIX powered off");
        assert!(reg(BLK_CLK_EN) & clk != 0, "{blk} {what} {off:#x} with {name} off");
        assert!(reg(BLK_SFT_RSTN) & clk != 0, "{blk} {what} {off:#x} with {name} in reset");
    }

    /// How many times the DSIM's FIFO pointers were initialised.
    pub fn dsim_fifo_inits(&self) -> u32 {
        self.0.borrow().dsim_fifo_inits
    }

    /// From now on, any GC access with GPUMIX or GPU3D unpowered, or the
    /// GPU clock roots or gates off, fails the test.
    pub fn require_gpu_power(&self) {
        self.0.borrow_mut().gc_needs_power = true;
    }

    fn check_gc(s: &State, what: &str, off: u32) {
        if !s.gc_needs_power {
            return;
        }
        assert!(s.powered & GPU_SHARE_LOGIC != 0, "GC {what} {off:#x} with GPUMIX powered off");
        assert!(s.powered & GPU3D != 0, "GC {what} {off:#x} with GPU3D powered off");
        for r in [CCM_GPU3D_CORE_ROOT, CCM_GPU_AXI_ROOT] {
            assert!(Model::value(s, CCM, r) & (1 << 28) != 0, "GC {what} {off:#x} with root {r:#x} off");
        }
        for g in [CCM_CCGR70, CCM_CCGR87] {
            assert!(Model::value(s, CCM, g) & 3 == 3, "GC {what} {off:#x} with gate {g:#x} closed");
        }
    }

    /// Mark PU domains powered (the state a test starts from).
    pub fn set_powered(&self, pu_bits: u32) {
        self.0.borrow_mut().powered = pu_bits;
    }

    pub fn powered(&self) -> u32 {
        self.0.borrow().powered
    }

    pub fn accesses(&self) -> Vec<Access> {
        self.0.borrow().log.clone()
    }

    pub fn writes(&self) -> Vec<(&'static str, u32, u32)> {
        self.0
            .borrow()
            .log
            .iter()
            .filter_map(|a| match *a {
                Access::W(b, o, v) => Some((b, o, v)),
                Access::R(..) => None,
            })
            .collect()
    }

    pub fn clear_log(&self) {
        self.0.borrow_mut().log.clear();
    }

    fn decode(addr: usize) -> (&'static str, u32) {
        for (name, base, size) in BLOCKS {
            if addr >= base && addr < base + size {
                assert!(addr.is_multiple_of(4), "unaligned access at {addr:#x}");
                return (name, (addr - base) as u32);
            }
        }
        panic!("access outside every modelled block: {addr:#x}");
    }

    /// MEDIA_BLK_CTRL is reachable only with MEDIAMIX powered and its bus
    /// clock root and gate on.
    fn check_blk(s: &State, what: &str, off: u32) {
        let root = Model::value(s, CCM, CCM_MEDIA_APB_ROOT);
        let gate = Model::value(s, CCM, CCM_CCGR93) & 3;
        assert!(s.powered & MEDIMIX != 0, "BLK_CTRL {what} {off:#x} with MEDIAMIX powered off");
        assert!(root & (1 << 28) != 0, "BLK_CTRL {what} {off:#x} with MEDIA_APB_CLK_ROOT off");
        assert!(gate != 0, "BLK_CTRL {what} {off:#x} with the MEDIA clock gate (CCGR93) closed");
    }

    /// GPC trigger completion, on a read of the trigger register.
    fn gpc_complete(s: &mut State, off: u32) {
        if off == GPC_PUP && s.pup_pending != 0 && !s.faults.gpc_pup_stuck {
            if s.pup_reads_left > 0 {
                s.pup_reads_left -= 1;
                return;
            }
            let map = s.regs.get(&(GPC, GPC_MAP)).copied().unwrap_or(0);
            for (pu, mbit) in PU_MAP {
                if s.pup_pending & pu == 0 {
                    continue;
                }
                if map & mbit == 0 || s.faults.gpc_pup_refused {
                    *s.regs.entry((GPC, GPC_PUP_STATUS0)).or_insert(0) |= pu;
                } else {
                    s.powered |= pu;
                    *s.regs.entry((GPC, GPC_PUP_STATUS0)).or_insert(0) &= !pu;
                }
            }
            let p = s.pup_pending;
            *s.regs.entry((GPC, GPC_PUP)).or_insert(0) &= !p;
            s.pup_pending = 0;
        }
        if off == GPC_PDN && s.pdn_pending != 0 && !s.faults.gpc_pdn_stuck {
            if s.pdn_reads_left > 0 {
                s.pdn_reads_left -= 1;
                return;
            }
            for (pu, _) in PU_MAP {
                if s.pdn_pending & pu == 0 {
                    continue;
                }
                let pcr = s.regs.get(&(GPC, pu_ctrl(pu))).copied().unwrap_or(0) & 1;
                if pcr != 0 && s.powered & pu != 0 {
                    s.powered &= !pu;
                    *s.regs.entry((GPC, pu_sr(pu))).or_insert(0) |= 1;
                    if pu == MEDIMIX {
                        s.regs.retain(|k, _| k.0 != BLK);
                    }
                }
            }
            let p = s.pdn_pending;
            *s.regs.entry((GPC, GPC_PDN)).or_insert(0) &= !p;
            s.pdn_pending = 0;
        }
    }
}

impl Mmio for Model {
    fn read32(&self, addr: usize) -> u32 {
        let (blk, off) = Model::decode(addr);
        let mut s = self.0.borrow_mut();
        if blk == BLK {
            Model::check_blk(&s, "read", off);
        }
        if blk == DSIM || blk == LCDIF1 {
            Model::check_media_ip(&s, blk, "read", off);
        }
        if blk == GPC {
            Model::gpc_complete(&mut s, off);
        }
        if blk == DSIM && off == D_STATUS {
            if let Some(n) = s.dsim_pll_reads_left {
                if n > 0 && Model::dsim_pll_has_reference(&s) && !s.faults.dsim_pll_no_lock {
                    s.dsim_pll_reads_left = Some(n - 1);
                }
            }
        }
        if blk == DSIM && (off == D_STATUS || off == D_SWRST) {
            if let Some(n) = s.dsim_swrst_reads_left {
                if n > 0 {
                    s.dsim_swrst_reads_left = Some(n - 1);
                } else if !s.faults.dsim_swrst_stuck && Model::dsim_pll_stable(&s) {
                    // the reset is over: every register it covers back to reset
                    s.regs.retain(|k, _| k.0 != DSIM || DSIM_SWRST_KEEPS.contains(&k.1));
                    s.dsim_swrst_reads_left = None;
                }
            }
        }
        if blk == LCDIF1 && off == L_DESC5 && Model::lcdif_frames(&s) {
            // a frame boundary passes between two reads: the shadow load is taken
            let v = s.regs.get(&(LCDIF1, off)).copied().unwrap_or(0);
            if v & (1 << 30) != 0 {
                s.regs.insert((LCDIF1, off), v & !(1 << 30));
                s.log.push(Access::R(blk, off, v));
                return v;
            }
        }
        if blk == GC {
            Model::check_gc(&s, "read", off);
        }
        let mut v = Model::value(&s, blk, off);
        if blk == GC && off == GC_INTR_ACK {
            s.regs.insert((GC, off), 0);
        }
        if blk == ANATOP && off == VIDEO_PLL1_GEN && s.lock_reads_left > 0 {
            s.lock_reads_left -= 1;
        }
        if blk == PWM && off == PWMCR && v & SWR != 0 && !s.faults.pwm_swr_stuck {
            if s.swr_reads_left == 0 {
                // The reset is over: every register back to zero.
                s.regs.retain(|k, _| k.0 != PWM);
                s.pwm_fifo = 0;
                v = 0;
            } else {
                s.swr_reads_left -= 1;
            }
        }
        s.log.push(Access::R(blk, off, v));
        v
    }

    fn write32(&self, addr: usize, val: u32) {
        let (blk, off) = Model::decode(addr);
        let mut s = self.0.borrow_mut();
        s.log.push(Access::W(blk, off, val));
        if blk == BLK {
            Model::check_blk(&s, "write", off);
        }
        if blk == DSIM || blk == LCDIF1 {
            Model::check_media_ip(&s, blk, "write", off);
        }
        if blk == DSIM {
            match off {
                D_STATUS => return, // read only
                D_SWRST => {
                    if val & D_SWRST_BIT != 0 {
                        s.dsim_swrst_reads_left = Some(2);
                    }
                    return;
                }
                D_INTSRC => {
                    let cur = Model::dsim_reg(&s, off);
                    s.regs.insert((DSIM, off), cur & !val);
                    return;
                }
                D_PLLCTRL => {
                    let was = Model::dsim_reg(&s, off);
                    if val & D_PLLEN == 0 {
                        s.dsim_pll_reads_left = None;
                    } else if was & D_PLLEN == 0 {
                        s.dsim_pll_reads_left = Some(3);
                    }
                }
                D_FIFOCTRL => {
                    let was = Model::dsim_reg(&s, off);
                    if was & 0x1f == 0 && val & 0x1f == 0x1f {
                        s.dsim_fifo_inits += 1;
                    }
                    // the status bits 25:8 are read only
                    s.regs.insert((DSIM, off), (was & !0x1f) | (val & 0x1f));
                    return;
                }
                _ => {}
            }
        }
        if blk == LCDIF1 && off < 0x10 {
            // CTRL and its SET / CLR / TOG aliases
            let reg = s.regs.entry((LCDIF1, 0)).or_insert(0x8000_0000);
            match off {
                0 => *reg = val,
                4 => *reg |= val,
                8 => *reg &= !val,
                _ => *reg ^= val,
            }
            return;
        }
        if blk == LCDIF1 && off == L_INT_STATUS_D0 {
            *s.regs.entry((LCDIF1, off)).or_insert(0) &= !val;
            return;
        }
        if blk == GC {
            Model::check_gc(&s, "write", off);
        }
        if blk == PWM && off == PWMSAR {
            // A sample goes into the four-word FIFO; a fifth sets FWE.
            if s.pwm_fifo < 4 {
                s.pwm_fifo += 1;
            } else {
                *s.regs.entry((PWM, PWMSR)).or_insert(0) |= 1 << 6;
            }
        }
        if blk == PWM && off == PWMCR && val & SWR != 0 {
            s.swr_reads_left = 1;
        }
        if blk == CCM && off >= 0x4000 && off & 0xf != 0 {
            // SET / CLR / TOG aliases (5.1.6.1)
            let reg = s.regs.entry((CCM, off & !0xf)).or_insert(0);
            match off & 0xf {
                4 => *reg |= val,
                8 => *reg &= !val,
                0xc => *reg ^= val,
                _ => panic!("CCM alias {off:#x}"),
            }
            return;
        }
        if blk == GPC && off == GPC_PUP {
            s.pup_pending |= val;
            s.pup_reads_left = 1;
        }
        if blk == GPC && off == GPC_PDN {
            s.pdn_pending |= val;
            s.pdn_reads_left = 1;
        }
        if blk == GPC && (0xb0c..0x1000).contains(&off) && off & 0x3f == 0xc {
            // PU_SR: PSR is write-1-to-clear, the rest read-only for us
            let cur = s.regs.get(&(GPC, off)).copied().unwrap_or(0);
            s.regs.insert((GPC, off), cur & !(val & 1));
            return;
        }
        if blk == ANATOP && off == VIDEO_PLL1_GEN {
            let was = s.regs.get(&(ANATOP, off)).copied().unwrap_or(0);
            if val & PLL_RST != 0 && was & PLL_RST == 0 {
                s.lock_reads_left = 2;
            }
            s.regs.insert((ANATOP, off), val & !PLL_LOCK);
            return;
        }
        s.regs.insert((blk, off), val);
    }

    // Every block modelled here has 32-bit registers; a byte access is a
    // driver bug. The 8-bit HDMI TX controller is model_hdmi.rs's.
    fn read8(&self, addr: usize) -> u8 {
        panic!("8-bit read at {addr:#x} of a 32-bit block");
    }

    fn write8(&self, addr: usize, _val: u8) {
        panic!("8-bit write at {addr:#x} of a 32-bit block");
    }
}

/// The OS side of a test: waits add up, log lines are kept.
#[derive(Default)]
pub struct TestEnv {
    pub waited_us: RefCell<u64>,
    pub lines: RefCell<Vec<(Level, String)>>,
}

impl Env for TestEnv {
    fn busy_wait_us(&self, us: u32) {
        *self.waited_us.borrow_mut() += u64::from(us);
    }
    fn sleep_ms(&self, ms: u32) {
        *self.waited_us.borrow_mut() += u64::from(ms) * 1000;
    }
    fn log_str(&self, level: Level, line: &str) {
        self.lines.borrow_mut().push((level, line.to_string()));
    }
    fn i2c_write(&self, _bus: u32, _buf: &[u8]) -> Result<(), i32> {
        Err(-crate::errno::ENOTSUP)
    }
    fn i2c_write_read(&self, _bus: u32, _w: &[u8], _r: &mut [u8]) -> Result<(), i32> {
        Err(-crate::errno::ENOTSUP)
    }
    fn gpio_set(&self, _line: u32, _value: bool) -> Result<(), i32> {
        Err(-crate::errno::ENOTSUP)
    }
}

// ---- the SN65DSI8x on I2C ------------------------------------------------

/// The SN65DSI8x SOFT_RESET register (SLLSEC2H Table 7-6, 0x09), for a test
/// that watches the end of the bridge's programming.
pub const BRIDGE_SOFT_RESET: u8 = 0x09;

/// What [`BridgeModel`] saw, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeEvent {
    /// A GPIO line set (line, logical value).
    Gpio(u32, bool),
    /// `sleep_ms`.
    Sleep(u32),
    /// `busy_wait_us`.
    Wait(u32),
    /// One byte written to a register.
    Write(u8, u8),
    /// A read starting at a register, and the first byte it returned.
    Read(u8, u8),
}

/// Injected failures.
#[derive(Default, Clone, Copy)]
pub struct BridgeFaults {
    /// I2C transactions NACKed after EN rises (the adapter powering up);
    /// `u32::MAX` never answers.
    pub nack_reads_after_en: u32,
    /// The ID bytes are some other part's.
    pub foreign_id: bool,
    /// PLL_EN_STAT never sets.
    pub pll_never_starts: bool,
    /// Writes to this register are lost.
    pub stuck_reg: Option<u8>,
}

/// The reset image of the CSRs, from the data sheet's DEFAULT columns
/// (SLLSEC2H Tables 7-5..7-10), restated here rather than shared with the
/// driver. Bits the data sheet does not describe are 0, except 0x10 bits 2:1:
/// those read 1 1 on the board (golden dump 0x10 = 0x26) and the data sheet's
/// example script (8.2.2.1) writes them as 1 1, so the model resets them so.
fn bridge_reset_image(foreign_id: bool) -> Vec<u8> {
    let mut r = vec![0u8; 256];
    let id: [u8; 9] = if foreign_id { *b"ADV7535\0\0" } else { [0x35, 0x38, 0x49, 0x53, 0x44, 0x20, 0x20, 0x20, 0x01] };
    r[..9].copy_from_slice(&id);
    r[0x0a] = 0b101 << 1; // LVDS_CLK_RANGE 101
    r[0x10] = (0b01 << 5) | (0b11 << 3) | (0b11 << 1); // reserved 01, one lane, 2:1 as above
    r[0x18] = (1 << 6) | (1 << 5) | (1 << 4); // HS/VS negative, single link
    r[0x19] = (0b01 << 2) | 0b01; // VOD swing 01 on A and B
    r[0x1a] = 0b11; // 200 ohm on A and B
    r[0xe5] = 1; // PLL_UNLOCK
    r
}

#[derive(Default)]
struct BridgeState {
    regs: Vec<u8>,
    en: bool,
    nack_left: u32,
    pll_reads_left: u32,
    soft_resets: u32,
    irq_source: u8,
    events: Vec<BridgeEvent>,
    lines: Vec<String>,
    faults: BridgeFaults,
}

/// An SN65DSI8x behind I2C bus 0, its EN on line 0, as an [`Env`]. While EN
/// is low the part does not answer (7.5.1: "The SN65DSI84 local I2C interface
/// is enabled when EN is input high"); EN rising resets every CSR (7.4.1).
pub struct BridgeModel(RefCell<BridgeState>);

impl BridgeModel {
    pub fn new() -> BridgeModel {
        BridgeModel::with_faults(BridgeFaults::default())
    }

    pub fn with_faults(f: BridgeFaults) -> BridgeModel {
        BridgeModel(RefCell::new(BridgeState {
            regs: bridge_reset_image(f.foreign_id),
            faults: f,
            ..Default::default()
        }))
    }

    pub fn events(&self) -> Vec<BridgeEvent> {
        self.0.borrow().events.clone()
    }
    pub fn clear_events(&self) {
        self.0.borrow_mut().events.clear();
    }
    pub fn lines(&self) -> Vec<String> {
        self.0.borrow().lines.clone()
    }
    pub fn reg(&self, r: u8) -> u8 {
        self.0.borrow().regs[r as usize]
    }
    /// All 256 registers, as an i2cdump would read them.
    pub fn image(&self) -> [u8; 256] {
        let s = self.0.borrow();
        let mut a = [0u8; 256];
        a.copy_from_slice(&s.regs);
        a[0xe5] |= s.irq_source;
        a
    }
    pub fn soft_resets(&self) -> u32 {
        self.0.borrow().soft_resets
    }
    /// Latch error bits in 0xE5 once (they clear on a write of 1).
    pub fn latch_irq(&self, bits: u8) {
        self.0.borrow_mut().regs[0xe5] |= bits;
    }
    /// An error that keeps being detected: it reads set whatever is cleared.
    pub fn set_irq_source(&self, bits: u8) {
        self.0.borrow_mut().irq_source = bits;
    }

    fn answer(s: &mut BridgeState) -> Result<(), i32> {
        if !s.en {
            return Err(-crate::errno::EIO);
        }
        if s.nack_left > 0 {
            if s.nack_left != u32::MAX {
                s.nack_left -= 1;
            }
            return Err(-crate::errno::EIO);
        }
        Ok(())
    }

    fn read_reg(s: &mut BridgeState, r: u8) -> u8 {
        match r {
            0x09 => 0, // SOFT_RESET is write only; reads 0 (Table 7-6)
            0x0a => {
                if s.regs[0x0d] & 1 != 0 && !s.faults.pll_never_starts {
                    if s.pll_reads_left == 0 {
                        s.regs[0x0a] |= 0x80;
                    } else {
                        s.pll_reads_left -= 1;
                    }
                }
                s.regs[0x0a]
            }
            0xe5 => s.regs[0xe5] | s.irq_source,
            _ => s.regs[r as usize],
        }
    }

    fn write_reg(s: &mut BridgeState, r: u8, v: u8) {
        s.events.push(BridgeEvent::Write(r, v));
        if s.faults.stuck_reg == Some(r) {
            return;
        }
        match r {
            0x00..=0x08 => {} // RO
            0x09 => {
                if v & 1 != 0 {
                    s.soft_resets += 1;
                }
            }
            0x0a => s.regs[0x0a] = (s.regs[0x0a] & 0x80) | (v & 0x7f), // PLL_EN_STAT is RO
            0x0d => {
                s.regs[0x0d] = v;
                if v & 1 == 0 {
                    s.regs[0x0a] &= !0x80;
                } else {
                    s.pll_reads_left = 2;
                }
            }
            0xe5 => s.regs[0xe5] &= !v, // RW1C
            _ => s.regs[r as usize] = v,
        }
    }
}

impl Default for BridgeModel {
    fn default() -> Self {
        BridgeModel::new()
    }
}

impl Env for BridgeModel {
    fn busy_wait_us(&self, us: u32) {
        self.0.borrow_mut().events.push(BridgeEvent::Wait(us));
    }
    fn sleep_ms(&self, ms: u32) {
        self.0.borrow_mut().events.push(BridgeEvent::Sleep(ms));
    }
    fn log_str(&self, _level: Level, line: &str) {
        self.0.borrow_mut().lines.push(line.to_string());
    }
    fn i2c_write(&self, bus: u32, buf: &[u8]) -> Result<(), i32> {
        assert_eq!(bus, 0);
        let mut s = self.0.borrow_mut();
        Self::answer(&mut s)?;
        let (reg, data) = buf.split_first().expect("a register address");
        for (i, &v) in data.iter().enumerate() {
            Self::write_reg(&mut s, reg.wrapping_add(i as u8), v);
        }
        Ok(())
    }
    fn i2c_write_read(&self, bus: u32, wbuf: &[u8], rbuf: &mut [u8]) -> Result<(), i32> {
        assert_eq!(bus, 0);
        assert_eq!(wbuf.len(), 1, "one sub-address byte");
        let mut s = self.0.borrow_mut();
        Self::answer(&mut s)?;
        for (i, b) in rbuf.iter_mut().enumerate() {
            *b = Self::read_reg(&mut s, wbuf[0].wrapping_add(i as u8));
        }
        let first = rbuf.first().copied().unwrap_or(0);
        s.events.push(BridgeEvent::Read(wbuf[0], first));
        Ok(())
    }
    fn gpio_set(&self, line: u32, value: bool) -> Result<(), i32> {
        let mut s = self.0.borrow_mut();
        s.events.push(BridgeEvent::Gpio(line, value));
        if line == 0 {
            if value && !s.en {
                s.regs = bridge_reset_image(s.faults.foreign_id);
                s.nack_left = s.faults.nack_reads_after_en;
                s.soft_resets = 0;
            }
            s.en = value;
        }
        Ok(())
    }
}

/// The bridge registers of the i.MX8MP board under Linux with the panel on:
/// the "== SN65DSI83 i2c-1 0x2c" i2cdump in
/// driver/src/display/golden-linux-2026-09-21.txt, parsed from the file.
pub fn golden_imx8mp_bridge() -> [u8; 256] {
    const DUMP: &str = include_str!("../../src/display/golden-linux-2026-09-21.txt");
    let mut lines = DUMP.lines().skip_while(|l| !l.starts_with("== SN65DSI83 i2c-1 0x2c")).skip(2);
    let mut img = [0u8; 256];
    for row in 0..16 {
        let l = lines.next().expect("16 dump rows");
        let (addr, rest) = l.split_once(':').expect("row address");
        assert_eq!(usize::from_str_radix(addr.trim(), 16).unwrap(), row * 16, "{l}");
        for (i, h) in rest.split_whitespace().take(16).enumerate() {
            img[row * 16 + i] = u8::from_str_radix(h, 16).expect("hex byte");
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ylog;

    #[test]
    fn long_log_lines_are_cut_with_a_marker() {
        let e = TestEnv::default();
        let long = "é".repeat(200);
        ylog!(&e, Inf, "{long}");
        let l = &e.lines.borrow()[0].1;
        assert!(l.ends_with("...") && l.len() <= crate::env::LINE_MAX);
        assert!(l.trim_end_matches("...").chars().all(|c| c == 'é'));
    }
}
