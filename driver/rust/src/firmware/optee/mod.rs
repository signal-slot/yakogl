// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! An OP-TEE message-protocol client and an SCMI transport over it: the
//! SCMI pseudo-TA's PROCESS_MSG_CHANNEL (command 4), as ST's OP-TEE 4.0
//! serves SCMI on the STM32MP2 (`linaro,scmi-optee`).
//!
//! Sources: OP-TEE `optee_os`, ST's tag `4.0.0-stm32mp-r3.1` (commit
//! `0063536697f1`; BSD-2-Clause, cited, not copied): the SMC ABI
//! (`core/arch/arm/include/sm/optee_smc.h`), the message layout
//! (`core/include/optee_msg.h`), its parsing (`core/arch/arm/kernel/
//! thread_optee_smc.c`, `core/tee/entry_std.c`, `core/kernel/msg_param.c`)
//! and the pseudo-TA (`core/pta/scmi.c`, `lib/libutee/include/
//! pta_scmi_client.h`). docs/notes/pre-study-stm32mp257f-dk.md section 4.
//!
//! The sequence: EXCHANGE_CAPABILITIES (a fast call), then standard calls
//! that each carry a `struct optee_msg_arg` at the physical address of a
//! page of [`Shm`]: REGISTER_SHM of the data page, OPEN_SESSION to the SCMI
//! pseudo-TA with the login `TEE_LOGIN_REE_KERNEL` (the only one it takes,
//! `scmi.c:381-396`), GET_CHANNEL_HANDLE (command 3) with the capability
//! MSG_HEADER, then one PROCESS_MSG_CHANNEL per SCMI message. The message
//! is the bare SCMI one (header + payload, no SMT header) at offset 0 of the
//! data page; the reply comes back at offset 0x80.
//!
//! Memory: no allocator, no statics. The caller owns a [`Shm`] (three 4 KiB
//! pages, page-aligned; a board declares it static) and gives its physical
//! address. OP-TEE maps it as cached normal memory; the image must too.
//!
//! RPCs: a standard call may come back with an RPC request in a0 instead of
//! its result; the client answers and resumes (RETURN_FROM_RPC) until the
//! call ends, at most [`MAX_RPC`] times. A foreign interrupt is just resumed;
//! an allocation is refused (the RPC argument lives in the arg page, which
//! the capability RPC_ARG lets OP-TEE use); a free needs nothing; an RPC
//! command is answered NOT_IMPLEMENTED; an OCALL2 with an error.
//!
//! Logging: every SMC, every message frame and every SCMI request and reply
//! goes to [`Smc::log`] as an [`Event`]; the board layer prints them.

use super::scmi;

#[cfg(test)]
mod tests;

/// SMC function ids (`optee_smc.h:36-47`: fast `0x80000000 | owner << 24 | n`,
/// standard `owner << 24 | n`; owner 50 the trusted OS, 63 its API).
pub const SMC_CALLS_UID: u32 = 0xbf00_ff01;
pub const SMC_CALLS_REVISION: u32 = 0xbf00_ff03;
pub const SMC_GET_OS_UUID: u32 = 0xb200_0000;
pub const SMC_GET_OS_REVISION: u32 = 0xb200_0001;
pub const SMC_EXCHANGE_CAPABILITIES: u32 = 0xb200_0009;
pub const SMC_CALL_WITH_ARG: u32 = 0x3200_0004;
pub const SMC_RETURN_FROM_RPC: u32 = 0x3200_0003;
pub const SMC_CALL_WITH_RPC_ARG: u32 = 0x3200_0012;
/// ST's variant with foreign interrupts kept masked (`optee_smc.h:649-651`),
/// offered with [`SEC_CAP_PM_NOIRQ`].
pub const SMC_CALL_WITH_RPC_ARG_NSEC_NOIRQ: u32 = 0x3200_0039;

/// a0 on return (`optee_smc.h:814-824`).
pub const RETURN_OK: u32 = 0;
pub const RETURN_ETHREAD_LIMIT: u32 = 1;
pub const RETURN_EBUSY: u32 = 2;
pub const RETURN_ERESUME: u32 = 3;
pub const RETURN_EBADADDR: u32 = 4;
pub const RETURN_EBADCMD: u32 = 5;
pub const RETURN_ENOMEM: u32 = 6;
pub const RETURN_ENOTAVAIL: u32 = 7;
pub const RETURN_UNKNOWN_FUNCTION: u32 = 0xffff_ffff;

/// An RPC request in a0: the prefix, the function in the low half
/// (`optee_smc.h:686-811`).
pub const RPC_PREFIX: u32 = 0xffff_0000;
pub const RPC_ALLOC: u32 = 0;
pub const RPC_FREE: u32 = 2;
pub const RPC_FOREIGN_INTR: u32 = 4;
pub const RPC_CMD: u32 = 5;
pub const RPC_OCALL2: u32 = 2048;

/// Capabilities (`optee_smc.h:300-321`).
pub const NSEC_CAP_UNIPROCESSOR: u32 = 1 << 0;
pub const SEC_CAP_HAVE_RESERVED_SHM: u32 = 1 << 0;
pub const SEC_CAP_DYNAMIC_SHM: u32 = 1 << 2;
pub const SEC_CAP_MEMREF_NULL: u32 = 1 << 4;
pub const SEC_CAP_ASYNC_NOTIF: u32 = 1 << 5;
pub const SEC_CAP_RPC_ARG: u32 = 1 << 6;
pub const SEC_CAP_PM_NOIRQ: u32 = 1 << 15;

/// OPTEE_MSG_UID_0..3, CALLS_UID's answer (`optee_msg.h:251-254`).
pub const MSG_UID: [u32; 4] = [0x384f_b3e0, 0xe7f8_11e3, 0xaf63_0002, 0xa5d5_c51b];

/// Message commands (`optee_msg.h:335-340`).
pub const MSG_CMD_OPEN_SESSION: u32 = 0;
pub const MSG_CMD_INVOKE_COMMAND: u32 = 1;
pub const MSG_CMD_CLOSE_SESSION: u32 = 2;
pub const MSG_CMD_REGISTER_SHM: u32 = 4;
pub const MSG_CMD_UNREGISTER_SHM: u32 = 5;

/// Parameter attributes (`optee_msg.h:21-71`).
pub const ATTR_NONE: u64 = 0x0;
pub const ATTR_VALUE_INPUT: u64 = 0x1;
pub const ATTR_VALUE_OUTPUT: u64 = 0x2;
pub const ATTR_VALUE_INOUT: u64 = 0x3;
pub const ATTR_RMEM_INPUT: u64 = 0x5;
pub const ATTR_RMEM_OUTPUT: u64 = 0x6;
pub const ATTR_TMEM_OUTPUT: u64 = 0xa;
pub const ATTR_META: u64 = 1 << 8;
pub const ATTR_NONCONTIG: u64 = 1 << 9;

/// `TEE_LOGIN_REE_KERNEL` (`tee_api_defines_extensions.h:138`).
pub const LOGIN_REE_KERNEL: u64 = 0x8000_0000;

/// TEE_Result values met here (`tee_api_defines.h:107-123`).
pub const TEE_SUCCESS: u32 = 0;
pub const TEE_ERROR_GENERIC: u32 = 0xffff_0000;
pub const TEE_ERROR_ACCESS_DENIED: u32 = 0xffff_0001;
pub const TEE_ERROR_BAD_PARAMETERS: u32 = 0xffff_0006;
pub const TEE_ERROR_NOT_IMPLEMENTED: u32 = 0xffff_0009;
pub const TEE_ERROR_NOT_SUPPORTED: u32 = 0xffff_000a;

/// The SCMI pseudo-TA: its UUID a8cfe406-d4f5-4a2e-9f8d-a25dc754c099 as the
/// 16 octets OPEN_SESSION carries (RFC 4122 order, `pta_scmi_client.h:8`,
/// `core/tee/uuid.c:23-30`), its commands (`:19-69`) and capabilities
/// (`:100-109`).
pub const PTA_SCMI_UUID: [u8; 16] =
    [0xa8, 0xcf, 0xe4, 0x06, 0xd4, 0xf5, 0x4a, 0x2e, 0x9f, 0x8d, 0xa2, 0x5d, 0xc7, 0x54, 0xc0, 0x99];
pub const PTA_SCMI_CMD_CAPABILITIES: u32 = 0;
pub const PTA_SCMI_CMD_GET_CHANNEL_HANDLE: u32 = 3;
pub const PTA_SCMI_CMD_PROCESS_MSG_CHANNEL: u32 = 4;
pub const PTA_SCMI_CAPS_SMT_HEADER: u32 = 1 << 0;
pub const PTA_SCMI_CAPS_MSG_HEADER: u32 = 1 << 1;

/// The noncontiguous page size (`optee_msg.h:97`) and our page.
pub const PAGE: usize = 4096;
/// Size of a message with `n` parameters (`optee_msg.h:226`).
pub const fn arg_size(n: usize) -> usize {
    32 + 32 * n
}
/// The RPC argument OP-TEE expects after ours: THREAD_RPC_MAX_NUM_PARAMS 4
/// (`core/include/kernel/thread.h:35`).
pub const RPC_ARG_PARAMS: usize = 4;

/// The cookie the data page is registered under.
pub const SHM_COOKIE: u64 = 1;
/// Offsets in the data page: the SCMI request, the reply buffer.
pub const DATA_TX: usize = 0;
pub const DATA_RX: usize = 0x80;
pub const DATA_MAX: usize = 0x80;

/// RPC requests one call answers before it gives up.
pub const MAX_RPC: u32 = 4096;

/// The memory shared with OP-TEE: the message page (our `optee_msg_arg`,
/// then the RPC argument), the page list naming the data page, the data
/// page. Physically contiguous, at the address given to [`Client::new`].
#[repr(C, align(4096))]
pub struct Shm {
    pub arg: [u8; PAGE],
    pub list: [u8; PAGE],
    pub data: [u8; PAGE],
}

impl Shm {
    pub const fn new() -> Self {
        Shm { arg: [0; PAGE], list: [0; PAGE], data: [0; PAGE] }
    }
}

impl Default for Shm {
    fn default() -> Self {
        Self::new()
    }
}

/// What the board layer may print, in the order it happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event<'a> {
    /// One SMC: x0..x7 set, x0..x7 returned.
    Smc {
        call: [u64; 8],
        ret: [u64; 8],
    },
    /// A message frame before the call (`done` false) and after (true).
    Msg {
        done: bool,
        frame: &'a [u8],
    },
    /// The SCMI request, and its reply.
    ScmiRequest(&'a [u8]),
    ScmiReply(&'a [u8]),
}

/// The SMC conduit, injectable: the board's is [`HwSmc`], the host tests'
/// a scripted fake. `shm` is the memory the call may read and write (the
/// fake answers through it; the hardware conduit hands its address to the
/// `asm!` so the compiler sees it change).
pub trait Smc {
    fn smc(&mut self, regs: [u64; 8], shm: &mut Shm) -> [u64; 8];
    /// Each [`Event`]; the board layer prints it. Default: nothing.
    fn log(&mut self, _ev: &Event<'_>) {}
}

/// The SMC instruction (SMCCC: x0..x7 arguments and results, x8..x17 may be
/// clobbered).
#[cfg(target_arch = "aarch64")]
pub struct HwSmc(());

#[cfg(target_arch = "aarch64")]
impl HwSmc {
    /// # Safety
    /// The secure monitor routes owner-50/63 SMCs to OP-TEE (the caller knows
    /// the board), and the [`Shm`] handed to each call is non-secure memory
    /// at the physical address given to [`Client::new`].
    pub const unsafe fn new() -> Self {
        HwSmc(())
    }
}

#[cfg(target_arch = "aarch64")]
impl Smc for HwSmc {
    fn smc(&mut self, regs: [u64; 8], shm: &mut Shm) -> [u64; 8] {
        let mut x = regs;
        let p = shm as *mut Shm as u64;
        // SAFETY: HwSmc::new's contract. The secure side writes only `shm`,
        // whose address the asm takes (no `nomem`), and x0..x17.
        unsafe {
            core::arch::asm!(
                "smc #0",
                inout("x0") x[0], inout("x1") x[1], inout("x2") x[2], inout("x3") x[3],
                inout("x4") x[4], inout("x5") x[5], inout("x6") x[6], inout("x7") x[7],
                inout("x8") p => _,
                out("x9") _, out("x10") _, out("x11") _, out("x12") _,
                out("x13") _, out("x14") _, out("x15") _, out("x16") _, out("x17") _,
                options(nostack),
            );
        }
        x
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A call came back with this a0 (`RETURN_*`).
    Smc(u32),
    /// The message came back with `ret` (a TEE_Result) from `origin`.
    Tee { ret: u32, origin: u32 },
    /// More than [`MAX_RPC`] RPC requests in one call.
    RpcLimit,
    /// CALLS_UID is not OP-TEE's, a capability needed is missing, or the
    /// shared memory is not page-aligned.
    Unsupported,
    /// A reply does not fit its buffer.
    Malformed,
    /// No session (call [`Client::open`] first).
    NotOpen,
    /// The SCMI request is longer than [`DATA_MAX`].
    TooLong,
}

impl Error {
    /// The code [`scmi::Error::Transport`] carries: the TEE_Result of a
    /// refused message (0xffffxxxx, negative), a0 of a refused call
    /// (1..7, or -1 for an unknown function), 0x100 + n for the others.
    pub fn code(&self) -> i32 {
        match *self {
            Error::Smc(a0) => a0 as i32,
            Error::Tee { ret, .. } => ret as i32,
            Error::RpcLimit => 0x100,
            Error::Unsupported => 0x101,
            Error::Malformed => 0x102,
            Error::NotOpen => 0x103,
            Error::TooLong => 0x104,
        }
    }
}

/// One `optee_msg_param` (`optee_msg.h:176`): the attribute and three words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Param {
    pub attr: u64,
    pub a: u64,
    pub b: u64,
    pub c: u64,
}

impl Param {
    pub const fn new(attr: u64, a: u64, b: u64, c: u64) -> Self {
        Param { attr, a, b, c }
    }
}

/// The secure side's answer to EXCHANGE_CAPABILITIES: a1 the capabilities,
/// a2 the highest notification value, a3 the RPC argument's parameter count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub sec: u32,
    pub notif_max: u32,
    pub rpc_params: u32,
}

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn rd64(b: &[u8], o: usize) -> u64 {
    rd32(b, o) as u64 | (rd32(b, o + 4) as u64) << 32
}

fn wr32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn wr64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

/// A client of one OP-TEE, then of its SCMI pseudo-TA on one channel.
pub struct Client<'a, S: Smc> {
    pub smc: S,
    shm: &'a mut Shm,
    pa: u64,
    /// The standard call [`Client::exchange_capabilities`] chose.
    call: u32,
    registered: bool,
    session: Option<u32>,
    channel: u32,
    /// The last error of a [`scmi::Transport::transfer`], for the log.
    pub last_error: Option<Error>,
}

impl<'a, S: Smc> Client<'a, S> {
    /// `pa` is the physical address of `shm` (page-aligned).
    pub fn new(smc: S, shm: &'a mut Shm, pa: u64) -> Self {
        Client { smc, shm, pa, call: SMC_CALL_WITH_ARG, registered: false, session: None, channel: 0, last_error: None }
    }

    /// The standard call in use: CALL_WITH_ARG until the capabilities are
    /// exchanged, then CALL_WITH_RPC_ARG (its NSEC_NOIRQ variant when the
    /// secure side offers [`SEC_CAP_PM_NOIRQ`]).
    pub fn call_id(&self) -> u32 {
        self.call
    }

    pub fn session(&self) -> Option<u32> {
        self.session
    }

    fn arg_pa(&self) -> u64 {
        self.pa
    }

    fn list_pa(&self) -> u64 {
        self.pa + PAGE as u64
    }

    fn data_pa(&self) -> u64 {
        self.pa + 2 * PAGE as u64
    }

    fn raw(&mut self, call: [u64; 8]) -> [u64; 8] {
        let ret = self.smc.smc(call, self.shm);
        self.smc.log(&Event::Smc { call, ret });
        ret
    }

    /// A fast call: x0 = `id`, x1..x3 as given.
    fn fast(&mut self, id: u32, a1: u32) -> [u64; 8] {
        self.raw([id as u64, a1 as u64, 0, 0, 0, 0, 0, 0])
    }

    /// CALLS_UID: Ok when the trusted OS is OP-TEE.
    pub fn calls_uid(&mut self) -> Result<(), Error> {
        let r = self.fast(SMC_CALLS_UID, 0);
        let uid = [r[0] as u32, r[1] as u32, r[2] as u32, r[3] as u32];
        if uid == MSG_UID {
            Ok(())
        } else {
            Err(Error::Unsupported)
        }
    }

    /// CALLS_REVISION: (major, minor), 2.0 for this ABI.
    pub fn calls_revision(&mut self) -> (u32, u32) {
        let r = self.fast(SMC_CALLS_REVISION, 0);
        (r[0] as u32, r[1] as u32)
    }

    /// EXCHANGE_CAPABILITIES (`entry_fast.c:67-123`). `nsec` may only be
    /// [`NSEC_CAP_UNIPROCESSOR`] or 0. Requires DYNAMIC_SHM (the platform has
    /// no reserved shared memory) and picks the standard call.
    pub fn exchange_capabilities(&mut self, nsec: u32) -> Result<Caps, Error> {
        let r = self.fast(SMC_EXCHANGE_CAPABILITIES, nsec);
        if r[0] as u32 != RETURN_OK {
            return Err(Error::Smc(r[0] as u32));
        }
        let caps = Caps { sec: r[1] as u32, notif_max: r[2] as u32, rpc_params: r[3] as u32 };
        if caps.sec & SEC_CAP_DYNAMIC_SHM == 0 {
            return Err(Error::Unsupported);
        }
        self.call = if caps.sec & SEC_CAP_RPC_ARG == 0 {
            SMC_CALL_WITH_ARG
        } else if caps.sec & SEC_CAP_PM_NOIRQ != 0 {
            SMC_CALL_WITH_RPC_ARG_NSEC_NOIRQ
        } else {
            SMC_CALL_WITH_RPC_ARG
        };
        Ok(caps)
    }

    /// Write a message frame at the start of the arg page; the rest of the
    /// page (the RPC argument) is cleared.
    fn write_arg(&mut self, cmd: u32, func: u32, session: u32, params: &[Param]) {
        let a = &mut self.shm.arg;
        a.fill(0);
        wr32(a, 0, cmd);
        wr32(a, 4, func);
        wr32(a, 8, session);
        wr32(a, 28, params.len() as u32);
        for (i, p) in params.iter().enumerate() {
            let o = arg_size(i);
            wr64(a, o, p.attr);
            wr64(a, o + 8, p.a);
            wr64(a, o + 16, p.b);
            wr64(a, o + 24, p.c);
        }
    }

    /// Parameter `i` of the frame as OP-TEE left it.
    pub fn param(&self, i: usize) -> Param {
        let a = &self.shm.arg;
        let o = arg_size(i);
        Param::new(rd64(a, o), rd64(a, o + 8), rd64(a, o + 16), rd64(a, o + 24))
    }

    /// Answer one RPC request in `r` and return the registers to resume
    /// with (`optee_smc.h:695-811`).
    fn rpc(&mut self, mut r: [u64; 8], nparams: usize) -> [u64; 8] {
        let func = (r[0] as u32) & !RPC_PREFIX;
        r[0] = SMC_RETURN_FROM_RPC as u64;
        match func {
            // No memory to give: a1:a2 the address, a4:a5 the cookie, all 0.
            RPC_ALLOC => {
                r[1] = 0;
                r[2] = 0;
                r[4] = 0;
                r[5] = 0;
            }
            // The RPC argument we supplied (cookie 0): its `ret`.
            RPC_CMD if r[1] == 0 && r[2] == 0 && self.call != SMC_CALL_WITH_ARG => {
                wr32(&mut self.shm.arg, arg_size(nparams) + 20, TEE_ERROR_NOT_IMPLEMENTED);
            }
            RPC_OCALL2 => {
                r[1] = 0;
                r[2] = TEE_ERROR_NOT_SUPPORTED as u64;
            }
            // FOREIGN_INTR, FREE, an unknown one: resume as is.
            _ => {}
        }
        r
    }

    /// One standard call with the frame in the arg page, RPCs answered;
    /// Ok when a0 and the frame's `ret` are 0.
    fn call(&mut self, nparams: usize) -> Result<(), Error> {
        let n = arg_size(nparams);
        let mut frame = [0u8; arg_size(RPC_ARG_PARAMS)];
        frame[..n].copy_from_slice(&self.shm.arg[..n]);
        self.smc.log(&Event::Msg { done: false, frame: &frame[..n] });
        let pa = self.arg_pa();
        let mut regs = [self.call as u64, pa >> 32, pa & 0xffff_ffff, 0, 0, 0, 0, 0];
        let mut rpcs = 0;
        loop {
            let r = self.raw(regs);
            let a0 = r[0] as u32;
            if a0 != RETURN_UNKNOWN_FUNCTION && a0 & RPC_PREFIX == RPC_PREFIX {
                rpcs += 1;
                if rpcs > MAX_RPC {
                    return Err(Error::RpcLimit);
                }
                regs = self.rpc(r, nparams);
                continue;
            }
            if a0 != RETURN_OK {
                return Err(Error::Smc(a0));
            }
            break;
        }
        frame[..n].copy_from_slice(&self.shm.arg[..n]);
        self.smc.log(&Event::Msg { done: true, frame: &frame[..n] });
        let ret = rd32(&frame, 20);
        if ret != TEE_SUCCESS {
            return Err(Error::Tee { ret, origin: rd32(&frame, 24) });
        }
        Ok(())
    }

    /// REGISTER_SHM of the data page under [`SHM_COOKIE`]: one
    /// TMEM_OUTPUT|NONCONTIG parameter naming the page list
    /// (`entry_std.c:483-504`, `msg_param.c:36-149`).
    pub fn register_shm(&mut self) -> Result<(), Error> {
        let data = self.data_pa();
        self.shm.list.fill(0);
        wr64(&mut self.shm.list, 0, data);
        let p = Param::new(ATTR_TMEM_OUTPUT | ATTR_NONCONTIG, self.list_pa(), PAGE as u64, SHM_COOKIE);
        self.write_arg(MSG_CMD_REGISTER_SHM, 0, 0, &[p]);
        self.call(1)?;
        self.registered = true;
        Ok(())
    }

    /// UNREGISTER_SHM (`entry_std.c:506-518`).
    pub fn unregister_shm(&mut self) -> Result<(), Error> {
        self.write_arg(MSG_CMD_UNREGISTER_SHM, 0, 0, &[Param::new(ATTR_RMEM_INPUT, 0, 0, SHM_COOKIE)]);
        self.registered = false;
        self.call(1)
    }

    /// OPEN_SESSION to the TA `uuid` with the login REE_KERNEL: the two meta
    /// parameters (`entry_std.c:315-350`). Returns the session id.
    pub fn open_session(&mut self, uuid: &[u8; 16]) -> Result<u32, Error> {
        let mut lo = [0u8; 8];
        let mut hi = [0u8; 8];
        lo.copy_from_slice(&uuid[..8]);
        hi.copy_from_slice(&uuid[8..]);
        let ta = Param::new(ATTR_META | ATTR_VALUE_INPUT, u64::from_le_bytes(lo), u64::from_le_bytes(hi), 0);
        let client = Param::new(ATTR_META | ATTR_VALUE_INPUT, 0, 0, LOGIN_REE_KERNEL);
        self.write_arg(MSG_CMD_OPEN_SESSION, 0, 0, &[ta, client]);
        self.call(2)?;
        Ok(rd32(&self.shm.arg, 8))
    }

    /// CLOSE_SESSION (`entry_std.c:400-418`).
    pub fn close_session(&mut self, session: u32) -> Result<(), Error> {
        self.write_arg(MSG_CMD_CLOSE_SESSION, 0, session, &[]);
        self.call(0)
    }

    /// INVOKE_COMMAND `func` of the open session.
    pub fn invoke(&mut self, func: u32, params: &[Param]) -> Result<(), Error> {
        let s = self.session.ok_or(Error::NotOpen)?;
        self.write_arg(MSG_CMD_INVOKE_COMMAND, func, s, params);
        self.call(params.len())
    }

    /// The pseudo-TA's CAPABILITIES (command 0): its mask.
    pub fn pta_capabilities(&mut self) -> Result<u32, Error> {
        self.invoke(PTA_SCMI_CMD_CAPABILITIES, &[Param::new(ATTR_VALUE_OUTPUT, 0, 0, 0)])?;
        Ok(self.param(0).a as u32)
    }

    /// GET_CHANNEL_HANDLE (command 3) for `channel` with the MSG_HEADER
    /// capability: the handle (the channel id itself under ST's SCP-firmware
    /// server, `scmi.c:373-376`), kept for [`Client::process`].
    pub fn get_channel_handle(&mut self, channel: u32) -> Result<u32, Error> {
        let p = Param::new(ATTR_VALUE_INOUT, channel as u64, PTA_SCMI_CAPS_MSG_HEADER as u64, 0);
        self.invoke(PTA_SCMI_CMD_GET_CHANNEL_HANDLE, &[p])?;
        self.channel = self.param(0).a as u32;
        Ok(self.channel)
    }

    /// The whole opening: capabilities, the data page registered, the
    /// session to the SCMI pseudo-TA, the channel's handle.
    pub fn open(&mut self, nsec: u32, channel: u32) -> Result<(), Error> {
        if self.pa & (PAGE as u64 - 1) != 0 {
            return Err(Error::Unsupported);
        }
        self.exchange_capabilities(nsec)?;
        self.register_shm()?;
        self.session = Some(self.open_session(&PTA_SCMI_UUID)?);
        self.get_channel_handle(channel)?;
        Ok(())
    }

    /// Close the session and unregister the data page; both are tried.
    pub fn close(&mut self) -> Result<(), Error> {
        let a = match self.session.take() {
            Some(s) => self.close_session(s),
            None => Ok(()),
        };
        let b = if self.registered { self.unregister_shm() } else { Ok(()) };
        a.and(b)
    }

    /// One SCMI message through PROCESS_MSG_CHANNEL (command 4,
    /// `scmi.c:131-180`): value(channel), RMEM_INPUT the request at
    /// [`DATA_TX`], RMEM_OUTPUT the reply buffer at [`DATA_RX`]. Returns the
    /// reply's length, the output parameter's size.
    pub fn process(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, Error> {
        if self.session.is_none() {
            return Err(Error::NotOpen);
        }
        if tx.len() > DATA_MAX {
            return Err(Error::TooLong);
        }
        self.smc.log(&Event::ScmiRequest(tx));
        self.shm.data[DATA_TX..DATA_TX + tx.len()].copy_from_slice(tx);
        self.shm.data[DATA_RX..DATA_RX + DATA_MAX].fill(0);
        let params = [
            Param::new(ATTR_VALUE_INPUT, self.channel as u64, 0, 0),
            Param::new(ATTR_RMEM_INPUT, DATA_TX as u64, tx.len() as u64, SHM_COOKIE),
            Param::new(ATTR_RMEM_OUTPUT, DATA_RX as u64, DATA_MAX as u64, SHM_COOKIE),
        ];
        self.invoke(PTA_SCMI_CMD_PROCESS_MSG_CHANNEL, &params)?;
        let n = self.param(2).b as usize;
        if n > DATA_MAX || n > rx.len() {
            return Err(Error::Malformed);
        }
        rx[..n].copy_from_slice(&self.shm.data[DATA_RX..DATA_RX + n]);
        self.smc.log(&Event::ScmiReply(&rx[..n]));
        Ok(n)
    }
}

/// The SCMI codec's transport: [`scmi::Client::new`] takes an open
/// [`Client`].
impl<S: Smc> scmi::Transport for Client<'_, S> {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
        self.process(tx, rx).map_err(|e| {
            self.last_error = Some(e);
            e.code()
        })
    }
}
