<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D targets past 2048: the cause of the Pi 4 precision-group abort

Issue #1261. Images S03-S08 of `tools/cts/pi4-gles3-tree.txt` (192 / 201 /
202 / 204 / 206 / 142 cases, 1,147 in all) each stop at their first
`shaders.builtin_functions.precision.<fn>.highp_fragment.scalar` case (S03
`abs`, S04 `cosh`, S05 `fract`, S06 `mix`, S07 `reflect`, S08 `sub`). 1,142
mustpass cases never began. This note records the cause as far as the code and
the records show it; the fix (commit `6d2e7d0d`, plan #376 item fix-v3d) is
the last section.

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
- `docs/notes/v3d-refusals.md` row A5 said this refusal fired 0 times; it
  now names the cause and the fix.

## Possible fixes (fix-v3d's)

- Size the tile memory for 4096 (raise `MAX_TARGET`; `MAX_JOB_TILES` is a
  square, so it would grow 4x, while a 4096 x 6 target bins far fewer tiles:
  the cost is for tile-cost to measure).
- Render a large target in bands (several jobs, each within the tile memory).
- Report smaller limits (2048 for renderbuffer, texture and viewport), which
  is plan #375's ground (RC10 limits) and would refuse textures and
  attachments the V3D samples today.

## After the fix (fix-v3d, plan #376)

The first option was taken: the tile memory is sized for 4096 a side
(`6d2e7d0d`). Line numbers below are those of `src/v3d/backend.rs` at
`4cc135c1`.

- `MAX_TARGET` is 4096 (line 440), the side the GL core reports for
  `GL_MAX_RENDERBUFFER_SIZE`, `GL_MAX_TEXTURE_SIZE` and `GL_MAX_VIEWPORT_DIMS`
  (`src/gl` is unchanged). `MAX_JOB_TILES` (line 446) follows it: 128 x 128 =
  16,384 tiles, the 32x32 tiles of four 32-bpp RTs over 4096 square.
- At init the binner's tile pool and the tile state are sized from that
  tiling (line 5915): tile alloc 2,105,344 bytes (was 532,480 at 2048: 64 x
  64 x 128 + 8,192), tile state 4,194,304 (was 1,048,576), about 4.5 MiB more
  of the carve-out, allocated once. A kick names and zeroes only the tile
  state of its own tiles (`a_kick_zeroes_the_tile_state_of_its_own_tiles`),
  so a small target costs per job what it did; the board cost is not
  measured.
- The draw path names each cause (lines 6294, 6297): "an empty target (0
  pixels on a side)" (`refuse`, `-EINVAL`, a malformed draw) and "a target
  larger than the tile memory is sized for (4096x4096)"
  (`refuse_unsupported`, `-ENOTSUP`). The GL core answers a side past 4096
  with `GL_INVALID_VALUE` at the storage, so no GL call reaches the second
  one: it is a guard.
- The typed (64/128-bpp) job keeps its own tile-count check (line 2689, row
  A1): 4096 square at two RTs of a 128-bpp format bins 32,768 tiles and is
  refused by that name; 2048 square of the same draws.
- `rt_texture` (line 1879) takes a render image up to 4096 a side.

What the executor's targets take now: 4096 x 2-6 at 1-3 RTs is one row of
64x64 to 16x16 tiles (64 to 256 tiles, 128-bpp outputs included), one job
the size of the target, far under the 16,384 the pools hold.

Host tests (feature sets in their headers): `tests/es3_large_target_v3d.rs`
(4096 x 4, 4 x 4096 and 4096 x 6 cleared, drawn and read back; 4097 is
`GL_INVALID_VALUE`; past 4096 and empty targets forced into the back end are
refused by their own names); `src/v3d/backend_tests.rs`
`a_target_of_4096_a_side_is_one_job` and the re-pinned
`a_backend_gap_is_enotsup_and_counted`;
`tests/es3_v3d_refusal_gl.rs` `a_4096_target_is_no_longer_refused`;
`tests/es3_rb_formats_v3d.rs` `a_draw_at_the_largest_size_is_taken` and
`the_4096_edge_in_each_dimension`; `tests/v3d_es2ctx_refusals.rs`
`a_4096_target_is_accepted`, no longer `#[ignore]`d.

Host run (HOST-ONLY: every pixel reads black, so a render verdict means
nothing), from the first version of this fix (`6d5e59d6`, closed unmerged:
the same 4096 side, with the pools kept at 2048 square and limited by tile
count; the executor's targets bin 64 to 256 tiles under both):
`MODULE=gles3 GPU=v3d tools/cts/host-deqp-run.sh
dEQP-GLES3.functional.shaders.builtin_functions.precision.`, VK-GL-CTS
93bca01. All 1,147 cases of S03-S08 began and ended, no chunk stopped, and
the back end refused no draw. The executor's targets were 4096 x 2, 4, 5 or
6 at 1 to 3 colour targets, single-sampled (1,035 draws). Per image
(QualityWarning is the black-pixel precision verdict):

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
size. The run has not been repeated at `6d2e7d0d`'s pool sizing; the board
session after the fix (plan #376 item 3) is the check.
