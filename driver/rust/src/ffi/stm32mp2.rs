// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! OP-TEE's `arm,smc-wdt` watchdog on the STM32MP257F-DK's A35
//! (soc::stm32mp2::smc_wdt) and PSCI `SYSTEM_RESET`, for a Zephyr board layer
//! that takes them from Rust instead of driver/zephyr/lib/board/stm32mp2's
//! own C (stm32mp2_wdog.c sends the same calls). No device, no env: each
//! entry is one [`SmcWdt`] operation with the board's `arm,smc-id` as
//! `func_id`. Declarations: include/yakogl_drivers.h ("STM32MP2 OP-TEE
//! watchdog"). Plan #322, issues #7 and #1281.

#[cfg(any(target_arch = "aarch64", test))]
use crate::errno::{EINVAL, EIO};
#[cfg(any(target_arch = "aarch64", test))]
use crate::soc::stm32mp2::smc_wdt::{Error, Smc, SmcWdt};
use core::mem::{offset_of, size_of};

/// `cmd` of [`YdMp2Wdt`] when no call was made.
pub const YD_MP2_WDT_NO_CMD: u32 = u32::MAX;

/// `struct yd_mp2_wdt`: the last call an entry made and its answer.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YdMp2Wdt {
    /// x1 of the answer: GET_TIMELEFT's seconds, INIT's value; 0 otherwise
    /// or on a refusal.
    pub value: u64,
    /// The last command sent (`smc_wdt::CMD_*`; on `-EIO` the one that
    /// refused), [`YD_MP2_WDT_NO_CMD`] when none was.
    pub cmd: u32,
    /// Its code: 0, or the firmware's refusal (`smc_wdt::RET_*`).
    pub code: i32,
}

impl Default for YdMp2Wdt {
    fn default() -> Self {
        YdMp2Wdt { value: 0, cmd: YD_MP2_WDT_NO_CMD, code: 0 }
    }
}

const _: () = assert!(size_of::<YdMp2Wdt>() == 16);
const _: () = assert!(offset_of!(YdMp2Wdt, value) == 0);
const _: () = assert!(offset_of!(YdMp2Wdt, cmd) == 8);
const _: () = assert!(offset_of!(YdMp2Wdt, code) == 12);

/// The conduit, keeping the command of the last call (x1).
#[cfg(any(target_arch = "aarch64", test))]
struct Last<'a, S: Smc> {
    smc: S,
    cmd: &'a mut u32,
}

#[cfg(any(target_arch = "aarch64", test))]
impl<S: Smc> Smc for Last<'_, S> {
    fn call(&mut self, args: [u64; 4]) -> [u64; 4] {
        *self.cmd = args[1] as u32;
        self.smc.call(args)
    }
}

/// `op` on a [`SmcWdt`] over `smc`, its outcome in `out`: 0, `-EIO` when the
/// firmware refused, `-EINVAL` for a zero timeout (no call made).
#[cfg(any(target_arch = "aarch64", test))]
fn run<S: Smc>(
    smc: S,
    func_id: u32,
    out: &mut YdMp2Wdt,
    op: impl FnOnce(&mut SmcWdt<Last<'_, S>>) -> Result<u64, Error>,
) -> i32 {
    let mut cmd = YD_MP2_WDT_NO_CMD;
    let r = op(&mut SmcWdt::new(Last { smc, cmd: &mut cmd }, func_id));
    *out = YdMp2Wdt { cmd, ..YdMp2Wdt::default() };
    match r {
        Ok(value) => {
            out.value = value;
            0
        }
        Err(Error::Firmware { code }) => {
            out.code = code;
            -EIO
        }
        Err(Error::BadTimeout) => -EINVAL,
    }
}

#[cfg(any(target_arch = "aarch64", test))]
fn arm_with<S: Smc>(smc: S, func_id: u32, timeout_s: u32, out: &mut YdMp2Wdt) -> i32 {
    run(smc, func_id, out, |w| w.arm(timeout_s).map(|()| 0))
}

#[cfg(any(target_arch = "aarch64", test))]
fn ping_with<S: Smc>(smc: S, func_id: u32, out: &mut YdMp2Wdt) -> i32 {
    run(smc, func_id, out, |w| w.ping().map(|()| 0))
}

#[cfg(any(target_arch = "aarch64", test))]
fn time_left_with<S: Smc>(smc: S, func_id: u32, out: &mut YdMp2Wdt) -> i32 {
    run(smc, func_id, out, |w| w.time_left())
}

#[cfg(any(target_arch = "aarch64", test))]
fn disable_with<S: Smc>(smc: S, func_id: u32, out: &mut YdMp2Wdt) -> i32 {
    run(smc, func_id, out, |w| w.disable().map(|()| 0))
}

#[cfg(target_arch = "aarch64")]
mod board {
    use super::*;
    use crate::soc::stm32mp2::smc_wdt::ArchSmc;

    /// `out` or a scratch record when C passed NULL.
    ///
    /// # Safety
    /// `out` is NULL or valid for writes for as long as `scratch` lives.
    unsafe fn or_scratch(out: *mut YdMp2Wdt, scratch: &mut YdMp2Wdt) -> &mut YdMp2Wdt {
        // SAFETY: the caller's contract.
        unsafe { out.as_mut() }.unwrap_or(scratch)
    }

    /// [`SmcWdt::arm`]: SET_TIMEOUT `timeout_s`, ENABLE 1, PET, stopping at
    /// the first refusal. 0, `-EIO` (the refusing command and its code in
    /// `out`), `-EINVAL` for `timeout_s` 0 with no call made.
    ///
    /// # Safety
    /// [`ArchSmc::new`]'s: `func_id` is the board's `arm,smc-id` watchdog
    /// service. `out` is NULL or valid for writes.
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_wdt_arm(func_id: u32, timeout_s: u32, out: *mut YdMp2Wdt) -> i32 {
        let mut s = YdMp2Wdt::default();
        // SAFETY: the caller's contract.
        let (smc, out) = unsafe { (ArchSmc::new(), or_scratch(out, &mut s)) };
        arm_with(smc, func_id, timeout_s, out)
    }

    /// [`SmcWdt::ping`]: PET. 0 does not say the watchdog runs (OP-TEE
    /// answers PET with success when it is disabled too):
    /// [`yd_mp2_wdt_time_left`] tells.
    ///
    /// # Safety
    /// As [`yd_mp2_wdt_arm`].
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_wdt_ping(func_id: u32, out: *mut YdMp2Wdt) -> i32 {
        let mut s = YdMp2Wdt::default();
        // SAFETY: the caller's contract.
        let (smc, out) = unsafe { (ArchSmc::new(), or_scratch(out, &mut s)) };
        ping_with(smc, func_id, out)
    }

    /// [`SmcWdt::time_left`]: GET_TIMELEFT, the seconds in `out.value`.
    ///
    /// # Safety
    /// As [`yd_mp2_wdt_arm`].
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_wdt_time_left(func_id: u32, out: *mut YdMp2Wdt) -> i32 {
        let mut s = YdMp2Wdt::default();
        // SAFETY: the caller's contract.
        let (smc, out) = unsafe { (ArchSmc::new(), or_scratch(out, &mut s)) };
        time_left_with(smc, func_id, out)
    }

    /// [`SmcWdt::disable`]: ENABLE 0. Not for an image's run (the board
    /// layer never sends it, stm32mp2_wdog.c); for a probe that hands the
    /// board back some other way.
    ///
    /// # Safety
    /// As [`yd_mp2_wdt_arm`].
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_wdt_disable(func_id: u32, out: *mut YdMp2Wdt) -> i32 {
        let mut s = YdMp2Wdt::default();
        // SAFETY: the caller's contract.
        let (smc, out) = unsafe { (ArchSmc::new(), or_scratch(out, &mut s)) };
        disable_with(smc, func_id, out)
    }

    /// PSCI `SYSTEM_RESET` over SMC (PSCI 1.1 on the board, #1281). Returns
    /// only when the firmware did not reset, with the PSCI return code.
    /// Named apart from rk3588's `yd_psci_system_reset`, which a build with
    /// both features would also link.
    ///
    /// # Safety
    /// The board's firmware implements PSCI over SMC.
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_psci_system_reset() -> i32 {
        // SAFETY: the caller's contract.
        unsafe { crate::aarch64::psci_system_reset() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soc::stm32mp2::smc_wdt::{
        CMD_ENABLE, CMD_GET_TIMELEFT, CMD_PET, CMD_SET_TIMEOUT, RET_DENIED, RET_INVALID_PARAMS,
    };

    const F: u32 = 0xbc00_0000;

    /// Answers `replies` in order, then success; keeps the calls.
    struct Fake<'a> {
        calls: &'a mut Vec<[u64; 4]>,
        replies: Vec<[u64; 4]>,
    }
    impl Smc for Fake<'_> {
        fn call(&mut self, a: [u64; 4]) -> [u64; 4] {
            self.calls.push(a);
            if self.replies.is_empty() {
                [0; 4]
            } else {
                self.replies.remove(0)
            }
        }
    }
    fn fake(calls: &mut Vec<[u64; 4]>, replies: Vec<[u64; 4]>) -> Fake<'_> {
        Fake { calls, replies }
    }
    /// A refusal as the firmware puts it in x0: 32 bits, the upper half 0.
    fn refuse(code: i32) -> [u64; 4] {
        [u64::from(code as u32), 0, 0, 0]
    }

    #[test]
    fn smc_wdt_ffi_arm_sends_set_timeout_enable_pet() {
        let mut calls = Vec::new();
        let mut out = YdMp2Wdt::default();
        assert_eq!(arm_with(fake(&mut calls, vec![]), F, 32, &mut out), 0);
        let f = u64::from(F);
        assert_eq!(calls, vec![[f, CMD_SET_TIMEOUT, 32, 0], [f, CMD_ENABLE, 1, 0], [f, CMD_PET, 0, 0]]);
        assert_eq!(out, YdMp2Wdt { value: 0, cmd: CMD_PET as u32, code: 0 });
    }

    #[test]
    fn smc_wdt_ffi_a_refusal_is_minus_eio_with_the_command_and_code() {
        let mut calls = Vec::new();
        let mut out = YdMp2Wdt::default();
        let smc = fake(&mut calls, vec![[0; 4], refuse(RET_DENIED)]);
        assert_eq!(arm_with(smc, F, 32, &mut out), -EIO);
        assert_eq!(calls.len(), 2, "stops at the refusing ENABLE");
        assert_eq!(out, YdMp2Wdt { value: 0, cmd: CMD_ENABLE as u32, code: RET_DENIED });
    }

    #[test]
    fn smc_wdt_ffi_a_zero_timeout_is_minus_einval_with_no_call() {
        let mut calls = Vec::new();
        let mut out = YdMp2Wdt { value: 7, cmd: 1, code: 1 };
        assert_eq!(arm_with(fake(&mut calls, vec![]), F, 0, &mut out), -EINVAL);
        assert!(calls.is_empty());
        assert_eq!(out, YdMp2Wdt::default());
    }

    /// OP-TEE answers PET with success also when the watchdog is disabled
    /// (smc_wdt::SmcWdt::ping): the ping is 0, and only GET_TIMELEFT says
    /// whether it counts.
    #[test]
    fn smc_wdt_ffi_ping_succeeds_on_a_disabled_watchdog_time_left_tells() {
        let mut calls = Vec::new();
        let mut out = YdMp2Wdt::default();
        assert_eq!(ping_with(fake(&mut calls, vec![[0; 4]]), F, &mut out), 0);
        assert_eq!(out, YdMp2Wdt { value: 0, cmd: CMD_PET as u32, code: 0 });

        let smc = fake(&mut calls, vec![refuse(RET_INVALID_PARAMS)]);
        assert_eq!(time_left_with(smc, F, &mut out), -EIO);
        assert_eq!(out, YdMp2Wdt { value: 0, cmd: CMD_GET_TIMELEFT as u32, code: RET_INVALID_PARAMS });

        let smc = fake(&mut calls, vec![[0, 29, 0, 0]]);
        assert_eq!(time_left_with(smc, F, &mut out), 0);
        assert_eq!(out, YdMp2Wdt { value: 29, cmd: CMD_GET_TIMELEFT as u32, code: 0 });
    }

    #[test]
    fn smc_wdt_ffi_disable_is_enable_zero() {
        let mut calls = Vec::new();
        let mut out = YdMp2Wdt::default();
        assert_eq!(disable_with(fake(&mut calls, vec![]), F, &mut out), 0);
        assert_eq!(calls, vec![[u64::from(F), CMD_ENABLE, 0, 0]]);
    }

    #[test]
    fn smc_wdt_ffi_the_c_header_has_the_same_numbers() {
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        assert!(H.contains("#define YD_MP2_WDT_NO_CMD 0xffffffffu"));
        assert_eq!(YD_MP2_WDT_NO_CMD, 0xffff_ffff);
        assert!(H.contains("_Static_assert(sizeof(struct yd_mp2_wdt) == 16,"));
        assert!(H.contains("_Static_assert(offsetof(struct yd_mp2_wdt, cmd) == 8,"));
        assert!(H.contains("_Static_assert(offsetof(struct yd_mp2_wdt, code) == 12,"));
    }
}
