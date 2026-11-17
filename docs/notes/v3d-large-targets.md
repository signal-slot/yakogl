<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D targets past 2048: the cause of the Pi 4 precision-group abort

Issue #1261. Images S03-S08 of `tools/cts/pi4-gles3-tree.txt` (192 / 201 /
202 / 204 / 206 / 142 cases, 1,147 in all) each stop at their first
`shaders.builtin_functions.precision.<fn>.highp_fragment.scalar` case (S03
`abs`, S04 `cosh`, S05 `fract`, S06 `mix`, S07 `reflect`, S08 `sub`). 1,142
mustpass cases never began. This note records the cause as far as the code and
the records show it. The fix belongs to plan item fix-v3d, which updates this
note; the options are listed at the end.

Line numbers are those of the tree at commit `9e1220dd`.

## The refusal

`src/v3d/backend.rs`:

- `pub const MAX_TARGET: u32 = 2048;` (line 435), "the largest target the
  tile memory is sized for at init". `MAX_JOB_TILES` (line 441) is derived from
  it, and so is the tile-state sizing at line 5910.
- In the draw path (lines 6287-6293) one text, `PAST_2048` ("a target larger
  than the tile memory is sized for (2048x2048), or empty"), answers two arms:
  - `d.target_w == 0 || d.target_h == 0` goes through `refuse()` (line 1532):
    `-EINVAL`.
  - a side `> MAX_TARGET` goes through `refuse_unsupported()` (line 1540):
    `-ENOTSUP`.
- The back end prints `yakogl/v3d: draw refused: <text>` the first time a text
  is seen (`count_refusal`).

The cap is below what the GL core reports (`src/gl/state.rs`):

| query | reported | line |
|---|---|---|
| `GL_MAX_RENDERBUFFER_SIZE` | 4096 | 1228 |
| `GL_MAX_TEXTURE_SIZE`, `GL_MAX_CUBE_MAP_TEXTURE_SIZE` | 4096 | 1226, 1227 (`texture.rs` `MAX_TEXTURE_SIZE`, line 472) |
| `GL_MAX_VIEWPORT_DIMS` | 4096 x 4096 | 1216-1219 (`MAX_VIEWPORT_DIM`, line 293) |

So a 4096-wide renderbuffer or texture attachment is legal GL (ES 3.0 asks for
2048 at least) and the V3D back end cannot draw to it.

## The error chain

`submit_draw` in `src/gl/draw.rs` (line 632) turns a negative back-end result
into a GL error (line 650): `-ENOTSUP` becomes `GL_INVALID_OPERATION`, every
other value `GL_OUT_OF_MEMORY` (it logs `glDrawArrays was not recorded (-22)`).
dEQP treats `GL_OUT_OF_MEMORY` as fatal: the executor's draw check
(`glsShaderExecUtil.cpp`, `FragmentOutExecutor`, line 684 as the board logs
name it) records a `ResourceError` and the run ends `Test run was ABORTED!`,
`complete=0`. Chain: refused draw -> `GL_OUT_OF_MEMORY` -> `ResourceError` ->
ABORTED -> every later case of the image never began. The board runner still
reads `rc=0 verdict=done`; only the tally's `complete=0` shows it.

## What draws to the large target

The executor sizes its framebuffer from the number of inputs: about 4096
pixels wide by 2-6 rows (the plan's host repro, item repro-v3d; the CTS source
was not read for this note). On the host only
`shaders.builtin_functions.precision` reaches the refusal: 1,030 of its 1,147
cases, at 4096 x 2-6 targets.

## A caution: which arm the board hit

The board records show `-22` (`-EINVAL`) and `GL error 0x505`
(`GL_OUT_OF_MEMORY`). With the code of this tree a side past 2048 is `-ENOTSUP`
and reaches the application as `GL_INVALID_OPERATION`, which fails one case and
does not abort the run (the contract in `src/gl/draw.rs` lines 615-626). The
split of the two arms is commit `b7c1484e` (2026-11-12); neither board commit
descends from it. At `b1b5b845` (the rows16-30 session) `src/v3d/backend.rs`
line 6019 refused empty and past-2048 targets together through `refuse()`, so
the abort is explained for that commit. On the current main a 4096-wide target
would give `GL_INVALID_OPERATION` instead: the groups would then fail case by
case (a Fail count) rather than abort. That is not run on a board; fix-v3d's
host run shows it, and the board session after the fix decides.

## The PowerVR's precision abort

`docs/es3-cts-failures.md` RC16 ("Large draws", PowerVR, image 7) is a
different cause in the same group: a 24,384-vertex `glDrawArrays`
(`precision.clamp.highp_fragment.vec3`) did not fit the 1 MiB general arena
and was refused whole. Fixed by cutting the draw into pieces in
`draw_dispatch` (`src/pvr/backend.rs`, line 6572). That is a draw-size limit
(vertices); the V3D one is a target-size limit (pixels). Cutting the draw
does not help the V3D: one draw still binds one 4096-wide target.

## Records

- `tools/cts/log/2026-11-pi4-gles3-b1b5b845-rows16-30/README.md` (94b733a6,
  2026-11-14, plan #320 session 2): S03-S08 aborted on their first case,
  1,142 never began.
- `tools/cts/log/2026-11-pi4-gles3-b/README.md` (8990a5cc): the same, "Why
  S03-S08 stopped early".
- `docs/notes/v3d-refusals.md` row A5 still says this refusal fired 0 times;
  fix-v3d updates it.

## Possible fixes (fix-v3d's)

- Size the tile memory for 4096 (raise `MAX_TARGET`; `MAX_JOB_TILES` is a
  square, so it would grow 4x, while a 4096 x 6 target bins far fewer tiles:
  the cost is for tile-cost to measure).
- Render a large target in bands (several jobs, each within the tile memory).
- Report smaller limits (2048 for renderbuffer, texture and viewport), which
  is plan #375's ground (RC10 limits) and would refuse textures and
  attachments the V3D samples today.

## After the fix (fix-v3d, plan #376)

The tile memory limits a target by its tile count, not by its side: the
binner's tile pool and the tile state are sized at init for 4,096 tiles (2048
square at four 32-bpp RTs, 32x32 tiles: tile alloc 1,056,768 bytes, tile state
1,048,576), and the hardware, Mesa (`v3d_limits.h` 4096 a side) and the packet
fields take 4096 a side. So the buffers stay as they were and only the checks
moved (`src/v3d/backend.rs`, line numbers of the fix commit):

- `MAX_TARGET` is 4096 (line 438), the side the GL core reports for
  `GL_MAX_RENDERBUFFER_SIZE`, `GL_MAX_TEXTURE_SIZE` and `GL_MAX_VIEWPORT_DIMS`
  (unchanged, src/gl is not touched). `POOL_TARGET` (2048, line 441) sizes the
  pools at init and `MAX_JOB_TILES` (4,096, line 447); `rt_texture` keeps its
  2048 limit for render images through it.
- The draw path names each cause: "an empty target (0 pixels on a side)"
  (`refuse`, `-EINVAL`, a malformed draw) and "a target past 4096 pixels on a
  side" (`refuse_unsupported`, `-ENOTSUP`; the GL core makes no larger
  storage, so it is a guard).
- `begin_job` checks every job's tiles before it sets anything up, at the tile
  size the job builder picks for the RT count, 4x and bpp
  (`v3d_frame_tiling_bpp`): "a target with more tiles than the tile memory
  holds (4096)" (`-ENOTSUP`). Before, only a typed (ES 3.0 format) job was
  checked, under "a 64- or 128-bpp colour target with more tiles than the job
  pools hold"; an untyped job could not exceed the pools while its side was
  2048.
- `-ENOTSUP` reaches the application as `GL_INVALID_OPERATION`
  (`src/gl/draw.rs` `submit_draw`), and the back end logs `yakogl/v3d: draw
  refused: <cause>` the first time a cause is seen.

What a 4096 side takes now:

| target | tile size | tiles | result |
|---|---|---:|---|
| the executor's 4096 x 2-6, one 32-bpp RT | 64x64 | 64 | drawn |
| the same at 4 RTs and 128 bpp, the smallest tiles | 16x16 | 256 | within the tile memory (by the tile count; not a host test) |
| 4096 x 4096, one 32-bpp RT | 64x64 | 4,096 | drawn |
| 4096 x 4096, 2 RTs | 64x32 | 8,192 | refused, tile count |
| 4096 x 4096, 4x, or 64/128 bpp (RGB10_A2, sRGB, R16F, RGBA16F, RG32, RGBA16, RGBA32 and their integer forms) | 32x32 to 16x16 | 8,192-16,384 | refused, tile count |
| a side of 4097 | - | - | `GL_INVALID_VALUE` at storage (the core); the back end's guard |

Host tests (feature sets in their headers): `src/v3d/backend_tests.rs`
`a_target_up_to_4096_a_side_draws_while_its_tiles_fit` and the re-pinned
`a_backend_gap_is_enotsup_and_counted`; `tests/es3_v3d_refusal_gl.rs`
`a_target_up_to_4096_a_side_draws` (4096 x 6, 4096 x 16, 6 x 4096 and 4096
square through the GL core: no error, no refusal) and
`a_target_with_too_many_tiles_is_invalid_operation`;
`tests/es3_rb_formats_v3d.rs` `pinned_draw_at_4096_a_side_by_tile_count`
(every renderable format at 4096 square: the eleven 64/128-bpp ones are
refused by their tiles, the rest draw) and
`pinned_the_4096_edge_in_each_dimension`; `tests/v3d_es2ctx_refusals.rs`
`a_4096_target_is_accepted`, no longer `#[ignore]`d.

Host run (HOST-ONLY: every pixel reads black, so a render verdict means
nothing): `MODULE=gles3 GPU=v3d tools/cts/host-deqp-run.sh
dEQP-GLES3.functional.shaders.builtin_functions.precision.` at the fix
commit, VK-GL-CTS 93bca01, with a temporary log of every draw's target past
2048 (not committed). All 1,147 cases of S03-S08 began and ended, no chunk
stopped, and the back end refused no draw. The executor's targets were 4096 x
2, 4, 5 or 6 at 1 to 3 colour targets, single-sampled (1,035 draws). Per
image (QualityWarning is the black-pixel precision verdict):

| image | cases | Pass | QualityWarning | Fail |
|---|---:|---:|---:|---:|
| S03 | 192 | 24 | 168 | 0 |
| S04 | 201 | 32 | 169 | 0 |
| S05 | 202 | 10 | 174 | 18 |
| S06 | 204 | 24 | 162 | 18 |
| S07 | 206 | 32 | 174 | 0 |
| S08 | 142 | 16 | 108 | 18 |

The 54 Fail are one message, "Failed to set up framebuffer object:
glGetError() returned GL_INVALID_ENUM" (glFramebufferRenderbuffer, `GL error
0x500`): the matrix-output cases' fourth colour attachment, not the target
size; inv-enum-mrt of plan #376 has it. No ResourceError and no
`GL_OUT_OF_MEMORY` at framebuffer setup appeared in this run (inv-oom-fb).
