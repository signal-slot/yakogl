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
//! This file holds the design, the refusals, the constants, the bring-up
//! ([`start`]), the submit ([`submit`]), the completion wait ([`wait`]),
//! the stream's drain ([`drained`]) and the group's end ([`stop`], [`terminate`], [`clear_events`],
//! [`stop_and_terminate`]); the instruction words panthor wraps a job in are [`job`]'s.
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
//! charged its padded count (`calc_job_credits`, :3122-3131), and a job's
//! credits come back when its done fence signals, that is when the sync
//! object reached its number (`group_sync_upd_work`, :2932-2937;
//! `drm_sched_job_done`, scheduler/sched_main.c:372). The caller of
//! [`submit`] counts the same way (its `in_flight` bytes), and a job that
//! would not fit is refused instead of held back
//! ([`SchedRefusal::RingOverflow`]). The output page's `extract` plays no
//! part: the firmware publishes it later than the sync object moves
//! (issue #4515), see [`submit`].
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
//! - the job: [`JOB_TIMEOUT_US`], panthor's 5 s, counted from the call
//!   right after the stream's doorbell, polled every [`JOB_POLL_US`]; the
//!   poll reads the group's `req ^ ack` events, the stream's `cs_irq` and
//!   events, and the sync object. It does not read `JOB_INT`: the words
//!   it stands for are read directly, and the interrupt is left to Z3's
//!   setup;
//! - CS 0's state reading STOP: ours, [`CS_ACK_WAIT_US`] (panthor writes
//!   STOP and does not wait);
//! - the terminate's acknowledge: [`CSG_ACK_WAIT_US`], panthor's 100 ms.
//!
//! # What the implementation settled
//!
//! - The stop comes before the terminate, and is rung. panthor terminates
//!   the group and then writes the streams' STOP with no doorbell
//!   (:1275-1288), so the firmware never looks at it before the next start
//!   and its acknowledge could not be checked. [`stop`] writes STOP while
//!   the group still runs, toggles CS 0's bit of the group's
//!   `doorbell_req` and rings the group, as the start does (:1345, 1855),
//!   and waits for the acknowledge. [`stop_and_terminate`] then runs
//!   [`terminate`] whatever the stop's result, and keeps both results
//!   ([`Ended`]; the terminate's is a [`Terminated`], the acknowledge's
//!   microseconds and the events [`clear_events`] cleared after it).
//! - Only the timeout stops and terminates by itself (`queue_timedout_job`,
//!   :3233-3267). After a fault, a fatal, a progress timer event or a sync
//!   object error, [`wait`] returns the record and the caller decides
//!   (panthor goes on after a fault and terminates after a fatal).
//! - The wait acknowledges `CSG_SYNC_UPDATE` and `CSG_IDLE` (panthor's
//!   mask, :1695-1698, less the refused progress timer) and copies
//!   `cs_irq_req` to `cs_irq_ack` (:1687-1693) only when nothing is
//!   refused, then rings the group (:1719-1722). Completion is the sync
//!   object's `seqno` alone. A `seqno` seen before its `SYNC_UPDATE` event
//!   ends the wait with the event still pending; the next wait acknowledges
//!   it. A group state of 4 to 7 on any read of CSG 0's `ack` ends the wait
//!   at once as [`SchedRefusal::CsgStateUnknown`].
//! - 64-bit words go through [`Mmio`] as two 32-bit writes, low half first:
//!   the ring's words, the input page's `extract` and `insert`. panthor
//!   stores them with one 64-bit store. Before the doorbell the firmware
//!   reads `insert` whole, and below 4 GiB of jobs its high half never
//!   changes. The sync object's `seqno` is read high, low, high, so a carry
//!   between the reads is never taken for completion; a high word that
//!   still moves after [`READ64_TRIES`] reads is
//!   [`SchedRefusal::Read64Unstable`], and [`WaitRefused::seqno_unstable`]
//!   marks the record's `seqno` as a plain, possibly torn read.
//! - panthor's `wmb()` before `insert` (:3188-3191) is the `dmb sy` that
//!   every [`crate::mmio::Hw`] write begins with. The ring is uncached, so
//!   nothing is cleaned.
//! - The sync object is the queue's ([`Queue::syncobj_va`], :3019-3020),
//!   not a separate argument of [`submit`]. The sequence number is the
//!   caller's count, refused when the sync object has already reached it.
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
//! - a group state that is none of the four, 4 to 7
//!   ([`SchedRefusal::CsgStateUnknown`]; panthor resets);
//! - a sync object `seqno` whose high word does not settle in
//!   [`READ64_TRIES`] reads ([`SchedRefusal::Read64Unstable`]; ours);
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
//! panthor does). [`wait`]'s timeout does both ends itself, and its
//! [`WaitRefused`] carries the refusal and both results ([`Ended`]).
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
//! event is refused, not acknowledged; `SYNC_UPDATE` is acknowledged and the
//! group rung (1695-1698, 1719-1722). The job timeout is panthor's 5 s (90,
//! 3323). On it CS 0 goes to `STOP`, rung through the group's doorbell and
//! acknowledged within 100 ms (panthor writes it after the terminate, without
//! ringing, 1275-1288). Then `CSG_STATE_TERMINATE` is requested with its
//! 100 ms acknowledge (2197-2207); on it the group's and CS 0's pending
//! events are acknowledged and the group rung, as before the slot is
//! unbound (1665-1723, 2245-2256), and reported. Both results are reported,
//! and a terminate that times out goes to the reset (2742-2754). A group
//! state of 4 to 7 and a sync object `seqno` that does not settle in a few
//! reads are refused by name. `start` does not touch the MMU: the caller
//! has enabled address space 1 and mapped the ring and the sync object in
//! it (the MCU's AS0 holds the user pages and suspend buffers) first.
//!
//! # What the caller has set up before [`start`]
//!
//! [`start`] programs CSG 0 and CS 0 only; it does not touch the MMU. Before
//! it the caller has enabled the group's address space, [`GROUP_AS`] (1),
//! as panthor's `panthor_vm_active` does when the group is bound
//! (panthor_sched.c:982; panthor_mmu.c's `panthor_mmu_as_enable`): the
//! VM's page table base in `AS_TRANSTAB`, its attributes in `AS_MEMATTR`,
//! its configuration in `AS_TRANSCFG`, then the `AS_COMMAND` update
//! command, waited for. In that address space it has mapped, uncached, the
//! ring buffer and the 16-byte sync object ([`GroupVm`]); in the MCU's
//! (AS0) the user input and output pages and the suspend buffers
//! ([`csf::McuMemory`]). [`start`] checks that each range is mapped for
//! the CPU and that the address space is present, not that the GPU's MMU
//! has been programmed: a group started in an address space that is not
//! enabled faults on its first access to the ring.
//!
//! # After the terminate: the pending events cleared
//!
//! panthor processes a terminated group's pending interrupts before it
//! unbinds the slot, "to clear any pending interrupts before we start the
//! new group" (`tick_ctx_apply`, panthor_sched.c:2245-2256, calling
//! `sched_process_csg_irq_locked`, :1665-1723). Once [`terminate`] has its
//! acknowledge it does the same ([`clear_events`]): when anything is
//! pending, `cs_irq_ack` set to `cs_irq_req` (:1687-1693), the group's
//! events `CSG_SYNC_UPDATE | CSG_IDLE | CSG_PROGRESS_TIMER_EVENT`
//! acknowledged by updating `req` to `ack` under them (:1695-1698), CS 0's
//! events acknowledged by updating its `req` to its `ack` (:1598), CS 0's
//! bit of the group's `doorbell_req` toggled after a fault or a tiler OOM
//! (`cs_slot_process_irq_locked`'s return value, :1573-1601) and the group
//! rung (:1719-1722). Two differences, both ours: CS 0's events are
//! acknowledged whether or not its `cs_irq` bit is set, and all four of
//! `CS_EVT_MASK`, not only `CS_FATAL | CS_FAULT`
//! (panthor leaves `CS_TILER_OOM` to its heap work, which acknowledges it
//! only while the group has a slot); a later [`start`] refuses any of them.
//! Nothing is lost: [`Terminated::cleared`] carries what was pending.
//!
//! # The group's state, and 64-bit reads that do not settle
//!
//! The state field of CSG 0's `ack` has four values panthor knows
//! (`CSG_STATE_TERMINATE`, `_START`, `_SUSPEND`, `_RESUME`, panthor_fw.h);
//! `csg_slot_sync_state_locked` takes any other (4 to 7) for a firmware
//! corruption and schedules a reset (panthor_sched.c:1250-1260). Here every
//! read of that `ack` (the slot check before the start, the start's and the
//! terminate's acknowledge waits, the job's poll) refuses such a state at
//! once as [`SchedRefusal::CsgStateUnknown`] with the `ack` word, instead of
//! waiting out a budget for a state that cannot come.
//!
//! The sync object's 64-bit `seqno` is read high, low, high
//! (`read64_stable`) at most [`READ64_TRIES`] times; a high word that
//! still moves is refused as [`SchedRefusal::Read64Unstable`]. Ours:
//! panthor reads it with one 64-bit load.

pub mod job;

use super::csf::{
    self, Cs, CsInput, CsOutput, Csg, CsgInput, CsgOutput, Iface, McuMemory, RingbufInput, RingbufOutput,
    CSG_ENDPOINT_CONFIG, CSG_STATE_MASK, CSG_STATE_START, CSG_STATE_TERMINATE, CS_EVT_MASK, CS_EXTRACT_EVENT, CS_FATAL,
    CS_FAULT, CS_IDLE_EMPTY, CS_IDLE_SYNC_WAIT, CS_PROTM_PENDING, CS_STATE_MASK, CS_STATE_START, CS_TILER_OOM,
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
    /// The job [`submit`] was given cannot be encoded (none is panthor's,
    /// which takes validated ioctl input). The value: the check, 1 a
    /// zero-size stream (panthor runs nothing, panthor_sched.c:3150-3153),
    /// 2 a stream or sync object address above `MOV48`'s 48 bits, 3 the
    /// stream's work registers too few or odd for `addr_reg`
    /// (:3013-3015), 4 a scoreboard count giving no `WAIT` mask (:3023),
    /// 5 a sequence number the sync object has already reached.
    JobParams = 24,
    /// The user input page's `insert` is not a multiple of 8, so the job's
    /// words would straddle the ring's 64-bit slots (the value: `insert`'s
    /// low 32 bits). Ours: panthor's `insert` only ever advances by whole
    /// cache lines (:3116-3119, 3181).
    RingInsert = 25,
    /// A 64-bit word read high, low, high [`READ64_TRIES`] times without
    /// the two highs agreeing (the value: the last high word). Ours:
    /// panthor reads the sync object's `seqno` with one 64-bit load.
    Read64Unstable = 26,
    /// The output page's `extract` did not reach the input page's `insert`
    /// within [`CS_ACK_WAIT_US`] after the job was done (the value:
    /// `extract`'s low 32 bits). Ours: panthor never waits for it.
    DrainTimeout = 27,
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
    check_csg_state(ack)?;
    if ack & CSG_STATE_MASK != CSG_STATE_TERMINATE {
        return Err((SchedRefusal::CsgSlotBusy, ack));
    }
    Ok(Placed { ring_cpu, input_cpu, output_cpu, syncobj_cpu, suspend_buf, protm_suspend_buf })
}

/// CSG 0's `ack` word with a state field panthor knows: `CSG_STATE_TERMINATE`,
/// `_START`, `_SUSPEND` or `_RESUME`. Any other (4 to 7) is what
/// `csg_slot_sync_state_locked` calls a firmware corruption and resets for
/// (panthor_sched.c:1250-1260): [`SchedRefusal::CsgStateUnknown`] with the
/// word.
fn check_csg_state(ack: u32) -> Result<(), Refused> {
    if ack & CSG_STATE_MASK > csf::CSG_STATE_RESUME {
        return Err((SchedRefusal::CsgStateUnknown, ack));
    }
    Ok(())
}

/// [`csf::wait_acks`] on CSG 0 for `mask`, within [`CSG_ACK_WAIT_US`]
/// (`panthor_fw_csg_wait_acks(..., 100)`, panthor_sched.c:1867;
/// panthor_fw.c:1270-1294), reading `req` then `ack` once a poll as it
/// does, with every `ack` read checked by [`check_csg_state`] first. The
/// microseconds it took, or `timeout` with the bits that were (the state
/// field only when all of its bits are, panthor_fw.c:1290-1291).
fn wait_csg_acks(
    m: &impl Mmio,
    csg: &Csg,
    mask: u32,
    timeout: SchedRefusal,
    delay_us: &mut impl FnMut(u32),
) -> Result<u32, Refused> {
    let (req, ack) = (csg.input + offset_of!(CsgInput, req), csg.output + offset_of!(CsgOutput, ack));
    let look = || -> Result<u32, Refused> {
        let (r, a) = (m.read32(req), m.read32(ack));
        check_csg_state(a)?;
        Ok(!(r ^ a) & mask)
    };
    for t in 0..=CSG_ACK_WAIT_US {
        if look()? == mask {
            return Ok(t);
        }
        delay_us(1);
    }
    let a = look()?;
    let a = if a & CSG_STATE_MASK != CSG_STATE_MASK { a & !CSG_STATE_MASK } else { a };
    Err((timeout, a))
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
/// both judged on the same `req` and `ack` read. A group state none of the
/// four is refused on the poll that reads it ([`wait_csg_acks`]).
fn wait_started(m: &impl Mmio, csg: &Csg, cs: &Cs, delay_us: &mut impl FnMut(u32)) -> Result<(u32, u32), Refused> {
    let csg_us = wait_csg_acks(m, csg, CSG_START_MASK, SchedRefusal::CsgStartAckTimeout, delay_us)?;
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
///
/// Precondition, not checked here (the module doc's "What the caller has
/// set up before `start`"): the caller has enabled `group.as_nr`
/// ([`GROUP_AS`], 1) as `panthor_vm_active` does (panthor_sched.c:982):
/// `AS_TRANSTAB`, `AS_MEMATTR`, `AS_TRANSCFG`, then the update command,
/// waited for; and has mapped there, uncached, the ring and the sync object
/// (`vm`), and in AS0 the user input and output pages and the suspend
/// buffers (`mem`). `start` checks the CPU's view of each range and
/// `AS_PRESENT`, not the GPU's MMU.
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
        priority: csf::csg_ep_priority(endpoint_req),
        csg_ack_us,
        cs_ack_us,
    })
}

/// The job the caller hands [`submit`]: what `drm_panthor_queue_submit`
/// carries (`stream_addr`, `stream_size`, `latest_flush`), read by
/// `get_job_cs_params` (panthor_sched.c:3025-3027).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Job {
    /// The user stream: its GPU address in the group's VM and its size in
    /// bytes, the target of the block's `CALL`.
    pub cs_start: u64,
    pub cs_size: u32,
    /// The flush id `FLUSH_CACHE2` compares against (:3044-3048).
    pub latest_flush: u32,
}

/// What [`submit`] wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Submitted {
    /// The sequence number the job's `SYNC_ADD64` brings the sync object
    /// to (panthor's `job->done_fence->seqno`, :3165-3169).
    pub seqno: u64,
    /// `insert` before and after (`job->ringbuf.start`, `.end`, :3175,
    /// 3181), and the ring offset the block went to (:2975).
    pub start: u64,
    pub end: u64,
    pub ring_offset: u32,
    /// The block as written ([`job::job_block`]).
    pub block: [u64; job::JOB_WORDS],
}

/// The job wait's poll interval. Ours: panthor sleeps on the DRM fence.
/// [`JOB_TIMEOUT_US`] is a multiple of it, so the timeout is exact.
pub const JOB_POLL_US: u32 = 10;
const _: () = assert!(JOB_TIMEOUT_US.is_multiple_of(JOB_POLL_US));

/// The group events the wait acknowledges, as panthor's
/// `sched_process_csg_irq_locked` does (panthor_sched.c:1695-1698) less
/// `CSG_PROGRESS_TIMER_EVENT`, which is refused before it.
pub const CSG_EVT_ACK: u32 = csf::CSG_SYNC_UPDATE | csf::CSG_IDLE;

/// How many times [`read64_stable`] reads low then high before it refuses.
/// The sync object's `seqno` moves by 1 a job, so its high word changes
/// once in 2^32 jobs; a high word that changes on 8 reads in a row is not a
/// carry. Ours.
pub const READ64_TRIES: u32 = 8;

/// A 64-bit word the GPU may change between the two 32-bit reads: high,
/// low, high again, until both highs agree, so a carry between the reads
/// is never taken for a jump of 2^32. At most [`READ64_TRIES`] low/high
/// reads after the first high: then [`SchedRefusal::Read64Unstable`] with
/// the last high word.
fn read64_stable(m: &impl Mmio, addr: usize) -> Result<u64, Refused> {
    let mut hi = m.read32(addr + 4);
    for _ in 0..READ64_TRIES {
        let lo = m.read32(addr);
        let again = m.read32(addr + 4);
        if again == hi {
            return Ok(u64::from(hi) << 32 | u64::from(lo));
        }
        hi = again;
    }
    Err((SchedRefusal::Read64Unstable, hi))
}

/// `get_job_cs_params` (panthor_sched.c:3005-3030) from CS 0's features
/// and the queue, checked as [`job::check_params`] does.
fn job_params(cs: &Cs, queue: &Queue, j: &Job) -> Result<job::JobParams, Refused> {
    let regs = csf::cs_features_work_regs(cs.features);
    let refuse = |check: u32| (SchedRefusal::JobParams, check);
    // :3013-3014; job::addr_reg would underflow below the unpreserved count
    let addr_reg = regs.checked_sub(csf::UNPRESERVED_REG_COUNT).ok_or(refuse(3))?;
    let p = job::JobParams {
        addr_reg: u64::from(addr_reg),
        cs_start: j.cs_start,
        cs_size: j.cs_size,
        // :3019-3020: the queue's own 16 bytes of the group's sync objects
        sync_addr: queue.syncobj_va,
        last_flush: j.latest_flush,
        // :3023
        waitall_mask: job::waitall_mask(csf::cs_features_scoreboards(cs.features)),
    };
    // the val_reg (:3015) must be below the stream's work registers too
    if job::val_reg(p.addr_reg) >= u64::from(regs) {
        return Err(refuse(3));
    }
    job::check_params(&p).map_err(|e| {
        refuse(match e {
            job::ParamsError::EmptyStream => 1,
            job::ParamsError::AddressAbove48Bits(_) => 2,
            job::ParamsError::RegisterRange(_) => 3,
            job::ParamsError::WaitMask(_) => 4,
        })
    })?;
    Ok(p)
}

/// One job on the started stream, as `queue_run_job` puts it there
/// (panthor_sched.c:3133-3231):
///
/// 1. checks before any write: a pending stream event ([`cs_events`]), the
///    job's words ([`job_params`]; `seqno` above the sync object's), the
///    ring's room: its size less `in_flight`, the bytes of the jobs the
///    caller has not yet seen complete ([`job::check_ring_fit`] from
///    `insert - in_flight`: [`SchedRefusal::RingOverflow`] with the free
///    bytes, where panthor's DRM scheduler would hold the job back until
///    its credits fit, :3322);
/// 2. [`job::job_block`] (:3039-3120) copied into the ring at `insert &
///    (size - 1)`, wrapping at its end (`copy_instrs_to_ringbuf`,
///    :2969-2994), each 64-bit word as its low then its high half;
/// 3. the input page's `extract` from the output page's, then `insert` to
///    the end (:3193-3194), after the ring: panthor's `wmb()`
///    (:3188-3191) is the `dmb sy` every [`crate::mmio::Hw`] write begins
///    with, and the ring is uncached, so nothing is cleaned;
/// 4. the stream's doorbell, [`Started::doorbell_id`] (:3210).
///
/// `seqno` is the value the job's `SYNC_ADD64` of 1 brings the sync object
/// to: one more than the jobs before it (:3165-3169), the caller's count.
///
/// `in_flight` is panthor's credit count in bytes: [`job::JOB_BYTES`] for
/// each job submitted whose number the caller has not yet read from the
/// sync object. The room is not measured from the output page's `extract`,
/// which the board's firmware publishes later than the sync object moves
/// (issue #4515): a ring whose jobs all completed is free even while
/// `extract` lags. Why the sync object is enough: a job's `SYNC_ADD64` is
/// its word 9, after `WAIT` on every scoreboard (word 8), so words 0 to 9
/// ran when its number is visible. Words 10 to 15, the `ERROR_BARRIER` and
/// five zero words of the padding, may not have run yet; they are written
/// again only by the job one ring later, at the same offset, and every
/// [`job::job_block`] has the same six words there
/// ([`job::JOB_TAIL`]), so that write changes no word the stream has yet
/// to read. panthor has the same order: its credits come back on the done
/// fence, which follows the sync object.
#[allow(clippy::too_many_arguments)]
pub fn submit(
    m: &impl Mmio,
    gpu: usize,
    cs: &Cs,
    queue: &Queue,
    s: &Started,
    j: &Job,
    seqno: u64,
    in_flight: u64,
) -> Result<Submitted, Refused> {
    cs_events(m, cs)?;
    let p = job_params(cs, queue, j)?;
    if seqno <= read64_stable(m, s.syncobj_cpu + offset_of!(SyncObj64, seqno))? {
        return Err((SchedRefusal::JobParams, 5));
    }
    let insert = read64(m, s.input_cpu + offset_of!(RingbufInput, insert));
    let extract = read64(m, s.output_cpu + offset_of!(RingbufOutput, extract));
    if !insert.is_multiple_of(8) {
        return Err((SchedRefusal::RingInsert, insert as u32));
    }
    // a count above insert wraps the start past insert: refused as no room
    job::check_ring_fit(queue.ring_size, insert, insert.wrapping_sub(in_flight), job::JOB_BYTES as u32)?;
    let block = job::job_block(&p);
    let ring_offset = job::ring_offset(insert, queue.ring_size);
    for (i, w) in block.iter().enumerate() {
        let off = job::ring_offset(insert + 8 * i as u64, queue.ring_size) as usize;
        write64(m, s.ring_cpu + off, *w);
    }
    let end = insert + job::JOB_BYTES;
    write64(m, s.input_cpu + offset_of!(RingbufInput, extract), extract);
    write64(m, s.input_cpu + offset_of!(RingbufInput, insert), end);
    csf::ring_doorbell(m, gpu, s.doorbell_id);
    Ok(Submitted { seqno, start: insert, end, ring_offset, block })
}

/// What [`wait`] saw when the job was done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Waited {
    /// The sync object's `seqno`, at least the job's.
    pub seqno: u64,
    /// Microseconds polled from the call (right after the doorbell).
    pub us: u32,
    /// `CSG_SYNC_UPDATE` events acknowledged on the way.
    pub sync_updates: u32,
}

/// [`stop_and_terminate`]'s two results, each kept: one never hides the
/// other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ended {
    /// [`stop`]: the microseconds until CS 0's `ack` read `CS_STATE_STOP`.
    pub stop: Result<u32, Refused>,
    /// [`terminate`]: the microseconds until CSG 0's acknowledge, and the
    /// events cleared after it.
    pub terminate: Result<Terminated, Refused>,
}

/// What [`clear_events`] found pending and cleared, each 0 for nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cleared {
    /// CSG 0's `(req ^ ack) & CSG_EVT_MASK`.
    pub csg_events: u32,
    /// CS 0's bit of `cs_irq_req ^ cs_irq_ack`.
    pub cs_irq: u32,
    /// CS 0's `(req ^ ack) & CS_EVT_MASK`.
    pub cs_events: u32,
}

/// [`terminate`]'s result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terminated {
    /// Microseconds polled until CSG 0's acknowledge.
    pub us: u32,
    /// The events pending after it, now cleared.
    pub cleared: Cleared,
}

/// Why [`wait`] stopped short of the job's completion, with what the
/// refusal's single value cannot carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitRefused {
    pub refused: Refused,
    /// `fault_info` for [`SchedRefusal::CsFault`], `fatal_info` for
    /// [`SchedRefusal::CsFatal`] (panthor prints both, panthor_sched.c:
    /// 1349-1446); 0 otherwise.
    pub info: u64,
    /// The sync object's `seqno` and CS 0's `status_cmd_ptr` when refused
    /// (panthor's timeout report names the queue's progress the same way).
    pub seqno: u64,
    pub status_cmd_ptr: u64,
    /// The `seqno` above did not settle within [`READ64_TRIES`]: it is the
    /// last plain low/high read, possibly torn.
    pub seqno_unstable: bool,
    /// Microseconds polled.
    pub us: u32,
    /// The stop and terminate [`wait`] ran itself: on the timeout only, as
    /// panthor's `queue_timedout_job` terminates the group
    /// (panthor_sched.c:3233-3267). After any other refusal the caller
    /// decides (`None`).
    pub ended: Option<Ended>,
}

/// The poll's verdict on one look at the words.
enum Poll {
    Done(u64),
    Pending,
}

/// The acknowledges [`poll_job`]'s looks made, counted only: what it
/// writes does not depend on them.
#[derive(Clone, Copy, Debug, Default)]
struct Acks {
    /// `CSG_SYNC_UPDATE` events.
    sync_updates: u32,
    /// Looks that set `cs_irq_ack` to `cs_irq_req` for CS 0's bit.
    cs_irqs: u32,
    /// `CSG_IDLE` events.
    idles: u32,
}

/// One look at CSG 0, CS 0 and the sync object, in the order panthor's
/// interrupt handling takes them (`sched_process_csg_irq_locked`,
/// panthor_sched.c:1665-1723), every failure refused before a completion
/// could hide it:
///
/// 1. `CSG_PROGRESS_TIMER_EVENT` in the group's `req ^ ack` (:1703-1704):
///    refused, not acknowledged;
/// 2. CS 0's events ([`cs_events`], :1573-1601): refused, not
///    acknowledged; then, when CS 0's bit of `cs_irq_req ^ cs_irq_ack`
///    is set, `cs_irq_ack` set to `cs_irq_req` (:1687-1693);
/// 3. `CSG_SYNC_UPDATE` and `CSG_IDLE` acknowledged by updating `req` to
///    `ack` under them (:1695-1698; the sync update is
///    `csg_slot_sync_update_locked`'s, :1629-1641), and, when anything was
///    acknowledged, the group rung (:1719-1722);
/// 4. the sync object: `status` non-zero refused, `seqno` (read whole by
///    [`read64_stable`]; [`SchedRefusal::Read64Unstable`] when it does not
///    settle) reaching the job's done (`group_sync_upd_work`'s test, :2933).
///
/// Before all of them a group state of 4 to 7 in CSG 0's `ack` is refused
/// ([`SchedRefusal::CsgStateUnknown`], :1250-1260).
#[allow(clippy::too_many_arguments)]
fn poll_job(
    m: &impl Mmio,
    gpu: usize,
    iface: &Iface,
    csg: &Csg,
    cs: &Cs,
    s: &Started,
    target: u64,
    acks: &mut Acks,
) -> Result<Poll, Refused> {
    let (req, ack) = (csg.input + offset_of!(CsgInput, req), csg.output + offset_of!(CsgOutput, ack));
    let (r, a) = (m.read32(req), m.read32(ack));
    check_csg_state(a)?;
    let events = (r ^ a) & csf::CSG_EVT_MASK;
    if events & csf::CSG_PROGRESS_TIMER_EVENT != 0 {
        return Err((SchedRefusal::CsgProgressTimeout, a));
    }
    cs_events(m, cs)?;
    let mut rung = false;
    let irq_ack = csg.input + offset_of!(CsgInput, cs_irq_ack);
    let irq_req = m.read32(csg.output + offset_of!(CsgOutput, cs_irq_req));
    if (irq_req ^ m.read32(irq_ack)) & 1 << CS_SLOT != 0 {
        m.write32(irq_ack, irq_req);
        acks.cs_irqs += 1;
        rung = true;
    }
    if events & CSG_EVT_ACK != 0 {
        csf::update_reqs(m, req, a, CSG_EVT_ACK);
        if events & csf::CSG_SYNC_UPDATE != 0 {
            acks.sync_updates += 1;
        }
        if events & csf::CSG_IDLE != 0 {
            acks.idles += 1;
        }
        rung = true;
    }
    if rung {
        csf::ring_csg_doorbells(m, gpu, iface, 1 << CSG_SLOT);
    }
    let status = m.read32(s.syncobj_cpu + offset_of!(SyncObj64, status));
    if status != 0 {
        return Err((SchedRefusal::SyncObjError, status));
    }
    let seqno = read64_stable(m, s.syncobj_cpu + offset_of!(SyncObj64, seqno))?;
    Ok(if seqno >= target { Poll::Done(seqno) } else { Poll::Pending })
}

/// The job's completion, polled every [`JOB_POLL_US`] for up to
/// [`JOB_TIMEOUT_US`] (`JOB_TIMEOUT_MS`, panthor_sched.c:90, 3323), each
/// look as [`poll_job`] lists it. On the timeout the group is stopped and
/// terminated ([`stop_and_terminate`], as `queue_timedout_job` does,
/// :3233-3267) and [`SchedRefusal::JobTimeout`] carries the sync object's
/// `seqno` (low 32 bits), with both ends' results in the record
/// ([`WaitRefused::ended`]). Any other refusal leaves the group running,
/// `ended` `None`. Call it
/// right after [`submit`]: the budget counts from the call.
#[allow(clippy::too_many_arguments)]
pub fn wait(
    m: &impl Mmio,
    gpu: usize,
    iface: &Iface,
    csg: &Csg,
    cs: &Cs,
    s: &Started,
    sub: &Submitted,
    delay_us: &mut impl FnMut(u32),
) -> Result<Waited, WaitRefused> {
    wait_seqno(m, gpu, iface, csg, cs, s, sub.seqno, delay_us)
}

/// [`wait`] for the sync object to reach `target`, the [`Submitted::seqno`]
/// of a job submitted earlier: the same looks, the same 5 s budget counted
/// from this call, the same stop and terminate on its timeout. An OSAL that
/// keeps jobs in flight waits for an older job's number this way.
#[allow(clippy::too_many_arguments)]
pub fn wait_seqno(
    m: &impl Mmio,
    gpu: usize,
    iface: &Iface,
    csg: &Csg,
    cs: &Cs,
    s: &Started,
    target: u64,
    delay_us: &mut impl FnMut(u32),
) -> Result<Waited, WaitRefused> {
    match poll_for(m, gpu, iface, csg, cs, s, target, JOB_TIMEOUT_US, delay_us)? {
        Ok(w) => Ok(w),
        Err(t) => {
            // the record's words as the timeout found them, before the ends move them
            let mut r = refused_at(m, cs, s, (SchedRefusal::JobTimeout, 0), t);
            r.refused.1 = r.seqno as u32;
            r.ended = Some(stop_and_terminate(m, gpu, iface, csg, cs, delay_us));
            Err(r)
        }
    }
}

/// The sync object polled for `target` as [`wait_seqno`] polls it, for at
/// most `budget_us` (rounded up to [`JOB_POLL_US`], at most
/// [`JOB_TIMEOUT_US`]): `Ok(None)` when the budget ran out, and then
/// nothing is stopped: the job may still complete. Ours: the OSAL's `wait`
/// with a timeout shorter than panthor's job timeout.
#[allow(clippy::too_many_arguments)]
pub fn poll_seqno(
    m: &impl Mmio,
    gpu: usize,
    iface: &Iface,
    csg: &Csg,
    cs: &Cs,
    s: &Started,
    target: u64,
    budget_us: u32,
    delay_us: &mut impl FnMut(u32),
) -> Result<Option<Waited>, WaitRefused> {
    let budget = budget_us.min(JOB_TIMEOUT_US).next_multiple_of(JOB_POLL_US);
    Ok(poll_for(m, gpu, iface, csg, cs, s, target, budget, delay_us)?.ok())
}

/// The sync object's `seqno` now, with no wait and no event acknowledged:
/// [`SchedRefusal::SyncObjError`] when its `status` is non-zero,
/// [`SchedRefusal::Read64Unstable`] when `seqno` does not settle. What an
/// OSAL's `completed` reads.
pub fn completed_seqno(m: &impl Mmio, s: &Started) -> Result<u64, Refused> {
    let status = m.read32(s.syncobj_cpu + offset_of!(SyncObj64, status));
    if status != 0 {
        return Err((SchedRefusal::SyncObjError, status));
    }
    read64_stable(m, s.syncobj_cpu + offset_of!(SyncObj64, seqno))
}

/// The [`WaitRefused`] of `refused` after `us` microseconds, the words read
/// now.
fn refused_at(m: &impl Mmio, cs: &Cs, s: &Started, refused: Refused, us: u32) -> WaitRefused {
    let o = |f: usize| cs.output + f;
    let info = match refused.0 {
        SchedRefusal::CsFault => read64(m, o(offset_of!(CsOutput, fault_info))),
        SchedRefusal::CsFatal => read64(m, o(offset_of!(CsOutput, fatal_info))),
        _ => 0,
    };
    let at = s.syncobj_cpu + offset_of!(SyncObj64, seqno);
    let (seqno, seqno_unstable) = match read64_stable(m, at) {
        Ok(v) => (v, false),
        Err(_) => (read64(m, at), true),
    };
    WaitRefused {
        refused,
        info,
        seqno,
        status_cmd_ptr: read64(m, o(offset_of!(CsOutput, status_cmd_ptr))),
        seqno_unstable,
        us,
        ended: None,
    }
}

/// [`poll_job`] every [`JOB_POLL_US`] until `target` is reached (`Ok(Ok)`),
/// a look refuses (`Err`), or `budget_us` has passed (`Ok(Err)` with the
/// microseconds polled).
#[allow(clippy::too_many_arguments)]
fn poll_for(
    m: &impl Mmio,
    gpu: usize,
    iface: &Iface,
    csg: &Csg,
    cs: &Cs,
    s: &Started,
    target: u64,
    budget_us: u32,
    delay_us: &mut impl FnMut(u32),
) -> Result<Result<Waited, u32>, WaitRefused> {
    let mut acks = Acks::default();
    let mut t = 0;
    loop {
        match poll_job(m, gpu, iface, csg, cs, s, target, &mut acks) {
            Ok(Poll::Done(seqno)) => return Ok(Ok(Waited { seqno, us: t, sync_updates: acks.sync_updates })),
            Ok(Poll::Pending) => {}
            Err(r) => return Err(refused_at(m, cs, s, r, t)),
        }
        if t >= budget_us {
            return Ok(Err(t));
        }
        delay_us(JOB_POLL_US);
        t += JOB_POLL_US;
    }
}

/// What [`drained`] saw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drained {
    /// Microseconds polled from the call.
    pub us: u32,
    /// The output page's `extract` on the last look.
    pub extract: u64,
    /// `CSG_SYNC_UPDATE` events acknowledged on the way.
    pub sync_updates: u32,
    /// Looks that acknowledged CS 0's `cs_irq` bit (`cs_irq_ack` set to
    /// `cs_irq_req`).
    pub cs_irq_acks: u32,
    /// `CSG_IDLE` events acknowledged on the way.
    pub idle_acks: u32,
}

/// Why [`drained`] stopped short, with what it had seen until then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrainRefused {
    pub refused: Refused,
    pub at: Drained,
}

// drained's look counts its budget in steps of JOB_POLL_US
const _: () = assert!(CS_ACK_WAIT_US.is_multiple_of(JOB_POLL_US));

/// The stream drained: after [`wait`] saw the job done, the output page's
/// `extract` polled every [`JOB_POLL_US`] until it equals the input page's
/// `insert`, within [`CS_ACK_WAIT_US`]; otherwise
/// [`SchedRefusal::DrainTimeout`] with `extract`'s low 32 bits. Each look
/// reads `extract` ([`read64_stable`]) and `insert`, then takes the events
/// as [`poll_job`] does (refusing a progress timeout or a stream event,
/// acknowledging `cs_irq`, `CSG_SYNC_UPDATE` and `CSG_IDLE` and ringing
/// the group, refusing a sync object `status`), with the sequence number
/// already reached, and counts each of the three acknowledges. A refusal
/// leaves the group running.
///
/// Why: the sync object's `seqno` is written by the stream itself (its
/// `SYNC_ADD64`), while `extract` and `CSG_SYNC_UPDATE` are the MCU
/// firmware's to publish. The first NanoPC-T6 Z4 run (issue #4515) saw
/// the job done 220 µs after the doorbell with `extract` still 0 and no
/// sync update; the event was pending at the group's terminate. panthor
/// decides completion from the sync object (`group_sync_upd_work`) and
/// ring space from its scheduler's credits, and reads `output->extract`
/// only when it programs a slot (`cs_slot_prog_locked`, :1061) and for
/// its idle check (`input->insert == output->extract`,
/// `cs_slot_sync_queue_state_locked`); it has no such wait. Ours: the
/// record needs to know when the firmware publishes `extract`. A driver
/// that runs more than one job does not wait for it: [`submit`] takes the
/// ring's room from the jobs the caller saw complete, as panthor's
/// credits.
pub fn drained(
    m: &impl Mmio,
    gpu: usize,
    iface: &Iface,
    csg: &Csg,
    cs: &Cs,
    s: &Started,
    delay_us: &mut impl FnMut(u32),
) -> Result<Drained, DrainRefused> {
    let mut d = Drained::default();
    let mut acks = Acks::default();
    loop {
        let look = read64_stable(m, s.output_cpu + offset_of!(RingbufOutput, extract)).and_then(|extract| {
            d.extract = extract;
            let insert = read64(m, s.input_cpu + offset_of!(RingbufInput, insert));
            let polled = poll_job(m, gpu, iface, csg, cs, s, 0, &mut acks);
            (d.sync_updates, d.cs_irq_acks, d.idle_acks) = (acks.sync_updates, acks.cs_irqs, acks.idles);
            polled?;
            Ok(extract == insert)
        });
        match look {
            Ok(true) => return Ok(d),
            Ok(false) => {}
            Err(refused) => return Err(DrainRefused { refused, at: d }),
        }
        if d.us >= CS_ACK_WAIT_US {
            return Err(DrainRefused { refused: (SchedRefusal::DrainTimeout, d.extract as u32), at: d });
        }
        delay_us(JOB_POLL_US);
        d.us += JOB_POLL_US;
    }
}

/// CS 0 stopped: its state field to `CS_STATE_STOP` under `CS_STATE_MASK`
/// (`cs_slot_reset_locked`'s write, panthor_sched.c:1106-1108), CS 0's bit
/// of the group's `doorbell_req` toggled and the group rung (as for its
/// start, :1345, 1855), then CS 0's `ack` waited for within
/// [`CS_ACK_WAIT_US`]: [`SchedRefusal::CsStopAckTimeout`] with the `ack`
/// word. panthor writes the stop only after the group's terminate and
/// neither rings nor waits (:1275-1288); we stop the stream while its
/// group still runs, so the firmware looks at the request and its
/// acknowledge can be checked.
pub fn stop(
    m: &impl Mmio,
    gpu: usize,
    iface: &Iface,
    csg: &Csg,
    cs: &Cs,
    delay_us: &mut impl FnMut(u32),
) -> Result<u32, Refused> {
    let (req, ack) = (cs.input + offset_of!(CsInput, req), cs.output + offset_of!(CsOutput, ack));
    csf::update_reqs(m, req, csf::CS_STATE_STOP, CS_STATE_MASK);
    csf::toggle_reqs(
        m,
        csg.input + offset_of!(CsgInput, doorbell_req),
        csg.output + offset_of!(CsgOutput, doorbell_ack),
        1 << CS_SLOT,
    );
    csf::ring_csg_doorbells(m, gpu, iface, 1 << CSG_SLOT);
    for t in 0..=CS_ACK_WAIT_US {
        if m.read32(ack) & CS_STATE_MASK == csf::CS_STATE_STOP && (m.read32(req) ^ m.read32(ack)) & CS_STATE_MASK == 0 {
            return Ok(t);
        }
        delay_us(1);
    }
    Err((SchedRefusal::CsStopAckTimeout, m.read32(ack)))
}

/// CSG 0 terminated: its state field to `CSG_STATE_TERMINATE` under
/// `CSG_STATE_MASK` (`tick_ctx_apply`, panthor_sched.c:2197-2207), applied
/// and rung as the start is (:1850-1855), its acknowledge waited for
/// within [`CSG_ACK_WAIT_US`] (:2237, 1867):
/// [`SchedRefusal::CsgTerminateAckTimeout`] with the bits that were (the
/// state field only when all of its bits are, panthor_fw.c:1290-1291).
/// After that refusal the GPU needs the reset (`panthor_sched_suspend`,
/// panthor_sched.c:2742-2754), which is not ours. A state none of the four
/// is refused as [`SchedRefusal::CsgStateUnknown`] on the poll that reads
/// it. On the acknowledge the pending events are cleared ([`clear_events`],
/// as `tick_ctx_apply` does before it unbinds the slot, :2245-2256), so
/// that the next [`start`] does not meet them; what was pending is in
/// [`Terminated::cleared`].
pub fn terminate(
    m: &impl Mmio,
    gpu: usize,
    iface: &Iface,
    csg: &Csg,
    cs: &Cs,
    delay_us: &mut impl FnMut(u32),
) -> Result<Terminated, Refused> {
    let req = csg.input + offset_of!(CsgInput, req);
    csf::update_reqs(m, req, CSG_STATE_TERMINATE, CSG_STATE_MASK);
    csf::ring_csg_doorbells(m, gpu, iface, 1 << CSG_SLOT);
    let us = wait_csg_acks(m, csg, CSG_STATE_MASK, SchedRefusal::CsgTerminateAckTimeout, delay_us)?;
    Ok(Terminated { us, cleared: clear_events(m, gpu, iface, csg, cs) })
}

/// CSG 0's and CS 0's pending events cleared, as panthor's
/// `sched_process_csg_irq_locked` (panthor_sched.c:1665-1723) does when
/// `tick_ctx_apply` runs it on a terminated group before unbinding the slot
/// (:2245-2256), with nothing refused: the group is gone.
///
/// 1. The group's `req`, `ack`, `cs_irq_req`, `cs_irq_ack` (:1677-1680),
///    and CS 0's `req` and `ack`; nothing pending, nothing written
///    (panthor returns when `req == ack` and the `cs_irq` words agree; we
///    also look at CS 0's events);
/// 2. `cs_irq_ack` set to `cs_irq_req` (:1687-1693);
/// 3. `CSG_SYNC_UPDATE | CSG_IDLE | CSG_PROGRESS_TIMER_EVENT` acknowledged:
///    `req` updated to `ack` under [`csf::CSG_EVT_MASK`] (:1695-1698);
/// 4. CS 0's events acknowledged: its `req` updated to its `ack` under
///    [`CS_EVT_MASK`] (panthor: `CS_FATAL | CS_FAULT`, :1598, and only for
///    a stream whose `cs_irq` bit is set; ours, every event, so that a later
///    start refuses none of them);
/// 5. after a `CS_FAULT` or `CS_TILER_OOM`, CS 0's bit of the group's
///    `doorbell_req` toggled (`cs_slot_process_irq_locked`'s return value,
///    :1573-1601), and the group rung (:1719-1722).
pub fn clear_events(m: &impl Mmio, gpu: usize, iface: &Iface, csg: &Csg, cs: &Cs) -> Cleared {
    let (gi, go) = (csg.input, csg.output);
    let greq = gi + offset_of!(CsgInput, req);
    let (req, ack) = (m.read32(greq), m.read32(go + offset_of!(CsgOutput, ack)));
    let irq_req = m.read32(go + offset_of!(CsgOutput, cs_irq_req));
    let irq_ack = m.read32(gi + offset_of!(CsgInput, cs_irq_ack));
    let creq = cs.input + offset_of!(CsInput, req);
    let (cr, ca) = (m.read32(creq), m.read32(cs.output + offset_of!(CsOutput, ack)));
    let cleared = Cleared {
        csg_events: (req ^ ack) & csf::CSG_EVT_MASK,
        cs_irq: (irq_req ^ irq_ack) & 1 << CS_SLOT,
        cs_events: (cr ^ ca) & CS_EVT_MASK,
    };
    if req == ack && irq_req == irq_ack && cleared.cs_events == 0 {
        return cleared;
    }
    m.write32(gi + offset_of!(CsgInput, cs_irq_ack), irq_req);
    csf::update_reqs(m, greq, ack, csf::CSG_EVT_MASK);
    if cleared.cs_events != 0 {
        csf::update_reqs(m, creq, ca, CS_EVT_MASK);
    }
    if cleared.cs_events & (CS_FAULT | CS_TILER_OOM) != 0 {
        csf::toggle_reqs(
            m,
            gi + offset_of!(CsgInput, doorbell_req),
            go + offset_of!(CsgOutput, doorbell_ack),
            1 << CS_SLOT,
        );
    }
    csf::ring_csg_doorbells(m, gpu, iface, 1 << CSG_SLOT);
    cleared
}

/// The group's end: [`stop`], then [`terminate`] whatever the stop's
/// result, both results kept.
pub fn stop_and_terminate(
    m: &impl Mmio,
    gpu: usize,
    iface: &Iface,
    csg: &Csg,
    cs: &Cs,
    delay_us: &mut impl FnMut(u32),
) -> Ended {
    let stop = stop(m, gpu, iface, csg, cs, delay_us);
    let terminate = terminate(m, gpu, iface, csg, cs, delay_us);
    Ended { stop, terminate }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [SchedRefusal; 27] = [
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
        SchedRefusal::JobParams,
        SchedRefusal::RingInsert,
        SchedRefusal::Read64Unstable,
        SchedRefusal::DrainTimeout,
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
        assert_eq!(csf::csg_ep_req(0, 0, 0, CSG_PRIORITY), csf::CSG_EP_REQ_PRIORITY_MASK);
        assert_eq!(csf::csg_ep_priority(csf::csg_ep_req(4, 4, 1, CSG_PRIORITY)), CSG_PRIORITY);
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
    use csf::{GlbInput, GlbOutput, CSG_REQ_MASK};
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
        // 96 work registers, 8 scoreboards (the G610's, as csf.rs's model)
        Cs { input: at(CS_IN), output: at(CS_OUT), features: 0x085f, ..Cs::default() }
    }

    /// The bits of CS 0's `req` the model's firmware copies into `ack`: the
    /// state field and `CS_EXTRACT_EVENT`, not the idle bits
    /// (`CS_IDLE_SYNC_WAIT`, `CS_IDLE_EMPTY`) the start also sets. The first
    /// NanoPC-T6 Z4 run (issue #4515) read `req` 0x0511, `ack` 0x0011; it
    /// cannot tell a copy of bit 4 from an extract event's toggle (both
    /// read 1).
    const CS_ACK_MASK: u32 = CS_STATE_MASK | CS_EXTRACT_EVENT;

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
    /// started, `cs_lag` reads later CS 0's [`CS_ACK_MASK`] bits of `req`,
    /// raising `fault` with them. Every write, memory and register, is
    /// logged in order.
    ///
    /// The submit: on doorbell 1, `job_lag` reads later, it runs every
    /// block between the output page's `extract` and the input page's
    /// `insert`, read from the ring at `extract & (size - 1)` as the
    /// firmware would: `MOV32` and `MOV48` load registers, `SYNC_ADD64`
    /// adds to the 64-bit word at the GPU address it names (and writes
    /// `sync_status`), the rest is skipped. Then `extract` advances to
    /// `insert` and `CSG_SYNC_UPDATE` is raised in CSG 0's `ack`, at once
    /// (`publish_lag` `None`), or, as the board's firmware does,
    /// `publish_lag` reads after the sync object was written, with CS 0's
    /// bit of `cs_irq_req` raised too (the first Z4 run's terminate found
    /// both pending, issue #4515). With
    /// `job_fault` the `SYNC_ADD64` is not reached: the event is raised on
    /// CS 0 and in `cs_irq_req` instead. Knobs: `never_complete` ignores
    /// doorbell 1, `no_stop_ack` and `no_terminate_ack` leave a stop or a
    /// terminate request unacknowledged. `ack_state` puts that value in
    /// CSG 0's acknowledged state field instead of the requested one;
    /// `unstable` names a word whose every read adds 1 to it.
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
        job_lag: u32,
        job_due: Cell<Option<u32>>,
        never_complete: bool,
        job_fault: Option<Fault>,
        sync_status: u32,
        no_stop_ack: bool,
        no_terminate_ack: bool,
        ack_state: Cell<Option<u32>>,
        unstable: Cell<Option<usize>>,
        /// Blocks run.
        runs: Cell<u32>,
        /// Reads from the sync object's write to the output page's
        /// `extract` and the events (`None`: the same read).
        publish_lag: Option<u32>,
        publish_due: Cell<Option<u32>>,
        /// The `extract` the next publish writes.
        published: Cell<u64>,
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
                job_lag: 20,
                job_due: Cell::new(None),
                never_complete: false,
                job_fault: None,
                sync_status: 0,
                no_stop_ack: false,
                no_terminate_ack: false,
                ack_state: Cell::new(None),
                unstable: Cell::new(None),
                runs: Cell::new(0),
                publish_lag: None,
                publish_due: Cell::new(None),
                published: Cell::new(0),
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
            let req = i + offset_of!(CsgInput, req);
            let terminating = self.word(req) & CSG_STATE_MASK == CSG_STATE_TERMINATE;
            let mask = if self.no_terminate_ack && terminating { CSG_REQ_MASK & !CSG_STATE_MASK } else { CSG_REQ_MASK };
            self.ack_field(req, o + offset_of!(CsgOutput, ack), mask);
            if let Some(state) = self.ack_state.get() {
                let a = o + offset_of!(CsgOutput, ack);
                self.put(a, (self.word(a) & !CSG_STATE_MASK) | state);
            }
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
            let req = i + offset_of!(CsInput, req);
            let stopping = self.word(req) & CS_STATE_MASK == csf::CS_STATE_STOP;
            let mask = if self.no_stop_ack && stopping { CS_ACK_MASK & !CS_STATE_MASK } else { CS_ACK_MASK };
            self.ack_field(req, ack, mask);
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
        fn read64(&self, a: usize) -> u64 {
            u64::from(self.word(a)) | u64::from(self.word(a + 4)) << 32
        }
        fn put64(&self, a: usize, v: u64) {
            self.put(a, v as u32);
            self.put(a + 4, (v >> 32) as u32);
        }
        /// A stream event raised on CS 0 and signalled to the group.
        fn raise(&self, f: Fault) {
            let o = at(CS_OUT);
            let (word, info) = if f.event == CS_FATAL {
                (offset_of!(CsOutput, fatal), offset_of!(CsOutput, fatal_info))
            } else {
                (offset_of!(CsOutput, fault), offset_of!(CsOutput, fault_info))
            };
            self.put(o + word, f.word);
            self.put64(o + info, f.info);
            self.put(o + offset_of!(CsOutput, ack), self.word(o + offset_of!(CsOutput, ack)) ^ f.event);
            let irq = at(CSG_OUT) + offset_of!(CsgOutput, cs_irq_req);
            self.put(irq, self.word(irq) ^ 1 << CS_SLOT);
        }
        /// The firmware's side of a submit: the blocks run from the ring.
        fn answer_job(&self) {
            let (inp, out) = (at(QUEUE.input_va), at(QUEUE.output_va));
            let size = u64::from(self.word(at(CS_IN) + offset_of!(CsInput, ringbuf_size)));
            let ring = VM + self.read64(at(CS_IN) + offset_of!(CsInput, ringbuf_base)) as usize;
            let insert = self.read64(inp + offset_of!(RingbufInput, insert));
            let mut extract = self.read64(out + offset_of!(RingbufOutput, extract));
            let mut regs = [0u64; 256];
            while extract < insert {
                for i in 0..job::JOB_WORDS as u64 {
                    let w = self.read64(ring + ((extract + 8 * i) & (size - 1)) as usize);
                    let r = (w >> 48 & 0xff) as usize;
                    match w >> 56 {
                        1 => regs[r] = w & 0xffff_ffff_ffff,
                        2 => regs[r] = w & 0xffff_ffff,
                        51 => {
                            if let Some(f) = self.job_fault {
                                self.raise(f);
                                return;
                            }
                            let sync = VM + regs[(w >> 40 & 0xff) as usize] as usize;
                            self.put64(sync, self.read64(sync) + regs[(w >> 32 & 0xff) as usize]);
                            if self.sync_status != 0 {
                                self.put(sync + offset_of!(SyncObj64, status), self.sync_status);
                            }
                        }
                        _ => {}
                    }
                }
                self.runs.set(self.runs.get() + 1);
                extract += job::JOB_BYTES;
            }
            self.published.set(extract);
            match self.publish_lag {
                Some(n) => self.publish_due.set(Some(n)),
                None => {
                    self.put64(out + offset_of!(RingbufOutput, extract), extract);
                    let ack = at(CSG_OUT) + offset_of!(CsgOutput, ack);
                    self.put(ack, self.word(ack) ^ csf::CSG_SYNC_UPDATE);
                }
            }
        }
        /// The firmware's late side of a job: `extract`, `CSG_SYNC_UPDATE`
        /// and CS 0's `cs_irq_req` bit.
        fn publish(&self) {
            self.put64(at(QUEUE.output_va) + offset_of!(RingbufOutput, extract), self.published.get());
            let ack = at(CSG_OUT) + offset_of!(CsgOutput, ack);
            self.put(ack, self.word(ack) ^ csf::CSG_SYNC_UPDATE);
            let irq = at(CSG_OUT) + offset_of!(CsgOutput, cs_irq_req);
            self.put(irq, self.word(irq) ^ 1 << CS_SLOT);
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
            self.tick(&self.job_due, || self.answer_job());
            self.tick(&self.publish_due, || self.publish());
            let v = self.word(addr);
            if self.unstable.get() == Some(addr) {
                self.put(addr, v.wrapping_add(1));
            }
            v
        }
        fn write32(&self, addr: usize, val: u32) {
            self.log.borrow_mut().push((addr, val));
            if addr >= MEM {
                self.put(addr, val);
            } else if addr == GPU + csf::doorbell(csf::GLB_DOORBELL_ID) && !self.never_ack {
                self.csg_due.set(Some(self.lag));
            } else if addr == GPU + csf::doorbell(CS_DOORBELL_ID) && !self.never_complete {
                self.job_due.set(Some(self.job_lag));
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
            (gi + offset_of!(CsgInput, cs_irq_ack), "csg.cs_irq_ack"),
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
        let ring = VM + QUEUE.ring_va as usize..VM + (QUEUE.ring_va + u64::from(RING_MAX_SIZE)) as usize;
        let name = |a: usize| match fields.iter().find(|f| f.0 == a) {
            Some(f) => f.1,
            None if ring.contains(&a) => "ring",
            None => "?",
        };
        log.iter().map(|&(a, v)| (name(a), v)).collect()
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
                cs_ack: 0x0011,
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

    // ---- the submit, the wait and the end ----

    const STREAM: Job = Job { cs_start: 0x1_0040_0000, cs_size: 0x180, latest_flush: 0x1234 };

    /// CSG 0 / CS 0 started on `mcu`, the log cleared.
    fn started(mcu: &Mcu) -> Started {
        let s = run(mcu, &csg0(), &group(), &QUEUE).0.unwrap();
        mcu.log.borrow_mut().clear();
        s
    }

    fn do_submit(mcu: &Mcu, s: &Started, seqno: u64) -> Result<Submitted, Refused> {
        submit(mcu, GPU, &cs0(), &QUEUE, s, &STREAM, seqno, 0)
    }

    fn do_wait(mcu: &Mcu, s: &Started, sub: &Submitted) -> (Result<Waited, WaitRefused>, u64) {
        let mut slept = 0u64;
        let r = wait(mcu, GPU, &iface(), &csg0(), &cs0(), s, sub, &mut |us| slept += u64::from(us));
        (r, slept)
    }

    /// The block panthor's `prepare_job_instrs` builds for `STREAM` on the
    /// G610 (r92:r93 the address pair, r94 the value, 8 scoreboards): job.rs's
    /// hand-computed example with this module's stream and sync object.
    const BLOCK: [u64; job::JOB_WORDS] = [
        0x025e_0000_0000_1234, // MOV32 r94, latest_flush
        0x2400_5e00_0000_0233, // FLUSH_CACHE2 r94
        0x015c_0001_0040_0000, // MOV48 r92:93, cs.start
        0x025e_0000_0000_0180, // MOV32 r94, cs.size
        0x0300_0000_0001_0000, // WAIT(0)
        0x2000_5c5e_0000_0000, // CALL r92:93, r94
        0x015c_0001_0090_0000, // MOV48 r92:93, sync object
        0x015e_0000_0000_0001, // MOV48 r94, 1
        0x0300_0000_00ff_0000, // WAIT(all)
        0x3300_5c5e_0000_0001, // SYNC_ADD64
        0x2f00_0000_0000_0000, // ERROR_BARRIER
        0,
        0,
        0,
        0,
        0,
    ];

    const RING: usize = VM + QUEUE.ring_va as usize;

    fn ring_word(mcu: &Mcu, off: u32) -> u64 {
        mcu.read64(RING + off as usize)
    }

    /// The input page's `insert` and the output page's `extract` set as if
    /// `n` bytes of jobs had run.
    fn consumed(mcu: &Mcu, n: u64) {
        mcu.put64(at(QUEUE.input_va), n);
        mcu.put64(at(QUEUE.output_va), n);
    }

    #[test]
    fn submit_order_is_ring_then_extract_and_insert_then_doorbell_1() {
        let mcu = Mcu { never_complete: true, ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        assert_eq!((sub.seqno, sub.start, sub.end, sub.ring_offset), (1, 0, 128, 0));
        assert_eq!(sub.block, BLOCK);
        // panthor_sched.c:2969-2994, then 3193-3194, then 3210
        let mut want: Vec<(&str, u32)> =
            BLOCK.iter().flat_map(|w| [("ring", *w as u32), ("ring", (w >> 32) as u32)]).collect();
        want.extend([
            ("in.extract", 0),
            ("in.extract.hi", 0),
            ("in.insert", 128),
            ("in.insert.hi", 0),
            ("doorbell 1", 1),
        ]);
        assert_eq!(named(&mcu.log()), want);
        // each ring write at its word's offset, in order
        let offs: Vec<usize> = mcu.log().iter().take(32).map(|w| w.0 - RING).collect();
        assert_eq!(offs, (0..32).map(|i| 4 * i).collect::<Vec<_>>());
    }

    #[test]
    fn submit_ring_bytes_are_job_blocks_words_at_insert() {
        let mcu = Mcu { never_complete: true, ..Mcu::new() };
        let s = started(&mcu);
        consumed(&mcu, 0x300);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        assert_eq!((sub.start, sub.end, sub.ring_offset), (0x300, 0x380, 0x300));
        for (i, w) in BLOCK.iter().enumerate() {
            assert_eq!(ring_word(&mcu, 0x300 + 8 * i as u32), *w, "word {i}");
        }
        // nothing around it written
        assert_eq!((mcu.word(RING + 0x2fc), mcu.word(RING + 0x380)), (0, 0));
    }

    #[test]
    fn submit_wraps_at_the_ring_end() {
        // insert 64 bytes before the end of the second lap: eight words at
        // 0xfc0, eight at 0 (copy_instrs_to_ringbuf's two copies, :2986-2993)
        let mcu = Mcu::new();
        let s = started(&mcu);
        let insert = 2 * 0x1000 - 64;
        consumed(&mcu, insert);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        assert_eq!((sub.ring_offset, sub.end), (0xfc0, insert + 128));
        for (i, w) in BLOCK.iter().enumerate() {
            let off = (0xfc0 + 8 * i as u32) % 0x1000;
            assert_eq!(ring_word(&mcu, off), *w, "word {i} at {off:#x}");
        }
        assert_eq!(mcu.word(RING + 64), 0);
        // and the model's firmware ran it from there
        let (r, _) = do_wait(&mcu, &s, &sub);
        assert_eq!(r.unwrap().seqno, 1);
        assert_eq!(mcu.read64(at(QUEUE.output_va)), insert + 128);
        // a whole lap: insert at the ring's size lands at offset 0
        let mcu = Mcu { never_complete: true, ..Mcu::new() };
        let s = started(&mcu);
        consumed(&mcu, 0x1000);
        assert_eq!(do_submit(&mcu, &s, 1).unwrap().ring_offset, 0);
        assert_eq!(ring_word(&mcu, 0), BLOCK[0]);
    }

    #[test]
    fn completion_is_the_sync_objects_seqno_with_sync_update_acknowledged() {
        let mcu = Mcu::new();
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        mcu.log.borrow_mut().clear();
        let (r, slept) = do_wait(&mcu, &s, &sub);
        let w = r.unwrap();
        assert_eq!((w.seqno, w.sync_updates), (1, 1));
        assert_eq!(u64::from(w.us), slept);
        assert!(w.us > 0 && w.us.is_multiple_of(JOB_POLL_US), "{w:?}");
        assert_eq!(mcu.runs.get(), 1);
        // the stream consumed the job
        assert_eq!(mcu.read64(at(QUEUE.output_va)), 128);
        // CSG_SYNC_UPDATE acknowledged: req updated to ack under the events
        // (panthor_sched.c:1695-1698), then the group rung (:1719-1722)
        let (req, ack) = (mcu.word(at(CSG_IN)), mcu.word(at(CSG_OUT)));
        assert_eq!((req ^ ack) & csf::CSG_EVT_MASK, 0);
        assert_eq!(req, 0x11 | csf::CSG_SYNC_UPDATE);
        assert_eq!(
            named(&mcu.log()),
            [("csg.req", 0x11 | csf::CSG_SYNC_UPDATE), ("glb.doorbell_req", 0), ("doorbell 0", 1)]
        );
        // a second job on the same stream: seqno 2, the next 128 bytes
        let sub = do_submit(&mcu, &s, 2).unwrap();
        assert_eq!((sub.start, sub.ring_offset), (128, 128));
        let w = do_wait(&mcu, &s, &sub).0.unwrap();
        assert_eq!((w.seqno, w.sync_updates), (2, 1));
        assert_eq!(mcu.word(at(CSG_IN)) ^ mcu.word(at(CSG_OUT)), 0);
    }

    #[test]
    fn ring_overflow_is_refused_with_nothing_written_and_no_doorbell() {
        let mcu = Mcu::new();
        let s = started(&mcu);
        let sub = |in_flight| submit(&mcu, GPU, &cs0(), &QUEUE, &s, &STREAM, 1, in_flight);
        // 64 bytes free: jobs not seen complete hold all of the ring but 64
        mcu.put64(at(QUEUE.input_va), 0x1000);
        assert_eq!(sub(0x1000 - 64), Err((SchedRefusal::RingOverflow, 64)));
        // a full ring
        assert_eq!(sub(0x1000), Err((SchedRefusal::RingOverflow, 0)));
        // more in flight than was ever inserted: no room either
        assert_eq!(sub(0x1001), Err((SchedRefusal::RingOverflow, 0)));
        assert_eq!(mcu.log(), vec![]);
        assert_eq!(mcu.runs.get(), 0);
    }

    /// The room is the caller's count, not the output page's `extract`:
    /// with `extract` a whole ring behind `insert` (the board's firmware
    /// publishes it late, issue #4515) and every job seen complete, the
    /// next job goes on.
    #[test]
    fn a_ring_whose_jobs_completed_has_room_while_extract_lags() {
        let mcu = Mcu::new();
        let s = started(&mcu);
        mcu.put64(at(QUEUE.input_va), 0x1000);
        mcu.put64(at(QUEUE.output_va), 0);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        assert_eq!((sub.start, sub.end, sub.ring_offset), (0x1000, 0x1080, 0));
        // the input page's extract is still the output page's (:3193)
        assert_eq!(mcu.read64(at(QUEUE.input_va + offset_of!(RingbufInput, extract) as u32)), 0);
    }

    #[test]
    fn bad_jobs_are_refused_before_any_write() {
        let mcu = Mcu::new();
        let s = started(&mcu);
        let cases = [
            (Job { cs_size: 0, ..STREAM }, cs0(), 1, (SchedRefusal::JobParams, 1)),
            (Job { cs_start: 1 << 48, ..STREAM }, cs0(), 1, (SchedRefusal::JobParams, 2)),
            // 3 work registers, then 7: too few for r(X), r(X+2)
            (STREAM, Cs { features: 2, ..cs0() }, 1, (SchedRefusal::JobParams, 3)),
            (STREAM, Cs { features: 6, ..cs0() }, 1, (SchedRefusal::JobParams, 3)),
            // no scoreboards
            (STREAM, Cs { features: 0x5f, ..cs0() }, 1, (SchedRefusal::JobParams, 4)),
            (STREAM, cs0(), 0, (SchedRefusal::JobParams, 5)),
        ];
        for (j, cs, seqno, want) in cases {
            assert_eq!(submit(&mcu, GPU, &cs, &QUEUE, &s, &j, seqno, 0), Err(want), "{j:?} {cs:?}");
        }
        consumed(&mcu, 4);
        assert_eq!(do_submit(&mcu, &s, 1), Err((SchedRefusal::RingInsert, 4)));
        assert_eq!(mcu.log(), vec![]);
    }

    #[test]
    fn a_cs_fault_mid_job_is_refused_with_fault_word_and_info() {
        let f = Fault { event: CS_FAULT, word: 0x0012_3400 | EXCEPTION_CS_INHERIT_FAULT, info: 0x1_dead_beef };
        let mcu = Mcu { job_fault: Some(f), ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        let (r, slept) = do_wait(&mcu, &s, &sub);
        let e = r.unwrap_err();
        assert_eq!((e.refused, e.info, e.seqno, e.ended), ((SchedRefusal::CsFault, f.word), f.info, 0, None));
        assert_eq!(u64::from(e.us), slept);
        // left pending: neither the stream's event nor the group's cs_irq
        // acknowledged
        let ack = at(CS_OUT) + offset_of!(CsOutput, ack);
        assert_eq!((mcu.word(at(CS_IN)) ^ mcu.word(ack)) & CS_EVT_MASK, CS_FAULT);
        assert_eq!(mcu.word(at(CSG_IN) + offset_of!(CsgInput, cs_irq_ack)), 0);
        assert_eq!(mcu.word(at(CSG_OUT) + offset_of!(CsgOutput, cs_irq_req)), 1);
    }

    #[test]
    fn a_cs_fatal_mid_job_is_refused_with_fatal_word_and_info() {
        let f = Fault { event: CS_FATAL, word: 0x0000_0100 | EXCEPTION_CS_UNRECOVERABLE, info: 0x2_0000_0040 };
        let mcu = Mcu { job_fault: Some(f), ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        let e = do_wait(&mcu, &s, &sub).0.unwrap_err();
        assert_eq!((e.refused, e.info, e.ended), ((SchedRefusal::CsFatal, f.word), f.info, None));
        assert_eq!((mcu.word(at(CS_IN)) ^ mcu.word(at(CS_OUT))) & CS_EVT_MASK, CS_FATAL);
    }

    #[test]
    fn a_sync_object_status_is_refused_with_its_value() {
        let mcu = Mcu { sync_status: 0xffff_ffff, ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        let e = do_wait(&mcu, &s, &sub).0.unwrap_err();
        // refused although seqno reached the job's
        assert_eq!((e.refused, e.seqno, e.ended), ((SchedRefusal::SyncObjError, 0xffff_ffff), 1, None));
    }

    #[test]
    fn a_progress_timer_event_is_refused_not_acknowledged() {
        let mcu = Mcu { never_complete: true, ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        let ack = at(CSG_OUT) + offset_of!(CsgOutput, ack);
        mcu.put(ack, mcu.word(ack) ^ csf::CSG_PROGRESS_TIMER_EVENT);
        let e = do_wait(&mcu, &s, &sub).0.unwrap_err();
        assert_eq!(e.refused, (SchedRefusal::CsgProgressTimeout, 0x11 | csf::CSG_PROGRESS_TIMER_EVENT));
        assert_eq!(mcu.word(at(CSG_IN)), 0x11);
    }

    /// The timeout of a job that never completes, `mcu`'s knobs as given.
    fn timed_out(mcu: &Mcu) -> (WaitRefused, u64) {
        let s = started(mcu);
        mcu.put64(at(CS_OUT) + offset_of!(CsOutput, status_cmd_ptr), 0x1_0040_0040);
        let sub = do_submit(mcu, &s, 1).unwrap();
        mcu.log.borrow_mut().clear();
        let (r, slept) = do_wait(mcu, &s, &sub);
        (r.unwrap_err(), slept)
    }

    #[test]
    fn a_job_timeout_sleeps_exactly_5_s_then_stops_and_terminates() {
        let mcu = Mcu { never_complete: true, ..Mcu::new() };
        let (e, slept) = timed_out(&mcu);
        assert_eq!((e.refused, e.seqno, e.status_cmd_ptr), ((SchedRefusal::JobTimeout, 0), 0, 0x1_0040_0040));
        assert_eq!(e.us, JOB_TIMEOUT_US);
        let ended = e.ended.unwrap();
        let (stop_us, term_us) = (ended.stop.unwrap(), ended.terminate.unwrap().us);
        // the model answers the group 3 reads after doorbell 0 and the
        // stream 2 reads after that: the stop's poll reads one word until
        // the state reads STOP (5 µs), the terminate's reads two (1 µs)
        assert_eq!((stop_us, term_us), (5, 1));
        assert_eq!(slept, u64::from(JOB_TIMEOUT_US) + u64::from(stop_us) + u64::from(term_us));
        // the polls wrote nothing; then the stop, then the terminate
        assert_eq!(
            named(&mcu.log()),
            [
                ("cs.req", 0x0510),
                ("csg.doorbell_req", 0),
                ("glb.doorbell_req", 0),
                ("doorbell 0", 1),
                ("csg.req", 0x10),
                ("glb.doorbell_req", 1),
                ("doorbell 0", 1),
            ]
        );
        assert_eq!(mcu.word(at(CS_OUT)) & CS_STATE_MASK, csf::CS_STATE_STOP);
        assert_eq!(mcu.word(at(CSG_OUT)) & CSG_STATE_MASK, CSG_STATE_TERMINATE);
    }

    #[test]
    fn no_stop_ack_is_refused_and_the_terminate_still_runs() {
        let mcu = Mcu { never_complete: true, no_stop_ack: true, ..Mcu::new() };
        let (e, slept) = timed_out(&mcu);
        assert_eq!(e.refused.0, SchedRefusal::JobTimeout);
        let ended = e.ended.unwrap();
        // CS 0's ack still reads the start's state
        assert_eq!(ended.stop, Err((SchedRefusal::CsStopAckTimeout, 0x0011)));
        assert_eq!(ended.terminate, Ok(Terminated { us: 1, cleared: Cleared::default() }));
        assert_eq!(slept, u64::from(JOB_TIMEOUT_US) + u64::from(CS_ACK_WAIT_US) + 1 + 1);
    }

    #[test]
    fn no_terminate_ack_is_refused_after_the_whole_budget() {
        let mcu = Mcu { never_complete: true, no_terminate_ack: true, ..Mcu::new() };
        let (e, slept) = timed_out(&mcu);
        let ended = e.ended.unwrap();
        assert_eq!(ended.stop, Ok(5));
        // the state field not acknowledged: none of its bits counted
        assert_eq!(ended.terminate, Err((SchedRefusal::CsgTerminateAckTimeout, 0)));
        assert_eq!(slept, u64::from(JOB_TIMEOUT_US) + 5 + u64::from(CSG_ACK_WAIT_US) + 1);
        assert_eq!(mcu.word(at(CSG_OUT)) & CSG_STATE_MASK, CSG_STATE_START);
        // on their own, outside the timeout path, the same refusals
        let mcu = Mcu { no_stop_ack: true, no_terminate_ack: true, ..Mcu::new() };
        started(&mcu);
        let mut slept = 0u64;
        let ended = stop_and_terminate(&mcu, GPU, &iface(), &csg0(), &cs0(), &mut |us| slept += u64::from(us));
        assert_eq!(
            ended,
            Ended {
                stop: Err((SchedRefusal::CsStopAckTimeout, 0x0011)),
                terminate: Err((SchedRefusal::CsgTerminateAckTimeout, 0)),
            }
        );
        assert_eq!(slept, 2 * (u64::from(CSG_ACK_WAIT_US) + 1));
    }

    // ---- after the terminate, unknown states, unsettled 64-bit reads ----

    #[test]
    fn terminate_clears_pending_events_so_a_later_start_meets_none() {
        // a fault mid-job, refused and left pending (cs_irq and CS_FAULT),
        // and a CSG_SYNC_UPDATE the wait never acknowledged
        let f = Fault { event: CS_FAULT, word: EXCEPTION_CS_INHERIT_FAULT, info: 0 };
        let mcu = Mcu { job_fault: Some(f), ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        assert_eq!(do_wait(&mcu, &s, &sub).0.unwrap_err().refused.0, SchedRefusal::CsFault);
        let gack = at(CSG_OUT) + offset_of!(CsgOutput, ack);
        mcu.put(gack, mcu.word(gack) ^ csf::CSG_SYNC_UPDATE);
        mcu.log.borrow_mut().clear();
        let ended = stop_and_terminate(&mcu, GPU, &iface(), &csg0(), &cs0(), &mut |_| {});
        assert!(ended.stop.is_ok(), "{ended:?}");
        let t = ended.terminate.unwrap();
        assert_eq!(t.cleared, Cleared { csg_events: csf::CSG_SYNC_UPDATE, cs_irq: 1, cs_events: CS_FAULT });
        // after the terminate's three writes, the clearing's
        // (panthor_sched.c:1687-1698, 1598, 1719-1722): cs_irq_ack, the
        // group's req, CS 0's req, CS 0's doorbell bit (after a fault),
        // the group's doorbell
        let log = named(&mcu.log());
        let term = log.iter().position(|w| *w == ("csg.req", 0x10)).unwrap();
        assert_eq!(
            log[term + 3..],
            [
                ("csg.cs_irq_ack", 1),
                ("csg.req", 0x10 | csf::CSG_SYNC_UPDATE),
                ("cs.req", 0x0510 | CS_FAULT),
                ("csg.doorbell_req", 1),
                ("glb.doorbell_req", 0),
                ("doorbell 0", 1),
            ]
        );
        // nothing pending any more
        let w = |a: usize| mcu.word(a);
        assert_eq!((w(at(CSG_IN)) ^ w(gack)) & csf::CSG_EVT_MASK, 0);
        assert_eq!(
            w(at(CSG_IN) + offset_of!(CsgInput, cs_irq_ack)),
            w(at(CSG_OUT) + offset_of!(CsgOutput, cs_irq_req))
        );
        assert_eq!((w(at(CS_IN)) ^ w(at(CS_OUT))) & CS_EVT_MASK, 0);
        // so the slot starts again, its stream's events clean
        let s = run(&mcu, &csg0(), &group(), &QUEUE).0.unwrap();
        assert_eq!(s.csg_ack & CSG_STATE_MASK, CSG_STATE_START);
        // with nothing pending the clearing writes nothing
        let mcu = Mcu::new();
        started(&mcu);
        assert_eq!(clear_events(&mcu, GPU, &iface(), &csg0(), &cs0()), Cleared::default());
        assert_eq!(mcu.log(), vec![]);
    }

    #[test]
    fn an_unknown_csg_state_is_refused_with_the_ack_word() {
        // before the start: a slot in state 4 to 7 is neither free nor busy
        for state in 4..=7 {
            let mcu = Mcu::new();
            mcu.put(at(CSG_OUT) + offset_of!(CsgOutput, ack), CSG_ENDPOINT_CONFIG | state);
            assert_eq!(
                refused_before_writing(&mcu, &csg0(), &group(), &QUEUE),
                (SchedRefusal::CsgStateUnknown, CSG_ENDPOINT_CONFIG | state)
            );
        }
        // the start acknowledged with state 5: refused on the poll that
        // reads it (the second, as for a good start), not after 100 ms
        let mcu = Mcu::new();
        mcu.ack_state.set(Some(5));
        let (r, slept) = run(&mcu, &csg0(), &group(), &QUEUE);
        assert_eq!(r, Err((SchedRefusal::CsgStateUnknown, CSG_ENDPOINT_CONFIG | 5)));
        assert_eq!(slept, 1);
        // during the job
        let mcu = Mcu { never_complete: true, ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        let gack = at(CSG_OUT) + offset_of!(CsgOutput, ack);
        mcu.put(gack, (mcu.word(gack) & !CSG_STATE_MASK) | 6);
        let e = do_wait(&mcu, &s, &sub).0.unwrap_err();
        assert_eq!((e.refused, e.us, e.ended), ((SchedRefusal::CsgStateUnknown, 0x16), 0, None));
        // the terminate acknowledged with state 7
        let mcu = Mcu::new();
        started(&mcu);
        mcu.ack_state.set(Some(7));
        let mut slept = 0u64;
        let r = terminate(&mcu, GPU, &iface(), &csg0(), &cs0(), &mut |us| slept += u64::from(us));
        assert_eq!((r, slept), (Err((SchedRefusal::CsgStateUnknown, 0x17)), 1));
    }

    /// CS 0 raising `event` as the firmware does: its `ack` bit flipped
    /// against `req`, the group's `cs_irq_req` bit for CS 0 flipped.
    fn raise_event(mcu: &Mcu, event: u32) {
        let ack = at(CS_OUT) + offset_of!(CsOutput, ack);
        mcu.put(ack, mcu.word(ack) ^ event);
        let irq = at(CSG_OUT) + offset_of!(CsgOutput, cs_irq_req);
        mcu.put(irq, mcu.word(irq) ^ 1 << CS_SLOT);
    }

    #[test]
    fn tiler_oom_is_refused_by_name_and_cleared_with_the_doorbell_toggled() {
        // panthor_sched.c:1584-1593 reads the three events from
        // (req ^ ack) & CS_EVT_MASK; CS_TILER_OOM's word is heap_vt_start.
        // 0x77 is the model's.
        let oom_word = at(CS_OUT) + offset_of!(CsOutput, heap_vt_start);

        // a job under way: the wait refuses it by name, with that word,
        // writes nothing (no acknowledge, no doorbell) and ends nothing
        let mcu = Mcu { never_complete: true, ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        mcu.put(oom_word, 0x77);
        raise_event(&mcu, CS_TILER_OOM);
        mcu.log.borrow_mut().clear();
        let (r, slept) = do_wait(&mcu, &s, &sub);
        let e = r.unwrap_err();
        assert_eq!((e.refused, e.us, slept, e.ended), ((SchedRefusal::TilerOom, 0x77), 0, 0, None));
        assert_eq!(mcu.log(), vec![]);
        // a submit meets it first, before any write
        assert_eq!(do_submit(&mcu, &s, 2), Err((SchedRefusal::TilerOom, 0x77)));
        assert_eq!(mcu.log(), vec![]);

        // the group's end: the event acknowledged on CS 0's req, the
        // stream's bit of doorbell_req toggled and the group rung
        // (cs_slot_process_irq_locked returns true for CS_FAULT |
        // CS_TILER_OOM, :1573-1601, and the group's ring follows, :1719-1722)
        let ended = stop_and_terminate(&mcu, GPU, &iface(), &csg0(), &cs0(), &mut |_| {});
        let t = ended.terminate.unwrap();
        assert_eq!(t.cleared, Cleared { csg_events: 0, cs_irq: 1, cs_events: CS_TILER_OOM });
        let log = named(&mcu.log());
        let term = log.iter().position(|w| *w == ("csg.req", 0x10)).unwrap();
        assert_eq!(
            log[term + 3..],
            [
                ("csg.cs_irq_ack", 1),
                ("csg.req", 0x10),
                ("cs.req", 0x0510 | CS_TILER_OOM),
                ("csg.doorbell_req", 1),
                ("glb.doorbell_req", 0),
                ("doorbell 0", 1),
            ]
        );
        let w = |a: usize| mcu.word(a);
        assert_eq!((w(at(CS_IN)) ^ w(at(CS_OUT))) & CS_EVT_MASK, 0);

        // a fatal alone is acknowledged too but toggles nothing: panthor
        // returns the toggle only for a fault or a tiler OOM
        let mcu = Mcu { never_complete: true, ..Mcu::new() };
        started(&mcu);
        raise_event(&mcu, CS_FATAL);
        let c = clear_events(&mcu, GPU, &iface(), &csg0(), &cs0());
        assert_eq!(c, Cleared { csg_events: 0, cs_irq: 1, cs_events: CS_FATAL });
        let log = named(&mcu.log());
        assert!(log.iter().all(|w| w.0 != "csg.doorbell_req"), "{log:?}");
        assert_eq!(log.last(), Some(&("doorbell 0", 1)));
    }

    // ---- the firmware's late publish of extract, and the drain ----

    /// The model's late publish: 10 reads after the sync object's write,
    /// after the wait's last look (its seqno read) and before the group's
    /// end has cleared the events, as on the board.
    const PUBLISH_LAG: u32 = 10;

    fn do_drained(mcu: &Mcu, s: &Started) -> (Result<Drained, DrainRefused>, u64) {
        let mut slept = 0u64;
        let r = drained(mcu, GPU, &iface(), &csg0(), &cs0(), s, &mut |us| slept += u64::from(us));
        (r, slept)
    }

    #[test]
    fn the_board_s_firmware_publishes_extract_and_the_sync_update_after_the_seqno() {
        // the first Z4 run (issue #4515): the wait saw seqno 1 with extract
        // 0 and no sync update; the terminate then cleared CSG_SYNC_UPDATE
        // and CS 0's cs_irq bit (cleared 0x10000000 1 0)
        let mcu = Mcu { publish_lag: Some(PUBLISH_LAG), ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        let w = do_wait(&mcu, &s, &sub).0.unwrap();
        assert_eq!((w.seqno, w.sync_updates), (1, 0));
        assert_eq!(mcu.read64(at(QUEUE.output_va)), 0);
        let mut slept = 0u64;
        let ended = stop_and_terminate(&mcu, GPU, &iface(), &csg0(), &cs0(), &mut |us| slept += u64::from(us));
        let cleared = ended.terminate.unwrap().cleared;
        assert_eq!(cleared, Cleared { csg_events: csf::CSG_SYNC_UPDATE, cs_irq: 1, cs_events: 0 });
        assert_eq!(mcu.read64(at(QUEUE.output_va)), 128);
    }

    #[test]
    fn drained_waits_for_extract_and_acknowledges_the_late_events() {
        // a publish later than the drain's first look
        let mcu = Mcu { publish_lag: Some(40), ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        assert_eq!(do_wait(&mcu, &s, &sub).0.unwrap().sync_updates, 0);
        mcu.log.borrow_mut().clear();
        let (r, slept) = do_drained(&mcu, &s);
        let d = r.unwrap();
        // the late publish raised CS 0's cs_irq bit with the sync update
        // (the first run's cleared[1] = 1): the drain counts its
        // acknowledge
        assert_eq!((d.extract, d.sync_updates, d.cs_irq_acks, d.idle_acks), (128, 1, 1, 0));
        assert_eq!(u64::from(d.us), slept);
        assert!(d.us > 0 && d.us.is_multiple_of(JOB_POLL_US), "{d:?}");
        // the events acknowledged as the wait's poll does: cs_irq_ack to
        // cs_irq_req, the group's req to its ack under the events, the
        // group rung once
        assert_eq!(
            named(&mcu.log()),
            [
                ("csg.cs_irq_ack", 1),
                ("csg.req", 0x11 | csf::CSG_SYNC_UPDATE),
                ("glb.doorbell_req", 0),
                ("doorbell 0", 1),
            ]
        );
        // so the group's end finds nothing pending
        let ended = stop_and_terminate(&mcu, GPU, &iface(), &csg0(), &cs0(), &mut |_| {});
        assert_eq!(ended.terminate.unwrap().cleared, Cleared::default());
    }

    #[test]
    fn drained_after_an_instant_publish_returns_at_once_and_writes_nothing() {
        let mcu = Mcu::new();
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        assert_eq!(do_wait(&mcu, &s, &sub).0.unwrap().sync_updates, 1);
        mcu.log.borrow_mut().clear();
        let (r, slept) = do_drained(&mcu, &s);
        // the wait took the events: nothing left for the drain to count
        let d = Drained { us: 0, extract: 128, ..Drained::default() };
        assert_eq!((r, slept), (Ok(d), 0));
        assert_eq!(mcu.log(), vec![]);
    }

    #[test]
    fn drained_counts_a_csg_idle_it_acknowledges() {
        let mcu = Mcu::new();
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        do_wait(&mcu, &s, &sub).0.unwrap();
        let ack = at(CSG_OUT) + offset_of!(CsgOutput, ack);
        mcu.put(ack, mcu.word(ack) ^ csf::CSG_IDLE);
        let (r, slept) = do_drained(&mcu, &s);
        let d = Drained { us: 0, extract: 128, idle_acks: 1, ..Drained::default() };
        assert_eq!((r, slept), (Ok(d), 0));
        assert_eq!(mcu.word(at(CSG_IN)) ^ mcu.word(ack), 0);
    }

    #[test]
    fn an_extract_that_never_comes_is_refused_after_the_budget() {
        let mcu = Mcu { publish_lag: Some(u32::MAX), ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        assert_eq!(do_wait(&mcu, &s, &sub).0.unwrap().seqno, 1);
        mcu.log.borrow_mut().clear();
        let (r, slept) = do_drained(&mcu, &s);
        let at = Drained { us: CS_ACK_WAIT_US, ..Drained::default() };
        assert_eq!(r, Err(DrainRefused { refused: (SchedRefusal::DrainTimeout, 0), at }));
        assert_eq!(slept, u64::from(CS_ACK_WAIT_US));
        // nothing pending was acknowledged, nothing written
        assert_eq!(mcu.log(), vec![]);
    }

    #[test]
    fn a_fault_after_the_job_is_refused_by_the_drain_with_what_it_saw() {
        let mcu = Mcu { publish_lag: Some(PUBLISH_LAG), ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        do_wait(&mcu, &s, &sub).0.unwrap();
        mcu.put(at(CS_OUT) + offset_of!(CsOutput, fault), EXCEPTION_CS_INHERIT_FAULT);
        raise_event(&mcu, CS_FAULT);
        let (r, slept) = do_drained(&mcu, &s);
        // the extract was read before the events (published by then)
        let at = Drained { us: 0, extract: 128, ..Drained::default() };
        assert_eq!(
            (r, slept),
            (Err(DrainRefused { refused: (SchedRefusal::CsFault, EXCEPTION_CS_INHERIT_FAULT), at }), 0)
        );
    }

    #[test]
    fn read64_stable_gives_up_on_a_high_word_that_keeps_changing() {
        let hi = VM + QUEUE.syncobj_va as usize + 4;
        // the submit's check of seqno: refused before any write
        let mcu = Mcu { never_complete: true, ..Mcu::new() };
        let s = started(&mcu);
        mcu.unstable.set(Some(hi));
        assert_eq!(do_submit(&mcu, &s, 1), Err((SchedRefusal::Read64Unstable, READ64_TRIES)));
        assert_eq!(mcu.log(), vec![]);
        // the high word read once, then once a try
        assert_eq!(mcu.word(hi), READ64_TRIES + 1);
        // the wait's poll: refused at once, the record's seqno marked
        let mcu = Mcu { never_complete: true, ..Mcu::new() };
        let s = started(&mcu);
        let sub = do_submit(&mcu, &s, 1).unwrap();
        mcu.unstable.set(Some(hi));
        let (r, slept) = do_wait(&mcu, &s, &sub);
        let e = r.unwrap_err();
        assert_eq!((e.refused, e.us, slept, e.ended), ((SchedRefusal::Read64Unstable, READ64_TRIES), 0, 0, None));
        assert!(e.seqno_unstable);
        // the refusal carries the last high word read, whatever it started at
        let mcu = Mcu::new();
        mcu.put64(hi - 4, 0x1_ffff_ffff);
        mcu.unstable.set(Some(hi));
        assert_eq!(read64_stable(&mcu, hi - 4), Err((SchedRefusal::Read64Unstable, 1 + READ64_TRIES)));
        // and a word that holds still reads whole on the first try
        mcu.unstable.set(None);
        mcu.put64(hi - 4, 0x2_0000_0000);
        assert_eq!(read64_stable(&mcu, hi - 4), Ok(0x2_0000_0000));
    }
}
