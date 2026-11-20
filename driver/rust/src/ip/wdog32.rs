// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The NXP WDOG32 watchdog (the i.MX 95's WDOG3 at 0x42490000, the one the
//! Verdin iMX95's A55 non-secure side owns): arm it for a timeout, whatever
//! state the previous stage left it in, read it back, and refresh (feed) it.
//!
//! Why: an image that U-Boot starts with `go` must not depend on U-Boot's
//! watchdog still running, nor on its timeout. [`arm`] leaves WDOG3 running
//! with the timeout asked for, configured as Linux configures it on this
//! module, and reports what it found, so the board says which case it was.
//!
//! Register offsets and fields: Linux drivers/watchdog/imx7ulp_wdt.c
//! (torvalds/linux 4dd10fbb, the driver of `fsl,imx93-wdt`, which
//! imx95.dtsi's wdog3 is) and U-Boot drivers/watchdog/ulp_wdog.c (the same
//! fields, `WDGCS_*`); both read for the facts only. Facts this module
//! relies on:
//!
//! 1. `CS` (0x0), `CNT` (0x4), `TOVAL` (0x8) are 32-bit registers. `CS`:
//!    `STOP` bit 0, `WAIT` bit 1, `UPDATE` bit 5, `INT` bit 6, `EN` bit 7,
//!    `CLK` bits 9:8 (1 = the LPO clock), `RCS` bit 10, `ULK` bit 11, `PRES`
//!    bit 12 (the 256 prescaler), `CMD32EN` bit 13, `FLG` bit 14
//!    (imx7ulp_wdt.c:17-30).
//! 2. Configuration is unlocked by 0xD928C520 written to `CNT` when
//!    `CS.CMD32EN` is set, else by 0xC520 then 0xD928 as two 16-bit writes;
//!    `CS.ULK` then reads 1 (imx7ulp_wdt.c `_imx7ulp_wdt_init`, ulp_wdog.c
//!    `ulp_watchdog_init`). `TOVAL` and then `CS` are written, and `CS.RCS`
//!    reads 1 once the new configuration took effect (`imx7ulp_wdt_wait_rcs`).
//!    A refresh is 0xB480A602 written to `CNT` (32-bit commands, which the
//!    `CS` this writes enables).
//! 3. `CS.UPDATE` 1 keeps the configuration writable after it is written;
//!    both drivers write it, so a later stage (Linux after a reset, or this
//!    image again) can reconfigure. With `UPDATE` 0 the unlock does not take
//!    and [`arm`] refuses on the read-back.
//! 4. On the i.MX 93/95 (`fsl,imx93-wdt`) the counter runs on the LPO
//!    (32 kHz) through the 256 prescaler: 125 ticks per second
//!    (imx7ulp_wdt.c `imx93_wdt_hw`: `prescaler_enable`, `wdog_clock_rate`
//!    125; ulp_wdog.c `CLK_RATE_32KHZ` 125 for `fsl,imx93-wdt`). `TOVAL` is
//!    16 bits of that: at most 65535 ticks, 524.28 s.
//! 5. `CS.INT`: Linux sets it when the node has `fsl,ext-reset-output`
//!    (imx7ulp_wdt.c:292, `ext_reset`), which the Verdin's wdog3 has
//!    (imx95-verdin.dtsi:747-751); U-Boot sets it on every i.MX 9
//!    (ulp_wdog.c `IS_ENABLED(CONFIG_ARCH_IMX9)`). On the i.MX 95 the System
//!    Manager takes WDOG3's timeout as its WDOG3 interrupt
//!    (imx-sm devices/MIMX95/sm/dev_sm_handlers.c `WDOG3_IRQHandler` ->
//!    `FaultHandler(DEV_SM_FAULT_WDOG3)`), so [`arm`] sets it when asked
//!    (`ext_reset`) and reads it back.
//! 6. `WAIT` and `STOP` 1 keep the counter running while the core waits or
//!    stops (Linux writes both): a `wfi` does not pause the watchdog.
//!
//! What the timeout does on the Verdin iMX95 is the System Manager's, not
//! this module's: docs/notes/pre-study-imx95.md, "The watchdog".

use crate::mmio::{Mmio, Mmio16};

/// Register offsets.
pub const CS: usize = 0x0;
pub const CNT: usize = 0x4;
pub const TOVAL: usize = 0x8;

/// `CS` fields.
pub mod cs {
    pub const STOP: u32 = 1 << 0;
    pub const WAIT: u32 = 1 << 1;
    pub const UPDATE: u32 = 1 << 5;
    pub const INT: u32 = 1 << 6;
    pub const EN: u32 = 1 << 7;
    pub const CLK_LPO: u32 = 1 << 8;
    pub const CLK_MASK: u32 = 0x3 << 8;
    pub const RCS: u32 = 1 << 10;
    pub const ULK: u32 = 1 << 11;
    pub const PRES: u32 = 1 << 12;
    pub const CMD32EN: u32 = 1 << 13;
    pub const FLG: u32 = 1 << 14;
}

/// The unlock word (32-bit commands) and its two 16-bit halves, in order.
pub const UNLOCK: u32 = 0xd928_c520;
pub const UNLOCK16: [u16; 2] = [0xc520, 0xd928];
/// The refresh word (32-bit commands).
pub const REFRESH: u32 = 0xb480_a602;

/// Counter ticks per second: the LPO (32 kHz) through the 256 prescaler.
pub const TICK_HZ: u32 = 125;
/// The longest timeout `TOVAL` can say.
pub const TIMEOUT_MAX_MS: u32 = 0xffff * 1000 / TICK_HZ;

/// Why [`arm`] refused. The value crosses to C (`YD_WDOG32_REFUSED_*`), so
/// each cause is one number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Refusal {
    /// The timeout is not a whole number of 8 ms ticks in 8 ms ..= 524.28 s.
    Timeout = 1,
    /// `CS.ULK` did not read 1 within the polls after the unlock.
    Unlock = 2,
    /// `CS.RCS` did not read 1 within the polls after the write.
    Reconfig = 3,
    /// `CS` read back other than written (the flags `FLG`, `ULK`, `RCS`
    /// aside): a locked configuration (`UPDATE` 0) reads like this.
    CsReadBack = 4,
    /// `TOVAL` read back other than written.
    ToValReadBack = 5,
}

/// `TOVAL` for a timeout, or `None` when it cannot say it exactly. A timeout
/// that is not a whole number of ticks is refused rather than rounded: the
/// caller asked for a bound, and a rounded one is a different bound.
pub const fn toval_for_timeout_ms(ms: u32) -> Option<u32> {
    let ticks = ms as u64 * TICK_HZ as u64;
    if ms == 0 || !ticks.is_multiple_of(1000) || ticks / 1000 > 0xffff {
        return None;
    }
    Some((ticks / 1000) as u32)
}

/// The timeout a `TOVAL` gives.
pub const fn timeout_ms_of_toval(toval: u32) -> u32 {
    ((toval & 0xffff) as u64 * 1000 / TICK_HZ as u64) as u32
}

/// The `CS` value [`arm`] writes: Linux's for `fsl,imx93-wdt` (32-bit
/// commands, the LPO, `UPDATE`, `WAIT`, `STOP`, the prescaler; `INT` with
/// `ext_reset`), and `EN`. Nothing of the value found is kept: every field
/// that matters is set here, and a window, the debug mode or a test mode an
/// earlier stage left are cleared.
pub const fn cs_for(ext_reset: bool) -> u32 {
    let int = if ext_reset { cs::INT } else { 0 };
    cs::CMD32EN | cs::CLK_LPO | cs::UPDATE | cs::WAIT | cs::STOP | cs::PRES | int | cs::EN
}

/// The bits of a `CS` read that say state, not configuration.
pub const CS_STATUS: u32 = cs::FLG | cs::ULK | cs::RCS;

/// What [`arm`] found and left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Armed {
    pub cs_found: u32,
    pub toval_found: u32,
    pub cs_now: u32,
    pub toval_now: u32,
    /// `CS.EN` was set before this ran: an earlier stage had started it.
    pub was_running: bool,
    /// The polls `ULK` and `RCS` took (the reads that saw them clear).
    pub ulk_polls: u32,
    pub rcs_polls: u32,
}

/// What [`arm`] read before it refused, for the refusal's line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused {
    pub why: Refusal,
    pub cs_found: u32,
    pub toval_found: u32,
    /// The last `CS` read (0 when nothing was written yet).
    pub cs_now: u32,
    /// The last `TOVAL` read (0 unless the refusal is about it).
    pub toval_now: u32,
}

/// Refresh the watchdog at `base`: one write (32-bit commands).
pub fn refresh(m: &impl Mmio, base: usize) {
    m.write32(base + CNT, REFRESH);
}

/// Poll `CS` until `bit` reads 1, at most `polls` reads. The reads that saw
/// it clear, or `Err` with the last value.
fn wait_cs(m: &impl Mmio, base: usize, bit: u32, polls: u32) -> Result<u32, u32> {
    let mut v = 0;
    for n in 0..polls {
        v = m.read32(base + CS);
        if v & bit != 0 {
            return Ok(n);
        }
    }
    Err(v)
}

/// Arm the WDOG32 at `base` for `timeout_ms`, from whatever state it is in,
/// refresh it and read it back. `polls` bounds each of the two waits (`ULK`,
/// `RCS`) in reads of `CS`; the caller measures the time. On success the
/// watchdog runs with exactly that timeout, counted from the refresh this
/// made. A refusal leaves it as the failed step left it; the caller resets
/// the SoC. The caller holds interrupts off: the unlock opens a window of
/// 128 bus clocks in which the configuration must be written.
pub fn arm(
    m: &impl Mmio,
    m16: &impl Mmio16,
    base: usize,
    timeout_ms: u32,
    ext_reset: bool,
    polls: u32,
) -> Result<Armed, Refused> {
    let cs_found = m.read32(base + CS);
    let toval_found = m.read32(base + TOVAL);
    let refuse = |why, cs_now, toval_now| Refused { why, cs_found, toval_found, cs_now, toval_now };

    let Some(toval) = toval_for_timeout_ms(timeout_ms) else {
        return Err(refuse(Refusal::Timeout, 0, 0));
    };
    if cs_found & cs::CMD32EN != 0 {
        m.write32(base + CNT, UNLOCK);
    } else {
        for half in UNLOCK16 {
            m16.write16(base + CNT, half);
        }
    }
    let ulk_polls = wait_cs(m, base, cs::ULK, polls).map_err(|v| refuse(Refusal::Unlock, v, 0))?;
    let want = cs_for(ext_reset);
    m.write32(base + TOVAL, toval);
    m.write32(base + CS, want);
    let rcs_polls = wait_cs(m, base, cs::RCS, polls).map_err(|v| refuse(Refusal::Reconfig, v, 0))?;
    refresh(m, base);
    let cs_now = m.read32(base + CS);
    if cs_now & !CS_STATUS != want {
        return Err(refuse(Refusal::CsReadBack, cs_now, 0));
    }
    let toval_now = m.read32(base + TOVAL);
    if toval_now != toval {
        return Err(refuse(Refusal::ToValReadBack, cs_now, toval_now));
    }
    Ok(Armed { cs_found, toval_found, cs_now, toval_now, was_running: cs_found & cs::EN != 0, ulk_polls, rcs_polls })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    const BASE: usize = 0x4249_0000;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Op {
        R(usize, u32),
        W(usize, u32),
        W16(usize, u16),
    }

    /// A WDOG32 as the two drivers describe it, plus the faults the refusals
    /// are for. An unlock (in the width `CMD32EN` says) sets `ULK` after
    /// `ulk_after` further reads of `CS`; a `CS` write while unlocked takes
    /// (unless `locked`: `UPDATE` 0 from an earlier stage), clears `ULK` and
    /// sets `RCS` after `rcs_after` reads; `int_dropped` loses `INT`;
    /// `toval_frozen` ignores `TOVAL` writes.
    #[derive(Default)]
    struct Wdog32 {
        cs: Cell<u32>,
        toval: Cell<u32>,
        ops: RefCell<Vec<Op>>,
        refreshes: Cell<u32>,
        half: Cell<Option<u16>>,
        unlocked: Cell<bool>,
        ulk_in: Cell<Option<u32>>,
        rcs_in: Cell<Option<u32>>,
        ulk_after: u32,
        rcs_after: u32,
        locked: bool,
        no_ulk: bool,
        no_rcs: bool,
        int_dropped: bool,
        toval_frozen: bool,
    }

    impl Wdog32 {
        fn with(cs: u32, toval: u32) -> Wdog32 {
            Wdog32 { cs: Cell::new(cs), toval: Cell::new(toval), ..Wdog32::default() }
        }
        fn ops(&self) -> Vec<Op> {
            self.ops.borrow().clone()
        }
        fn unlock(&self) {
            if !self.no_ulk {
                self.ulk_in.set(Some(self.ulk_after));
            }
            self.unlocked.set(!self.locked);
        }
        fn tick(&self, slot: &Cell<Option<u32>>, bit: u32) {
            match slot.get() {
                Some(0) => {
                    self.cs.set(self.cs.get() | bit);
                    slot.set(None);
                }
                Some(n) => slot.set(Some(n - 1)),
                None => {}
            }
        }
    }

    impl Mmio for Wdog32 {
        fn read32(&self, addr: usize) -> u32 {
            let v = match addr - BASE {
                CS => {
                    self.tick(&self.ulk_in, cs::ULK);
                    self.tick(&self.rcs_in, cs::RCS);
                    self.cs.get()
                }
                TOVAL => self.toval.get(),
                off => panic!("read of WDOG32 offset {off:#x} the driver has no business reading"),
            };
            self.ops.borrow_mut().push(Op::R(addr - BASE, v));
            v
        }
        fn write32(&self, addr: usize, val: u32) {
            self.ops.borrow_mut().push(Op::W(addr - BASE, val));
            match addr - BASE {
                CNT if val == UNLOCK => {
                    assert_ne!(self.cs.get() & cs::CMD32EN, 0, "a 32-bit unlock without CMD32EN");
                    self.unlock();
                }
                CNT if val == REFRESH => {
                    assert_ne!(self.cs.get() & cs::CMD32EN, 0, "a 32-bit refresh without CMD32EN");
                    self.refreshes.set(self.refreshes.get() + 1);
                }
                CNT => panic!("CNT written with {val:#x}: not a command (an unexpected value resets the SoC)"),
                TOVAL => {
                    assert!(self.unlocked.get() || self.locked, "TOVAL written while locked");
                    if self.unlocked.get() && !self.toval_frozen {
                        self.toval.set(val);
                    }
                }
                CS => {
                    if self.unlocked.get() {
                        let mut v = val & !CS_STATUS;
                        if self.int_dropped {
                            v &= !cs::INT;
                        }
                        self.cs.set(v);
                        self.unlocked.set(false);
                        if !self.no_rcs {
                            self.rcs_in.set(Some(self.rcs_after));
                        }
                    }
                }
                off => panic!("write of WDOG32 offset {off:#x}"),
            }
        }
        fn read8(&self, _: usize) -> u8 {
            panic!("8-bit read of the WDOG32")
        }
        fn write8(&self, _: usize, _: u8) {
            panic!("8-bit write of the WDOG32")
        }
    }

    impl Mmio16 for Wdog32 {
        fn read16(&self, _: usize) -> u16 {
            panic!("16-bit read of the WDOG32")
        }
        fn write16(&self, addr: usize, val: u16) {
            self.ops.borrow_mut().push(Op::W16(addr - BASE, val));
            assert_eq!(addr - BASE, CNT, "16-bit write outside CNT");
            assert_eq!(self.cs.get() & cs::CMD32EN, 0, "a 16-bit command with CMD32EN set");
            match (self.half.get(), val) {
                (None, 0xc520) => self.half.set(Some(val)),
                (Some(0xc520), 0xd928) => {
                    self.half.set(None);
                    self.unlock();
                }
                other => panic!("16-bit CNT sequence {other:x?}"),
            }
        }
    }

    /// Linux's CS for fsl,imx93-wdt with fsl,ext-reset-output, running:
    /// CMD32EN | PRES | CLK 1 | EN | INT | UPDATE | WAIT | STOP.
    const LINUX_CS: u32 = 0x2000 | 0x1000 | 0x0100 | 0x80 | 0x40 | 0x20 | 0x2 | 0x1;

    #[test]
    fn layout_is_the_drivers() {
        // imx7ulp_wdt.c:17-33 and ulp_wdog.c's WDGCS_*.
        assert_eq!((CS, CNT, TOVAL), (0x0, 0x4, 0x8));
        assert_eq!(
            [
                cs::STOP,
                cs::WAIT,
                cs::UPDATE,
                cs::INT,
                cs::EN,
                cs::CLK_LPO,
                cs::RCS,
                cs::ULK,
                cs::PRES,
                cs::CMD32EN,
                cs::FLG
            ],
            [0x1, 0x2, 0x20, 0x40, 0x80, 0x100, 0x400, 0x800, 0x1000, 0x2000, 0x4000]
        );
        assert_eq!((UNLOCK, REFRESH), (0xd928_c520, 0xb480_a602));
        assert_eq!(UNLOCK16, [0xc520, 0xd928]);
    }

    #[test]
    fn the_value_written_is_linuxs_with_the_counter_on() {
        assert_eq!(LINUX_CS, 0x31e3);
        assert_eq!(cs_for(true), LINUX_CS);
        assert_eq!(cs_for(false), LINUX_CS & !cs::INT);
        assert_eq!(cs_for(true) & cs::CLK_MASK, cs::CLK_LPO);
        assert_eq!(cs_for(true) & CS_STATUS, 0);
    }

    #[test]
    fn timeouts_are_whole_ticks_of_8_ms_up_to_524_s() {
        // 125 Hz: Linux's toval = 125 * seconds.
        assert_eq!(toval_for_timeout_ms(60_000), Some(7500));
        assert_eq!(toval_for_timeout_ms(40_000), Some(5000)); // imx95.dtsi timeout-sec
        assert_eq!(toval_for_timeout_ms(30_000), Some(3750)); // U-Boot's 30 s
        assert_eq!(toval_for_timeout_ms(8), Some(1));
        assert_eq!(toval_for_timeout_ms(TIMEOUT_MAX_MS), Some(0xffff));
        assert_eq!(TIMEOUT_MAX_MS, 524_280);
        for refused in [0, 1, 7, 9, 59_999, TIMEOUT_MAX_MS + 8, u32::MAX] {
            assert_eq!(toval_for_timeout_ms(refused), None, "{refused} ms");
        }
        for t in [1u32, 125, 3750, 7500, 0xffff] {
            assert_eq!(toval_for_timeout_ms(timeout_ms_of_toval(t)), Some(t));
        }
        assert_eq!(timeout_ms_of_toval(7500), 60_000);
        // and a deliberately wrong expectation is caught
        assert_ne!(toval_for_timeout_ms(60_000), Some(60_000));
    }

    #[test]
    fn u_boots_running_watchdog_is_retimed_with_32_bit_commands() {
        // U-Boot's ulp_watchdog_init on an i.MX 9 with CMD32EN kept, 30 s:
        // CMD32EN | PRES | INT | UPDATE | EN | CLK 1 | FLG written (w1c).
        let w = Wdog32 { ulk_after: 2, rcs_after: 3, ..Wdog32::with(0x2000 | 0x1000 | 0x0100 | 0xe0, 3750) };
        let a = arm(&w, &w, BASE, 60_000, true, 100).expect("armed");
        let found = 0x31e0;
        assert_eq!(
            w.ops(),
            vec![
                Op::R(CS, found),
                Op::R(TOVAL, 3750),
                Op::W(CNT, UNLOCK),
                Op::R(CS, found),
                Op::R(CS, found),
                Op::R(CS, found | cs::ULK),
                Op::W(TOVAL, 7500),
                Op::W(CS, LINUX_CS),
                Op::R(CS, LINUX_CS),
                Op::R(CS, LINUX_CS),
                Op::R(CS, LINUX_CS),
                Op::R(CS, LINUX_CS | cs::RCS),
                Op::W(CNT, REFRESH),
                Op::R(CS, LINUX_CS | cs::RCS),
                Op::R(TOVAL, 7500),
            ]
        );
        assert_eq!(w.refreshes.get(), 1);
        assert_eq!(
            a,
            Armed {
                cs_found: found,
                toval_found: 3750,
                cs_now: LINUX_CS | cs::RCS,
                toval_now: 7500,
                was_running: true,
                ulk_polls: 2,
                rcs_polls: 3,
            }
        );
    }

    #[test]
    fn a_stopped_16_bit_watchdog_is_unlocked_in_two_halves_and_started() {
        // CMD32EN clear (U-Boot keeps what it found): the 16-bit unlock,
        // then the CS written switches to 32-bit commands for the refresh.
        let w = Wdog32::with(0x0100, 0xffff);
        let a = arm(&w, &w, BASE, 60_000, true, 10).expect("armed");
        assert_eq!(&w.ops()[2..4], &[Op::W16(CNT, 0xc520), Op::W16(CNT, 0xd928)]);
        assert!(!a.was_running);
        assert_eq!((a.cs_now & !CS_STATUS, a.toval_now), (LINUX_CS, 7500));
        assert_eq!(w.refreshes.get(), 1);
    }

    #[test]
    fn every_found_value_leaves_the_same_configuration() {
        for found in [0u32, 0x2980, 0x2180, 0x31e0, 0x33e3, 0x7fff & !CS_STATUS, 0x8000 | 0x2184] {
            let w = Wdog32::with(found, 0x1234);
            let a = arm(&w, &w, BASE, 60_000, true, 10).unwrap_or_else(|r| panic!("{found:#x}: {r:?}"));
            assert_eq!(a.cs_now & !CS_STATUS, LINUX_CS, "{found:#x}");
            assert_eq!(a.was_running, found & cs::EN != 0, "{found:#x}");
        }
    }

    #[test]
    fn a_bad_timeout_touches_nothing() {
        let w = Wdog32::with(0x31e0, 3750);
        let r = arm(&w, &w, BASE, 60_001, true, 10).unwrap_err();
        assert_eq!(r.why, Refusal::Timeout);
        assert_eq!(w.ops(), vec![Op::R(CS, 0x31e0), Op::R(TOVAL, 3750)]);
    }

    #[test]
    fn an_unlock_that_never_shows_is_refused_without_a_write() {
        let w = Wdog32 { no_ulk: true, ..Wdog32::with(0x31e0, 3750) };
        let r = arm(&w, &w, BASE, 60_000, true, 5).unwrap_err();
        assert_eq!(r.why, Refusal::Unlock);
        assert_eq!(r.cs_now, 0x31e0);
        assert!(!w.ops().iter().any(|op| matches!(op, Op::W(TOVAL | CS, _))));
        assert_eq!(w.ops().iter().filter(|op| matches!(op, Op::R(CS, _))).count(), 1 + 5);
    }

    #[test]
    fn a_reconfiguration_that_never_completes_is_refused() {
        let w = Wdog32 { no_rcs: true, ..Wdog32::with(0x31e0, 3750) };
        let r = arm(&w, &w, BASE, 60_000, true, 5).unwrap_err();
        assert_eq!(r.why, Refusal::Reconfig);
        assert_eq!(w.refreshes.get(), 0);
    }

    #[test]
    fn a_locked_configuration_is_caught_on_the_read_back() {
        // UPDATE 0 from an earlier stage: the unlock shows, the writes do not
        // take, and the reconfiguration never completes.
        let w = Wdog32 { locked: true, ..Wdog32::with(0x2180, 3750) };
        assert_eq!(arm(&w, &w, BASE, 60_000, true, 5).unwrap_err().why, Refusal::Reconfig);
        let w = Wdog32 { locked: true, rcs_after: 0, ..Wdog32::with(0x2180 | cs::RCS, 3750) };
        let r = arm(&w, &w, BASE, 60_000, true, 5).unwrap_err();
        assert_eq!(r.why, Refusal::CsReadBack);
        assert_eq!(r.cs_now & !CS_STATUS, 0x2180);
    }

    #[test]
    fn a_dropped_int_and_a_frozen_toval_are_refused() {
        let w = Wdog32 { int_dropped: true, ..Wdog32::with(0x31e0, 3750) };
        let r = arm(&w, &w, BASE, 60_000, true, 5).unwrap_err();
        assert_eq!(r.why, Refusal::CsReadBack);
        assert_eq!(r.cs_now & cs::INT, 0);
        // without ext_reset the same block arms
        let w = Wdog32 { int_dropped: true, ..Wdog32::with(0x31e0, 3750) };
        assert!(arm(&w, &w, BASE, 60_000, false, 5).is_ok());

        let w = Wdog32 { toval_frozen: true, ..Wdog32::with(0x31e0, 3750) };
        let r = arm(&w, &w, BASE, 60_000, true, 5).unwrap_err();
        assert_eq!((r.why, r.toval_found, r.toval_now), (Refusal::ToValReadBack, 3750, 3750));
    }

    #[test]
    fn a_refresh_is_one_32_bit_write() {
        let w = Wdog32::with(LINUX_CS, 7500);
        refresh(&w, BASE);
        assert_eq!(w.ops(), vec![Op::W(CNT, REFRESH)]);
        assert_eq!(w.refreshes.get(), 1);
    }
}
