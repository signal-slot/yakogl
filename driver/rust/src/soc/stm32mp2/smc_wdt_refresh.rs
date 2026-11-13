// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! An opt-in refresh of OP-TEE's `arm,smc-wdt` watchdog on the
//! STM32MP257F-DK, for an image that runs longer than the watchdog's 32 s
//! (plan #362, issues #7 and #1281). Off by default: no board-layer code calls
//! it; an image that wants it calls [`Refresh::init`] (from C,
//! [`yd_mp2_wdtr_init`]), the probe `tools/probes/stm32mp257f_dk/mp2_wdt` does.
//!
//! Host tests: `cargo test -p yakogl-drivers --features stm32mp2
//! smc_wdt_refresh` (the module is compiled for every host test run; the C
//! ABI only for aarch64 with the feature `stm32mp2`).
//!
//! BL2 starts the IWDG before U-Boot and OP-TEE serves it over SMC: the node
//! is `arm,smc-wdt` with `arm,smc-id = <0xbc000000>` and `timeout-sec = <32>`
//! (`stm32mp251.dtsi:78-82`, `stm32mp257f-dk.dts:216-219`;
//! docs/notes/pre-study-stm32mp257f-dk.md, section 1). Nothing pets it after
//! U-Boot's `go`, so an image is reset about 32 s after the last pet, after
//! OP-TEE's panic at about 27 s.
//!
//! The call layout is that of [`super::smc_wdt`] (the same `CMD_*` and conduit):
//! x0 the function id, x1 the command, x2 its argument, x3 0; the answer's
//! x0 is a 32-bit PSCI-style code, x1 and x2 its values. Sources:
//!
//! - Linux v6.6 `drivers/watchdog/arm_smc_wdt.c`: the commands INIT 0,
//!   SET_TIMEOUT 1, ENABLE 2, PET 3, GET_TIMELEFT 4 (`:23-29`); the call
//!   `arm_smccc_smc(fid, call, arg, 0, ...)` (`:42-43`); INIT is the first
//!   call and answers the minimum timeout in x1 and the maximum in x2
//!   (`:128-140`); ENABLE 1 starts, ENABLE 0 stops (`:79-87`); GET_TIMELEFT's
//!   seconds are x1 (`:59-67`).
//! - ST's OP-TEE `4.0.0-stm32mp-r3.1`, `core/drivers/wdt/watchdog_sm.c`
//!   (`__wdt_sm_handler`): INIT fills the bounds SET_TIMEOUT checks, so a
//!   SET_TIMEOUT before INIT is refused with INVALID_PARAMETERS (`:27-53`);
//!   ENABLE takes only 0 or 1 (`:54-64`); PET always answers success
//!   (`:65-68`); GET_TIMELEFT answers DISABLED (-8) while the watchdog is
//!   stopped (`:70-82`). Codes: `core/arch/arm/include/sm/psci.h`
//!   (`PSCI_RET_*`).
//! - The same tree's `core/drivers/stm32_iwdg.c`: the early interrupt comes
//!   `IWDG_ETIMEOUT_SEC` (5 s) before the timeout and `configure_timeout`
//!   arms it only when `timeout - IWDG_ETIMEOUT_SEC >= 1`, so a timeout
//!   under 6 s runs without it; `stm32_iwdg_it_handler` panics
//!   (`panic("Watchdog")`) when no refresh came, which is the panic at about
//!   27 s with 32 s. `iwdg_wdt_init` answers INIT's bounds from the LSI rate.
//!   Stopping (ENABLE 0, `iwdg_wdt_stop`) releases the reset line and sets
//!   `nb_int` to `ULONG_MAX`, so the handler reloads the counter at every
//!   early interrupt, for ever: after it a hang is never reset and the board
//!   is stranded. Hence the rules: INIT first, timeouts of 6 s or more, never
//!   ENABLE 0.
//!
//! The rules hold by construction here: [`Refresh`] only comes out of
//! [`Refresh::init`], which sends INIT before anything else and refuses a
//! timeout under [`MIN_TIMEOUT_S`] without a call; [`Call`] has no way to say
//! ENABLE 0 ([`encode`] always sends ENABLE 1).

use super::smc_wdt::{Smc, CMD_ENABLE, CMD_GET_TIMELEFT, CMD_INIT, CMD_PET, CMD_SET_TIMEOUT};
use crate::errno::{EINVAL, EIO, ENODEV};
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// The DK's `arm,smc-id` (`stm32mp251.dtsi:78-82`).
pub const FUNC_ID_DK: u32 = 0xbc00_0000;
/// The DK's `timeout-sec`, the one Linux uses (`stm32mp257f-dk.dts:216-219`).
pub const TIMEOUT_DK_S: u32 = 32;
/// OP-TEE's early interrupt comes this long before the timeout
/// (`IWDG_ETIMEOUT_SEC`, `stm32_iwdg.c`).
pub const EARLY_S: u32 = 5;
/// The shortest timeout sent: one with OP-TEE's early interrupt armed.
pub const MIN_TIMEOUT_S: u32 = EARLY_S + 1;

/// One call to the watchdog service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    Init,
    /// The timeout in seconds.
    SetTimeout(u32),
    /// ENABLE 1. There is no ENABLE 0 (the module's documentation says why).
    Enable,
    Pet,
    GetTimeLeft,
}

impl Call {
    /// The command, x1.
    pub const fn cmd(self) -> u64 {
        match self {
            Call::Init => CMD_INIT,
            Call::SetTimeout(_) => CMD_SET_TIMEOUT,
            Call::Enable => CMD_ENABLE,
            Call::Pet => CMD_PET,
            Call::GetTimeLeft => CMD_GET_TIMELEFT,
        }
    }
}

/// x0..x3 of `call` to the service `func_id`.
pub const fn encode(func_id: u32, call: Call) -> [u64; 4] {
    let arg = match call {
        Call::SetTimeout(s) => s as u64,
        Call::Enable => 1,
        Call::Init | Call::Pet | Call::GetTimeLeft => 0,
    };
    [func_id as u64, call.cmd(), arg, 0]
}

/// The answer to `call`: x1 and x2, or the refusal. The code is the low 32
/// bits of x0, sign-extended (the upper half is not part of it).
pub fn decode(call: Call, ret: [u64; 4]) -> Result<[u64; 2], Error> {
    match ret[0] as u32 as i32 {
        0 => Ok([ret[1], ret[2]]),
        code => Err(Error::Firmware { cmd: call.cmd(), code }),
    }
}

/// `PSCI_RET_DISABLED`: GET_TIMELEFT while the watchdog is stopped.
pub const RET_DISABLED: i32 = -8;
/// `PSCI_RET_INTERNAL_FAILURE`: INIT without a watchdog driver behind it.
pub const RET_INTERNAL_FAILURE: i32 = -6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The service refused `cmd` with `code`.
    Firmware { cmd: u64, code: i32 },
    /// Under [`MIN_TIMEOUT_S`]: nothing was sent.
    TooShort { timeout_s: u32 },
    /// Outside the bounds INIT answered: nothing after INIT was sent.
    OutOfRange { timeout_s: u32, min_s: u64, max_s: u64 },
}

/// The watchdog after a successful [`Refresh::init`].
pub struct Refresh<S: Smc> {
    smc: S,
    func_id: u32,
    timeout_s: u32,
    min_s: u64,
    max_s: u64,
}

impl<S: Smc> Refresh<S> {
    /// INIT, then SET_TIMEOUT `timeout_s`, ENABLE 1 and PET, stopping at the
    /// first refusal. A `timeout_s` under [`MIN_TIMEOUT_S`] sends nothing; one
    /// outside INIT's bounds sends only INIT. ENABLE 1 on the running IWDG
    /// restarts nothing: it programs the timeout and arms the early interrupt
    /// (`stm32_iwdg.c` `iwdg_wdt_start`).
    pub fn init(smc: S, func_id: u32, timeout_s: u32) -> Result<Self, Error> {
        if timeout_s < MIN_TIMEOUT_S {
            return Err(Error::TooShort { timeout_s });
        }
        let mut w = Refresh { smc, func_id, timeout_s, min_s: 0, max_s: 0 };
        let [min_s, max_s] = w.call(Call::Init)?;
        if u64::from(timeout_s) < min_s || u64::from(timeout_s) > max_s {
            return Err(Error::OutOfRange { timeout_s, min_s, max_s });
        }
        w.min_s = min_s;
        w.max_s = max_s;
        w.call(Call::SetTimeout(timeout_s))?;
        w.call(Call::Enable)?;
        w.call(Call::Pet)?;
        Ok(w)
    }

    fn call(&mut self, call: Call) -> Result<[u64; 2], Error> {
        decode(call, self.smc.call(encode(self.func_id, call)))
    }

    /// PET. OP-TEE answers success whatever the state (`watchdog_sm.c:65-68`);
    /// [`Self::time_left`] tells whether it counts.
    pub fn pet(&mut self) -> Result<(), Error> {
        self.call(Call::Pet).map(|_| ())
    }

    /// GET_TIMELEFT: seconds until the reset, counted from the last pet.
    /// Refused with [`RET_DISABLED`] while the watchdog is stopped.
    pub fn time_left(&mut self) -> Result<u64, Error> {
        self.call(Call::GetTimeLeft).map(|[s, _]| s)
    }

    pub fn timeout_s(&self) -> u32 {
        self.timeout_s
    }

    /// INIT's bounds, seconds.
    pub fn bounds_s(&self) -> (u64, u64) {
        (self.min_s, self.max_s)
    }
}

/// `struct yd_mp2_wdtr`: what a C ABI call did.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YdMp2Wdtr {
    /// INIT's bounds (init), 0 otherwise.
    pub min_s: u64,
    pub max_s: u64,
    /// GET_TIMELEFT's seconds (time_left), 0 otherwise.
    pub left_s: u64,
    /// The last command sent (on `-EIO` the refusing one), or
    /// [`YD_MP2_WDTR_NO_CMD`].
    pub cmd: u32,
    /// Its code: 0, or the firmware's refusal.
    pub code: i32,
}

/// `cmd` of [`YdMp2Wdtr`] when no call was made.
pub const YD_MP2_WDTR_NO_CMD: u32 = u32::MAX;

impl Default for YdMp2Wdtr {
    fn default() -> Self {
        YdMp2Wdtr { min_s: 0, max_s: 0, left_s: 0, cmd: YD_MP2_WDTR_NO_CMD, code: 0 }
    }
}

const _: () = assert!(core::mem::size_of::<YdMp2Wdtr>() == 32);
const _: () = assert!(core::mem::offset_of!(YdMp2Wdtr, left_s) == 16);
const _: () = assert!(core::mem::offset_of!(YdMp2Wdtr, cmd) == 24);
const _: () = assert!(core::mem::offset_of!(YdMp2Wdtr, code) == 28);

/// The C ABI's state: the function id of a successful init, 0 before one
/// (0 is never a fast SMC id). PET and GET_TIMELEFT are refused without it,
/// so INIT comes first across the ABI too.
pub struct Session(AtomicU32);

impl Session {
    pub const fn new() -> Self {
        Session(AtomicU32::new(0))
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

/// The conduit, keeping the command of the last call.
struct Last<'a, S: Smc> {
    smc: S,
    cmd: &'a mut u32,
}

impl<S: Smc> Smc for Last<'_, S> {
    fn call(&mut self, args: [u64; 4]) -> [u64; 4] {
        *self.cmd = args[1] as u32;
        self.smc.call(args)
    }
}

fn errno(e: Error, out: &mut YdMp2Wdtr) -> i32 {
    match e {
        Error::Firmware { code, .. } => {
            out.code = code;
            -EIO
        }
        Error::TooShort { .. } => -EINVAL,
        Error::OutOfRange { min_s, max_s, .. } => {
            out.min_s = min_s;
            out.max_s = max_s;
            -EINVAL
        }
    }
}

/// [`Refresh::init`] for C: 0, `-EIO` (the refusing command and code in
/// `out`), `-EINVAL` for a timeout under [`MIN_TIMEOUT_S`] (no call) or out of
/// INIT's bounds (they are in `out`), or for `func_id` 0 (no call). On
/// success `session` keeps `func_id`; on a failure it is cleared.
pub fn init_with<S: Smc>(smc: S, session: &Session, func_id: u32, timeout_s: u32, out: &mut YdMp2Wdtr) -> i32 {
    *out = YdMp2Wdtr::default();
    session.0.store(0, Relaxed);
    if func_id == 0 {
        return -EINVAL;
    }
    let mut cmd = YD_MP2_WDTR_NO_CMD;
    let r = Refresh::init(Last { smc, cmd: &mut cmd }, func_id, timeout_s).map(|w| w.bounds_s());
    out.cmd = cmd;
    match r {
        Ok((min_s, max_s)) => {
            out.min_s = min_s;
            out.max_s = max_s;
            session.0.store(func_id, Relaxed);
            0
        }
        Err(e) => errno(e, out),
    }
}

/// `call` (PET or GET_TIMELEFT) on the session's watchdog; `-ENODEV` with no
/// call before a successful [`init_with`].
fn session_call<S: Smc>(smc: S, session: &Session, call: Call, out: &mut YdMp2Wdtr) -> i32 {
    *out = YdMp2Wdtr::default();
    let func_id = session.0.load(Relaxed);
    if func_id == 0 {
        return -ENODEV;
    }
    out.cmd = call.cmd() as u32;
    let mut smc = smc;
    match decode(call, smc.call(encode(func_id, call))) {
        Ok([v, _]) => {
            if call == Call::GetTimeLeft {
                out.left_s = v;
            }
            0
        }
        Err(e) => errno(e, out),
    }
}

/// [`Refresh::pet`] for C.
pub fn pet_with<S: Smc>(smc: S, session: &Session, out: &mut YdMp2Wdtr) -> i32 {
    session_call(smc, session, Call::Pet, out)
}

/// [`Refresh::time_left`] for C: the seconds in `out.left_s`.
pub fn time_left_with<S: Smc>(smc: S, session: &Session, out: &mut YdMp2Wdtr) -> i32 {
    session_call(smc, session, Call::GetTimeLeft, out)
}

#[cfg(all(target_arch = "aarch64", feature = "stm32mp2"))]
mod board {
    use super::*;
    use crate::soc::stm32mp2::smc_wdt::ArchSmc;

    static SESSION: Session = Session::new();

    /// `out` or a scratch record when C passed NULL.
    ///
    /// # Safety
    /// `out` is NULL or valid for writes for as long as `scratch` lives.
    unsafe fn or_scratch(out: *mut YdMp2Wdtr, scratch: &mut YdMp2Wdtr) -> &mut YdMp2Wdtr {
        // SAFETY: the caller's contract.
        unsafe { out.as_mut() }.unwrap_or(scratch)
    }

    /// [`init_with`] over the SMC instruction.
    ///
    /// # Safety
    /// [`ArchSmc::new`]'s: `func_id` is the board's `arm,smc-id` watchdog
    /// service. `out` is NULL or valid for writes.
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_wdtr_init(func_id: u32, timeout_s: u32, out: *mut YdMp2Wdtr) -> i32 {
        let mut s = YdMp2Wdtr::default();
        // SAFETY: the caller's contract.
        let (smc, out) = unsafe { (ArchSmc::new(), or_scratch(out, &mut s)) };
        init_with(smc, &SESSION, func_id, timeout_s, out)
    }

    /// [`pet_with`]: PET on the watchdog [`yd_mp2_wdtr_init`] set up.
    ///
    /// # Safety
    /// `out` is NULL or valid for writes.
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_wdtr_pet(out: *mut YdMp2Wdtr) -> i32 {
        let mut s = YdMp2Wdtr::default();
        // SAFETY: the caller's contract; the function id is the one init's
        // caller vouched for.
        let (smc, out) = unsafe { (ArchSmc::new(), or_scratch(out, &mut s)) };
        pet_with(smc, &SESSION, out)
    }

    /// [`time_left_with`]: GET_TIMELEFT, the seconds in `out->left_s`.
    ///
    /// # Safety
    /// As [`yd_mp2_wdtr_pet`].
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_wdtr_time_left(out: *mut YdMp2Wdtr) -> i32 {
        let mut s = YdMp2Wdtr::default();
        // SAFETY: as yd_mp2_wdtr_pet.
        let (smc, out) = unsafe { (ArchSmc::new(), or_scratch(out, &mut s)) };
        time_left_with(smc, &SESSION, out)
    }
}

#[cfg(test)]
mod tests {
    use super::super::smc_wdt::{RET_DENIED, RET_INVALID_PARAMS};
    use super::*;

    const F: u32 = FUNC_ID_DK;
    const FW: u64 = FUNC_ID_DK as u64;
    /// INIT's answer on the DK: LSI at 32 kHz, `3 * 1024 / rate` and
    /// `4096 * 1024 / rate` (`stm32_iwdg.c`, `iwdg_wdt_init`).
    const INIT_OK: [u64; 4] = [0, 0, 131, 0];

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

    fn refuse(code: i32) -> [u64; 4] {
        [u64::from(code as u32), 0, 0, 0]
    }

    #[test]
    fn encode_is_linux_layout() {
        assert_eq!(encode(F, Call::Init), [FW, 0, 0, 0]);
        assert_eq!(encode(F, Call::SetTimeout(32)), [FW, 1, 32, 0]);
        assert_eq!(encode(F, Call::Enable), [FW, 2, 1, 0]);
        assert_eq!(encode(F, Call::Pet), [FW, 3, 0, 0]);
        assert_eq!(encode(F, Call::GetTimeLeft), [FW, 4, 0, 0]);
        // x0 is the 32-bit id, zero-extended.
        assert_eq!(encode(0xbc00_0000, Call::Pet)[0], 0x0000_0000_bc00_0000);
    }

    #[test]
    fn encode_never_says_enable_zero() {
        let calls = [Call::Init, Call::SetTimeout(0), Call::Enable, Call::Pet, Call::GetTimeLeft];
        for c in calls {
            let r = encode(F, c);
            assert!(!(r[1] == CMD_ENABLE && r[2] == 0), "{c:?}");
        }
    }

    #[test]
    fn decode_code_is_low_32_bits() {
        assert_eq!(decode(Call::Init, [0xffff_ffff_0000_0000, 3, 131, 9]), Ok([3, 131]));
        assert_eq!(
            decode(Call::GetTimeLeft, [u64::MAX - 7, 0, 0, 0]),
            Err(Error::Firmware { cmd: CMD_GET_TIMELEFT, code: RET_DISABLED })
        );
        assert_eq!(decode(Call::Pet, refuse(RET_DENIED)), Err(Error::Firmware { cmd: CMD_PET, code: -3 }));
    }

    #[test]
    fn init_sends_init_first_then_timeout_enable_pet() {
        let mut calls = Vec::new();
        let w = Refresh::init(fake(&mut calls, vec![INIT_OK]), F, 32).unwrap();
        assert_eq!(w.bounds_s(), (0, 131));
        assert_eq!(w.timeout_s(), 32);
        drop(w);
        assert_eq!(calls, [[FW, 0, 0, 0], [FW, 1, 32, 0], [FW, 2, 1, 0], [FW, 3, 0, 0]]);
    }

    #[test]
    fn init_refuses_under_six_seconds_without_a_call() {
        for t in [0, 1, 5] {
            let mut calls = Vec::new();
            let r = Refresh::init(fake(&mut calls, vec![]), F, t);
            assert_eq!(r.err(), Some(Error::TooShort { timeout_s: t }));
            assert!(calls.is_empty());
        }
        let mut calls = Vec::new();
        assert!(Refresh::init(fake(&mut calls, vec![INIT_OK]), F, MIN_TIMEOUT_S).is_ok());
        assert_eq!(calls[1], [FW, 1, 6, 0]);
    }

    #[test]
    fn init_out_of_bounds_sends_only_init() {
        let mut calls = Vec::new();
        let r = Refresh::init(fake(&mut calls, vec![[0, 8, 131, 0]]), F, 7);
        assert_eq!(r.err(), Some(Error::OutOfRange { timeout_s: 7, min_s: 8, max_s: 131 }));
        let mut calls2 = Vec::new();
        let r = Refresh::init(fake(&mut calls2, vec![INIT_OK]), F, 132);
        assert_eq!(r.err(), Some(Error::OutOfRange { timeout_s: 132, min_s: 0, max_s: 131 }));
        assert_eq!(calls, [[FW, 0, 0, 0]]);
        assert_eq!(calls2, [[FW, 0, 0, 0]]);
    }

    #[test]
    fn init_stops_at_the_first_refusal() {
        let mut calls = Vec::new();
        let r = Refresh::init(fake(&mut calls, vec![refuse(RET_INTERNAL_FAILURE)]), F, 32);
        assert_eq!(r.err(), Some(Error::Firmware { cmd: CMD_INIT, code: RET_INTERNAL_FAILURE }));
        assert_eq!(calls.len(), 1);

        let mut calls = Vec::new();
        let r = Refresh::init(fake(&mut calls, vec![INIT_OK, refuse(RET_INVALID_PARAMS)]), F, 32);
        assert_eq!(r.err(), Some(Error::Firmware { cmd: CMD_SET_TIMEOUT, code: RET_INVALID_PARAMS }));
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn pet_and_time_left() {
        let mut calls = Vec::new();
        let replies = vec![INIT_OK, [0; 4], [0; 4], [0; 4], [0; 4], [0, 31, 0, 0], refuse(RET_DISABLED)];
        let mut w = Refresh::init(fake(&mut calls, replies), F, 32).unwrap();
        w.pet().unwrap();
        assert_eq!(w.time_left(), Ok(31));
        assert_eq!(w.time_left(), Err(Error::Firmware { cmd: CMD_GET_TIMELEFT, code: RET_DISABLED }));
        drop(w);
        assert_eq!(&calls[4..], [[FW, 3, 0, 0], [FW, 4, 0, 0], [FW, 4, 0, 0]]);
    }

    #[test]
    fn abi_pet_before_init_is_refused_without_a_call() {
        let s = Session::new();
        let mut calls = Vec::new();
        let mut out = YdMp2Wdtr::default();
        assert_eq!(pet_with(fake(&mut calls, vec![]), &s, &mut out), -ENODEV);
        assert_eq!(time_left_with(fake(&mut calls, vec![]), &s, &mut out), -ENODEV);
        assert!(calls.is_empty());
        assert_eq!(out, YdMp2Wdtr::default());
    }

    #[test]
    fn abi_init_then_pet_and_time_left() {
        let s = Session::new();
        let mut calls = Vec::new();
        let mut out = YdMp2Wdtr::default();
        assert_eq!(init_with(fake(&mut calls, vec![INIT_OK]), &s, F, 32, &mut out), 0);
        assert_eq!(out, YdMp2Wdtr { min_s: 0, max_s: 131, left_s: 0, cmd: CMD_PET as u32, code: 0 });
        assert_eq!(pet_with(fake(&mut calls, vec![]), &s, &mut out), 0);
        assert_eq!(out.cmd, CMD_PET as u32);
        assert_eq!(time_left_with(fake(&mut calls, vec![[0, 29, 0, 0]]), &s, &mut out), 0);
        assert_eq!(out, YdMp2Wdtr { min_s: 0, max_s: 0, left_s: 29, cmd: CMD_GET_TIMELEFT as u32, code: 0 });
        assert_eq!(&calls[4..], [[FW, 3, 0, 0], [FW, 4, 0, 0]]);
    }

    #[test]
    fn abi_failures() {
        let s = Session::new();
        let mut calls = Vec::new();
        let mut out = YdMp2Wdtr::default();
        assert_eq!(init_with(fake(&mut calls, vec![]), &s, F, 5, &mut out), -EINVAL);
        assert_eq!(init_with(fake(&mut calls, vec![]), &s, 0, 32, &mut out), -EINVAL);
        assert!(calls.is_empty());

        assert_eq!(init_with(fake(&mut calls, vec![[0, 8, 131, 0]]), &s, F, 7, &mut out), -EINVAL);
        assert_eq!(out, YdMp2Wdtr { min_s: 8, max_s: 131, left_s: 0, cmd: CMD_INIT as u32, code: 0 });

        let replies = vec![INIT_OK, [0; 4], refuse(RET_DENIED)];
        assert_eq!(init_with(fake(&mut calls, replies), &s, F, 32, &mut out), -EIO);
        assert_eq!(out, YdMp2Wdtr { min_s: 0, max_s: 0, left_s: 0, cmd: CMD_ENABLE as u32, code: RET_DENIED });
        // A failed init leaves no session.
        let n = calls.len();
        assert_eq!(pet_with(fake(&mut calls, vec![]), &s, &mut out), -ENODEV);
        assert_eq!(calls.len(), n);

        assert_eq!(init_with(fake(&mut calls, vec![INIT_OK]), &s, F, 32, &mut out), 0);
        assert_eq!(time_left_with(fake(&mut calls, vec![refuse(RET_DISABLED)]), &s, &mut out), -EIO);
        assert_eq!(out.code, RET_DISABLED);
    }

    /// The probe's header (`mp2_wdt/src/mp2_wdtr.h`) declares the C ABI; its
    /// numbers are these.
    #[test]
    fn abi_probe_header_has_the_same_numbers() {
        const H: &str = include_str!("../../../../../tools/probes/stm32mp257f_dk/mp2_wdt/src/mp2_wdtr.h");
        assert!(H.contains("#define MP2_WDTR_FUNC_ID_DK 0xbc000000u"));
        assert!(H.contains("#define MP2_WDTR_MIN_TIMEOUT_S 6u"));
        assert!(H.contains("#define YD_MP2_WDTR_NO_CMD 0xffffffffu"));
        assert!(H.contains("_Static_assert(sizeof(struct yd_mp2_wdtr) == 32,"));
        assert!(H.contains("_Static_assert(offsetof(struct yd_mp2_wdtr, left_s) == 16,"));
        assert!(H.contains("_Static_assert(offsetof(struct yd_mp2_wdtr, cmd) == 24,"));
        assert!(H.contains("_Static_assert(offsetof(struct yd_mp2_wdtr, code) == 28,"));
        assert_eq!((FUNC_ID_DK, MIN_TIMEOUT_S, YD_MP2_WDTR_NO_CMD), (0xbc00_0000, 6, u32::MAX));
    }
}
