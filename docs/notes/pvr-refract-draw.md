<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR host: where glmark2 refract's (and shadow's) glDrawArrays time goes

**Status (2026-11-27): fixes 1 and 2 are on main.** The measurement below
(`41e95169`) was made before fix 1. Fix 1 landed in main `21099fcd`; fix 2
(the row resolve) in `f67df7b4` and `006e44a3`, measured in "Measured after
fix 2" below. Fix 1's design, its TPU
state words and its board figures are `docs/notes/pvr-depth-sample.md`. The
PowerVR back end answers `samples_rendered_depth` (`pvr_depth_sampled`,
`src/pvr/backend.rs`): a rendered depth texture of one level, sampled by the
next draw, is bound as the depth buffer's X8U24 stride image
(`depth_buffer_binding`) and `depth_for_draw` (`src/gl/texture.rs`) resolves
nothing, so glmark2 `shadow` and `refract` no longer reach the CPU depth
resolve on main. The resolve remains only for the fallback paths: a CPU read
or write of the texels, a rendered level above 0 or a chain, a buffer holding
no stored depth of the texture's size or only a whole clear, a buffer the
frame being recorded still draws into, a multisampled buffer, and the host
tests' mock tables; fix 2 made those paths cheaper (1600 x 1200 depth resolve
1.6 to 2.2 ms best before, 0.25 to 0.27 ms after) but glmark2 does not take
them. Everything below is the path at the measured commit
`41e95169`, kept as the historical record of what the resolve cost before fix
1 landed.

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
at the measured commit the PowerVR back end did not answer
`samples_rendered_depth` (it does now — `pvr_depth_sampled`, see the status at
the top), so before the draw that samples the depth texture the GL core submits
the rendering kick,
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
into an R32F/D32F target of the texture itself) had not been tried on the
AM62P at the time of measurement. Fix 1 has since answered the first question:
the TPU reads the ZLS word as an X8U24 stride image (main `21099fcd`,
`docs/notes/pvr-depth-sample.md`). The ISP-stores-into-the-texture variant
remains untried, the same question the GC7000 has in
`docs/notes/gc-depth-sample.md`.

## Ranked fixes

Estimates are of the draw call's host CPU time; the board's will be larger.
**State (2026-11-27): fix 1 has landed (main `21099fcd`,
`docs/notes/pvr-depth-sample.md`); fix 2 has landed (main `f67df7b4` helper,
`006e44a3` back end); fixes 3 and 4 are unchanged.**

1. **Sample the rendered depth from the ZLS buffer, no resolve** — **landed
   (main `21099fcd`)**, design, TPU state words and board figures:
   `docs/notes/pvr-depth-sample.md`. (`samples_rendered_depth`, the hook the
   V3D already answers; see `depth_for_draw` in `src/gl/texture.rs`.) Gain: the
   whole step, 94 to 97% of refract's and shadow's draw calls (about 3.3 ms a
   frame on the host, more on the board), less nothing: the V3D's equivalent
   still copies. Files: `src/pvr/backend.rs` (answer `samples_rendered_depth`,
   bind the buffer as the draw's texture with the depth format and the aligned
   stride), `src/pvr/backend_tests.rs`, a board experiment first (done, in
   `docs/notes/pvr-depth-sample.md`). Risk: high. The TPU must read the ZLS
   word (24-bit unorm in bits 23..0, stencil in 31..24) at the buffer's linear
   aligned pitch; it can, as an X8U24 stride image, so fix 2 stands as the
   answer for the resolve paths that fix 1 leaves. A texture sampled this way
   stays stale in its texels (the GL core already resolves on the next CPU
   access).
2. **Convert by rows on slices, no closure, no per-texel `Option`** — **landed
   (main `f67df7b4`: `ds_retention` row helpers; `006e44a3`: the back end's
   depth and stencil resolve use them; measured below)**. (`pvr_backend_depth_resolve`,
   `ds_retention::write_depth_texels`.) Gain: the GC7000's identical rewrite
   halved a 1024 x 1024 resolve (1.10 to 0.56 ms best); expect 1.5 to 2 ms of
   the 3.3 ms here (about 45 to 60% of the draw call); after fix 1 landed, that
   is the gain on the resolve paths fix 1 leaves — `shadow` and `refract` no
   longer resolve on main. Files: `src/pvr/backend.rs`, `src/ds_retention.rs`,
   `tests/depth_resolve_pvr_rows.rs` (on main: bit-exact against the old loop,
   the pitch, padding, 1 x 1 and oversized-request cases). Risk: low,
   bit-exact by construction of the test. The unorm24 conversion is unchanged.
   Measured: the 1600 x 1200 depth resolve 1.63 to 2.20 ms best before, 0.25
   to 0.27 ms after (6 to 9 times); the stencil resolve 0.46 to 0.59 ms to
   0.11 ms (see "Measured after fix 2").
3. **A faster unorm24 conversion** (reciprocal multiply: `gc-unorm24-fix`, in
   review). Gain on the PowerVR host: none measured (see above), so it is not
   a PowerVR fix on its own; it only helps once fix 2 removes the closure.
   Files: `src/ds_retention.rs`. Risk: low (exhaustive check exists). After
   fix 1 landed the loop it speeds is reached on main only on the fallback
   paths named in the status.
4. **Narrow the resolve to the region the sampling draw reads.** glmark2's
   shadow and refract sample the whole map, so no gain on them; recorded
   only so nobody expects it. Not proposed.

Not proposed: the first-draw gap (1 to 4%, no code path to change), any step of
the record (0.04 to 0.09 ms a frame), the flush and finish (host: 0; on the board
they are the GPU's time, the lead's), anything about glGenerateMipmap (#916).

## Measured after fix 2

Two `origin/main` commits, each built in its own git worktree, the same
machine (x86-64 Linux, AMD Ryzen AI Max+ 395, 4 cores available, shared):

* BEFORE `4236a921`: the first parent of the row-resolve commit; it holds fix
  1 (`21099fcd`) and not fix 2. `tests/depth_resolve_pvr_rows.rs` is there
  (its timing test measures the old per-texel resolve).
* AFTER `006e44a3` (`pvr: depth and stencil resolve convert by rows,
  bit-identical`, on `f67df7b4`'s helpers), main's tip when measured.

Harness: `tools/hostharness/glmark2/run.sh <glmark2 22c527cb> <out> powervr`,
`GM2_PROFILE=1 GM2_FRAMES=20` (19 counted frames), one process per scene,
entries `texture:texture-filter=nearest`, `shadow`, `refract`. Three runs per
commit, alternating BEFORE, AFTER, BEFORE, AFTER, BEFORE, AFTER.
Frame is the harness's
`gl-frames` wall ms a frame (the scene's whole frame); `glDrawArrays` is its
`gl-time` wall total over the counted frames, per frame and per draw (refract 2
draws a frame, shadow 3, texture 1). Median, with min to max of the three
runs in brackets.

| scene | commit | frame ms | `glDrawArrays` ms a frame | `glDrawArrays` us a draw |
|---|---|---|---|---|
| texture | BEFORE `4236a921` | 0.0245 (0.0212 to 0.0247) | 0.0061 (0.0057 to 0.0079) | 6.1 (5.7 to 7.9) |
| texture | AFTER `006e44a3` | 0.0215 (0.0207 to 0.0224) | 0.0058 (0.0057 to 0.0060) | 5.8 (5.7 to 6.0) |
| shadow | BEFORE `4236a921` | 0.0511 (0.0501 to 0.0519) | 0.0169 (0.0164 to 0.0180) | 5.6 (5.5 to 6.0) |
| shadow | AFTER `006e44a3` | 0.0485 (0.0421 to 0.0523) | 0.0160 (0.0128 to 0.0167) | 5.3 (4.3 to 5.6) |
| refract | BEFORE `4236a921` | 0.0921 (0.0820 to 0.0959) | 0.0277 (0.0251 to 0.0290) | 13.8 (12.5 to 14.5) |
| refract | AFTER `006e44a3` | 0.0914 (0.0798 to 0.0960) | 0.0269 (0.0236 to 0.0293) | 13.5 (11.8 to 14.7) |

**Result: the glmark2 scenes did not move.** Every difference is inside the
spread of the runs. Both commits are after fix 1: in each of the 12 runs of
`shadow` and `refract` the library logs once `pvr_backend: a 1600x1200 depth
texture is sampled from its depth buffer (X8U24, stride 1600): no CPU resolve`
(there is no "sampled N, refused M" counter line in the log; every draw ran,
`refused=0 gl_errors=0` in all 18 `scene-end` lines), so the scenes never
call `pvr_backend_depth_resolve` and fix 2 is off their path. Compare the
`41e95169` figures above, 3.1 to 3.4 ms a frame with the resolve and 0.05 to
0.09 ms without it. The host does not model the GPU (nothing is rendered), so
these are the CPU cost of the GL calls only.

The resolve itself, where it still runs: `cargo test --release
--no-default-features --features host-test,gpu-powervr,shaderc-online --test
depth_resolve_pvr_rows -- --nocapture resolve_1600x1200_timing` (1600 x 1200,
one rendered depth buffer, 8 resolves a run), three runs per worktree,
alternating:

| resolve (1600 x 1200) | BEFORE `4236a921` best / mean ms | AFTER `006e44a3` best / mean ms |
|---|---|---|
| depth, run 1 | 1.629 / 1.890 | 0.261 / 0.494 |
| depth, run 2 | 1.692 / 2.016 | 0.270 / 0.518 |
| depth, run 3 | 2.201 / 2.469 | 0.251 / 0.476 |
| depth, median of the runs | 1.692 / 2.016 | 0.261 / 0.494 |
| stencil, run 1 | 0.455 / 0.530 | 0.110 / 0.196 |
| stencil, run 2 | 0.471 / 1.112 | 0.114 / 0.215 |
| stencil, run 3 | 0.587 / 0.673 | 0.107 / 0.163 |
| stencil, median of the runs | 0.471 / 0.673 | 0.110 / 0.196 |

The depth resolve is 6.5 times faster on its best (median 1.69 to 0.26 ms), the
stencil resolve 4.3 times (0.47 to 0.11 ms): more than the 45 to 60% the
GC7000's rewrite suggested, because on this host the old loop's per-texel
closure and checked index were most of the time (the 3.3 ms of the original
note was a slower stretch). The 1x layout only; a 4x (2 by 2) buffer still
converts through `step_by` and was not timed. Which paths still run it on main
is in the status at the top; none of glmark2's.

Raw logs of these runs are not committed (a local `out/` of the workspace).

## Suggested updates elsewhere (applied; both on main, checked 2026-11-27)

* `docs/README.md`: **done** — the index carries
  `notes/pvr-refract-draw.md` ("PowerVR host, glmark2 refract and shadow: 94
  to 97% of the draw call was the CPU depth resolve (before fix 1); a ranked
  fix list.") and `notes/pvr-depth-sample.md`.
* `tools/hostharness/glmark2/FINDINGS-pvr-allscenes.md`, rows 1 and 2 of "The top 5 costs": **done** — both rows now name `pvr_backend_depth_resolve` (the CPU conversion of the 1600 x 1200 ZLS depth into the sampled depth texture, 94 to 97% of the draw call), cite this
  note and main `21099fcd`, and the first bullet of "What a rerun still owes"
  points to this note instead of owing a `perf` run.
