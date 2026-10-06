<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 2.0 CTS failure inventory

dEQP-GLES2, the mustpass list `tools/cts/gles2-mustpass.txt` (17,165 cases),
per board. Written 2026-10-07 for plan #1 (goal: zero failures on the AM62P,
the i.MX8MP and the Pi 4). Later plan items update this file; the rules for
that are at the end.

Every number below comes from what is committed: `docs/HANDOFF.md` (2026-10-06),
`tools/cts/STATUS.md`, `tools/cts/RESULTS.md`, `tools/cts/imx8mp-full-tree.txt`,
`tools/cts/imx8mp-full-tree-29460f96.results.tsv`, `tools/cts/pi4-full-tree.txt`,
`tools/cts/am62p-full-tree.txt`. Nothing was run for this file. The raw
records (`records/...`) are not in the repository (they are gitignored), so the
case names come from the prose of RESULTS.md and from the `.tsv`.

## Decision that is superseded

On 2026-09-25 (16:52) the owner left the 25 Pi 4 cube cases out of the "all
pass" count (RESULTS.md, "Reference-selection difference"). The goal of plan
#1 is zero failures. That decision no longer holds. The 25 cases are now to be
fixed (items `v3d-cube-design`, `v3d-cube-fix`).

## Summary

| Board | Last full tree | Date | Pass | Fail | NotSupported | QualityWarning |
|---|---|---|---:|---:|---:|---:|
| AM62P (PowerVR) | `d60b9d76` | 2026-10-02 | 17,141 | 0 | 19 | 5 |
| i.MX8MP (GC7000) | `d8583f20` | 2026-10-01 | 17,112 to 17,116 | 0 | 30 to 34 | 19 |
| Pi 4 (V3D) | `fe5e8e73` | 2026-10-01 | 17,075 | 25 | 62 | 3 |

Each row adds up to 17,165. The i.MX8MP range exists because the run covered
206 cases that are not in the mustpass list, and four of them sit in the
NotSupported group (see below).

Known regressions after the last full tree (not in the table above):

- i.MX8MP, on `72d16864` (a 12-row subset, not a full tree): 1 fail, 1
  NotSupported bucket.
- gl_test (our own suite, not dEQP) on the i.MX8MP: 44 to 46 of 48, with
  intermittent failures (see section E).

The AM62P and the Pi 4 have not run a full tree since their last one. Their
numbers are stale by up to 6 days of changes (section D).

## A. Per board: last full tree, counts, record names

### AM62P (PowerVR BXS-4-64)

- Commit `d60b9d76`, 2026-10-02. RESULTS.md section 1z2 ("AM62P whole tree at
  d60b9d76"). Earlier full trees: 1y (`db7a2085`, 10/1: 17,123 / 0 / 35 / 7),
  1x (9/30).
- Mustpass 17,165: **17,141 pass / 0 fail / 19 NotSupported / 5 warnings.**
  Every case has a result.
- Built by `tools/cts/am62p-deqp-images.sh` with `tools/cts/am62p-full-tree.txt`
  (17 filters, one image per boot) plus one extra image for `multisample.*`
  built with `DEQP_GL_CONFIG_NAME=rgba8888d24s8ms4`. The 17 images at the
  default 1x config give 17,125 / 0 / 35 / 5; the ms4 image replaces the 16
  multisample cases (16 NotSupported become 16 pass).
- Image record names (sha256 prefix): 1 `5c5c59d288249949`, 2
  `b2a5bb55e3da2f11`, 3 `93bbc9d8801bb71c`, 4 `839af33e68b9798b`, 5
  `89f94b612c72e08f`, 6 `8c294e0d4fca798a`, 7 `6ce8bbb2d5f2626c`, 8
  `942c4ca017832e31`, 9 `5491ea41f0ecf157`, 10 `eb1404696ffdfa44`, 11
  `d4bc52b096176a7d`, 12 `1c6fceb17d42f06b`, 13 `54787e490fc635ba`, 14
  `070c05e5c397e7ad`, 15 `ca7432975d5a399d`, 16 `12aa720106de6616`, 17
  `776ab68ec120a7ae`, ms4 `d11c011aad024954`. Image 5 was run twice (one
  result line was lost in the first capture; the tally flagged it).
- Nothing got worse against 1y. Two flush_finish timing warnings did not
  recur.
- Verified later on the board, not as a full tree: `05f8f749` gl_test 48 of 48
  (compiler-only and precompiled + compiler); `2c24de8`/`81c53c61` gl_test 48
  of 48. HANDOFF: "AM62P gl_test 48 / 48".

### i.MX8MP (Vivante GC7000)

- Commit `d8583f20`, 2026-10-01. RESULTS.md section "i.MX8MP: the whole
  dEQP-GLES2 tree on Zephyr at `d8583f20` (2026-10-01, gc-vsunits)". Image
  `wt-d8583f20`, zephyr.bin sha256 `3bf5391bcd3741c0`. Table: batch 1's
  `table-re0.conf`, entries 0 to 28 are the whole tree.
- 17,371 cases begun (206 of them are not in the mustpass list): 17,318 pass,
  0 fail, 34 NotSupported, 19 other (QualityWarning). On the mustpass list:
  **0 fail, 19 QualityWarning, 30 to 34 NotSupported, 17,112 to 17,116 pass.**
  The 34 and 17,112 are the likely values: the four non-mustpass cases in
  entry 28 passed.
- The per-entry results are committed in
  `tools/cts/imx8mp-full-tree-29460f96.results.tsv` (29 entries; it matches
  the `d8583f20` table). The entry list is `tools/cts/imx8mp-full-tree.txt`.
  I do not know what `29460f96` names; I take it as the baseline's own id.
- Zephyr, ES 2.0 reporting, default configuration. 0 resets, 0 timeouts.
- Subset re-run on `72d16864` (2026-10-06, 12 of 29 entries: shaders tail,
  texture, fbo, uniform_api; image `c2-72d16864`, `c3-shaders-72d16864`):
  `TALLY (12 entries): begun 4346 passed 4330 failed 1 notsup 1 other 14
  noresult 0`; 10 of 12 identical. Two regressions (section B). The records are
  in `records/imx8mp-tree/` (gitignored).
- gl_test on the board (not dEQP): `260711b9` 45 of 48 (first failure case 10,
  #9); `72d16864` 46 of 48 (case 16, #8); `05f8f749` 44, 45, 46 of 48 (case 2
  twice, case 16 once, #25); `112dd4fb` 46 of 48.

### Raspberry Pi 4 (V3D 4.2)

- Commit `fe5e8e73` (origin/main, "Merge v3d/loops-regression (f9c0060f) into
  land117"), 2026-10-01. RESULTS.md section "Pi 4: sweep 30". All 29 images
  built from one commit and run in one sweep.
- Mustpass 17,165: **17,075 pass / 25 fail / 62 NotSupported / 3
  QualityWarning.** Every case has a result. The result matched the prediction
  made before the build. Against the previous composed result only
  `debug_marker.{invalid,random,supported}` changed (NotSupported to pass).
- Images (`tools/cts/pi4-full-tree.txt`; sha256 prefix): AL `445f31570656ad5a`,
  D `7a3612176ed16711`, E `ca62eb25eb96d493`, F1 `32667791fd4eea38`, FB1
  `a380258756396c10`, FB2 `ea863374eb930aad`, FF `ab0b7b778847d50d`, H
  `32711a5b0583efc2`, LA `27eb7779abbefea1`, MISC `87f57a2f39ea94e5`, NS
  `d113b096519030cd`, RT `a5fb438687649fa1`, S1 `cf318d3a39d6debd`, S2
  `b1141f3f3e5e6fc6`, S3 `30623c610351e45c`, S4 `154bfd0d0a513261`, S5f
  `8f0f80dbcec704b6`, S5v `5c675b9a9aeb98a5`, SMALL `b5c19cfbb29e5caa`, T1
  `433be23d2a13f4d4`, T2 `ae03ba8c342121c4`, T3 `f169d78a82f0f017`, T4
  `1a4b78df5e031efd`, T5 `7e303b6c265bfcbd`, TF `17b60f718aad82ff`, U1
  `f3fa5be17dc12831`, VA `29c3a3f3b37901fa`, DR `77dc957e0988dc94`, XT
  `719ac80e9012779e`. The failing images are T1 (18 fail), T3 (6 fail) and TF
  (1 fail).
- Later checks that found no change against sweep 30 (RESULTS.md, 2026-10-01):
  the upstream-form check (3,480 cases) and the QPU FFI check (7,485 cases).
- gl_test: 12 of 12 on `afa12e9b`, not run since (HANDOFF).

## B. Every failing case, by name

Only the Pi 4 has dEQP fails in a full tree. The i.MX8MP has one fail and one
NotSupported bucket in the 12-row subset on `72d16864`. The AM62P has none.

Classes: driver, shader compiler, precompiled tables, OSAL-Zephyr, memory, test
harness, hardware-reference.

| # | Board | Case | Result | Root-cause class | Planned fix / owner |
|---|---|---|---|---|---|
| 1 | i.MX8MP (72d16864) | `dEQP-GLES2.functional.fbo.completeness.renderable.renderbuffer.depth.depth_component24` | Fail (fbo 711 became 710 pass) | driver (GL core: an ES 2.0 surface must refuse a DEPTH_COMPONENT24 renderbuffer; `72d16864` flipped the GC7000 defaults and the surface now accepts it) | `es2-gl-core` |
| 2 | i.MX8MP (72d16864) | `dEQP-GLES2.functional.shaders.fragdata.*`, one case (the name is not in the committed records; `es2fShaderFragDataTests.cpp:175`) | NotSupported ("Dynamic indexing of gl_FragData[] not supported"; shaders tail 1512 became 1511 pass + 1) | driver (GL core: GL_MAX_DRAW_BUFFERS is now 3 in an ES 2.0 context, so the test writes `gl_FragData[i]` and the compiler refuses it; the compiler's refusal is correct) | `es2-gl-core` (gl_MaxDrawBuffers = 1, no MAX_DRAW_BUFFERS query) |
| 3 | Pi 4 | `shaders.texture_functions.vertex.texturecubelod` | Fail (image TF) | hardware-reference | `v3d-cube-design`, then `v3d-cube-fix` |
| 4 to 9 | Pi 4 | `texture.mipmap.cube.{basic,bias,projected}.{linear_linear,linear_nearest}` (6; image T3) | Fail ("Image verification failed") | hardware-reference | `v3d-cube-design`, `v3d-cube-fix` |
| 10 to 19 | Pi 4 | `texture.vertex.cube.filtering.{linear_mipmap_linear_linear_clamp, linear_mipmap_linear_linear_mirror, linear_mipmap_linear_nearest_clamp, linear_mipmap_linear_nearest_mirror, nearest_mipmap_linear_linear_clamp, nearest_mipmap_linear_linear_mirror, nearest_mipmap_linear_linear_repeat, nearest_mipmap_nearest_linear_clamp, nearest_mipmap_nearest_linear_mirror, nearest_mipmap_nearest_linear_repeat}` (10; image T1) | Fail ("Image comparison failed") | hardware-reference | `v3d-cube-design`, `v3d-cube-fix` |
| 20 to 27 | Pi 4 | `texture.vertex.cube.wrap.{clamp_clamp, clamp_mirror, clamp_repeat, mirror_clamp, mirror_mirror, mirror_repeat, repeat_clamp, repeat_mirror}` (8; image T1) | Fail ("Image comparison failed") | hardware-reference | `v3d-cube-design`, `v3d-cube-fix` |

Rows 3 to 27 are the 25 Pi 4 failures (issue #12). Why they fail: the V3D 4.2
filters cube maps seamlessly and has no bit to turn that off. dEQP picks its
reference by context: a context that answers GL_MAJOR_VERSION without error is
judged against the seamless (ES 3) reference, any other against the
non-seamless ES 2.0 reference (`sampler.seamlessCubeMap = m_isES3Capable`,
es2fVertexTextureTests.cpp:1154). The same 25 cases pass under an
ES 3-reporting context. Mesa's v3d has no seamless control either. The plan's
fix is to emulate non-seamless sampling in the V3D compiler for ES 2.0 builds
and to prove it on the host with the QPU/TMU interpreter
(`src/shaderc/online/v3d/sim.rs`).

Names that failed earlier and are closed (kept so nobody reopens them): the
AM62P and i.MX8MP cube LOD-switch cases `texture.vertex.cube.filtering.
nearest_mipmap_{linear,nearest}_linear_{clamp,mirror,repeat}` (6 each, closed
2026-09-30 by the sampler-state shader variant, `YAKOGL_LOD_SWITCH_VARIANT`;
that variant is built for ES 2.0-reporting images). The Pi 4's first 19 cases
were once recorded as a hardware limit; the 6 of this family are inside the
25 above.

## C. NotSupported and QualityWarning buckets, with verdicts

A NotSupported result is not a dEQP failure. The goal says "zero failures", so
a bucket is acceptable when the hardware or the ES 2.0 profile does not offer
the feature. It is "to fix" when our code, not the profile, decides it.

### NotSupported

| Bucket | Cases | AM62P | i.MX8MP | Pi 4 | Verdict | Class |
|---|---:|---:|---:|---:|---|---|
| `negative_api.texture.compressedtexsubimage2d_{invalid_size, invalid_target, level_max_cube, level_max_tex2d, neg_level_cube, neg_level_tex2d, neg_offset, neg_wdt_hgt, offset_allowed}` | 9 | 9 | 9 | 9 | acceptable (the test needs a compressed format that allows sub-image updates; ES 2.0 core requires none, and the message is "No suitable compressed formats found") | driver (profile) |
| `clip_control.{depth_mode_one_to_one, depth_mode_zero_to_one, errors, face_culling, initial, modify_get, origin, viewport_bounds}` | 8 | 8 | 8 | 8 | acceptable (EXT_clip_control is an optional extension that we do not claim) | driver (profile) |
| `multisample.{common_edge_big_quad, common_edge_small_quads, common_edge_viewport_quad, constancy_alpha_to_coverage, constancy_both, constancy_both_inverted, constancy_sample_coverage, constancy_sample_coverage_inverted, depth, num_samples_line, num_samples_polygon, proportionality_alpha_to_coverage, proportionality_sample_coverage, proportionality_sample_coverage_inverted, sample_coverage_invert, stencil}` | 16 | 0 (16 pass in the ms4 image; 16 NotSupported at 1x) | 16 | 16 | acceptable (the default config has no multisample surface: "No multisample buffers"). On the AM62P the 4x image proves them. Optional follow-up on the i.MX8MP once 4x MSAA works (#8, `gc-msaa`). On the Pi 4 there is no 4x config yet and no item. | test harness (image config) and hardware (i.MX8MP #8) |
| `multisampled_render_to_texture.readpixels` | 1 | 1 | 1 | 1 | acceptable (EXT_multisampled_render_to_texture is not claimed) | driver (profile) |
| `shaders.fragdata.invalid_assign_to_1` | 1 | 1 | 0 (passed at `d8583f20`) | 1 | to fix: dEQP says "Test requires exactly one draw buffer"; an ES 2.0 context should report one. `gl_MaxDrawBuffers = 1` in `es2-gl-core` is expected to turn it into a pass on the AM62P and the Pi 4. Check it in the host/board recipe. | driver (GL core) |
| `fbo.completeness.attachment_combinations.<a>_<b>_<c>_tex`, a, b, c each one of none, rbo, tex (the 27 stencil-texture cases) | 27 | 0 (pass since a3856a06) | 0 (pass since f83a7361) | 27 | to verify: the Pi 4 sweep 30 had them NotSupported because the V3D did not claim the depth-texture extensions. STATUS.md (10/3) says the V3D now reads depth textures and claims GL_OES_depth_texture, so they probably pass now. Not run since. No plan item owns it; put it in the one board session (`docs-sync` recipe). If they still fail the stencil attachment, it is a V3D driver fix, outside this plan's list. | driver (V3D) |

Totals: AM62P 9 + 8 + 1 + 1 = 19. i.MX8MP 9 + 8 + 16 + 1 = 34 (the 25 of entry 28
plus negative_api 9; 30 if the four non-mustpass cases of entry 28 also count
there). Pi 4 8 + 27 + 16 + 1 + 9 + 1 = 62.

The sum of the AM62P table is 19 only on its ms4 composition. Without the
ms4 image the AM62P has 35.

### QualityWarning

dEQP counts QualityWarning as a pass (the process result is "Pass with a
quality warning"). All buckets below are acceptable.

| Bucket | Cases | Boards | Verdict | Class |
|---|---:|---|---|---|
| `texture.mipmap.cube.generate.{a8,l8,la88,rgb565,rgb888,rgba4444,rgba5551,rgba8888}_{fastest,nicest}` except `l8_nicest` and `la88_nicest` | 14 | i.MX8MP | acceptable (GC7000 behaviour: one corner pixel of a level-3 face reads a coarser level in the texture unit; NXP's own driver warns on the same 14 cases and faces; 8-bit chains are pixel-identical to it) | hardware-reference |
| `rasterization.interpolation.basic.{line_strip, line_strip_wide, lines}` and `.projected.{lines, lines_wide}` | 5 | AM62P (5), i.MX8MP (5) | acceptable (major-axis weights of lines; NXP's driver gives the same names on the narrow ones, images within 1 LSB) | hardware-reference |
| `rasterization.interpolation.basic.lines`, `.projected.lines`, `.projected.lines_wide` | 3 | Pi 4 | acceptable (same family: line interpolation; sweep 30 image H) | hardware-reference |

At 4x MSAA the AM62P also shows 9 CompatibilityWarnings in `rasterization` (wide
lines). That is a second configuration, not part of the full-tree count.
`flush_finish.{flush,flush_wait}` warned in the AM62P 1y run and not in 1z2
(dEQP's own timing check; it flips between runs).

## D. Commits since each board's last full tree that touch the GLES2 path

Important limit. This repository's history starts at `afb1633` (2026-10-04
21:09), a single import of everything up to then. The three full-tree commits
(`d60b9d76`, `d8583f20`, `fe5e8e73`) are not in it, so `git log <commit>..` does
not work. What can be listed is: (1) the 2026-10-04 import itself, which
holds all changes from each tree's date up to 10/4 in one piece; and (2) the
118 commits after it. Of those, 65 (non-merge) touch `src/`, `driver/` or
`zephyr/` (`git rev-list --count --no-merges HEAD -- :/src :/driver :/zephyr`
gives 66 with the import). Test-only changes under
`driver/tools/pvrlab/` (gl_test measurement cases) are collapsed at the end.

Window before the import (from STATUS.md, not from git): 10/2 the AM62P GPU
kernel side and display driver were moved to Rust (dEQP fbo, texture_functions
731 cases and fragment_ops.blend 1060 cases passed on the board); 10/3 the
GC7000 kernel side was moved to Rust (3 samples and 3 dEQP groups
byte-identical); 10/3 the Pi 4 got depth-texture reading, GL_OES_mapbuffer and
colour-less framebuffers on all GPUs (gl_test 169, 176, 177); 10/1 and 10/2
the i.MX8MP panel at 71.1 MHz. None of this was run as a full tree.

Risk key: H = changes what a dEQP case can see on that board; M = shared code
that could; L = tooling, logging, counters, or another board's back end.

### Shared by all three boards (GL core, shared compiler, OSAL)

| Commit | Subject | AM62P | i.MX8MP | Pi 4 |
|---|---|---|---|---|
| `afb1633` | YakoGL: OpenGL ES 2.0 + EGL for embedded GPUs, without an OS GPU driver (the import, 10/4) | M | M | M |
| `080b8aa` | Cargo: gpu-powervr, an explicit feature for the AM62P's PowerVR (touches src/gl draw, program, texture, egl) | M | M | M |
| `632687b` | shaderc: a variant refused for room is asked for again; slots are least recently used (src/gl/draw.rs +277: the sampler-state variants that closed the cube LOD cases) | H | H | H |
| `6688e60` | shaderc: a registered back end is kept; a shader is read as a table row only when it is one | M | M | M |
| `78c96ea` | Remove the remote shader back end | L | L | L |
| `3920be1` | Comments say what the code depends on, not how it was once measured | L | L | L |
| `78b5119` | depth/stencil: count the buffers given up for another target, on both GPUs | M | M | L |
| `2b219c5` | depth/stencil: 32 slots on every GPU, memory the real bound; the PowerVR gives up the least recently used | H | M | L |
| `a138f73` | log: every line a draw can reach is logged once per cause, the repeats counted on the close line (osal, pvr, v3d, vivante) | L | L | L |
| `068cb62` | draw steps: report each timed scope to the OSAL, which keeps the totals | L | L | L |
| `b0a9c0c` | draw steps: time each step of a draw and record the frame's slowest in CHECK_RUN | L | L | L |
| `3f0e1f5`, `cae0b2d` | cluster: GPU wait / block counters (osal) | L | L | L |
| `13161cb`, `f56fd15` | cluster: CHECK_RUN draw and compile-run counters (shaderc/online) | L | L | L |
| `68563f5` | shaderc: one source list for the tables, and a generator that precompiles it (no product change) | L | L | L |
| `dea95de` | shaderc: the Mesa reference leaves product code (tests/reference/, opt-in mesa-reference-table) | M | M | L |
| `84b5761` | shaderc: the in-process compiler is the default back end (Kconfig, CMake); the default-config samples move onto it | M | M | M |
| `17605c3` | shaderc: the precompiled tier in front of the compiler (Kconfig `YAKOGL_PRECOMPILED_SHADERS`, default off) | M | M | M |
| `17e7d34` | shaderc: an image without the compiler; a miss refused by name (`YAKOGL_SHADERC_PRECOMPILED_ONLY`, default off) | L | L | L |
| `3c8a3fa` | zephyr: the keep region defaults to 1 MiB, the size the cluster ran in (zephyr/, slint and cluster boards) | L | L | L |
| `990dbd3` | check.sh: one Zephyr gl_test image per GPU with -Werror; 142, 145, 148 dispatched on every GPU | L | L | L |
| `284571d` | samples: the products report OpenGL ES 2.0, the conformance-tested surface | L | L | L |

Risks worth the host check (`compiler-regression-host`, `deqp-images-current`):
`632687b` (the variant slots are what makes the cube LOD cases pass on two
boards, and the Pi 4 uses variants too), `84b5761` and `17605c3` (the default
back end moved; a dEQP image must still pick the compiler and ES 2.0
reporting), `2b219c5` (32 depth/stencil slots on every GPU; memory limits of
the 2048x2048 and fbo images).

### AM62P only (PowerVR back end and compiler)

| Commit | Subject | Risk |
|---|---|---|
| `b67c89b` | pvr: PVR_BACKEND_OPT_FS_TEMPS_MIN, a floor on the fragment kick's temps (a measurement; default 0) | L |
| `92939b0` | pvr: every PDS program generator's refusal refuses the draw by name | M (a draw that rendered before may now be refused) |
| `109b38b` | pvr check: the kick USC program's DOUTU word carries the fragment temps | M |
| `6f96104`, `18972dc` | pvr online: fuse a * b + c into one fmad; fusion is how the compiler works, a negated immediate factor keeps its sign | H (arithmetic results; dEQP shaders.* compares values) |
| `f4c41d0` | powervr: a clamp to [0, 1] is one saturating add, as pco builds fsat | H |
| `afa12e9` | powervr: an if with arithmetic arms runs its arms predicated, as pco does | H |
| `ecd341d` | pvr online: x * 1.0 reads x, copies forwarded to their readers | H |
| `7010bd2` | pvr online blend: a factor the same in every channel computed once | M |
| `39bd698` | pvr online: a product of a fragment shader's varying fuses with its sum | H |
| `81c53c6` | pvr online: a predicated if's arms write one register, no select after them | H |
| `97ba03d`, `5f4d367` | pvr online: a predicable if can run its arms under a branch; the default branches only when skipping either arm pays | M (0 of 168 corpus ifs branch) |
| `0fe778f` | shaderc table: blobs.rs records the toolchain, matattr's 20 rows | L |
| `05f8f74` | shaderc: the back ends' own programs come from our compiler (frame-load, masked-clear, clear-attach) | H (every render on this board) |

Host evidence already recorded for the AM62P changes: gl_test 48 of 48 on
`81c53c61`, `5f4d3677` and `05f8f749` (HANDOFF, STATUS.md), and the full gate
(96 ok / 0 failed / 0 gaps at `81c53c61`). A dEQP tree has not run on any of
them. The compiler-side changes are the main risk for dEQP's `shaders.*`
(the largest group of the mustpass list).

### i.MX8MP only (GC7000 back end and compiler)

| Commit | Subject | Risk |
|---|---|---|
| `dc80eef` | GC7000 online: select gl_VertexID and gl_InstanceID | L (ES 3) |
| `f36c176` | GC7000 online: texelFetch, textureSize and 3D / 2D array look-ups | L (ES 3) |
| `0670e85` | GC7000 online: derivatives and `%` as the reference and Mesa emit them | M |
| `ba88a0d` | vivante: a constant register's padding lanes are unused, not uniform dwords past the block | M (uniform_api, constants) |
| `260711b` | vivante: the debug knobs leave the product (4 removed, 5 hard-coded, 4 behind test-knobs) | M (gl_test 45 of 48 on the board; case 10 is #9) |
| `72d1686` | vivante: MRT, the vertex-stage tiled copy, INT_FILTER, the narrow L8 store and Mesa's descriptor order are the design; the last product knobs go (src/gl context, framebuffer, texture) | **H, the source of the two known regressions (section B rows 1 and 2)** |
| `05f8f74` (the clear pair part) | shaderc: the back ends' own programs come from our compiler | M (gl_test case 2 first failed here; host streams are identical, #25) |
| `c1f0e2a` | imx8mp display: back to 69.3 MHz, a checked DSI margin (driver/rust, driver/src/display) | L (display, not GL) |

### Pi 4 only (V3D)

`src/v3d` is untouched after the import except for the shared logging change
`a138f73`. The Pi 4's exposure is through the shared GL core, the shared
compiler front end, the depth/stencil slot count and the sample/Kconfig
defaults above. It was last run on `afa12e9b` (gl_test 12 of 12) and not
since. The Pi 4's risk is therefore mostly unknown, not high: that is why the
`compiler-regression-host` and `deqp-images-current` items matter for it.
The 72d16864 GL-core change (a context reporting `GL_MAX_DRAW_BUFFERS` 3 in
an ES 2.0 context and accepting a DEPTH_COMPONENT24 renderbuffer) may apply
to all three boards, not only the i.MX8MP, because it is in `src/gl`. Check
this on the host with `es2-gl-core`.

### Tooling and tests (no GLES2 path risk)

`e86958e`, `72f06c2`, `c1ca07b`, `45241db`, `45ba662` (boot and sample
build settings); `fc57f64`, `013ef3c`, `974af52`, `c65c931` (record kinds,
CHECK_RUN); `452541b`, `4dfc66d`, `215a228`, `34f4e57`, `635f1ff`, `5815747`,
`6314015`, `58402fd`, `01260ee`, `39a5dc3`, `a042565`, `bca6c4d`
(gl_test measurement cases in `driver/tools/pvrlab/` and gl_test sources).
`eaad3aa`, `0af3619` and the docs commits touch only `tools/` and `docs/`.

## E. gl_test known defects and why they matter for CTS runs

These are failures of our own suite (gl_test), not dEQP cases. They matter
because one board run must not be flaky if the goal is "zero failures" in a
repeatable way.

| Issue | What | Boards | Why it matters for CTS | Planned item |
|---|---|---|---|---|
| #9 | gl_test case 10 (`prims`) fails about 1 run in 3. Check 23 (frame 5, the 1 px point) flips even when run alone on Zephyr: 2 of 4 at `2362422d`, 2 of 7 at `04a0e2c2`. The cause is not found. | i.MX8MP | Missing colour writes can flip any dEQP rasterization, fragment_ops or draw case on the GC7000 from pass to fail between runs. The 10/1 full tree had 0 fail, so the rate in dEQP is low, but a "zero failures" claim needs repeat runs. | `gc-nondeterminism` |
| #25 | gl_test case 2 (`texture`) fails intermittently since `05f8f749`: 44, 45, 46 of 48 on the same image (first failure case 2, 2, 16); `112dd4fb` 46 of 48. The command streams of 24 `cs_ours` modes are byte-identical between `72d16864` and `05f8f749`. | i.MX8MP | Same class as #9 (not a deterministic regression). It touches texture sampling, so it could hit the `texture.*` cases (807 in RESULTS.md). | `gc-nondeterminism` |
| #8 | gl_test case 16 (4x MSAA) fails (check 255, first failure at `72d16864` and `81c53c61`). | i.MX8MP | Not in the default 1x mustpass count. The 16 `multisample.*` cases are NotSupported there; they can only run once 4x works (as on the AM62P's ms4 image). Also it sets "46 of 48" as the best gl_test result, so the case-2 and case-10 noise sits on top of it. | `gc-msaa` |
| #12 | The Pi 4 differs from the ES 2.0 reference in the 25 cube cases (section B). | Pi 4 | These are the 25 CTS failures. | `v3d-cube-design`, `v3d-cube-fix` |
| #22 | The GC7000 precompiled table lags on `matattr`. | i.MX8MP | Only matters for images that use the precompiled tier. The dEQP images use the compiler. No CTS case is known to hit it. | outside this plan |
| #23 | The i.MX8MP panel showed one and a half copies of the frame at 71.1 MHz; the device tree is back at 69.3 MHz (`e372545f`/`c1f0e2a6`). The picture is not yet viewed by eye. | i.MX8MP | Display only. dEQP reads pixels back and does not use the panel. | none (needs a person at the panel) |

## F. Plan items and what each one covers

| Item | Covers |
|---|---|
| `es2-gl-core` | Section B rows 1 and 2 (i.MX8MP regressions of `72d16864`); `shaders.fragdata.invalid_assign_to_1` on the AM62P and the Pi 4 (section C). Wanted: ES 2.0 context refuses DEPTH_COMPONENT24 renderbuffers, has no `GL_MAX_DRAW_BUFFERS` / `GL_DRAW_BUFFERi` query, `gl_MaxDrawBuffers` = 1. |
| `v3d-cube-design`, `v3d-cube-fix` | The 25 Pi 4 cube failures (section B rows 3 to 27). Design plus a failing host test, then the implementation, both on the QPU/TMU interpreter. |
| `gc-nondeterminism` | #9 and #25. Fix only with host proof; otherwise write an exact diagnostic recipe. |
| `gc-msaa` | #8 and, later, the i.MX8MP `multisample.*` NotSupported bucket. |
| `compiler-regression-host` | Section D compiler commits, checked on the host for ES 2.0 regressions (all three boards, the AM62P most). |
| `deqp-images-current` | The dEQP image configurations still match the current defaults (`84b5761`, `17605c3`, `2b219c5`, `3c8a3fa`); CTS tooling self-tests. Also the Pi 4 fbo 27 and `multisample` image configs. |
| `docs-sync` | Merges every outcome into this file, HANDOFF, `tools/cts/STATUS.md` and RESULTS.md; writes the one board verification recipe. |

## G. What is missing, and how to update this file

Missing from the repository (so not in this file):

- The raw dEQP result files of all three trees. The i.MX8MP `.tsv` is the
  only committed per-entry result file.
- The exact name of the NotSupported case in `shaders.fragdata.*` on `72d16864`
  (it is in `records/imx8mp-tree/c3-shaders-72d16864`, gitignored).
- Full trees at a current commit for the AM62P and the Pi 4. The first board
  session after the fixes should run all three.
- A full-tree run on the i.MX8MP that repeats the same image several times to
  measure how often #9 and #25 flip a dEQP case.

Update rules for later items:

- Change a number only with a record name and a commit. Keep the old value in
  the same cell as "was".
- When a case moves, change its row in section B or C and its planned item.
- Keep section D by commit: when a commit is checked on the host, add the
  evidence (test name, result) in a new column. Do not delete rows.
- When a full tree runs on a board, replace that board's row in the summary
  table and in section A.
