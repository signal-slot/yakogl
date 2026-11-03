// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2023 Collabora ltd.
//! Z4's host side (driver/docs/nanopc-t6-mali.md, "Submit (Z4)" and the
//! milestone list): command stream group 0 and its stream 0 programmed and
//! started from our kernel side, one job submitted on the stream's ring
//! buffer, its completion seen on a 16-byte sync object, a 5 s job timeout,
//! and the group's termination. The L1 frame of src/mali/frame.rs is what
//! the board probe will submit over it.
//!
//! Derived from Linux v6.18's drivers/gpu/drm/panthor (Copyright 2023
//! Collabora ltd.), used under the MIT option of its `GPL-2.0 or MIT`
//! licence (the design page's decision 1): panthor_sched.c, panthor_fw.c,
//! panthor_fw.h, and one line each of panthor_mmu.c and panthor_device.h.
//! Lines below are of v6.18 (`VERSION = 6`, `PATCHLEVEL = 18`,
//! `SUBLEVEL = 0`). Where this differs from panthor it says so.
//!
//! This file holds the design, the refusals, the constants and the
//! bring-up ([`start`], below). The submit is the next item of the plan;
//! the instruction words panthor wraps a job in are [`job`]'s.
//!
//! # The interface this uses
//!
//! [`super::csf`] (Z3) brings the firmware up and finds group 0 and stream 0
//! ([`csf::Booted`]'s `csg0` and `cs0`). Every request below goes through
//! its helpers: [`csf::update_reqs`] for a field (`panthor_fw_update_reqs`,
//! panthor_fw.h:450-458), [`csf::toggle_reqs`] for an event bit
//! (`panthor_fw_toggle_reqs`, panthor_fw.h:424-433), [`csf::acked`] and
//! [`csf::wait_acks`] for the acknowledge (`panthor_fw_wait_acks`,
//! panthor_fw.c:1203-1231), [`csf::ring_csg_doorbells`] for a group's
//! doorbell (`panthor_fw_ring_csg_doorbells`, panthor_fw.c:1305-1311) and
//! [`csf::ring_doorbell`] for the stream's.
//!
//! Two address spaces are involved, as in panthor. The stream's user
//! input and output pages and the suspend buffers are in the MCU's (AS0,
//! [`csf::McuMemory`]): `panthor_fw_alloc_queue_iface_mem` and
//! `panthor_fw_alloc_suspend_buf_mem` map them in the firmware's VM
//! (panthor_fw.c:448-453, 482-486). The ring buffer and the sync object are
//! in the group's VM (panthor_sched.c:3352-3358, 3522-3529), which panthor
//! binds to the first address space other than 0 (panthor_mmu.c:759:
//! `ffz(alloc_mask | BIT(0))`, so [`GROUP_AS`] 1 on a fresh device) and
//! writes to the group's `config` (panthor_sched.c:1330). Here the
//! MCU's side is [`csf::McuMemory`] and the group's is [`GroupVm`]; both
//! answer the CPU address of a range they map, and both must map it
//! uncached, as panthor does (panthor_fw.c:451, panthor_sched.c:3352-3358,
//! 3522-3529).
//!
//! # Bring-up: panthor's sequence for one group with one stream
//!
//! panthor starts a group in `tick_ctx_apply` (panthor_sched.c:2182-2301):
//! for a group with no slot it binds one (:2283), programs it (:2284) and
//! queues the state request (:2285-2291); `csgs_upd_ctx_apply_locked`
//! (:1833-1891) then writes, rings and waits. For our group in slot 0, step
//! by step:
//!
//! 1. Bind (`group_bind_locked`, panthor_sched.c:969-1003): the group's VM
//!    made active in its address space (:982), and every stream of the
//!    group given doorbell `csg_id + 1` (:997-998): [`CS_DOORBELL_ID`] 1.
//! 2. Program the stream (`cs_slot_prog_locked`, panthor_sched.c:1053-1084;
//!    called for each queue first, :1316-1321), on CS 0's input block:
//!    - the user input page's `extract` set to the output page's (:1061),
//!      insert not below extract (:1062);
//!    - `ringbuf_base` the ring's GPU address (:1064), `ringbuf_size` its
//!      size (:1065), `ringbuf_input` and `ringbuf_output` the MCU addresses
//!      of the user input and output pages (:1066-1067);
//!    - `config` = `CS_CONFIG_PRIORITY(queue priority) |
//!      CS_CONFIG_DOORBELL(doorbell_id)` (:1068-1069; panthor_fw.h:61-62;
//!      [`csf::cs_config`]);
//!    - `ack_irq_mask` = all ones (:1070);
//!    - `req` updated to `CS_IDLE_SYNC_WAIT | CS_IDLE_EMPTY | CS_STATE_START
//!      | CS_EXTRACT_EVENT` under the mask `CS_IDLE_SYNC_WAIT | CS_IDLE_EMPTY
//!      | CS_STATE_MASK | CS_EXTRACT_EVENT` (:1071-1079; [`CS_START_REQ`],
//!      [`CS_START_MASK`]). A field update, not a toggle, and no doorbell
//!      yet.
//! 3. Program the group (`csg_slot_prog_locked`, panthor_sched.c:1293-1347),
//!    on CSG 0's input block:
//!    - a priority above `MAX_CSG_PRIO` refused (:1303-1304; [`MAX_CSG_PRIO`]
//!      0xf, :92). The first group started in a tick gets `MAX_CSG_PRIO`
//!      (`new_csg_prio` starts there, :2264, and is taken at :2284):
//!      [`CSG_PRIORITY`];
//!    - `allow_compute`, `allow_fragment`, `allow_other` the group's
//!      compute, fragment and tiler core masks (:1323-1325);
//!    - `endpoint_req` = `CSG_EP_REQ_COMPUTE(max compute cores) |
//!      CSG_EP_REQ_FRAGMENT(max fragment cores) | CSG_EP_REQ_TILER(max tiler
//!      cores) | CSG_EP_REQ_PRIORITY(priority)` (:1326-1329;
//!      panthor_fw.h:165-171; [`csf::csg_ep_req`]);
//!    - `config` = the group VM's address space (:1330);
//!    - `suspend_buf` the suspend buffer's MCU address, or 0 when the group
//!      has none (:1332-1335); `protm_suspend_buf` likewise (:1337-1342).
//!      panthor allocates both for every group, of the sizes the CSG control
//!      block gives (`suspend_size`, `protm_suspend_size`,
//!      panthor_sched.c:3506-3520), in the MCU's VM, not uncached
//!      (panthor_fw.c:480-487);
//!    - `ack_irq_mask` = all ones (:1344);
//!    - `doorbell_req` toggled for the mask of the group's streams (:1345):
//!      bit 0, so the firmware looks at CS 0 when the group starts.
//! 4. Queue the requests (panthor_sched.c:2285-2291): the state field to
//!    `CSG_STATE_START` under `CSG_STATE_MASK` (a resumed group would get
//!    `CSG_STATE_RESUME`), and `CSG_ENDPOINT_CONFIG` toggled (the value
//!    `ack ^ CSG_ENDPOINT_CONFIG`): [`CSG_START_MASK`].
//! 5. Apply (`csgs_upd_ctx_apply_locked`, panthor_sched.c:1833-1891): `req`
//!    updated with the queued value and mask (:1850-1852); then the group's
//!    doorbell: `doorbell_req` of the GLB input toggled for bit 0 and the
//!    global doorbell rung (:1855; panthor_fw.c:1305-1311). Not doorbell
//!    `csg_id + 1`: that one is the stream's, rung by a submit.
//! 6. Wait (panthor_sched.c:1867: `panthor_fw_csg_wait_acks(..., 100)`,
//!    panthor_fw.c:1270-1294) up to 100 ms for the mask to be acknowledged;
//!    the state field counts as acknowledged only when all of its bits are
//!    (panthor_fw.c:1290-1291). On the acknowledge the endpoint priority is
//!    read back (panthor_sched.c:1869-1870, 1131-1141) and the state taken
//!    from `ack` (:1872-1873, 1220-1291): START or RESUME is active, a state
//!    that is none of the four is a firmware corruption and schedules a reset
//!    (:1250-1260, 1269-1270). A request still pending after the wait is a
//!    timeout (:1880-1888), and panthor resets the GPU (:2296-2300).
//!
//! # Submit (`queue_run_job`, panthor_sched.c:3133-3231)
//!
//! 1. The job's sequence number: the queue's counter plus one (:3165-3169).
//! 2. Its start: the user input page's `insert` (:3175).
//! 3. The instructions (`get_job_cs_params`, :3005-3030;
//!    `prepare_job_instrs`, :3039-3120), padded with zeroes to a multiple of
//!    8 (a 64-byte line, `NUM_INSTRS_PER_CACHE_LINE`, :94, :3116-3119),
//!    copied into the ring at `start & (size - 1)`, wrapping at its end
//!    (`copy_instrs_to_ringbuf`, :2969-2994). Without profiling the words
//!    are, with `X` = `cs_reg_count - unpreserved_cs_reg_count` (:3013-3017;
//!    96 - 4 = 92 on the G610, the design page's interface paragraph): `MOV32
//!    r(X+2), latest_flush` (:3044-3045); `FLUSH_CACHE2.clean_inv_all`
//!    (:3046-3048); `MOV48 rX, cs.start` (:3061-3062); `MOV32 r(X+2),
//!    cs.size` (:3063-3064); `WAIT(0)` (:3065-3066); `CALL rX, r(X+2)`
//!    (:3067-3069); `MOV48 rX, sync_addr` (:3082-3083); `MOV48 r(X+2), 1`
//!    (:3084-3085); `WAIT(all)`, all = the scoreboard slots (:3086-3087,
//!    :3023); `SYNC_ADD64.system_scope.propagate_err.nowait rX, r(X+2)`
//!    (:3088-3090); `ERROR_BARRIER` (:3091-3092). Eleven words, sixteen with
//!    the padding: 128 bytes a job. [`job`] holds them.
//! 4. Its end: start + the padded size (:3181).
//! 5. A write barrier, so the ring is written before `insert` (:3188-3191).
//! 6. The input page's `extract` set to the output page's, then `insert` to
//!    the end (:3193-3194).
//! 7. The stream's doorbell, `CSF_DOORBELL(doorbell_id)` (:3210): doorbell 1.
//!
//! The ring's bounds are panthor's queue creation's: 4 KiB to 64 KiB, a
//! power of two (panthor_sched.c:3335-3337, [`RING_MIN_SIZE`],
//! [`RING_MAX_SIZE`], [`check_ring_size`]), mapped no-exec and uncached in
//! the group's VM (:3352-3358). The user input and output pages are one
//! 8 KiB object in the MCU's VM, uncached and zeroed, input at 0 and output
//! at 4 KiB (panthor_fw.c:448-467; [`QUEUE_IFACE_SIZE`],
//! [`QUEUE_OUTPUT_OFFSET`]; their layouts [`RingbufInput`] and
//! [`RingbufOutput`], panthor_fw.h:15-23). Mesa's Gallium driver asks for a
//! 64 KiB ring at queue priority 1 (mesa:pan_csf.c:1823-1826); the design
//! page's record of a panthor session shows a 4096-byte ring. panthor never
//! lets more be in flight than the ring holds: the DRM scheduler's credit
//! limit is the ring's size in instructions (panthor_sched.c:3322), each job
//! charged its padded count (`calc_job_credits`, :3122-3131). We refuse a
//! job that would overwrite what the stream has not consumed instead
//! ([`SchedRefusal::RingOverflow`]).
//!
//! # Completion
//!
//! The sync object is panthor's `struct panthor_syncobj_64b`
//! (panthor_sched.c:338-351, [`SyncObj64`]): a 64-bit `seqno` the job's
//! `SYNC_ADD64` increments by 1, a 32-bit `status`, non-zero on failure, and
//! padding: 16 bytes, one per queue, zeroed at creation and mapped uncached
//! in the group's VM (:3522-3540). The job is done when `seqno` reaches its
//! sequence number (`group_sync_upd_work`, :2912-2951, the test at :2933).
//!
//! What wakes panthor: the job interrupt, bit n for group n
//! ([`csf::job_int_csg_if`]); the handler clears it and passes it on
//! (panthor_fw.c:1011-1025, panthor_sched.c:1782-1789, 1755-1777). Then
//! `sched_process_csg_irq_locked` (panthor_sched.c:1665-1723): it reads the
//! group's `req`, `ack`, `cs_irq_req` and `cs_irq_ack` (:1677-1680), copies
//! `cs_irq_req` to `cs_irq_ack` first (:1687-1693), acknowledges the events
//! `CSG_SYNC_UPDATE | CSG_IDLE | CSG_PROGRESS_TIMER_EVENT` by updating `req`
//! to `ack` under them (:1695-1698), handles an idle event (:1700-1701) and
//! the progress timer event (:1703-1704, 1643-1663: the group is marked
//! timed out), then each stream whose `cs_irq` differs
//! (`cs_slot_process_irq_locked`, :1573-1601: `CS_FATAL`, `CS_FAULT`,
//! `CS_TILER_OOM` from `(req ^ ack) & CS_EVT_MASK`, :1584-1593; `CS_FATAL`
//! and `CS_FAULT` acknowledged at :1598; a `CS_FATAL` of type
//! `CS_UNRECOVERABLE` schedules a reset, :1373-1378), then
//! `CSG_SYNC_UPDATE` (`csg_slot_sync_update_locked`, :1629-1641, which runs
//! the sync object test above), and finally the group's doorbell
//! (:1719-1722).
//!
//! # Timeout, stop and terminate
//!
//! The job timeout is 5 s: `JOB_TIMEOUT_MS` (panthor_sched.c:90) is the DRM
//! scheduler's timeout for every queue (:3323). On it
//! `queue_timedout_job` (:3233-3267) marks the group timed out and asks for
//! a tick, which terminates it: the state field to `CSG_STATE_TERMINATE`
//! under `CSG_STATE_MASK` (:2197-2207), applied as above, with the same
//! 100 ms wait (:2237, 1867). The termination's post-processing writes the
//! sync object itself, `status` all ones and `seqno` the last sequence
//! number, to release waiters (`group_term_post_processing`, :2071-2118,
//! the write at :2106-2110). When the group leaves the active state its
//! streams are set to `CS_STATE_STOP` under `CS_STATE_MASK`
//! (`csg_slot_sync_state_locked`, :1275-1288; `cs_slot_reset_locked`,
//! :1097-1120), with no wait and no doorbell: "the CS slot will only be
//! re-used on the next CSG start operation" (:1278-1282). Before the slot is
//! unbound its pending interrupts are processed (:2245-2256). A terminate
//! that is not acknowledged is left to the soft reset, which "will
//! automatically terminate all active groups" (`panthor_sched_suspend`,
//! :2742-2754); the reset is `panthor_device_reset_work`
//! (panthor_device.c:127-156).
//!
//! # Our waits and budgets
//!
//! panthor waits for interrupts; this polls, as [`super::csf`] does, with the
//! same `delay_us` closure and no interrupt:
//!
//! - the start's acknowledge (`CSG_STATE_START` and `CSG_ENDPOINT_CONFIG`):
//!   [`CSG_ACK_WAIT_US`], panthor's 100 ms;
//! - CS 0's state reading START in its `ack` after the group's: not a
//!   panthor wait (panthor trusts the group's acknowledge), ours, at the
//!   same 100 ms ([`CS_ACK_WAIT_US`]); it is a read, so it adds no write to
//!   panthor's sequence;
//! - the job: [`JOB_TIMEOUT_US`], panthor's 5 s, counted from the stream's
//!   doorbell; the poll reads the job interrupt's group bit, the group's
//!   `req ^ ack` events, the stream's `cs_irq` and events, and the sync
//!   object;
//! - the terminate's acknowledge: [`CSG_ACK_WAIT_US`], panthor's 100 ms;
//! - CS 0's state reading STOP after it: ours, [`CS_ACK_WAIT_US`] (panthor
//!   writes STOP and does not wait).
//!
//! # What we refuse instead of masking
//!
//! Each refusal is a [`SchedRefusal`] with the value that decided it, as
//! [`csf::CsfRefusal`] is. Where panthor recovers and continues, Z4 stops
//! and reports, so that a board record names every recovery:
//!
//! - a ring size outside panthor's bounds, a ring, user page, suspend
//!   buffer or sync object the memory cannot place, a priority above
//!   `MAX_CSG_PRIO`, the MCU's address space 0 as the group's, a slot that
//!   is not free before the start (panthor's -EINVAL and -ENOMEM paths, and
//!   our own checks);
//! - an acknowledge that does not come in its budget (panthor resets);
//! - a group state that is none of the four (panthor resets);
//! - `CS_FAULT` (panthor acknowledges it and lets the stream go on to its
//!   `ERROR_BARRIER`), `CS_FATAL` (panthor acknowledges it and terminates
//!   the group), `CS_TILER_OOM` (panthor grows the heap),
//!   `CS_PROTM_PENDING`, and `CSG_PROGRESS_TIMER_EVENT` (panthor terminates
//!   the group): each refused with its word, never acknowledged to go on;
//! - a sync object whose `status` is non-zero;
//! - a job that would overrun what the stream has not consumed;
//! - the 5 s timeout.
//!
//! After a refusal past the start the group may still run: the caller
//! terminates it (and resets the GPU when the terminate is refused, as
//! panthor does), and the record carries both refusals.
//!
//! # What Z4 does not do, and why one group with one stream suffices
//!
//! - No multi-group scheduling: no tick, no rotation, no priority update of
//!   running groups (`tick_work`, panthor_sched.c:2374;
//!   `tick_ctx_apply`'s eviction and priority paths, :2195-2256). With fewer
//!   groups than slots panthor itself "just let[s] the FW schedule the
//!   active groups" (:70-72), and the render service is one context per
//!   module (the design page: "One group, one stream is enough for GL").
//! - No suspend or resume (`CSG_STATE_SUSPEND`, `CSG_STATE_RESUME`,
//!   `panthor_sched_suspend`, :2688-2809): the group lives from the start to
//!   the terminate; the suspend buffers are given because the interface
//!   asks for them, not used.
//! - No tiler heap growth (`group_process_tiler_oom`, :1448-1532): the L1
//!   frame's heap is sized for it, and an OOM is refused.
//! - No protected mode (`CS_PROTM_PENDING`, the protected suspend buffer's
//!   use): GL here renders no protected content.
//! - No sync-wait evaluation of blocked streams (`sync_upd_work`,
//!   :2506-2549; `cs_slot_sync_queue_state_locked`, :1151-1200): one stream
//!   waits on nothing but its own job.
//! - No job profiling instructions (the `JOB_INSTR_CYCLES` and
//!   `JOB_INSTR_TIMESTAMP` words, :3049-3060, 3070-3081).
//! - No reset: a refusal names it, the caller runs the reset of Z1 to Z3.
//!
//! # For driver/docs/nanopc-t6-mali.md: "Submit (Z4)"
//!
//! The paragraph a later commit moves into the design page, replacing its
//! "Submit (Z4)" paragraph once plan #187's edits of that page have merged:
//!
//! **Submit (Z4)** (`gpu::mali::sched`): CSG 0 / CS 0 as panthor starts a
//! lone group (linux:`panthor_sched.c:2182-2301`): bind with doorbell CSG
//! slot + 1 (969-1003); program the stream (1053-1084: ring base and size,
//! the user input/output pages, priority and doorbell in `config`,
//! `ack_irq_mask` ~0, `req` START with `IDLE_SYNC_WAIT`, `IDLE_EMPTY`,
//! `EXTRACT_EVENT`); program the group (1293-1347: the core masks, the
//! endpoint request with priority 0xf, the AS in `config`, both suspend
//! buffers, `ack_irq_mask` ~0, `doorbell_req` for CS 0); request
//! `CSG_STATE_START` with `CSG_ENDPOINT_CONFIG`, ring the group through the
//! GLB `doorbell_req` and doorbell 0, wait 100 ms for the acknowledge
//! (1833-1891; `panthor_fw.c:1270-1311`). A submit (3133-3231) copies the
//! job's sixteen 64-bit words (eleven instructions ending in `SYNC_ADD64`
//! and `ERROR_BARRIER`, 3039-3120) into a 4-64 KiB uncached ring, advances
//! `insert` after a barrier and rings doorbell 1. Completion is the 16-byte
//! sync object's `seqno` (338-351, 2933) seen on the CSG `SYNC_UPDATE`
//! event (1665-1723); a fault, fatal, tiler OOM, protected-mode or progress
//! event is refused, not acknowledged. The job timeout is panthor's 5 s
//! (90, 3323); terminate is `CSG_STATE_TERMINATE` with its 100 ms
//! acknowledge, then CS 0 to `STOP` (2197-2207, 1275-1288); a terminate
//! that times out goes to the reset (2742-2754).

pub mod job;

use super::csf::{
    self, Cs, CsInput, CsOutput, Csg, CsgInput, CsgOutput, Iface, McuMemory, CSG_ENDPOINT_CONFIG, CSG_STATE_MASK,
    CSG_STATE_START, CSG_STATE_TERMINATE, CS_EVT_MASK, CS_EXTRACT_EVENT, CS_FATAL, CS_FAULT, CS_IDLE_EMPTY,
    CS_IDLE_SYNC_WAIT, CS_PROTM_PENDING, CS_STATE_MASK, CS_STATE_START, CS_TILER_OOM,
};
use super::Ident;
use crate::mmio::Mmio;
use core::mem::offset_of;

/// The group slot and the stream slot Z4 uses: the first of each (the
/// design page: one group, one stream).
pub const CSG_SLOT: u32 = 0;
pub const CS_SLOT: u32 = 0;
/// The stream's doorbell: its group slot + 1 (`group_bind_locked`,
/// panthor_sched.c:997-998); doorbell 0 is the global interface's
/// ([`csf::GLB_DOORBELL_ID`]).
pub const CS_DOORBELL_ID: u32 = CSG_SLOT + 1;
const _: () = assert!(CS_DOORBELL_ID != csf::GLB_DOORBELL_ID);
/// The group's address space: the first one that is not the MCU's
/// (panthor_mmu.c:759).
pub const GROUP_AS: u32 = 1;

/// The ring buffer's bounds (`group_create_queue`, panthor_sched.c:
/// 3335-3337): `SZ_4K` to `SZ_64K`, a power of two.
pub const RING_MIN_SIZE: u32 = 4 * 1024;
pub const RING_MAX_SIZE: u32 = 64 * 1024;

/// The user input and output pages: one 8 KiB object, input at 0, output
/// at 4 KiB (`panthor_fw_alloc_queue_iface_mem`, panthor_fw.c:448, 463-467).
pub const QUEUE_IFACE_SIZE: u32 = 8 * 1024;
pub const QUEUE_OUTPUT_OFFSET: u32 = 4 * 1024;

/// `JOB_TIMEOUT_MS` (panthor_sched.c:90), in microseconds.
pub const JOB_TIMEOUT_US: u32 = 5_000_000;
/// A group request's acknowledge: `csgs_upd_ctx_apply_locked` waits 100 ms
/// (panthor_sched.c:1867), for the start and the terminate alike.
pub const CSG_ACK_WAIT_US: u32 = 100_000;
/// The stream's state read back after its group's acknowledge: ours, not
/// a panthor wait, at the group's budget.
pub const CS_ACK_WAIT_US: u32 = CSG_ACK_WAIT_US;

/// `MAX_CSG_PRIO` (panthor_sched.c:92): the highest group priority, and the
/// one the first group of a tick gets (:2264, :2284).
pub const MAX_CSG_PRIO: u32 = 0xf;
pub const CSG_PRIORITY: u32 = MAX_CSG_PRIO;
/// `CSF_MAX_QUEUE_PRIO` (panthor_sched.c:392), and the queue priority
/// Mesa's Gallium driver asks for (mesa:pan_csf.c:1824).
pub const CSF_MAX_QUEUE_PRIO: u32 = 0xf;
pub const QUEUE_PRIORITY: u32 = 1;
/// `CSG_EP_REQ_PRIORITY_MASK` (panthor_fw.h:171). Not in [`csf`].
pub const CSG_EP_REQ_PRIORITY_MASK: u32 = 0xf000_0000;

/// The `ack_irq_mask` panthor writes for the group and the stream: every
/// bit (panthor_sched.c:1070, 1344).
pub const ACK_IRQ_MASK_ALL: u32 = !0;
/// The stream's start: `req` to this value under this mask
/// (`cs_slot_prog_locked`, panthor_sched.c:1071-1079).
pub const CS_START_REQ: u32 = CS_IDLE_SYNC_WAIT | CS_IDLE_EMPTY | CS_STATE_START | CS_EXTRACT_EVENT;
pub const CS_START_MASK: u32 = CS_IDLE_SYNC_WAIT | CS_IDLE_EMPTY | CS_STATE_MASK | CS_EXTRACT_EVENT;
/// The group's start requests: the state field and the endpoint
/// configuration (panthor_sched.c:2285-2291).
pub const CSG_START_MASK: u32 = CSG_STATE_MASK | CSG_ENDPOINT_CONFIG;
/// The stream events panthor acknowledges (panthor_sched.c:1598); Z4
/// refuses them instead.
pub const CS_EVT_ACK: u32 = CS_FATAL | CS_FAULT;

/// `DRM_PANTHOR_EXCEPTION_CS_UNRECOVERABLE`, `..._CS_INHERIT_FAULT`
/// (panthor_device.h:321, 326): the `CS_FATAL` type that makes panthor
/// reset (panthor_sched.c:1373-1378), and the `CS_FAULT` type that fails
/// the jobs it spans (:1414-1429).
pub const EXCEPTION_CS_UNRECOVERABLE: u32 = 0x41;
pub const EXCEPTION_CS_INHERIT_FAULT: u32 = 0x4b;

/// `struct panthor_fw_ringbuf_input_iface` (panthor_fw.h:15-18): the
/// stream's user input page. Not in [`csf`].
#[repr(C)]
pub struct RingbufInput {
    pub insert: u64,
    pub extract: u64,
}

/// `struct panthor_fw_ringbuf_output_iface` (panthor_fw.h:20-23): the
/// stream's user output page. Not in [`csf`].
#[repr(C)]
pub struct RingbufOutput {
    pub extract: u64,
    pub active: u32,
}

/// `struct panthor_syncobj_64b` (panthor_sched.c:338-351): the job's
/// completion, 16 bytes.
#[repr(C)]
pub struct SyncObj64 {
    /// Incremented by the job's `SYNC_ADD64`.
    pub seqno: u64,
    /// Non-zero on failure.
    pub status: u32,
    /// Must be zero.
    pub pad: u32,
}

pub const SYNCOBJ_SIZE: u32 = size_of::<SyncObj64>() as u32;

const _: () = assert!(size_of::<RingbufInput>() == 0x10);
const _: () = assert!(size_of::<RingbufOutput>() == 0x10);
const _: () = assert!(size_of::<SyncObj64>() == 0x10);
const _: () = assert!(QUEUE_OUTPUT_OFFSET + size_of::<RingbufOutput>() as u32 <= QUEUE_IFACE_SIZE);

/// Why a step of Z4 stopped. The value crosses to C, one number per cause,
/// with the value that decided it (as [`csf::CsfRefusal`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SchedRefusal {
    /// The ring's size is outside [`RING_MIN_SIZE`]..=[`RING_MAX_SIZE`] or
    /// not a power of two (the value: the size; panthor's -EINVAL,
    /// panthor_sched.c:3335-3337).
    RingSize = 1,
    /// The memory cannot place the whole ring at one CPU range (the value:
    /// its GPU address, low 32 bits).
    RingUnmapped = 2,
    /// [`csf::McuMemory::cpu`] returns `None` for the user input and output
    /// pages (the value: their MCU address).
    UserPagesUnmapped = 3,
    /// The CSG control block asks for a suspend buffer (`suspend_size` or
    /// `protm_suspend_size` non-zero) and none of that size is given, or
    /// AS0 does not map it (the value: the size asked for).
    SuspendBufMissing = 4,
    /// The group priority is above [`MAX_CSG_PRIO`] (the value; panthor's
    /// -EINVAL, panthor_sched.c:1303-1304).
    CsgPriority = 5,
    /// The group's address space is 0, the MCU's, or not in `AS_PRESENT`
    /// (the value: the address space).
    AddressSpace = 6,
    /// CSG 0's acknowledged state is not `CSG_STATE_TERMINATE` before the
    /// start: the slot is not free (the value: the `ack` word). Ours.
    CsgSlotBusy = 7,
    /// `CSG_STATE_START` and `CSG_ENDPOINT_CONFIG` not acknowledged within
    /// [`CSG_ACK_WAIT_US`] (the value: the bits that were; panthor: "CSG %d
    /// update request timedout", panthor_sched.c:1880-1883).
    CsgStartAckTimeout = 8,
    /// The group's acknowledged state is none of the four (the value: the
    /// `ack` word; panthor: "Invalid state on CSG", panthor_sched.c:
    /// 1250-1260).
    CsgStateUnknown = 9,
    /// CS 0's `ack` did not show `CS_STATE_START` within [`CS_ACK_WAIT_US`]
    /// after the group's acknowledge (the value: the `ack` word). Ours.
    CsStartAckTimeout = 10,
    /// `CS_FAULT` pending on CS 0 (the value: the `fault` word,
    /// [`csf::cs_exception_type`] in its low byte; panthor_sched.c:
    /// 1395-1446).
    CsFault = 11,
    /// `CS_FATAL` pending on CS 0 (the value: the `fatal` word;
    /// panthor_sched.c:1349-1393).
    CsFatal = 12,
    /// `CS_TILER_OOM` pending on CS 0 (the value: the output's
    /// `heap_vt_start`): Z4 grows no heap.
    TilerOom = 13,
    /// `CS_PROTM_PENDING` pending on CS 0 (the value: the `ack` word): Z4
    /// has no protected mode.
    ProtmPending = 14,
    /// `CSG_PROGRESS_TIMER_EVENT` from the firmware (the value: the group's
    /// `ack` word; panthor_sched.c:1643-1663 marks the group timed out).
    CsgProgressTimeout = 15,
    /// The job does not fit in what the stream has consumed: `insert` plus
    /// the job's padded size minus the output page's `extract` exceeds the
    /// ring (the value: the bytes free).
    RingOverflow = 16,
    /// The memory cannot place the 16-byte sync object (the value: its GPU
    /// address, low 32 bits).
    SyncObjUnmapped = 17,
    /// The sync object's `status` is non-zero (the value).
    SyncObjError = 18,
    /// The sync object's `seqno` did not reach the job's within
    /// [`JOB_TIMEOUT_US`] (the value: `seqno`'s low 32 bits;
    /// panthor_sched.c:90, 3233-3267).
    JobTimeout = 19,
    /// `CSG_STATE_TERMINATE` not acknowledged within [`CSG_ACK_WAIT_US`]
    /// (the value: the bits that were): the GPU needs the reset
    /// (panthor_sched.c:2742-2754).
    CsgTerminateAckTimeout = 20,
    /// CS 0's `ack` did not show `CS_STATE_STOP` within [`CS_ACK_WAIT_US`]
    /// after the stop (the value: the `ack` word). Ours.
    CsStopAckTimeout = 21,
    /// The queue priority is above [`CSF_MAX_QUEUE_PRIO`] (the value;
    /// panthor's -EINVAL, panthor_sched.c:3339-3340).
    QueuePriority = 22,
    /// A core mask outside the GPU's, or a core count above its mask's
    /// (panthor's -EINVAL, panthor_sched.c:3465-3473), or a tiler count
    /// wider than `CSG_EP_REQ_TILER`'s four bits (panthor_fw.h:167; ours,
    /// panthor masks it). The value: the check, 1 to 3 the compute,
    /// fragment and tiler masks, 4 to 6 their counts, 7 the tiler field.
    GroupCores = 23,
}

/// The ring's size as panthor's queue creation accepts it
/// (panthor_sched.c:3335-3337).
pub const fn check_ring_size(size: u32) -> Result<u32, (SchedRefusal, u32)> {
    if size < RING_MIN_SIZE || size > RING_MAX_SIZE || !size.is_power_of_two() {
        return Err((SchedRefusal::RingSize, size));
    }
    Ok(size)
}

type Refused = (SchedRefusal, u32);

/// The group's VM as the CPU sees it: the ring buffer and the sync object
/// are there (panthor_sched.c:3352-3358, 3522-3529), in [`GROUP_AS`].
pub trait GroupVm {
    /// The CPU address of the `len` bytes at GPU address `va`, or `None`
    /// when the group's page tables do not map all of them at one CPU
    /// range. Mapped uncached for the CPU, as panthor maps both
    /// (`DRM_PANTHOR_VM_BIND_OP_MAP_UNCACHED`): no cache maintenance follows
    /// a write.
    fn cpu(&self, va: u64, len: u32) -> Option<usize>;
}

/// The group, as `panthor_group_create` takes it (panthor_sched.c:
/// 3447-3520) and `csg_slot_prog_locked` writes it (:1323-1342).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Group {
    /// `allow_compute`, `allow_fragment`, `allow_other` (:542-557, 1323-1325).
    pub compute_core_mask: u64,
    pub fragment_core_mask: u64,
    pub tiler_core_mask: u64,
    /// The endpoint request's counts (:1326-1328).
    pub max_compute_cores: u8,
    pub max_fragment_cores: u8,
    pub max_tiler_cores: u8,
    /// The CSG slot priority, at most [`MAX_CSG_PRIO`] (:1303-1304, 1329).
    pub priority: u32,
    /// The group VM's address space, written to `config` (:1330).
    pub as_nr: u32,
    /// MCU addresses (AS0) of the suspend buffers, of the sizes the CSG
    /// control block gives (:3506-3520); `None` for none.
    pub suspend_buf: Option<u32>,
    pub protm_suspend_buf: Option<u32>,
}

impl Group {
    /// Every shader core for compute and fragment work, every tiler, at
    /// [`CSG_PRIORITY`] in [`GROUP_AS`]: the group GL uses. Ours (panthor
    /// takes what user space asks).
    pub const fn all_cores(id: &Ident, suspend_buf: Option<u32>, protm_suspend_buf: Option<u32>) -> Group {
        Group {
            compute_core_mask: id.shader_present,
            fragment_core_mask: id.shader_present,
            tiler_core_mask: id.tiler_present,
            max_compute_cores: id.shader_present.count_ones() as u8,
            max_fragment_cores: id.shader_present.count_ones() as u8,
            max_tiler_cores: id.tiler_present.count_ones() as u8,
            priority: CSG_PRIORITY,
            as_nr: GROUP_AS,
            suspend_buf,
            protm_suspend_buf,
        }
    }
}

/// The one stream's memory and priority (`group_create_queue`,
/// panthor_sched.c:3330-3375).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Queue {
    /// The ring buffer: GPU address in the group's VM, size
    /// [`RING_MIN_SIZE`]..=[`RING_MAX_SIZE`], a power of two (:3335-3358).
    pub ring_va: u64,
    pub ring_size: u32,
    /// MCU addresses (AS0) of the user input and output pages (panthor:
    /// one 8 KiB object, output at +4 KiB, panthor_fw.c:448-467).
    pub input_va: u32,
    pub output_va: u32,
    /// The 16-byte [`SyncObj64`], GPU address in the group's VM
    /// (panthor_sched.c:3522-3540).
    pub syncobj_va: u64,
    /// At most [`CSF_MAX_QUEUE_PRIO`] (:3339-3340); [`QUEUE_PRIORITY`] for
    /// GL.
    pub priority: u32,
}

/// What [`start`] left: where the queue's memory is for the CPU, and the
/// words read back after the acknowledges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Started {
    pub ring_cpu: usize,
    pub input_cpu: usize,
    pub output_cpu: usize,
    pub syncobj_cpu: usize,
    /// The stream's doorbell, [`CS_DOORBELL_ID`]: what a submit rings.
    pub doorbell_id: u32,
    /// CS 0's input `config` and `req`, its output `ack`.
    pub cs_config: u32,
    pub cs_req: u32,
    pub cs_ack: u32,
    /// CSG 0's input `endpoint_req`, `config`, suspend buffers,
    /// `doorbell_req` and `req`, its output `ack`.
    pub endpoint_req: u32,
    pub csg_config: u32,
    pub suspend_buf: u64,
    pub protm_suspend_buf: u64,
    pub csg_doorbell_req: u32,
    pub csg_req: u32,
    pub csg_ack: u32,
    /// The GLB input's `doorbell_req` after the group's doorbell.
    pub glb_doorbell_req: u32,
    /// The slot priority as panthor reads it back on the
    /// `CSG_ENDPOINT_CONFIG` acknowledge (`csg_slot_sync_priority_locked`,
    /// panthor_sched.c:1869-1870, 1140).
    pub priority: u32,
    /// Microseconds polled until CSG 0's acknowledge, then CS 0's.
    pub csg_ack_us: u32,
    pub cs_ack_us: u32,
}

fn write64(m: &impl Mmio, addr: usize, v: u64) {
    m.write32(addr, v as u32);
    m.write32(addr + 4, (v >> 32) as u32);
}

fn read64(m: &impl Mmio, addr: usize) -> u64 {
    u64::from(m.read32(addr)) | u64::from(m.read32(addr + 4)) << 32
}

/// Where everything is, found before the first write.
struct Placed {
    ring_cpu: usize,
    input_cpu: usize,
    output_cpu: usize,
    syncobj_cpu: usize,
    suspend_buf: u64,
    protm_suspend_buf: u64,
}

/// `panthor_group_create`'s checks of the cores (panthor_sched.c:
/// 3465-3473), and ours of the tiler field (panthor_fw.h:167).
fn check_cores(g: &Group, id: &Ident) -> Result<(), Refused> {
    let checks = [
        g.compute_core_mask & !id.shader_present != 0,
        g.fragment_core_mask & !id.shader_present != 0,
        g.tiler_core_mask & !id.tiler_present != 0,
        g.compute_core_mask.count_ones() < u32::from(g.max_compute_cores),
        g.fragment_core_mask.count_ones() < u32::from(g.max_fragment_cores),
        g.tiler_core_mask.count_ones() < u32::from(g.max_tiler_cores),
        g.max_tiler_cores > 0xf,
    ];
    match checks.iter().position(|&bad| bad) {
        Some(i) => Err((SchedRefusal::GroupCores, i as u32 + 1)),
        None => Ok(()),
    }
}

/// A suspend buffer of `size` bytes at `va`: 0 when the interface asks for
/// none (panthor_sched.c:1332-1342), refused when it asks and none is
/// given or AS0 does not map it.
fn suspend_buf_va(mem: &impl McuMemory, size: u32, va: Option<u32>) -> Result<u64, Refused> {
    if size == 0 {
        return Ok(0);
    }
    match va {
        Some(va) if mem.cpu(va, size).is_some() => Ok(u64::from(va)),
        _ => Err((SchedRefusal::SuspendBufMissing, size)),
    }
}

/// Every check before the first write, in the order panthor meets them:
/// the group's creation (panthor_sched.c:3465-3473, the suspend buffers
/// :3506-3520, the sync objects :3522-3529), the queue's (:3335-3340, the
/// ring :3352-3358, the user pages :3368-3375), the slot priority
/// (:1303-1304), the address space (`panthor_vm_active`, :982), and ours:
/// CSG 0 free.
fn place(
    m: &impl Mmio,
    mem: &impl McuMemory,
    vm: &impl GroupVm,
    id: &Ident,
    csg: &Csg,
    group: &Group,
    queue: &Queue,
) -> Result<Placed, Refused> {
    check_cores(group, id)?;
    let suspend_buf = suspend_buf_va(mem, csg.suspend_size, group.suspend_buf)?;
    let protm_suspend_buf = suspend_buf_va(mem, csg.protm_suspend_size, group.protm_suspend_buf)?;
    let syncobj_cpu =
        vm.cpu(queue.syncobj_va, SYNCOBJ_SIZE).ok_or((SchedRefusal::SyncObjUnmapped, queue.syncobj_va as u32))?;
    check_ring_size(queue.ring_size)?;
    if queue.priority > CSF_MAX_QUEUE_PRIO {
        return Err((SchedRefusal::QueuePriority, queue.priority));
    }
    let ring_cpu = vm.cpu(queue.ring_va, queue.ring_size).ok_or((SchedRefusal::RingUnmapped, queue.ring_va as u32))?;
    let page = |va: u32| mem.cpu(va, QUEUE_OUTPUT_OFFSET).ok_or((SchedRefusal::UserPagesUnmapped, va));
    let input_cpu = page(queue.input_va)?;
    let output_cpu = page(queue.output_va)?;
    if group.priority > MAX_CSG_PRIO {
        return Err((SchedRefusal::CsgPriority, group.priority));
    }
    if group.as_nr == 0 || group.as_nr >= 32 || id.as_present & (1 << group.as_nr) == 0 {
        return Err((SchedRefusal::AddressSpace, group.as_nr));
    }
    let ack = m.read32(csg.output + offset_of!(CsgOutput, ack));
    if ack & CSG_STATE_MASK != CSG_STATE_TERMINATE {
        return Err((SchedRefusal::CsgSlotBusy, ack));
    }
    Ok(Placed { ring_cpu, input_cpu, output_cpu, syncobj_cpu, suspend_buf, protm_suspend_buf })
}

/// The memory panthor zeroes when it creates the group: the sync object
/// (panthor_sched.c:3539-3540), then the user input and output pages
/// (`panthor_fw_alloc_queue_iface_mem`, panthor_fw.c:463, called at
/// panthor_sched.c:3368). Only the words of the structures are written.
fn zero_queue_mem(m: &impl Mmio, p: &Placed) {
    write64(m, p.syncobj_cpu + offset_of!(SyncObj64, seqno), 0);
    m.write32(p.syncobj_cpu + offset_of!(SyncObj64, status), 0);
    m.write32(p.syncobj_cpu + offset_of!(SyncObj64, pad), 0);
    write64(m, p.input_cpu + offset_of!(RingbufInput, insert), 0);
    write64(m, p.input_cpu + offset_of!(RingbufInput, extract), 0);
    write64(m, p.output_cpu + offset_of!(RingbufOutput, extract), 0);
    m.write32(p.output_cpu + offset_of!(RingbufOutput, active), 0);
}

/// `cs_slot_prog_locked` (panthor_sched.c:1053-1084) for CS 0.
fn prog_cs(m: &impl Mmio, cs: &Cs, queue: &Queue, p: &Placed) {
    let i = cs.input;
    // :1061: the input page's extract from the output page's
    let extract = read64(m, p.output_cpu + offset_of!(RingbufOutput, extract));
    write64(m, p.input_cpu + offset_of!(RingbufInput, extract), extract);
    // :1064-1067
    write64(m, i + offset_of!(CsInput, ringbuf_base), queue.ring_va);
    m.write32(i + offset_of!(CsInput, ringbuf_size), queue.ring_size);
    write64(m, i + offset_of!(CsInput, ringbuf_input), u64::from(queue.input_va));
    write64(m, i + offset_of!(CsInput, ringbuf_output), u64::from(queue.output_va));
    // :1068-1069, the doorbell of `group_bind_locked` (:997-998)
    m.write32(i + offset_of!(CsInput, config), csf::cs_config(queue.priority, CS_DOORBELL_ID));
    // :1070
    m.write32(i + offset_of!(CsInput, ack_irq_mask), ACK_IRQ_MASK_ALL);
    // :1071-1079, a field update
    csf::update_reqs(m, i + offset_of!(CsInput, req), CS_START_REQ, CS_START_MASK);
}

/// `csg_slot_prog_locked` (panthor_sched.c:1293-1347) for CSG 0, after its
/// stream (:1316-1321).
fn prog_csg(m: &impl Mmio, csg: &Csg, group: &Group, p: &Placed) {
    let i = csg.input;
    // :1323-1325; `allow_other` is 32 bits, the tiler mask 64 (panthor
    // stores it as is)
    write64(m, i + offset_of!(CsgInput, allow_compute), group.compute_core_mask);
    write64(m, i + offset_of!(CsgInput, allow_fragment), group.fragment_core_mask);
    m.write32(i + offset_of!(CsgInput, allow_other), group.tiler_core_mask as u32);
    // :1326-1329
    let ep = csf::csg_ep_req(
        u32::from(group.max_compute_cores),
        u32::from(group.max_fragment_cores),
        u32::from(group.max_tiler_cores),
        group.priority,
    );
    m.write32(i + offset_of!(CsgInput, endpoint_req), ep);
    // :1330
    m.write32(i + offset_of!(CsgInput, config), group.as_nr);
    // :1332-1342
    write64(m, i + offset_of!(CsgInput, suspend_buf), p.suspend_buf);
    write64(m, i + offset_of!(CsgInput, protm_suspend_buf), p.protm_suspend_buf);
    // :1344
    m.write32(i + offset_of!(CsgInput, ack_irq_mask), ACK_IRQ_MASK_ALL);
    // :1345, the mask of the programmed streams (:1319)
    csf::toggle_reqs(
        m,
        i + offset_of!(CsgInput, doorbell_req),
        csg.output + offset_of!(CsgOutput, doorbell_ack),
        1 << CS_SLOT,
    );
}

/// The start requested and the group rung: `tick_ctx_apply` queues
/// `CSG_STATE_START` under `CSG_STATE_MASK` and `ack ^ CSG_ENDPOINT_CONFIG`
/// under `CSG_ENDPOINT_CONFIG` (panthor_sched.c:2285-2291, merged as
/// `csgs_upd_ctx_queue_reqs` does, :1828-1829); `csgs_upd_ctx_apply_locked`
/// writes `req` (:1850-1852) and rings the group (:1855,
/// `panthor_fw_ring_csg_doorbells`, panthor_fw.c:1305-1311).
fn request_start(m: &impl Mmio, gpu: usize, iface: &Iface, csg: &Csg) {
    let ack = m.read32(csg.output + offset_of!(CsgOutput, ack));
    let value = CSG_STATE_START | ((ack ^ CSG_ENDPOINT_CONFIG) & CSG_ENDPOINT_CONFIG);
    csf::update_reqs(m, csg.input + offset_of!(CsgInput, req), value, CSG_START_MASK);
    csf::ring_csg_doorbells(m, gpu, iface, 1 << CSG_SLOT);
}

/// CS 0's pending events, refused (panthor handles them in this order,
/// panthor_sched.c:1584-1593; `CS_PROTM_PENDING` after them is ours): the
/// `fatal` word, the `fault` word, the tiler's `heap_vt_start`, the `ack`.
/// Nothing is acknowledged.
pub fn cs_events(m: &impl Mmio, cs: &Cs) -> Result<(), Refused> {
    let req = m.read32(cs.input + offset_of!(CsInput, req));
    cs_events_of(m, cs, req, m.read32(cs.output + offset_of!(CsOutput, ack)))
}

/// [`cs_events`] on `req` and `ack` words already read.
fn cs_events_of(m: &impl Mmio, cs: &Cs, req: u32, ack: u32) -> Result<(), Refused> {
    let events = (req ^ ack) & CS_EVT_MASK;
    let out = |f: usize| m.read32(cs.output + f);
    if events & CS_FATAL != 0 {
        return Err((SchedRefusal::CsFatal, out(offset_of!(CsOutput, fatal))));
    }
    if events & CS_FAULT != 0 {
        return Err((SchedRefusal::CsFault, out(offset_of!(CsOutput, fault))));
    }
    if events & CS_TILER_OOM != 0 {
        return Err((SchedRefusal::TilerOom, out(offset_of!(CsOutput, heap_vt_start))));
    }
    if events & CS_PROTM_PENDING != 0 {
        return Err((SchedRefusal::ProtmPending, ack));
    }
    Ok(())
}

/// The start's acknowledges: CSG 0's within [`CSG_ACK_WAIT_US`]
/// (`panthor_fw_csg_wait_acks(..., 100)`, panthor_sched.c:1867;
/// panthor_fw.c:1270-1294, whose state field counts only when all its bits
/// are, :1290-1291), then, ours, CS 0's state START within
/// [`CS_ACK_WAIT_US`], each poll refusing a pending stream event first,
/// both judged on the same `req` and `ack` read.
fn wait_started(m: &impl Mmio, csg: &Csg, cs: &Cs, delay_us: &mut impl FnMut(u32)) -> Result<(u32, u32), Refused> {
    let csg_us = csf::wait_acks(
        m,
        csg.input + offset_of!(CsgInput, req),
        csg.output + offset_of!(CsgOutput, ack),
        CSG_START_MASK,
        CSG_ACK_WAIT_US,
        delay_us,
    )
    .map_err(|a| {
        let a = if a & CSG_STATE_MASK != CSG_STATE_MASK { a & !CSG_STATE_MASK } else { a };
        (SchedRefusal::CsgStartAckTimeout, a)
    })?;
    let (req, ack) = (cs.input + offset_of!(CsInput, req), cs.output + offset_of!(CsOutput, ack));
    for t in 0..=CS_ACK_WAIT_US {
        let (r, a) = (m.read32(req), m.read32(ack));
        cs_events_of(m, cs, r, a)?;
        if (r ^ a) & CS_STATE_MASK == 0 {
            return Ok((csg_us, t));
        }
        delay_us(1);
    }
    Err((SchedRefusal::CsStartAckTimeout, m.read32(ack)))
}

/// Z4's bring-up: CSG 0 with CS 0 started as panthor starts a lone group in
/// a free slot (`tick_ctx_apply`, panthor_sched.c:2263-2301):
///
/// 1. every check, before any write (the group's, the queue's, the slot
///    priority, the address space, CSG 0 free); then the memory
///    panthor zeroes at the group's creation;
/// 2. CS 0 programmed (`cs_slot_prog_locked`, :1053-1084) with doorbell
///    CSG slot + 1 (`group_bind_locked`, :997-998);
/// 3. CSG 0 programmed (`csg_slot_prog_locked`, :1293-1347);
/// 4. `CSG_STATE_START` and `CSG_ENDPOINT_CONFIG` requested, the group's
///    GLB doorbell bit toggled and doorbell 0 rung (:2285-2291, 1850-1855);
/// 5. the acknowledges waited for (:1867, then ours for CS 0).
///
/// `iface`, `csg` and `cs` are Z3's ([`csf::Booted`]); `id` gives the
/// cores and address spaces present. The stream's doorbell is not rung:
/// that is the submit's (:3210). A refusal past step 1 leaves the slot
/// programmed and possibly running: the caller terminates it or resets.
#[allow(clippy::too_many_arguments)]
pub fn start(
    m: &impl Mmio,
    gpu: usize,
    mem: &impl McuMemory,
    vm: &impl GroupVm,
    id: &Ident,
    iface: &Iface,
    csg: &Csg,
    cs: &Cs,
    group: &Group,
    queue: &Queue,
    delay_us: &mut impl FnMut(u32),
) -> Result<Started, Refused> {
    let p = place(m, mem, vm, id, csg, group, queue)?;
    zero_queue_mem(m, &p);
    prog_cs(m, cs, queue, &p);
    prog_csg(m, csg, group, &p);
    request_start(m, gpu, iface, csg);
    let (csg_ack_us, cs_ack_us) = wait_started(m, csg, cs, delay_us)?;
    let (ci, co, gi, go) = (cs.input, cs.output, csg.input, csg.output);
    let endpoint_req = m.read32(gi + offset_of!(CsgInput, endpoint_req));
    Ok(Started {
        ring_cpu: p.ring_cpu,
        input_cpu: p.input_cpu,
        output_cpu: p.output_cpu,
        syncobj_cpu: p.syncobj_cpu,
        doorbell_id: CS_DOORBELL_ID,
        cs_config: m.read32(ci + offset_of!(CsInput, config)),
        cs_req: m.read32(ci + offset_of!(CsInput, req)),
        cs_ack: m.read32(co + offset_of!(CsOutput, ack)),
        endpoint_req,
        csg_config: m.read32(gi + offset_of!(CsgInput, config)),
        suspend_buf: read64(m, gi + offset_of!(CsgInput, suspend_buf)),
        protm_suspend_buf: read64(m, gi + offset_of!(CsgInput, protm_suspend_buf)),
        csg_doorbell_req: m.read32(gi + offset_of!(CsgInput, doorbell_req)),
        csg_req: m.read32(gi + offset_of!(CsgInput, req)),
        csg_ack: m.read32(go + offset_of!(CsgOutput, ack)),
        glb_doorbell_req: m.read32(iface.glb_input + offset_of!(csf::GlbInput, doorbell_req)),
        priority: (endpoint_req & CSG_EP_REQ_PRIORITY_MASK) >> 28,
        csg_ack_us,
        cs_ack_us,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [SchedRefusal; 23] = [
        SchedRefusal::RingSize,
        SchedRefusal::RingUnmapped,
        SchedRefusal::UserPagesUnmapped,
        SchedRefusal::SuspendBufMissing,
        SchedRefusal::CsgPriority,
        SchedRefusal::AddressSpace,
        SchedRefusal::CsgSlotBusy,
        SchedRefusal::CsgStartAckTimeout,
        SchedRefusal::CsgStateUnknown,
        SchedRefusal::CsStartAckTimeout,
        SchedRefusal::CsFault,
        SchedRefusal::CsFatal,
        SchedRefusal::TilerOom,
        SchedRefusal::ProtmPending,
        SchedRefusal::CsgProgressTimeout,
        SchedRefusal::RingOverflow,
        SchedRefusal::SyncObjUnmapped,
        SchedRefusal::SyncObjError,
        SchedRefusal::JobTimeout,
        SchedRefusal::CsgTerminateAckTimeout,
        SchedRefusal::CsStopAckTimeout,
        SchedRefusal::QueuePriority,
        SchedRefusal::GroupCores,
    ];

    #[test]
    fn every_refusal_has_its_own_number_from_1() {
        let v = ALL.map(|r| r as u32);
        for (i, n) in v.iter().enumerate() {
            assert_eq!(*n, i as u32 + 1, "{:?}", ALL[i]);
        }
    }

    #[test]
    fn the_constants_are_panthors() {
        // panthor_sched.c:90, 92, 392, 1867, 997-998, 3335-3337
        assert_eq!(JOB_TIMEOUT_US, 5000 * 1000);
        assert_eq!((MAX_CSG_PRIO, CSG_PRIORITY, CSF_MAX_QUEUE_PRIO), (0xf, 0xf, 0xf));
        assert_eq!(CSG_ACK_WAIT_US, 100 * 1000);
        assert_eq!((CSG_SLOT, CS_SLOT, CS_DOORBELL_ID), (0, 0, 1));
        assert_eq!(csf::doorbell(CS_DOORBELL_ID), 0x90000);
        assert_eq!((RING_MIN_SIZE, RING_MAX_SIZE), (0x1000, 0x10000));
        // panthor_fw.c:448, 467; panthor_sched.c:338-351
        assert_eq!((QUEUE_IFACE_SIZE, QUEUE_OUTPUT_OFFSET, SYNCOBJ_SIZE), (0x2000, 0x1000, 16));
        assert_eq!(core::mem::offset_of!(SyncObj64, status), 8);
        assert_eq!(core::mem::offset_of!(RingbufInput, extract), 8);
        assert_eq!(core::mem::offset_of!(RingbufOutput, active), 8);
        // panthor_fw.h:37-48, 138-143, 170-171
        assert_eq!(CS_START_REQ, 0x0511);
        assert_eq!(CS_START_MASK, 0x0517);
        assert_eq!(CSG_START_MASK, 0x17);
        assert_eq!(CS_EVT_ACK, 0xc000_0000);
        assert_eq!(csf::csg_ep_req(0, 0, 0, CSG_PRIORITY), CSG_EP_REQ_PRIORITY_MASK);
        // the endpoint word for the G610's shader_present 0x50005 (4 cores)
        // and its one tiler, at priority 0xf
        assert_eq!(csf::csg_ep_req(4, 4, 1, CSG_PRIORITY), 0xf001_0404);
        assert_eq!(csf::cs_config(QUEUE_PRIORITY, CS_DOORBELL_ID), 0x0101);
    }

    #[test]
    fn the_ring_size_is_panthors_bound() {
        for s in [0x1000, 0x2000, 0x4000, 0x8000, 0x10000] {
            assert_eq!(check_ring_size(s), Ok(s));
        }
        for s in [0, 0x800, 0xfff, 0x1001, 0x3000, 0x20000, u32::MAX] {
            assert_eq!(check_ring_size(s), Err((SchedRefusal::RingSize, s)));
        }
    }

    // ---- the MCU's group and stream interface, modelled ----

    use crate::gpu::mali::fw::MCU_SHARED_REGION_START;
    use csf::{GlbInput, GlbOutput, CSG_REQ_MASK, CS_REQ_MASK};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    const GPU: usize = 0xfb00_0000;
    /// AS0 at one CPU window: MCU address `va` is CPU address `MEM + va`.
    const MEM: usize = 0x1_0000_0000;
    /// The group's VM likewise: GPU address `va` is CPU address `VM + va`.
    const VM: usize = 0x100_0000_0000;

    /// The interface pages, where the model's firmware puts them in the
    /// shared section (offsets as csf.rs's model has them).
    const GLB_IN: u32 = MCU_SHARED_REGION_START + 0x2000;
    const GLB_OUT: u32 = MCU_SHARED_REGION_START + 0x6000;
    const CSG_IN: u32 = MCU_SHARED_REGION_START + 0x3000;
    const CSG_OUT: u32 = MCU_SHARED_REGION_START + 0x7000;
    const CS_IN: u32 = MCU_SHARED_REGION_START + 0x3400;
    const CS_OUT: u32 = MCU_SHARED_REGION_START + 0x7200;
    const SUSPEND_SIZE: u32 = 0x4000;
    const PROTM_SUSPEND_SIZE: u32 = 0x5000;

    const fn at(va: u32) -> usize {
        MEM + va as usize
    }

    fn iface() -> Iface {
        Iface { glb_input: at(GLB_IN), glb_output: at(GLB_OUT), group_num: 8, ..Iface::default() }
    }

    fn csg0() -> Csg {
        Csg {
            input: at(CSG_IN),
            output: at(CSG_OUT),
            suspend_size: SUSPEND_SIZE,
            protm_suspend_size: PROTM_SUSPEND_SIZE,
            stream_num: 8,
            ..Csg::default()
        }
    }

    fn cs0() -> Cs {
        Cs { input: at(CS_IN), output: at(CS_OUT), ..Cs::default() }
    }

    /// A stream event the model's firmware raises when CS 0 starts.
    #[derive(Clone, Copy)]
    struct Fault {
        /// `CS_FAULT` or `CS_FATAL`.
        event: u32,
        word: u32,
        info: u64,
    }

    /// The MCU's group and stream request/acknowledge behaviour. On the
    /// global doorbell it takes the groups whose GLB `doorbell_req` bit
    /// differs from `doorbell_ack`; `lag` reads of the shared memory later
    /// it acknowledges CSG 0's request fields (`CSG_REQ_MASK`) and its
    /// `doorbell_req`; and when that doorbell names CS 0 and the group is
    /// started, `cs_lag` reads later CS 0's request fields (`CS_REQ_MASK`),
    /// raising `fault` with them. Every write, memory and register, is
    /// logged in order.
    struct Mcu {
        mem: RefCell<HashMap<usize, u32>>,
        log: RefCell<Vec<(usize, u32)>>,
        lag: u32,
        cs_lag: u32,
        /// Reads left before the group's, then the stream's, answer.
        csg_due: Cell<Option<u32>>,
        cs_due: Cell<Option<u32>>,
        never_ack: bool,
        csg_only: bool,
        fault: Option<Fault>,
        /// An MCU address AS0 does not map, a GPU address the group's VM
        /// does not.
        mcu_hole: Option<u32>,
        vm_hole: Option<u64>,
    }

    impl Mcu {
        fn new() -> Mcu {
            Mcu {
                mem: RefCell::new(HashMap::new()),
                log: RefCell::new(vec![]),
                lag: 3,
                cs_lag: 2,
                csg_due: Cell::new(None),
                cs_due: Cell::new(None),
                never_ack: false,
                csg_only: false,
                fault: None,
                mcu_hole: None,
                vm_hole: None,
            }
        }
        fn word(&self, a: usize) -> u32 {
            *self.mem.borrow().get(&a).unwrap_or(&0)
        }
        fn put(&self, a: usize, v: u32) {
            self.mem.borrow_mut().insert(a, v);
        }
        /// The firmware's side of a field: `ack` takes `req`'s `mask` bits.
        fn ack_field(&self, req: usize, ack: usize, mask: u32) {
            self.put(ack, (self.word(ack) & !mask) | (self.word(req) & mask));
        }
        fn answer_csg(&self) {
            let (gi, go) = (at(GLB_IN), at(GLB_OUT));
            let (db_req, db_ack) = (gi + offset_of!(GlbInput, doorbell_req), go + offset_of!(GlbOutput, doorbell_ack));
            let rung = self.word(db_req) ^ self.word(db_ack);
            self.ack_field(db_req, db_ack, rung);
            if rung & 1 << CSG_SLOT == 0 {
                return;
            }
            let (i, o) = (at(CSG_IN), at(CSG_OUT));
            self.ack_field(i + offset_of!(CsgInput, req), o + offset_of!(CsgOutput, ack), CSG_REQ_MASK);
            let (sreq, sack) = (i + offset_of!(CsgInput, doorbell_req), o + offset_of!(CsgOutput, doorbell_ack));
            let streams = self.word(sreq) ^ self.word(sack);
            self.ack_field(sreq, sack, streams);
            let started = self.word(o + offset_of!(CsgOutput, ack)) & CSG_STATE_MASK == CSG_STATE_START;
            if streams & 1 << CS_SLOT != 0 && started && !self.csg_only {
                self.cs_due.set(Some(self.cs_lag));
            }
        }
        fn answer_cs(&self) {
            let (i, o) = (at(CS_IN), at(CS_OUT));
            let ack = o + offset_of!(CsOutput, ack);
            self.ack_field(i + offset_of!(CsInput, req), ack, CS_REQ_MASK);
            if let Some(f) = self.fault {
                let (word, info) = if f.event == CS_FATAL {
                    (offset_of!(CsOutput, fatal), offset_of!(CsOutput, fatal_info))
                } else {
                    (offset_of!(CsOutput, fault), offset_of!(CsOutput, fault_info))
                };
                self.put(o + word, f.word);
                self.put(o + info, f.info as u32);
                self.put(o + info + 4, (f.info >> 32) as u32);
                self.put(ack, self.word(ack) ^ f.event);
            }
        }
        fn tick(&self, due: &Cell<Option<u32>>, answer: impl FnOnce()) {
            match due.get() {
                Some(0) => {
                    due.set(None);
                    answer();
                }
                Some(n) => due.set(Some(n - 1)),
                None => {}
            }
        }
        fn log(&self) -> Vec<(usize, u32)> {
            self.log.borrow().clone()
        }
    }

    impl Mmio for Mcu {
        fn read32(&self, addr: usize) -> u32 {
            assert!(addr >= MEM, "no GPU register is read: {addr:#x}");
            self.tick(&self.csg_due, || self.answer_csg());
            self.tick(&self.cs_due, || self.answer_cs());
            self.word(addr)
        }
        fn write32(&self, addr: usize, val: u32) {
            self.log.borrow_mut().push((addr, val));
            if addr >= MEM {
                self.put(addr, val);
            } else if addr == GPU + csf::doorbell(csf::GLB_DOORBELL_ID) && !self.never_ack {
                self.csg_due.set(Some(self.lag));
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    impl McuMemory for Mcu {
        fn cpu(&self, va: u32, len: u32) -> Option<usize> {
            match self.mcu_hole {
                Some(h) if h >= va && h - va < len => None,
                _ => Some(at(va)),
            }
        }
        fn clean(&self, _: usize, _: usize) {
            unreachable!("the queue's memory is uncached")
        }
    }

    impl GroupVm for Mcu {
        fn cpu(&self, va: u64, len: u32) -> Option<usize> {
            match self.vm_hole {
                Some(h) if h >= va && h - va < u64::from(len) => None,
                _ => Some(VM + va as usize),
            }
        }
    }

    const T6: Ident = Ident { shader_present: 0x50005, tiler_present: 1, as_present: 0xff, ..zero_ident() };
    const fn zero_ident() -> Ident {
        Ident {
            gpu_id: 0,
            l2_features: 0,
            tiler_features: 0,
            mem_features: 0,
            mmu_features: 0,
            as_present: 0,
            csf_id: 0,
            shader_present: 0,
            tiler_present: 0,
            l2_present: 0,
            coherency_features: 0,
            status: 0,
            mcu_status: 0,
        }
    }

    const SUSPEND_VA: u32 = 0x0510_0000;
    const PROTM_SUSPEND_VA: u32 = 0x0518_0000;
    const QUEUE: Queue = Queue {
        ring_va: 0x1_0080_0000,
        ring_size: 0x1000,
        input_va: 0x0500_0000,
        output_va: 0x0500_0000 + QUEUE_OUTPUT_OFFSET,
        syncobj_va: 0x1_0090_0000,
        priority: QUEUE_PRIORITY,
    };

    fn group() -> Group {
        Group::all_cores(&T6, Some(SUSPEND_VA), Some(PROTM_SUSPEND_VA))
    }

    /// `start` on the model with the T6's group and `QUEUE`, counting the
    /// microseconds slept.
    fn run(mcu: &Mcu, csg: &Csg, g: &Group, q: &Queue) -> (Result<Started, Refused>, u64) {
        let mut slept = 0u64;
        let r = start(mcu, GPU, mcu, mcu, &T6, &iface(), csg, &cs0(), g, q, &mut |us| slept += u64::from(us));
        (r, slept)
    }

    /// Each logged write by its field's name.
    fn named(log: &[(usize, u32)]) -> Vec<(&'static str, u32)> {
        let (ci, gi) = (at(CS_IN), at(CSG_IN));
        let (inp, out) = (at(QUEUE.input_va), at(QUEUE.output_va));
        let sync = VM + QUEUE.syncobj_va as usize;
        let fields: Vec<(usize, &'static str)> = vec![
            (sync, "sync.seqno"),
            (sync + 4, "sync.seqno.hi"),
            (sync + 8, "sync.status"),
            (sync + 12, "sync.pad"),
            (inp, "in.insert"),
            (inp + 4, "in.insert.hi"),
            (inp + 8, "in.extract"),
            (inp + 12, "in.extract.hi"),
            (out, "out.extract"),
            (out + 4, "out.extract.hi"),
            (out + 8, "out.active"),
            (ci + offset_of!(CsInput, req), "cs.req"),
            (ci + offset_of!(CsInput, config), "cs.config"),
            (ci + offset_of!(CsInput, ack_irq_mask), "cs.ack_irq_mask"),
            (ci + offset_of!(CsInput, ringbuf_base), "cs.ringbuf_base"),
            (ci + offset_of!(CsInput, ringbuf_base) + 4, "cs.ringbuf_base.hi"),
            (ci + offset_of!(CsInput, ringbuf_size), "cs.ringbuf_size"),
            (ci + offset_of!(CsInput, ringbuf_input), "cs.ringbuf_input"),
            (ci + offset_of!(CsInput, ringbuf_input) + 4, "cs.ringbuf_input.hi"),
            (ci + offset_of!(CsInput, ringbuf_output), "cs.ringbuf_output"),
            (ci + offset_of!(CsInput, ringbuf_output) + 4, "cs.ringbuf_output.hi"),
            (gi + offset_of!(CsgInput, req), "csg.req"),
            (gi + offset_of!(CsgInput, ack_irq_mask), "csg.ack_irq_mask"),
            (gi + offset_of!(CsgInput, doorbell_req), "csg.doorbell_req"),
            (gi + offset_of!(CsgInput, allow_compute), "csg.allow_compute"),
            (gi + offset_of!(CsgInput, allow_compute) + 4, "csg.allow_compute.hi"),
            (gi + offset_of!(CsgInput, allow_fragment), "csg.allow_fragment"),
            (gi + offset_of!(CsgInput, allow_fragment) + 4, "csg.allow_fragment.hi"),
            (gi + offset_of!(CsgInput, allow_other), "csg.allow_other"),
            (gi + offset_of!(CsgInput, endpoint_req), "csg.endpoint_req"),
            (gi + offset_of!(CsgInput, suspend_buf), "csg.suspend_buf"),
            (gi + offset_of!(CsgInput, suspend_buf) + 4, "csg.suspend_buf.hi"),
            (gi + offset_of!(CsgInput, protm_suspend_buf), "csg.protm_suspend_buf"),
            (gi + offset_of!(CsgInput, protm_suspend_buf) + 4, "csg.protm_suspend_buf.hi"),
            (gi + offset_of!(CsgInput, config), "csg.config"),
            (at(GLB_IN) + offset_of!(GlbInput, doorbell_req), "glb.doorbell_req"),
            (GPU + csf::doorbell(0), "doorbell 0"),
            (GPU + csf::doorbell(CS_DOORBELL_ID), "doorbell 1"),
        ];
        log.iter().map(|&(a, v)| (fields.iter().find(|f| f.0 == a).map_or("?", |f| f.1), v)).collect()
    }

    /// panthor's sequence for one group with one stream in slot 0, as
    /// sched.rs's doc lists it, with the T6's values: the zeroing at the
    /// group's creation (panthor_sched.c:3539-3540, panthor_fw.c:463),
    /// `cs_slot_prog_locked` (:1061-1079), `csg_slot_prog_locked`
    /// (:1323-1345), the request (:2285-2291, 1850-1852) and the group's
    /// doorbell (:1855, panthor_fw.c:1305-1311).
    const PANTHOR_ORDER: [(&str, u32); 40] = [
        ("sync.seqno", 0),
        ("sync.seqno.hi", 0),
        ("sync.status", 0),
        ("sync.pad", 0),
        ("in.insert", 0),
        ("in.insert.hi", 0),
        ("in.extract", 0),
        ("in.extract.hi", 0),
        ("out.extract", 0),
        ("out.extract.hi", 0),
        ("out.active", 0),
        // :1061
        ("in.extract", 0),
        ("in.extract.hi", 0),
        // :1064-1067
        ("cs.ringbuf_base", 0x0080_0000),
        ("cs.ringbuf_base.hi", 1),
        ("cs.ringbuf_size", 0x1000),
        ("cs.ringbuf_input", 0x0500_0000),
        ("cs.ringbuf_input.hi", 0),
        ("cs.ringbuf_output", 0x0500_1000),
        ("cs.ringbuf_output.hi", 0),
        // :1068-1079: priority 1, doorbell 1; ~0; START | IDLE_SYNC_WAIT |
        // IDLE_EMPTY | EXTRACT_EVENT
        ("cs.config", 0x0101),
        ("cs.ack_irq_mask", 0xffff_ffff),
        ("cs.req", 0x0511),
        // :1323-1330: shader_present 0x50005, one tiler, 4/4/1 cores at 0xf, AS 1
        ("csg.allow_compute", 0x50005),
        ("csg.allow_compute.hi", 0),
        ("csg.allow_fragment", 0x50005),
        ("csg.allow_fragment.hi", 0),
        ("csg.allow_other", 1),
        ("csg.endpoint_req", 0xf001_0404),
        ("csg.config", 1),
        // :1332-1342
        ("csg.suspend_buf", SUSPEND_VA),
        ("csg.suspend_buf.hi", 0),
        ("csg.protm_suspend_buf", PROTM_SUSPEND_VA),
        ("csg.protm_suspend_buf.hi", 0),
        // :1344-1345
        ("csg.ack_irq_mask", 0xffff_ffff),
        ("csg.doorbell_req", 1),
        // :2285-2291, 1850-1852: START, ENDPOINT_CONFIG toggled from ack 0
        ("csg.req", 0x11),
        // :1855
        ("glb.doorbell_req", 1),
        ("doorbell 0", 1),
        // (no more writes: the waits only read)
        ("end", 0),
    ];

    #[test]
    fn the_writes_follow_panthors_order() {
        let mcu = Mcu::new();
        let (r, _) = run(&mcu, &csg0(), &group(), &QUEUE);
        assert!(r.is_ok(), "{r:?}");
        let mut got = named(&mcu.log());
        got.push(("end", 0));
        assert_eq!(got, PANTHOR_ORDER);
    }

    #[test]
    fn the_words_of_cs0_and_csg0_are_panthors() {
        let mcu = Mcu::new();
        let (r, _) = run(&mcu, &csg0(), &group(), &QUEUE);
        let s = r.unwrap();
        let w = |a: usize| mcu.word(a);
        let (ci, gi) = (at(CS_IN), at(CSG_IN));
        let cs = [
            offset_of!(CsInput, req),
            offset_of!(CsInput, config),
            offset_of!(CsInput, ack_irq_mask),
            offset_of!(CsInput, ringbuf_base),
            offset_of!(CsInput, ringbuf_base) + 4,
            offset_of!(CsInput, ringbuf_size),
            offset_of!(CsInput, ringbuf_input),
            offset_of!(CsInput, ringbuf_output),
            // left as found
            offset_of!(CsInput, heap_start),
            offset_of!(CsInput, instr_config),
        ]
        .map(|f| w(ci + f));
        assert_eq!(cs, [0x0511, 0x0101, !0, 0x0080_0000, 1, 0x1000, 0x0500_0000, 0x0500_1000, 0, 0]);
        let csg = [
            offset_of!(CsgInput, req),
            offset_of!(CsgInput, ack_irq_mask),
            offset_of!(CsgInput, doorbell_req),
            offset_of!(CsgInput, cs_irq_ack),
            offset_of!(CsgInput, allow_compute),
            offset_of!(CsgInput, allow_fragment),
            offset_of!(CsgInput, allow_other),
            offset_of!(CsgInput, endpoint_req),
            offset_of!(CsgInput, suspend_buf),
            offset_of!(CsgInput, protm_suspend_buf),
            offset_of!(CsgInput, config),
            offset_of!(CsgInput, iter_trace_config),
        ]
        .map(|f| w(gi + f));
        assert_eq!(csg, [0x11, !0, 1, 0, 0x50005, 0x50005, 1, 0xf001_0404, SUSPEND_VA, PROTM_SUSPEND_VA, 1, 0]);
        assert_eq!(
            s,
            Started {
                ring_cpu: VM + 0x1_0080_0000,
                input_cpu: at(0x0500_0000),
                output_cpu: at(0x0500_1000),
                syncobj_cpu: VM + 0x1_0090_0000,
                doorbell_id: 1,
                cs_config: 0x0101,
                cs_req: 0x0511,
                cs_ack: 0x0511,
                endpoint_req: 0xf001_0404,
                csg_config: 1,
                suspend_buf: SUSPEND_VA.into(),
                protm_suspend_buf: PROTM_SUSPEND_VA.into(),
                csg_doorbell_req: 1,
                csg_req: 0x11,
                csg_ack: 0x11,
                glb_doorbell_req: 1,
                priority: 0xf,
                // the model answers the group 3 reads after the doorbell
                // (on the second poll), the stream 2 reads after that (on
                // its first)
                csg_ack_us: 1,
                cs_ack_us: 0,
            }
        );
    }

    #[test]
    fn the_group_rings_doorbell_0_once_after_its_request_and_the_stream_none() {
        let mcu = Mcu::new();
        let (r, _) = run(&mcu, &csg0(), &group(), &QUEUE);
        let s = r.unwrap();
        let log = mcu.log();
        let doorbells: Vec<_> = log.iter().enumerate().filter(|(_, (a, _))| *a < MEM).collect();
        // only the global doorbell, the last write, right after the GLB
        // doorbell_req toggle, which follows CSG 0's request
        assert_eq!(doorbells, [(log.len() - 1, &(GPU + 0x80000, 1))]);
        let names = named(&log);
        assert_eq!(names[log.len() - 3..], [("csg.req", 0x11), ("glb.doorbell_req", 1), ("doorbell 0", 1)]);
        // the stream's doorbell is CSG slot + 1, in its config, for the submit
        assert_eq!((s.doorbell_id, s.cs_config >> 8 & 0xff), (1, 1));
        assert_eq!(csf::doorbell(s.doorbell_id), 0x90000);
        // the firmware took the group's doorbell and CS 0's
        assert_eq!(mcu.word(at(GLB_OUT) + offset_of!(GlbOutput, doorbell_ack)), 1);
        assert_eq!(mcu.word(at(CSG_OUT) + offset_of!(CsgOutput, doorbell_ack)), 1);
    }

    #[test]
    fn a_csg_without_suspend_sizes_gets_no_buffers() {
        let mcu = Mcu::new();
        let csg = Csg { suspend_size: 0, protm_suspend_size: 0, ..csg0() };
        let g = Group { suspend_buf: None, protm_suspend_buf: None, ..group() };
        let s = run(&mcu, &csg, &g, &QUEUE).0.unwrap();
        assert_eq!((s.suspend_buf, s.protm_suspend_buf), (0, 0));
    }

    #[test]
    fn a_second_start_toggles_endpoint_config_against_the_ack() {
        // ENDPOINT_CONFIG is a toggle: with the bit already acknowledged
        // the request clears it (panthor_sched.c:2289-2291)
        let mcu = Mcu::new();
        mcu.put(at(CSG_IN) + offset_of!(CsgInput, req), CSG_ENDPOINT_CONFIG);
        mcu.put(at(CSG_OUT) + offset_of!(CsgOutput, ack), CSG_ENDPOINT_CONFIG);
        let s = run(&mcu, &csg0(), &group(), &QUEUE).0.unwrap();
        assert_eq!((s.csg_req, s.csg_ack), (CSG_STATE_START, CSG_STATE_START));
    }

    /// A refusal before the first write: nothing written.
    fn refused_before_writing(mcu: &Mcu, csg: &Csg, g: &Group, q: &Queue) -> Refused {
        let (r, slept) = run(mcu, csg, g, q);
        assert_eq!((mcu.log(), slept), (vec![], 0));
        r.unwrap_err()
    }

    #[test]
    fn a_ring_size_out_of_bounds_is_refused_before_any_write() {
        for size in [0x800, 0x3000, 0x20000] {
            let q = Queue { ring_size: size, ..QUEUE };
            assert_eq!(refused_before_writing(&Mcu::new(), &csg0(), &group(), &q), (SchedRefusal::RingSize, size));
        }
    }

    #[test]
    fn an_unmapped_ring_is_refused_before_any_write() {
        let mcu = Mcu { vm_hole: Some(QUEUE.ring_va + 0xff8), ..Mcu::new() };
        assert_eq!(refused_before_writing(&mcu, &csg0(), &group(), &QUEUE), (SchedRefusal::RingUnmapped, 0x0080_0000));
        // a 64 KiB ring reaches a hole a 4 KiB one does not
        let mcu = Mcu { vm_hole: Some(QUEUE.ring_va + 0x8000), ..Mcu::new() };
        let q = Queue { ring_size: 0x10000, ..QUEUE };
        assert_eq!(refused_before_writing(&mcu, &csg0(), &group(), &q), (SchedRefusal::RingUnmapped, 0x0080_0000));
    }

    #[test]
    fn unmapped_user_pages_are_refused_before_any_write() {
        for va in [QUEUE.input_va, QUEUE.output_va + 0xffc] {
            let mcu = Mcu { mcu_hole: Some(va), ..Mcu::new() };
            let want = va & !0xfff;
            assert_eq!(
                refused_before_writing(&mcu, &csg0(), &group(), &QUEUE),
                (SchedRefusal::UserPagesUnmapped, want)
            );
        }
    }

    #[test]
    fn an_unmapped_sync_object_is_refused_before_any_write() {
        let mcu = Mcu { vm_hole: Some(QUEUE.syncobj_va + 8), ..Mcu::new() };
        assert_eq!(
            refused_before_writing(&mcu, &csg0(), &group(), &QUEUE),
            (SchedRefusal::SyncObjUnmapped, 0x0090_0000)
        );
    }

    #[test]
    fn a_missing_suspend_buffer_is_refused_before_any_write() {
        let g = Group { suspend_buf: None, ..group() };
        assert_eq!(
            refused_before_writing(&Mcu::new(), &csg0(), &g, &QUEUE),
            (SchedRefusal::SuspendBufMissing, SUSPEND_SIZE)
        );
        let g = Group { protm_suspend_buf: None, ..group() };
        assert_eq!(
            refused_before_writing(&Mcu::new(), &csg0(), &g, &QUEUE),
            (SchedRefusal::SuspendBufMissing, PROTM_SUSPEND_SIZE)
        );
        // given, but AS0 does not map all of it
        let mcu = Mcu { mcu_hole: Some(SUSPEND_VA + SUSPEND_SIZE - 4), ..Mcu::new() };
        assert_eq!(
            refused_before_writing(&mcu, &csg0(), &group(), &QUEUE),
            (SchedRefusal::SuspendBufMissing, SUSPEND_SIZE)
        );
    }

    #[test]
    fn bad_group_and_queue_parameters_are_refused_before_any_write() {
        let cases = [
            (Group { compute_core_mask: 0x10_0000, ..group() }, QUEUE, (SchedRefusal::GroupCores, 1)),
            (Group { tiler_core_mask: 2, ..group() }, QUEUE, (SchedRefusal::GroupCores, 3)),
            (Group { max_fragment_cores: 5, ..group() }, QUEUE, (SchedRefusal::GroupCores, 5)),
            (Group { max_tiler_cores: 2, ..group() }, QUEUE, (SchedRefusal::GroupCores, 6)),
            (group(), Queue { priority: 0x10, ..QUEUE }, (SchedRefusal::QueuePriority, 0x10)),
            (Group { priority: 0x10, ..group() }, QUEUE, (SchedRefusal::CsgPriority, 0x10)),
            (Group { as_nr: 0, ..group() }, QUEUE, (SchedRefusal::AddressSpace, 0)),
            (Group { as_nr: 8, ..group() }, QUEUE, (SchedRefusal::AddressSpace, 8)),
            (Group { as_nr: 40, ..group() }, QUEUE, (SchedRefusal::AddressSpace, 40)),
        ];
        for (g, q, want) in cases {
            assert_eq!(refused_before_writing(&Mcu::new(), &csg0(), &g, &q), want, "{g:?} {q:?}");
        }
        // a tiler field wider than CSG_EP_REQ_TILER's four bits
        let id = Ident { tiler_present: 0xffff, ..T6 };
        let g = Group { tiler_core_mask: 0xffff, max_tiler_cores: 16, ..group() };
        assert_eq!(check_cores(&g, &id), Err((SchedRefusal::GroupCores, 7)));
    }

    #[test]
    fn a_busy_slot_is_refused_before_any_write() {
        let mcu = Mcu::new();
        mcu.put(at(CSG_OUT) + offset_of!(CsgOutput, ack), 0x11);
        assert_eq!(refused_before_writing(&mcu, &csg0(), &group(), &QUEUE), (SchedRefusal::CsgSlotBusy, 0x11));
    }

    #[test]
    fn no_csg_ack_is_refused_after_the_whole_budget() {
        let mcu = Mcu { never_ack: true, ..Mcu::new() };
        let (r, slept) = run(&mcu, &csg0(), &group(), &QUEUE);
        // nothing acknowledged: the state field's 0 bits agree with req's
        // but count only all together (panthor_fw.c:1290-1291)
        assert_eq!(r, Err((SchedRefusal::CsgStartAckTimeout, 0)));
        // one poll per microsecond, 0 to the budget, each failing one followed by 1 µs
        assert_eq!(slept, u64::from(CSG_ACK_WAIT_US) + 1);
        // the sequence was written whole; the refusal wrote nothing more
        assert_eq!(named(&mcu.log()), PANTHOR_ORDER[..PANTHOR_ORDER.len() - 1]);
    }

    #[test]
    fn no_cs_ack_is_refused_with_the_ack_word() {
        let mcu = Mcu { csg_only: true, ..Mcu::new() };
        let (r, slept) = run(&mcu, &csg0(), &group(), &QUEUE);
        assert_eq!(r, Err((SchedRefusal::CsStartAckTimeout, 0)));
        // 1 µs for the group's acknowledge, then the stream's whole budget
        assert_eq!(slept, 1 + u64::from(CS_ACK_WAIT_US) + 1);
        assert_eq!(mcu.word(at(CSG_OUT) + offset_of!(CsgOutput, ack)), 0x11);
    }

    #[test]
    fn a_fault_or_fatal_at_start_is_refused_with_its_word_not_acknowledged() {
        let fault = Fault { event: CS_FAULT, word: 0x0012_3400 | EXCEPTION_CS_INHERIT_FAULT, info: 0xdead_beef };
        let fatal = Fault { event: CS_FATAL, word: 0x0000_0100 | EXCEPTION_CS_UNRECOVERABLE, info: 0x1_0000_0000 };
        for (f, want) in [(fault, SchedRefusal::CsFault), (fatal, SchedRefusal::CsFatal)] {
            let mcu = Mcu { fault: Some(f), ..Mcu::new() };
            let (r, _) = run(&mcu, &csg0(), &group(), &QUEUE);
            assert_eq!(r, Err((want, f.word)));
            // the event is left pending in req ^ ack: req not written again
            let req = at(CS_IN) + offset_of!(CsInput, req);
            assert_eq!(mcu.log().iter().filter(|w| w.0 == req).count(), 1);
            let ack = mcu.word(at(CS_OUT) + offset_of!(CsOutput, ack));
            assert_eq!((mcu.word(req) ^ ack) & CS_EVT_MASK, f.event);
        }
    }

    #[test]
    fn a_fatal_is_reported_before_a_fault_then_oom_then_protm() {
        // panthor_sched.c:1586-1593's order
        let mcu = Mcu::new();
        let (o, i) = (at(CS_OUT), at(CS_IN));
        mcu.put(o + offset_of!(CsOutput, fatal), 0x41);
        mcu.put(o + offset_of!(CsOutput, fault), 0x4b);
        mcu.put(o + offset_of!(CsOutput, heap_vt_start), 0x77);
        let ack = o + offset_of!(CsOutput, ack);
        let steps = [
            (CS_FATAL | CS_FAULT | CS_TILER_OOM | CS_PROTM_PENDING, (SchedRefusal::CsFatal, 0x41)),
            (CS_FAULT | CS_TILER_OOM | CS_PROTM_PENDING, (SchedRefusal::CsFault, 0x4b)),
            (CS_TILER_OOM | CS_PROTM_PENDING, (SchedRefusal::TilerOom, 0x77)),
            (CS_PROTM_PENDING, (SchedRefusal::ProtmPending, CS_PROTM_PENDING)),
        ];
        for (events, want) in steps {
            mcu.put(ack, events);
            assert_eq!(cs_events(&mcu, &cs0()), Err(want));
        }
        mcu.put(ack, 0);
        mcu.put(i + offset_of!(CsInput, req), CS_STATE_START);
        assert_eq!(cs_events(&mcu, &cs0()), Ok(()));
    }
}
