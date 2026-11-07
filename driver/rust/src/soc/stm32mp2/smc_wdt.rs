// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The `arm,smc-wdt` watchdog: OP-TEE's, reached by SMC. On the
//! STM32MP257F-DK ST's Linux uses it with a 32 s timeout (#1281); the
//! function id is the node's `arm,smc-id`, so it is a parameter here.
//!
//! The command set is that of Linux's `arm_smc_wdt` driver: x0 the function
//! id, x1 the command, x2 its argument; the answer is a signed 32-bit code in
//! x0 and a value in x1.
//!
//! Handoff, on the board:
//! `SmcWdt::new(unsafe { ArchSmc::new() }, fid)`, then [`SmcWdt::arm`] early
//! and [`SmcWdt::ping`] from the main loop.

/// `SMCWD_INIT`.
pub const CMD_INIT: u64 = 0;
/// `SMCWD_SET_TIMEOUT`: x2 is the timeout in seconds.
pub const CMD_SET_TIMEOUT: u64 = 1;
/// `SMCWD_ENABLE`: x2 is 1 to enable, 0 to disable.
pub const CMD_ENABLE: u64 = 2;
/// `SMCWD_PET`.
pub const CMD_PET: u64 = 3;
/// `SMCWD_GET_TIMELEFT`.
pub const CMD_GET_TIMELEFT: u64 = 4;

/// `SMCWD_NOT_SUPPORTED`.
pub const RET_NOT_SUPPORTED: i32 = -1;
/// `SMCWD_INVALID_PARAMS`.
pub const RET_INVALID_PARAMS: i32 = -2;
/// `SMCWD_DENIED`.
pub const RET_DENIED: i32 = -3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The firmware answered this non-zero code (32-bit, sign-extended).
    Firmware { code: i32 },
    /// A zero timeout, which is not sent.
    BadTimeout,
}

/// The SMC conduit, injectable: the board's is [`ArchSmc`], the host tests'
/// a scripted fake. Arguments and results are x0..x3.
pub trait Smc {
    fn call(&mut self, args: [u64; 4]) -> [u64; 4];
}

/// The SMC instruction.
#[cfg(target_arch = "aarch64")]
pub struct ArchSmc(());

#[cfg(target_arch = "aarch64")]
impl ArchSmc {
    /// # Safety
    /// The firmware implements SMCCC over SMC at EL3, and the function id
    /// given to [`SmcWdt::new`] with this conduit is the board's
    /// `arm,smc-id` watchdog service (never an id that reaches memory or
    /// other services).
    pub const unsafe fn new() -> Self {
        ArchSmc(())
    }
}

#[cfg(target_arch = "aarch64")]
impl Smc for ArchSmc {
    fn call(&mut self, args: [u64; 4]) -> [u64; 4] {
        let [mut x0, mut x1, mut x2, mut x3] = args;
        // SAFETY: ArchSmc::new's contract. The watchdog service reads
        // x0..x3 and answers in x0..x3; x4..x17 are marked clobbered, as
        // firmware::optee::HwSmc does, whatever SMCCC version the firmware
        // implements.
        unsafe {
            core::arch::asm!(
                "smc #0",
                inout("x0") x0, inout("x1") x1, inout("x2") x2, inout("x3") x3,
                out("x4") _, out("x5") _, out("x6") _, out("x7") _,
                out("x8") _, out("x9") _, out("x10") _, out("x11") _,
                out("x12") _, out("x13") _, out("x14") _, out("x15") _,
                out("x16") _, out("x17") _,
                options(nostack),
            );
        }
        [x0, x1, x2, x3]
    }
}

pub struct SmcWdt<S: Smc> {
    smc: S,
    func_id: u64,
}

impl<S: Smc> SmcWdt<S> {
    pub fn new(smc: S, func_id: u32) -> Self {
        SmcWdt { smc, func_id: u64::from(func_id) }
    }

    fn cmd(&mut self, cmd: u64, arg: u64) -> Result<u64, Error> {
        let r = self.smc.call([self.func_id, cmd, arg, 0]);
        // The code is 32 bits wide; the upper half is not part of it.
        let code = r[0] as u32 as i32;
        if code == 0 {
            Ok(r[1])
        } else {
            Err(Error::Firmware { code })
        }
    }

    /// `SMCWD_INIT`; the value is the firmware's answer in x1.
    pub fn init(&mut self) -> Result<u64, Error> {
        self.cmd(CMD_INIT, 0)
    }

    /// Set the timeout, enable, then pet once, in this order. Stops at the
    /// first refusal.
    pub fn arm(&mut self, timeout_s: u32) -> Result<(), Error> {
        if timeout_s == 0 {
            return Err(Error::BadTimeout);
        }
        self.cmd(CMD_SET_TIMEOUT, u64::from(timeout_s))?;
        self.cmd(CMD_ENABLE, 1)?;
        self.cmd(CMD_PET, 0)?;
        Ok(())
    }

    /// Pet. This does not tell whether the watchdog runs: OP-TEE's `PET`
    /// answers success even when it is disabled; use [`Self::time_left`].
    pub fn ping(&mut self) -> Result<(), Error> {
        self.cmd(CMD_PET, 0).map(|_| ())
    }

    /// Seconds left before the reset.
    pub fn time_left(&mut self) -> Result<u64, Error> {
        self.cmd(CMD_GET_TIMELEFT, 0)
    }

    pub fn disable(&mut self) -> Result<(), Error> {
        self.cmd(CMD_ENABLE, 0).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: u64 = 0x8200_0001;

    struct Fake {
        calls: Vec<[u64; 4]>,
        replies: Vec<[u64; 4]>,
    }

    impl Smc for Fake {
        fn call(&mut self, a: [u64; 4]) -> [u64; 4] {
            self.calls.push(a);
            if self.replies.is_empty() {
                [0; 4]
            } else {
                self.replies.remove(0)
            }
        }
    }

    fn fake(replies: Vec<[u64; 4]>) -> SmcWdt<Fake> {
        SmcWdt::new(Fake { calls: vec![], replies }, F as u32)
    }

    #[test]
    fn arm_sequence() {
        let mut w = fake(vec![]);
        w.arm(32).unwrap();
        assert_eq!(w.smc.calls, [[F, 1, 32, 0], [F, 2, 1, 0], [F, 3, 0, 0]]);
    }

    #[test]
    fn arm_zero_timeout_makes_no_call() {
        let mut w = fake(vec![]);
        assert_eq!(w.arm(0), Err(Error::BadTimeout));
        assert!(w.smc.calls.is_empty());
    }

    #[test]
    fn arm_stops_at_refusal() {
        let mut w = fake(vec![[0; 4], [RET_DENIED as u32 as u64, 0, 0, 0]]);
        assert_eq!(w.arm(32), Err(Error::Firmware { code: RET_DENIED }));
        assert_eq!(w.smc.calls.len(), 2);
    }

    #[test]
    fn code_is_32_bit() {
        let mut w = fake(vec![[u64::MAX, 0, 0, 0]]);
        assert_eq!(w.ping(), Err(Error::Firmware { code: -1 }));
        let mut w = fake(vec![[0xffff_ffff_0000_0000, 7, 0, 0]]);
        assert_eq!(w.time_left(), Ok(7));
    }

    #[test]
    fn commands() {
        let mut w = fake(vec![]);
        w.ping().unwrap();
        w.time_left().unwrap();
        w.init().unwrap();
        w.disable().unwrap();
        assert_eq!(w.smc.calls, [[F, 3, 0, 0], [F, 4, 0, 0], [F, 0, 0, 0], [F, 2, 0, 0]]);
    }
}
