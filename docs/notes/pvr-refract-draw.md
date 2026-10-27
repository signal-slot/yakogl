<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR host: where glmark2 refract's (and shadow's) glDrawArrays time goes

`tools/hostharness/glmark2/FINDINGS-pvr-allscenes.md` ("The top 5 costs",
rows 1 and 2) left the cost of `refract`'s and `shadow`'s `glDrawArrays`
unattributed: 0.94 and 0.67 ms a draw against about 2 us for an ordinary draw,
and `perf` was refused. This note splits those draws into steps.

**Answer: 93 to 97% of the time of both scenes' draw calls is one step,
`pvr_backend_depth_resolve`, the CPU conversion of the 1600 x 1200 D24S8 depth
buffer the ISP stored into the R32F texels of the depth texture the next draw
samples.** It runs once a frame, inside the one draw that samples the depth
texture (draw 2 of 2 in refract, draw 2 of 3 in shadow). The other draws of the
frame cost 6 to 90 us, as does a `texture` draw (9 us). Vertex staging, the
draw's record, the PDS programs, the OSAL's allocation model (`l_gpu_alloc`
memset) and the kick submit are each at or under 3% of the draw call.

## Method

* Commit measured: `41e95169` (main, `hostharness: compare.py ...`), x86-64
  Linux host, `tools/hostharness/glmark2/run.sh` for the `powervr` build,
  glmark2 `22c527cb` (the commit the harness pins), 800x600,
  `GM2_PROFILE=1 GM2_FRAMES=20` (19 counted frames: frames 2 to 20), one process
  per scene, entries `texture:texture-filter=nearest` (the plain baseline),
  `shadow` and `refract`.
* `GM2_PERF=1` was tried and refused: `perf record` fails with "Failure to open
  any events" (`kernel.perf_event_paranoid` above 2 on this host). So the draw
  was split with clock laps instead: a local build only, **not committed**
  (`src/pvr/backend.rs` and `src/gl/**` belong to running items). A lap is a
  `clock_gettime(CLOCK_MONOTONIC)` read into a static array of ns by step,
  printed after each `glDrawArrays`. Laps stood in
  `draw_arrays` (arguments, pending-clear settle, the gap before the back end),
  at the `draw_steps::Timer` laps of `build_common` (program, uniforms,
  attributes, textures, state), around `resolve_sampled_depth` and inside
  `texture.rs resolve_depth` (the flush of the rendering kick, the finish, the
  back end's `depth_resolve`, the cache flush), and at each section of the
  back end (`pvr_backend_draw` checks, `draw_one` frame handling,
  `record_draw`: vertex PDS, kick-USC PDS, coefficient PDS, common-store DMA PDS,
  PPP state, vertex uniforms, VDM stream). Steps of one draw run one after the
  other, so their sum is the draw's time; every lap interval is named below, so
  100% of the draw is attributed to a step (the one unexplained interval is
  listed).
* The host varies by a factor of up to 4 from run to run (shared machine):
  `depth_resolve` of the same draw took 3.0, 3.3, 7.9, 10.1, 11.6 ms in five
  runs of the same code, with the other steps scaling together (the ratio of
  the resolve to the draw's record stayed 33 to 53 for refract and 100 to 190
  for shadow). The table is the quietest run of 5 (`laps3`); the shares are
  the same in all 5 (refract 93 to 95%, shadow 95 to 97%).
* Repeats: one plain `GM2_PROFILE=1` run (no laps) and eight runs with the
  laps (the laps' per-run `profile-powervr.md` has `glDrawArrays` too). The
  plain run gave `glDrawArrays` per frame (ms) refract 7.12, shadow 10.99; the
  lap runs 3.41 and 3.14 on the quiet stretch, 12.3 and 10.3 on the slowest. These are not the
  0.94 ms and 0.67 ms a draw of the FINDINGS file (a faster stretch of a
  different machine, 3 frames): a draw here is 3.44 ms (refract) and 3.06 ms
  (shadow) for the draw that resolves. Read the shares, not the milliseconds.

## Per-step table (ms, quietest run, 18 counted frames, mean a frame)

`glDrawArrays` of the scene: refract 2 draws, shadow 3 draws, texture 1.

| step | refract ms a frame | share | shadow ms a frame | share | texture ms a frame |
|---|---|---|---|---|---|
| **back end `depth_resolve`** (ZLS unorm24 to R32F, 1600 x 1200 = 1.92 Mpx) | 3.299 | **94.1%** | 3.049 | **96.9%** | 0 |
| flush of the kick that rendered the depth (`backend::flush`) and the `finish` | 0.0007 | 0.0% | 0.0014 | 0.0% | 0 |
| `cache_flush` of the texels and bookkeeping after the resolve | 0.003 | 0.1% | 0.002 | 0.1% | 0 |
| `record_draw`: vertex PDS, kick-USC PDS, coefficient PDS, common-store DMA PDS, PPP state, vertex uniforms | 0.089 | 2.5% | 0.042 | 1.3% | 0.0066 |
| back end: frame handling (`ds_make_room`, flush at a target change, `frame_begin`) | 0.030 | 0.8% | 0.005 | 0.1% | 0.0005 |
| back end: VDM stream, tail, restamp | 0.009 | 0.2% | 0.003 | 0.1% | 0.0005 |
| GL core: arguments, `settle_pending_clear`, `build_common` (program, uniforms, attributes, textures, state) | 0.013 | 0.4% | 0.012 | 0.4% | 0.0013 |
| gap between `build_common` and the back end's entry, first draw of a frame only | 0.063 | 1.8% | 0.032 | 1.0% | 0.0001 |
| total | 3.506 | 100% | 3.145 | 100% | 0.009 |

Per draw (medians, us): refract draw 1 (the pass into the depth texture) 90,
refract draw 2 (samples it, resolves) 3437 of which `depth_resolve` 3322;
shadow draw 1 47, draw 2 (resolves) 3065 of which `depth_resolve` 3024, draw 3 7;
a `texture` draw 8.9. In the other four runs the same step is 92.9 to 95.1% of the
refract frame and 95.1 to 97.0% of the shadow frame; the sum of all the other
steps is 0.2 to 0.7 ms (refract), 0.04 to 0.4 ms (shadow) when the machine is
slow.

The one interval with no code in it is the "gap": 30 to 150 us (up to 0.5 ms
in the slowest run) between the end of `build_common` and the entry of the
back end, in the **first draw of each frame only** (`draw_arrays` sets three
fields and builds a struct of pointers between them). It is not a step of the
library; it is 1 to 4% of refract's draw call and the size of a page fault or
a cold cache after the swap. It is the only part of the draw not attributed to
a code path, and it is under the 20% the attribution leaves.

## What the resolve does

`resolve_depth` (`src/gl/texture.rs`) is the fallback of `depth_for_draw`:
the PowerVR back end does not answer `samples_rendered_depth`, so before the
draw that samples the depth texture the GL core submits the rendering kick,
waits for it, and has the back end write the depth into the texture's R32F
texels (`pvr_backend_depth_resolve`, `src/pvr/backend.rs`, "Write the depth
plane of GL object `id`"). The branch taken on the host is the live one
(counted in a local run: 20 of 20 resolves of each scene read the ISP's buffer;
none took the cleared-value `fill`). The loop is
`ds_retention::write_depth_texels` with a closure per texel
(`x < sw && y < sh`, a checked `words[y * aw + x]`, an `Option<f32>`), and
`unorm24_depth`: one f64 divide per texel. 1.92 Mpx in 3.3 ms is 1.7 ns a
texel.

A reciprocal multiply for the divide was tried in the local build (two runs,
`laps6`, `laps7`) and moved nothing measurable against the noise (resolve to
record ratio 34 and 53, against 33, 44 and 35 with the divide, refract): the
loop is dominated by the closure, the checked index and the memory traffic, not
the divide. That is what the GC7000 note found when it rewrote the same loop by
rows (`docs/notes/gc-depth-resolve.md`: 2x on the micro-benchmark, the divide
was what was left).

## Host-model artefacts: what would not show on the AM62P

* **Nothing waits.** linux.c completes every submit at once, so the flush and
  the finish before the resolve cost 0.001 ms. On the AM62P the finish is
  the wait for the depth pass's GPU time (shadow's board kick times are the lead's
  work and not proposed here), which sits in the same `glDrawArrays` in front of
  the conversion.
* **Cached, malloc'd memory.** The host reads the 7.7 MB depth buffer and writes
  the 7.7 MB texels at cache speed; the board maps GPU memory and its
  `cache_invd`/`cache_flush` model is real (`osal::cache_invd` over 7.7 MB, then
  the texels): on a Cortex-A53 with a non-cacheable or write-combined mapping
  the same loop would be slower (not measured here; the Pi 4 note
  `docs/notes/pi4-glmark2-cpu.md` has the board's CPU reads against the host's
  for the V3D). Expect the host figure to be a lower bound for the AM62P.
* **The depth buffer holds zeros** (nothing is rendered), so the converted
  values are all 0.0. The f64 divide and the loop cost do not depend on the
  value. Nothing else in the draw reads it.
* **The OSAL model's allocation** (`l_gpu_alloc` memset) is not in these frames:
  the arenas and the depth buffer are allocated at set-up and reused; the
  resolve writes into the texture's existing storage. It is not a cause here.
* The gap in the first draw of each frame (above) may be partly a host effect
  (page faults on memory the swap just recycled); it is small either way.

Not an artefact: the resolve itself. The loop, its 1.92 Mpx and its CPU
conversion run unchanged on the board.

## What Mesa's PowerVR driver does

Mesa's `pvr` (Vulkan, `src/imagination/vulkan`, 26.2.1) has no depth-resolve
step: a depth/stencil image is an ordinary image the ISP stores to and the TPU
samples. `pvr_get_format_swizzle_for_tpu` (`pvr_formats.c`) is all the format
work a depth image needs for sampling: the TPU reads the depth into the first
channel, "hardware can only access either depth or stencil at once". There is
no readback and no format conversion on the CPU (nothing of the kind in the
driver's image or format code); ordering between the depth pass and the
sampling draw is the Vulkan barrier's job, not a CPU `finish`. The cost of that design is that the
ISP's depth store must be in a layout the TPU can sample (Mesa's own image
layout, with a D24S8 or D32 format the TPU supports). YakoGL's store is the
ZLS buffer: packed D24S8 laid out linearly at the ZLS-tile-aligned width, which
`pvr_backend_depth_resolve` documents as "not a format the texture unit is
known to sample"; whether the TPU can read it directly (or the ISP can store
into an R32F/D32F target of the texture itself) has not been tried on the
AM62P. That is the experiment the first fix below needs, the same question
the GC7000 has in `docs/notes/gc-depth-sample.md`.

## Ranked fixes

Estimates are of the draw call's host CPU time; the board's will be larger.

1. **Sample the rendered depth from the ZLS buffer, no resolve**
   (`samples_rendered_depth`, the hook the V3D already answers; see
   `depth_for_draw` in `src/gl/texture.rs`). Gain: the whole step, 94 to 97% of
   refract's and shadow's draw calls (about 3.3 ms a frame on the host, more on the
   board), less nothing: the V3D's equivalent still copies. Files:
   `src/pvr/backend.rs` (answer `samples_rendered_depth`, bind the buffer as the
   draw's texture with the depth format and the aligned stride),
   `src/pvr/backend_tests.rs`, a board experiment first. Risk: high. The TPU must
   read the ZLS word (24-bit unorm in bits 23..0, stencil in 31..24) at the
   buffer's linear aligned pitch; if it cannot, the fix is not available and
   fix 2 is the answer. A texture sampled this way stays stale in its texels
   (the GL core already resolves on the next CPU access).
2. **Convert by rows on slices, no closure, no per-texel `Option`**
   (`pvr_backend_depth_resolve`, `ds_retention::write_depth_texels`). Gain: the
   GC7000's identical rewrite halved a 1024 x 1024 resolve (1.10 to 0.56 ms best);
   expect 1.5 to 2 ms of the 3.3 ms here (about 45 to 60% of the draw call).
   Files: `src/pvr/backend.rs`, `src/ds_retention.rs`, a test like
   `tests/gc_depth_resolve.rs` (bit-exact against the old loop, the pitch,
   padding, 1 x 1 and oversized-request cases). Risk: low, bit-exact by
   construction of the test. Needs the unorm24 conversion left as it is or the
   exhaustive check of `gc-unorm24-exhaustive` for any change to it.
3. **A faster unorm24 conversion** (reciprocal multiply: `gc-unorm24-fix`, in
   review). Gain on the PowerVR host: none measured (see above), so it is not
   a PowerVR fix on its own; it only helps once fix 2 removes the closure.
   Files: `src/ds_retention.rs`. Risk: low (exhaustive check exists).
4. **Narrow the resolve to the region the sampling draw reads.** glmark2's
   shadow and refract sample the whole map, so no gain on them; recorded
   only so nobody expects it. Not proposed.

Not proposed: the first-draw gap (1 to 4%, no code path to change), any step of
the record (0.04 to 0.09 ms a frame), the flush and finish (host: 0; on the board
they are the GPU's time, the lead's), anything about glGenerateMipmap (#916).

## Suggested updates elsewhere (not made here)

* `docs/README.md`: `- [pvr-refract-draw.md](notes/pvr-refract-draw.md): PowerVR host, glmark2 refract and shadow: 94 to 97% of the draw call is the CPU depth resolve, a ranked fix list.`
* `FINDINGS-pvr-allscenes.md`, rows 1 and 2 of "The top 5 costs": where "Where
  inside is not narrowed" and "The draw: unknown until the symbols are sampled" stand,
  write "`pvr_backend_depth_resolve` (the CPU conversion of the 1600 x 1200
  ZLS depth into the sampled depth texture), 94 to 97% of the draw call,
  `docs/notes/pvr-refract-draw.md`", and in "What a rerun still owes" replace the
  first bullet (the top-3 symbols column) with a pointer to this note, which
  answers it with laps because `perf` was refused.
