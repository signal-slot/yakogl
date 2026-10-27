// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX "WDOG" watchdog (i.MX 8M Plus WDOG1..3): arm it for a fixed
//! timeout, whatever state the previous stage left it in, and service it.
//!
//! Why: an image that U-Boot starts with `go` (docs/standalone-boot.md,
//! "What the image does on a `go` entry") must not depend on whoever ran
//! before to have armed WDOG1. A kexec entry
//! finds it running with Linux's timeout (systemd: 30 s); a U-Boot entry
//! finds it running with U-Boot's (when U-Boot's driver model watchdog
//! autostarted it) or stopped. [`arm`] leaves it running with the timeout
//! asked for in every one of those cases, reads it back and reports what it
//! found, so that the board says which case it was.
//!
//! Register names, offsets and bit positions: hal_nxp (BSD-3, revision
//! 1e91007b) `MIMX8ML8_ca53.h`, `WDOG_Type` and the `WDOG_WCR_*` /
//! `WDOG_WMCR_*` fields, `WDOG1_BASE` 0x30280000. Facts this module relies
//! on, with where they come from (the i.MX 8M Plus Applications Processor
//! Reference Manual, IMX8MPRM Rev. 3, was not on the host this was written
//! on, so its WDOG chapter is a citation pending; nothing below is taken from
//! it):
//!
//! 1. Every WDOG register is 16 bits wide (`WDOG_Type`: `uint16_t WCR` at 0h,
//!    `WSR` at 2h, `WRSR` at 4h, `WICR` at 6h, `WMCR` at 8h), so the block is
//!    accessed through [`Mmio16`] only.
//! 2. The timeout is `(WT + 1) * 0.5 s`: the `WDOG_WCR_WT` value list,
//!    "0b00000000..- 0.5 Seconds (Default)", "0b00000001..- 1.0 Seconds",
//!    ..., "0b11111111..- 128 Seconds". The step does not depend on any
//!    clock root the CCM sets, so there is no rate to read back, unlike the
//!    AM62P's RTI (driver/zephyr/lib/board/am62p/rti_arm.c).
//! 3. `WCR.SRS` = 0 asserts the system reset and `WCR.WDA` = 0 asserts
//!    `WDOG_B` (the field lists: "0b0..Assert system reset signal", "0b0..
//!    Assert WDOG_B output"; 1 is "No effect"). So every write of `WCR` here
//!    carries SRS = WDA = 1, whatever was read.
//! 4. `WCR.WDT` = 1 asserts `WDOG_B` on a timeout ("0b1..Assert WDOG_B upon
//!    a Watchdog Time-out event"). The Verdin's U-Boot and Linux device trees
//!    give WDOG1 `fsl,ext-reset-output` (records/imx8mp/netboot/
//!    uboot-control-dt-2026-10-03.dts), i.e. the board resets through
//!    `WDOG_B`; [`arm`] sets WDT when asked to (`ext_reset`).
//! 5. The service sequence is 0x5555 then 0xAAAA written to `WSR` (hal_nxp
//!    `fsl_wdog.h` `WDOG_Refresh`; the same sequence gc_sample.c has used on
//!    this board since 2026-09-20).
//! 6. `WMCR.PDE` is the power-down counter enable ("Power Down Enable"):
//!    left set, it resets the SoC some time after reset. U-Boot clears it on
//!    i.MX 8M (`imx_set_wdog_powerdown(false)`, arch/arm/mach-imx/imx8m/soc.c
//!    in U-Boot v2024.07, read for the fact only) and Linux's imx2_wdt does
//!    too; [`arm`] clears it if it is still set and reads it back.
//!
//! What a write of `WCR` does to a watchdog that is already running: the
//! Linux and U-Boot drivers both rewrite `WT` of a running WDOG and then
//! service it (U-Boot drivers/watchdog/imx_watchdog.c `imx_watchdog_init`,
//! v2024.07, read for the fact only), and the new timeout counts from that
//! service. [`arm`] does the same: write, then service, then read back.
//! Whether `WDE`, `WDT` and `WDZST` are write-once is not relied on: for a
//! running WDOG they are written back as read.

use crate::mmio::Mmio16;

/// Register offsets (`WDOG_Type`).
pub const WCR: usize = 0x0;
pub const WSR: usize = 0x2;
pub const WRSR: usize = 0x4;
pub const WMCR: usize = 0x8;

/// `WCR` fields (`WDOG_WCR_*_MASK`).
pub mod wcr {
    pub const WDZST: u16 = 1 << 0;
    pub const WDBG: u16 = 1 << 1;
    pub const WDE: u16 = 1 << 2;
    pub const WDT: u16 = 1 << 3;
    pub const SRS: u16 = 1 << 4;
    pub const WDA: u16 = 1 << 5;
    pub const WT_MASK: u16 = 0xff00;
    pub const WT_SHIFT: u32 = 8;
}

/// `WMCR.PDE` (`WDOG_WMCR_PDE_MASK`).
pub const WMCR_PDE: u16 = 1 << 0;

/// The service sequence written to `WSR`, in order.
pub const SERVICE: [u16; 2] = [0x5555, 0xaaaa];

/// One `WT` step, and the shortest and longest timeouts `WT` can say.
pub const WT_STEP_MS: u32 = 500;
pub const TIMEOUT_MIN_MS: u32 = WT_STEP_MS;
pub const TIMEOUT_MAX_MS: u32 = 256 * WT_STEP_MS;

/// Why [`arm`] refused. The value crosses to C (`YD_IMX_WDOG_REFUSED_*`)
/// and into the SNVS record, so each cause is one number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Refusal {
    /// The timeout is not a whole number of 0.5 s steps in 0.5 s ..= 128 s.
    Timeout = 1,
    /// `WMCR.PDE` read back set after it was cleared.
    PowerDownStuck = 2,
    /// `WCR.WDE` read back clear after the write: the watchdog is not
    /// running (a block that ignores writes reads like this).
    NotEnabled = 3,
    /// `WCR.WT` read back other than written.
    TimeoutReadBack = 4,
    /// `WDT` was asked for and read back clear: a timeout would not reach
    /// `WDOG_B`, and on this board that is the reset path.
    ExtResetReadBack = 5,
}

/// `WT` for a timeout, or `None` when `WT` cannot say it exactly. A timeout
/// that is not a whole number of steps is refused rather than rounded: the
/// caller asked for a bound, and a rounded one is a different bound.
pub const fn wt_for_timeout_ms(ms: u32) -> Option<u8> {
    if ms < TIMEOUT_MIN_MS || ms > TIMEOUT_MAX_MS || !ms.is_multiple_of(WT_STEP_MS) {
        return None;
    }
    Some((ms / WT_STEP_MS - 1) as u8)
}

/// The timeout `WT` gives.
pub const fn timeout_ms_of_wt(wt: u8) -> u32 {
    (wt as u32 + 1) * WT_STEP_MS
}

/// The `WT` field of a `WCR` value.
pub const fn wt_of(wcr: u16) -> u8 {
    ((wcr & wcr::WT_MASK) >> wcr::WT_SHIFT) as u8
}

/// The `WCR` value to write, given the value found.
///
/// - Running (`WDE` set): every bit as found, `WT` replaced, SRS and WDA
///   forced to 1 (fact 3: a 0 there would reset the SoC now), and WDT set
///   when `ext_reset` (setting it never makes the reset path shorter).
/// - Stopped: WDZST | WDBG | WDE | SRS | WDA | WT, plus WDT when
///   `ext_reset`. That is the value U-Boot's imx_watchdog_init writes on this
///   SoC (with `fsl,ext-reset-output`), so a U-Boot that autostarts the
///   watchdog and this function agree bit for bit.
pub const fn wcr_for(found: u16, wt: u8, ext_reset: bool) -> u16 {
    let ext = if ext_reset { wcr::WDT } else { 0 };
    let wt_bits = (wt as u16) << wcr::WT_SHIFT;
    if found & wcr::WDE != 0 {
        (found & !wcr::WT_MASK) | wcr::SRS | wcr::WDA | ext | wt_bits
    } else {
        wcr::WDZST | wcr::WDBG | wcr::WDE | wcr::SRS | wcr::WDA | ext | wt_bits
    }
}

/// What [`arm`] found and left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Armed {
    pub wcr_found: u16,
    pub wmcr_found: u16,
    pub wcr_now: u16,
    /// `WDE` was set before this ran: an earlier stage had started it.
    pub was_running: bool,
    /// `WMCR.PDE` was set and this cleared it.
    pub cleared_power_down: bool,
}

/// What [`arm`] read before it refused, for the refusal's log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused {
    pub why: Refusal,
    pub wcr_found: u16,
    pub wmcr_found: u16,
    /// The last value read from the register the refusal is about (`WCR`,
    /// or `WMCR` for [`Refusal::PowerDownStuck`]; 0 when nothing was
    /// written yet).
    pub read_back: u16,
}

/// Service the watchdog at `base`.
pub fn service(m: &impl Mmio16, base: usize) {
    for v in SERVICE {
        m.write16(base + WSR, v);
    }
}

/// Arm the WDOG at `base` for `timeout_ms`, from whatever state it is in,
/// service it, and read it back. On success the watchdog runs with exactly
/// that timeout, counted from the service this made. A refusal leaves the
/// watchdog as the failed step left it; the caller resets the SoC.
pub fn arm(m: &impl Mmio16, base: usize, timeout_ms: u32, ext_reset: bool) -> Result<Armed, Refused> {
    let wcr_found = m.read16(base + WCR);
    let wmcr_found = m.read16(base + WMCR);
    let refuse = |why, read_back| Refused { why, wcr_found, wmcr_found, read_back };

    let Some(wt) = wt_for_timeout_ms(timeout_ms) else {
        return Err(refuse(Refusal::Timeout, 0));
    };
    let mut cleared_power_down = false;
    if wmcr_found & WMCR_PDE != 0 {
        m.write16(base + WMCR, wmcr_found & !WMCR_PDE);
        let now = m.read16(base + WMCR);
        if now & WMCR_PDE != 0 {
            return Err(refuse(Refusal::PowerDownStuck, now));
        }
        cleared_power_down = true;
    }
    m.write16(base + WCR, wcr_for(wcr_found, wt, ext_reset));
    service(m, base);
    let wcr_now = m.read16(base + WCR);
    if wcr_now & wcr::WDE == 0 {
        return Err(refuse(Refusal::NotEnabled, wcr_now));
    }
    if wt_of(wcr_now) != wt {
        return Err(refuse(Refusal::TimeoutReadBack, wcr_now));
    }
    if ext_reset && wcr_now & wcr::WDT == 0 {
        return Err(refuse(Refusal::ExtResetReadBack, wcr_now));
    }
    Ok(Armed { wcr_found, wmcr_found, wcr_now, was_running: wcr_found & wcr::WDE != 0, cleared_power_down })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const BASE: usize = 0x3028_0000;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Op {
        R(usize, u16),
        W(usize, u16),
    }

    /// A WDOG as the field lists describe it, plus the faults the refusals
    /// are for. `WCR` reads back what was written unless `dead`; `WT` writes
    /// are ignored when `wt_frozen`; `WDT` is dropped when `no_wdt`.
    #[derive(Default)]
    struct Wdog {
        wcr: RefCell<u16>,
        wmcr: RefCell<u16>,
        ops: RefCell<Vec<Op>>,
        services: RefCell<u32>,
        last_wsr: RefCell<u16>,
        dead: bool,
        pde_stuck: bool,
        wt_frozen: bool,
        no_wdt: bool,
    }

    impl Wdog {
        fn with(wcr: u16, wmcr: u16) -> Wdog {
            Wdog { wcr: RefCell::new(wcr), wmcr: RefCell::new(wmcr), ..Wdog::default() }
        }
        fn ops(&self) -> Vec<Op> {
            self.ops.borrow().clone()
        }
    }

    impl Mmio16 for Wdog {
        fn read16(&self, addr: usize) -> u16 {
            let v = match addr - BASE {
                WCR if self.dead => 0,
                WCR => *self.wcr.borrow(),
                WMCR => *self.wmcr.borrow(),
                off => panic!("read of WDOG offset {off:#x} the driver has no business reading"),
            };
            self.ops.borrow_mut().push(Op::R(addr - BASE, v));
            v
        }
        fn write16(&self, addr: usize, val: u16) {
            self.ops.borrow_mut().push(Op::W(addr - BASE, val));
            match addr - BASE {
                WCR => {
                    assert_ne!(val & wcr::SRS, 0, "a WCR write with SRS = 0 resets the SoC");
                    assert_ne!(val & wcr::WDA, 0, "a WCR write with WDA = 0 asserts WDOG_B");
                    let mut w = self.wcr.borrow_mut();
                    let mut v = val;
                    if self.wt_frozen {
                        v = (v & !wcr::WT_MASK) | (*w & wcr::WT_MASK);
                    }
                    if self.no_wdt {
                        v &= !wcr::WDT;
                    }
                    *w = v;
                }
                WSR => {
                    let prev = self.last_wsr.replace(val);
                    if prev == SERVICE[0] && val == SERVICE[1] {
                        *self.services.borrow_mut() += 1;
                    }
                }
                WMCR => {
                    if !self.pde_stuck {
                        *self.wmcr.borrow_mut() = val;
                    }
                }
                off => panic!("write of WDOG offset {off:#x}"),
            }
        }
    }

    #[test]
    fn layout_is_the_headers() {
        // hal_nxp WDOG_Type: WCR 0x0, WSR 0x2, WRSR 0x4, WICR 0x6, WMCR 0x8;
        // WDOG_WCR_*_MASK 0x1, 0x2, 0x4, 0x8, 0x10, 0x20, WT 0xFF00.
        assert_eq!((WCR, WSR, WRSR, WMCR), (0, 2, 4, 8));
        assert_eq!(
            [wcr::WDZST, wcr::WDBG, wcr::WDE, wcr::WDT, wcr::SRS, wcr::WDA, wcr::WT_MASK],
            [0x1, 0x2, 0x4, 0x8, 0x10, 0x20, 0xff00]
        );
        assert_eq!(WMCR_PDE, 0x1);
    }

    #[test]
    fn timeouts_are_whole_half_seconds_from_half_a_second_to_128() {
        // Worked by hand from "(WT + 1) * 0.5 s": 60 s = 120 steps = WT 119.
        assert_eq!(wt_for_timeout_ms(60_000), Some(0x77));
        assert_eq!(wt_for_timeout_ms(30_000), Some(0x3b)); // systemd's 30 s
        assert_eq!(wt_for_timeout_ms(500), Some(0));
        assert_eq!(wt_for_timeout_ms(128_000), Some(0xff));
        for refused in [0, 1, 499, 750, 59_999, 60_001, 128_500, u32::MAX] {
            assert_eq!(wt_for_timeout_ms(refused), None, "{refused} ms");
        }
        assert_eq!(timeout_ms_of_wt(0x77), 60_000);
        assert_eq!(timeout_ms_of_wt(0xff), 128_000);
        for wt in 0..=255u8 {
            assert_eq!(wt_for_timeout_ms(timeout_ms_of_wt(wt)), Some(wt));
        }
        // and a deliberately wrong expectation is caught
        assert_ne!(wt_for_timeout_ms(60_000), Some(0x78));
    }

    #[test]
    fn a_stopped_watchdog_gets_u_boots_value() {
        // U-Boot imx_watchdog_init with fsl,ext-reset-output, 60 s:
        // WDZST | WDBG | WDE | SRS | WDA | WDT | WT 0x77 = 0x773f.
        assert_eq!(wcr_for(0x0030, 0x77, true), 0x773f);
        assert_eq!(wcr_for(0x0030, 0x77, false), 0x7737);
        assert_eq!(wcr_for(0x0000, 0x77, true), 0x773f);
    }

    #[test]
    fn a_running_watchdog_keeps_its_bits_and_gets_the_new_timeout() {
        // Linux's 30 s with WDZST | WDE | WDT | SRS | WDA (0x3b3d): only WT moves.
        assert_eq!(wcr_for(0x3b3d, 0x77, true), 0x773d);
        // U-Boot's own 60 s: unchanged.
        assert_eq!(wcr_for(0x773f, 0x77, true), 0x773f);
        // a running watchdog without WDT gets it when asked, keeps it off otherwise
        assert_eq!(wcr_for(0x3b35, 0x77, true), 0x773d);
        assert_eq!(wcr_for(0x3b35, 0x77, false), 0x7735);
    }

    #[test]
    fn no_value_found_makes_a_write_that_resets_or_stops_the_watchdog() {
        for found in 0..=u16::MAX {
            for ext in [false, true] {
                let w = wcr_for(found, 0x77, ext);
                assert_eq!(w & (wcr::SRS | wcr::WDA), wcr::SRS | wcr::WDA, "{found:#06x}");
                assert_ne!(w & wcr::WDE, 0, "{found:#06x}");
                assert_eq!(wt_of(w), 0x77, "{found:#06x}");
                let kept_wdt = found & wcr::WDE != 0 && found & wcr::WDT != 0;
                assert_eq!(w & wcr::WDT != 0, ext || kept_wdt, "{found:#06x}");
            }
        }
    }

    #[test]
    fn cold_arm_is_one_write_one_service_one_read_back() {
        let w = Wdog::with(0x0030, 0x0000);
        let a = arm(&w, BASE, 60_000, true).expect("armed");
        assert_eq!(
            w.ops(),
            vec![
                Op::R(WCR, 0x0030),
                Op::R(WMCR, 0),
                Op::W(WCR, 0x773f),
                Op::W(WSR, 0x5555),
                Op::W(WSR, 0xaaaa),
                Op::R(WCR, 0x773f),
            ]
        );
        assert_eq!(*w.services.borrow(), 1);
        assert_eq!(
            a,
            Armed { wcr_found: 0x30, wmcr_found: 0, wcr_now: 0x773f, was_running: false, cleared_power_down: false }
        );
    }

    #[test]
    fn a_running_watchdog_is_retimed_and_reported_as_found() {
        let w = Wdog::with(0x3b3d, 0x0000);
        let a = arm(&w, BASE, 60_000, true).expect("armed");
        assert!(a.was_running);
        assert_eq!(a.wcr_found, 0x3b3d);
        assert_eq!(a.wcr_now, 0x773d);
        assert_eq!(*w.services.borrow(), 1);
    }

    #[test]
    fn the_power_down_counter_is_cleared_and_read_back() {
        let w = Wdog::with(0x0030, 0x0001);
        let a = arm(&w, BASE, 60_000, true).expect("armed");
        assert!(a.cleared_power_down);
        assert_eq!(&w.ops()[..4], &[Op::R(WCR, 0x30), Op::R(WMCR, 1), Op::W(WMCR, 0), Op::R(WMCR, 0)]);

        let stuck = Wdog { pde_stuck: true, ..Wdog::with(0x0030, 0x0001) };
        let r = arm(&stuck, BASE, 60_000, true).unwrap_err();
        assert_eq!(r.why, Refusal::PowerDownStuck);
        assert_eq!(r.read_back, 1);
        assert!(!stuck.ops().iter().any(|o| matches!(o, Op::W(WCR, _))), "no WCR write after a refusal");
    }

    #[test]
    fn each_read_back_failure_is_its_own_refusal() {
        let dead = Wdog { dead: true, ..Wdog::with(0, 0) };
        assert_eq!(arm(&dead, BASE, 60_000, true).unwrap_err().why, Refusal::NotEnabled);

        let frozen = Wdog { wt_frozen: true, ..Wdog::with(0x3b3d, 0) };
        let r = arm(&frozen, BASE, 60_000, true).unwrap_err();
        assert_eq!((r.why, r.read_back), (Refusal::TimeoutReadBack, 0x3b3d));

        let no_wdt = Wdog { no_wdt: true, ..Wdog::with(0x0030, 0) };
        assert_eq!(arm(&no_wdt, BASE, 60_000, true).unwrap_err().why, Refusal::ExtResetReadBack);
        // the same block passes when WDOG_B is not asked for
        let no_wdt = Wdog { no_wdt: true, ..Wdog::with(0x0030, 0) };
        assert!(arm(&no_wdt, BASE, 60_000, false).is_ok());
    }

    #[test]
    fn an_unrepresentable_timeout_writes_nothing() {
        let w = Wdog::with(0x0030, 0x0001);
        let r = arm(&w, BASE, 60_001, true).unwrap_err();
        assert_eq!(r.why, Refusal::Timeout);
        assert!(w.ops().iter().all(|o| matches!(o, Op::R(..))), "{:?}", w.ops());
    }
}
