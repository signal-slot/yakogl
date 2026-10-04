// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX WDOG self-arm (ip::imx_wdog) for the gc_board shim
//! driver/zephyr/lib/board/imx8mp/gc_wdog_arm.c. No device, no env: the shim
//! calls [`yd_imx_wdog_arm`] once at PRE_KERNEL_1 with its two mappings and
//! logs, records and feeds from what comes back. Declarations:
//! include/yakogl_drivers.h ("i.MX WDOG self-arm").

use crate::errno::{EINVAL, EIO};
use crate::ip::imx_wdog::{self, Armed, Refused};
use crate::mmio::{Hw, Mmio, Mmio16};
use crate::soc::imx8mp::ccm;
use core::mem::{offset_of, size_of};

/// `flags` of [`YdImxWdogArm`].
pub const YD_IMX_WDOG_WAS_RUNNING: u32 = 1 << 0;
pub const YD_IMX_WDOG_CLEARED_PDE: u32 = 1 << 1;
pub const YD_IMX_WDOG_ROOT_ENABLED: u32 = 1 << 2;
pub const YD_IMX_WDOG_GATE_OPENED: u32 = 1 << 3;

/// `struct yd_imx_wdog_arm`: what the arming found and left.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdImxWdogArm {
    /// `WCR` and `WMCR` as found, before anything was written.
    pub wcr_found: u16,
    pub wmcr_found: u16,
    /// `WCR` read back after arming (on a refusal: the read the refusal is
    /// about, see [`Refused::read_back`]).
    pub wcr_now: u16,
    pub _pad: u16,
    pub flags: u32,
    /// 0, or the [`imx_wdog::Refusal`] number (`YD_IMX_WDOG_REFUSED_*`).
    pub refusal: u32,
    /// The timeout the watchdog now runs with, from the read-back `WT`.
    pub timeout_ms: u32,
}

const _: () = assert!(size_of::<YdImxWdogArm>() == 20);
const _: () = assert!(offset_of!(YdImxWdogArm, wcr_found) == 0);
const _: () = assert!(offset_of!(YdImxWdogArm, wmcr_found) == 2);
const _: () = assert!(offset_of!(YdImxWdogArm, wcr_now) == 4);
const _: () = assert!(offset_of!(YdImxWdogArm, flags) == 8);
const _: () = assert!(offset_of!(YdImxWdogArm, refusal) == 12);
const _: () = assert!(offset_of!(YdImxWdogArm, timeout_ms) == 16);

/// The clock root and gate first (a root is enabled before the gate it
/// feeds, ccm.rs), then [`imx_wdog::arm`]. 0, `-EIO` when the WDOG refused
/// (the reason in `out.refusal`), `-EINVAL` for a CCM index out of range.
pub fn arm_with(
    m32: &impl Mmio,
    m16: &impl Mmio16,
    wdog_base: usize,
    ccm_base: usize,
    timeout_ms: u32,
    ext_reset: bool,
    out: &mut YdImxWdogArm,
) -> i32 {
    *out = YdImxWdogArm::default();
    match ccm::enable_root(m32, ccm_base, ccm::root::WDOG) {
        Ok(true) => out.flags |= YD_IMX_WDOG_ROOT_ENABLED,
        Ok(false) => {}
        Err(e) => return e,
    }
    match ccm::gate_on(m32, ccm_base, ccm::gate::WDOG1) {
        Ok(true) => out.flags |= YD_IMX_WDOG_GATE_OPENED,
        Ok(false) => {}
        Err(e) => return e,
    }
    match imx_wdog::arm(m16, wdog_base, timeout_ms, ext_reset) {
        Ok(Armed { wcr_found, wmcr_found, wcr_now, was_running, cleared_power_down }) => {
            out.wcr_found = wcr_found;
            out.wmcr_found = wmcr_found;
            out.wcr_now = wcr_now;
            if was_running {
                out.flags |= YD_IMX_WDOG_WAS_RUNNING;
            }
            if cleared_power_down {
                out.flags |= YD_IMX_WDOG_CLEARED_PDE;
            }
            out.timeout_ms = imx_wdog::timeout_ms_of_wt(imx_wdog::wt_of(wcr_now));
            0
        }
        Err(Refused { why, wcr_found, wmcr_found, read_back }) => {
            out.wcr_found = wcr_found;
            out.wmcr_found = wmcr_found;
            out.wcr_now = read_back;
            if wcr_found & imx_wdog::wcr::WDE != 0 {
                out.flags |= YD_IMX_WDOG_WAS_RUNNING;
            }
            out.refusal = why as u32;
            -EIO
        }
    }
}

/// Arm WDOG1 (`wdog_base`, the shim's `device_map()` of 0x30280000) for
/// `timeout_ms`, through the CCM at `ccm_base` (its mapping of 0x30380000).
/// `ext_reset` != 0 sets `WCR.WDT` (the board resets through `WDOG_B`).
///
/// # Safety
/// Both bases are mapped device memory covering the block; `out` is valid.
#[no_mangle]
pub unsafe extern "C" fn yd_imx_wdog_arm(
    wdog_base: usize,
    ccm_base: usize,
    timeout_ms: u32,
    ext_reset: u32,
    out: *mut YdImxWdogArm,
) -> i32 {
    if out.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let hw = unsafe { Hw::new() };
    // SAFETY: the caller's contract.
    unsafe { arm_with(&hw, &hw, wdog_base, ccm_base, timeout_ms, ext_reset != 0, &mut *out) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ip::imx_wdog::{wcr, Refusal, WCR, WMCR, WSR};
    use crate::model::{Model, CCM};
    use std::cell::RefCell;

    /// A WDOG that keeps what is written; `dead` reads 0 from WCR.
    struct Wdog {
        regs: RefCell<[u16; 5]>,
        dead: bool,
    }
    impl Mmio16 for Wdog {
        fn read16(&self, addr: usize) -> u16 {
            if self.dead && addr == W + WCR {
                return 0;
            }
            self.regs.borrow()[(addr - W) / 2]
        }
        fn write16(&self, addr: usize, val: u16) {
            if addr != W + WSR {
                self.regs.borrow_mut()[(addr - W) / 2] = val;
            }
        }
    }
    const W: usize = 0x3028_0000;

    #[test]
    fn cold_root_and_gate_are_opened_then_the_watchdog_armed() {
        let m = Model::new();
        let w = Wdog { regs: RefCell::new([0x0030, 0, 0, 0, 0x0001]), dead: false };
        let mut out = YdImxWdogArm::default();
        assert_eq!(arm_with(&m, &w, W, m.base(CCM), 60_000, true, &mut out), 0);
        // ROOT[114] ENABLE through its SET alias (0xb904), then CCGR83_SET = 3
        assert_eq!(m.writes(), vec![(CCM, 0xb904, 0x1000_0000), (CCM, 0x4534, 0x3)]);
        assert_eq!(
            out,
            YdImxWdogArm {
                wcr_found: 0x30,
                wmcr_found: 1,
                wcr_now: 0x773f,
                _pad: 0,
                flags: YD_IMX_WDOG_CLEARED_PDE | YD_IMX_WDOG_ROOT_ENABLED | YD_IMX_WDOG_GATE_OPENED,
                refusal: 0,
                timeout_ms: 60_000,
            }
        );
        assert_eq!(w.regs.borrow()[WMCR / 2], 0);
    }

    #[test]
    fn a_running_watchdog_with_its_clock_on_writes_nothing_to_the_ccm() {
        let m = Model::new();
        m.poke(CCM, 0xb900, 0x1000_0000);
        m.poke(CCM, 0x4530, 0x3);
        let w = Wdog { regs: RefCell::new([0x3b3d, 0, 0, 0, 0]), dead: false };
        let mut out = YdImxWdogArm::default();
        assert_eq!(arm_with(&m, &w, W, m.base(CCM), 60_000, true, &mut out), 0);
        assert!(m.writes().is_empty());
        assert_eq!(out.flags, YD_IMX_WDOG_WAS_RUNNING);
        assert_eq!((out.wcr_found, out.wcr_now, out.timeout_ms), (0x3b3d, 0x773d, 60_000));
    }

    #[test]
    fn a_refusal_is_minus_eio_with_its_number() {
        let m = Model::new();
        let w = Wdog { regs: RefCell::new([0; 5]), dead: true };
        let mut out = YdImxWdogArm::default();
        assert_eq!(arm_with(&m, &w, W, m.base(CCM), 60_000, true, &mut out), -EIO);
        assert_eq!(out.refusal, Refusal::NotEnabled as u32);
        assert_eq!(out.timeout_ms, 0);
        assert_eq!(out.wcr_now & wcr::WDE, 0);
    }

    #[test]
    fn the_c_header_has_the_same_numbers() {
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let def = |name: &str| -> u32 {
            let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            let v = l.split_whitespace().nth(2).expect(name);
            let v = v.trim_start_matches('(').trim_end_matches(')').trim_end_matches('u');
            if let Some(shift) = l.split("<<").nth(1) {
                let n: String = shift.trim().chars().take_while(char::is_ascii_digit).collect();
                1 << n.parse::<u32>().unwrap()
            } else {
                v.parse().unwrap_or_else(|_| panic!("{name}: {v}"))
            }
        };
        for (name, v) in [
            ("YD_IMX_WDOG_WAS_RUNNING", YD_IMX_WDOG_WAS_RUNNING),
            ("YD_IMX_WDOG_CLEARED_PDE", YD_IMX_WDOG_CLEARED_PDE),
            ("YD_IMX_WDOG_ROOT_ENABLED", YD_IMX_WDOG_ROOT_ENABLED),
            ("YD_IMX_WDOG_GATE_OPENED", YD_IMX_WDOG_GATE_OPENED),
            ("YD_IMX_WDOG_REFUSED_TIMEOUT", Refusal::Timeout as u32),
            ("YD_IMX_WDOG_REFUSED_PDE_STUCK", Refusal::PowerDownStuck as u32),
            ("YD_IMX_WDOG_REFUSED_NOT_ENABLED", Refusal::NotEnabled as u32),
            ("YD_IMX_WDOG_REFUSED_WT_READ_BACK", Refusal::TimeoutReadBack as u32),
            ("YD_IMX_WDOG_REFUSED_WDT_READ_BACK", Refusal::ExtResetReadBack as u32),
        ] {
            assert_eq!(def(name), v, "{name}");
        }
        assert!(H.contains("_Static_assert(sizeof(struct yd_imx_wdog_arm) == 20,"));
    }
}
