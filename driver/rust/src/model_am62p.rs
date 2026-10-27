// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host tests' model of the AM62P display path (tests only): DSS1
//! (common, VID, VIDL1, one overlay, one video port), the DSI host
//! controller, its wrapper and the D-PHY TX, at fake addresses.
//!
//! Like [`crate::model::Model`] it records every access in order and answers
//! only what the drivers poll, as far as the AM62Px TRM (SPRUJ83D) says the
//! hardware does. Offsets and reset values are restated here from the TRM,
//! not shared with the drivers, so a wrong constant cannot agree with itself.
//!
//! What is modelled beyond plain storage:
//! - Reset values the drivers read (14.8.9.1.2: DSS_REVISION 64040901h,
//!   SYSCONFIG 11h, SYSSTATUS 207h, BUF_SIZE_STATUS A00h, BUF_THRESHOLD
//!   09FF09F8h, PRELOAD 100h, PIXEL_INC / ROW_INC 1, GLOBAL_ALPHA FFh,
//!   VP_CONTROL 40h, DISPC_GLOBAL_BUFFER 4688h, ATTRIBUTES2 3C000000h;
//!   14.8.9.2.2: IP_CONF 948490D0h, ID_REG CADD5131h, MCTL_DPHY_CFG0 600h,
//!   VID_MAIN_CTL 80000000h, MCTL_3DVIDEO_CTL 10h, CMD_MODE_CTL2 7FF800h,
//!   DPI_CFG 8000000h, the wrapper's DSI_REVISION 640A1101h; 14.8.6.3.2: MOD_VER
//!   69924001h, PLL_CTRL 10FF0101h, PSM_FREQ 1).
//! - DSS SOFTRESET: reads 1 and RESETDONE reads 0 for a few reads, then
//!   every DSS register is back at reset and SOFTRESET clears (12.9.1.4.1.3).
//! - Shadow registers and GO (12.9.1.4.1.13): with the port enabled, writes
//!   go to the shadow copy; [`Am62pModel::vsync`] (a frame end) copies the
//!   shadow to the active copy if GO is set, clears GO, and raises VSYNC
//!   (and the GO-clear event, VPSYNC_IRQ); with the port off, writes take
//!   effect at once and there are no frame ends.
//! - DSS interrupt status bits are write-1-to-clear; DISPC_IRQSTATUS shows a
//!   pipeline's or port's bit while an enabled event of it is pending.
//! - D-PHY: PLL_LOCK reads 1 a few reads after the start state machines are
//!   enabled with the common reset released and the PLL powered (PLL_PD 0);
//!   O_CMN_READY follows the lock. The WIZ registers sit at 0xF00 (where
//!   TI's driver has them and the board's previous driver ran), or at 0
//!   (the TRM's summary) with [`Am62pFaults::wiz_at_zero`].
//! - DSI MCTL_MAIN_STS: PLL_LCK while PLL_START is set, the D-PHY PLL is
//!   locked and MCTL_DPHY_CFG0 has the PLL's PDN and PSO low; CLKLANE_READY /
//!   DATn_READY while PLL_LCK, the lane's reset is released (C_RSTB / D_RSTB
//!   bit n) and the lane is started (CKLANE_EN / DATn_EN).
//! - DIRECT_CMD_SEND sets WRITE_COMPLETED in DIRECT_CMD_STS; DIRECT_CMD_STS_CLR
//!   clears what it is written.

use crate::mmio::Mmio;
use std::cell::RefCell;
use std::collections::BTreeMap;

pub const COMMON: &str = "dss-common";
pub const VID: &str = "dss-vid";
pub const VIDL1: &str = "dss-vidl1";
pub const OVR: &str = "dss-ovr";
pub const VP: &str = "dss-vp";
pub const DSI: &str = "dsi";
pub const WRAP: &str = "dsi-wrap";
pub const DPHY: &str = "dphy";

const BLOCKS: [(&str, usize, usize); 8] = [
    (COMMON, 0x1000_0000, 0x1000),
    (VID, 0x1100_0000, 0x1000),
    (VIDL1, 0x1200_0000, 0x1000),
    (OVR, 0x1300_0000, 0x1000),
    (VP, 0x1400_0000, 0x1000),
    (DSI, 0x2000_0000, 0x1000),
    (WRAP, 0x2100_0000, 0x100),
    (DPHY, 0x2200_0000, 0x1000),
];

/// Blocks whose registers are shadowed behind GO (the VID/VIDL1 pipelines,
/// the overlay and the port's timings; common is not).
const SHADOWED: [&str; 4] = [VID, VIDL1, OVR, VP];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    R(&'static str, u32, u32),
    W(&'static str, u32, u32),
}

/// Injected failures.
#[derive(Default, Clone, Copy, Debug)]
pub struct Am62pFaults {
    /// SYSSTATUS never shows the power-on reset done.
    pub dss_por_not_done: bool,
    /// A soft reset never completes.
    pub dss_reset_stuck: bool,
    /// GO is never consumed at a frame end.
    pub go_stuck: bool,
    /// The D-PHY PLL never locks.
    pub dphy_no_lock: bool,
    /// O_CMN_READY never sets.
    pub dphy_cmn_not_ready: bool,
    /// The WIZ registers at offset 0 instead of 0xF00.
    pub wiz_at_zero: bool,
    /// No WIZ at all (MOD_VER reads 0 everywhere).
    pub no_wiz: bool,
    /// The data lanes never become ready.
    pub lanes_stuck: bool,
    /// ID_REG is something else.
    pub foreign_dsi: bool,
    /// A direct command never completes.
    pub cmd_stuck: bool,
}

// DSS offsets (14.8.9.1.2).
const SYSCONFIG: u32 = 0x8;
const SYSSTATUS: u32 = 0x20;
const IRQSTATUS: u32 = 0x2c;
const IRQENABLE_SET: u32 = 0x30;
const IRQENABLE_CLR: u32 = 0x40;
const VID_IRQENABLE: [u32; 2] = [0x44, 0x48];
const VID_IRQSTATUS: [u32; 2] = [0x58, 0x5c];
const VP_IRQENABLE: [u32; 2] = [0x70, 0x74];
const VP_IRQSTATUS: [u32; 2] = [0x7c, 0x80];
const VP_CONTROL: u32 = 0x4;
const ENABLE: u32 = 1;
const GOBIT: u32 = 1 << 5;
const VSYNC: u32 = 1 << 1;
const GO_CLEAR: u32 = 1 << 11;
const SYNCLOST: u32 = 1 << 4;
const UNDERFLOW: u32 = 1;
// DSI (14.8.9.2.2).
const MAIN_EN: u32 = 0xc;
const DPHY_CFG0: u32 = 0x10;
const MAIN_STS: u32 = 0x24;
const CMD_SEND: u32 = 0x80;
const CMD_STS: u32 = 0x88;
const CMD_STS_CLR: u32 = 0x158;
const ID_REG: u32 = 0x1fc;
// D-PHY (14.8.6.3.2), WIZ offsets relative to the wrapper.
const TBIT2: u32 = 0x20;
const W_MOD_VER: u32 = 0x0;
const W_PLL_CTRL: u32 = 0x4;
const W_STATUS: u32 = 0x8;
const W_RST_CTRL: u32 = 0xc;
const W_PSM: u32 = 0x10;
const PLL_LOCK: u32 = 1 << 31;

fn reset_image(block: &'static str, wiz: u32) -> Vec<(u32, u32)> {
    match block {
        COMMON => vec![(0x4, 0x6404_0901), (SYSCONFIG, 0x11), (SYSSTATUS, 0x207), (0x98, 0x4688), (0x9c, 0xc)],
        VID | VIDL1 => vec![
            (0x24, 0x3c00_0000),
            (0x38, 0xa00),
            (0x3c, 0x09ff_09f8),
            (0x1fc, 0xff),
            (0x210, 1),
            (0x218, 0x100),
            (0x21c, 1),
        ],
        VP => vec![(VP_CONTROL, 0x40)],
        DSI => vec![
            (0x0, 0x9484_90d0),
            (DPHY_CFG0, 0x600),
            (0x20, 0x10),
            (0x74, 0x007f_f800),
            (0xb0, 0x8000_0000),
            (0x1ac, 0x0800_0000),
            (ID_REG, 0xcadd_5131),
        ],
        WRAP => vec![(0x0, 0x640a_1101)],
        DPHY => vec![
            (TBIT2, 0x28),
            (0x40, 0x0080_0000),
            (wiz + W_MOD_VER, 0x6992_4001),
            (wiz + W_PLL_CTRL, 0x10ff_0101),
            (wiz + W_PSM, 1),
            (wiz + 0x14, 1),
        ],
        _ => vec![],
    }
}

#[derive(Default)]
struct State {
    regs: BTreeMap<(&'static str, u32), u32>,
    /// What the hardware uses of the shadowed blocks (see the module text).
    active: BTreeMap<(&'static str, u32), u32>,
    log: Vec<Access>,
    faults: Am62pFaults,
    wiz: u32,
    reset_reads_left: u32,
    lock_reads_left: u32,
    frames: u32,
    port: usize,
}

/// The model. See the module text.
pub struct Am62pModel(RefCell<State>);

impl Am62pModel {
    pub fn new() -> Am62pModel {
        Am62pModel::with_faults(Am62pFaults::default())
    }

    pub fn with_faults(f: Am62pFaults) -> Am62pModel {
        let wiz = if f.wiz_at_zero { 0 } else { 0xf00 };
        let mut s = State { faults: f, wiz, ..Default::default() };
        for (b, _, _) in BLOCKS {
            for (o, v) in reset_image(b, wiz) {
                s.regs.insert((b, o), v);
            }
        }
        if f.no_wiz {
            s.regs.remove(&(DPHY, wiz + W_MOD_VER));
        }
        if f.foreign_dsi {
            s.regs.insert((DSI, ID_REG), 0x1234_5678);
        }
        Am62pModel(RefCell::new(s))
    }

    pub fn base(&self, block: &str) -> usize {
        BLOCKS.iter().find(|b| b.0 == block).expect("block").1
    }

    pub fn poke(&self, block: &'static str, off: u32, val: u32) {
        self.0.borrow_mut().regs.insert((block, off), val);
    }

    /// The register as written (the shadow copy of a shadowed block).
    pub fn peek(&self, block: &'static str, off: u32) -> u32 {
        let s = self.0.borrow();
        s.regs.get(&(block, off)).copied().unwrap_or(0)
    }

    /// What the hardware uses: the active copy of a shadowed block while
    /// the port runs, otherwise the register.
    pub fn active(&self, block: &'static str, off: u32) -> u32 {
        let s = self.0.borrow();
        if SHADOWED.contains(&block) && Self::port_on(&s) {
            s.active.get(&(block, off)).copied().unwrap_or(0)
        } else {
            s.regs.get(&(block, off)).copied().unwrap_or(0)
        }
    }

    pub fn wiz(&self) -> u32 {
        self.0.borrow().wiz
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

    /// Frame ends since the port was enabled.
    pub fn frames(&self) -> u32 {
        self.0.borrow().frames
    }

    fn port_on(s: &State) -> bool {
        s.regs.get(&(VP, VP_CONTROL)).copied().unwrap_or(0) & ENABLE != 0
    }

    fn snapshot(s: &mut State) {
        let shadow: Vec<_> = s.regs.iter().filter(|(k, _)| SHADOWED.contains(&k.0)).map(|(k, v)| (*k, *v)).collect();
        s.active.clear();
        s.active.extend(shadow);
    }

    /// A frame end of the running port (nothing while it is off).
    pub fn vsync(&self) {
        let mut s = self.0.borrow_mut();
        if !Self::port_on(&s) {
            return;
        }
        s.frames += 1;
        let ctl = s.regs[&(VP, VP_CONTROL)];
        let mut ev = VSYNC;
        if ctl & GOBIT != 0 && !s.faults.go_stuck {
            Self::snapshot(&mut s);
            s.regs.insert((VP, VP_CONTROL), ctl & !GOBIT);
            ev |= GO_CLEAR;
        }
        let port = s.port;
        *s.regs.entry((COMMON, VP_IRQSTATUS[port])).or_insert(0) |= ev;
    }

    /// Which port the modelled VP block is (0 = VP1): its events go to
    /// VP_IRQSTATUS_<port>.
    pub fn set_port(&self, port: usize) {
        self.0.borrow_mut().port = port;
    }

    pub fn sync_lost(&self) {
        let mut s = self.0.borrow_mut();
        let port = s.port;
        *s.regs.entry((COMMON, VP_IRQSTATUS[port])).or_insert(0) |= SYNCLOST;
    }

    /// A buffer underflow of pipeline `pipe` (0 VID, 1 VIDL1).
    pub fn underflow(&self, pipe: usize) {
        *self.0.borrow_mut().regs.entry((COMMON, VID_IRQSTATUS[pipe])).or_insert(0) |= UNDERFLOW;
    }

    fn reg(s: &State, block: &'static str, off: u32) -> u32 {
        s.regs.get(&(block, off)).copied().unwrap_or(0)
    }

    fn dphy_locked(s: &State) -> bool {
        let w = s.wiz;
        let ssm = Self::reg(s, DPHY, TBIT2);
        let started = ssm & 1 != 0 && ssm & (1 << 9) != 0;
        let released = Self::reg(s, DPHY, w + W_RST_CTRL) & (1 << 31) != 0;
        let powered = Self::reg(s, DPHY, w + W_PLL_CTRL) & (1 << 28) == 0;
        started && released && powered && !s.faults.dphy_no_lock
    }

    fn value(s: &State, block: &'static str, off: u32) -> u32 {
        let raw = Self::reg(s, block, off);
        let w = s.wiz;
        match (block, off) {
            (COMMON, SYSSTATUS) if s.faults.dss_por_not_done => 0,
            (COMMON, SYSSTATUS) if s.reset_reads_left > 0 => raw & !0x7,
            (COMMON, IRQSTATUS) => {
                let mut top = 0;
                for i in 0..2 {
                    if Self::reg(s, COMMON, VP_IRQSTATUS[i]) & Self::reg(s, COMMON, VP_IRQENABLE[i]) != 0 {
                        top |= 1 << i;
                    }
                    if Self::reg(s, COMMON, VID_IRQSTATUS[i]) & Self::reg(s, COMMON, VID_IRQENABLE[i]) != 0 {
                        top |= 1 << (4 + i);
                    }
                }
                top & Self::reg(s, COMMON, IRQENABLE_SET)
            }
            // "Read-1: Interrupt Enabled" (14.8.9.1.2.8): both read the enables.
            (COMMON, IRQENABLE_CLR) => Self::reg(s, COMMON, IRQENABLE_SET),
            (DPHY, o) if o == w + W_PLL_CTRL => {
                let lock = Self::dphy_locked(s) && s.lock_reads_left == 0;
                (raw & !PLL_LOCK) | if lock { PLL_LOCK } else { 0 }
            }
            (DPHY, o) if o == w + W_STATUS => {
                let ready = Self::dphy_locked(s) && s.lock_reads_left == 0 && !s.faults.dphy_cmn_not_ready;
                if ready {
                    1 << 31
                } else {
                    0
                }
            }
            (DSI, MAIN_STS) => {
                let en = Self::reg(s, DSI, MAIN_EN);
                let cfg0 = Self::reg(s, DSI, DPHY_CFG0);
                let pll = en & 1 != 0 && Self::dphy_locked(s) && cfg0 & ((1 << 10) | (1 << 1)) == 0;
                if !pll {
                    return 0;
                }
                let mut v = 1;
                if cfg0 & (1 << 20) != 0 && en & (1 << 3) != 0 {
                    v |= 1 << 1;
                }
                for lane in 0..4 {
                    let rstb = cfg0 & (1 << (16 + lane)) != 0;
                    let started = en & (1 << (4 + lane)) != 0;
                    if rstb && started && !s.faults.lanes_stuck {
                        v |= 1 << (2 + lane);
                    }
                }
                v
            }
            _ => raw,
        }
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
}

impl Default for Am62pModel {
    fn default() -> Self {
        Am62pModel::new()
    }
}

impl Mmio for Am62pModel {
    fn read32(&self, addr: usize) -> u32 {
        let (blk, off) = Self::decode(addr);
        let mut s = self.0.borrow_mut();
        let v = Self::value(&s, blk, off);
        if blk == COMMON
            && (off == SYSCONFIG || off == SYSSTATUS)
            && s.reset_reads_left > 0
            && !s.faults.dss_reset_stuck
        {
            s.reset_reads_left -= 1;
            if s.reset_reads_left == 0 {
                // The reset is over: every DSS register back at reset.
                s.regs.retain(|k, _| ![COMMON, VID, VIDL1, OVR, VP].contains(&k.0));
                let wiz = s.wiz;
                for b in [COMMON, VID, VIDL1, OVR, VP] {
                    for (o, val) in reset_image(b, wiz) {
                        s.regs.insert((b, o), val);
                    }
                }
            }
        }
        if blk == DPHY && off == s.wiz + W_PLL_CTRL && Self::dphy_locked(&s) && s.lock_reads_left > 0 {
            s.lock_reads_left -= 1;
        }
        s.log.push(Access::R(blk, off, v));
        v
    }

    fn write32(&self, addr: usize, val: u32) {
        let (blk, off) = Self::decode(addr);
        let mut s = self.0.borrow_mut();
        s.log.push(Access::W(blk, off, val));
        let w = s.wiz;
        match (blk, off) {
            (COMMON, SYSCONFIG) => {
                if val & 2 != 0 {
                    s.reset_reads_left = 3;
                }
                s.regs.insert((COMMON, SYSCONFIG), val);
                if s.faults.dss_reset_stuck {
                    s.reset_reads_left = u32::MAX;
                }
            }
            (COMMON, o) if VP_IRQSTATUS.contains(&o) || VID_IRQSTATUS.contains(&o) => {
                let cur = Self::reg(&s, COMMON, o);
                s.regs.insert((COMMON, o), cur & !val);
            }
            (COMMON, IRQSTATUS) => {} // the first level follows the second
            (COMMON, IRQENABLE_SET) => {
                *s.regs.entry((COMMON, IRQENABLE_SET)).or_insert(0) |= val;
            }
            (COMMON, IRQENABLE_CLR) => {
                *s.regs.entry((COMMON, IRQENABLE_SET)).or_insert(0) &= !val;
            }
            (VP, VP_CONTROL) => {
                let was = Self::reg(&s, VP, VP_CONTROL);
                s.regs.insert((VP, VP_CONTROL), val);
                if val & ENABLE != 0 && was & ENABLE == 0 {
                    // Enabled: what was written takes effect; GO not needed.
                    let v = val & !GOBIT;
                    s.regs.insert((VP, VP_CONTROL), v);
                    Self::snapshot(&mut s);
                    s.frames = 0;
                }
            }
            (DPHY, o) if o == w + W_PLL_CTRL => {
                s.regs.insert((DPHY, o), val & !PLL_LOCK);
            }
            (DPHY, TBIT2) => {
                s.regs.insert((DPHY, TBIT2), val);
                s.lock_reads_left = 2;
            }
            (DPHY, o) if o == w + W_MOD_VER || o == w + W_STATUS => {} // read only
            (DSI, CMD_SEND) => {
                if !s.faults.cmd_stuck {
                    *s.regs.entry((DSI, CMD_STS)).or_insert(0) |= 1 << 1;
                }
            }
            (DSI, CMD_STS_CLR) => {
                let cur = Self::reg(&s, DSI, CMD_STS);
                s.regs.insert((DSI, CMD_STS), cur & !val);
            }
            (DSI, MAIN_STS) | (DSI, ID_REG) | (DSI, 0x0) => {} // read only
            _ => {
                s.regs.insert((blk, off), val);
            }
        }
    }

    fn read8(&self, addr: usize) -> u8 {
        panic!("8-bit read at {addr:#x} of a 32-bit block");
    }

    fn write8(&self, addr: usize, _val: u8) {
        panic!("8-bit write at {addr:#x} of a 32-bit block");
    }
}
