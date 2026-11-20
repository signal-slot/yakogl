// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Verdin iMX95's WDOG3 (ip::wdog32) for the board glue
//! driver/zephyr/lib/board/verdin_imx95/verdin_imx95_wdog.c. No device, no
//! env: the glue calls [`yd_wdog32_arm`] once at PRE_KERNEL_1 with its
//! mapping, interrupts off, and feeds with [`yd_wdog32_refresh`].
//! Declarations: include/yakogl_drivers.h ("NXP WDOG32").

use crate::errno::{EINVAL, EIO};
use crate::ip::wdog32::{self, Armed, Refused};
use crate::mmio::Hw;
use core::mem::{offset_of, size_of};

/// `flags` of [`YdWdog32Arm`].
pub const YD_WDOG32_WAS_RUNNING: u32 = 1 << 0;

/// `struct yd_wdog32_arm`: what the arming found and left.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdWdog32Arm {
    /// `CS` and `TOVAL` as found, before anything was written.
    pub cs_found: u32,
    pub toval_found: u32,
    /// `CS` and `TOVAL` read back after arming (on a refusal: the reads the
    /// refusal is about, 0 when not read).
    pub cs_now: u32,
    pub toval_now: u32,
    pub flags: u32,
    /// 0, or the [`wdog32::Refusal`] number (`YD_WDOG32_REFUSED_*`).
    pub refusal: u32,
    /// The timeout the watchdog now runs with, from the read-back `TOVAL`
    /// (0 on a refusal).
    pub timeout_ms: u32,
    /// The reads of `CS` that saw `ULK`, then `RCS`, still clear.
    pub ulk_polls: u32,
    pub rcs_polls: u32,
}

const _: () = assert!(size_of::<YdWdog32Arm>() == 36);
const _: () = assert!(offset_of!(YdWdog32Arm, cs_found) == 0);
const _: () = assert!(offset_of!(YdWdog32Arm, cs_now) == 8);
const _: () = assert!(offset_of!(YdWdog32Arm, flags) == 16);
const _: () = assert!(offset_of!(YdWdog32Arm, refusal) == 20);
const _: () = assert!(offset_of!(YdWdog32Arm, timeout_ms) == 24);
const _: () = assert!(offset_of!(YdWdog32Arm, rcs_polls) == 32);

/// [`wdog32::arm`] into `out`. 0, or `-EIO` when the WDOG32 refused (the
/// reason in `out.refusal`).
pub fn arm_into(
    m: &impl crate::mmio::Mmio,
    m16: &impl crate::mmio::Mmio16,
    base: usize,
    timeout_ms: u32,
    ext_reset: bool,
    polls: u32,
    out: &mut YdWdog32Arm,
) -> i32 {
    *out = YdWdog32Arm::default();
    match wdog32::arm(m, m16, base, timeout_ms, ext_reset, polls) {
        Ok(Armed { cs_found, toval_found, cs_now, toval_now, was_running, ulk_polls, rcs_polls }) => {
            *out = YdWdog32Arm {
                cs_found,
                toval_found,
                cs_now,
                toval_now,
                flags: if was_running { YD_WDOG32_WAS_RUNNING } else { 0 },
                refusal: 0,
                timeout_ms: wdog32::timeout_ms_of_toval(toval_now),
                ulk_polls,
                rcs_polls,
            };
            0
        }
        Err(Refused { why, cs_found, toval_found, cs_now, toval_now }) => {
            *out = YdWdog32Arm {
                cs_found,
                toval_found,
                cs_now,
                toval_now,
                flags: if cs_found & wdog32::cs::EN != 0 { YD_WDOG32_WAS_RUNNING } else { 0 },
                refusal: why as u32,
                ..YdWdog32Arm::default()
            };
            -EIO
        }
    }
}

/// Arm the WDOG32 at `base` (the glue's `device_map()` of it) for
/// `timeout_ms`; `ext_reset` != 0 sets `CS.INT` (the System Manager takes the
/// timeout as WDOG3's interrupt); `polls` bounds each wait in reads of `CS`.
/// Interrupts off around the call.
///
/// # Safety
/// `base` is mapped device memory covering the block; `out` is valid.
#[no_mangle]
pub unsafe extern "C" fn yd_wdog32_arm(
    base: usize,
    timeout_ms: u32,
    ext_reset: u32,
    polls: u32,
    out: *mut YdWdog32Arm,
) -> i32 {
    if out.is_null() || base == 0 {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let hw = unsafe { Hw::new() };
    // SAFETY: the caller's contract.
    unsafe { arm_into(&hw, &hw, base, timeout_ms, ext_reset != 0, polls, &mut *out) }
}

/// Refresh (feed) the WDOG32 at `base`: one write. Any context.
///
/// # Safety
/// `base` is mapped device memory covering the block.
#[no_mangle]
pub unsafe extern "C" fn yd_wdog32_refresh(base: usize) {
    // SAFETY: the caller's contract.
    let hw = unsafe { Hw::new() };
    wdog32::refresh(&hw, base);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ip::wdog32::{cs, Refusal, CNT, CS, REFRESH, TOVAL, UNLOCK};
    use crate::mmio::{Mmio, Mmio16};
    use std::cell::RefCell;

    const W: usize = 0x4249_0000;

    /// A WDOG32 that keeps what is written and answers ULK and RCS at once;
    /// `dead` reads 0 everywhere.
    struct Wdog32 {
        regs: RefCell<[u32; 3]>,
        dead: bool,
    }
    impl Mmio for Wdog32 {
        fn read32(&self, addr: usize) -> u32 {
            if self.dead {
                return 0;
            }
            self.regs.borrow()[(addr - W) / 4]
        }
        fn write32(&self, addr: usize, val: u32) {
            match addr - W {
                CNT if val == UNLOCK => self.regs.borrow_mut()[0] |= cs::ULK,
                CNT => assert_eq!(val, REFRESH),
                CS => self.regs.borrow_mut()[0] = val | cs::RCS,
                TOVAL => self.regs.borrow_mut()[2] = val,
                off => panic!("{off:#x}"),
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }
    impl Mmio16 for Wdog32 {
        fn read16(&self, _: usize) -> u16 {
            unreachable!()
        }
        fn write16(&self, addr: usize, _: u16) {
            // the 16-bit unlock: only the dead block (CS reads 0) gets it
            assert!(self.dead && addr == W + CNT);
        }
    }

    #[test]
    fn armed_is_reported_with_the_timeout_read_back() {
        let w = Wdog32 { regs: RefCell::new([0x31e0, 0, 3750]), dead: false };
        let mut out = YdWdog32Arm::default();
        assert_eq!(arm_into(&w, &w, W, 60_000, true, 10, &mut out), 0);
        assert_eq!(
            out,
            YdWdog32Arm {
                cs_found: 0x31e0,
                toval_found: 3750,
                cs_now: 0x31e3 | cs::RCS,
                toval_now: 7500,
                flags: YD_WDOG32_WAS_RUNNING,
                refusal: 0,
                timeout_ms: 60_000,
                ulk_polls: 0,
                rcs_polls: 0,
            }
        );
    }

    #[test]
    fn a_refusal_is_minus_eio_with_its_number() {
        let w = Wdog32 { regs: RefCell::new([0; 3]), dead: true };
        let mut out = YdWdog32Arm { timeout_ms: 1, ..YdWdog32Arm::default() };
        assert_eq!(arm_into(&w, &w, W, 60_000, true, 3, &mut out), -EIO);
        // CS reads 0 (no CMD32EN: the 16-bit unlock), and ULK never shows.
        assert_eq!(out.refusal, Refusal::Unlock as u32);
        assert_eq!((out.timeout_ms, out.flags), (0, 0));
    }

    #[test]
    fn a_null_out_or_base_is_einval() {
        // SAFETY: refused before any access.
        assert_eq!(unsafe { yd_wdog32_arm(W, 60_000, 1, 1, core::ptr::null_mut()) }, -EINVAL);
        let mut out = YdWdog32Arm::default();
        // SAFETY: refused before any access.
        assert_eq!(unsafe { yd_wdog32_arm(0, 60_000, 1, 1, &mut out) }, -EINVAL);
    }

    #[test]
    fn the_c_header_has_the_same_numbers() {
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let def = |name: &str| -> u32 {
            let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            if let Some(shift) = l.split("<<").nth(1) {
                let n: String = shift.trim().chars().take_while(char::is_ascii_digit).collect();
                1 << n.parse::<u32>().unwrap()
            } else {
                let v = l.split_whitespace().nth(2).expect(name);
                v.trim_end_matches('u').parse().unwrap_or_else(|_| panic!("{name}: {v}"))
            }
        };
        for (name, v) in [
            ("YD_WDOG32_WAS_RUNNING", YD_WDOG32_WAS_RUNNING),
            ("YD_WDOG32_REFUSED_TIMEOUT", Refusal::Timeout as u32),
            ("YD_WDOG32_REFUSED_UNLOCK", Refusal::Unlock as u32),
            ("YD_WDOG32_REFUSED_RECONFIG", Refusal::Reconfig as u32),
            ("YD_WDOG32_REFUSED_CS_READ_BACK", Refusal::CsReadBack as u32),
            ("YD_WDOG32_REFUSED_TOVAL_READ_BACK", Refusal::ToValReadBack as u32),
        ] {
            assert_eq!(def(name), v, "{name}");
        }
        assert!(H.contains("_Static_assert(sizeof(struct yd_wdog32_arm) == 36,"));
    }
}
