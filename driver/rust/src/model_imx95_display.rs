// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host tests' model of the i.MX 95 display block (tests only), at the
//! real addresses: the display CSR (0x4B010000), the LVDS CSR (0x4B0C0000,
//! "ldb"), the pixel interleaver (0x4B0D0000) and the DPU (0x4B400000),
//! joined to the mock System Manager (`soc::imx95::sm_mock`) for the power
//! domain and the pixel clock.
//!
//! As the other display models do, it restates every offset and bit from
//! the design note (driver/docs/imx95-display.md) and NXP's headers, never
//! from the code under test, so a wrong constant cannot agree with itself;
//! it records every access in order, answers the polls and refuses what
//! the silicon would not take:
//!
//! - Any access with the DISPLAY domain (13) off is refused (logged, a
//!   write dropped, a read 0): what such a read does on the board is the
//!   note's U6, the code must not try.
//! - A read of a write-only register or of a reserved word refuses (logged
//!   in `refused`, reads 0): the seven words whose single read killed the
//!   reading process on the board under Linux (2026-11-25; the header's
//!   `__O` registers and `RESERVED` holes: FetchLayer0 +0x14 and +0x4C,
//!   FetchYUV0 +0x24, ExtDst0 SOFTWAREKICK, FrameGen0 FGSLR and
//!   FGCHSTATCLR, Signature0 SOFTWAREKICK), [`FAULTS`].
//! - FrameGen0 runs (`FGENSTS.EnSts`) while `FGENABLE.FgEn` is set and its
//!   pixel clock arrives: `DSIP_CLK0_SEL` = 0 (CCM) with DISP1PIX (80)
//!   enabled at a non-zero rate, or `DSIP_CLK0_SEL` = 2 (LDB PLL / 7) with
//!   LDBPLL (38) enabled at a non-zero rate. Select 1 (the DSI PLL) gives no
//!   clock here.
//! - Each read of `FGTIMESTAMP`, `FGCHSTAT`, `IRQ_INTERRUPTSTATUS0` or
//!   `SIG0 STATUS` while running is a frame: the frame index moves, a
//!   pending ExtDst0 shadow load is taken (the fetch address written
//!   becomes the one scanned out, `IRQ_INTERRUPTSTATUS0` bit 3 sets) and a
//!   requested Signature0 window loads.
//! - `FGCHSTAT.PrimSyncStat` sets `lag` frames after the start when the
//!   content path is complete: ExtDst0's domain mask has FetchLayer0,
//!   LayerBlend1 and ConstFrame0; its pipeline is powered, sources
//!   LayerBlend1 and is kicked externally; LayerBlend1 blends ConstFrame0
//!   (0x0C) and FetchLayer0 (0x1A) with its clock on; FetchLayer0's
//!   sublayer 0 is enabled; a pipeline shadow load was taken; FrameGen0
//!   has a timing and a kick; the display mode shows the primary input.
//! - FrameGen0's sequence complete (group 0 bit 20) sets in the display
//!   engine 0 interrupt block (`DISPIRQ0_INTERRUPTSTATUS0`, DPU + 0x38102C;
//!   `DISPIRQ0_INTERRUPTCLEAR0` 0x381020 clears it), where NXP's
//!   `dpu95-core.c` takes it, and never in the common-control
//!   `IRQ_INTERRUPTSTATUS0` (DPU + 0x102C): the board polled that word for
//!   1 s after `FGENABLE` = 0 and never saw it (vi95-down-board-check,
//!   after 211ed16a). It sets once `FGENABLE` was cleared while FrameGen0
//!   ran, on the [`Display::seq_after`]-th read of `DISPIRQ0_INTERRUPTSTATUS0`
//!   that follows while the pixel clock arrives (a frame end needs it).
//!   `DISPIRQ0_INTERRUPTENABLE0` is a plain word; the status latches
//!   without it (Linux's handler masks the status with it).
//! - DISPIRQ0's write protection (`DISPIRQ0_LOCKUNLOCK`, DPU + 0x381000;
//!   `DISPIRQ0_LOCKSTATUS`, DPU + 0x381004; `PERI_DISPLAY_SEERIS.h`): an unlock
//!   counter, 1 after reset; the lock key decrements it, the unlock key
//!   increments it (up to 15), the privilege keys set and clear
//!   `PrivilegeStatus`, the freeze key makes every later key a no-op. While
//!   the counter is 0 (`LockStatus` 1) the block's other words take no
//!   writes (logged, not applied); its status still latches and reads.
//! - Signature0 window 0's CRC is a function of the buffer scanned out
//!   (the content the test gives an address), the window and the mode: the
//!   model's stand-in for the hardware's polynomial (U7).

use crate::mmio::Mmio;
use crate::soc::imx95::sm_mock::Sm;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub const CSR: usize = 0x4B01_0000;
pub const LDB: usize = 0x4B0C_0000;
/// `LVDS_PHY_CLOCK_CONTROL` bits 1-4: all gated (`ldb::CLOCKS_ALL_GATED`).
const LDB_CLOCKS_GATED: u32 = 0x1E;
pub const IL: usize = 0x4B0D_0000;
pub const DPU: usize = 0x4B40_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    R(&'static str, u32, u32),
    W(&'static str, u32, u32),
}

/// DPU offsets the model gives a meaning (PERI_DISPLAY_SEERIS.h).
mod o {
    pub const IRQ_STATUS0: u32 = 0x102c;
    pub const IRQ_CLEAR0: u32 = 0x1020;
    pub const DIRQ0_LOCKUNLOCK: u32 = 0x38_1000;
    pub const DIRQ0_LOCKSTATUS: u32 = 0x38_1004;
    pub const DIRQ0_END: u32 = 0x38_1038;
    pub const DIRQ0_CLEAR0: u32 = 0x38_1020;
    pub const DIRQ0_STATUS0: u32 = 0x38_102c;
    pub const DM_EXTDST0: u32 = 0x200c;
    pub const ED0_STATICCONTROL: u32 = 0x11_0008;
    pub const ED0CFG_STATIC: u32 = 0x11_1008;
    pub const ED0CFG_DYNAMIC: u32 = 0x11_100c;
    pub const ED0CFG_TRIGGER: u32 = 0x11_1014;
    pub const LB1_CONTROL: u32 = 0x17_000c;
    pub const LB1CFG_DYNAMIC: u32 = 0x17_1008;
    pub const FL0_BASE0: u32 = 0x1d_0018;
    pub const FL0_BASEMSB0: u32 = 0x1d_001c;
    pub const FL0_LAYERPROPERTY0: u32 = 0x1d_0048;
    pub const FG_HTCFG1: u32 = 0x2b_000c;
    pub const FG_VTCFG1: u32 = 0x2b_0014;
    pub const FG_PKICK: u32 = 0x2b_002c;
    pub const FG_INCTRL: u32 = 0x2b_0080;
    pub const FG_ENABLE: u32 = 0x2b_008c;
    pub const FG_ENSTS: u32 = 0x2b_0094;
    pub const FG_TIMESTAMP: u32 = 0x2b_0098;
    pub const FG_CHSTAT: u32 = 0x2b_009c;
    pub const SIG_SHADOWLOAD: u32 = 0x2d_0018;
    pub const SIG_STATUS: u32 = 0x2d_0028;
    pub const SIG_CONTROL0: u32 = 0x2d_002c;
    pub const SIG_UL0: u32 = 0x2d_0030;
    pub const SIG_LR0: u32 = 0x2d_0034;
    pub const SIG_CRC_R0: u32 = 0x2d_0400;
    pub const SIG_CRC_G0: u32 = 0x2d_0404;
    pub const SIG_CRC_B0: u32 = 0x2d_0408;
}

/// The DPU words (offsets) a read of faults on the board (module text).
pub const FAULTS: [u32; 7] = [0x1d_0014, 0x1d_004c, 0x20_0024, 0x11_0010, 0x2b_0090, 0x2b_00a0, 0x2d_0020];

#[derive(Default, Clone, Copy)]
pub struct Faults {
    /// FrameGen0's primary channel never syncs.
    pub no_sync: bool,
    /// ExtDst0's shadow load never happens.
    pub no_shadow_load: bool,
    /// FrameGen0 keeps reading enabled after FGENABLE is cleared.
    pub fg_stuck: bool,
    /// As the hardware: after FGENABLE is cleared FGENSTS reads 1 until the
    /// running frame ends, and a frame ends only while the pixel clock
    /// arrives (for the LDB route: the PLL runs and the LDB's
    /// `LVDS_PHY_CLOCK_CONTROL` has not gated its clocks).
    pub frame_end_needed: bool,
}

/// `DISPIRQ0_LOCKUNLOCK` keys (`PERI_DISPLAY_SEERIS.h`).
pub mod key {
    pub const LOCK: u32 = 0x5651_F763;
    pub const UNLOCK: u32 = 0x691D_B936;
    pub const PRIVILEGE_ON: u32 = 0xAEE9_5CDC;
    pub const PRIVILEGE_OFF: u32 = 0xB5E2_466E;
    pub const FREEZE: u32 = 0xFBE8_B1E6;
}

/// A lock unit's state (`DISPIRQ0` here).
#[derive(Clone, Copy, Debug)]
struct LockUnit {
    counter: u32,
    privilege: bool,
    frozen: bool,
}

impl Default for LockUnit {
    fn default() -> LockUnit {
        LockUnit { counter: 1, privilege: false, frozen: false }
    }
}

impl LockUnit {
    fn key(&mut self, k: u32) {
        if self.frozen {
            return;
        }
        match k {
            key::LOCK => self.counter = self.counter.saturating_sub(1),
            key::UNLOCK => self.counter = (self.counter + 1).min(15),
            key::PRIVILEGE_ON => self.privilege = true,
            key::PRIVILEGE_OFF => self.privilege = false,
            key::FREEZE => self.frozen = true,
            _ => {}
        }
    }
    fn status(&self) -> u32 {
        u32::from(self.counter == 0) | u32::from(self.privilege) << 4 | u32::from(self.frozen) << 8
    }
}

#[derive(Default)]
struct State {
    regs: BTreeMap<(&'static str, u32), u32>,
    log: Vec<Access>,
    refused: Vec<Access>,
    frame: u32,
    frames_running: u32,
    shadow_pending: bool,
    shadow_taken: bool,
    scanned: Option<u64>,
    sig_pending: bool,
    sig_valid: bool,
    /// FGENABLE was cleared while running; the frame has not ended.
    fg_ending: bool,
    /// FGENABLE was cleared while running; reads of DISPIRQ0's status since
    /// (`None`: no sequence pending).
    seq_reads: Option<u32>,
    dirq0_lock: LockUnit,
}

pub struct Display {
    pub sm: Rc<RefCell<Sm>>,
    st: RefCell<State>,
    pub faults: Faults,
    /// Frames from the start to the primary channel's sync.
    pub lag: u32,
    /// What each buffer shows, by address (the CRC's input).
    pub content: BTreeMap<u64, u32>,
    /// The read of `DISPIRQ0_INTERRUPTSTATUS0` after the stop on which the
    /// sequence complete sets (`None`: it never does).
    pub seq_after: Option<u32>,
}

impl Display {
    pub fn new(sm: Rc<RefCell<Sm>>) -> Display {
        Display {
            sm,
            st: RefCell::new(State::default()),
            faults: Faults::default(),
            lag: 2,
            content: BTreeMap::new(),
            seq_after: Some(1),
        }
    }

    fn block(addr: usize) -> (&'static str, u32) {
        if (CSR..CSR + 0x1_0000).contains(&addr) {
            ("csr", (addr - CSR) as u32)
        } else if (LDB..LDB + 0x1_0000).contains(&addr) {
            ("ldb", (addr - LDB) as u32)
        } else if (IL..IL + 0x2_0000).contains(&addr) {
            ("il", (addr - IL) as u32)
        } else if (DPU..DPU + 0x40_0000).contains(&addr) {
            ("dpu", (addr - DPU) as u32)
        } else {
            panic!("access outside the display blocks: 0x{addr:x}")
        }
    }

    fn reg(st: &State, b: &'static str, off: u32) -> u32 {
        *st.regs.get(&(b, off)).unwrap_or(&0)
    }

    pub fn log(&self) -> Vec<Access> {
        self.st.borrow().log.clone()
    }

    pub fn writes(&self) -> Vec<(&'static str, u32, u32)> {
        self.log()
            .into_iter()
            .filter_map(|a| match a {
                Access::W(b, o, v) => Some((b, o, v)),
                _ => None,
            })
            .collect()
    }

    /// A register's value as the model holds it (no side effect, not logged).
    pub fn peek(&self, block: &'static str, off: u32) -> u32 {
        Self::reg(&self.st.borrow(), block, off)
    }

    pub fn clear_log(&self) {
        self.st.borrow_mut().log.clear();
    }

    pub fn refused(&self) -> Vec<Access> {
        self.st.borrow().refused.clone()
    }

    /// The buffer the last frame scanned out.
    pub fn scanned(&self) -> Option<u64> {
        self.st.borrow().scanned
    }

    fn powered(&self) -> bool {
        self.sm.borrow().on(13)
    }

    /// The pixel clock arrives at FrameGen0 (the LDB route also needs the
    /// LDB's clocks ungated).
    fn pixel_flows(&self, st: &State) -> bool {
        let sm = self.sm.borrow();
        let clocked = |c| sm.enabled(c) && sm.rate_of(c) > 0;
        match Self::reg(st, "csr", 0x0) & 0x3 {
            0 => clocked(80),
            2 => clocked(38) && Self::reg(st, "ldb", 0) & LDB_CLOCKS_GATED != LDB_CLOCKS_GATED,
            _ => false,
        }
    }

    fn running(&self, st: &State) -> bool {
        let pix = self.pixel_flows(st);
        let fgen = Self::reg(st, "dpu", o::FG_ENABLE) & 1 != 0;
        (fgen || self.faults.fg_stuck && st.frames_running > 0) && pix
    }

    fn path_complete(st: &State) -> bool {
        let r = |off| Self::reg(st, "dpu", off);
        let dm = r(o::DM_EXTDST0);
        dm & (1 << 20 | 1 << 14 | 1 << 10) == (1 << 20 | 1 << 14 | 1 << 10)
            && r(o::ED0CFG_STATIC) & 0x10 == 0
            && r(o::ED0CFG_DYNAMIC) & 0x3f == 0x14
            && r(o::ED0_STATICCONTROL) & 0x100 != 0
            && r(o::LB1CFG_DYNAMIC) & 0x3f == 0x0c
            && r(o::LB1CFG_DYNAMIC) >> 8 & 0x3f == 0x1a
            && r(o::LB1CFG_DYNAMIC) >> 24 & 3 != 0
            && r(o::LB1_CONTROL) & 1 != 0
            && r(o::FL0_LAYERPROPERTY0) & 1 << 31 != 0
            && st.shadow_taken
            && r(o::FG_HTCFG1) != 0
            && r(o::FG_VTCFG1) != 0
            && r(o::FG_PKICK) & 1 << 31 != 0
            && r(o::FG_INCTRL) & 7 == 2
    }

    /// One frame passes while running.
    fn tick(&self, st: &mut State) {
        if !self.running(st) {
            return;
        }
        st.frame += 1;
        st.frames_running += 1;
        if st.shadow_pending && !self.faults.no_shadow_load {
            st.shadow_pending = false;
            st.shadow_taken = true;
            let a = Self::reg(st, "dpu", o::FL0_BASE0) as u64 | (Self::reg(st, "dpu", o::FL0_BASEMSB0) as u64) << 32;
            st.scanned = Some(a);
            let s = Self::reg(st, "dpu", o::IRQ_STATUS0) | 1 << 3;
            st.regs.insert(("dpu", o::IRQ_STATUS0), s);
        }
        if st.sig_pending && Self::reg(st, "dpu", o::SIG_CONTROL0) & 1 != 0 {
            st.sig_pending = false;
            st.sig_valid = true;
        }
    }

    fn crc(&self, st: &State, ch: u32) -> u32 {
        let c = st.scanned.and_then(|a| self.content.get(&a)).copied().unwrap_or(0);
        let ul = Self::reg(st, "dpu", o::SIG_UL0);
        let lr = Self::reg(st, "dpu", o::SIG_LR0);
        let mut x = c ^ ul.rotate_left(7) ^ lr.rotate_left(13) ^ ch.wrapping_mul(0x9e37_79b9);
        x ^= x >> 16;
        x = x.wrapping_mul(0x7feb_352d);
        x ^ x >> 15
    }

    fn read(&self, addr: usize) -> u32 {
        let (b, off) = Self::block(addr);
        let mut st = self.st.borrow_mut();
        if !self.powered() || b == "dpu" && FAULTS.contains(&off) {
            st.refused.push(Access::R(b, off, 0));
            return 0;
        }
        let v = match (b, off) {
            ("dpu", o::FG_ENSTS) => {
                if st.fg_ending && self.pixel_flows(&st) {
                    st.fg_ending = false;
                }
                u32::from(self.running(&st) || st.fg_ending)
            }
            ("dpu", o::FG_TIMESTAMP) => {
                self.tick(&mut st);
                st.frame << 14
            }
            ("dpu", o::FG_CHSTAT) => {
                self.tick(&mut st);
                let sync = self.running(&st)
                    && !self.faults.no_sync
                    && st.frames_running >= self.lag
                    && Self::path_complete(&st);
                u32::from(sync) << 8
            }
            ("dpu", o::IRQ_STATUS0) => {
                self.tick(&mut st);
                Self::reg(&st, b, off)
            }
            ("dpu", o::DIRQ0_STATUS0) => {
                if let Some(n) = st.seq_reads.filter(|_| self.pixel_flows(&st)) {
                    let n = n + 1;
                    st.seq_reads = Some(n);
                    if self.seq_after.is_some_and(|after| n >= after) {
                        st.seq_reads = None;
                        let s = Self::reg(&st, b, off) | 1 << 20;
                        st.regs.insert((b, off), s);
                    }
                }
                Self::reg(&st, b, off)
            }
            ("dpu", o::SIG_STATUS) => {
                self.tick(&mut st);
                u32::from(st.sig_valid) << 1 | u32::from(st.sig_valid)
            }
            ("dpu", o::DIRQ0_LOCKSTATUS) => st.dirq0_lock.status(),
            // write-only
            ("dpu", o::DIRQ0_LOCKUNLOCK) => 0,
            ("dpu", o::SIG_CRC_R0) => self.crc(&st, 0),
            ("dpu", o::SIG_CRC_G0) => self.crc(&st, 1),
            ("dpu", o::SIG_CRC_B0) => self.crc(&st, 2),
            // COMCTRL_IPIDENTIFIER: a value for the read-back only.
            ("dpu", 0) => 0x5e5e_0001,
            _ => Self::reg(&st, b, off),
        };
        st.log.push(Access::R(b, off, v));
        v
    }

    fn write(&self, addr: usize, val: u32) {
        let (b, off) = Self::block(addr);
        let mut st = self.st.borrow_mut();
        if !self.powered() {
            st.refused.push(Access::W(b, off, val));
            return;
        }
        st.log.push(Access::W(b, off, val));
        if b == "dpu" && off == o::DIRQ0_LOCKUNLOCK {
            st.dirq0_lock.key(val);
            return;
        }
        if b == "dpu" && (o::DIRQ0_LOCKUNLOCK..o::DIRQ0_END).contains(&off) && st.dirq0_lock.counter == 0 {
            // locked: the write does not land
            return;
        }
        match (b, off) {
            ("dpu", o::IRQ_CLEAR0) => {
                let s = Self::reg(&st, b, o::IRQ_STATUS0) & !val;
                st.regs.insert((b, o::IRQ_STATUS0), s);
            }
            ("dpu", o::DIRQ0_CLEAR0) => {
                let s = Self::reg(&st, b, o::DIRQ0_STATUS0) & !val;
                st.regs.insert((b, o::DIRQ0_STATUS0), s);
            }
            ("dpu", o::ED0CFG_TRIGGER) => {
                if val & 1 != 0 {
                    st.shadow_pending = true;
                }
            }
            ("dpu", o::SIG_SHADOWLOAD) => {
                if val & 1 != 0 {
                    st.sig_pending = true;
                    st.sig_valid = false;
                }
            }
            ("dpu", o::FG_ENABLE) => {
                if val & 1 == 0 && self.running(&st) {
                    st.seq_reads = Some(0);
                }
                if val & 1 == 0
                    && self.faults.frame_end_needed
                    && Self::reg(&st, "dpu", o::FG_ENABLE) & 1 != 0
                    && st.frames_running > 0
                {
                    st.fg_ending = true;
                }
                st.regs.insert((b, off), val);
                if val & 1 == 0 && !self.faults.fg_stuck {
                    st.frames_running = 0;
                }
            }
            _ => {
                st.regs.insert((b, off), val);
            }
        }
    }
}

impl Mmio for Display {
    fn read32(&self, addr: usize) -> u32 {
        self.read(addr)
    }
    fn write32(&self, addr: usize, val: u32) {
        self.write(addr, val)
    }
    fn read8(&self, _: usize) -> u8 {
        panic!("the display blocks take 32-bit accesses")
    }
    fn write8(&self, _: usize, _: u8) {
        panic!("the display blocks take 32-bit accesses")
    }
}
