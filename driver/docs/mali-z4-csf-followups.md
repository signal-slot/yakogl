<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali Z4: what Z4 needs from `csf.rs` (follow-up list)

**Status: G1, G2 and G4 are in `csf.rs` (the three items `sched.rs` marked
"Not in [`csf`]"); G3 and G5-G16 are still local in `sched.rs`, or not yet
written.** Z4 (`driver/rust/src/gpu/mali/sched.rs` and its `job` submodule) may
not edit `driver/rust/src/gpu/mali/csf.rs` (Z3's firmware interface). Every
CSF item Z4 needs that `csf.rs` lacks is defined locally in `sched.rs` for
now; this note is the list of what must move into `csf.rs`, entry by entry,
in one follow-up commit once the sched work is on main. Until that commit,
the local definitions are the temporary ones, each `sched.rs` item below
says "Not in [`csf`]" where it says so.

References:

- `linux:` Linux v6.18 (`7d0a66e4`, the tag `v6.18`),
  `drivers/gpu/drm/panthor/` — `panthor_fw.h` (503 lines), `panthor_fw.c`
  (1410 lines), `panthor_sched.c` (3991 lines). Every line cited below was
  read from that tree. `panthor_fw.h:1-2` and `panthor_sched.c:1-2` carry
  `SPDX-License-Identifier: GPL-2.0 or MIT` (Copyright 2023 Collabora ltd.);
  panthor is used, transcribed and quoted here **under the MIT option**, as
  the design page's "Decisions" 1 records. Two secondary citations name
  `panthor_device.h` and `panthor_mmu.c` of the same tree.
- `main:` `driver/rust/src/gpu/mali/csf.rs` and `driver/rust/src/gpu/mali/sched.rs`
  as of `8c51d5c3`. Where this note gives a `sched.rs:` line number it is
  that snapshot; the plan's bring-up and jobwords items are editing those
  files, so the follow-up commit should locate each item by its name, not
  its line.

Entry ids are G1 to G16. They supersede the id'd gap list the plan's
`csf-gaps` reading item reported by handoff: same intent, now written down
where a follow-up can work through it. Each entry gives: the panthor fact
with file:line; the exact Rust item to add to `csf.rs` (name, kind, value,
with the repo's `pub const` / `pub const fn` / `#[repr(C)]`-layout style);
and the `sched.rs` local definition it replaces. Section M lists the ways a
naive caller of `csf.rs`'s existing helpers misuses them; a follow-up that
lands G1-G16 should land M's doc notes with it. What is *identical* today
("Already equal") is listed so the follow-up does not re-verify it.

## The additions

### G1 — the stream's user input page: `RingbufInput`

`struct panthor_fw_ringbuf_input_iface` is `{ u64 insert; u64 extract; }`
(`panthor_fw.h:15-18`). The host writes `insert` after the ring copy, behind
a barrier (`panthor_sched.c:3188-3191, 3194`), and syncs `extract` from the
output page before it (`panthor_sched.c:1061, 3193`); the ring-overflow test
reads both. Replace the local `#[repr(C)] pub struct RingbufInput { pub
insert: u64, pub extract: u64 }` (`sched.rs:368-372`, marked "Not in
[`csf`]") with a `csf.rs` definition of the same layout, plus its size
assert (`const _: () = assert!(size_of::<RingbufInput>() == 0x10);`, the
style of `csf.rs:893-901`).

### G2 — the stream's user output page: `RingbufOutput`

`struct panthor_fw_ringbuf_output_iface` is `{ u64 extract; u32 active; }`
(`panthor_fw.h:20-23`). `extract` is what the firmware has consumed
(`panthor_sched.c:1415` reads it to match a fault to a job; :1061, :3193
copy it to the input page). Replace the local `#[repr(C)] pub struct
RingbufOutput { pub extract: u64, pub active: u32 }` (`sched.rs:376-380`,
"Not in [`csf`]") with the same in `csf.rs`, plus the 0x10 size assert.
Both pages live in one 8 KiB object — see G15.

### G3 — the job's completion: `SyncObj64`

`struct panthor_syncobj_64b` is `{ u64 seqno; u32 status; u32 pad; }`:
seqno is the sequence number the job's `SYNC_ADD64` increments; `status` is
"Not zero on failure" (`panthor_sched.c:338-351`); `pad` is MBZ. One per
queue, `queues.count * 16` bytes mapped NOEXEC|UNCACHED in the **group's**
VM and zeroed (`panthor_sched.c:3522-3540`). Completion is
`syncobj->seqno >= job->seqno` (`panthor_sched.c:2933`); a terminate writes
the object itself — `status = ~0`, `seqno` the queue's last sequence number
(`panthor_sched.c:2106-2110`). Replace the local `#[repr(C)] pub struct
SyncObj64 { pub seqno: u64, pub status: u32, pub pad: u32 }` and
`pub const SYNCOBJ_SIZE: u32 = size_of::<SyncObj64>() as u32`
(`sched.rs:385-394`) with `csf.rs`
definitions carrying those semantics in their doc comments (the words
"Non-zero on failure" and "MBZ" are panthor's own, :343-350).

### G4 — reading the endpoint priority back: `CSG_EP_REQ_PRIORITY_MASK` and `csg_ep_priority`

`CSG_EP_REQ_PRIORITY(x)` and `CSG_EP_REQ_PRIORITY_MASK` are
`GENMASK(31, 28)` (`panthor_fw.h:170-171`). After the start's acknowledge,
panthor reads the applied priority back as
`(input->endpoint_req & CSG_EP_REQ_PRIORITY_MASK) >> 28`
(`panthor_sched.c:1131-1141` at :1140, called at :1869-1870 when
`acked & CSG_ENDPOINT_CONFIG`). `csf.rs` has `csg_ep_req` (which masks
priority into `0xf000_0000` when *writing*, `csf.rs:268-270`) but no mask
constant and no getter for *reading*. Add:

```rust
pub const CSG_EP_REQ_PRIORITY_MASK: u32 = 0xf000_0000;
pub const fn csg_ep_priority(endpoint_req: u32) -> u32 {
    (endpoint_req & CSG_EP_REQ_PRIORITY_MASK) >> 28
}
```

and replace `sched.rs:342-343` (`pub const CSG_EP_REQ_PRIORITY_MASK`, the
`0xf000_0000` already there, marked "Not in [`csf`]") with a use of them.

### G5 — `ACK_IRQ_MASK_ALL`

panthor writes `ack_irq_mask = ~0` on both the stream (`panthor_sched.c:1070`)
and the group (`panthor_sched.c:1344`): every request bit asks for the job
interrupt. `csf.rs` has `CsgInput::ack_irq_mask` and `CsInput::ack_irq_mask`
but no constant. Add `pub const ACK_IRQ_MASK_ALL: u32 = !0;` and replace
`sched.rs:347`.

### G6 — the stream's start word: `CS_START_REQ` and `CS_START_MASK`

`cs_slot_prog_locked` updates the CS `req` to
`CS_IDLE_SYNC_WAIT | CS_IDLE_EMPTY | CS_STATE_START | CS_EXTRACT_EVENT`
under the mask `CS_IDLE_SYNC_WAIT | CS_IDLE_EMPTY | CS_STATE_MASK |
CS_EXTRACT_EVENT` (`panthor_sched.c:1071-1079`) — one **update** of level
bits (`panthor_fw_update_reqs`, `panthor_fw.h:435-458`), not a toggle, with
no doorbell yet. Those are `csf.rs`'s own bits (`csf.rs:341-354`) combined;
the combination belongs in `csf.rs`. Add:

```rust
pub const CS_START_REQ: u32 = CS_IDLE_SYNC_WAIT | CS_IDLE_EMPTY | CS_STATE_START | CS_EXTRACT_EVENT; // 0x0511
pub const CS_START_MASK: u32 = CS_IDLE_SYNC_WAIT | CS_IDLE_EMPTY | CS_STATE_MASK | CS_EXTRACT_EVENT; // 0x0517
```

and replace `sched.rs:350-351`.

### G7 — the group's start requests: `CSG_START_MASK`

A group start queues two things (the tick's start path,
`panthor_sched.c:2285-2291`): the state field set to `CSG_STATE_START`
under `CSG_STATE_MASK`, and `CSG_ENDPOINT_CONFIG` toggled (queued as the
value `ack ^ CSG_ENDPOINT_CONFIG` under its own mask; applied by one
`panthor_fw_update_reqs` at :1850-1852). Note `CSG_STATUS_UPDATE`
(`panthor_fw.h:144`) is **not** requested at the start: `csf.rs`'s
`CSG_REQ_MASK` (`csf.rs:264`, per `panthor_fw.h:149-151`) is the legal
*request* mask, not the *start* mask. Add `pub const CSG_START_MASK: u32 =
CSG_STATE_MASK | CSG_ENDPOINT_CONFIG; // 0x17` and replace `sched.rs:354`.

### G8 — what a stream's event words get acknowledged: `CS_EVT_ACK`

`cs_slot_process_irq_locked` acknowledges exactly `CS_FATAL | CS_FAULT` by
updating `req` to `ack` under that mask (`panthor_sched.c:1598`), and
deliberately does **not** acknowledge `CS_TILER_OOM` — its handling is
deferred to separate work (:1595-1597). `CS_PROTM_PENDING` is left pending
too (it never appears in an ack call). Add
`pub const CS_EVT_ACK: u32 = CS_FATAL | CS_FAULT; // 0xc000_0000` and
replace `sched.rs:357`. Z4 refuses all four events instead of acknowledging
them (`sched.rs`'s "What we refuse" list); the constant still names which
ones panthor acknowledges, which is what the refusal record must say.

### G9 — the pending-event words: `cs_events_pending` and `csg_events_pending`

Both event sets are computed the same way: pending events are
`(req ^ ack)` masked — `events = (req ^ ack) & CS_EVT_MASK`
(`panthor_sched.c:1584`, dispatched at :1586-1593) and
`csg_events = (req ^ ack) & CSG_EVT_MASK` (`panthor_sched.c:1681`,
dispatched at :1700-1704, :1716-1717). Add:

```rust
pub const fn cs_events_pending(req: u32, ack: u32) -> u32 { (req ^ ack) & CS_EVT_MASK }
pub const fn csg_events_pending(req: u32, ack: u32) -> u32 { (req ^ ack) & CSG_EVT_MASK }
```

Z4's completion poll needs both (its job poll reads the group's and the
stream's events alongside the sync object); `sched.rs`'s doc names the
words (:1573-1601, :1665-1723) and the bring-up item will hold the
expression inline — move it to `csf.rs` next to `CS_EVT_MASK`/`CSG_EVT_MASK`
(`csf.rs:265, 355`).

### G10 — copying the CS interrupt bits: `copy_csg_irq_ack`

`sched_process_csg_irq_locked` reads `cs_irq_req` (output) and `cs_irq_ack`
(input) (:1679-1680), then writes `input->cs_irq_ack = cs_irq_req`
immediately, before examining anything, with the stated race reason
(:1687-1693), and the pending set is `cs_irq_req ^ cs_irq_ack` of *that
moment* (:1706, looping per stream at :1707-1714). Note this is a plain
word copy between two dedicated fields, **not** a req/ack helper call:
`cs_irq_ack` is a host-owned input field (`panthor_fw.h:159`), not a bit of
the `req` word. Add:

```rust
/// `sched_process_csg_irq_locked`'s copy (panthor_sched.c:1687-1693): set the group's
/// `cs_irq_ack` to its `cs_irq_req` and return the pending bits (`cs_irq_req ^ cs_irq_ack`
/// as read, panthor_sched.c:1706).
pub fn copy_csg_irq_ack(m: &impl Mmio, csg_in: usize, csg_out: usize) -> u32 {
    let req = m.read32(csg_out + offset_of!(CsgOutput, cs_irq_req));
    let old = m.read32(csg_in + offset_of!(CsgInput, cs_irq_ack));
    m.write32(csg_in + offset_of!(CsgInput, cs_irq_ack), req);
    req ^ old
}
```

### G11 — the group's per-stream doorbell request: `csg_toggle_doorbell_req`

`csg_slot_prog_locked` toggles the group input's `doorbell_req` against its
`doorbell_ack` for the mask of the group's streams (`panthor_sched.c:1345`,
queue_mask built at :1316-1321 — bit 0 for CS 0), so the firmware looks at
CS 0 when the group starts. `sched_process_csg_irq_locked` re-rings the same
pair for the streams whose events need re-processing — fault or tiler OOM
(:1710-1711, :1719-1720). `csf.rs` has `ring_csg_doorbells`, which is the
*GLB side* (GLB `doorbell_req` + the global doorbell, `panthor_fw.c:1305-1311`);
it has nothing for the CSG pair. Add:

```rust
/// `panthor_fw_toggle_reqs` on the group's own doorbell pair (panthor_sched.c:1345, 1719-1720).
pub fn csg_toggle_doorbell_req(m: &impl Mmio, csg_in: usize, csg_out: usize, cs_mask: u32) {
    toggle_reqs(m, csg_in + offset_of!(CsgInput, doorbell_req),
                   csg_out + offset_of!(CsgOutput, doorbell_ack), cs_mask);
}
```

### G12 — the stream's doorbell id: `cs_doorbell_id`

`group_bind_locked` gives every queue of the group the doorbell
`csg_id + 1` (`panthor_sched.c:997-998`: "doorbell is assigned to the group
and all queues use the same doorbell"), and `queue_run_job` rings exactly
that (`panthor_sched.c:3210`). `csf.rs` owns `doorbell(id)` and
`GLB_DOORBELL_ID` (`csf.rs:59-65`) but not the id rule; a bring-up that
rings `ring_csg_doorbells` and thinks the group's doorbell is rung for the
*stream* gets it wrong (that one is the GLB pair — see G11; :1855 applies
the group's state request through it). Add `pub const fn cs_doorbell_id(csg_slot: u32) -> u32 { csg_slot + 1 }`
and replace `sched.rs:309` (`pub const CS_DOORBELL_ID: u32 = CSG_SLOT + 1`).

### G13 — masks a `wait_acks` may take: the two guards

panthor refuses a wait whose mask asks the wrong direction. For the GLB:
`req_mask & (~GLB_REQ_MASK | GLB_HALT)` is a `drm_WARN_ON` and `-EINVAL`
("GLB_HALT doesn't get acked through the FW interface",
`panthor_fw.c:1249-1251`); for a CSG: `req_mask & ~CSG_REQ_MASK` is
`-EINVAL` (`panthor_fw.c:1276-1277`). `csf.rs`'s `wait_acks` and `acked`
(`csf.rs:710-733`) take any mask: a caller that passes event bits waits for
bits the firmware *reports*, never bits the host *requests*, and times out
as if the firmware were deaf. Add the two guards as named refusals in
`CsfRefusal` (`csf.rs:389-430`), e.g.

```rust
/// A wait mask outside the legal request bits (panthor_fw.c:1249-1251 for GLB, :1276-1277 for CSG).
pub const fn glb_wait_mask(mask: u32) -> bool { mask & !(GLB_REQ_MASK & !GLB_HALT) == 0 }
pub const fn csg_wait_mask(mask: u32) -> bool { mask & !CSG_REQ_MASK == 0 }
```

and a `CsfRefusal::AckMaskOutside` checked by `wait_acks`'s callers; Z4's
start/terminate waits pass [`CSG_START_MASK`] (G7), which the guard admits.

### G14 — the state-field ack rule: `csg_acked`

`panthor_fw_csg_wait_acks` post-processes the reported `*acked` after the
wait: "Check that all bits in the state field were updated, if any mismatch
then clear all bits in the state field. This allows code to do (acked &
CSG_STATE_MASK) and get the right value" — the rule is
`if ((*acked & CSG_STATE_MASK) != CSG_STATE_MASK) *acked &= ~CSG_STATE_MASK;`
(`panthor_fw.c:1284-1291`, the two lines at :1290-1291). Its consumers are
`if (acked & CSG_STATE_MASK) csg_slot_sync_state_locked(...)` and the
endpoint consumer right before it (`panthor_sched.c:1869-1873`). `csf.rs`'s
`acked()` and `wait_acks`'s error value report per-bit agreement
(`csf.rs:710, 732`; the same expression panthor reports at
`panthor_fw.c:1229`) with no field rule: a partially acknowledged state
field reads as a garbage state (section M4 works one out). Add:

```rust
/// `panthor_fw_csg_wait_acks`'s field rule (panthor_fw.c:1290-1291): a state field counts as acknowledged only as a whole.
pub const fn csg_acked(acked: u32) -> u32 {
    if acked & CSG_STATE_MASK == CSG_STATE_MASK { acked } else { acked & !CSG_STATE_MASK }
}
```

Apply it to the mask returned by any `csf::wait_acks` that waited on a CSG
req word (this is what `csgs_upd_ctx_apply_locked`'s consumers rely on).
Z4's `CsgStartAckTimeout`/`CsgTerminateAckTimeout` report the masked value.

### G15 — the queue interface object: `QUEUE_IFACE_SIZE`, `QUEUE_OUTPUT_OFFSET`

`panthor_fw_alloc_queue_iface_mem` (`panthor_fw.c:439-470`, declared
`panthor_fw.h:477-481`) allocates one **8 KiB** object in the MCU's VM,
NOEXEC|UNCACHED (`panthor_kernel_bo_create(ptdev, ptdev->fw->vm, SZ_8K, ...,
VM_BIND_OP_MAP_NOEXEC | VM_BIND_OP_MAP_UNCACHED, ...)`, :448-451), zeroes it
(:463), puts the input page at offset 0 and the output page at +4 KiB
(:464-467; the doc says exactly "The input interface is at offset 0, and the
output interface at offset 4096", :434-435). Add
`pub const QUEUE_IFACE_SIZE: u32 = 8 * 1024;` and
`pub const QUEUE_OUTPUT_OFFSET: u32 = 4 * 1024;` to `csf.rs` (the CSF-side
memory contract, next to `McuMemory`) and replace `sched.rs:322-323`.

### G16 — naming a stalled stream: the `CS_STATUS_WAIT_*` and `CS_STATUS_BLOCKED_REASON_*` bits (diagnostic, lowest priority)

`panthor_fw.h:84-94` defines the bits of the stream output's `status_wait`
word (`CS_STATUS_WAIT_SB_MASK` 0xffff, `..._SB_SRC_MASK` 0xf_0000,
`..._SB_SRC_NONE` 0, `..._SB_SRC_WAIT` 8<<16, `CS_STATUS_WAIT_SYNC_COND_LE`
0 / `..._GT` 1<<24 under `..._COND_MASK` 0xf00_0000, `CS_STATUS_WAIT_PROGRESS`
1<<28, `CS_STATUS_WAIT_PROTM` 1<<29, `CS_STATUS_WAIT_SYNC_64B` 1<<30,
`CS_STATUS_WAIT_SYNC` 1<<31) and `panthor_fw.h:101-108` the values of
`status_blocked_reason` (`..._UNBLOCKED` 0, `..._SB_WAIT` 1,
`..._PROGRESS_WAIT` 2, `..._SYNC_WAIT` 3, `..._DEFERRED` 4, `..._RESOURCE`
5, `..._FLUSH` 6 under `CS_STATUS_BLOCKED_REASON_MASK` 0xf). `csf.rs` has
the two fields (`CsOutput`, `csf.rs:321, 325`) but none of the bits. Z4's
words do not need them; a Z4 record that names *why* a job timed out or a
CS refused to start does (its `JobTimeout` refusal currently carries only
the seqno). Add the 15 constants to `csf.rs` next to the `CsOutput` layout;
no `sched.rs` definition to replace (the bring-up item reads the words
inline if at all).

## M — how a naive caller misuses `toggle_reqs` and `update_reqs`

`csf.rs` transcribes the two macros (`csf.rs:695-706` from
`panthor_fw.h:424-433` and :450-458). Each request bit belongs to one of
three classes, and the class decides the helper. A follow-up should put
this list in `csf.rs`'s doc comment over the two helpers.

- **M1 — level fields go through `update_reqs`, never `toggle_reqs`.**
  The state fields and the idle bits: the CS update of G6
  (`panthor_sched.c:1071-1079`), `CS_STATE_STOP` on reset
  (`panthor_sched.c:1106-1108`), the group state applied from the tick's
  queued value (`panthor_sched.c:1850-1852` with :2285-2288), and
  `GLB_IDLE_EN` in `csf.rs`'s own `global_init` (`csf.rs:822`). Failure
  worked out: a CS slot reused from an earlier group still reads
  `req == ack == CS_START_REQ` (0x0511). A bring-up that *toggles*
  G6's mask computes `(ack ^ mask) & mask = 0x511 ^ 0x517 = 0x006`:
  the state field becomes 6 — none of STOP/START/SUSPEND/RESUME — and the
  idle bits clear; that is the "invalid state" path
  (`panthor_sched.c:1250-1260` on the group side) instead of a start.
- **M2 — edge bits go through `toggle_reqs` (or the queued `ack ^ bit`
  value), never `update_reqs` with a fixed value.** `CSG_ENDPOINT_CONFIG`
  is queued as `csg_iface->output->ack ^ CSG_ENDPOINT_CONFIG` under its own
  mask (`panthor_sched.c:2289-2291`); doorbell bits are toggled against
  their ack words (`panthor_fw.c:1309`, `panthor_sched.c:1345, 1719-1720`,
  and `csf.rs`'s own `ring_csg_doorbells` and `ping` do it right at
  `csf.rs:742-750, 831-837`). Failure worked out: `update_reqs(req, 1,
  GLB_..._doorbell)` rings the group exactly once — after the firmware
  mirrors the bit into `doorbell_ack` there is no difference left, and the
  start or terminate waits as if the MCU were deaf (`CsgStartAckTimeout`,
  `CsgTerminateAckTimeout`). One nuance to record: panthor reads the ack it
  complements when *queuing* into the tick ctx and applies the value
  later (:2289-2291 applied at :1850-1852); `csf::toggle_reqs` re-reads the
  ack at write time instead. On a single core with one group that is the
  same request; say so in the doc so nobody later "fixes" a difference.
- **M3 — event bits are acknowledged by an update to the *ack's own value*,
  never by a toggle, and never all at once.** `panthor_fw_update_reqs(cs_iface,
  req, ack, CS_FATAL | CS_FAULT)` (`panthor_sched.c:1598`) and
  `panthor_fw_update_reqs(csg_iface, req, ack, CSG_SYNC_UPDATE | CSG_IDLE |
  CSG_PROGRESS_TIMER_EVENT)` (`panthor_sched.c:1695-1698`): the host copies
  the ack into the request to erase the difference. `CS_TILER_OOM` is
  deliberately *left* pending (:1595-1597). Toggling an event bit instead
  re-creates a difference on a bit whose difference means "event raised",
  and the irq handler's skip test `req == ack`
  (`panthor_sched.c:1683-1685`) is never reached: a poll loop spins. Z4
  refuses each event with its word instead of acknowledging any of them —
  the class rules still say which ones *may* be acknowledged afterwards.
- **M4 — `acked()` reports bits, the state is a field.** A wait's error
  value run through `& CSG_STATE_MASK` without G14's rule reports a partial
  field as a state: with the start's mask `CSG_START_MASK` (0x17) —
  `req = 0x17`, firmware acknowledged the endpoint but not the state,
  `ack = 0x16` — `csf::acked` returns `!(req ^ ack) & mask` = `~1 & 0x17`
  = 0x16, and `0x16 & 0x7` = 6 reads as an "invalid state 6" (a false
  `CsgStateUnknown`, value 6) instead of the true story: endpoint acked,
  state pending (`panthor_sched.c:1869-1873` reads the same words *after*
  applying the field rule, `panthor_fw.c:1290-1291`). Also: pass whole
  field masks (G13) — `wait_acks` on bit 0 alone can agree on a coincidence
  (requesting `CSG_STATE_TERMINATE` = 0 against an ack already holding 0
  looks acked before the firmware moves; the whole field under the same
  rule is what panthor waits for, `panthor_fw.c:1208-1214`).

## Already equal (do not re-litigate)

Checked against the v6.18 files; these `csf.rs` items match panthor and no
entry is needed: every layout and `offset_of` assert of `csf.rs:122-338`
against `panthor_fw.h:15-269` (sizes at `csf.rs:893-901`); the CS bit set
`csf.rs:341-355` against `panthor_fw.h:37-58` (`CS_REQ_MASK`,
`CS_EVT_MASK` equal); the CSG bits `csf.rs:253-266` against
`panthor_fw.h:138-154, 188`; `csg_ep_req` `csf.rs:268-270` against
`panthor_fw.h:165-170` (tiler `0xf_0000` = `GENMASK(19,16)`); `cs_config`
`csf.rs:357-359` against :61-62; `cs_features_*` `csf.rs:284-292` against
:26-30; `cs_exception_*` `csf.rs:362-367` against :113-114; the GLB bits
and masks `csf.rs:174-202` against :205-227, :234-235 (`GLB_EVT_MASK` is
`GENMASK(26,20)` = `0x07f0_0000`); the halt values `csf.rs:204-206`
against :255-259; `MAX_CSGS`/`MAX_CS_PER_CSG` `csf.rs:79, 81` against
:12-13 and `MIN_*` against `panthor_fw.c:202-203`; the doorbell arithmetic
`csf.rs:59-65` against `panthor_regs.h:205`; `ring_csg_doorbells`
`csf.rs:742-750` against `panthor_fw.c:1305-1311`; `toggle_reqs`/
`update_reqs` against `panthor_fw.h:424-433, 450-458`; the acked
expression against `panthor_fw.c:1229`. `csf.rs`'s documented polling
difference to `panthor_fw_wait_acks`'s busy-then-sleep wait
(`panthor_fw.c:1211-1227`, said at `csf.rs:715-717`) stands as written.

## What stays in `sched.rs` (policy, not interface)

Not CSF items; the follow-up does not move them, and the list is here so it
is not re-proposed. The ring's bounds 4 KiB to 64 KiB, a power of two
(`RING_MIN_SIZE`, `RING_MAX_SIZE`, `check_ring_size`, `sched.rs:316-318,
480-485`, from `panthor_sched.c:3335-3337`); the ring's and the sync
objects' maps in the *group's* VM, NOEXEC|UNCACHED
(`panthor_sched.c:3352-3358, 3522-3529`); the priorities `MAX_CSG_PRIO`
0xf (`panthor_sched.c:92`) and `CSF_MAX_QUEUE_PRIO` (`panthor_sched.c:392`,
`GENMASK(3, 0)`); the 5 s `JOB_TIMEOUT_US` (`panthor_sched.c:90` used at
:3323); the wait budgets `CSG_ACK_WAIT_US` 100 ms (`panthor_sched.c:1867`)
and ours' `CS_ACK_WAIT_US`; the group AS number `GROUP_AS = 1` (written to
`config` at `panthor_sched.c:1330`; the choice rule `ffz(alloc_mask |
BIT(0))` is `panthor_mmu.c:758` — the `sched.rs` doc cites it as :759, one
line off in this tree, worth a touch-up in the follow-up's pass over
`sched.rs`'s comments rather than an interface change); the exception
type values `EXCEPTION_CS_UNRECOVERABLE` 0x41 and `EXCEPTION_CS_INHERIT_FAULT`
0x4b (`panthor_device.h:321, 326`, used in `sched.c`'s fatal/fault paths at
`panthor_sched.c:1373-1378` and :1414-1429) — these name refusals, not
interface words; and the suspend buffers' *allocation sizes*, which come
from the CSG control block (`panthor_sched.c:3506-3520`) that `csf.rs`
already reads (`csf.rs:583-584, 623-624`) — `csf.rs` needs no new constant
there, only G15's pair.

## Sequence, and the paragraph for the design page

- This plan does not edit `csf.rs` (the plan's rule), and touches no code:
  the note is the deliverable. G1-G16 land in **one** follow-up commit to
  `csf.rs`, after this plan's sched work (bring-up, jobwords, submit) is on
  main: the follow-up moves each definition, deletes the local one from
  `sched.rs`, keeps every existing test green (the layout asserts at
  `csf.rs:893-901`, `sched.rs`'s `the_constants_are_panthors`), adds the
  G13 guards with tests, and re-runs `tools/check.sh --changed origin/main`.
  It may add the `CsfRefusal::AckMaskOutside` variant (G13) — a refusal is
  the `csf.rs` way, per `csf.rs:387-390`.
- The paragraph this note exists to feed: the "**Submit (Z4)**" paragraph
  of driver/docs/nanopc-t6-mali.md — lines 332-335 at the main tip this
  branch was cut against (the four lines starting "**Submit (Z4)**: a
  ring buffer (4-64 KiB, uncached) bound to CS 0 of CSG 0", ending at the
  `SYNC_UPDATE` interrupt sentence; the "**Hang**" paragraph right after
  it (:336) stays, and the milestone-list line "**Z4**" (:770) is not the
  target). A
  later commit replaces it with the draft already written in
  `driver/rust/src/gpu/mali/sched.rs`'s module doc, section "For
  driver/docs/nanopc-t6-mali.md: 'Submit (Z4)'" — plan #187 has since
  edited that page, so that commit should relocate the paragraph by its
  opening phrase, not by these line numbers. The page then cites this
  note for the csf.rs-side definitions it names.
