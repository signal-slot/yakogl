// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests of the OP-TEE client against a scripted secure side.
//!
//! The vectors are mp2-scmi-vectors' (plan #322, `tools/optee/scmi_vectors.py`,
//! `tools/testdata/optee-scmi/*.txt`, derived from `optee_os` 4.0.0 and ST's
//! `4.0.0-stm32mp-r3.1`), copied verbatim with their names; illustrative
//! addresses: arg page 0x88000000, page list 0x88001000, data page
//! 0x88002000, cookie 1, session 1, channel 0. One deliberate difference: the
//! open-session login is TEE_LOGIN_REE_KERNEL, which ST's SCMI pseudo-TA
//! requires (`core/pta/scmi.c:381-396`), not the vectors' PUBLIC.

use super::*;
use std::collections::VecDeque;
use std::vec::Vec;

const PA: u64 = 0x8800_0000;

// smc.txt
const CALL_CALLS_UID: &str = "01ff00bf000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const RET_CALLS_UID: &str = "e0b34f3800000000e311f8e700000000020063af000000001bc5d5a5000000000000000000000000000000000000000000000000000000000000000000000000";
const CALL_CALLS_REVISION: &str = "03ff00bf000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const RET_CALLS_REVISION: &str = "02000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const CALL_EXCHANGE_CAPABILITIES: &str = "090000b2000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const RET_EXCHANGE_CAPABILITIES: &str = "00000000000000007400000000000000450000000000000004000000000000000000000000000000000000000000000000000000000000000000000000000000";
const CALL_WITH_RPC_ARG: &str = "12000032000000000000000000000000000000880000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const CALL_WITH_ARG: &str = "04000032000000000000000000000000000000880000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const RET_CALL_OK: &str = "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

// msg_arg.txt
const REGISTER_SHM_REQ: &str = "04000000000000000000000000000000000000000000000000000000010000000a02000000000000001000880000000000100000000000000100000000000000";
const REGISTER_SHM_REPLY: &str = "04000000000000000000000000000000000000000000000000000000010000000a02000000000000001000880000000000100000000000000100000000000000";
const UNREGISTER_SHM_REQ: &str = "05000000000000000000000000000000000000000000000000000000010000000500000000000000000000000000000000000000000000000100000000000000";
const UNREGISTER_SHM_REPLY: &str = "05000000000000000000000000000000000000000000000000000000010000000500000000000000000000000000000000000000000000000100000000000000";
const OPEN_SESSION_REQ: &str = "00000000000000000000000000000000000000000000000000000000020000000101000000000000a8cfe406d4f54a2e9f8da25dc754c09900000000000000000101000000000000000000000000000000000000000000000000000000000000";
const OPEN_SESSION_REPLY: &str = "00000000000000000100000000000000000000000000000004000000020000000101000000000000a8cfe406d4f54a2e9f8da25dc754c09900000000000000000101000000000000000000000000000000000000000000000000000000000000";
const OPEN_SESSION_UNKNOWN_UUID_REPLY: &str = "00000000000000000000000000000000000000000800ffff03000000020000000101000000000000a8cfe406d4f54a2e9f8da25dc754c06600000000000000000101000000000000000000000000000000000000000000000000000000000000";
const INVOKE_CAPABILITIES_REQ: &str = "01000000000000000100000000000000000000000000000000000000010000000200000000000000000000000000000000000000000000000000000000000000";
const INVOKE_CAPABILITIES_REPLY: &str = "01000000000000000100000000000000000000000000000004000000010000000200000000000000020000000000000000000000000000000000000000000000";
const INVOKE_GET_CHANNEL_HANDLE_REQ: &str = "01000000030000000100000000000000000000000000000000000000010000000300000000000000000000000000000002000000000000000000000000000000";
const INVOKE_GET_CHANNEL_HANDLE_REPLY: &str = "01000000030000000100000000000000000000000000000004000000010000000300000000000000000000000000000002000000000000000000000000000000";
const INVOKE_GET_CHANNEL_HANDLE_BAD_REPLY: &str = "01000000030000000100000000000000000000000600ffff04000000010000000300000000000000070000000000000002000000000000000000000000000000";
const CLOSE_SESSION_REQ: &str = "0200000000000000010000000000000000000000000000000000000000000000";
const CLOSE_SESSION_REPLY: &str = "0200000000000000010000000000000000000000000000000300000000000000";

// invoke_frames.txt
const INVOKE_PD_STATE_SET_ON_REQ: &str = "0100000004000000010000000000000000000000000000000000000003000000010000000000000000000000000000000000000000000000000000000000000005000000000000000000000000000000100000000000000001000000000000000600000000000000800000000000000080000000000000000100000000000000";
const INVOKE_PD_STATE_SET_ON_REPLY: &str = "0100000004000000010000000000000000000000000000000400000003000000010000000000000000000000000000000000000000000000000000000000000005000000000000000000000000000000100000000000000001000000000000000600000000000000800000000000000008000000000000000100000000000000";
const INVOKE_PERF_LEVEL_GET_REQ: &str = "0100000004000000010000000000000000000000000000000000000003000000010000000000000000000000000000000000000000000000000000000000000005000000000000000000000000000000080000000000000001000000000000000600000000000000800000000000000080000000000000000100000000000000";
const INVOKE_PERF_LEVEL_GET_REPLY: &str = "010000000400000001000000000000000000000000000000040000000300000001000000000000000000000000000000000000000000000000000000000000000500000000000000000000000000000008000000000000000100000000000000060000000000000080000000000000000c000000000000000100000000000000";
const INVOKE_UNKNOWN_CHANNEL_REPLY: &str = "01000000040000000100000000000000000000000600ffff0400000003000000010000000000000007000000000000000000000000000000000000000000000005000000000000000000000000000000080000000000000001000000000000000600000000000000800000000000000080000000000000000100000000000000";

// scmi_msgs.txt
const REQ_PD_STATE_SET_ON: &str = "04440400000000000000000000000000";
const REP_PD_STATE_SET_ON: &str = "0444040000000000";
const REQ_PERF_LEVEL_GET: &str = "084c040001000000";
const REP_PERF_LEVEL_GET: &str = "084c04000000000000350c00";
const REQ_PD_PROTOCOL_VERSION: &str = "00440400";
const REP_PD_PROTOCOL_VERSION: &str = "004404000000000000000200";

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn regs(s: &str) -> [u64; 8] {
    let b = hex(s);
    assert_eq!(b.len(), 64);
    core::array::from_fn(|i| rd64(&b, 8 * i))
}

/// The vectors' open-session frame with the login REE_KERNEL in param 1's c
/// (offset 32 + 32 + 24).
fn with_ree_kernel(s: &str) -> Vec<u8> {
    let mut b = hex(s);
    wr64(&mut b, 88, LOGIN_REE_KERNEL);
    b
}

/// One answer of the secure side: the registers, and what it writes into
/// the shared memory first.
#[derive(Default)]
struct Step {
    ret: [u64; 8],
    arg: Option<Vec<u8>>,
    rpc_frame: Option<(usize, Vec<u8>)>,
    rx: Option<Vec<u8>>,
}

fn ok() -> Step {
    Step::default()
}

fn ret(r: [u64; 8]) -> Step {
    Step { ret: r, ..Step::default() }
}

fn reply(frame: &str) -> Step {
    Step { arg: Some(hex(frame)), ..Step::default() }
}

fn reply_bytes(frame: Vec<u8>) -> Step {
    Step { arg: Some(frame), ..Step::default() }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Logged {
    Smc([u64; 8], [u64; 8]),
    Msg(bool, Vec<u8>),
    Req(Vec<u8>),
    Rep(Vec<u8>),
}

/// Records every register set and the shared pages as each SMC saw them,
/// then answers from the script.
#[derive(Default)]
struct Fake {
    script: VecDeque<Step>,
    calls: Vec<[u64; 8]>,
    args: Vec<Vec<u8>>,
    lists: Vec<Vec<u8>>,
    datas: Vec<Vec<u8>>,
    log: Vec<Logged>,
}

impl Smc for Fake {
    fn smc(&mut self, regs: [u64; 8], shm: &mut Shm) -> [u64; 8] {
        self.calls.push(regs);
        self.args.push(shm.arg.to_vec());
        self.lists.push(shm.list.to_vec());
        self.datas.push(shm.data.to_vec());
        let s = self.script.pop_front().expect("an SMC the script does not expect");
        if let Some(a) = s.arg {
            shm.arg[..a.len()].copy_from_slice(&a);
        }
        if let Some((o, f)) = s.rpc_frame {
            shm.arg[o..o + f.len()].copy_from_slice(&f);
        }
        if let Some(rx) = s.rx {
            shm.data[DATA_RX..DATA_RX + rx.len()].copy_from_slice(&rx);
        }
        s.ret
    }

    fn log(&mut self, ev: &Event<'_>) {
        self.log.push(match *ev {
            Event::Smc { call, ret } => Logged::Smc(call, ret),
            Event::Msg { done, frame } => Logged::Msg(done, frame.to_vec()),
            Event::ScmiRequest(b) => Logged::Req(b.to_vec()),
            Event::ScmiReply(b) => Logged::Rep(b.to_vec()),
        });
    }
}

fn fake(steps: Vec<Step>) -> Fake {
    Fake { script: steps.into(), ..Fake::default() }
}

/// The four calls of [`Client::open`] answered as the vectors say.
fn open_steps() -> Vec<Step> {
    vec![
        ret(regs(RET_EXCHANGE_CAPABILITIES)),
        reply(REGISTER_SHM_REPLY),
        reply_bytes(with_ree_kernel(OPEN_SESSION_REPLY)),
        reply(INVOKE_GET_CHANNEL_HANDLE_REPLY),
    ]
}

fn opened(shm: &mut Shm, more: Vec<Step>) -> Client<'_, Fake> {
    let mut steps = open_steps();
    steps.extend(more);
    let mut c = Client::new(fake(steps), shm, PA);
    c.open(0, 0).unwrap();
    c
}

#[test]
fn optee_smc_ids() {
    // optee_smc.h: fast 0x80000000 | owner << 24 | n; standard owner << 24 | n.
    assert_eq!(SMC_EXCHANGE_CAPABILITIES, 0x8000_0000 | 50 << 24 | 9);
    assert_eq!(SMC_CALL_WITH_ARG, 50 << 24 | 4);
    assert_eq!(SMC_RETURN_FROM_RPC, 50 << 24 | 3);
    assert_eq!(SMC_CALL_WITH_RPC_ARG, 50 << 24 | 18);
    assert_eq!(SMC_CALL_WITH_RPC_ARG_NSEC_NOIRQ, 50 << 24 | 57);
    assert_eq!(SMC_CALLS_UID, 0x8000_0000 | 63 << 24 | 0xff01);
    assert_eq!(regs(CALL_WITH_ARG)[0], SMC_CALL_WITH_ARG as u64);
    assert_eq!(core::mem::size_of::<Shm>(), 3 * PAGE);
    assert_eq!(core::mem::align_of::<Shm>(), PAGE);
    // The arg and the RPC arg after it fit the arg page.
    assert!(arg_size(3) + arg_size(RPC_ARG_PARAMS) <= PAGE);
}

#[test]
fn optee_calls_uid_and_revision() {
    let mut shm = Shm::new();
    let mut c = Client::new(fake(vec![ret(regs(RET_CALLS_UID)), ret(regs(RET_CALLS_REVISION))]), &mut shm, PA);
    assert_eq!(c.calls_uid(), Ok(()));
    assert_eq!(c.calls_revision(), (2, 0));
    assert_eq!(c.smc.calls, [regs(CALL_CALLS_UID), regs(CALL_CALLS_REVISION)]);
    let mut c = Client::new(fake(vec![ret(regs(RET_CALLS_REVISION))]), &mut shm, PA);
    assert_eq!(c.calls_uid(), Err(Error::Unsupported));
}

#[test]
fn optee_exchange_capabilities() {
    let mut shm = Shm::new();
    let mut c = Client::new(fake(vec![ret(regs(RET_EXCHANGE_CAPABILITIES))]), &mut shm, PA);
    assert_eq!(c.call_id(), SMC_CALL_WITH_ARG);
    assert_eq!(c.exchange_capabilities(0), Ok(Caps { sec: 0x74, notif_max: 69, rpc_params: 4 }));
    assert_eq!(c.smc.calls, [regs(CALL_EXCHANGE_CAPABILITIES)]);
    assert_eq!(c.call_id(), SMC_CALL_WITH_RPC_ARG);

    // ST's own tree adds PM_NOIRQ (0x8074, the pre-study note): the NOIRQ call.
    let mut r = regs(RET_EXCHANGE_CAPABILITIES);
    r[1] = 0x8074;
    let mut c = Client::new(fake(vec![ret(r)]), &mut shm, PA);
    c.exchange_capabilities(NSEC_CAP_UNIPROCESSOR).unwrap();
    assert_eq!(c.smc.calls[0][1], 1);
    assert_eq!(c.call_id(), SMC_CALL_WITH_RPC_ARG_NSEC_NOIRQ);

    // No RPC_ARG: the plain call.
    r[1] = 0x14;
    let mut c = Client::new(fake(vec![ret(r)]), &mut shm, PA);
    c.exchange_capabilities(0).unwrap();
    assert_eq!(c.call_id(), SMC_CALL_WITH_ARG);

    // No dynamic shared memory: nothing to talk through.
    r[1] = 0x70;
    let mut c = Client::new(fake(vec![ret(r)]), &mut shm, PA);
    assert_eq!(c.exchange_capabilities(0), Err(Error::Unsupported));

    // An unknown non-secure capability is refused with ENOTAVAIL.
    let mut c = Client::new(fake(vec![ret([RETURN_ENOTAVAIL as u64, 0, 0, 0, 0, 0, 0, 0])]), &mut shm, PA);
    assert_eq!(c.exchange_capabilities(2), Err(Error::Smc(RETURN_ENOTAVAIL)));
}

#[test]
fn optee_open_bytes() {
    let mut shm = Shm::new();
    let c = opened(&mut shm, vec![]);
    assert_eq!(c.session(), Some(1));
    let f = &c.smc;
    assert_eq!(f.calls.len(), 4);
    assert_eq!(f.calls[0], regs(CALL_EXCHANGE_CAPABILITIES));
    for r in &f.calls[1..] {
        assert_eq!(*r, regs(CALL_WITH_RPC_ARG));
    }
    let req = hex(REGISTER_SHM_REQ);
    assert_eq!(f.args[1][..req.len()], req[..]);
    // page_list_one_page: entry 0 the data page, then zeros (next_page_data 0).
    assert_eq!(f.lists[1][..8], 0x8800_2000u64.to_le_bytes());
    assert!(f.lists[1][8..].iter().all(|&b| b == 0));
    let req = with_ree_kernel(OPEN_SESSION_REQ);
    assert_eq!(f.args[2][..req.len()], req[..]);
    let req = hex(INVOKE_GET_CHANNEL_HANDLE_REQ);
    assert_eq!(f.args[3][..req.len()], req[..]);
    // Beyond the frame, the RPC argument's space is clear.
    assert!(f.args[3][req.len()..].iter().all(|&b| b == 0));
}

#[test]
fn optee_open_session_login_is_ree_kernel() {
    let b = with_ree_kernel(OPEN_SESSION_REQ);
    assert_eq!(rd64(&b, 64), ATTR_META | ATTR_VALUE_INPUT);
    assert_eq!(rd64(&b, 88), 0x8000_0000);
    // The UUID octets in the vectors are the constant's.
    assert_eq!(b[40..56], PTA_SCMI_UUID);
}

#[test]
fn optee_open_refused() {
    let mut shm = Shm::new();
    let steps =
        vec![ret(regs(RET_EXCHANGE_CAPABILITIES)), reply(REGISTER_SHM_REPLY), reply(OPEN_SESSION_UNKNOWN_UUID_REPLY)];
    let mut c = Client::new(fake(steps), &mut shm, PA);
    assert_eq!(c.open(0, 0), Err(Error::Tee { ret: 0xffff_0008, origin: 3 }));
    assert_eq!(c.session(), None);

    // An unknown channel at GET_CHANNEL_HANDLE.
    let steps = vec![
        ret(regs(RET_EXCHANGE_CAPABILITIES)),
        reply(REGISTER_SHM_REPLY),
        reply_bytes(with_ree_kernel(OPEN_SESSION_REPLY)),
        reply(INVOKE_GET_CHANNEL_HANDLE_BAD_REPLY),
    ];
    let mut c = Client::new(fake(steps), &mut shm, PA);
    assert_eq!(c.open(0, 7), Err(Error::Tee { ret: TEE_ERROR_BAD_PARAMETERS, origin: 4 }));

    // Shared memory not page-aligned: no SMC at all.
    let mut c = Client::new(fake(vec![]), &mut shm, PA + 0x80);
    assert_eq!(c.open(0, 0), Err(Error::Unsupported));
    assert!(c.smc.calls.is_empty());
}

#[test]
fn optee_pta_capabilities_bytes() {
    let mut shm = Shm::new();
    let mut c = opened(&mut shm, vec![reply(INVOKE_CAPABILITIES_REPLY)]);
    assert_eq!(c.pta_capabilities(), Ok(PTA_SCMI_CAPS_MSG_HEADER));
    let req = hex(INVOKE_CAPABILITIES_REQ);
    assert_eq!(c.smc.args[4][..req.len()], req[..]);
}

#[test]
fn optee_process_msg_bytes() {
    let mut shm = Shm::new();
    let step = Step { arg: Some(hex(INVOKE_PD_STATE_SET_ON_REPLY)), rx: Some(hex(REP_PD_STATE_SET_ON)), ..ok() };
    let mut c = opened(&mut shm, vec![step]);
    let mut rx = [0u8; scmi::MAX_MSG];
    assert_eq!(c.process(&hex(REQ_PD_STATE_SET_ON), &mut rx), Ok(8));
    assert_eq!(rx[..8], hex(REP_PD_STATE_SET_ON)[..]);
    let f = &c.smc;
    assert_eq!(f.calls[4], regs(CALL_WITH_RPC_ARG));
    let req = hex(INVOKE_PD_STATE_SET_ON_REQ);
    assert_eq!(f.args[4][..req.len()], req[..]);
    let msg = hex(REQ_PD_STATE_SET_ON);
    assert_eq!(f.datas[4][..msg.len()], msg[..]);
    // The reply buffer is cleared before the call.
    assert!(f.datas[4][DATA_RX..DATA_RX + DATA_MAX].iter().all(|&b| b == 0));
}

#[test]
fn optee_process_errors() {
    let mut shm = Shm::new();
    let mut c = opened(&mut shm, vec![reply(INVOKE_UNKNOWN_CHANNEL_REPLY)]);
    let mut rx = [0u8; scmi::MAX_MSG];
    assert_eq!(
        c.process(&hex(REQ_PERF_LEVEL_GET), &mut rx),
        Err(Error::Tee { ret: TEE_ERROR_BAD_PARAMETERS, origin: 4 })
    );
    // Too long a request never reaches OP-TEE.
    assert_eq!(c.process(&[0; DATA_MAX + 1], &mut rx), Err(Error::TooLong));
    assert_eq!(c.smc.calls.len(), 5);

    // A reply size beyond the buffer.
    let mut bad = hex(INVOKE_PERF_LEVEL_GET_REPLY);
    wr64(&mut bad, 96 + 16, 0x81);
    let mut c = opened(&mut shm, vec![reply_bytes(bad)]);
    assert_eq!(c.process(&hex(REQ_PERF_LEVEL_GET), &mut rx), Err(Error::Malformed));

    // A refused SMC (bad address) is a0.
    let mut c = opened(&mut shm, vec![ret([RETURN_EBADADDR as u64, 0, 0, 0, 0, 0, 0, 0])]);
    assert_eq!(c.process(&hex(REQ_PERF_LEVEL_GET), &mut rx), Err(Error::Smc(RETURN_EBADADDR)));

    // No session.
    let mut c = Client::new(fake(vec![]), &mut shm, PA);
    assert_eq!(c.process(&hex(REQ_PERF_LEVEL_GET), &mut rx), Err(Error::NotOpen));
    assert_eq!(c.pta_capabilities(), Err(Error::NotOpen));
}

#[test]
fn optee_close_bytes() {
    let mut shm = Shm::new();
    let mut c = opened(&mut shm, vec![reply(CLOSE_SESSION_REPLY), reply(UNREGISTER_SHM_REPLY)]);
    assert_eq!(c.close(), Ok(()));
    assert_eq!(c.session(), None);
    let f = &c.smc;
    let req = hex(CLOSE_SESSION_REQ);
    assert_eq!(f.args[4][..req.len()], req[..]);
    assert_eq!(rd32(&f.args[4], 28), 0);
    let req = hex(UNREGISTER_SHM_REQ);
    assert_eq!(f.args[5][..req.len()], req[..]);
    // A second close does nothing.
    assert_eq!(c.close(), Ok(()));
    assert_eq!(c.smc.calls.len(), 6);
}

#[test]
fn optee_data_page_holds_a_whole_mailbox() {
    // ST's SCP-firmware writes a reply of up to the channel's mailbox, 128
    // bytes (config_all.c:690), whatever the size of our output memref
    // (mod_msg_smt.c:166-178, :192-198, :302): the reply buffer is that long,
    // inside the registered page, after the request.
    assert_eq!(DATA_MAX, 128);
    const _: () = assert!(DATA_TX + DATA_MAX <= DATA_RX);
    const _: () = assert!(DATA_RX + DATA_MAX <= PAGE);
    assert_eq!(DATA_MAX, scmi::MAX_MSG);
    // Cookie 0 is OP-TEE's transient mappings' (thread_optee_smc.c:244,
    // msg_param.c:78) in the same list a cookie lookup walks.
    assert_ne!(SHM_COOKIE, 0);
}

#[test]
fn optee_process_refuses_a_request_without_header() {
    // mod_msg_smt.c:149-150 takes the payload size as the request's less 4
    // unchecked: nothing shorter than the header reaches OP-TEE.
    let mut shm = Shm::new();
    let mut c = opened(&mut shm, vec![]);
    let mut rx = [0u8; scmi::MAX_MSG];
    assert_eq!(c.process(&[], &mut rx), Err(Error::TooShort));
    assert_eq!(c.process(&[0; DATA_MIN - 1], &mut rx), Err(Error::TooShort));
    assert_eq!(Error::TooShort.code(), 0x105);
    assert_eq!(c.smc.calls.len(), 4);
    assert!(!c.smc.log.iter().any(|l| matches!(l, Logged::Req(_))));
}

#[test]
fn optee_reply_the_server_never_wrote() {
    // A channel that is busy or not ready drops the message and returns
    // success (mod_msg_smt.c:286-296, arch_main.c:88-106); the pseudo-TA then
    // reports the output memref at the size we gave (scmi.c:170-174): 0x80
    // bytes of the cleared buffer come back, header 0, which no request has.
    let mut shm = Shm::new();
    let mut frame = hex(INVOKE_PERF_LEVEL_GET_REPLY);
    wr64(&mut frame, 96 + 16, DATA_MAX as u64);
    let mut c = opened(&mut shm, vec![reply_bytes(frame)]);
    let mut rx = [0xa5u8; scmi::MAX_MSG];
    assert_eq!(c.process(&hex(REQ_PERF_LEVEL_GET), &mut rx), Ok(DATA_MAX));
    assert!(rx.iter().all(|&b| b == 0));
}

#[test]
fn optee_rpc_foreign_intr_resumes() {
    let mut shm = Shm::new();
    let intr = [0xffff_0004, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77];
    let done = Step { arg: Some(hex(INVOKE_PERF_LEVEL_GET_REPLY)), rx: Some(hex(REP_PERF_LEVEL_GET)), ..ok() };
    let mut c = opened(&mut shm, vec![ret(intr), ret(intr), done]);
    let mut rx = [0u8; scmi::MAX_MSG];
    assert_eq!(c.process(&hex(REQ_PERF_LEVEL_GET), &mut rx), Ok(12));
    let f = &c.smc;
    assert_eq!(f.calls[4], regs(CALL_WITH_RPC_ARG));
    // RETURN_FROM_RPC with a1..a7 as OP-TEE left them (optee_smc.h:746-759).
    let resume = [SMC_RETURN_FROM_RPC as u64, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77];
    assert_eq!(f.calls[5], resume);
    assert_eq!(f.calls[6], resume);
    assert_eq!(f.calls.len(), 7);
}

#[test]
fn optee_rpc_alloc_free_ocall_unknown() {
    let mut shm = Shm::new();
    let alloc = [0xffff_0000, 0x120, 0, 0x33, 0x44, 0x55, 0x66, 0x77];
    let free = [0xffff_0002, 0, 0x9, 0x33, 0x44, 0x55, 0x66, 0x77];
    let ocall = [0xffff_0800, 1, 2, 0x33, 0x44, 0x55, 0x66, 0x77];
    let unknown = [0xffff_0042, 1, 2, 3, 4, 5, 6, 7];
    let mut c = opened(&mut shm, vec![ret(alloc), ret(free), ret(ocall), ret(unknown), reply(CLOSE_SESSION_REPLY)]);
    c.close_session(1).unwrap();
    let f = &c.smc;
    // ALLOC refused: address a1:a2 and cookie a4:a5 zero, a3, a6, a7 kept.
    assert_eq!(f.calls[5], [SMC_RETURN_FROM_RPC as u64, 0, 0, 0x33, 0, 0, 0x66, 0x77]);
    // FREE: nothing to free, resumed as is.
    assert_eq!(f.calls[6], [SMC_RETURN_FROM_RPC as u64, 0, 0x9, 0x33, 0x44, 0x55, 0x66, 0x77]);
    // OCALL2: a1 0 (error), a2 NOT_SUPPORTED.
    assert_eq!(f.calls[7], [SMC_RETURN_FROM_RPC as u64, 0, 0xffff_000a, 0x33, 0x44, 0x55, 0x66, 0x77]);
    assert_eq!(f.calls[8], [SMC_RETURN_FROM_RPC as u64, 1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn optee_rpc_cmd_not_implemented() {
    let mut shm = Shm::new();
    // OP-TEE writes its request into the RPC argument after our 3-param
    // frame (thread_optee_smc.c:516-588): cmd 4, one param, ret GENERIC.
    let mut rpc = vec![0u8; arg_size(1)];
    wr32(&mut rpc, 0, 4);
    wr32(&mut rpc, 20, TEE_ERROR_GENERIC);
    wr32(&mut rpc, 28, 1);
    let cmd = Step { ret: [0xffff_0005, 0, 0, 0x33, 0, 0, 0, 0], rpc_frame: Some((arg_size(3), rpc)), ..ok() };
    let done = Step { arg: Some(hex(INVOKE_PERF_LEVEL_GET_REPLY)), rx: Some(hex(REP_PERF_LEVEL_GET)), ..ok() };
    let mut c = opened(&mut shm, vec![cmd, done]);
    let mut rx = [0u8; scmi::MAX_MSG];
    assert_eq!(c.process(&hex(REQ_PERF_LEVEL_GET), &mut rx), Ok(12));
    let f = &c.smc;
    assert_eq!(f.calls[5], [SMC_RETURN_FROM_RPC as u64, 0, 0, 0x33, 0, 0, 0, 0]);
    let o = arg_size(3);
    assert_eq!(rd32(&f.args[5], o), 4);
    assert_eq!(rd32(&f.args[5], o + 20), TEE_ERROR_NOT_IMPLEMENTED);
    // Our frame is untouched by the answer.
    let req = hex(INVOKE_PERF_LEVEL_GET_REQ);
    assert_eq!(f.args[5][..req.len()], req[..]);
}

#[test]
fn optee_rpc_limit() {
    let mut shm = Shm::new();
    let intr = [0xffff_0004, 0, 0, 0, 0, 0, 0, 0];
    let mut steps = open_steps();
    steps.extend((0..=MAX_RPC).map(|_| ret(intr)));
    let mut c = Client::new(fake(steps), &mut shm, PA);
    c.open(0, 0).unwrap();
    let mut rx = [0u8; scmi::MAX_MSG];
    assert_eq!(c.process(&hex(REQ_PERF_LEVEL_GET), &mut rx), Err(Error::RpcLimit));
    assert_eq!(c.smc.calls.len(), 4 + 1 + MAX_RPC as usize);
}

#[test]
fn optee_unknown_function_is_not_rpc() {
    let mut shm = Shm::new();
    let mut c = opened(&mut shm, vec![ret([0xffff_ffff, 0, 0, 0, 0, 0, 0, 0])]);
    let mut rx = [0u8; scmi::MAX_MSG];
    assert_eq!(c.process(&hex(REQ_PERF_LEVEL_GET), &mut rx), Err(Error::Smc(RETURN_UNKNOWN_FUNCTION)));
    assert_eq!(Error::Smc(RETURN_UNKNOWN_FUNCTION).code(), -1);
}

#[test]
fn optee_plain_call_without_rpc_arg() {
    let mut shm = Shm::new();
    let mut r = regs(RET_EXCHANGE_CAPABILITIES);
    r[1] = 0x34;
    let steps = vec![ret(r), reply(REGISTER_SHM_REPLY)];
    let mut c = Client::new(fake(steps), &mut shm, PA);
    c.exchange_capabilities(0).unwrap();
    c.register_shm().unwrap();
    assert_eq!(c.smc.calls[1], regs(CALL_WITH_ARG));
    assert_eq!(regs(RET_CALL_OK), [0; 8]);
}

#[test]
fn optee_log_every_message() {
    let mut shm = Shm::new();
    let step = Step { arg: Some(hex(INVOKE_PD_STATE_SET_ON_REPLY)), rx: Some(hex(REP_PD_STATE_SET_ON)), ..ok() };
    let mut c = opened(&mut shm, vec![step]);
    let n = c.smc.log.len();
    // open: 1 fast SMC, then 3 x (frame out, SMC, frame back).
    assert_eq!(n, 1 + 3 * 3);
    assert_eq!(c.smc.log[0], Logged::Smc(regs(CALL_EXCHANGE_CAPABILITIES), regs(RET_EXCHANGE_CAPABILITIES)));
    assert_eq!(c.smc.log[1], Logged::Msg(false, hex(REGISTER_SHM_REQ)));
    let mut rx = [0u8; scmi::MAX_MSG];
    c.process(&hex(REQ_PD_STATE_SET_ON), &mut rx).unwrap();
    assert_eq!(
        c.smc.log[n..],
        [
            Logged::Req(hex(REQ_PD_STATE_SET_ON)),
            Logged::Msg(false, hex(INVOKE_PD_STATE_SET_ON_REQ)),
            Logged::Smc(regs(CALL_WITH_RPC_ARG), [0; 8]),
            Logged::Msg(true, hex(INVOKE_PD_STATE_SET_ON_REPLY)),
            Logged::Rep(hex(REP_PD_STATE_SET_ON)),
        ]
    );
}

#[test]
fn scmi_over_optee_bytes() {
    // The SCMI codec on the OP-TEE transport: its second message has token 1,
    // as the vectors' messages do.
    let mut shm = Shm::new();
    // rep_pd_protocol_version with the token (bits 27:18, byte 2) 0; a
    // 12-byte reply like perf_level_get's.
    let mut version_rx = hex(REP_PD_PROTOCOL_VERSION);
    version_rx[2] = 0;
    let version = Step { arg: Some(hex(INVOKE_PERF_LEVEL_GET_REPLY)), rx: Some(version_rx), ..ok() };
    let level = Step { arg: Some(hex(INVOKE_PERF_LEVEL_GET_REPLY)), rx: Some(hex(REP_PERF_LEVEL_GET)), ..ok() };
    let c = opened(&mut shm, vec![version, level]);
    let mut s = scmi::Client::new(c);
    assert_eq!(s.version(scmi::PROTO_POWER), Ok(0x2_0000));
    let mut msg = hex(REQ_PD_PROTOCOL_VERSION);
    msg[2] = 0;
    assert_eq!(s.transport.smc.datas[4][..4], msg[..]);
    assert_eq!(s.perf_level_get(1), Ok(800_000));
    let f = &s.transport.smc;
    let msg = hex(REQ_PERF_LEVEL_GET);
    assert_eq!(f.datas[5][..msg.len()], msg[..]);
    let req = hex(INVOKE_PERF_LEVEL_GET_REQ);
    assert_eq!(f.args[5][..req.len()], req[..]);
}

#[test]
fn scmi_status_and_transport_errors_over_optee() {
    let mut shm = Shm::new();
    // DENIED (-3) from the server: the codec's status error.
    let mut rep = hex(REP_PD_STATE_SET_ON);
    rep[2] = 0;
    rep[4..8].copy_from_slice(&(-3i32).to_le_bytes());
    let set = Step { arg: Some(hex(INVOKE_PD_STATE_SET_ON_REPLY)), rx: Some(rep), ..ok() };
    let c = opened(&mut shm, vec![set, reply(INVOKE_UNKNOWN_CHANNEL_REPLY)]);
    let mut s = scmi::Client::new(c);
    assert_eq!(s.power_set(0, scmi::POWER_ON), Err(scmi::Error::Status(scmi::STATUS_DENIED)));
    // A refused message: the TEE_Result as the transport's code.
    assert_eq!(s.power_get(0), Err(scmi::Error::Transport(TEE_ERROR_BAD_PARAMETERS as i32)));
    assert_eq!(s.transport.last_error, Some(Error::Tee { ret: TEE_ERROR_BAD_PARAMETERS, origin: 4 }));
}
