<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glInvalidateFramebuffer of a colour attachment on V3D 4.2 and PowerVR BXS-4-64: the load and the store it may skip

Design note of plan #368 (RC14, four draw targets), item `invalidate-doc`.
Today `glInvalidateFramebuffer` only validates (`invalidate_framebuffer`,
`src/gl/framebuffer.rs`) and `GlesDraw` (`src/backend.rs`) has no field for
it, so a back end never learns that `GL_COLOR_ATTACHMENT3` (or any other
attachment) may be dropped. Two host tests pin that as a defect:
`tests/es3_mrt4_invalidate_v3d.rs` (`an_invalidated_rt3_is_not_loaded_by_the_next_job`,
about line 887: the next job loads RT 3) and `tests/es3_mrt4_blit_pvr.rs`
(`an_invalidated_attachment3_is_not_stored_by_the_next_draw`, about line 750).
This note fixes what the discard must be, against Mesa 26.2.1, and the order
of the work. **No code comes with it; nothing was run, on the host or a
board.** It is a saving (fewer tile loads and stores, bandwidth), never a
correctness feature, so everything here is opt-in until a board run agrees.

Sources: Mesa 26.2.1, tag `mesa-26.2.1`. `third_party/mesa-26.2.1` is a
trimmed copy without the files cited below; they were read from a full tree
of the same tag in the dependency cache. Cited as `file:line` relative to the
tree root. Mesa is described in words, never copied.

## 1. The rule (what ES 3.0 allows, and the one thing that must hold)

ES 3.0 4.5.1: after `glInvalidateFramebuffer` the contents of the named
attachments are **undefined until written again**. Two consequences, the
second of which is the one an earlier draft of the PowerVR pin got wrong:

* **Load.** The next render pass that starts from the invalidated attachment
  need not load its old contents (they are undefined). Skipping the load is
  allowed, never required.
* **Store.** A render pass that **draws after the invalidate must still store
  the attachment**: the pixels it drew are defined and a later read-back,
  sample or blit reads them. The store may be skipped only when nothing is
  drawn (and no clear is done) into the attachment after the invalidate, that
  is, when the invalidate came after the last write of the pass. A design
  that drops the store of "the next render" loses the new pixels. The PowerVR
  pin at `tests/es3_mrt4_blit_pvr.rs` asks exactly for that and is wrong; it
  is rewritten, not un-ignored (section 6).

Mesa agrees on both (section 2). Not required: dropping anything for
attachments that are layers, levels, 3D slices or cube faces (Mesa does not
either, `src/mesa/main/fbobject.c:5649-5651`).

## 2. What Mesa 26.2.1 does

### 2.1 V3D (Gallium, `src/gallium/drivers/v3d`)

* The state tracker calls `pipe->invalidate_resource` for each named
  attachment of a complete framebuffer, but only for a simple 2D resource
  (depth 1, one layer, no mip chain): `src/mesa/main/fbobject.c:5638-5655`
  (check at `:5649-5651`, call at `:5653-5654`). The driver hook is installed
  at `src/gallium/drivers/v3d/v3d_context.c:460`.
* `v3d_invalidate_resource` (`v3d_context.c:116-141`) does two things. It
  marks the resource invalidated and forgets its initialised buffers
  (`:122-123`); and, if a job that writes it is open (`write_jobs` lookup,
  `:125-127`), it removes the buffer from that job's **store** mask, the
  depth and stencil bits for the z/s buffer (`:131-134`) and the colour bit
  of its slot (`:136-140`). It does not touch the job's load mask.
* The **load** side: when a job is created after a framebuffer-state change,
  `v3d_job.c:440-479` copies the "invalidated" flag of each attached colour
  resource into the job's `invalidated_load` mask (`:449-452`, field declared
  at `v3d_context.h:463-466`) and clears the resource flag. It applies only
  to the first job after the state change (`submitted_any_jobs_for_current_fbo`,
  set at `v3d_job.c:749`, reset at `v3dx_state.c:549`). The z/s version
  (`v3d_job.c:469-479`) acts on depth and stencil together only (comment
  there).
* `invalidated_load` is used when a draw fixes the job's masks:
  `v3d_update_job_tlb_load_store` (`v3dx_draw.c:1062-1111`) builds
  `no_load_mask` from the TLB clear, the draw clear and `invalidated_load`
  (`:1070-1071`); a colour buffer is added to the job's **load** mask only
  when it is not in `no_load_mask` (`:1105`), and added to the **store** mask
  when the blend state's colour mask is not zero (`:1107-1108`).
  **That is the "draw after the invalidate still stores" rule**: the draw
  sets the store bit again after `v3d_invalidate_resource` cleared it. A
  clear sets it as well (`job->store |= buffers`, `v3dx_draw.c:1792`).
* The render control list reads the masks: the loads are emitted for the
  colour slots whose bit is in `job->load` (`v3dx_rcl.c:196-212`), the stores
  for the slots whose bit is in `job->store` (`v3dx_rcl.c:274-275`, the z/s
  store at `:285-309`). A slot not named is simply not in the list.

### 2.2 PowerVR (Vulkan only: Mesa has no Gallium or GL driver for it)

Mesa's Imagination driver is Vulkan; it has no `glInvalidateFramebuffer`. The
equivalent is the attachment's per-pass `loadOp` and `storeOp`, decided at
begin-rendering, not afterwards, which is why a GL back end needs section 4's
deferred decision.

* **Load.** `VK_ATTACHMENT_LOAD_OP_DONT_CARE` (and `NONE`) put the colour in
  neither the load mask nor the clear mask of the render's background
  ("load op") program: `src/imagination/vulkan/pvr_arch_mrt.c:332-343`
  (`rt_clear_mask` at `:333`, `rt_load_mask` at `:336`, nothing for
  DONT_CARE at `:338-340`). An attachment counts towards the render's
  initialisation list (`color_init_count`) only for LOAD or CLEAR:
  `pvr_arch_cmd_buffer.c:4219-4222` in the dynamic-rendering setup;
  `pvr_arch_mrt.c:283-297` (`pvr_rendering_info_needs_load`) makes a render
  with no LOAD/CLEAR attachment need no load pass at all.
* **Store.** Only `VK_ATTACHMENT_STORE_OP_STORE` counts a surface into the
  render's end-of-tile list: `eot_surface_count++` at
  `pvr_arch_cmd_buffer.c:4224-4226`, the PBE state for it at
  `:4344-4350`; anything else sets `requires_frag_pr` (`:4227`) and has no
  PBE emit. The pass-based path does the same: `pvr_arch_hw_pass.c:2642`.
  A `DONT_CARE` target therefore has no PBE state in the render's end-of-tile
  program, and an empty list is a z-only render (`pvr_arch_cmd_buffer.c:1687`,
  list size tested at `:1508`).
* A target is stored or not for the whole render: Mesa has no "stored only if
  drawn after" because Vulkan states the op up front. For GL the same
  outcome is "no PBE emit for a target nothing wrote since the invalidate".

## 3. What the code at the tip does (where the design meets it)

V3D (`src/v3d/backend.rs`, C in `driver/src/v3d/`):

* Loads. `V3dJobClear` (`src/v3d/ffi.rs:616` `load_rt0`, `:644` `load_rt:
  [bool; 3]`, offset assert `:953`; C `driver/src/v3d/v3d_job.h:287` and
  `:326-328`) holds one load flag per RT, default true
  (`src/v3d/backend.rs:1022`, `:1038`), cleared by a clear of that RT
  (`backend.rs` about `:6935-6938`, mask from `clear_target_mask`). The C
  frame builder reads them at `driver/src/v3d/v3d_job.c:1181`, `:1321` (the
  4x "no raster buffer" refusal) and `:1365`. So the load skip is a one-line
  effect in the place that already clears `load_rt[n - 1]`.
* Stores. There is no per-RT store flag: `v3d_job.c:1255-1267` (RT 1..3, in RT
  order) stores every RT whose `rt[n - 1].address` is set. A store mask is a
  new field next to `load_rt` (this is the `store_rt[4]` of section 5; RT 0's
  store has its own conditions at `:1240-1245`).
* One V3D job spans several GL draws (`src/v3d/backend.rs` header, "job is
  ended and submitted when the target changes, on flush/finish ..."), as a
  Mesa job does, so Mesa's open-job handling (2.1) carries over exactly.

PowerVR (`src/pvr/backend.rs`, `driver/src/pvr_render.[ch]`):

* Loads. RT 0's old colour is reloaded by the frame's first object when the
  frame did not begin with a colour clear (`frame_loads_color`, `backend.rs`
  about `:1725`); attachments 1.. by `record_attachment_loads`
  (`backend.rs` about `:4511`): one full-screen quad per attachment with a
  target, skipping a GL_NONE slot. Skipping a load is `continue` there.
* Stores. The EOT PBE state pairs are built per kick (`emit[j * 2..]`,
  `backend.rs` about `:9762`); a slot with no target (`i != 0 && a == 0`)
  is configured in the output registers but gets no emit and so is not stored
  (comment at about `:9745-9751`), `slot_of[]` mapping the emit index back to
  the slot. **An invalidated, not re-written attachment is the same case**,
  which is why the PowerVR store-side change is small. Slot 0 always has an
  emit (a z-only frame's empty-tile state, `pbe_emit_words_z_only`), so
  dropping RT 0 is out of the first version.
* The EOT program is selected by the number of emits (`eot_n[n_rt]`,
  `driver/src/pvr_render.c:86`, `:271-283`, `:514-523`), so a dropped emit
  gives a three-emit program on a four-attachment frame; the output
  registers stay laid out for four (`mrt_layout`), only the emit list
  shrinks.
* Frame splitting (`backend.rs` about `:6640-6665`) and the partial-render
  scratch reload (`pr_load_*`) are not touched here.

GL core: `invalidate_framebuffer` validates and returns
(`src/gl/framebuffer.rs` about `:3791-3830`, its doc comment at `:3831` says
"nothing to discard ... a no-op"; its unit tests `:6790`, `:6811`).

## 4. The design

**The carried value.** `invalidated: u8`, a bitmask of the **colour
attachments 0-3** (bits 0-3), depth (bit 4) and stencil (bit 5) whose
contents are undefined. Bits 4 and 5 are reserved and not acted on in the
first version (Mesa acts only on depth and stencil together, `v3d_job.c:471-474`,
and the 7.1 store word and a PowerVR partial render are not designed). Mesa
refuses simple-2D-only (`fbobject.c:5649`): the GL core sets a bit only for
an attachment that is a whole 2D renderbuffer or level-0 single-layer
texture; layer, level, 3D slice and cube-face attachments set none.

**Two pieces, because load and store are decided at different times** (Mesa
too: `invalidated_load` at job creation, the store mask at the call):

1. *Load side: a field on the draw.* `GlesDraw::invalidated_before: u8`. The
   GL core keeps the pending mask per framebuffer object (Mesa keeps it on
   the resource and consumes it at the first job after a state change,
   `v3d_job.c:449-452`, `:450`); `glInvalidateFramebuffer` ORs bits in,
   the next draw or clear into that framebuffer copies it into `GlesDraw`
   and **clears** it once a render pass has used it (second pass loads
   again). A back end acts on it only when the draw **opens a new job or
   frame**; a draw joining a job that already has draws has loaded already
   and ignores it. A colour clear of that attachment makes the bit moot
   (cleared, nothing to load).
2. *Store side: a call.* `YakoglBackend::invalidate` (a function pointer, the
   `None` default meaning no discard, like the other optional entries; the
   argument is the same mask). The GL core calls it from
   `glInvalidateFramebuffer` immediately. The back end then:
   * no job or frame open on that framebuffer: nothing (the load side covers
     the next one);
   * a job open with draws: clear the stored flag of each named RT in the
     job (`store_rt[n]`, section 5);
   * **any later draw into that RT (draw buffer enabled and colour mask not
     zero, the test Mesa makes at `v3dx_draw.c:1107`) or clear of it sets the
     flag again** (`v3dx_draw.c:1108`, `:1792`). A pass that draws after the
     invalidate therefore **stores**; only a trailing invalidate, with no
     write after it, drops the store.

An invalidate that reaches a back end whose hook is `None`, or with the
opt-in off, behaves as today.

**Opt-in.** The load and store skips are off until a board run checks them,
as the plan keeps `DRAW_TARGETS` at 3. Shape and name follow the draw-target
knob (`docs/notes/mrt4-v3d-pvr.md` section 3, decided in item `knob`): a
build or start-up switch, default off, listed in `tools/check.sh` in the same
commit. This note does not name the Cargo feature.

## 5. By GPU

**V3D 4.2** (item order 3): `V3dJobClear` gets `store_rt: [bool; 3]` next to
`load_rt` (RT 1..3 only; default true; the struct, its offset asserts at
`src/v3d/ffi.rs:953` and the C struct `driver/src/v3d/v3d_job.h:326-328` move
together, and the generated bytes the gate compares if any). The Rust
backend (`src/v3d/backend.rs`): on a draw that opens a job, clear
`load_rt[n - 1]` for each bit of `invalidated_before` (the existing clear
site, about `:6938`), without touching `rgba8_rt`; in `invalidate`, clear
`store_rt[n - 1]` of an open job; on any draw or clear that writes RT n, set
it. The C builder (`v3d_job.c:1266`) stores RT n only `if address &&
store_rt[n - 1]`, as `v3dx_rcl.c:274` does. Two checks stay as they are: a 4x
job that would load an RT with no raster buffer is refused
(`v3d_job.c:1321`); an RT whose load was skipped by an invalidate is not
loaded, so it is not refused. RT 0 and the 7.1 store word (`Store TMU`
variant) are not in the first version.

**PowerVR BXS-4-64** (item order 4): `record_attachment_loads` skips slot
`n` when its bit is in `invalidated_before` and the draw opens the frame,
`frame_loads_color` stays as is for RT 0. The emit builder treats an
invalidated, not re-written slot 1..3 as a slot with no target (the existing
`continue`), and `invalidate` sets a per-slot "stored" flag false on an open
frame; any later write sets it true. Nothing is dropped for slot 0, for a
frame in the partial-render path (the scratch reload) or for a split frame:
they keep the store. The EOT selection needs no change: it counts emits.

## 6. Files and owners, order, tests

The change reaches files that other plans hold; it is sequenced after them
and rebased onto each, not run in parallel.

| File | What | Held by |
| --- | --- | --- |
| `src/backend.rs` | `GlesDraw::invalidated_before`, `YakoglBackend::invalidate`; grep `tests/` and `src/` for every `YakoglBackend {` literal (CLAUDE.md, the 2026-10-14 case) | #354 |
| `src/gl/framebuffer.rs` | per-framebuffer pending mask, the call, doc comment, unit tests `:6790` and `:6811` | other plans (#364 lists it) |
| `src/gl/draw.rs` | copies the mask into `GlesDraw` and clears it | #361 (target keys) |
| `src/v3d/backend.rs`, `ffi.rs`, `driver/src/v3d/v3d_job.[ch]` | `store_rt`, load skip, `invalidate` | #359 |
| `src/pvr/backend.rs`, `driver/src/pvr_render.c` | load skip, emit drop, `invalidate` | #350 and #325 |
| `tests/es3_mrt4_invalidate_v3d.rs`, `tests/es3_mrt4_blit_pvr.rs` | the pins | this change, after the above merge |

Order of items:

1. `src/backend.rs`: the field (default 0) and the hook (default `None`) with
   every literal updated. Behaviour unchanged. After #354.
2. GL core: the per-framebuffer mask, set by `glInvalidateFramebuffer` for
   whole level-0 2D attachments only, consumed by the next draw or clear,
   and the hook call. Behaviour unchanged while no back end implements it.
3. V3D: load skip, `store_rt`, `invalidate`. After #359.
4. PowerVR: load skip, emit drop, `invalidate`. After #350 and #325.
5. Tests (below), then the opt-in switch in `tools/check.sh` with them.
6. Board: after the draw-target board runs, one gl_test phase (below).

**Test edits.**

* `tests/es3_mrt4_invalidate_v3d.rs`: un-ignore
  `an_invalidated_rt3_is_not_loaded_by_the_next_job` (the load pin is sound:
  `load_rt[2]` false, RT 0..2 kept) with the switch on. Add: a trailing
  invalidate of attachment 3 clears `store_rt[2]` of the open job; an
  invalidate then a draw into attachment 3 keeps `store_rt[2]` (the rule of
  section 1); the same with a colour clear after it; the same with the
  colour mask zero or attachment 3 not in the draw-buffer list (store dropped
  stays dropped); the second job after the invalidate loads again; a
  layer or mip-level attachment 3 changes nothing; the other attachments'
  flags are untouched; with the switch off every case equals today.
* `tests/es3_mrt4_blit_pvr.rs`: **rewrite**
  `an_invalidated_attachment3_is_not_stored_by_the_next_draw`; its expectation
  (target 3 not stored by the draw after the invalidate) is wrong. Replace
  it with: the load quad of attachment 3 is skipped for the first frame after
  the invalidate; a draw into attachment 3 after the invalidate still stores
  it (`targets[3].0 != 0`, and the drawn colour reads back); a trailing
  invalidate with no later write drops the emit (target 3 unstored, the three
  others unchanged); attachment 0 is never dropped.
* `src/gl/framebuffer.rs` unit tests (`:6790`, `:6811`): keep the validation
  rows, rename "does nothing"; add the mask rows (which attachments set a
  bit, consumed once). Owned by the #364 plan: edit after it merges.
* Not touched: `docs/es3-cts-failures.md` (a later docs item).

**Board verdict (item 6, optional, after the draw-target flip).** Pi 4 and
AM62P: in the gl_test mrt4 case, after drawing four targets and
`glInvalidateFramebuffer(GL_COLOR_ATTACHMENT3)`, draw again into the four and
read all four back: attachments 0-2 and the new attachment 3 pixels must
match (a dropped store would show here). A trailing invalidate's dropped
store is not observable by a read-back (undefined contents); it is a host
test only. The line is `gl_test:   mrt4 invalidate fail mask (...)  = 0
want 0`, the shape of the case's existing lines.

## 7. Not designed

* The V3D 7.1 store word and per-buffer depth and stencil discard (Mesa does
  depth and stencil together only, `v3d_job.c:471-474`).
* RT 0 and the default framebuffer's `GL_COLOR`.
* The PowerVR partial-render scratch reload and split frames.
* `glInvalidateSubFramebuffer` (a sub-rectangle is never a whole-surface
  discard; it keeps validating and discarding nothing).
* A texture invalidated through one framebuffer and used through another
  (Mesa keeps the flag on the resource; this design keeps it per framebuffer,
  the conservative reading: no skip).
