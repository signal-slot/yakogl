<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 occlusion queries: design for PowerVR, V3D 4.2 and GC7000UL

Design for item `occlusion-query` of plan #3. Written 2026-10-08 by item
`occlusion-design`. Docs only: nothing was built or run for this file, and
nothing here has been measured on a board. Every hardware statement is
either a Mesa citation or marked **[unverified]**.

Scope: `GL_ANY_SAMPLES_PASSED` and `GL_ANY_SAMPLES_PASSED_CONSERVATIVE`
(`glBeginQuery`, `glEndQuery`, `glGetQueryiv`, `glGetQueryObjectuiv`,
`glIsQuery`, `glDeleteQueries`). `GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN`
shares the query objects but belongs to `tf-api`/`tf-capture`
(`docs/es3-transform-feedback.md`).

dEQP-GLES3 `functional.occlusion_query.*` fails all 62 cases on the AM62P
(`tools/cts/RESULTS.md:1476`); `negative_api` loses 4 more cases to the same
stub (`RESULTS.md:873`).

## 0. Sources and citation conventions

| tag | tree |
|---|---|
| `mesa:` | `$CCFARM_DEPS_DIR/mesa-26.2.1-caps/src/` (Mesa 26.2.1, sparse: it holds the files cited here) |
| `tp:` | `third_party/mesa-26.2.1/src/` in the repository (headers, `csbgen`, `broadcom/cle`; no gallium drivers, no `pvr_query.c`) |
| `cts:` | `$CCFARM_DEPS_DIR/VK-GL-CTS/modules/gles3/functional/` |
| none | this repository |

`$CCFARM_DEPS_DIR/mesa-26.2.1-src` is an **empty directory** on node `nuc`
(2026-10-08). The gallium and `imagination/vulkan` files are only in
`mesa-26.2.1-caps`. The etnaviv register offsets
(`VIVS_GL_OCCLUSION_QUERY_ADDR/CONTROL`) are in the vendored rnndb header
`driver/src/vivante/hw/state.xml.h`: see 3.3.

## 1. What the tests need

`es3fOcclusionQueryTests.cpp` (all 62 cases are one class, `OcclusionQueryCase`):

1. `glClear` colour+depth+stencil (`cts:216-219`); optional occluders
   (depth-writing draws `:254-259`, a depth clear under a scissor box
   `:276-281`, stencil-writing draws `:297-302`, a stencil clear under a
   scissor `:319-325`), **before** the query.
2. `glGenQueries`, `glBeginQuery(mode)` (`:351-352`).
3. Target draws with `GL_DEPTH_TEST` and `GL_STENCIL_TEST` (EQUAL 0) on and
   optionally `GL_SCISSOR_TEST` (`:354-372`). Client-side vertex arrays.
4. `glEndQuery`, **`glFinish()`** (`:374-375`), then
   `GL_QUERY_RESULT_AVAILABLE` must be true (`:383-386`), then
   `GL_QUERY_RESULT` read as a boolean (`:392-393`).
5. `glReadPixels` and compare: for `GL_ANY_SAMPLES_PASSED` the answer must
   **equal** "some pixel is red"; for the conservative target a false
   positive is allowed (`:429-436`).

So the result must be exact for `ANY_SAMPLES_PASSED`: the counter has to see
the depth, stencil and scissor tests, it must not see the clears, and it
must see the result of draws that were recorded before `glEnd`/`glFinish`.
`es3fNegativeFragmentApiTests.cpp:305-380` and
`es3fNegativeStateApiTests.cpp:1015-1050, 1240-1262` pin the API errors
(section 5). `es3fLifetimeTests.cpp:195-215` calls `glBeginQuery(ANY_SAMPLES_PASSED)`
as its "bind" and `glEndQuery` as "unbind".

## 2. Where yakogl is today

* `src/gl/es3.rs:233-329`: names are real (`glGenQueries` pool of
  `MAX_QUERIES = 32` per context, `:45`, `QUERIES` `:107`, `Es3Name {used,
  bound}` `:53-57`); `glBeginQuery` sets `bound` and raises
  `GL_INVALID_OPERATION` (`:271-286`), `glEndQuery` always
  `GL_INVALID_OPERATION` (`:289-297`), `glGetQueryiv` answers 0
  (`:300-317`), `glGetQueryObjectuiv` always `GL_INVALID_OPERATION`
  (`:320-328`). The module comment (`:25-30`) states the rule that a call
  needing an unmodelled feature errors loudly; this item removes the
  occlusion half of that rule.
* Host tests that pin the stub and must change: `src/gl/es3.rs:1348-1418`
  (`a_query_name_is_an_object_only_after_begin_query`,
  `begin_query_checks_the_target_before_the_name`,
  `end_query_and_the_object_query_refuse`,
  `get_queryiv_answers_that_no_query_is_active`).
* Backends: no counter anywhere. `grep -i occlusion` over `src/` finds only
  the constants (`src/gl/consts.rs:597-598, 1432-1434`). The V3D C job
  builder already emits the OQ packet with a **zero address** (section 3.1).
* Nothing in `GL_EXTENSIONS` mentions queries: `ALL_EXTENSION_NAMES`
  (`src/gl/state.rs:32-...`) has no `GL_EXT_occlusion_query_boolean`, and
  there are no `glBeginQueryEXT`/`glEndQueryEXT` exports (`src/` and
  `capi/` hold none; only the Khronos header `include/GLES2/gl2ext.h:1314,
  1793-1797` and the unused constant `GL_EXT_occlusion_query_boolean`
  `src/gl/consts.rs:1432`).

## 3. Per-GPU design

All three share the same GL-core shape (section 4): the GL core tells the
back end, **per draw**, which query counts it (`GlesDraw::occlusion_query`,
0 = none); the back end owns the counter storage; the GL core learns
completion through the existing kick-id machinery. The per-GPU part is what
a counted draw emits and how the counter is read.

### 3.1 V3D 4.2 (Pi 4): the BCL packet `OCCLUSION_QUERY_COUNTER`

**Mesa.**

* Hardware model, `mesa:gallium/drivers/v3d/v3d_query_pipe.c:27-30`: "native
  support for occlusion queries, with the query result being loaded and
  stored by the TLB unit... jobs that need to be tracking queries are
  bracketed by the start and end of counting, even across FBO transitions".
* Begin: allocate a 4096-byte BO, write 0 to its first dword, make it
  `current_oq`, set `V3D_DIRTY_OQ` (`v3d_query_pipe.c:91-101`; flag
  `v3d_context.h:94`, field `:711`, `active_queries` `:679`). The same BO is
  kept for the whole query, however many jobs it spans; no per-job summing
  (`:231-233` has "XXX: Sum up per-core values"). So the TLB **adds** the
  job's count to the dword at the address at the end of each job
  [inference from that reuse; V3D 4.2 has one core].
* End: `current_oq = NULL`, `V3D_DIRTY_OQ` (`:156-161`).
* The packet: `tp:broadcom/cle/v3d_packet.xml:822-824` (code 92, one field
  `address`, 32 bits); packed form 5 bytes, `V3D42_OCCLUSION_QUERY_COUNTER_pack`
  at `driver/src/v3d/generated/v3d_packet_v42_pack.h:2753-2790`.
* Emission: in the **binner** list, as state, when `V3D_DIRTY_OQ`
  (`mesa:gallium/drivers/v3d/v3dx_emit.c:720-726`; the address is written
  only when `active_queries && current_oq`, otherwise a zero-address packet
  is the "disable"). It is also emitted at job start with a zero address
  "to disable any leftover OQ state from another job"
  (`v3dx_draw.c:107-108`). `v3d_job.c:436` sets `dirty = ~0` when switching
  jobs, so every new job re-emits the packet. There is no RCL packet: the
  counter is a binner-state entry that rides in the tile lists.
* Pipeline interaction: with discard and an active query Mesa turns early-Z
  off, `v3d_program.c:628-631` (`!(v3d->active_queries && v3d->current_oq)`).
  yakogl already runs with early Z off in the RCL
  (`driver/src/v3d/v3d_job.c:1029-1031`), so nothing to add.
* Clears/blits while a query is active: `v3d_set_active_query_state(false)`
  (`v3d_query.c:132-139`) clears the packet around internal draws.
* Result: `v3d_flush_jobs_using_bo` submits any open job that references the
  BO (`v3d_job.c:157-167`), `v3d_bo_wait` (`v3d_query_pipe.c:204-222`), read
  the first dword (`:232-233`); `OCCLUSION_COUNTER` = u64 of it,
  `OCCLUSION_PREDICATE[_CONSERVATIVE]` = `!= 0` (`:243-249`).
  `PIPE_QUERY_OCCLUSION_PREDICATE_CONSERVATIVE` is the same code path.

**yakogl today.** The C job builder already writes the packet, always
zero: `driver/src/v3d/v3d_job.c:167-168` (job start, right before
`START_TILE_BINNING`), `:556-566` (the per-draw "fixed" group, with
`TRANSFORM_FEEDBACK_SPECS` and `SAMPLE_STATE`, packet-skip group
`V3D_PACK_FIXED`, `v3d_job.h:278`), `driver/src/v3d/v3d_frame.c:100-101,
503-504` (the clear-quad frame). The comment at `v3d_job.c:557` even cites
`v3dx_emit.c:720-726`.

A draw becomes a job like this: `v3d_draw` (`src/v3d/backend.rs:4199`) ->
`draw_call` (`:4247`) -> `state::job_state(d)` (`src/v3d/state.rs:396`,
the C `struct v3d_job_state`, `driver/src/v3d/v3d_job.h:102`, mirrored by
`V3dJobState`, `src/v3d/ffi.rs:271`) -> `draw_in_job` (`:3881`, opens a job
with `begin_job` `:1529` if none) -> `record` (`:2418`) ->
`record_arrays_and_draw` -> C `v3d_job_draw` (`v3d_job.c:783`) ->
`emit_state` (`:459`). The job is closed by `flush_job` (`:1659`: builds
the RCL) and run by `submit_job` (`:2000`: `cache_flush` of the lists,
`V3dKick`, `src/v3d/kick.rs:20-34`). One job = one kick; `g.kicks`
increments per submit; `v3d_kick_id()` is "the kick being recorded"
(`:4527`), `v3d_kick_pending` (`:4540`), `v3d_wait_kick` (`:4452`).

**Changes.**

| where | change |
|---|---|
| back end storage | `V3dState` gets an `oq` arena: one 4 KiB GPU buffer from `osal::gpu_alloc` in `v3d_init` (`:3962`), mapped. Handle `h` (1..=32) owns the dword at `h * 64` (a 64-byte stride: the CPU zeroes and reads one line while the GPU writes another's). |
| `driver/src/v3d/v3d_job.h:102` `struct v3d_job_state` + `src/v3d/ffi.rs:271` `V3dJobState` | new field `oq_addr: u32` (0 = none). `static_assert` the layout the way the other mirrors do. |
| `src/v3d/state.rs:396` `job_state` | set `oq_addr` from `GlesDraw::occlusion_query` (`arena.gpu + h*64`, 0 for none and for every `clear_quad`, `state::clear_quad_draw` `:148`). |
| `driver/src/v3d/v3d_job.c:556-566` | `PACK(V3D_PACK_FIXED, 3, mask, st->oq_addr)`: **the address joins the key**, or the packet-skip cache would not re-emit it when it changes. `OCCLUSION_QUERY_COUNTER` gets `oq.address = cl_address(...)` when non-zero. The job-start packet (`:168`) stays zero. |
| BCL relocation | the C builder emits GPU addresses directly (`cl_address`); no BO list exists. The arena is allocated once and lives as long as the context, so no per-job pinning. |
| `src/v3d/backend.rs` | `v3d_query_begin(h)`: if the job in flight/open is using `h` (it is not: Begin zeroes a fresh handle) write 0 to the dword, `cache_flush` the line. `v3d_query_result(h)`: `osal::cache_invd` (`src/osal/mod.rs:454`) then read the dword. Both are new `YakoglBackend` entries. |
| a counted draw into an open job | nothing special; later jobs of the same query re-emit the packet via `PACK` (the first draw of every job writes every group: `pack_group` "not written yet in this job"). |

**Availability.** `GL_QUERY_RESULT_AVAILABLE` = `!kick_pending(end_kick)`
(section 4.4). The count is added by the TLB at the end of the job, which
the OSAL has waited for when `v3d_wait_kick` returns.

**Risks.** (a) The accumulate-into-memory behaviour is inferred from Mesa;
the host cannot check it (`tests/v3d_sim_mesa.rs` simulates QPU code, not the
tile lists). The first board test is two jobs on one query summing.
(b) MSAA: a 4x job counts samples, so a 4x target's count can be up to 4x a
pixel count; fine for the boolean query. (c) The V3D tile lister counts
samples that **pass** the TLB depth/stencil test; scissor is applied by the
clip window, as in Mesa; both are in the dEQP cases.

### 3.2 PowerVR BXS-4-64 (AM62P): ISP visibility test

**Mesa (Vulkan `pvr`; there is no gallium driver for this GPU).**

* Per-object enable in the PPP state: `TA_STATE_ISPCTL` bit 15 `vistest`,
  bit 14 `visbool`, bits 0..13 `visreg`
  (`tp:imagination/csbgen/rogue/ppp.xml:131-143`, `vistest` at `:140`).
  `pvr_setup_isp_faces_and_control` sets `ispctl.vistest = true; ispctl.visreg
  = state.vis_reg` for every draw while `vis_test_enabled` and not
  rasterizer-discard (`mesa:imagination/vulkan/pvr_arch_cmd_buffer.c:6878-6881`);
  the ISP control is re-emitted when `state->dirty.vis_test` changes
  (`:7942, :7983`, cleared `:8428`). `visbool` is never set by Mesa.
* Begin/End: `pvr_arch_cmd_query.c:164-218` (state `vis_test_enabled`,
  `vis_reg = query`, `dirty.vis_test`; the query index is also appended to
  `query_indices`, one entry per view), `:220-231` (End just clears the
  flag). One query at a time (`:176-177`, "can't be nested"). A query that
  spans two pools ends the render and starts another that **loads** the
  colour back (`:179-204`).
* Per sub-command: `sub_cmd->job.get_vis_results = state->vis_test_enabled`
  (`pvr_arch_cmd_buffer.c:8297`) and `:2747-2748` (a render opened during a
  query inherits the pool).
* Kick: the frag stream carries `CR_ISP_OCLQRY_BASE` = the pool's result
  buffer address, or invalid when no pool
  (`pvr_arch_job_render.c:942-957`; register `tp:...rogue/cr.xml:589-591`:
  address bits 4..39, **16-byte aligned**, `ROGUE_OQUERY_ALIGN` =
  16, `tp:imagination/include/hwdef/rogue_hw_defs.h:51`); for
  multicore parts `isp_oclqry_stride` (`pvr_arch_job_render.c:1243-1246`
  is a `finishme` for `core_count > 1`; fw struct
  `winsys/pvrsrvkm/fw-api/pvr_rogue_fwif.h:276, :287`). The submit flag
  `GETVISRESULTS` = `0x20` ("the firmware collects the visibility
  results", `pvr_rogue_fwif.h:53-56`; set in
  `winsys/pvrsrvkm/pvr_arch_srv_job_render.c:750-751`, and as
  `DRM_PVR_SUBMIT_JOB_FRAG_CMD_GET_VIS_RESULTS` in
  `winsys/powervr/pvr_drm_job_render.c:454-455`).
* Storage: the result buffer comes from a dedicated **vis-test heap**
  (`pvr_query.c:82-91`, `pvr_arch_device.c:62, 771-774`: 128 KiB suballocator on
  `heaps.vis_test_heap`; `winsys/powervr/pvr_drm.c:384, 519`;
  `winsys/pvrsrvkm/pvr_srv.c:219-326`). One dword per query index, per core:
  `result_stride = ALIGN(queryCount*4, 16)`, `alloc = stride * core_count`
  (`pvr_query.c:56, 78, 85`). Availability is a separate array in the
  general heap (`:95-99`, read by `pvr_query_is_available` `:138-144`) which
  Mesa writes with a compute program after the render
  (`pvr_arch_cmd_buffer.c:2540-2570`).
* Precision: Mesa advertises `occlusionQueryPrecise = false`
  (`pvr_physical_device.c:256`): the hardware gives a non-exact count,
  which is exactly enough for the boolean targets.

**yakogl today.** A draw -> `pvr_backend_draw` (`src/pvr/backend.rs:3891`)
-> `draw_one` (`:3984`) -> `draw_chunked` -> `draw_part` -> `record_draw`
(`:4568`), which builds **one PPP state block per draw** through
`pvr_ppp_build_block` (`:5227-5262, :5547-5553`, C
`driver/src/pvr_vdm_ppp_gen.c`, whose `TA_STATE_ISPCTL` is packed at
`:311-321` with no vistest). Draws accumulate in one frame; `frame_ends_here`
(`:3856`) decides when a different target, a clear, etc. ends it; `flush`
(`:5779`) fills the kick (`:6025-6149`, `isp_ctl` `:6139`) and submits it
(`:6255`, `PvrRenderKick` of 208 bytes,
`driver/src/pvr_render.h:96-131`, asserted at `driver/src/pvr_render.c:700-707`
and `driver/rust/src/gpu/pvr/render.rs:138-139`). The firmware command is
built in `driver/rust/src/gpu/pvr/render.rs:1270-1306`: dwords 18-19 scissor
base, 20-21 dbias base, **22-23 are the OCLQRY base and are never written**,
24+ zls; dword 76 is `cmd_flags` (copied from `frag.cmd_flags`). Kick ids:
`pvr_backend_kick_id` = last fence + 1 (`:6436`), `pvr_backend_kick_pending`
= `id > completed_fence` (`:6588`). The vis-test heap does not exist in the
driver: `enum pvr_render_heap` has GENERAL, PDS, USC only
(`driver/src/pvr_render.h:48-52`).

**Changes.**

| where | change |
|---|---|
| `driver/src/pvr_vdm_ppp_gen.{h,c}` `struct pvr_ppp_state` + `src/pvr/ffi.rs:154` `PvrPppState` + `src/pvr/ffi_layout_check.c` | `vis_test: i32`, `vis_reg: u32` (< 16384, 14 bits). Pack `c.vistest`/`c.visreg` in the `TA_STATE_ISPCTL` at `:311-321`. Extend the C generator's own test and the layout assertions. |
| `src/pvr/backend.rs` `record_draw` (`:5227`) | `st.vis_test = d.occlusion_query != 0; st.vis_reg = slot_of(d.occlusion_query, this kick)`. Clear/load quads (`record_frame_quad` `:2677`, `record_clear_quad` `:2814`) never set it. |
| `PvrRenderFragRegs` (C `driver/src/pvr_render.h:96`, Rust `src/pvr/ffi.rs`, `src/abi_layout.rs`, `src/abi_layout_check.c`, `driver/rust/src/gpu/pvr/render.rs`, `tools/hostharness/rec.c:736`) | new `isp_oclqry_base: u64`, appended **after** `cmd_flags` (offset 208; struct size 216) so no existing offset moves; the `BUILD_ASSERT`s are updated. `render.rs:1284-1291`: `lohi(&mut dk, 22, f.isp_oclqry_base)`. |
| `src/pvr/backend.rs` `flush` (`:6139` region) | `kick.frag.isp_oclqry_base = vis.gpu` and `cmd_flags |= FRAG_FLAGS_GETVISRESULTS (0x20)` when the frame has any counted draw. |
| heap | **Open point.** Mesa allocates from a dedicated vis-test heap. Whether the ISP/firmware accept a base in the general heap is unknown **[unverified]**. First try: a 4 KiB block from `PVR_RENDER_HEAP_GENERAL` (16-byte aligned, zeroed by the CPU before the kick). If the firmware ignores or faults, add `PVR_RENDER_HEAP_VIS_TEST` to the driver (the DRM/KM heap exists in Mesa's winsys: `pvr_srv.c:219-326`). |
| result slots | see "accumulation" below. |

**Accumulation: one visreg per kick (decision).** Mesa uses one `visreg`
for the whole query, also across the extra renders it creates
(`pvr_arch_cmd_query.c:181-204`), and never sums on the CPU, so the
firmware/ISP either adds into the dword or Mesa relies on it. I cannot
tell which **[unverified]**. The design does not depend on it: every
counted kick gets **its own** 4-byte register from a pool
(`vis_reg` = next free index, zeroed by the CPU before submit), and the
back end remembers for each query handle the list of
`(kick_id, vis_reg)`. `query_result` sums those dwords on the CPU after the
kicks completed. A query that outlives 16 kicks folds the completed ones
into a running sum and recycles their registers (it waits for the oldest
kick, the same wait the frame-set reuse does, `wait_fence` `:1874`).

**TBDR ordering hazard (the main PowerVR risk).** The ISP resolves
hidden surfaces per tile and the back end deliberately makes depth-writing
opaque draws `PASSTYPE_OPAQUE` so that "front geometry submitted before back
geometry in the same kick still wins" (`src/pvr/backend.rs:5240-5260`). The
visibility test counts what survives the ISP's depth test, so:

1. An occluder **recorded after** `glEndQuery` in the same kick would hide
   a target recorded inside the query: the count would be 0 although GL
   says the target was visible at the time. Rule: **`glEndQuery` ends the
   frame** (`frame_ends_here` gains "the query that was counting has
   ended", implemented as the GL core calling `flush()` at `glEndQuery`, so
   no backend rule is needed). Occluders before `glBeginQuery` stay in the
   kick and correctly hide targets. The cost is one extra kick per query.
2. Inside the query, objects must be tested in submission order: counted
   draws are given `PVR_PPP_PASSTYPE_TRANSLUCENT` (order preserving,
   `:5247-5260`) instead of OPAQUE. This is a conservative choice made from
   the comments in the code, not from ISP documentation **[unverified]**:
   if board results show OPAQUE counted draws are exact, the override can
   go.
3. `ANY_SAMPLES_PASSED_CONSERVATIVE` is allowed false positives
   (`cts:429-436`), so if (1)/(2) turn out wrong on some case, the
   conservative target can be answered with "true when the count is
   nonzero **or** the draw was refused"; the exact target cannot.

**Availability.** Same as V3D: the result is final when the **last** kick of
the query is complete. Mesa's availability compute program
(`pvr_arch_cmd_buffer.c:2540-2570`, `pvr_arch_query_compute.c`) is not
needed: the CPU reads after `completed_fence >= kick`.

**Effort driver.** The code is small, but it widens the C/Rust kick ABI in
seven places, depends on firmware behaviour nobody has observed
(`GETVISRESULTS`, the heap, accumulation, `isp_oclqry_stride` for
`core_count > 1`; the AM62P BXS-4-64 is single-core: skip the stride) and
has the TBDR ordering hazard above.

### 3.3 GC7000UL (i.MX8MP): PE sample counter, `GL_OCCLUSION_QUERY_ADDR/CONTROL`

**Mesa (etnaviv gallium).**

* Supported when `HALTI0` (`mesa:gallium/drivers/etnaviv/etnaviv_screen.c:308-311`:
  `occlusion_query`, `conditional_render`, `conditional_render_inverted`).
  The GC7000UL is HALTI5. (The `OCCLUSION_SAMPLE_COUNTER` bit in
  `mesa:etnaviv/hwdb/nxp/gc_feature_database.h:595` is a different flag,
  0 in every NXP entry; Mesa does not look at it.)
* `etnaviv_query_acc_occlusion.c`: sample provider for
  `OCCLUSION_COUNTER`, `_PREDICATE`, `_PREDICATE_CONSERVATIVE` (`:48-61`).
  Storage: a 4 KiB buffer, **64-bit** per sample, `MAX_OQ_SAMPLES` 511
  (`:39`; `etnaviv_query_acc.c:62-80` allocates `0x1000` and memsets 0).
  * **resume** (`:69-87`): `etna_set_state_reloc(stream,
    VIVS_GL_OCCLUSION_QUERY_ADDR, {bo, offset = samples * 8})`.
  * **suspend** (`:89-96`): `etna_set_state(stream,
    VIVS_GL_OCCLUSION_QUERY_CONTROL, 0x1DF5E76)` ("the value used by the
    blob - but any random value will work", `:92`) and `samples++`. The
    write to CONTROL makes the PE store its running count at the current
    ADDR.
  * result (`:98-114`): sum of the first `samples` 64-bit slots;
    predicate = `sum != 0`.
* Lifecycle: begin = new zeroed buffer, `samples = 0`, **resume**, put on
  `active_acc_queries` (`etnaviv_query_acc.c:82-97`); end = **suspend**,
  remove (`:99-109`). Every **flush** suspends all active queries before
  the stream is submitted and resumes them after, i.e. **in the next
  stream, with a new slot** (`etnaviv_context.c:643-644` and `:677-678`);
  `set_active_query_state` does the same around internal draws
  (`etnaviv_query.c:134-146`).
* Result: flush if the buffer has a pending write (`etna_acc_get_query_result`
  `etnaviv_query_acc.c:122-130`, quoting the GL rule "querying the state
  forces that occlusion query to complete within a finite amount of time"),
  `etna_bo_cpu_prep(READ[|NOSYNC])` (`:132-138`), sum.

**yakogl today.** A draw is `draw_one` (`src/vivante/backend.rs:2154`) ->
`record` (`:2451`: clear quad first when the draw carries a clear, then
the draw) -> `emit_draw` (`:1609`) into the `GcCmd` of the open `GcFrame`
(`src/vivante/state.rs:139-154`). A frame is one command stream and one
kick: `frame_open` (`:1682`, runs `emit_context_init` `:1645`),
`frame_submit` (`:1733`, `GcKick`, `src/vivante/kick.rs`); `gc_flush`
`:2531`, `gc_finish` `:2546`, `gc_kick_id` `:2590` (the frame being
recorded), `gc_kick_pending` `:2596`, `completed_fence` `:1850`. A draw that
does not fit is taken back (`Mark::take/restore` `:2404-2440`) and recorded
into the next frame. `GcCmd::state(reg, value)` is LOAD_STATE
(`src/vivante/cmd.rs:75`), `stall` `:86`. `src/vivante/regs.rs` has **no**
occlusion register: it ends the GL block at `GC_GL_PIPE_SELECT 0x3800 ..
GC_GL_SEMAPHORE_TOKEN 0x3808, GC_GL_FLUSH_CACHE 0x380c,
GC_GL_MULTI_SAMPLE_CONFIG 0x3818, GC_GL_VARYING_COMPONENT_USE0/1/2
0x3828/0x382c/0x3838` (`:603-642`).

**Registers.** The rnndb header vendored in this repository defines
`VIVS_GL_OCCLUSION_QUERY_ADDR` = `0x00003824`
(`driver/src/vivante/hw/state.xml.h:341`) and
`VIVS_GL_OCCLUSION_QUERY_CONTROL` = `0x00003830` (`:395`). Neither is
defined in `src/vivante/regs.rs`, whose GL block holds
0x3800/0x380c/0x3818/0x3828/0x382c/0x3838/0x3854. The `0x1DF5E76` control
value is quoted from `etnaviv_query_acc_occlusion.c:93`.

**Changes.**

| where | change |
|---|---|
| `src/vivante/regs.rs` | `GC_GL_OCCLUSION_QUERY_CONTROL`, `GC_GL_OCCLUSION_QUERY_ADDR` and `GC_OQ_CONTROL_VALUE = 0x1DF5E76` (`ADDR` 0x3824, `CONTROL` 0x3830) with the `state.xml.h:341/:395` citation. |
| `src/vivante/state.rs` | `GcState` gets `oq: OsalGpuMem` (one 16 KiB buffer: 2048 slots of 8 bytes, allocated at `gc_init` `:1905`; per handle a CPU-side `slots: Vec`-free array of `(first, count)`: the handle's slots are the contiguous range `[first, first+count)` of the **current** query; one query is active at a time, so slots are handed out in order) and per-frame `oq_on: u32` (the handle whose slot is currently armed, 0 = off). |
| `emit_draw` `:1609` | before `emit_flushes` (`:1597`, which ends with the RA->PE stall): `oq_arm(f, d.gl.occlusion_query)`. If it differs from `oq_on`: if `oq_on != 0` **suspend** (`c.stall(GC_SYNC_RA, GC_SYNC_PE)` then `c.state(CONTROL, 0x1DF5E76)`, slot count++); if the new handle is non-zero **resume**: `c.state(ADDR, oq.gpu + slot*8)`. The stall before the CONTROL write [unverified: Mesa has none, but its writes are in the same stream order; a PE that still holds fragments of the previous draw would store a short count; the blob's pattern is unknown]. |
| `emit_clear` `:1799` (the clear quad) and clear-only kicks | `oq_arm(f, 0)` first: clears never count (ES 3.0 4.2.3: clear is not a draw). |
| `frame_submit` `:1733`, before `f.w.pad()` | `oq_arm(f, 0)` (suspend), as Mesa suspends at flush; the next frame's first counted draw resumes with a new slot. `frame_open` resets `oq_on = 0`; the slot counter is per **query**, not per frame. |
| `Mark` `:2404` | add `oq_on` and the slot counter, restored by `restore()`; a taken-back draw must not leave a half-armed counter. |
| `gc_query_begin(h)` | zero the handle's slot range **and** reset `first/count`; `cache_flush` the buffer. `gc_query_result(h)`: `osal::cache_invd`, sum the `count` slots (`u64`), `!= 0`. |
| slot exhaustion | 2048 suspend/resume transitions in one query (a draw loop alternating counted draws and clears). The query is flagged `overflow`; the boolean answer is then `true` (a false positive, allowed for the conservative target only; for the exact target it is logged once). |

**Availability.** The PE stores at the CONTROL write, in stream order, so
the data is there once the frame's fence passed:
`!gc_kick_pending(end_kick)`.

**Risks [unverified]:** the GC7000UL's PE counts samples after early depth
(`GC_PE`'s early-Z path), which also means a draw the shader `discard`s may
or may not count; dEQP `occlusion_query` has no discard. Also: the 64-bit
store at ADDR needs 8-byte alignment (`slot*8` gives it); a context-init
reset (`emit_context_init`) must not reset ADDR to something stale: ADDR is
written by every resume, so it cannot.

## 4. The GL layer (shared by all three)

### 4.1 State (src/gl/es3.rs)

Replace the `Es3Name { used, bound }` pool of queries by a record per name:

```text
struct Query {
    used: bool,        // glGenQueries returned it
    bound: bool,       // it is an object (first successful glBeginQuery)
    target: GLenum,    // fixed at the first Begin; later Begin with another target = INVALID_OPERATION
    active: bool,
    handle: u32,       // backend handle (1..), 0 = none; a handle per query *use*
    end_kick: u32,     // backend::kick_id() at glEndQuery
    has_result: bool,  // the result was read once and is cached
    result: u32,       // 0/1
}
```

and per context `active_occlusion: u32` (query name or 0). **Both occlusion
targets share one slot**: dEQP says "GL_ANY_SAMPLES_PASSED and
GL_ANY_SAMPLES_PASSED_CONSERVATIVE alias to the same target for the purposes of
this error" (`cts:es3fNegativeFragmentApiTests.cpp:320-322`).
`GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN` keeps its own slot (`tf-api`).

### 4.2 The per-draw field

`GlesDraw` (`src/backend.rs:432`, C twin `struct gles_draw` `src/backend.h:201`,
layout checks `src/abi_layout.rs`, `src/abi_layout_check.c`) gets
`occlusion_query: u32` (backend handle, 0 = none), documented in
`backend.h`. `src/gl/draw.rs` sets it where it builds the draw
(`:2842`, the regular draw; `:3679, :4083` submit it): `ctx.active_occlusion`'s
handle, **0** for the pending-clear draw (`:2024-2077`, a clear is not
counted), for `glBlit*`/internal quads, and when rasterizer discard is on
(PVR: `pvr_arch_cmd_buffer.c:6878`: no vistest with rasterizer discard;
`GL_RASTERIZER_DISCARD` is `tf-api`'s state, `src/gl/state.rs:434`).
The pending clear must be flushed **at `glBeginQuery` and `glEndQuery`** (it
already is "settled" before any draw; calling `flush_pending_clear`
`draw.rs:2236` in both makes the clear land on the correct side of the
query).

### 4.3 New `YakoglBackend` entries (`src/backend.rs:743`, `src/backend.h:384`; all optional)

```text
query_begin:  fn(handle: u32) -> i32     // 0, or -ENOMEM (no slot / no counter memory)
query_end:    fn(handle: u32)            // optional; V3D/PVR need nothing, GC may suspend
query_result: fn(handle: u32, out: *mut u64) -> i32   // call only after the last kick is complete
query_free:   fn(handle: u32)            // after the query's last kick has completed
```

A back end with `query_begin == None` keeps today's behaviour exactly
(`glBeginQuery` -> `GL_INVALID_OPERATION`); that is what the host test
back ends of `src/gl` keep, and what lets the three back-end items land one
at a time.

### 4.4 Completion, "across several jobs/flushes" and RESULT_AVAILABLE

* `glEndQuery`: `end_kick = backend::kick_id()` (the kick being recorded,
  which **contains** every draw so far: V3D `:4527`, GC `:2590`, PVR
  `:6436`). For the PVR it also calls `backend::flush()` (3.2).
* `GL_QUERY_RESULT_AVAILABLE` = `!backend::kick_pending(end_kick)`. If the
  kick is the one still being recorded the GL core **first flushes**
  (`backend::flush`, `src/backend.rs:253`): the GL rule "querying forces the
  query to complete within a finite time" (Mesa does the same:
  `etnaviv_query_acc.c:122-130`, `v3d_query_pipe.c:204`), and an
  application that polls `AVAILABLE` without a `glFlush` would spin
  forever otherwise.
* `GL_QUERY_RESULT`: flush if needed, `backend::wait_kick(end_kick)`
  (`src/backend.rs:1179`; falls back to `finish`), then `query_result`;
  cache the answer (`has_result`) and free the handle. The boolean target
  returns `GL_TRUE`/`GL_FALSE` (1/0).
* Spanning jobs: nothing in the GL core. V3D and GC re-arm the counter at
  the start of every job/frame from the per-draw field; PVR takes a new
  register per kick. `end_kick` is the last of them and the GPU executes
  kicks in order (the contract in `src/pvr/backend.rs:6259-6263`), so one
  wait covers all.
* A query that is active across `eglSwapBuffers`: unchanged, the swap does
  `flush`; the query stays active (ES 3.0 2.14 has no rule that ends it).
* A query active across an FBO change or a target-size change: the
  per-draw field is independent of the target; V3D re-emits the packet per
  job (`v3d_job.c:436`), PVR splits the frame as usual, GC has one stream.
* Context loss / `eglDestroyContext` / `eglMakeCurrent(NO_CONTEXT)`
  (`src/gl/egl.rs:1843-1870`, `src/gl/context.rs:1827-1852`): queries are
  per context (`QUERIES[MAX_CONTEXTS]`, `es3.rs:107`; ES 3.0 Appendix D
  does not share queries). On destroy: for every query with a handle,
  `wait_kick(end_kick)` is **not** required (destroy already runs
  `flush`+`finish` when a clear is pending, `context.rs:1840-1844`); call
  `query_free` for each handle and clear `active_occlusion`. An active
  query at destroy time is simply dropped. A lost GPU job (`gpu_work_lost`,
  `src/backend.rs:960-1000`: sticky counter, no GL error) makes
  `wait_kick` fail: `GL_QUERY_RESULT` then returns the **conservative**
  answer (`GL_TRUE`) and `AVAILABLE` becomes true, so a polling loop
  terminates; the loss is counted by the existing mechanism. ES 3.0 has no
  `GL_CONTEXT_LOST`.

## 5. ES 3.0 section 2.14 rules to implement (and the test that pins each)

| rule | error | dEQP |
|---|---|---|
| `glGenQueries(n<0)` | INVALID_VALUE | `negative_api` (existing) |
| `glBeginQuery` bad target | INVALID_ENUM | `es3fNegativeFragmentApiTests.cpp:310-313` |
| a query of the same target already active; **ANY and CONSERVATIVE are one target** | INVALID_OPERATION | `:314-322` |
| `id == 0`, a name never generated or deleted | INVALID_OPERATION | `:332-343` |
| `id` is the name of an already active query (any target) | INVALID_OPERATION | `:347-352` |
| `id` is an existing query of another target | INVALID_OPERATION | `:354-359` |
| `glEndQuery` bad target / nothing active on that target | INVALID_ENUM / INVALID_OPERATION | `:368-381` |
| `glGetQueryObjectuiv`: name not an object (never begun), or **active** | INVALID_OPERATION | `es3fNegativeStateApiTests.cpp:1240-1262` |
| bad pname | INVALID_ENUM | same |
| `glGetQueryiv(GL_CURRENT_QUERY)` = active name or 0; bad pname/target INVALID_ENUM | | `:1005` |
| `glIsQuery`: false until the first Begin, false after delete | | `es3fNegativeStateApiTests.cpp:1015-1050` |
| `glDeleteQueries` of an active query: end it (`active_occlusion = 0`), then free; a pending result is dropped, the handle is returned only after `end_kick` completed | no error | `es3fLifetimeTests.cpp:195-215` |
| `glGetQueryObjectuiv(QUERY_RESULT)` before End: the active-query rule above; after End: waits | | `es3fOcclusionQueryTests.cpp:383-393` |

Order of checks in `glBeginQuery` must stay as `es3.rs:271-286` has it
(target, then name), then: active-on-this-target, id already active, target
of an existing object. A **rejected** Begin must not create the object
(`es3.rs:1367-1382` pins that already).

`MAX_QUERIES = 32` per context is enough for dEQP (the lifetime tests make a
handful); the back end handle space is the same 1..=32. A name beyond that
keeps today's behaviour (`GL_OUT_OF_MEMORY` from `glGenQueries`).

## 6. ES 2.0 boundary

* With `report-es2` (the default; `src/lib.rs:85` `REPORT_ES2`) **nothing
  changes**. The ES 3.0 entry points already exist in the library, but
  `GL_VERSION` says ES 2.0 and dEQP-GLES2 has no query test. No
  `GL_EXTENSIONS` string lists a query extension
  (`src/gl/state.rs:32-...`): `GL_EXT_occlusion_query_boolean` is **not**
  exposed, `glBeginQueryEXT`/`glEndQueryEXT`/`glGetQueryObjectuivEXT` do not
  exist. This item must not add the extension: the ES 2.0 result is zero
  failures and `tools/cts/` ES2 rows pin the extension list
  (`es2-guard`).
* The back-end ABI gains only appended fields; with `occlusion_query == 0`
  every back end emits exactly the words it emits today: PVR the same
  ISPCTL (`vistest` 0), V3D the same zero-address OQ packet, GC no extra
  state. That is checkable on the host (section 8, "no-regression words")
  and it is the ES 2.0 no-regression test.
* If the extension is wanted later (Qt/Skia use it): add
  `"GL_EXT_occlusion_query_boolean\0"` to `ALL_EXTENSION_NAMES`, export
  `glGenQueriesEXT`, `glDeleteQueriesEXT`, `glIsQueryEXT`,
  `glBeginQueryEXT`, `glEndQueryEXT`, `glGetQueryivEXT`,
  `glGetQueryObjectuivEXT` as thin aliases (`include/GLES2/gl2ext.h:1314`
  has the prototypes; `GL_ANY_SAMPLES_PASSED_EXT`/`_CONSERVATIVE_EXT` are in
  `src/gl/consts.rs:1433-1434`) and only then run the ES 2.0 extension
  tests. That is a separate decision for the owner (`es3-docs`), not part
  of `occlusion-query`.

## 7. Effort ranking

| rank | GPU | why | rough size |
|---|---|---|---|
| 1 (least) | **V3D 4.2** | the packet and the zero-address emission already exist in the C builder; one `u32` in `V3dJobState`, the pack-key word, a 4 KiB arena, two small back-end functions; the packet bytes are testable on the host; Mesa proves the whole flow (same BO across jobs). | 2 h |
| 2 | **GC7000UL** | stream-only (two LOAD_STATEs), no ABI change, Mesa proves it; the register offsets are in the vendored `state.xml.h`, but the suspend/resume bookkeeping touches `emit_draw`, `emit_clear`, `frame_submit` and `Mark`, and the PE timing of the store is unobserved. | 3 h |
| 3 (most) | **PowerVR** | widest ABI change (PPP builder, `PvrRenderFragRegs` in C and Rust, firmware dword 22-23, flags), the vis heap, accumulation and `GETVISRESULTS` are unobserved, plus the TBDR ordering hazard that forces a frame split at `glEndQuery` and TRANSLUCENT counted draws. The only GPU where the dEQP run is the first real test. | 4 h + board |

The shared GL part (state machine, errors, `GlesDraw` field, `backend`
entry points, tests) is 2-3 h and goes first; a host-test back end that
implements the four entries lets the GL half be finished without any GPU.
Suggested split for `occlusion-query` (the plan item is one item; if it is
split): GL core + ABI; V3D; GC; PVR. Order of landing V3D, GC, PVR.

## 8. Host tests

All of these run without a board (`cargo test`, the existing per-back-end
rigs).

**GL core (src/gl/es3.rs tests + a fake backend with the four entries).**
State machine, one test per row of section 5:

* generate / is-query lifecycle (existing four tests rewritten:
  `a_query_name_is_an_object_only_after_begin_query` etc. now expect
  `GL_NO_ERROR` for a valid Begin);
* aliasing: Begin(ANY) then Begin(CONSERVATIVE, other id) -> INVALID_OPERATION;
  Begin(TF_PRIMITIVES) while an occlusion query is active -> no error;
* End without Begin per target; End with the wrong target;
* `glGetQueryObjectuiv` on the active query -> INVALID_OPERATION; after End
  it returns 0/1 from the fake;
* `RESULT_AVAILABLE`: false while `kick_pending(end_kick)` is true **and**
  the open frame was flushed (the fake counts `flush` calls: exactly one
  per poll that finds the open kick);
* `RESULT` waits (`wait_kick` counted) and caches (second read: no wait);
* a query across three kicks (fake returns the sum of three parts);
* delete while active: `active_occlusion == 0`, handle freed only after the
  fake reports the kick complete;
* context destroy: all handles freed (`query_free` count), no wait;
* `gpu_work_lost` path: `wait_kick` fails -> `RESULT` = 1, `AVAILABLE` = 1;
* per-draw field: a draw built while a query is active carries the handle;
  the pending clear, `glClear`, `glBlitFramebuffer`, and a draw with
  rasterizer discard carry 0;
* a back end without the entries: Begin still `GL_INVALID_OPERATION`;
* `report-es2` build: `glGetString(GL_EXTENSIONS)` has no query extension
  (extend the existing extension-list test in `src/gl/state.rs:1424-1439`).

**V3D (`src/v3d/backend_tests.rs`, rig `:1025`, helper `words` `:1407`).**
Command words:

* *enable*: a counted draw's job BCL contains opcode `92` (0x5c) followed by
  the 4-byte little-endian address `arena.gpu + h*64` **before** the draw's
  `GL_SHADER_STATE`/vertex packets, and after `START_TILE_BINNING`;
* *reset/disable*: the first packet of every job (`v3d_job.c:168`) has
  address 0; an uncounted draw after a counted one in the same job emits
  `92` + 0 (the pack key changed); two counted draws of the same handle emit
  the packet once (packet skip), a counted draw of another handle emits it
  again;
* a `clear_quad` draw while a query is active emits address 0 and the next
  counted draw re-emits the address;
* the kick of a second job re-emits the address (new job);
* *read*: the fake OSAL writes N into the counter dword after the kick;
  `v3d_query_result` returns N after `cache_invd` (count the call), 0 for
  a handle that was never counted; `v3d_query_begin` zeroes the dword and
  `cache_flush`es it;
* no regression: with `occlusion_query == 0` the BCL byte stream of every
  existing test is **identical** (the existing tests stay green unmodified);
* the C packer `driver/src/v3d/` unit test (`v3d_job.c`'s own) for the key
  word.

**PowerVR (`src/pvr/backend_tests.rs`, rig `:658`, `rig.kicks()`;
`driver/src/pvr_vdm_ppp_gen.c` test).**

* *enable*: with a counted draw the PPP block's `TA_STATE_ISPCTL` word has
  bit 15 set and bits 0..13 = the register; an uncounted draw in the same
  frame has them clear; clear/load quads never have them;
* *kick*: `kick.frag.cmd_flags & 0x20 != 0` and `isp_oclqry_base` = the vis
  buffer with 16-byte alignment, only when the frame has a counted draw;
  otherwise both 0 (ES 2.0 identical: `ISP_CTL_DEFAULT` etc. tests
  `:863, :1686` untouched);
* *reset*: the register's dword is zeroed in the mapped buffer before
  `submit` (the fake OSAL asserts the value at submit time);
* *one register per kick*: two kicks of one query use two registers, the
  fake writes 3 and 0, `query_result` = 3; a query over 17 kicks folds;
* *frame split*: `glEndQuery` leaves no open frame (`rig.kicks()` grew by 1);
  an occluder drawn after End is in the next kick;
* *pass type*: counted draws are `PASSTYPE_TRANSLUCENT` (bits of the ISPA
  word), uncounted keep OPAQUE;
* ABI: `PvrRenderKick` size 216, `isp_oclqry_base` at 208 on both the C and
  Rust asserts; `driver/rust` `render.rs` test: dwords 22-23 of the CMD3D
  body hold the base (the existing selftest/`rec.c` dump prints it).

**GC7000UL (`src/vivante/backend_tests.rs`, rig `:346`, which decodes the
command stream).**

* *enable*: a counted draw is preceded by `LOAD_STATE ADDR = oq.gpu +
  slot*8` (the exact word: `load_state(reg, 1, false)` header, value,
  pad) and, if another handle was armed, by the `CONTROL = 0x1DF5E76`
  write first;
* *reset/suspend*: `frame_submit` ends an armed stream with the `CONTROL`
  write; the clear quad (`emit_clear`) is preceded by it; the next counted
  draw after a clear re-arms with **slot + 1**;
* the 64-bit slot offsets are 8-byte aligned and never exceed the buffer;
  the 2049th transition sets `overflow` and the result is `true`;
* `Mark::restore` after a taken-back draw: `oq_on` and the slot counter are
  back (draw fills the frame, is re-recorded in the next: exactly one
  resume in the next frame, none lost);
* *read*: the fake writes `[0, 5, 0]` into slots 0..3: result `true`; all
  zeros: `false`; `query_begin` zeroes;
* no regression: with `occlusion_query == 0` the stream of every existing
  test is word-for-word unchanged.

**Cross-GPU.** `tests/backend_differential.rs` (the same GL calls through
all three back ends) gets one `es3_occlusion_*` test:
`Begin / draw with depth test / End / Finish / result` on the fake OSALs of
the three GPUs, using one scripted "samples passed" source each.

**Not host-testable (board items):** that the counter counts at all; the
V3D accumulate-across-jobs behaviour; PVR heap, `GETVISRESULTS`,
accumulation and ordering; the GC PE store timing; the dEQP 62 + 4 cases.

## 9. Open questions, in the order they will bite

1. **PVR vis-test heap** (3.2): general heap first, else a new heap in
   `driver/src/pvr_render.c`.
2. **PVR OPAQUE vs TRANSLUCENT** for counted draws, and whether `EndQuery`
   needs the frame split (3.2): decide from the first board run of
   `occlusion_query.depth_write*`.
3. **V3D accumulate** (3.1): one board test.
4. **ES 2.0 extension** (section 6): owner decision, not this plan.
