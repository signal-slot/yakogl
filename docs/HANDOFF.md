<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Handoff

The state of the work and what comes next. Update this file in the same
commit as any result that changes it. Last update: 2026-10-10.

Board addresses, UART paths and credentials never go in this file; they come
from the environment (`BOARD_HOST`, `BOARD_UART`, ...).

## Goal, in order

1. ES 2.0 finished on the three boards (AM62P PowerVR, i.MX8MP GC7000,
   Raspberry Pi 4 V3D), each booted from its own storage by its bootloader.
2. Minimum performance.
3. A public website (`website/`). It goes live when the repository is public:
   `.github/workflows/pages.yml` deploys it once Pages is turned on
   (`website/README.md`). Every figure it shows is in `website/data.js` with
   its commit, date and record; update that file with each new board result.
   `tools/website-check.sh` checks links, metadata and `data.js`.
4. ES 3.0 on the three boards (the dEQP-GLES3 mustpass, 42,810 cases). Host
   work runs ahead of the boards. A product image still reports ES 2.0 by
   default (Kconfig `YAKOGL_REPORT_ES2` default `y`, CMake
   `YAKOGL_REPORT_ES2=ON`); ES 3.0 is what an image that turns that off gets,
   and what the cargo crate reports without the `report-es2` feature.
5. ES 3.1 / 3.2 (#19).

## State of main

**Plan #1 (ES 2.0 CTS, zero failures on the three boards), 2026-10-07; board
readings 2026-10-08.** Every fix below is proved on the host. No image built
from main has run them, and no whole dEQP tree has run since the last full
trees (AM62P `d60b9d76` 10/2: 17,141 pass / 0 fail / 19 NotSupported / 5
warnings; i.MX8MP `d8583f20` 10/1: 0 fail; Pi 4 `fe5e8e73` 10/1: 25 fail, all
cube). Two board sessions ran images of the ccfarm integration branch that
contain these fixes: the i.MX8MP at `b20e4d11` (steps 1, 2, 3 and 5 of "Next
board session", record `tools/cts/log/board-sessions/imx8mp-plan1.md`) and the AM62P at
`b83216d4` (images 1 to 7, record `tools/cts/log/board-sessions/am62p-plan1.md`). They are
readings, not landing proofs; the main sessions below repeat them. The Pi 4
has run nothing since `fe5e8e73`. The inventory with every failure's state
and commit is `docs/es2-cts-failures.md`; it does not have the readings yet.

**Update, 2026-10-10:** main `32778b57` has now run on the AM62P (whole tree,
ms4 image, gl_test) and the i.MX8MP (whole tree, gl_test), and `e17fb6a7` on
the AM62P's image 1; the rows below named `32778b57` and `e17fb6a7` carry the
numbers, and the sentences above that say no image from main has run, or that
`d60b9d76` and `d8583f20` are the last trees, are superseded for those two
boards. The Pi 4 then ran the whole tree on main `a27d69c0` (2026-10-10,
`records/pi4/deqp-a27d69c0/full/`): 17,083 pass / 9 fail / 34 NotSupported,
the 9 being cube cases fixed on the host since (#12 row).

| Failure | State on main | Commit |
|---|---|---|
| Pi 4, 25 cube cases (#12): 25 -> 9 on the board, 9 -> 0 on the host | non-seamless cube sampling in ES 2.0 builds (a cube look-up is a 2D-array look-up of the selected face; the V3D has no switch for seamless filtering). The owner's 2026-09-25 exclusion of these cases is replaced by this fix. Design: `docs/v3d-noseamless-cube.md`. **Pi 4 whole tree on main `a27d69c0`** (2026-10-10, `records/pi4/deqp-a27d69c0/full/`, tally `build/pi4-tally-a27d69c0.txt`): 17,083 pass / 9 fail / 34 NotSupported; 19 of the 25 pass (the 18 `texture.vertex.cube` and `texturecubelod`). The 9: T3 `texture.mipmap.cube.{basic,bias,projected}.{linear_linear,linear_nearest}` ("Image verification failed") and T4 `texture.size.cube.15x15_{l8,rgb888,rgba8888}` ("Image comparison failed"; they passed at sweep 30, before the emulation keyed their min LINEAR; `15x15_rgba4444` passes). One cause: a fragment look-up's level came from the TMU's differences of the per-face (s, t), meaningless in a quad that straddles a face edge, as every edge quad of dEQP's face-filling draws does through its helper pixels (the smallest level on the cell edges; a LINEAR minification on the 15x15 face's edge row, up to 8.43 / 255 against an 8-bit threshold of 8). Fixed on the host: the fragment variant computes dEQP's cube level from FDX / FDY of the direction and the face size word and retires with TMUSLOD; the job builder now fills `QUNIFORM_TEXTURE_WIDTH` / `_HEIGHT` (which also ends textureSize's refusal on the V3D). Host tests `tests/v3d_cube_noseamless.rs` (8), `job_check.c` part 5, backend test. Board confirmation pending: Pi 4 images T3 and T4 (rows 21, 22), with T2, TF, RT, S5f unchanged ("Next board session") | `c3e4146` design and host reproduction, `ec5ba4f` implementation, `53ef6d7`, `d8ce931`; fragment level (this commit) |
| i.MX8MP, the two dEQP regressions of `72d16864` (fbo `depth_component24`; one `shaders.fragdata.*` NotSupported) | fixed on the host: in `report-es2` builds `glRenderbufferStorage(GL_DEPTH_COMPONENT24)` raises GL_INVALID_ENUM unless `GL_OES_depth24` is listed (`state::extension_listed`); `GL_MAX_DRAW_BUFFERS` and `GL_DRAW_BUFFERi` are refused and `gl_MaxDrawBuffers` is 1. On the integration images: the i.MX8MP shaders-tail and fbo rows are back at the `29460f96` numbers (1512 / 1512 / 0 and 711 / 711 / 0), and on the AM62P `fragdata.invalid_assign_to_1` moved from NotSupported to Pass, as predicted. Not done: other ES 3-only renderbuffer formats such as `GL_DEPTH_COMPONENT32F` are still accepted in ES 2.0 builds (a static audit lists 44 ungated ES 3.0 rows, none reached by a dEQP-GLES2 case: `tools/cts/log/investigations/es2-error-rules.md`) | `4e9397c` |
| i.MX8MP gl_test case 16 (#8) | harness fixed: it failed in the default image only because that image offers no 4x config (`check 255` is the record's none-marker, not a check). Case 16 runs its 1x half and skips the 4x half by name without a 4x config. The driver did not scale viewport, scissor and clip to the 2x scratch surface: fixed behind `gc_debug_msaa` bit 3, off by default. On the integration image case 16 alone is the by-name skip. The set walker in `gl_test.c` counted any non-zero return (the skip's -3) as a failure, so the default set recorded 46 / 48 and on `c4ccc966` 47 / 49 with case 16 first and the other failure unnamed; it now counts a skip as skipped (not run, not failed), and the set's record (CASE status `CASE_SET`, `record.rs` `SetSummary`) carries passed of run, the skip count and first skip, and the first two failing cases; the bit 3 A/B failed in both arms at check 4 (`x4 inside` got 0x00000000), so bit 3 is not the fix and #8 stays open | `6bddfc5` |
| i.MX8MP gl_test cases 10 and 2 (#9, #25) | not fixed. Reading our draw, resolve and read-back sync against etnaviv's finds no missing stall, flush or cache operation (`src/vivante/VIVANTE_NOTES.md`; two host tests pin it). The product is unchanged. The recipe's repeat runs on the integration image (60 boots): base 4 / 40, page-aligned 1 / 10, no-reuse 2 / 10; a second session (`0175bac1`, `tools/cts/log/board-sessions/imx8mp-gc-case10.md`): case 10 1 of 30, case 2 0 of 40. The rate fell from 14 / 30 and no arm removes the failure, so the three hypotheses are not separated | `8409419` |
| i.MX8MP gl_test case 44 `es3hdr`, RGBA16F render target (#6) | fixed on the host, board run pending. On main `a0f439d6` the default set reads 47 / 48 run, 1 skipped (16), first failure case 44 (`records/imx8mp/20261010-1623-gt-a0f439d6-44175f39`; no console, so not which check). Two causes, both against etnaviv: (1) `PE_COLOR_FORMAT` for a float target set bit 31 (FORMAT_EXT's own write mask, so the PE kept its old FORMAT_EXT) where etnaviv sets bit 4, FORMAT's mask (`etnaviv_state.c:282-283`, `state_3d.xml.h` `VIVS_PE_COLOR_FORMAT_FORMAT_MASK 0x10`): every RGBA16F/R16F/R32F target was drawn in a stale format; (2) the texture unit misreads a LINEAR texture of 64-bit texels at a 64-byte-aligned base (issue #6's probes), a layout Mesa never samples: RGBA16F textures are now sampled from a 4x4-tiled copy at a page boundary (`tiled_copy.rs` `wide_texels`), as Mesa's TILED, page-aligned buffers are. Issue #6's third item (check 7 read `0x40042814`, texel (5,0) where (7,0) is wanted) was measured with only the probes' page alignment; whether it outlives the tiled copy is what the board run says. Host tests in `src/vivante/backend_tests.rs` (`a_float_target_goes_through_the_extended_format_field`, `the_pe_color_format_bits_are_the_rnndb_headers`, `an_rgba16f_texture_is_sampled_tiled_from_a_page`, `a_render_into_an_rgba16f_texture_reaches_the_draw_that_samples_it`). Board: `tools/cts/imx8mp-gltest-build.sh gc44-<head8> 44 -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y`, run with `SEL=0`, expect a CASE pass record for 44 | (this commit) |
| PowerVR vertex attribute registers | wrong-code defect found by the host compiler check and fixed: an attribute read only where the unroller drops code took no registers, and every later attribute was read four registers early. No dEQP-GLES2 case is known to reach it; it changes AM62P vertex code, so the AM62P tree is re-run. On the integration image six of the seven shaders images equal `d60b9d76` | `c6aa6ba` |
| AM62P, 9 `shaders.invariance.{lowp,mediump,highp}.common_subexpression_{1,2,3}` cases | fixed (`e17fb6a7`) and confirmed on the board: image 1 of the AM62P tree on `e17fb6a7` passes 1,379 / 0 (2026-10-10, `records/am62p/deqp-e17fb6a7/img1`); the PowerVR online compiler fused `a * b + c` into one `fmad` only where the product had one reader, and the shader that also wrote `v_unrelated` gave the position's products a second reader, so one shader of each pair rounded the position once where the other rounded it twice. A sum on the way to an `invariant` output (`invariant gl_Position;`, `invariant varying`, `#pragma STDGL invariant(all)`, now carried on `ir::IoDecl::invariant`) is no longer fused, as Mesa's exact `fadd` is not `contract` (`powervr/prepare.rs` `mark_exact`). On the host the 9 pairs went from differing to identical position code (case 0 was identical before and after); no corpus source changes. Expect the 9 to pass in image 1 | (this commit) |
| AM62P, the 45 `texture.vertex.2d.{filtering,wrap}.*` cases | **regression of the full tree at `32778b57`** (2026-10-10, image 9, `records/am62p/deqp-32778b57/full/`): "Image comparison failed", every draw refused, not a sampling fault. dEQP's grid is 127 x 127 quads of four client vertices (two vec2 arrays, 516,128 bytes each) and 96,774 client u16 indices: 1,225,804 bytes a draw, which the PowerVR copies into the frame's GENERAL arena. `dcaafa41` made that arena 1 MiB (was 4 MiB at `d60b9d76`), so each draw logged `does not fit an empty frame: the general arena (1048576 bytes) is too small; not drawn`. Fixed on the host: an indexed draw whose client indices and arrays are over half an arena is cut into pieces of its index list, each copying its indices and only the vertex window they read (`draw_indexed_pieces`; the cube cases, 317,520 + 47,628 bytes, never reached the limit). Host tests in `src/pvr/backend_tests.rs`; gl_test 197 `vtxgrid` draws the grid four times with a mipmapped 2D texture in the VS. Board confirmation pending: image 9 of the AM62P tree | `a6fc6f14` |
| AM62P, whole dEQP-GLES2 tree on main `32778b57` | measured 2026-10-10 (`records/am62p/deqp-32778b57/full/`, 17 images, 0 resets, every case with a result): mustpass 17,070 pass / 54 fail / 34 NotSupported / 5 QualityWarning / 2 CompatibilityWarning (`flush_finish.flush`, `flush_wait`). The 54 are the 9 invariance and the 45 vertex-texture cases of the two rows above; no other case failed. NotSupported 34: `clip_control` 8, `negative_api.texture.compressedtexsubimage2d_*` 9, `multisampled_render_to_texture.readpixels` 1, `multisample.*` 16 (1x config; measured on the ms4 image). `fragdata.invalid_assign_to_1` passes. Against `d60b9d76` (17,141 / 0 / 19 / 5) 56 cases are worse counting the ms4 image's 2 (next row); `tools/cts/RESULTS.md`, "AM62P: the whole dEQP-GLES2 tree on origin/main `32778b57`" | `32778b57` |
| i.MX8MP, whole dEQP-GLES2 tree on main `32778b57` | measured 2026-10-10 (`records/imx8mp/deqp-32778b57/full`, 29 selectors): 17,371 begun, 17,318 pass / 0 fail / 34 NotSupported / 19 QualityWarning / 0 no result. Every selector equals `d8583f20`'s (`tools/cts/imx8mp-full-tree-29460f96.results.tsv`, 2026-10-01); entry 21 `fbo` is 711 / 711 and `fragdata.invalid_assign_to_1` passes, so the two regressions of `72d16864` are fixed on the board | `32778b57` |
| AM62P 4x image, `multisample.constancy_alpha_to_coverage` and `constancy_both_inverted` | **regression of the `ms4` image at `32778b57`** (2026-10-10, 14 / 2, deterministic: `records/am62p/deqp-32778b57/ms4/` and `ms4-rerun/`; 16 / 16 at `d60b9d76`). Not the coverage: the sample-kill variant and its epilogue are the same words at both commits and on every draw, and kill by alpha alone in the byte model; the GL streams of the two commits are equal. The frame: each case draws 1024 (or 1020) triangles into one 4x frame, a green quad and the red one over it per cell, and each draw uploaded six PDS blocks (768 bytes: vertex data and code, the fragment kick, the coefficients, the control word's DMA data and code). `dcaafa41` made the PDS arena 768 KiB (1 MiB at `d60b9d76`), so the frame held 1014 draws and the 1015th, the red quad over cell 253's green one, opened a second kick (Rig and host harness), and the split's next kick loaded the resolved colour into all four samples: green stayed where the red quad's coverage leaves samples out (`constancy_both` and `sample_coverage` have 768 draws; `sample_coverage_inverted` splits at the same draw, in the cell whose inverted coverage is all four samples, so nothing is lost there). Fixed on the host: a frame uploads a PDS program it already holds once (`pds_put`, the words compared with the arena below its bump), two blocks a draw after the first, as Mesa keeps those programs with the pipeline; the 1024 draws are one kick. Tests: `a_4x_frame_of_deqps_1024_multisample_draws_is_one_kick`, `a_pds_program_the_frame_already_holds_is_pointed_at_not_uploaded` (`src/pvr/backend_tests.rs`), `deqps_multisample_shader_keeps_samples_by_its_alpha_alone` (`tests/usc_limits.rs`), `sample_coverage_invert_is_the_masks_complement_at_deqps_values`, `every_alpha_to_coverage_draw_runs_the_sample_kill_variant_from_the_first` (`src/gl/draw.rs`); gl_test 198 `a2cconst` draws the picture twice. The split itself still loses samples (known defects, pvr-msaa-split). **Confirmed on the board:** the `ms4` image on `452d6c31`, 16 / 16 with both constancy cases (`records/am62p/deqp-452d6c31/ms4/`) | `a4f64950` |
| AM62P gl_test 198 `a2cconst` (the same picture on a 64x64 4x pbuffer) | **hung the fragment DM** on `452d6c31` (2026-10-10, `records/am62p/20261010-1515-c196-198-452d6c31-e85a4597`, cases 196,198): both pictures, `FWCCB: CONTEXT_RESET reason=1 dm=3` twice, case TIMEOUT, read-backs zero. Decoded (DDK `rgx_fwif_shared.h`, `rgx_fwif_km.h`, the trace through `driver/tools/sfdecode`): reason 1 GUILTY_LOCKUP, HWR type 0 UNKNOWNFAILURE (no page fault: the `BIF write VA=0` words are empty), rflags 0x20 DM_STATE_GUILTY_LOCKUP, `EVENT_STATUS` 0x00100000 the Rogue XE safety bit every HWR record shows; trace: "DM3 timed out (phase count=0)", "USC slots: 0 used by DM3, 64 empty". A 3D stall with no USC work. Not the split: the host harness (online compiler, l0) gives three kicks, the depth clear and one per picture, 1024 draws each; HWRT (rgn 64, tpc 512) and ZLS extents are Mesa's at 64x64 x4; the PDS sharing is the dEQP image's. The kick's one self-contradiction: CR_ISP_CTL let four 4x tiles be in flight from a 2-register pixel while CR_USC_PIXEL_OUTPUT_CTRL says 4 registers, so four partitions of 4096 dwords against the 8192 reserved; dEQP's picture over 256x256 has 4 objects a 16x16 tile, case 198's 64. Fixed on the host: the tiles in flight come from the same pixel width (Mesa `pvr_arch_setup_tiles_in_flight`), two at 4x (ISP_CTL 0x80031000; one sample unchanged, 0x80035000). Test: `a_4x_kicks_tiles_in_flight_fit_the_partitions_its_pixel_width_takes`. Board confirmation pending: gl_test 198 alone; if it still hangs, the next arms are case 198 at 256x256 and without the depth load | (this commit) |
| dEQP image recipes | checked against the current defaults on the host (`tools/cts/check-deqp-images.py`, 206 `board_selftest` checks, fixtures in `tools/cts/testdata/deqp-config`); the AM62P 4x image has its own table `tools/cts/am62p-ms4-tree.txt` | `e6e5f14`, `292310c`, `688a574` |

The compiler check of `c6aa6ba` ran the PowerVR and GC7000 back ends against
the IR on dEQP-shaped generated shaders and dEQP's 1,060 blend cases
(`tests/common/deqp_style.rs`, `tests/backend_differential.rs`,
`tests/usc_limits.rs`). No CTS checkout, `ZEPHYR_WS`, `HAL_NXP_DIR`,
`PCO_SHC` or `GLSLANG` was available to that item, so
`tools/cts/host-deqp-run.sh` is not a committed record: the commit message
reports a run of it (17,165 of 17,165 cases began on both back ends, 877
compile refusals, all on negative cases dEQP passes), nothing of it is kept,
and it could not be checked here, so treat it as unconfirmed.
`tools/check.sh --quick` there: 91 ok, 5 failed, each for a tool the host
lacks (no clang, no libclang).

Host diagnoses since (records under `tools/cts/log/investigations/`, no product change):
the Pi 4's 27 fbo `attachment_combinations.*_tex` cases should all pass now
that the V3D lists the depth-texture pair (`es2-v3d-fbo-combos.md`); the V3D
already draws every scissored or masked clear the TLB cannot as a clear quad,
and the board passed those groups (`es2-v3d-clear.md`); the two preprocessor
`define_value_and_function` failures are one stale row of
`tools/cts/RESULTS.md`, and all 534 preprocessor shaders get the CTS's compile
outcome on the host (`es2-preprocessor.md`); HOST-ONLY dEQP-GLES2
compile/link runs exist for the three back ends (`es2-host-deqp-baseline.md`,
`es2-host-deqp-v3d.md`; the V3D run lost one chunk, 87 cases never began,
to a refused draw of a large cube texture).

**Plan #2 (the benchmarks correct on the three boards), 2026-10-07 to
10-09.** Host-proved; none of these commits has run on a board.

| Defect | State | Commit |
|---|---|---|
| glmark2 terrain samples black (#1): `glGenerateMipmap` on its 800x600 targets was GL_INVALID_OPERATION under ES 2.0's NPOT rules | fixed on the host: every ES 2.0 build lists `GL_OES_texture_npot`, and a NPOT texture is complete with REPEAT and a mipmap filter. The V3D and the GC7000 lay the chain out the first time it is needed. The host harness logs no 0x502 on any GPU any more (`tools/cts/log/hostharness/glmark2/FINDINGS-post-npot.md`). The prediction in `docs/es2-cts-failures.md` section H (18 cases Pass to NotSupported) is wrong: the CTS never reports NotSupported for this extension, so those 18 are real tests that must pass; the host model passes all 18 (`tools/cts/log/investigations/es2-npot-cases.md`). Unconfirmed: the GC7000's `NON_POWER_OF_TWO` feature bit | `2673197`, `77693ff` |
| the PowerVR held level 0 of a NPOT texture only (terrain aliases where minified) | fixed on the host: a NPOT 2D chain is laid out as Mesa's pvr driver does, at the power-of-two extent (800x600 is the 1024x1024 chain, ten GL levels, 5,592,404 bytes); a level is a render target at its power-of-two pitch (`GlesDraw::target_pitch`). A stacked NPOT cube keeps the level-0 fallback. Texture words checked against Mesa's formulas in host tests; the TPU's walk of such a chain is unmeasured on the board | `cc78859` |
| AM62P: refract's 7.6 MB depth buffer refused every frame (#2) | fixed on the host: the pool's top region places GENERAL requests over 256 KiB from the carve-out's end downward, so small long-lived blocks no longer split the free space (`driver/docs/pvr-render-pool.md`). The replay of glmark2's allocations leaves a 31,908 KiB range before refract (15,100 without), which needs 22,600; the replay cannot reproduce the board's exact placement. `-DPVR_POOL_TOP_ABOVE=0` builds the old pool for the A/B | `541c9d3` |
| Pi 4: refract and terrain do not fit the 48 MiB V3D carve-out (#4, #5) | glmark2's Pi 4 image takes 96 MiB (`ca4074a`), replayed on the host (`ac0c254`, `tests/v3d_glmark2_mem.rs`). After the NPOT chains the host harness shows refract refusing one draw on the V3D: its 1600x1200 mip chain does not fit the 8 MiB tiled-copy budget (`tools/cts/log/hostharness/glmark2/FINDINGS-refract-budget.md`, predictions only) | `ca4074a`, `ac0c254` |
| gl_test case 171 `viewwin` ignored byte order (#15) | fixed: the raw-memory check follows the window's byte order (the GC7000 stores BGRA) | `41f56eb` |
| glmark2 shadow and refract on the PowerVR sample stale depth (pvr-depth-resolve) | fixed on the host. The cause (`860f6b1`): the GL core stamps what a draw renders into and reads with `backend::kick_id()` while it builds the draw, and a draw that ends the open frame (the depth pass after glmark2's window clear) lands in the next kick, so the depth texture's stamp named the clear's kick, complete by the sampling draw: `resolve_depth` neither submitted nor waited and the PowerVR answered -EBUSY. Now the core notes where each stamp of a draw lives (render-into stamps, texture and buffer last use, staging blocks; `draw::draw_stamp`) and moves them to `kick_id()` once the back end has recorded the draw, which names the kick holding it (`draw::restamp_draw`; contract on `YakoglBackend::kick_id`). The same stale stamp let a CPU write into a texture or buffer such a draw uses go under its open kick; that is fixed with it. The V3D's `open_alias` (the same repair made inside its back end, `src/v3d/backend.rs`) no longer has a stale stamp to answer for; it stays until a V3D change with a Pi 4 run removes it. Tests: `tests/depth_resolve_pvr.rs` (the BUG test passes with one resolve, no retry), `a_render_that_ends_the_open_frame_is_stamped_with_the_kick_holding_it`, `a_cpu_write_after_a_render_that_ended_the_open_frame_waits_for_its_kick` (`src/gl/draw.rs`), `after_a_draw_the_kick_id_names_the_kick_holding_it` (`src/pvr/backend_tests.rs`). The host harness logs no `did not resolve its depth` line in any entry (FINDINGS.md rows 23-24; `GM2_KICKS=1` shows the depth pass's kick order), and the full gate's glmark2 row now fails on one (`tools/check.sh`, `glmark2_corpus_tests`). Board confirmation pending: the common set's GM image on the AM62P ("Next board session") | (this commit) |

The i.MX8MP benchmark baseline is a board result from main `8abeaabb`
(the CL row from the integration script, `0f608491`; record
`tools/cts/log/benchmarks-baseline/imx8mp.md`): glmark2 score 42, 33 scenes ran / 0 failed / 0
unsupported; Slint (copy and scan-out view) and cluster probes pass; gl_test
46 / 48, case 16 first. The board has no console, so there are no per-scene
scores and no fps. The AM62P and Pi 4 baselines measured nothing (no console
capture on the AM62P, no environment for the Pi 4).

**ES 3.0, host only, 2026-10-08 to 10-09.** The GL core's ES 3.0 half grew;
no back end renders any of it yet, and no board has run it. Every "dEQP"
number here is HOST-ONLY (the Linux OSAL executes no command stream, so only
API errors and compile/link outcomes mean anything; `tools/cts/log/es3-host-env.md`).
Under `report-es2` every old answer is kept and pinned.

| Feature | State | Commit |
|---|---|---|
| Transform feedback API and rasterizer discard | the GL half is built (`src/gl/tf.rs`): 32 objects a context with indexed bindings, begin/end/pause/resume with the ES 3.0 errors, `glTransformFeedbackVaryings` resolved at link against the vertex outputs with the 2.11.8 link errors, `glGetTransformFeedbackVarying`, the PRIMITIVES_WRITTEN query state, limits 64 / 4 / 4. `GL_RASTERIZER_DISCARD` stops draws and clears before the back end. Capture is not built: a draw that would capture is refused with GL_INVALID_OPERATION and a log line, and nothing is ever written (open work 4, tf-capture). Tests: `src/gl/tf_tests.rs` (17), `tests/es3_tf_api.rs` (8; one under `report-es2`), in the V3D rows of the gate. No dEQP run. Design `docs/es3-transform-feedback.md` | `16ac864`; design `f818f84`, `0f06eba` |
| Uniform blocks | `tests/es3_ubo.rs` ports dEQP's `UniformBlockCase::iterate` for every `functional.ubo` group but `random`: 2,032 cases through the GL API, comparing the rows each stage's default block hands the back end with what the generated shader compares against. It found and pins one defect, fixed: `glGetUniform*` of `mat2x3`, `mat2x4`, `mat3x4` and `mat4x3` read the wrong dwords (it indexed by the column count as if it were the row count); host dEQP-GLES3 on the GC7000 `uniform_api.value.*.get_uniform` 24 Fail to 0. Passes on PowerVR and GC7000; on the GC7000 four `single_nested_struct_array *_instance_array_both` cases are refused at link (184 + 184 constant registers against the 320 the stages share), pinned in `KNOWN_LIMITS` | `1b0e431`, `b0c0fa6` |
| Sized internal formats | the 35 colour formats of ES 3.0 table 3.2 the tables lacked (snorm, packed float, `RGB9_E5`, `RGB16F`, `RG32F`, `RGB32F`, `RGB10_A2`, `RGB10_A2UI`, the 24 integer formats) are accepted with their table 3.2 pairs and converted on the CPU at upload into the first storage class the back end samples (`src/gl/es3_formats.rs`, `docs/es3-sized-formats.md`); integer textures and 32-bit float ones are complete only with NEAREST filters. `RGB5_A1` takes `UNSIGNED_INT_2_10_10_10_REV`; copies into integer, snorm or float images from a fixed-point buffer are GL_INVALID_OPERATION. The PowerVR samples every new class but `RGB10A2` in Mesa's TPU format (`docs/es3-pvr-formats.md` section 10), the integer ones through `isampler2D`/`usampler2D` (an `smp` without `fcnorm`; other integer sampler dimensions refused by name); `RGB10_A2` stays in `RGBA8` there (renderable and mipmappable). On the GC7000 and the V3D the integer formats and `RGB10_A2UI` are refused by name, and the float, packed-float and snorm ones land in `RGBA16F` on the GC7000 and are refused on the V3D. Host dEQP-GLES3 `texture.specification` on the PowerVR: the `glGetError() == GL_NO_ERROR` bucket 720 to 594 (before the PowerVR classes). Tests `tests/es3_sized_formats.rs`, `src/gl/es3_format_tests.rs`, `pvr::backend` `es3_class_descriptors_are_mesas`; on the board: gl_test 201, pending | `888af5d`, `e5c4a18`; PowerVR `074c48f`, `03340c8` |
| Occlusion queries | `GL_ANY_SAMPLES_PASSED{,_CONSERVATIVE}` are counted on all three back ends (design `docs/es3-occlusion-query.md` (`d303712`, `e37662e`); the GL core's state machine and errors per ES 3.0 2.14, `glGetQueryObjectuiv` waits and answers 0/1, the back-end hooks `query_begin/end/result` and `GlesDraw::occlusion_query`). V3D: `OCCLUSION_QUERY_COUNTER` with the query's word, in the packet-skip key. GC7000: the PE's `GL_OCCLUSION_QUERY_ADDR/CONTROL`, suspended at every switch and stream end. PowerVR: `ISPCTL.vistest`/`visreg` on TRANSLUCENT counted draws, a result area per kick as `ISP_OCLQRY_BASE` with `GETVISRESULTS`, the frame ended at `glEndQuery`. Proven on the host only (`gl::es3::tests::occlusion`, each back end's tests, `job_check` part 19, `vdm_field_check`); the first board test is gl_test 196 `occl` ("Next board session"). Under `report-es2` every target is refused as before | `af6d259`, `b03863c`, `a6cc10d`, `45fb13f`; gl_test 196 `71430ed` |
| Integer colour render targets | the GL half is built: the 19 colour-renderable integer formats are renderbuffer storage and complete framebuffers, `glClearBuffer{i,ui}v` clear them with raw bits, `glReadPixels` takes `RGBA_INTEGER`, blending is off for them. The PowerVR writes all nineteen as a draw's one colour attachment (`INTEGER_RT_FORMATS`; the program bit-packs the int/uint output per Mesa's `pack_to_format`, the PBE stores it in its integer pack mode, clears are quads of the raw packed dwords, a frame without a clear reloads the bits raw; `docs/es3-pvr-formats.md` section 10); refused by name there: a colour mask keeping some of the target's channels, an integer attachment beside another one. The GC7000 and the V3D write none yet, so there a draw or clear into one is refused by name with GL_INVALID_OPERATION (open work 4, int-rt). Tests `tests/es3_int_rt.rs` (14; one under `report-es2`), in the gate's V3D rows; `tests/usc_limits.rs` `integer_render_targets_get_the_packed_bits`, the `pvr::backend` integer tests; on the board: gl_test 200, pending | `b9b2ac0`; PowerVR `1779e02`, `03340c8` |
| NPOT levels of arrays and storage textures | every level of a NPOT 2D array or `glTexStorage2D/3D` texture is valid (ES 3.0 3.8.14); levels a back end cannot hold are recorded and their texels dropped, as for a NPOT cube. Tests `src/gl/es3_level_tests.rs` | `c40414b` |
| Stage-interface link rules | the link refuses a varying whose interpolation qualifier differs between the stages and a uniform block declared differently in the two (`tests/es3_link_iface.rs`) | `b88fea5`, `03ff7a6` |
| GLSL ES 3.00 in the compiler | measured over the dEQP gles3 `.test` corpus, 5,596 programs (`docs/es3-shaderc-coverage.md`, `tests/es3_shaderc_corpus.rs`); line continuation inside tokens and directive names (`tests/es3_shaderc_pp.rs`) | `4ab482e`, `79fe171`, `ed0c81d` |
| GL-core fixes from the dEQP-GLES3 inventory | `GL_RGB10_A2` is renderbuffer storage (held as RGBA8, which the size queries say); `glRenderbufferStorageMultisample` with one sample keeps `GL_RENDERBUFFER_SAMPLES` 1, a framebuffer mixing sample counts is `GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE`, and `glReadPixels` from one with sample buffers is GL_INVALID_OPERATION; a texture attachment of an ES 3.0 sized format answers its own component type and bits; `glReadPixels` into a pixel pack buffer checks the whole write against the buffer and writes the rows back from the cache; `glUnmapBufferOES` / `glGetBufferPointervOES` take every ES 3.0 target; `glDrawBuffers` on a framebuffer object checks `n` before the enums; `glVertexAttribI4*` values reach an ivec4/uvec4 input and `glGetVertexAttribI{i,ui}v` exactly, `glGetVertexAttribiv` rounds. Under `report-es2` `GL_HALF_FLOAT`, the packed types and the integer array queries are GL_INVALID_ENUM. Tests `tests/es3_gl_core.rs`, `tests/es3_vertex_api.rs`; the vertex buckets of the AM62P run in `docs/es3-vertex-gap.md` | `468528a`, `0384071`, `3d1e5ba`; vertex `8b3e703` |
| Designs, docs only | sRGB render targets and sampling (`docs/es3-srgb.md`), PowerVR 2D-array and shadow sampling (`docs/es3-pvr-2darray.md`), PowerVR format map (`docs/es3-pvr-formats.md`), the AM62P GLES3 fbo / fragment_ops / fragment_out failure map (`docs/es3-fbo-fragops-failure-map.md`), the AM62P dEQP-GLES3 failure inventory by root cause from the `396d667a` numbers (`docs/es3-cts-failures.md`, `1e58b9b`). Every hardware statement cites Mesa 26.2.1 or is marked unverified; nothing in them is measured on a board | `948710f`, `1a48782`, `71d8dd7`; `0edefef`; `6abe43d`, `8a1260b`, `7dedef7`, `a43ec9b`, `710934d`; `53c7ba3`, `3cdbe16` |
| ES 2.0 report guard | `tests/es2_report_guard.rs` (`report-es2` builds only) pins what an ES 2.0 client sees, each pin with its ES 2.0 rule: the version strings, the exact extension list, the ES 3.0 pnames and entry points refused, the table 6.18-6.20 limits, the renderbuffer and texture formats. Four pins the code does not meet yet are `#[ignore = "BUG: ..."]`: ES 3.0-only `glGet` pnames and `GL_RASTERIZER_DISCARD` / `GL_PRIMITIVE_RESTART_FIXED_INDEX` are answered, `GL_R32F`, `GL_DEPTH_COMPONENT32F`, `GL_DEPTH32F_STENCIL8` and `GL_BGRA8_EXT` renderbuffers are accepted, and the ES 3.0-only pixel types get GL_INVALID_OPERATION, all where ES 2.0 says GL_INVALID_ENUM. It replaces the reverted `75dedad` | `6c9e9ee` |
| Reverted | the ES 3.0 limits test with its back-end caps doc (`b6004e1`, reverted `3f78982`), merged without the review it needed. Open work 4 redoes it | |

The only dEQP-GLES3 board run is still the AM62P tree at `396d667a`
(2026-09-26): of 42,810, 31,278 pass, 8,835 fail, 2,636 NotSupported, 5
QualityWarning, and 56 `lifetime` cases without a result because transform
feedback was missing (`tools/cts/RESULTS.md` 1r).
An attempt at the AM62P tree on `32778b57` (2026-10-10) produced no results:
its images reported ES 2.0, so dEQP's GLES3 context was not created
(`tools/cts/am62p-deqp-images.sh` now passes `-DCONFIG_YAKOGL_REPORT_ES2=n`).
It is a note, not a measurement.
The Pi 4 has a dEQP-GLES3 tree too, not yet run: `tools/cts/pi4-gles3-tree.txt`,
91 images sized for the `v3d_deqp` image's window, selecting each of the 42,810
cases once (`tools/cts/check-gles3-tree.py`, run by `check-deqp-images.py` and
`cts-selftest.sh` check 19 on both GLES3 tables); `pi4-deqp-images.sh
DEQP_MODULE=gles3` builds it and `docs/es3-board-recipe-pi4.md` is the recipe.

The AM62P GLES3 tree on `c4ccc966` (2026-10-10, `records/am62p/deqp-gles3-c4ccc966`)
ran 24 of 25 images; the 19th (`texture.filtering.*` and four neighbours) never
finished because `texture.filtering.3d.combinations.linear_mipmap_linear_*` (not in
the mustpass list) takes minutes a case and the 600 s watchdog reset the board.
The tree is now 28 images (`tools/cts/gen-am62p-gles3-tree.py`, checked by
`tools/check.sh`): `texture.filtering` is split by dimension, its 3d combinations
name only the mustpass prefixes, and 3d `formats` and `sizes` have an image each.
Measured spans per image, all under 260 s, are in `tools/cts/README.md` 2e; the new
rows 19 to 22 are unmeasured and the next tree run records them.

Results are from main as of `186cbb56`: AM62P gl_test on `05f8f749`, Slint and cluster on `05f8f749` (`7dada1ff` for the fill_color figures), i.MX8MP gc_gl_test on `05f8f749`, the Pi 4 on `afa12e9b`. All three boards boot standalone from eMMC (the two Verdins, U-Boot) or SD (Pi 4).
`docs/standalone-boot.md` has the procedure.

| Board | gl_test | Slint | Cluster |
|---|---|---|---|
| AM62P | 49 / 49, 0 resets on `c4ccc966` (2026-10-10, `records/am62p/20261010-13*-gt-c4ccc966-*`, 197 `vtxgrid` in the set, compiler runs 202); 48 / 48, 0 resets on `32778b57` (2026-10-10, `records/am62p/20261010-0611-gt-32778b57-a1cebe9a`, compiler runs 197, precompiled hits 0); 48 / 48, 0 resets (the set now has 192 `predlive`); 48 / 48 in the compiler-only and precompiled + compiler images (`05f8f749`) | pass on `7dada1ff` (705 frames in 10007 ms; also with the precompiled tier, same records, `17605c36`) | 91.8 fps, 56.4 Mpx/s, 16.28 ms/Mpx on `7dada1ff` (also with the precompiled tier, same records, `17605c36`; `39bd6983`: 91.2 fps, 56.0 Mpx/s, 16.11 ms/Mpx) |
| i.MX8MP | 47 / 48 run, 1 skipped (first skip case 16), first failure case 44 `es3hdr` (#6) on `a0f439d6` (2026-10-10, `records/imx8mp/20261010-1623-gt-a0f439d6-44175f39`, `594b010e 002c0107 ffff002f 00100030`): the walker's fix shows the unnamed failure of `c4ccc966` is case 44; its fix (the table row above) waits for the board. 47 / 49 on `c4ccc966` (two runs, `594b010e 0010ff06 0000002f 00000031`): one real failure, unnamed, and case 16's by-name skip, which the walker then counted as a failure (#8); from the `CASE_SET` record on, the same set should read 47 / 48 run, 1 skipped (first skip case 16), and name the failing case (board run pending). 46 / 48 on `72d16864` and `112dd4fb`, first failure case 16 (#8). On `05f8f749`: 44 / 48 (first failure case 2), 45 / 48 (case 2), 46 / 48 (case 16): case 2 is intermittent (#25). On `8abeaabb` (the benchmark baseline): 46 / 48, case 16 first | probes pass on `8abeaabb`, copy and scan-out view | probes pass (image from the integration script, `0f608491`); no fps, the board has no console |
| Pi 4 | 12 / 12 on `afa12e9b` (not run since) | not run since | not run since |

AM62P, the fill shader from femtovg (`fill_color`), 3D busy per frame:

| Measurement | Run-time compiler `81c53c61` | `39bd6983` | Mesa table (`afa12e9b`) |
|---|---|---|---|
| gl_test case 39, perf 2 (16 full-screen quads) | 77.19 ms | 81.1 ms | 83.4 ms |
| case 186 arm 0 (blend) | 76.98 ms | 80.9 ms | 83.4 ms |
| case 186 arm 1 (nodiscard, blend) | 54.56 ms | 55.3 ms | 54.9 ms |
| case 186 arm 2 (blend off) | 68.93 ms | 69.8 ms | 71.6 ms |
| case 186 arm 3 (nodiscard, blend off) | 44.00 ms | | |
| case 187 arms 0 / 1 / 2 / 3 | 81.02 / 59.05 / 72.52 / 47.94 ms | | |

The table column comes from an earlier image. No table image of `81c53c61`
was run.

Case 193 `offarm` (`7dada1ff`): 186 arm 1 with the never-run arm grown by N
groups, 3D busy per frame (record `20261005-2118-case193-7dada1ff-ad3d86cf`
online, `20261005-2119-case193-table-7dada1ff-ed2efe33` table; pixels equal
at every N):

| Added groups N | Run-time compiler | Table |
|---|---|---|
| 0 | 54564 us | 54851 us |
| 8 | 55415 us | 56008 us |
| 32 | 59323 us | 60633 us |
| 128 | 91142 us | 91745 us |

The offline PowerVR table (`tests/reference/blobs.rs`) records the toolchain
that reproduces it byte for byte: a `pco_shc` built by
`tools/pco_shc/build_pco_shc.sh` with the patched usclib. It was regenerated
2026-10-07 on a second host (`pco_shc=01aa62a069bb glslang=2e8c31daf6b2`)
and again 2026-10-10 on the landing host (`// toolchain: pco_shc=f537fc86b41f
glslang=9e007f4bf514`, the `0fe778f6` ids): all 2476 USC rows are
byte-identical across the three, so the board results recorded against that
table carry over. The id names the binary, so a host whose gcc build differs
gets another id for the same rows; the gate row compares the whole file, and
the recorded line is the landing host's. The `0fe778f6` commit also
adds `matattr`'s 20 rows and re-keys `texsize.frag` to its current text.
On the AM62P (`60ffeec8`, table image) the regression set passes 39 / 40,
0 resets; the one failure is case 48 `es3vol`, which was already failing in
table images. Case 46 `es3size` (it uses `texsize.frag`) passes now; it was
refused before the re-key. Case 63 `matattr` passes, served from the table
for the first time.
The GC7000 table still lags on `matattr`: issue #22, not fixed by `0fe778f6`.
Since `84b5761e` no product image links a Mesa-built row: the reference lives
in `tests/reference/` for the host gate, and the measurement table image opts
in with `mesa-reference-table`. An image can now carry its app's shaders
precompiled by our compiler (feature `precompiled-shaders`, Kconfig
`YAKOGL_PRECOMPILED_SHADERS` and `_APPS`) in front of the compiler.

A never-run arm of a predicated `if` still costs time (gl_test 193, 194 on the
AM62P): 0.11 ms a group for a small arm, rising to 0.38 ms a group beyond
about 128 groups (an executed group is about 0.79 ms). A branch has a fixed
cost of 5336 us a frame, about 6.8 executed groups (195). The default lowering
(`select::branch_pays`, `5f4d3677`, board-proved: gl_test 48 / 48, record
`20261006-0403-gt-5f4d3677-ecce9b75`) branches an `if` iff the masked cost of
each arm exceeds that fixed cost, which is crossed at 49 groups an arm; 0 of
168 corpus `if`s branch, and `fill_color` (31 / 23 groups) stays predicated.
`PVR_BACKEND_OPT_IF_LOWERING` PREDICATE and BRANCH force a form. If a real
shader ever shows a large arm under a uniform condition, the step is a per-draw
variant chosen by the GL core from the condition's value (predicated when the
long arm runs, branched when it is skipped; `fill_color` at N = 128 would go
from 91.1 to 55.3 ms with the arm off); it is not planned now.

## Open work, in order

1. **Compile shaders at build time with our own compiler.** Shaders known
   at build time are compiled then; the run-time compiler runs only on a
   miss. An image without the run-time compiler refuses a miss by name.
   Mesa's `pco_shc` / `etna_shc` then compile nothing.
   Done, in this order: `68563f54` the one source list every table generator
   reads (`tools/shaders/precompiled.list`) and `yakogl-precompile`, which
   compiles it with our compiler on the host (tools/README.md); `dea95deb`
   the Mesa reference leaves product code (`tests/reference/`,
   `shaderc::mesa_reference`; a product links it only as a measurement
   image's table, `mesa-reference-table`, Kconfig `YAKOGL_SHADERC_OFFLINE`);
   `84b5761e` the compiler is the default back end in Kconfig and CMake
   (only `baremetal.mk` still takes the reference: its OSAL lends the
   compiler no region); `17605c36` the precompiled tier in front of the
   compiler (feature `precompiled-shaders`, Kconfig
   `YAKOGL_PRECOMPILED_SHADERS` and `_APPS`: the table looked up by the
   compiler's record key before any compile, a hit running none), with
   `17e7d34c` the image without a compiler (`YAKOGL_SHADERC_PRECOMPILED_ONLY`:
   a miss, a variant or a program the table lacks is refused by name,
   -ENOENT; gl_test's closing line counts precompiled hits and compiler
   runs); `05f8f749` the back ends' own programs from our compiler (the
   PowerVR's frame-load, masked-clear and clear-attach programs, the
   GC7000's clear pair: the list's `internal` lines, `yakogl-precompile
   --internal` into `src/pvr/internal.rs` / `src/vivante/internal.rs`).
   Board proof on the AM62P (`05f8f749`): gl_test 48 / 48 compiler-only
   (hits 0, compiler runs 197) and precompiled + compiler (hits 111, runs
   65); the image without a compiler 40 / 48 (cases 181, 182, 183, 188 to
   192 refused by name, hits 111, runs 0); Slint and the cluster keep their
   records, cluster 91.8 fps. Earlier: the tier on `17605c36` with
   `gl_test_es100`'s 1291 rows, 48 / 48; Slint and the cluster with
   `femtovg`'s 583 rows, same pictures.
   Open, not parked: the driver's five end-of-tile programs, the boot
   self-test's vertex shader and the VDM-sync program are still Mesa's text --
   `driver/src/pvr_render.c:56` says "Static VDM_SYNC USC program (Mesa
   static_data_areas: pco_usclib ...)" -- so pco compiles them for a product
   image. A branch `work/pco-driver-programs` (`f5e4aa57`) that made them come
   from our encoder, byte-identical to pco's, was recorded here as parked; it
   is not in this repository (no branch of that name, and `f5e4aa57` is not an
   object -- checked 2026-10-07). The work is open.
   Closed: the list's gl_test app missed the inline shader sources of cases
   181, 182, 183 and 188 to 192. gl_test.c's 61 GLSL string literals became
   `tools/shaders/gltest/` -- 55 files, four texts appearing twice or three
   times -- named in `gen_gl_test_shaders.py`'s ENTRIES, and both generated
   headers carry them (`88246148`). The pair reader gained the two rules the
   moved sources showed it needed: a link call whose other half a case fills
   with GLSL while it runs names no program, and a table holding both halves
   names its programs by its rows, not by its vertex shaders times its
   fragment shaders. The list's gl_test apps now hold 145 + 145 pairs (101 +
   101 before), and `tools/precompile/gl_test_pairs.py` refuses to generate a
   list from a gl_test.c that writes a GLSL literal of its own. Four sites
   stay unresolved and are named by case -- 149 case_vsunits, 150 shprobe and
   166 fshprobe's shp_arm, 166's fshp_arm_units: each links a named source
   beside a buffer it fills with GLSL while it runs, so no pair names that
   program and none can. Those cases need the in-process compiler.
   The move moved the pinned counts, and the gate is what surfaced it. In
   `tests/fixtures/precompile_counts.txt`, `powervr`'s whole-list rows are 3346
   and 2651 (`es2=0`, `es2=1`) against 2749 and 2054, and `gl_test_es100`'s 1888
   and 1785 against 1291 and 1188; the GC7000's did not move, because `dir gltest`
   has role `-`. Measured per app on the `es2=1` build,
   the growth is exactly the 44 pairs the move made nameable: `gl_test` 639 →
   1234 rows, `gl_test_es100` 1188 → 1785, and the whole list 2054 → 2651 --
   +597, the same as `gl_test_es100`'s, because the two apps list the same 44
   pairs (42 of them the same lines; the two that differ pair a gltest vertex
   shader with the root `interp_color.frag`, a `#version 450` source the compiler
   refuses, so those pairs are left out and add nothing). The corpus pin moved
   with the files: `tools/shaders/` holds 789 `.vert`/`.frag` sources, 734 before,
   and none of the 55 new ones is refused -- all four
   `tests/fixtures/online_refused_*.txt` are unchanged.
   Closed by a gate row added with this commit: nothing compared the two
   generated headers to the files they are made from. `gen_gl_test_shaders.py
   --check` regenerates both in memory and refuses a drift, and `tools/check.sh`
   runs it beside the rows that compile gl_test.c.
   Open: the GC7000 has no row for a gltest source (138 of the 184 gl_test
   sources have no Vivante row, 123 of them before the move); a `gc7000` role
   for `dir gltest` needs `gen_shader_table.py --append` with etna_shc, then
   an i.MX8MP run with no compiler. `tools/v3d/glsl2v3d.py` and
   `tools/v3d/sim_fixture.py` fail on any unresolved site, and the reader had
   two at `b2e0144f` (case_psizvpm, offarm_run), so the V3D table cannot be
   regenerated as it stands.
   Open, and a limit rather than a gap: sampler-state (lod-key) variants are
   not precompiled (`gl_test_es100/texture.frag has no variant for sampler key
   0x1`). A variant is a fact about a draw, not about a program: the key is the
   sampler and texture state of the draw that asks for the program
   (`src/gl/draw.rs`, `stage_lod_key`), and `gl_test_es100`'s 1888 rows carry
   1705 non-zero sensitivities, 1586 of them the sample-kill bit alone.
   Enumerating them is 2^k rows per program on a table that is already 11.4 MB
   of Rust for one app. A missing variant is never a refusal:
   `lod_variant_for()` gives the draw the program's own shaders -- the TPU's
   quad level, or the hardware's switch at 0 -- which is what every draw did
   before the tier existed. So an image with a compiler loses only a compile,
   and an image without one draws at the base level; what the compiler-less
   image cannot do is pick a mip-free or a switch-at-N variant.
   Step 6 (the default turns on) was measured, and the measurement reversed it:
   the default stays off. The AM62P gl_test image (verdin_am62p,
   aarch64-unknown-none, release), built four ways on 2026-10-07 by
   `tools/precompile/measure-image.sh`, which also reads each arm's `size -A`
   and refuses an arm whose built `zephyr/.config` is not the arm it names:
   2,427,152 bytes with the tier off, 4,602,128 with `gl_test_es100`'s rows in
   front of the compiler, 3,717,064 with those rows and no compiler at all,
   5,953,808 with every app the list names. A repeat run built all four
   byte-identically. The seconds are not a property of an arm: the same four
   arms took 11, 52, 28 and 74 s one run and 11, 17, 15 and 19 s the next.
   The compiler is 712,704 of .text and 171,497 of .rodata -- 884,201 of flash
   -- and 5,244,035 of .bss, the scratch and keep regions
   `YAKOGL_SHADERC_SCRATCH_BYTES` lends it; the compiler-less arm gives back
   exactly those three numbers. The table for that one app's 176 sources is
   2,173,648 of .rodata and 5 of .bss, of which 1,447,864 are instruction
   words: 2.5x the compiler's flash, for one app, and it grows with the app's
   shader set while the compiler does not. So the tier is not what every image
   should pay -- it is what an image pays when it wants no compiler, and there
   the trade is 1,289,912 bytes of flash for 5,244,035 bytes of RAM.
   Done: that reflection was written once per variant row. Every row carried its
   own `ShcShader` -- varyings, uniforms, attribute map, blocks -- although all
   the variants of one source share one reflection, so the same 176 sources'
   reflection appeared about 10.7 times. The table stores it once per compile
   (the next Done corrects what "per source" missed): a `ShcPrecompiledRow`
   points at a `ShcPrecompiledReflection`, and the generator refuses a compile
   whose rows disagree on a field stored once. Measured on
   the host, same list and same compiler, the old emitter against the new:
   `gl_test_es100` is 1888 rows of 176 sources and 10,798,308 -> 8,600,409 bytes
   of table (-2,197,899, -20.4%) with its 1,447,864 instruction words unchanged,
   so the whole saving is reflection; the powervr whole list is 3346 rows of 613
   sources and 27,440,696 -> 21,455,420 bytes (-21.8%).
   Done: the board's number for the new table, which is what step 6 waited for.
   The same image and the same list, measured above: that app's rows are
   2,173,648 bytes of .rodata and 2,174,976 bytes of image where the per-row
   layout cost 4,881,999 and 4,882,432 (-55% both ways). The image moves more
   than the table's own source does (-20.4%) because the instruction words,
   which did not change, are verbose as Rust source and compact as data. Both sides of the comparison step 6 turns on are now
   measured on the board -- 884,201 bytes of flash and 5,244,035 of RAM for a
   compiler that serves any shader, against 2,173,648 bytes of flash for a
   table that serves one app and grows with it -- and the answer is the same
   one the first measurement gave: the default stays off.
   Done: the reflection is stored once per compile, not per source -- the split
   above was wrong for a vertex shader whose compile defers its attribute layout
   to the link. Every back end that defers sets the same flag: the GC7000 and
   the V3D at more than 16 attribute locations, the PowerVR when its 64
   vertex-input registers run out (`deferred_vertex`). The deferred record is a
   stub -- no code, no attribute list, the sentinel offsets
   `meta_off = SHC_NO_META` and `vs_psize_off = SHC_NO_PSIZE` -- while the
   record the link's map produces in its place has all of them, so the two are
   not variants of one compile and cannot share a reflection. Both exist at
   runtime: `src/gl/program.rs` reads `attribs`, `declared` and `layout_at_link`
   per shader, and `shaderc::precompiled` serves both compiles from the table
   (`compile` takes the stub row, `compile_vs_linked` the laid-out one). So the
   three fields moved to `ShcPrecompiledRow`, and a reflection is named for its
   compile -- `S_<sha256>_<VS|FS>`, plus `_DEFERRED` when the layout was left to
   the link. The generator now refuses two variants of one compile that disagree
   on a field stored once, naming every field it sees, and its own tests cover
   the deferred case (the PowerVR deferral has no reproducer from attribute
   locations alone: 17 attributes fit its 64 registers, and 17 mat4 attributes
   exceed the front end's 32-location cap, so it is reached only under register
   pressure). The real corpus is unchanged: powervr 1888 rows of 176 reflections,
   gc7000 111 rows of 110, and neither has a deferred row.
   Open: an image that turns the tier on must name its apps. With the tier on
   and `YAKOGL_PRECOMPILED_APPS` empty the build stops (CMakeLists.txt,
   zephyr/CMakeLists.txt) instead of linking the whole list -- 14,154,000 bytes
   against 2,427,152, measured 2026-10-07 with the per-row layout, through the
   empty value the build now refuses -- which is every shader this repository
   lists, Qt's and Qt3D's and the test suite's among them, for an image that
   runs one app.
   `yakogl-precompile` with no `--app` still means the whole list, and that is
   what the table generators and the gate's tier rows use.
   Fact, from the arm that names every app: 407 of the source list's 613
   sources belong to no app. An `app` line claims the `pair` lines after it
   (tools/shaders/precompiled.list), so the list's `src`-only sources -- the
   PowerVR table's inputs, in the table's row order -- reach an image only
   through the whole list, which the build refuses for an image that names
   nothing. Naming every app the list has is 2389 rows of 206 sources
   (5,953,808 bytes on this image), not the list's 3346 rows of 613, so the
   apps arm is the largest table this list lets an image name.
   Open: three kinds of shader no table can serve. gl_triangle
   (`driver/samples/zephyr/gl_triangle/src/main.c:71`) and gc_gl_triangle
   (`.../gc_gl_triangle/src/main.c:170`) write their GLSL as C literals in the
   sample's own main.c -- the class of defect gl_test had -- so the list has no
   app for them and a compiler-less image cannot draw them. glmark2 and dEQP
   take their shaders from outside this repository, so no list names them
   either. Those images need the compiler.
   Remaining steps: 6 measured and decided -- the default stays off (the board
   numbers above); 7 rename to "precompiled shaders" and
   "shader compiler" (features `precompiled-shaders` and `shader-compiler` for
   `shaderc-online` and `shaderc-offline`; Kconfig symbols, the C ABI, log
   prefixes); 8 the source moves from `src/shaderc/online` to
   `src/shaderc/compiler` and `docs/online-shaderc` to `docs/shader-compiler`;
   9 optionally, V3D rows. The words online, offline and remote go away.
2. **Design clean-up.** Decided 2026-10-06; the measured inventory is in
   parentheses.
   - Done: clean-up 1 (`260711b9`, the GC7000's debug statics: 4 removed, 5
     hard-coded at the default every image ran, 3 with their C entry points
     behind the new `test-knobs` feature, `CONFIG_YAKOGL_TEST_KNOBS`;
     product archive `gc_debug_*`/`gc_perf_*` symbols 32 -> 8) and clean-up
     2 (`72d16864`, `mrt`, `vs_tiled`, `int_filter` and `desc_seq` become
     the design, the narrow L8/A8/LA8 store on by default; the product
     archive defines no `gc_debug_*` symbol). Board, i.MX8MP gc_gl_test:
     `260711b9` 45 / 48 (first failure case 10, #9), `gc_gl_triangle` byte
     identical to `84b5761e`; `72d16864` 46 / 48 (first failure case 16,
     #8), with MRT on cases 106 `mrt3` and 163 `drawbuf0` run. dEQP on
     `72d16864`: 10 of 12 rows identical to the 29460f96 baseline; two
     regressions, `fbo.completeness.renderable.renderbuffer.depth.
     depth_component24` and `shaders.fragdata.*` (indexed `gl_FragData[]`
     write refused).
   - Awaiting the gate and landing, branch `work/cleanup-1-vivante-knobs`:
     clean-up 3 (the PowerVR's switches leave the product: 5 hard-coded at
     their measured defaults, 2 switches and 8 options behind test-knobs, 5
     options retired; no image changes behaviour). Its two dEQP fixes were
     redone on main by plan #1 (`4e9397c`, "State of main"); do not bring
     them in a second time when the branch lands.
   - One more knob to remove: `gc_debug_msaa` (the GC7000's 4x MSAA switch:
     bit 0 the 4x path, bits 1 and 2 etnaviv's extras, bit 3 the viewport
     and clip scaling). This clean-up removes debug knobs from the product,
     so it goes too: if the bit 3 arm passes in "Next board session", the
     scaling becomes the design and the knob is removed (follow-up there).
   - Order from here (4 to 15): GL core / V3D knobs; the driver Kconfig
     instruments; samples cut; gl_test Rust harness; (after the shader-compiler
     rename, item 1) compiler knobs; samples stage 2; bare metal; names;
     OSALs to Rust; docs. Clean-up 1 to 3 are the Vivante and PowerVR knobs.
   - Globals: about 59 debug and experiment knobs (32 `static mut`, 16
     `PVR_BACKEND_OPT_*` slots, 11 others). Only `IF_LOWERING` and
     `ONE_IN_FLIGHT` stay product options. The measurement infrastructure
     (draw-step timing, `CHECK_RUN`, 3D-busy accounting) stays.
   - Samples: 22 today -> triangle, gl_test (one sample, merged across the
     GPUs), slint, cluster, deqp, plus glmark2 for measurement.
   - Bare metal is made true, not removed. `src/osal/baremetal.c` is a
     skeleton with three `TODO(board)`. It must lend the compiler its
     scratch and keep regions as `zephyr_shaderc_scratch.c` does; the
     application must provide the documented 256 KiB compile stack; a board
     layer is needed (firmware boot, GPU MMU, cache maintenance, IRQ-less
     waits); the gate must build `baremetal.mk`; one board boot must be
     recorded. Today no gate row runs `baremetal.mk` and
     `tools/cts/RESULTS.md` has no bare-metal boot. Design first, after the
     rename.
   - Names: `SAFEUI_*` -> `YAKOGL_DRV_*` (652 occurrences in 119 files, 48
     Kconfig symbols), including the cross-core protocol names shared with
     the M7 (`SAFEUI_FRAME_RING_*`, `SAFEUI_M7_PAGE_*`, `SAFEUI_LAYOUT_DIR`,
     `samples/zephyr/safeui/`); `safeui,*` DT compatibles -> `yakogl,*`;
     module `powervr-bxs` -> `yakogl-drivers`; `PVR_SAMPLE_*` ->
     `YAKOGL_SAMPLE_*`. The external `slint-safe-ui-zephyr` references stay.
   - Docs: the records the ccfarm agents wrote 2026-10-07 to 10-09 (board
     sessions, investigations, benchmark baselines, the host harness's
     glmark2 findings, the ES 3.0 host environment notes) are under
     `tools/cts/log/` (`board-sessions/`, `investigations/`,
     `benchmarks-baseline/`, `hostharness/`). They are records of what was
     run and read at a commit, not design; a design statement in one of them
     counts only once a `docs/` file or this file says it.
   - Docs: the plans and bring-up diaries in `driver/docs` and
     `docs/direct-boot.md` move to `tools/cts/log/` (the 35 code references
     are updated in the same commits); `docs/` and `driver/docs` then
     describe only the current design.
   - Remaining C moves to Rust: the OSALs (3,740 lines), `gl_test.c` (20,291
     lines: a Rust runner calls the C cases through `extern "C"`, ported in
     batches), `driver/src` (about 16k lines).
3. **Known defects.** Each has a GitHub issue unless marked "no issue yet";
   what remains is the fixes. State as of 2026-10-10; none is confirmed on a
   board from main:
   - **pvr-msaa-split** (no issue yet): a 4x frame the PowerVR splits into
     two kicks (a full arena, the 1024-draw cap, which counts the frame's
     own quads, or a glFinish / glReadPixels before more drawing) loses its
     samples. The end-of-tile store resolves them into the target (PBE
     `downscale`) and the next kick's colour-load quad samples the resolved
     target once a pixel and writes that colour to all four samples, so a
     sample the later draws leave uncovered holds the average. The two
     `multisample.constancy_*` cases met it through the PDS arena (fixed on
     the host by sharing the frame's PDS programs: "State of main"); a 4x
     frame past 1024 draws, or one read back mid-picture, still does.
     gl_test 198 does not split (the host harness: one kick a picture);
     its hang on `452d6c31` is not this defect ("State of main", gl_test
     198 row).
     `a_split_multisampled_frame_reloads_each_sample_not_the_resolved_colour`
     (`src/pvr/backend_tests.rs`) is `#[ignore = "BUG: ..."]` and asserts
     the right answer: the load quad runs per sample. The fix keeps a store
     of each sample across kicks (an unresolved emit beside the resolved
     one, or a 4x image resolved at the frame's end, as Mesa's pvr keeps an
     MSAA attachment) and reloads it at the full sample rate; the PBE's
     unresolved layout and the per-sample load are unmeasured on the board;
   - AM62P: the 45 `texture.vertex.2d.*` cases of the `32778b57` tree
     (image 9) are fixed on the host, not yet on a board ("State of main");
     an indexed draw whose staged data cannot be cut (a fan or a line loop,
     a second index list, or one primitive whose vertex window is over half
     the arena) is still refused by name when it does not fit an empty
     frame. The GC7000 solved the same draw by giving it memory of its own
     (`7af5e841`); the PowerVR keeps its fixed render pool;
   - AM62P: 9 `shaders.invariance.{lowp,mediump,highp}.common_subexpression_
     {1,2,3}` cases failed on the integration image `b83216d4` (image 1;
     they passed at `d60b9d76`). Diagnosed and fixed on the host: the
     PowerVR multiply-add fusion fused a product only where it had one
     reader, which depends on what else the shader computes; a sum feeding
     an `invariant` output is now never fused (the table row above). Not yet
     confirmed on the board; until image 1 passes them they block goal 1 on
     the AM62P;
   - AM62P: `multisample.constancy_alpha_to_coverage` and
     `constancy_both_inverted` fail on the ms4 image of the `32778b57` tree
     (14 / 16, two identical runs; 16 / 16 at `d60b9d76`; no issue yet).
     Alpha to coverage goes through the fragment shader's `sample_kill`
     variant (`needs_sample_kill` in `src/gl/draw.rs`); the cause is not
     found, diagnosis is under way;
   - i.MX8MP gl_test case 10 fails about 1 run in 3 (#9), and case 2
     (`texture`) fails intermittently since `05f8f749` (2 of 3 runs: 44 / 48
     and 45 / 48 with case 2 first, 46 / 48 with case 16 first; `112dd4fb`,
     without commits 4 and 5, 46 / 48) (#25). The command streams of 24
     `cs_ours` modes are byte-identical between `72d16864` and `05f8f749` and
     the two images differ only in `record_key`, `gl_test_run` and the clear
     tables, so #25 is the nondeterminism class of #9, not a deterministic
     regression. **New state (`8409419`): cause not found, no product
     change.** Our sync sequence has a counterpart for every etnaviv step;
     three surface hypotheses remain (a linear PE target, 64-byte target
     alignment, pool placement that depends on history). The recipe's
     repeat runs on integration images (`b20e4d11`: base 4 / 40, page 1 /
     10, no-reuse 2 / 10; `0175bac1`: 1 of 30, case 2 0 of 40) did not
     separate them: every arm still fails, at a lower rate than 14 / 30. A
     host reading (`tools/cts/log/investigations/gc-case10-flake.md`) rules
     out history-dependent placement for the colour target, which has the
     same address in every run. The next step is the KICK_DIFF instrument
     with a second read-back about 10 ms after the fence;
   - i.MX8MP case 16 (4x MSAA) (#8). **New state (`6bddfc5`):** the "failure"
     of the default image was the missing 4x config; the harness now skips
     the 4x half by name. The viewport/clip scaling to the 2x scratch is
     behind `gc_debug_msaa` bit 3 (off). On the integration image `b20e4d11`
     the A/B failed in both arms at check 4 (`x4 inside` got 0x00000000), so
     the 4x path fails for another reason, still unknown. The gl_test set
     walker now counts the skip's -3 as skipped, not failed: the default
     image's set record (`CASE_SET`) says 1 skipped, first skip case 16, and
     no longer names 16 as a failure (until then it did, 47 / 49 on
     `c4ccc966`). The 16 `multisample.*` dEQP cases stay NotSupported at 1x;
   - the Pi 4 differs from the ES 2.0 reference in 25 cube cases (#12).
     **New state (`ec5ba4f`): fixed on the host. The owner's 2026-09-25
     exclusion of these cases from the pass count is replaced by this fix.**
     The Pi 4's tree on `a27d69c0` passed 19 of them and failed 9 (6
     `mipmap.cube`, 3 `size.cube.15x15`), all the fragment stage's level;
     fixed on the host since (#12 row). It needs the Pi 4 run (T3, T4, with
     T2, TF, RT, S5f);
   - glmark2 terrain black (#1), AM62P refract's refused depth pass (#2),
     the Pi 4 carve-out (#4, #5): fixed on the host ("State of main", plan
     #2), waiting for the common-set round on each board;
   - the i.MX8MP panel showed one and a half copies of the frame per line at
     the 71.1 MHz pixel clock (no DSI margin); the device tree is back at
     69.3 MHz and the margin is a checked limit (`e372545f`); the DISPLAY
     record on `c1f0e2a6` shows the link at 69.3 MHz (lane rate 426600 kHz,
     headroom 25.9 per mille, bridge IRQ_STAT clean, health 0x3f); the
     viewing by eye is still pending (#23).
4. **ES 3.0, host first.** Goal 4. A proposed order, not a ranking anyone
   measured; the counts are AM62P failures from the `396d667a` tree where a
   design document derived them. Each item keeps every `report-es2` answer
   and adds a test that pins it.
   - **tf-capture**: write the captured varyings into the bound buffers and
     count PRIMITIVES_WRITTEN (`es3::add_primitives_written` is in place,
     not yet called). The GL half is built (`16ac864`); until this lands a
     capturing draw is refused with GL_INVALID_OPERATION.
     `docs/es3-transform-feedback.md` has the per-GPU paths: the V3D has
     hardware transform feedback, the GC7000UL has none. It unblocks the
     `transform_feedback` group and the 56 `lifetime` cases.
   - **int-rt**: each back end learns the integer render-target formats
     (`gles_surf_int_layout`, then its bits in `INTEGER_RT_FORMATS`). The
     PowerVR's is built on the host (all nineteen, one attachment;
     `docs/es3-pvr-formats.md` section 10): the proof is the AM62P, gl_test
     200 (intrt) and the dEQP-GLES3 `texture.specification`,
     `fragment_out` and `shaders.builtin_functions` buckets (736 + 522 +
     531 + 1,461 cases) -- every `builtin_functions` case renders into an
     integer renderbuffer. Left on
     the PowerVR: an integer attachment beside other attachments (the
     variant key carries one layout; `fragment_out.array`/`random` need
     per-attachment packing), colour masks that keep some of an integer
     target's channels, and precompiled rows for the integer variants (a
     table-only image refuses them by name). The GC7000 and the V3D: none.
   - **formats**: each back end samples the new storage classes of
     `docs/es3-sized-formats.md` (add the class to its
     `*_SAMPLED_TEXFMTS`), and the shader back ends gain isampler /
     usampler. The PowerVR's is built on the host (every class but
     `RGB10A2`, isampler2D/usampler2D): the proof is the AM62P, gl_test 201
     (usamp) and the same dEQP-GLES3 buckets. Left
     on the PowerVR: integer cube / 3D / 2D-array samplers (refused by name
     at compile time), and `RGB10A2` as a class (it needs the RGB10_A2
     render target: A2R10B10G10 packmode, the RGBA16F output). The GC7000
     and the V3D: none.
   - **srgb**: sRGB render targets and sampling, designed in
     `docs/es3-srgb.md` (its section 9 lists the steps); `SRGB8_ALPHA8` is
     stored and rendered as plain RGBA8 today. About 1,106 to 1,111 AM62P
     `fragment_ops.blend.fbo_srgb` failures.
   - **occlusion-query**: built on the three back ends and proven on the
     host (the feature table above); what remains is the board: gl_test 196
     `occl` on each board ("Next board session"), then dEQP-GLES3
     `functional.occlusion_query` (62) and the 4 `negative_api` cases on the
     AM62P. The unverified points, in the order a failure would point at
     them, are `docs/es3-occlusion-query.md` section 9 (PowerVR heap and
     `GETVISRESULTS`, TRANSLUCENT and the frame split, the V3D's
     accumulation across jobs, the GC7000 PE's store timing).
   - **pvr-2darray**: PowerVR 2D-array and shadow sampling, designed in
     `docs/es3-pvr-2darray.md`.
   - **es3-shaderc**: the online compiler's GLSL ES 3.00 gaps, counted over
     dEQP's `shaders/*.test` corpus in `docs/es3-shaderc-coverage.md`
     (PowerVR 3,453 / GC7000UL 3,451 / V3D 3,453 of 3,461 expected-build
     programs build; every expected-fail program refused). Host-proven: `trunc` on the PowerVR and the V3D, the
     V3D's integer shifts, min/max, `~` and unsigned orderings (gl_test 199
     es3trunc, not yet run on a board; dEQP `shaders.operator` trunc, 24
     programs); on the PowerVR a bias on a 3D texture, `textureProj` with a
     level, a bias, in a vertex shader, on a 3D texture or a 2D shadow
     sampler (divided in the shader), and a shadow look-up of a smooth
     varying (its plane gradients' W plane); on the V3D, and in the GL
     core's per-column split for the V3D and the GC7000, non-square matrix
     attributes; `centroid in` on the PowerVR (`ditrp` at the centroid, as
     pco) and the V3D (the centroid W and `centroid_flags`, as Mesa's v3d),
     and sema's refusals of `invariant` on an ES 3.00 fragment input, of a
     function declared inside a function, and of a name read past the `if`
     arm that declared it. That document's "Texture
     look-ups" section lists what each back end still refuses. Open: the
     rest of its "Work that closes the table"; of those, two need a
     decision first: the 63+-character names (the default-block rows of
     an instance block are named after the instance, which GL never
     shows, so a 1,024-character instance name fills the 64-byte field;
     naming them after the block, as GL names the members, or task #47's
     name pool), and the GC7000UL's 128-element constant array (34
     temporaries against a band ceiling of 21 measured to draw: the array
     wants the constant file with relative addressing, as etnaviv takes
     it, not a better allocation).
   - **es3-caps**: redo the ES 3.0 limits test, reverted for review
     (`3f78982`).
   - **es2-report-gaps**: the four `BUG` pins of `tests/es2_report_guard.rs`
     (ES 3.0-only pnames and capabilities answered, four ES 3.0
     renderbuffer formats accepted, ES 3.0-only pixel types
     GL_INVALID_OPERATION instead of GL_INVALID_ENUM under `report-es2`).
   - The GC7000 refuses four `ubo` cases at link for constant registers
     (`tests/es3_ubo.rs` `KNOWN_LIMITS`).
   - Then the dEQP-GLES3 tree on each board, after its ES 2.0 tree.

## Next board session

One session per board confirms every fix that waits for hardware. Nothing
below has been run from main. Two sessions ran parts of it on integration
images ("State of main"); their readings are noted per board and do not
replace the expectations. Expected values come from the host tests and the
committed records named in each line; a result that differs is a finding, not
a reason to edit the expectation. One GL client per board: check that the
board is idle first. `BOARD_BOOT=uboot-mmc` on the two Verdins.

Common to all three boards, from a detached checkout of main (the scripts
build the checkout they live in; `tools/cts/README.md` section 2d has the
long form and the stage 1 build):

```sh
git worktree add --detach <sweep> main
SW=<sweep>; HEAD8=$(git -C $SW rev-parse --short=8 HEAD)
export ZEPHYR_WS=<west workspace> WEST_VENV=$ZEPHYR_WS/.venv ZEPHYR_SDK_INSTALL_DIR=<zephyr sdk>
export BOARD_HOST=<user@host>     # the board's other variables: header of tools/board.sh
$SW/tools/board.sh --dry-run <board> tree <table> <outdir>    # first: prints every command, touches nothing
```

Every dEQP build prints `rc=0` and the `check-deqp-images.py` verdict for its
`.config` (an image that fails it reports `rc=not-conformance`). Record
`$HEAD8` with each result: an image from any other commit proves nothing here.

On every board: gl_test 196 `occl` alone (`CONFIG_GL_TEST_CASES="196"` in
the board's gl_test sample; not in case 7's regression set until it has
passed on all three). Expected: the five lines `gl_test: occl: q0 ... q4`
read `available 1` and results 0, 1, 0, 1, 1, the white quad in the centre
and the red occluder in a corner, `PASS`. q0 or q1 wrong: the counter does
not count (PowerVR: the general-heap result area or `GETVISRESULTS`; V3D:
`OCCLUSION_QUERY_COUNTER`; GC7000: ADDR/CONTROL); q3 0: the V3D's TLB does
not add across jobs; q4 0 on the PowerVR: the frame split at `glEndQuery`
is not in effect.

### i.MX8MP (Verdin, no UART: records are read from the SNVS)

Confirms `4e9397c` (fbo, fragdata), `6bddfc5` (case 16 and the bit 3 A/B), and
measures #9 and #25. Do steps 1 to 3 first (short), then 4 and 5. On the
integration image `b20e4d11` steps 1, 2, 3 and 5 ran
(`tools/cts/log/board-sessions/imx8mp-plan1.md`): step 1 as expected, step 2
46 / 48 with case 16 first (the set walker counts the skip), step 3 both arms
failed at check 4, step 5 as in "Known defects". The gl_test builds of steps
3 and 5 also need `-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y`, or `tools/board.sh`
refuses the image (rc=3), and a `;` inside a sweep list must be written `\;`
(`tools/cts/log/board-sessions/imx8mp-gc-case10.md`).

```sh
export ZEPHYR_WORKSPACE=$ZEPHYR_WS ZEPHYR_VENV=$ZEPHYR_WS/.venv IMX8MP_OUT=<out> DEQP_STAGE1=<stage 1>
$SW/tools/cts/imx8mp-deqp-stage1.sh     # only if there is no stage 1 for this CTS revision yet
```

1. **dEQP fbo and shaders-tail rows (`4e9397c`).** A two-row table from
   `imx8mp-full-tree.txt`: its entry 2 is the shaders tail (it holds
   `shaders.fragdata.*`), entry 21 is `fbo.*`; in the mini-table they are
   selectors 0 and 1.

   ```sh
   TBL=$IMX8MP_OUT/mini-table.txt
   grep -vE '^[[:space:]]*(#|$)' $SW/tools/cts/imx8mp-full-tree.txt | sed -n '3p;22p' > $TBL
   $SW/tools/cts/imx8mp-deqp-build.sh fbo-$HEAD8 'dEQP-GLES2.info.*' --table $TBL
   TREE_IMAGE=$IMX8MP_OUT/fbo-$HEAD8/zephyr/zephyr.bin TREE_PREV=records/imx8mp-tree/c2-72d16864 \
     $SW/tools/board.sh imx8mp tree $TBL <rec>/dEQP
   $SW/tools/board.sh imx8mp tally <rec>/dEQP
   ```

   (`records/imx8mp-tree/c2-72d16864` is the regression run; it is gitignored,
   so drop `TREE_PREV` if it is gone.) Expected in `<rec>/dEQP/results.tsv`:
   the fbo row 711 begun, 711 passed, 0 failed (was 710 + 1 fail at
   `72d16864`), including
   `fbo.completeness.renderable.renderbuffer.depth.depth_component24`; the
   shaders-tail row 1512 passed with 0 NotSupported (was 1511 + 1),
   including `fragdata.*`.
2. **gl_test, default image (`6bddfc5`; first sample of #25 and #9).**

   ```sh
   $SW/tools/cts/common-images.sh imx8mp $SW/tools/cts/common-tree.txt <img> GT
   for i in 1 2 3; do
     $SW/tools/board.sh imx8mp run <img>/GT/zephyr/zephyr.bin <rec>/gt gt-$HEAD8-$i
   done
   ```

   Expected (an image with the `CASE_SET` record): `set status 7 ... passed
   P of 48 run; skipped 1, first skip case 16; failing F: first case A,
   second case B`. Case 16 is the skip (without a 4x config it runs its 1x
   half and skips the 4x half by name), not a failure; the record names the
   first two failing cases, so the case that `c4ccc966`'s 47 / 49 left
   unnamed is case A. Cases 2 and 10 may still flip (#25, #9): record which
   runs fail and at which case. The earlier results (44, 45, 46 of 48 at `05f8f749`; 46
   of 48 at `72d16864` with case 16 failing) are not comparable one to one.
   Three clean runs do not show a fix of #9 or #25 (at a 1-in-3 rate that
   happens about 30 % of the time); step 5 is the measurement.
3. **4x MSAA, case 16 alone: the `gc_debug_msaa` bit 3 A/B (`6bddfc5`).** One
   image per arm with the 4x config offered (`CONFIG_GC_GL_TEST_MSAA=y` calls
   `gc_debug_set_msaa(CONFIG_GC_GL_TEST_MSAA_BITS)`), case 16 only (`SEL` is
   the case minus the first case, here 0):

   ```sh
   for bits in 1 9; do
     $SW/tools/cts/imx8mp-gltest-build.sh msaa$bits-$HEAD8 0 \
       -DCONFIG_GC_GL_TEST_MSAA=y -DCONFIG_GC_GL_TEST_MSAA_BITS=$bits
     SEL=16 $SW/tools/board.sh imx8mp run $IMX8MP_OUT/msaa$bits-$HEAD8/zephyr/zephyr.bin <rec>/msaa msaa$bits
   done
   ```

   Arm 1 is bit 3 off (the 4x path as it was before this plan), arm 9 is bit
   0 + bit 3 (viewport and clip scaled to the 2x scratch). The CASE record
   names the first bad check, got and want; the log's `x4: inside / outside /
   edge`, `x4 scratch`, `x4 clear only` and `x4 draw only` lines say which
   half failed. Expected if the hypothesis is true:

   | arm | expected | otherwise |
   |---|---|---|
   | 1 | `x4 inside` 0xff000000 (the clear colour), scratch non-zero, the `clear only` map `#` in the top-left quarter only | inside 0xffff00ff and the case passes: bit 3 was never the problem, only the config |
   | 9 | case 16 **passes**: inside 0xffff00ff, outside 0xff000000, edge 0xff800080 (+-2); the clear map covers the whole scratch | blank target with a full scratch map: the resolve (RS source, tiling) is wrong; empty scratch: the draw (PE supertiling); scrambled in 64x64 blocks or 4x4 tiles: supertile bits; triangle at half or double scale: window size or stride |

   A fence timeout or fault: `dmesg` on the Linux side after the boot, then
   tile status and `RS_EXTRA_CONFIG` are the next suspects. Only if arm 9
   passes, also run arms 11 (+ bit 1) and 13 (+ bit 2): they must not
   regress it.

   **Follow-up if arm 9 passes (code, after the session):** make the
   viewport/clip scaling the design (drop bit 3 from `gc_debug_msaa`, scale
   whenever the 4x path is on), turn `GC_MSAA_ENABLE` on, remove
   `gc_debug_msaa` from the product as Open work 2 removes the other debug
   knobs, and run dEQP `multisample.*` on a 4x-config image (the AM62P way:
   `DEQP_GL_CONFIG_NAME=rgba8888d24s8ms4`, 16 cases). If arm 9 does not pass,
   bit 3 stays off and #8 stays open with the readings above.
4. **dEQP, the whole tree (optional; at most about 5 h).** The same
   `imx8mp-deqp-build.sh wt-$HEAD8 'dEQP-GLES2.info.*' --table
   $SW/tools/cts/imx8mp-full-tree.txt`, then `TREE_IMAGE=... tools/board.sh
   imx8mp tree $SW/tools/cts/imx8mp-full-tree.txt <rec>/full`. Compare with
   `tools/cts/imx8mp-full-tree-29460f96.results.tsv`: expected 0 fail, the
   same rows as there, and the fbo and shaders-tail rows as in step 1.
5. **#9 and #25 repeat runs (`8409419`; 30 boots).** Three images on the
   regression set's list up to case 10, which reaches cases 2 and 10 (list 0,
   `SEL=512`):

   ```sh
   LIST='-DCONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10"'
   $SW/tools/cts/imx8mp-gltest-build.sh gc9-base   0 "$LIST"
   $SW/tools/cts/imx8mp-gltest-build.sh gc9-page   0 "$LIST" -DCONFIG_YAKOGL_GC_ALLOC_PAGE_ALIGN=y
   $SW/tools/cts/imx8mp-gltest-build.sh gc9-reuse0 0 "$LIST" -DCONFIG_YAKOGL_GC_POOL_NO_REUSE=y
   for arm in base page reuse0; do for i in 1 2 3 4 5 6 7 8 9 10; do
     SEL=512 $SW/tools/board.sh imx8mp run $IMX8MP_OUT/gc9-$arm/zephyr/zephyr.bin <rec>/gc9 $arm-$i
   done; done
   ```

   Read w3 of the SWEEP record (the bitmap of the cases that passed) and
   require w1 = 0 (no lost work, resets or fence timeouts), or the run is
   not a reading: `0x0000077e` every case passed; `0x0000037e` case 10 failed
   (#9); bit 2 (`0x4`) clear means case 2 failed (#25). The base arm should
   fail case 10 in about half its runs (14 of 30 earlier). Outcomes, from
   `src/vivante/VIVANTE_NOTES.md`:

   | base | page | reuse0 | reading |
   |---|---|---|---|
   | about 5/10 fail | 0/10 | about 5/10 | (b) the alignment: make page alignment the default for targets, re-measure dEQP color_clear |
   | about 5/10 | about 5/10 | 0/10 | (c) the pool's reuse: the open reuse defect above `gc_render_pool_reset` |
   | about 5/10 | 0/10 | 0/10 | (b) through (c): reuse moves a target off a page boundary; fix (b) |
   | about 5/10 | about 5/10 | about 5/10 | neither; (a) is left. Next the KICK_DIFF instrument with a second read-back about 10 ms after the fence (pixels appear: the event fired before the PE write-back landed; never: the PE dropped the dirty lines) |
   | 0/10 | | | the rate fell: run 30 more on base before reading the others |

   The single-case form (`SEL=10`, the same images) ends with case 10's CASE
   record (first failing check, got, want) and costs one boot a run; use it
   only for the first-after-boot rate (8 of 70 earlier, so about 40 runs to
   see one failure).

### Raspberry Pi 4 (tryboot, no `BOARD_BOOT`)

The whole tree ran on main `a27d69c0` (2026-10-10): 17,083 pass / 9 fail /
34 NotSupported, the nine being the fragment cube level fixed on the host
since (#12 row). This session confirms that fix. Needs
`RUSTUP_TOOLCHAIN=nightly`, stage 1 for `rpi_4b` (`DEQP_WORK`) and
`BOARD_UART` (the debug probe's device, or `host:port` of its serial
bridge) and `BOARD_HDMI_TIMINGS` as the lab's. Rows of `pi4-full-tree.txt`
(`TREE_ROWS` counts from 0, over the non-comment lines): T3 21 and T4 22
hold the nine; T2 20 (`texture.filtering.*`), TF 24, RT 11
(`shaders.random.texture.*`) and S5f 16 hold the other fragment cube
look-ups whose code changed.

```sh
export DEQP_WORK=<stage 1 for rpi_4b> RUSTUP_TOOLCHAIN=nightly
$SW/tools/cts/pi4-deqp-images.sh $SW/tools/cts/pi4-full-tree.txt <img> T2 T3 T4 TF RT S5f   # <img> must not exist yet
TREE_IMAGE_DIR=<img> TREE_ROWS=11,16,20,21,22,24 $SW/tools/board.sh pi4 tree $SW/tools/cts/pi4-full-tree.txt <rec>/cube
$SW/tools/board.sh pi4 tally <rec>/cube
```

Expected (`docs/v3d-noseamless-cube.md`, "Pi 4 verification"):

- T3: the six `texture.mipmap.cube.{basic,bias,projected}.{linear_linear,
  linear_nearest}` pass, 304 / 304; T4: the three
  `texture.size.cube.15x15_{l8,rgb888,rgba8888}` pass, 69 / 69;
- T2, TF, RT, S5f each equal their `a27d69c0` tally (all pass there);
- whole tree, if all 29 images run (`TREE_PREV=records/pi4/deqp-a27d69c0/full`):
  17,092 pass / 0 fail / 34 NotSupported;
- `v3d_gl_test` is an ES 3.0 image, which never keys the emulation: unchanged.

Reading a failure:

- T3 or T4 still fail: check that the fragment variant was built
  (`V3D_VARIANTS_BUILT` grows) and has `fdx`, `fdy`, `tmuslod`;
- a draw refused or a job failed on a bad field: the face size word found no
  size in the job builder's table (`texture_units`, v3d_job.c);
- only T2's `filtering.cube.nearest_mipmap_*_linear_*` fail: the TMU's
  magnification switch on an explicit fragment level is not GL's c = 0.5;
  key the fragment record's cube bindings for `mag_switch` and remap the
  computed level as the vertex stage does.

### AM62P (`BOARD_BOOT=uboot-tftp` or `uboot-mmc`; `BOARD_UART` set; `driver/src/fw/pvr_fw_blob.h` in the checkout)

Confirms that nothing moved after the PowerVR compiler changes since the
10/2 tree (the fusion and predication commits listed in
`docs/es2-cts-failures.md` section D, and `c6aa6ba`'s vertex attribute fix),
and that `4e9397c` turns `fragdata.invalid_assign_to_1` into a pass. The
whole GLES2 tree is re-run, since it last ran on `d60b9d76`. On the
integration image `b83216d4` images 1 to 7 ran: the fragdata case passed and
9 `shaders.invariance` cases failed ("Known defects"); with the host fix
for them (no fusion on the way to an `invariant` output) expect them to pass
in image 1. The `32778b57` tree failed the 45
`texture.vertex.2d.*` cases in image 9 (refused draws, fixed on the host by
the indexed pieces): image 9 alone (`TREE_ROWS=8`) is their confirmation,
with gl_test 197 `vtxgrid` in the regression set. The `32778b57` `ms4`
image failed `constancy_alpha_to_coverage` and `constancy_both_inverted`
(a 4x frame split by the PDS arena, fixed on the host by sharing the
frame's PDS programs): the `ms4` image confirmed them on `452d6c31` and
`a27d69c0` (16 / 16 both). gl_test 198 `a2cconst` (by number: it is not in
the set) hung the fragment DM on `452d6c31`; with the tiles in flight taken
from the pixel width it passes on `a27d69c0` (0 resets), as do 199 `es3trunc`,
200 `intrt` and 201 `usamp`. glmark2 `shadow` and `refract` sampled stale
depth (pvr-depth-resolve, fixed on the host: the draw's stamps move to the
kick holding it): the common set's GM image is their confirmation (below). The console must be the main_uart0 bridge
port: a session leased the mcu_uart0 port and captured nothing
(`tools/cts/log/investigations/am62p-console-empty.md`):

```sh
export DEQP_WORK=<stage 1>
$SW/tools/cts/am62p-deqp-images.sh $SW/tools/cts/am62p-full-tree.txt <img> $HEAD8 > <img>/build.out       # 17 images
$SW/tools/cts/am62p-deqp-images.sh $SW/tools/cts/am62p-ms4-tree.txt <img-ms4> ms4 > <img-ms4>/build.out   # 1 image
TREE_IMAGE_DIR=<img> TREE_TAG=$HEAD8 $SW/tools/board.sh am62p tree $SW/tools/cts/am62p-full-tree.txt <rec>/full
TREE_IMAGE_DIR=<img-ms4> TREE_TAG=ms4 $SW/tools/board.sh am62p tree $SW/tools/cts/am62p-ms4-tree.txt <rec>/ms4
$SW/tools/board.sh am62p tally <rec>/full
$SW/tools/board.sh am62p tally <rec>/ms4
$SW/tools/cts/deqp-uart-tally.py --mustpass $SW/tools/cts/gles2-mustpass.txt <console logs of both trees>
GLMARK2_SRC=<glmark2 at 22c527cb> $SW/tools/cts/common-images.sh am62p $SW/tools/cts/common-tree.txt <img-gm> GM
RUN_BOUND=960 $SW/tools/board.sh am62p run <img-gm>/GM/zephyr/zephyr.bin <rec>/gm gm-$HEAD8
```

glmark2 (the GM image), expected on the console capture: no line `texture
<n>: the backend did not resolve its depth (-16)` in any scene (before the
fix: one per rendered frame of `shadow` and `refract`, when the window
clear's kick had completed by the depth pass, the usual case), the verdict
`33 ran / 0 failed` as before, and by eye on the display the horse's shadow
on the ground in `shadow` and the refracted floor in `refract`. A `-16` line
still there is a finding: the board's kick order differs from the host's
(`tools/cts/log/hostharness/glmark2/FINDINGS.md`, "Diagnosis of rows
23-24").

Expected: combined mustpass 0 fail, the `fail` column of all 17 image rows 0
and of `ms4` 0, no case worse than at `d60b9d76` (17,141 pass / 0 fail / 19
NotSupported / 5 warnings of 17,165; RESULTS 1z2). If fragdata
`invalid_assign_to_1` passes (image 1, the table's first row, `TREE_ROWS=0`), that is 17,142
pass and 18 NotSupported. A case that moves from Pass to anything else is a
regression of the compiler changes since `d60b9d76`: bisect it first. If time
is short, run image 1 alone (`TREE_ROWS=0`) for the fragdata check and
images 1 to 7 (the shaders groups, `TREE_ROWS=0-6`) for the compiler. Time: at most about 11
min an image, 18 images.

ES 3.0 formats and integer render targets (`074c48f`, `1779e02`, `03340c8`;
host only so far): a gl_test image with `GL_TEST_CASES="200,201"` (the
online compiler's image; a table-only image refuses the integer variants by
name). Expected: case 200 intrt 16/16 checks (RGBA8UI and RGBA32UI: the
cleared integers, the drawn uvec4 with the column in alpha, a scissored draw
after a clear, a draw with no clear over the reloaded pixels), case 201
usamp 8/8 (RGBA8UI through usampler2D, RGBA8I sign-extended through
isampler2D). Then the dEQP-GLES3 `texture.specification`, `fragment_out`
and `shaders.builtin_functions` buckets (736 + 522 + 531 + 1,461 cases)
against their last numbers: `fragment_out.array.*`/`random.*` with an
integer attachment beside another stay refused by name.

When the three sessions are done, replace each board's row in
`docs/es2-cts-failures.md` (the summary table, section A and the "Status
after plan #1's fixes" table), `tools/cts/STATUS.md` and
`tools/cts/RESULTS.md` with the new numbers and record names. Nothing is
board-confirmed before then.

## How to work here

- The records prove the framebuffer, not the panel: a display change needs
  someone looking at the panel (#23).
- The gate is `tools/check.sh`. Run it from a clean detached checkout of
  the exact commit, with `driver/src/fw/pvr_fw_blob.h` copied in, and with
  `ZEPHYR_WS=<west workspace>`, `HAL_NXP_DIR=<hal_nxp>` and
  `GLMARK2_SRC=<glmark2 checkout at 22c527cb>` set. Without them the Zephyr,
  hal_nxp and glmark2 rows are gaps and the gate is red. The glmark2 shader
  corpus is not committed: `tools/hostharness/glmark2/run.sh` generates it from
  `GLMARK2_SRC` into `build/glmark2-host/shaders`, and
  `tests/glmark2_corpus.rs` reads `GLMARK2_CORPUS` (default that path) on the
  three GPUs. `tools/setup-gate-host.sh` prepares a gate host. The
  Mesa-table reproduction row also needs `PCO_SHC=<pco_shc from
  tools/pco_shc/build_pco_shc.sh, content id f537fc86b41f>`,
  `GLSLANG=<glslangValidator>` and, for speed, `GLSL2USC_CACHE=<dir>`;
  without them that row is a gap and the gate is red. It takes about 45
  minutes.
- Board runs use `tools/board.sh <board> run <zephyr.bin> <record dir>
  <label>`. Run them from a detached checkout of main, and only with images
  built from commits on origin/main. Use `BOARD_BOOT=uboot-mmc` on the
  Verdins.
- A dEQP image bakes its table in: splitting rows needs a rebuilt image
  (`tools/cts/imx8mp-deqp-build.sh --table`; tools/cts/README.md).
- One GL client per board. Check that a board is idle before you use it.
