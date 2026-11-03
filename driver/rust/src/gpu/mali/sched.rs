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
//! This file holds the design, the refusals and the constants. The
//! bring-up and the submit are the next items of the plan; the instruction
//! words panthor wraps a job in are [`job`]'s.
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
//! writes to the group's `config` (panthor_sched.c:1330).
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
    self, CSG_ENDPOINT_CONFIG, CSG_STATE_MASK, CS_EXTRACT_EVENT, CS_FATAL, CS_FAULT, CS_IDLE_EMPTY, CS_IDLE_SYNC_WAIT,
    CS_STATE_MASK, CS_STATE_START,
};

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
}

/// The ring's size as panthor's queue creation accepts it
/// (panthor_sched.c:3335-3337).
pub const fn check_ring_size(size: u32) -> Result<u32, (SchedRefusal, u32)> {
    if size < RING_MIN_SIZE || size > RING_MAX_SIZE || !size.is_power_of_two() {
        return Err((SchedRefusal::RingSize, size));
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [SchedRefusal; 21] = [
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
}
