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
case names come from the prose of RESULTS.md and from the `.tsv`. Updated
2026-10-11 with the 2026-10-10 trees of the AM62P and the i.MX8MP and the Pi 4
rows of 2026-10-10 and 10-11 (`tools/cts/RESULTS.md`).

## Decision that is superseded

On 2026-09-25 (16:52) the owner left the 25 Pi 4 cube cases out of the "all
pass" count (RESULTS.md, "Reference-selection difference"). The goal of plan
#1 is zero failures. That decision no longer holds. The 25 cases are now to be
fixed (items `v3d-cube-design`, `v3d-cube-fix`).

## Summary

| Board | Last full tree | Date | Pass | Fail | NotSupported | QualityWarning |
|---|---|---|---:|---:|---:|---:|
| AM62P (PowerVR) | `32778b57` | 2026-10-10 | 17,070 (was 17,141 at `d60b9d76`) | 54 (was 0) | 34 (was 19) | 7 (5 QualityWarning, 2 CompatibilityWarning) |
| i.MX8MP (GC7000) | `32778b57` | 2026-10-10 | 17,112 to 17,116 | 0 | 30 to 34 | 19 |
| Pi 4 (V3D) | `a27d69c0` tree, with the nine cube cases and the F1 rerun on later builds (composed, section A) | 2026-10-10 and 10-11 | 17,126 (was 17,075) | 0 (was 25) | 34 (was 62) | 5 (was 3) |

Each row adds up to 17,165. The i.MX8MP range exists because the run covered
206 cases that are not in the mustpass list, and four of them sit in the
NotSupported group (see below). Every i.MX8MP selector equals the `d8583f20`
tree's (2026-10-01).

The AM62P's 54 are regressions against `d60b9d76`: 9
`shaders.invariance.*.common_subexpression_*` (image 1; fixed in `e17fb6a7`,
and image 1 on `e17fb6a7` passes 1,379 / 0 on the board) and 45
`texture.vertex.2d.*` (image 9; fixed on the host in `a6fc6f14`, awaiting the
board). Its NotSupported 34 count the 16 `multisample.*` cases of the 1x
config; the ms4 image measures those, 14 / 2 on `32778b57` and 16 / 16 on
`452d6c31` after the PDS fix (`a4f64950`). The i.MX8MP's two regressions of
`72d16864` (a 12-row subset) are fixed on the board (the `32778b57` tree: fbo
711 / 711, the shaders tail 1,512 pass, 0 NotSupported). gl_test (our own
suite, not dEQP) on the i.MX8MP: 47 of 48 run with case 16 skipped, case 44
failing (#6), and intermittent failures of cases 2 and 10 (see section E).

The Pi 4 row is composed of three records, not one image set (section A): the
whole tree on `a27d69c0` (17,083 pass, 9 fail, 34 NotSupported, 3
QualityWarning, 2 CompatibilityWarning, 34 cases without a result), whose 9
failures pass on `1cf946a9` and whose 34 lost cases pass in a rerun. The 5 in
the QualityWarning column are those 3 and the 2 CompatibilityWarnings
(`flush_finish.flush`, `flush_wait`; a timing check that flips between runs).

Newer records (Round 1, 2026-10-12 note): the rows above are the origin/main
readings of 2026-10-10 and 10-11. The only newer full record in this tree is
the i.MX8MP integration tree `334d4e65` (same 17,318 / 0 / 34 / 19). The
AM62P has a partial one, images 8-17 on `f013dbbb` (45 fail, the same 45
`texture.vertex.2d` cases), and the Pi 4 a partial one, six rows on
`1cf946a9` (934 / 0). None is a final tree, so no row is a zero-failure
claim; see "Round 1" below.

## Status after plan #1's fixes (docs-sync, 2026-10-07)

Sections B to F below are the inventory as written before the fixes. This
section is the current state of each failure (updated 2026-10-11). "Fixed on
the host" means a host test passes at that commit; a row says when a board has
run it. The combined board recipe is `docs/HANDOFF.md`, "Next board session".

| Failure (section B/C/E row) | State | Commit and evidence |
|---|---|---|
| i.MX8MP `fbo...renderbuffer.depth.depth_component24` (B row 1) | **fixed and confirmed on the board** (`32778b57` tree: fbo 711 / 711) | `4e9397c` (`es2-gl-core`). In `report-es2` builds `glRenderbufferStorage(GL_DEPTH_COMPONENT24)` raises GL_INVALID_ENUM unless `GL_OES_depth24` is listed (`state::extension_listed`, ungated; `advertises()` calls it, so a GPU that lists the extension later gets the format with no further code change). Host tests `depth_component24_renderbuffer_follows_the_extension_list` (`src/gl/framebuffer.rs`), `draw_buffer_queries_are_es3_only` (`src/gl/state.rs`). Board expectation: fbo row 711 pass, 0 fail (was 710 + 1 fail at `72d16864`). |
| i.MX8MP `shaders.fragdata.*`, one NotSupported (B row 2) | **fixed and confirmed on the board** (`32778b57` tree: the shaders tail 1,512 pass, 0 NotSupported) | `4e9397c`. In `report-es2` builds `GL_MAX_DRAW_BUFFERS` and `GL_DRAW_BUFFERi` are refused and `gl_MaxDrawBuffers` is 1 (`src/shaderc/online/lower.rs`, host test `deqp_fragdata_uniform_index_compiles_in_an_es100_context`). Board expectation: shaders tail row 1512 pass, 0 NotSupported (was 1511 + 1). |
| `shaders.fragdata.invalid_assign_to_1` on the AM62P and the Pi 4 (C) | **passes on the board** (AM62P `32778b57`, Pi 4 `a27d69c0`) | Same commit: the case needs exactly one draw buffer, which an ES 2.0 context now reports. |
| Pi 4 25 cube cases (B rows 3 to 27, #12) | **fixed and confirmed on the board** (0 of 25 fail) | design `c3e4146` (`docs/v3d-noseamless-cube.md`; failing-then-passing host test `tests/v3d_cube_noseamless.rs`), implementation `ec5ba4f`, review fixes `53ef6d7`, doc `d8ce931`. In ES 2.0 builds a cube look-up whose filters can see seams is compiled as a 2D-array look-up of the selected face (`LodKey::cube_noseam`; `LodKey::mag_switch` narrowed to bits 32..47). Host evidence: 7 tests in `tests/v3d_cube_noseamless.rs` on the QPU/TMU interpreter in both builds, `an_unkeyed_program_is_unchanged` (FNV-1a of the code and uniform streams equal to `0252e9d`), backend and draw unit tests. The owner's 2026-09-25 exclusion of these cases is replaced by this fix. The Pi 4's tree on `a27d69c0` (2026-10-10) answered the TMUI question (float: 19 of the 25 pass) and failed 9: the 6 `mipmap.cube.*.linear_*` and `size.cube.15x15_{l8,rgb888,rgba8888}`, all the fragment stage's level from the per-face (s, t) in quads that straddle a face edge. Fixed since: the fragment variant computes dEQP's cube level from FDX / FDY of the direction and the face size word (`QUNIFORM_TEXTURE_WIDTH`, now filled by the job builder) and retires with TMUSLOD (`docs/v3d-noseamless-cube.md`). Board, main `1cf946a9` (2026-10-11): T3 304 / 304 and T4 69 / 69 pass (`records/pi4/deqp-1cf946a9/rows`, tally `build/pi4-rows-tally-1cf946a9.txt`), as do the other four rows whose fragment cube look-ups changed (RT 200, S5f 100, T2 248, TF 20 cases): 941 pass, 0 fail, 0 lost over the six rows, 934 of them in the mustpass list. |
| i.MX8MP gl_test case 16 (#8) | harness fixed on the host; the 4x path itself open | `6bddfc5` (`gc-msaa`). It failed in the default image only because that image offers no 4x config: `CONFIG_GC_GL_TEST_MSAA` is off, `gc_debug_msaa` is 0, `eglChooseConfig samples 4` reads 0 and the case returns before any draw. "Check 255" is the record's none-marker (`gc_result_case` stores 255 for "no check"; a SET_FAIL record has no check). Case 16 now runs its 1x half and skips its 4x half by name (-3, not a failure) when no 4x config is offered; the set walker counts that skip as skipped, and the set's record (`CASE_SET`) says so (until then it counted it as a failure: 47 / 49 on `c4ccc966`). Separately the driver did not scale viewport, scissor and clip to the 2x scratch surface (found on the host; explains "the resolve writes nothing"): the fix is behind `gc_debug_msaa` bit 3, off by default, host test `msaa_bit_three_scales_the_viewport_and_clip_to_the_scratch_samples`. Nothing proves yet that the core wants it. Awaiting the 4x-config A/B (HANDOFF). |
| i.MX8MP `multisample.*` NotSupported (C) | still open, acceptable at 1x | Depends on bit 3 passing, then the design change and a dEQP run on a 4x-config image. No board proof. |
| i.MX8MP gl_test case 10 (#9) and case 2 (#25) | cause not found; diagnostic recipe, no product change | `8409419` (`gc-nondeterminism`). Reading our sync sequence against etnaviv's (12 steps; `src/vivante/VIVANTE_NOTES.md`, "Draw, resolve, read back") finds no missing stall, flush or CPU cache operation: each etnaviv step has a counterpart in ours, at the same place or earlier, with the same or a larger flush set. Two host tests pin our column word for word (`a_frame_flushes_and_stalls_as_etnaviv_does`, `a_submit_flushes_and_stalls_before_its_event_as_etnaviv_does`). Three surface hypotheses, none proven: (a) a linear PE target, (b) 64-byte target alignment against etnaviv's page-aligned buffers, (c) placement that depends on the pool's history. The board recipe (10 runs of each of three images, `CONFIG_YAKOGL_GC_ALLOC_PAGE_ALIGN`, `CONFIG_YAKOGL_GC_POOL_NO_REUSE`) separates (b), (c) and "neither". |
| Compiler changes since the last full trees (D) | host check done; one wrong-code defect found and fixed; the AM62P `32778b57` tree then showed 9 invariance failures, fixed in `e17fb6a7` and passing on the board | `c6aa6ba` (`compiler-regression-host`), see the paragraph under section D. The invariance cause: the PowerVR fused `a * b + c` into one `fmad` only where the product had one reader; a sum on the way to an `invariant` output is no longer fused. |
| AM62P `texture.vertex.2d.*`, 45 (the `32778b57` tree, image 9) | fixed on the host, awaiting board | `a6fc6f14`: an indexed draw whose client data is over half the 1 MiB general arena is cut into pieces of its index list; gl_test 197 `vtxgrid` draws the same grid. |
| AM62P ms4 `multisample.constancy_alpha_to_coverage`, `constancy_both_inverted` (the `32778b57` ms4 image) | **fixed and confirmed on the board** (16 / 16 on `452d6c31`) | `a4f64950`: a frame uploads a PDS program it already holds once, so dEQP's 1024-draw 4x frame is one kick. |
| dEQP image recipes against the current defaults (D risk list) | checked on the host | `e6e5f14`, `292310c`, `688a574` (`deqp-images-current`, `deqp-config-check-test`): `tools/cts/check-deqp-images.py` checks symbols, conformance options, carve-outs and table coverage, on the recipes and on each built image's `.config`; `board_selftest` pins 206 checks; fixture `.config` files in `tools/cts/testdata/deqp-config`. AM62P 4x image table `tools/cts/am62p-ms4-tree.txt`. |
| Pi 4 27 `fbo...attachment_combinations` NotSupported (C) | **pass on the board** (`a27d69c0` tree) | No change in this plan: the V3D claims `GL_OES_depth_texture` (tools/cts/log/status-2026-09-25-to-10-07.md, 10/3). |
| #22 GC7000 `matattr` table, #23 panel | unchanged, outside this plan | |
| Acceptable buckets of section C (compressedtexsubimage2d 9, clip_control 8, multisample at 1x, `multisampled_render_to_texture.readpixels`, the QualityWarnings) | acceptable, unchanged | Not touched. |

Every board has run a full tree since the plan began (the summary table). The
rows still "awaiting board" are confirmed in one session per board (HANDOFF
"Next board session").

## Round 1 (2026-10-08 to 2026-10-11)

Written 2026-10-12 for roadmap stage 1 (zero dEQP-GLES2 failures on the
AM62P, the i.MX8MP and the Pi 4). Nothing was run for this section. It folds
in board-session and investigation documents that are merged in this tree
(`tools/cts/log/board-sessions/`, `tools/cts/log/investigations/`,
`docs/board-sessions/`), and no other source. Sections A to F stay as the
historical inventory of 2026-10-07; the Summary and Status tables above
already carry the origin/main readings of 2026-10-10 and 10-11.

**No board has a recorded final tree.** A final tree is a whole-tree run of
one origin/main tip that closes the plan. The only final-tree record in this
tree, `docs/board-sessions/pi4-es2-final.md`, ran nothing. So this file
makes no zero-failure claim for any board. The i.MX8MP reads 0 failures on
two commits (`334d4e65`, `32778b57`); that is a reading, not a final tree.
`am62p-es2-final.md`, `imx8mp-es2-final.md` and `pi4-es2-tip-r2.md` are not
in this tree and are not cited.

### Per board: what ran

| Board | Commit (date) | Rows covered | Pass / Fail / NotSupported / QualityWarning | Record (commit) |
|---|---|---|---|---|
| AM62P | integration `b83216d4` (10-08) | images 1-7 (10,101 mustpass cases in their filters) | 10,092 / **9** / 0 / 0; every case has a result | `tools/cts/log/board-sessions/am62p-plan1.md` (`7a57c78`) |
| AM62P | integration `f013dbbb` (10-11) | images 8-17 (7,064 mustpass cases), `ms4`, gl_test default list, gl_test 171 | images 8-17: 6,978 / **45** / 34 / 5 (+ 2 CompatibilityWarning); `ms4`: 14 pass / **2 fail**; gl_test 48 of 48; case 171 alone passes; 0 resets | `tools/cts/log/board-sessions/am62p-es2-r1c.md` (`c2dfdc0`) |
| AM62P | integration `47ad0712` (10-09), and `89d518ee` / `64e1714e` (10-11) | none: console silent (the lease named the wrong UART, later fixed), nothing built or run. Covers the sessions `am62p-es2-r1` and `am62p-es2-tip-img1-7` | - | `tools/cts/log/board-sessions/farm-console-blocked-2026-10.md` (`b406de0`) |
| AM62P (main) | origin/main `32778b57` (10-10) | whole tree, 17 images, `ms4`, gl_test | 17,070 / **54** / 34 / 5 (+ 2 CompatibilityWarning); `ms4` 14 / **2** | no session document: `docs/HANDOFF.md` "State of main", `tools/cts/RESULTS.md` |
| AM62P (main) | origin/main `e17fb6a7` (10-10); `452d6c31`, `a27d69c0` (10-10) | image 1; the `ms4` image | image 1: 1,379 / 0 / 0 / 0; `ms4`: 16 pass / 0 fail | no session document: `docs/HANDOFF.md`, `tools/cts/RESULTS.md` |
| i.MX8MP | integration `334d4e65` (10-09) | whole tree, 29 entries, one image; gl_test x3 | 17,318 / **0** / 34 / 19 of 17,371 begun (206 are not in the mustpass list); every entry equal to `tools/cts/imx8mp-full-tree-29460f96.results.tsv`; gl_test 46, 45, 45 of 48 | `tools/cts/log/board-sessions/imx8mp-es2-r1.md` (`348de58`) |
| i.MX8MP (main) | origin/main `32778b57` (10-10) | whole tree, 29 entries | the same 17,318 / **0** / 34 / 19, 29 of 29 entries equal | no session document: `tools/cts/RESULTS.md`, `tools/cts/STATUS.md` |
| Pi 4 | integration `7bc37edc`, `c4a4d7fa`, `e3efeaad` (10-08 to 10-11) | none: no inputs on the worker, then no console path; nothing built or run. Covers `pi4-es2-r1` and `pi4-es2-r1-tip` | - | `tools/cts/log/board-sessions/farm-console-blocked-2026-10.md` (`b406de0`) |
| Pi 4 | integration `76d349c4` (10-11) | none: a console-bridge probe. The connection was accepted, 0 bytes while idle, and a marker written on the board's console UART did not arrive as text | - | `tools/cts/log/board-sessions/farm-console-blocked-2026-10.md` (`b406de0`, row `pi4-console-probe`) |
| Pi 4 | integration `89d518ee` (10-11) | none: the host-written comparison baseline for the next Pi 4 tree (the `fe5e8e73` per-row table, expected changes) | the `fe5e8e73` figures (17,075 / 25 / 62 / 3), not re-measured | `tools/cts/log/board-sessions/pi4-es2-r2-baseline.md` (`9179ffa`) |
| Pi 4 | origin/main tip `2a3a36fc` (10-12) | none: no console value could be read, the lease is shared; nothing built or run. It found no `pi4-es2-tip-r2` record to cite | - | `docs/board-sessions/pi4-es2-final.md` (`1b9452e`, `c9b4f8b`) |
| Pi 4 (main) | origin/main `a27d69c0` (10-10), rows on `1cf946a9` (10-11) | whole tree (29 images) on `a27d69c0`; six rows on `1cf946a9` | `a27d69c0`: 17,083 / **9** / 34 / 3 (+ 2 CompatibilityWarning, 34 without a result, which pass in a rerun); the six rows on `1cf946a9`: 934 / 0 | no session document: section A (Pi 4), `docs/HANDOFF.md` |

Reading the table:

- The AM62P's two integration readings are of different commits (`b83216d4`
  images 1-7, `f013dbbb` images 8-17); 10,101 + 7,064 = 17,165 cases are
  covered, but not at one commit, so they are not a whole-tree total. The
  only whole AM62P tree of the round is origin/main `32778b57`, before the
  host fixes of the 45 and the 2.
- The Pi 4 has no board-session record of any run. Its readings are the
  origin/main ones already in the Summary. The Pi 4 numbers of `a27d69c0`
  add up to 17,165 only with the 34 cases the capture lost
  (17,083 + 9 + 34 + 3 + 2 + 34 = 17,165); those 34 pass in a rerun of the
  F1 image, by that image's own counters. There is no per-image record in
  the repository, so the figures are quoted from section A. `pi4-es2-r2-baseline.md` compares against `fe5e8e73`,
  because the integration branch did not carry the `a27d69c0` run.
- A reading on an "integration image" is not a landing proof.

### Per failure

State vocabulary. **Fixed on origin/main, awaiting the final tree**: the
commit is on origin/main and a board has read it, or it is host-proved; no
final tree has confirmed it. **Host-fixed, in review**: a fix or a pin
exists in an unmerged item. **Diagnosed only**: the cause or the expected
verdict is written down, no fix is needed or made. **Open**: not fixed.

| Failure (cases) | Board and record | State | Evidence and commit |
|---|---|---|---|
| `shaders.invariance.{lowp,mediump,highp}.common_subexpression_{1,2,3}` (9), a regression against `d60b9d76` | AM62P image 1: `am62p-plan1.md` (`b83216d4`); again in the main tree `32778b57` (image 1: 1,370 / 9) | **fixed on origin/main, awaiting the final tree** (board-read) | `f9688bc5` (a sum on the way to an `invariant` output is not fused into an `fmad`), merged by `e17fb6a7`. Board: image 1 at `e17fb6a7` 1,379 pass / 0 fail. `tools/cts/log/investigations/es2-invariance.md` (`13eb8a9`) is the diagnosis; main took the alternative that record lists, and `tests/es2_invariance.rs` runs all twelve cases. Closed by `am62p-es2-final-a` (image 1) |
| `texture.vertex.2d.{filtering (36), wrap (9)}.*` (45), a regression against `d60b9d76` | AM62P image 9: `am62p-es2-r1c.md` (`f013dbbb`: "Image comparison failed", all 45 and nothing else in the image); main `32778b57` image 9 | **fixed on origin/main, awaiting the final tree** (host-proved; `tools/cts/STATUS.md` says the image 9 confirmation is not done) | Cause (`tools/cts/log/investigations/es2-pvr-vertex-texture.md`, `985592b`): each cell is one `glDrawElements` of 127 x 127 quads, 1,225,804 bytes of client data, refused by the 1 MiB GENERAL arena (`dcaafa41`; 4 MiB at `d60b9d76`). The record proposed a frame of its own for such a draw; main took `a6fc6f14` (an indexed draw over half an empty arena is cut into pieces of its index list; gl_test 197 `vtxgrid`). A draw that cannot be cut is still refused by name. Closed by `am62p-es2-final-a` (image 9) |
| `multisample.constancy_alpha_to_coverage`, `multisample.constancy_both_inverted` (2), on the `ms4` image | AM62P: `am62p-es2-r1c.md` (`f013dbbb`: 14 / 2); main `32778b57`, two identical runs | **fixed on origin/main, awaiting the final tree** (board-read: `ms4` 16 / 0 on `452d6c31` and `a27d69c0`, per HANDOFF) | `a4f64950` (a frame uploads a PDS program it already holds once, so dEQP's 1024-draw 4x frame is one kick; the 768 KiB PDS arena had split the frame at draw 1015). `tools/cts/log/investigations/es2-pvr-ms4-constancy.md` (`52c322f`) is superseded: it did not find this cause, and keeps three program-level host tests. Closed by `am62p-es2-final-b` (`ms4` tree) |
| `shaders.fragdata.invalid_assign_to_1` (NotSupported at `fe5e8e73` on the Pi 4, and before `4e9397c` on the AM62P) | AM62P: Pass in `am62p-plan1.md` (`b83216d4`) and in `32778b57`; Pi 4: Pass on `a27d69c0` per section A | **fixed on origin/main, awaiting the final tree** (board-read on the AM62P and the Pi 4) | `4e9397c` (`gl_MaxDrawBuffers` 1 in `report-es2`). `tools/cts/log/investigations/es2-fragdata.md` (`a118943`), host pin `tests/es2_fragdata.rs`. Closed by `am62p-es2-final-a` and `pi4-es2-final-a` |
| i.MX8MP `fbo...renderbuffer.depth.depth_component24` and one `shaders.fragdata.*` NotSupported (section B rows 1 and 2) | i.MX8MP: `imx8mp-es2-r1.md` (`334d4e65`: fbo and shaders-tail entries equal to the baseline, 0 fail); the same on main `32778b57` | **fixed on origin/main, awaiting the final tree** (board-read on two commits) | `4e9397c`. Closed by `imx8mp-es2-final` |
| Pi 4 `texture.vertex.cube.{filtering (10), wrap (8)}` and `shaders.texture_functions.vertex.texturecubelod` (19 of section B rows 3 to 27) | Pi 4 main `a27d69c0`: all 19 pass (section A); `fe5e8e73`: Fail | **fixed on origin/main, awaiting the final tree** (board-read on `a27d69c0`) | `ec5ba4f`, `53ef6d7` (non-seamless cube emulation; `docs/v3d-noseamless-cube.md`). `pi4-es2-r2-baseline.md` expects them Fail to Pass. Closed by `pi4-es2-final-a` (rows 19 and 24) |
| Pi 4 `texture.mipmap.cube.{basic,bias,projected}.{linear_linear,linear_nearest}` (6) and `texture.size.cube.15x15_{l8,rgb888,rgba8888}` (3) | Pi 4 main `a27d69c0`: 9 fail; main `1cf946a9`: T3 304 / 304 and T4 69 / 69 pass (section A) | **fixed on origin/main, awaiting the final tree** (board-read on the rows on `1cf946a9`; no whole-tree run of that commit) | `9b24ea2` (the fragment variant computes dEQP's cube level from FDX / FDY of the direction and the face size word); `tests/v3d_cube_noseamless.rs`. Closed by `pi4-es2-final-a` (T3 row 21, T4 row 22; T2, TF, RT, S5f unchanged) |
| Pi 4 `fbo.completeness.attachment_combinations.<a>_<b>_<c>_tex` (27 NotSupported at `fe5e8e73`) | Pi 4 main `a27d69c0`: pass (section A) | **fixed on origin/main, awaiting the final tree** (board-read on `a27d69c0`; no code change in this plan) | The V3D claims `GL_OES_depth_texture` (`tools/cts/log/status-2026-09-25-to-10-07.md`, 10/3). The host investigation of the 81 combinations (`es2-v3d-fbo-combos`) is not in this tree. Closed by `pi4-es2-final-a` (FB1 row 4) |
| `texture.completeness.{2d,cube}.npot_*`, `negative_api.texture.generatemipmap_npot_wdt_hgt` (18, section H) | i.MX8MP: Pass (`imx8mp-es2-r1.md`: entry 10 40 / 40; entry 22 237 pass / 9 NotSupported). AM62P and Pi 4: none among the named failures | no failure on any board; **diagnosed only** for the AM62P and the Pi 4 by name | Section H's first prediction (Pass to NotSupported) is withdrawn there. Host replay `tests/es2_npot.rs` is in the tree. The investigation record `es2-npot-cases` is not in this tree. Residual risk no record has hit: the PowerVR TPU's level arithmetic for `2d.npot_size`, the GC7000's REPEAT on NPOT sizes |
| `flush_finish.flush`, `flush_finish.flush_wait` (CompatibilityWarning) | AM62P: `am62p-es2-r1c.md`, main `32778b57`; Pi 4 `a27d69c0` | **open**, acceptable (not a failure) | dEQP's own timing check; it flips between runs (none at `d60b9d76`, two at `32778b57` and `f013dbbb`) |
| i.MX8MP gl_test cases 10 (#9), 2 (#25), 16 (#8) | `imx8mp-es2-r1.md` (46, 45, 45 of 48; case 10 first in two of three), main `32778b57` (45, 46, 46) | **open** (not a dEQP case) | Case 16 is the skipped 4x half, fixed in the count by `6bddfc5`. The cause of #9 and #25 is not found (`8409419`); the later round is `tools/cts/log/board-sessions/imx8mp-gltest-r2.md` (`938e29a`) |
| AM62P gl_test 198 `a2cconst`, 197 `vtxgrid`, 171 `viewwin` | `am62p-es2-r1c.md`: 48 of 48 default list; 171 alone passes | 171: passes at `f013dbbb` (the earlier stop is not reproduced); 198: **fixed on origin/main, awaiting the final tree** (`a4f64950`); 197: no board reading | `am62p-es2-final-b` runs 198, 197 and 171 |
| `negative_api.texture.compressedtexsubimage2d_*` (9), `clip_control.*` (8), `multisampled_render_to_texture.readpixels` (1), `multisample.*` at the 1x config (16), the QualityWarning buckets | all three boards, unchanged in every reading of the round (AM62P 34 NotSupported and 5 QualityWarning; i.MX8MP 34 and 19; Pi 4 34 at `a27d69c0`) | acceptable buckets, unchanged (section C) | No commit |

Groups that no record of the round shows failing:

| Group | What the sources say | State |
|---|---|---|
| `shaders.preprocessor.*`, `define_value_and_function_{fragment,vertex}`, macro order | `tools/cts/log/investigations/es2-preprocessor.md` (`0d7af86`): the two failures were one stale row of `RESULTS.md`; all 534 preprocessor shaders get the CTS's compile outcome on the host. No record of the round fails a preprocessor case | diagnosed only (no defect) |
| ES error rules (ES 3-only values accepted by a `report-es2` build) | `tools/cts/log/investigations/es2-error-rules.md` (`47ab9b2`): status open; the vertex-array part landed (`tests/es2_error_rules_vertex.rs`); the other ungated ES 3.0 rows are reached by no dEQP-GLES2 case. Host `negative_api` is 237 pass / 0 fail / 9 NotSupported on all three back ends | conformance debt, not a CTS failure; open |
| `glClear` with scissor and mask on the V3D | Not in a record of this tree (`es2-v3d-clear` is not merged here). No `color_clear` or `depth_stencil_clear` case is among the failures of any reading | no failure seen; diagnosis not in this tree |
| Offline shader table | Not in a record of this tree (`es2-offline-table`). The dEQP images use the on-device compiler, so this is not a CTS case | outside the CTS; diagnosis not in this tree |
| Compile and link of the whole mustpass list on the host | `tools/cts/log/investigations/es2-host-deqp-v3d.md` (`1cc069b`): 0 gaps and 878 correct refusals on each back end; the V3D run aborted one chunk at the host's 8 MiB tiled-copy budget, and at the Pi 4 image's 16 MiB all 100 cases begin. Host only: compile and link, no rendering | diagnosed only (no gap) |

### Which final session is expected to close which rows

The final sessions build the origin/main tip. Until a record exists, none of
these rows is closed.

- `am62p-es2-final-a` (images 1-9): the 9 invariance cases (image 1), the 45
  `texture.vertex.2d` cases (image 9), `fragdata.invalid_assign_to_1`; images
  2-8 against `d60b9d76` for anything new.
- `am62p-es2-final-b` (images 10-17, `ms4`, whole-tree total, then gl_test
  198, 197, 171): the 2 `ms4` constancy cases, the NotSupported and
  QualityWarning buckets, the `flush_finish` warnings, and the AM62P
  whole-tree total, which only the two sessions together give.
- `imx8mp-es2-final` (29 entries): the i.MX8MP zero-failure reading on a
  final tree, and the 18 NPOT cases by entry. The gl_test flake is not its
  business (`imx8mp-gltest-r2`).
- `pi4-es2-final-a` (rows 0-28): the 9 cube cases (T3 row 21, T4 row 22),
  the 19 vertex cube cases (rows 19 and 24), the 27 `fbo` combinations
  (FB1 row 4), `fragdata.invalid_assign_to_1`, and the Pi 4 whole-tree total.
  Its first attempt (`docs/board-sessions/pi4-es2-final.md`) ran nothing: no
  console value, a shared lease. It found no complete `pi4-es2-tip-r2`
  record to cite, so the tree has to run.

## A. Per board: last full tree, counts, record names

### AM62P (PowerVR BXS-4-64)

- Latest: `32778b57`, 2026-10-10, 17 images and the ms4 image
  (`records/am62p/deqp-32778b57/full/`, `ms4/`): **17,070 pass / 54 fail / 34
  NotSupported / 5 QualityWarning / 2 CompatibilityWarning**, 0 resets, every
  case with a result. RESULTS.md, "AM62P: the whole dEQP-GLES2 tree on
  origin/main `32778b57`: 54 fail (2026-10-10)" and the ms4 entry after it.
  The 54 and their fixes: the summary above. The previous tree follows.
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

- Latest: `32778b57`, 2026-10-10, the 29 selectors
  (`records/imx8mp/deqp-32778b57/full`): the same numbers as `d8583f20` below,
  selector by selector (diffed, 29 of 29). RESULTS.md, "i.MX8MP: the whole
  dEQP-GLES2 tree on Zephyr at origin/main `32778b57`: 0 fail (2026-10-10)".
  The previous tree follows.
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

- Current: **17,126 pass / 0 fail / 34 NotSupported / 5 warnings of 17,165**
  (2026-10-10 and 10-11; sweep 30 below was 17,075 / 25 / 62 / 3). Three
  records, all on main builds:
  - the whole tree on `a27d69c0` (`records/pi4/deqp-a27d69c0/full/`, tally
    `build/pi4-tally-a27d69c0.txt`): 17,083 pass, 9 fail, 34 NotSupported, 3
    QualityWarning, 2 CompatibilityWarning, 34 without a result. The 9 are
    the fragment cube look-ups' level (6 `texture.mipmap.cube.*.linear_*`,
    3 `texture.size.cube.15x15_{l8,rgb888,rgba8888}`); the 18 vertex cube
    cases and `texturecubelod` that failed at sweep 30 pass;
  - the six rows whose fragment cube look-ups changed (RT, S5f, T2, T3, T4,
    TF) on `1cf946a9` (`records/pi4/deqp-1cf946a9/rows`, tally
    `build/pi4-rows-tally-1cf946a9.txt`): 934 mustpass cases, 934 pass, 0
    fail, 0 lost; the 9 pass;
  - the 34 cases of F1 (`fragment_ops.blend.equation_src_func_dst_func`,
    `reverse_subtract_src_alpha_saturate_*` and neighbours) the capture lost
    on `a27d69c0` pass in the rerun `records/pi4/deqp-a27d69c0/F1-rerun2`
    (the image's own counters: 1,923 cases, all passed). The capture lost
    cases again in the rerun (260, a different range) and the tally reads
    them as Lost, not as failures: the usb1 TCP serial bridge drops a burst
    when the connection stalls, so a missing result line is not a result
    until the image's counters say so (the fix belongs to the bridge
    firmware).
  The sum: 17,083 + 9 + 34 = 17,126 pass; 34 NotSupported (`clip_control` 8,
  `multisample` 16, `multisampled_render_to_texture.readpixels` 1,
  `negative_api.texture.compressedtexsubimage2d_*` 9); 5 warnings. The 27
  `fbo.completeness.attachment_combinations` cases and
  `shaders.fragdata.invalid_assign_to_1` that sweep 30 had NotSupported pass.
  Not one image set of one commit: a single-commit tree on `1cf946a9` is not
  run.
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

Only the Pi 4 had dEQP fails in a full tree (rows 3 to 27, all 0 on the board now). The i.MX8MP has one fail and one
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

Rows 3 to 27 are the 25 Pi 4 failures (issue #12), all passing on the board
since `a27d69c0` (19: the 18 vertex-stage cases and `texturecubelod`) and
`1cf946a9` (the 6 `mipmap.cube` cases; the 3 `size.cube.15x15` cases were
not among the 25, they passed at sweep 30 and failed on `a27d69c0` until the
same fix). Why they failed: the V3D 4.2
filters cube maps seamlessly and has no bit to turn that off. dEQP picks its
reference by context: a context that answers GL_MAJOR_VERSION without error is
judged against the seamless (ES 3) reference, any other against the
non-seamless ES 2.0 reference (`sampler.seamlessCubeMap = m_isES3Capable`,
es2fVertexTextureTests.cpp:1154). The same 25 cases pass under an
ES 3-reporting context. Mesa's v3d has no seamless control either. The fix
emulates non-seamless sampling in the V3D compiler for ES 2.0 builds, proved
on the host with the QPU/TMU interpreter (`src/shaderc/online/v3d/sim.rs`)
and on the board.

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
| `shaders.fragdata.invalid_assign_to_1` | 1 | 1 | 0 (passed at `d8583f20`) | 0 (passes at `a27d69c0`) | fixed: dEQP says "Test requires exactly one draw buffer"; an ES 2.0 context should report one. `gl_MaxDrawBuffers = 1` in `es2-gl-core` turned it into a pass on the AM62P (`32778b57`) and the Pi 4 (`a27d69c0`). | driver (GL core) |
| `fbo.completeness.attachment_combinations.<a>_<b>_<c>_tex`, a, b, c each one of none, rbo, tex (the 27 stencil-texture cases) | 27 | 0 (pass since a3856a06) | 0 (pass since f83a7361) | 0 (27 pass at `a27d69c0`) | closed: the Pi 4 sweep 30 had them NotSupported because the V3D did not claim the depth-texture extensions; since the V3D reads depth textures and claims GL_OES_depth_texture they pass (`fbo` 709 pass in the `a27d69c0` tally). | driver (V3D) |

Totals: AM62P 9 + 8 + 1 + 1 = 19. i.MX8MP 9 + 8 + 16 + 1 = 34 (the 25 of entry 28
plus negative_api 9; 30 if the four non-mustpass cases of entry 28 also count
there). Pi 4 8 + 27 + 16 + 1 + 9 + 1 = 62 at sweep 30; now 8 + 16 + 1 + 9 = 34 (the 27 fbo cases and fragdata pass).

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
`flush_finish.{flush,flush_wait}` warned in the AM62P 1y run and not in 1z2,
and on the Pi 4 at `a27d69c0` (2 CompatibilityWarnings; dEQP's own timing
check; it flips between runs).

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

### Host check of the compiler changes (`compiler-regression-host`, `c6aa6ba`, 2026-10-07)

The item's brief says no external CTS checkout, Zephyr workspace, `pco_shc`
or glslang was available (`CTS`, `ZEPHYR_WS`, `HAL_NXP_DIR`, `PCO_SHC`,
`GLSLANG` unset), so `tools/cts/host-deqp-run.sh` (the compile-and-link run
of the whole mustpass list on the host) could not have run there. The commit
message of `c6aa6ba` nevertheless reports such a run (17,165 of 17,165 cases
began on both back ends, no chunk crashed, 877 compile refusals, all on
negative cases dEQP passes; HOST-ONLY, compile and link outcomes, not
rendering). Nothing of that run is committed, and this item could not check
it, so it is a report, not a record: do not cite it as evidence until
`host-deqp-run.sh` is run with a CTS checkout. What is committed and
reproducible is the list below.

- New host tests: a seeded generator of dEQP-style GLSL ES 1.00 shaders
  (`tests/common/deqp_style.rs`); PowerVR: 300 seeds in both if-lowerings, 572
  of 600 compared and agree, 28 refused by name by the known
  `floor(-x)` / `floor(abs(x))` refusal; the 1,060 `fragment_ops.blend` cases
  as 1,481 blend keys run through the encoded blend tail: 1,481 agree;
  GC7000: 600 of 600 agree. 3,000- and 4,000-seed soaks agree after the fix.
- **Defect found and fixed (PowerVR vertex shaders, wrong code).** The
  attribute ranking ran over the module after the unroller, which drops loads
  inside a constant `if` or a zero-trip loop. In a shader with any loop, an
  attribute read only in such code took no registers and every later
  attribute was read four registers early. `select()` now ranks over the
  module it was given, before unrolling (test
  `an_attribute_read_only_where_the_unroller_drops_it_keeps_its_registers`).
  The code was in the first commit of the repository, not introduced since
  the last full trees, and no dEQP-GLES2 case is known to reach it. It
  changes AM62P vertex code, so the AM62P whole tree is re-run in the board
  session.
- Reported in the commit message, nothing committed (same caveat): `online_oracle.py` 0 false rejects of
  1,778, 0 false accepts of 1,005; `online_eval_oracle.py` 0 wrong of
  7,317; `es100_oracle.py` 2,787 of 2,793 (each its recorded figure).
- With `DEQP_PROGRAMS` naming dEQP's own `.test` programs, 1,723 variants
  agreed on both back ends (VK-GL-CTS `93bca0186`; the item names this as a
  recipe, not a run in this environment's default).
- Gate: `tools/check.sh --quick` 91 ok, 5 failed, each for a missing tool
  (no clang: three aarch64-unknown-none checks and the Mesa-reference control
  build; no libclang: the csbgen bindgen row).

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
| #12 | The Pi 4 differed from the ES 2.0 reference in the 25 cube cases (section B); fixed, 0 fail on the board (`a27d69c0`, `1cf946a9`). | Pi 4 | These were the 25 CTS failures. | `v3d-cube-design`, `v3d-cube-fix` |
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
- A whole tree of one commit for the Pi 4 (its row is composed of three
  records) and a current GLES2 tree for the AM62P (its last is `32778b57`, then
  image 1 on `e17fb6a7`; HANDOFF "Next board session").
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

## H. GL_OES_texture_npot: the dEQP-GLES2 cases it touches (`2673197d`, 2026-10-07)

**Status (2026-10-11): the earlier prediction was wrong, and the boards
agree with the CTS source.** This section first predicted that 18 cases
move from Pass to NotSupported. Reading the CTS source (2026-10-08) showed
that no case throws NotSupported on this extension; the cases run and must
Pass. The three boards' trees since then have none of the 18 among their
failures or NotSupported cases (H.4).

Every ES 2.0 build lists `GL_OES_texture_npot` in `GL_EXTENSIONS`, on all
three GPUs (`src/gl/state.rs`, `NPOT_EXTENSION`). Its rules replace ES 2.0's:
a non-power-of-two texture may have a mip chain and wrap with `GL_REPEAT` or
`GL_MIRRORED_REPEAT`, `glGenerateMipmap` of one succeeds, and a level above 0
that is not a power of two is no error (`src/gl/texture.rs`,
`NPOT_RESTRICTED` is false in every build). This is what glmark2's
terrain needed: 16 `GL error 0x502 in glGenerateMipmap` and a black 800x600
texture on every board (`tools/cts/RESULTS.md`, 2026-10-04).

### H.1 How each GPU holds the chain (host-tested)

| GPU | NPOT chain | Host tests |
|---|---|---|
| V3D (Pi 4) | whole chain; level i is max(w >> i, 1) x max(h >> i, 1), packed at its own pitch; the tiled copy follows Mesa's v3d_setup_slices (levels 0 and 1 at their size, the rest padded from the power-of-two level 1), which `tools/v3d/tex_check.c` already compares with Mesa for 13x7 x3, 37x61 x6 and 100x3 x2 | `v3d::backend::backend_tests::a_generated_npot_chain_is_sampled_whole` (800x600: ten levels, offsets, pitches, generated texels, binding, sampler, tiled levels) |
| GC7000 (i.MX8MP) | whole chain at one 800-texel pitch, each level on 64 bytes; the descriptor names each level's address | `vivante::backend::backend_tests::a_generated_npot_chain_is_named_level_by_level` |
| PowerVR (AM62P) | a 2D texture is laid out as the chain of its power-of-two extent (`cc78859`, 2026-10-08: `chain_layout_npot`, `pot_chain`); the TPU finds each level with its own arithmetic, which no board run has measured for an NPOT size. A cube keeps its base level and samples level 0 under a mip filter | `gl::draw::tests::a_generated_npot_chain_is_complete_with_repeat`, `a_powervr_npot_cube_keeps_its_base_level` |

The GC7000 caveat: etnaviv uses NPOT textures with any wrap mode only when
`chipMinorFeatures1_NON_POWER_OF_TWO` is set. This repository records the
part's `features` and `minor0` words (tools/cts/log/board-sessions/imx8mp-gc-driver-notes.md) and the
feature-database facts the kernel side uses (driver/rust/src/gpu/vivante/hw.rs),
not `minor_features[1]`. The part is HALTI5 (an ES 3.x-class core, whose ES
3.0 needs NPOT repeat and mipmaps), and the ES 3.0 build has always sampled
NPOT textures with REPEAT there. The i.MX8MP tree (H.4) passes the REPEAT
cases.

### H.2 What the CTS checks

Read in VK-GL-CTS `modules/gles2/functional/` (2026-10-08):
`es2fTextureCompletenessTests.cpp` (CC) and `es2fNegativeTextureApiTests.cpp`
(NT). A grep for `npot`, `PowerOfTwo` and the extension check over
`modules/gles2` found no other user.

| Where | What |
|---|---|
| CC:291-309 | `Incomplete2DSizeCase`: the relaxing extensions are `GL_OES_texture_npot` or `GL_NV_texture_npot_2D_mipmap` |
| CC:332-334 | each `glTexImage2D` expects `GL_INVALID_VALUE` only with **no** relaxing extension, level > 0 and an NPOT level; otherwise `GL_NO_ERROR` |
| CC:338-343 | only an NPOT **base** with the extension listed changes the expected colour to blue; otherwise black |
| CC:517-523, CC:920-926 | the wrap cases (2D, cube): extension listed, expected colour blue |
| CC:671, 701-703 | `IncompleteCubeSizeCase`: the error rule of CC:332-334; the expected colour stays black |
| NT:1341-1349 | `generatemipmap_npot_wdt_hgt`: extension listed, the case logs a line and returns; `es2fApiCase.cpp:54` set the result to Pass before the body |

Nothing throws `NotSupportedError` on this extension. Each completeness case
draws one quad with a solid blue texture over a 64x64 viewport, and every
pixel must equal the expected colour exactly.

### H.3 The 18 cases

| Case (`dEQP-GLES2.functional.` ...) | With the extension listed | Why yakogl passes it |
|---|---|---|
| `texture.completeness.2d.` `npot_t_repeat`, `npot_s_repeat`, `npot_all_repeat`, `npot_mirrored_repeat`, `repeat_width_npot`, `repeat_height_npot` (6) | runs; expects **blue** | an NPOT texture with REPEAT is complete (`incomplete_reason` has no NPOT arm) |
| `texture.completeness.cube.` `npot_t_repeat`, `npot_s_repeat`, `npot_all_repeat`, `npot_mirrored_repeat` (4) | runs; expects **blue**. The case binds its texture to `GL_TEXTURE_2D` and sets the images on the default cube texture (name 0) | the default cube object takes images and is sampled (`a_sampler_reads_the_default_texture_object_at_binding_0`) |
| `texture.completeness.2d.npot_size` | runs; a complete 255x255 chain, NEAREST_MIPMAP_NEAREST, REPEAT; no error at any level, expects **blue**. The only case that samples a mipmapped NPOT texture | the chain is accepted and complete (`a_npot_level_above_0_grows_the_chain_where_the_backend_holds_one`, at 15x15 only) |
| `texture.completeness.2d.` `npot_size_level_0`, `npot_size_level_1` (2) | run; no error; expect **black** | a level of the wrong size makes the texture incomplete |
| `texture.completeness.cube.` `npot_size_level_0`, `npot_size_level_1`, `npot_size_level_0_pos_x`, `npot_size_level_1_neg_x` (4) | run; no error; expect **black** | the same, per face |
| `negative_api.texture.generatemipmap_npot_wdt_hgt` | returns early; **Pass** | nothing to do |

The seven "expect black" cases also need the upload to succeed: a build that
returned `GL_INVALID_VALUE` for an NPOT level (the ES 2.0 rule without the
extension) would now fail them. So the extension and the rules must change
together, never in one place only. Other `*_npot` cases
(`texture.format.*_npot`, `texture.filtering.{2d,cube}.*_clamp_*_npot`,
`texture.wrap.clamp_clamp_*_npot_*`, `texture.size.2d.65x63_*`,
`texture.size.cube.15x15_*`, `fbo.render.*npot*`) use one level and
CLAMP_TO_EDGE and do not read the extension list.

### H.4 Board results

- **i.MX8MP**, integration image `334d4e65` (2026-10-09), the whole tree: entry
  10 (`texture.completeness.*`) 40/40 Pass, 0 NotSupported, so the 17
  completeness cases pass by name. Entry 22 (`negative_api.*`) is 237 Pass /
  9 NotSupported, the same as before the extension, so
  `generatemipmap_npot_wdt_hgt` passes by count. The REPEAT cases did not
  fail, so the GC7000 honours REPEAT on an NPOT size. The main tree on
  `32778b57` (2026-10-10) has the same numbers (`tools/cts/STATUS.md`).
- **AM62P**, main `32778b57` (2026-10-10): the 54 failures and 34
  NotSupported are named in `tools/cts/STATUS.md`; none of the 18 is among
  them.
- **Pi 4**, main `a27d69c0` (2026-10-11): the 9 failures and 34 NotSupported
  are named in `tools/cts/STATUS.md`; none of the 18 is among them.

### H.5 Still open

- Source comments still say dEQP expects the ES 2.0 error. With the
  extension listed the CTS expects `GL_NO_ERROR` (H.2). Follow-up for
  another item (this one changes no file under `src/`); lines at `9390731`:
  - `src/gl/texture.rs:2197`, in `glTexImage2D`'s level check: "dEQP
    texture.completeness.2d.npot_size expects it".
  - `src/gl/texture.rs:6974-6976`, the doc of
    `tex_image_level_on_npot_texture_follows_the_version`: "dEQP
    texture.completeness's npot_size expects it".
  - `src/gl/texture.rs:11294-11295`, the doc of `cube_chain_is_complete`:
    "GL_INVALID_VALUE, which dEQP expects too".
  - `src/gl/texture.rs:11582-11583`, the doc of
    `generate_mipmap_of_a_npot_texture_follows_the_reported_version`:
    "what dEQP's case is predicted to do then"; the prediction was wrong
    (H, status): the case returns early and is Pass.
  - `src/gl/draw.rs:13040-13044`, the doc of
    `a_npot_level_above_0_follows_the_version`: correct, but it calls a
    15x15 chain "npot_size"; the CTS's 255x255 chain is
    `tests/es2_npot.rs` `texture_completeness_2d_npot_size`.
  - The `NPOT_RESTRICTED` branches in the tests above
    (`src/gl/texture.rs:6989`, `src/gl/texture.rs:11593`,
    `src/gl/draw.rs:12867`, `src/gl/draw.rs:13055`) never take the
    restricted side in any build. They are revert checks, not stale text.
  - Correct, no change: `src/gl/texture.rs:946` (the constant),
    `src/gl/texture.rs:3464` and `src/gl/draw.rs:2542` (they say no build
    is restricted).
- Closed: `tests/es2_npot.rs` replays the 18 cases on the host, one test
  per case, in the CTS's order and at its sizes (`Incomplete2DSizeCase` at
  255x255, the cube size and wrap cases with the texture bound to
  `GL_TEXTURE_2D`), on the PowerVR shape and the layered (GC7000, V3D)
  shape. It asserts `GL_NO_ERROR` after every call and the completeness and
  sampling of the bind the back end receives, not pixels (the stand-in back
  end draws nothing). It needs `shaderc-online`.
- The PowerVR's level arithmetic for an NPOT 2D chain is exercised only by
  `2d.npot_size` (a 63x63 level 2 at a 64-texel pitch) and glmark2's terrain.

### H.6 Each case and its host test

One test per case in `tests/es2_npot.rs`. The test name is the case name
after `dEQP-GLES2.functional.`, with the dots as underscores. Every test
runs on a fresh context on the PowerVR shape and on the layered (GC7000,
V3D) shape. After every GL call it asserts `GL_NO_ERROR`. In a `report-es2`
build it also asserts that `GL_OES_texture_npot` is in `GL_EXTENSIONS`.
"Complete" means the bind the back end receives has `incomplete == 0` (the
CTS's blue). "Incomplete" means `incomplete == 1` (the CTS's black).

| Case (`dEQP-GLES2.functional.` ...) | Test | What it asserts |
|---|---|---|
| `texture.completeness.2d.npot_t_repeat` | `texture_completeness_2d_npot_t_repeat` | 127x127, S clamp, T repeat, NEAREST: complete, the wrap as set |
| `texture.completeness.2d.npot_s_repeat` | `texture_completeness_2d_npot_s_repeat` | the same with S repeat, T clamp |
| `texture.completeness.2d.npot_all_repeat` | `texture_completeness_2d_npot_all_repeat` | REPEAT on both axes |
| `texture.completeness.2d.npot_mirrored_repeat` | `texture_completeness_2d_npot_mirrored_repeat` | MIRRORED_REPEAT on both axes |
| `texture.completeness.2d.repeat_width_npot` | `texture_completeness_2d_repeat_width_npot` | 127x128, REPEAT: complete |
| `texture.completeness.2d.repeat_height_npot` | `texture_completeness_2d_repeat_height_npot` | 128x127, REPEAT: complete |
| `texture.completeness.cube.npot_t_repeat` | `texture_completeness_cube_npot_t_repeat` | new texture bound to 2D, six 127x127 faces on the default cube texture (name 0): complete, a cube bind |
| `texture.completeness.cube.npot_s_repeat` | `texture_completeness_cube_npot_s_repeat` | the same |
| `texture.completeness.cube.npot_all_repeat` | `texture_completeness_cube_npot_all_repeat` | the same |
| `texture.completeness.cube.npot_mirrored_repeat` | `texture_completeness_cube_npot_mirrored_repeat` | the same |
| `texture.completeness.2d.npot_size` | `texture_completeness_2d_npot_size` | 255x255, levels 255 down to 1: complete, a nearest-level mip filter, REPEAT; 8 levels on the layered shape |
| `texture.completeness.2d.npot_size_level_0` | `texture_completeness_2d_npot_size_level_0` | 256 chain with a 255x255 level 0: incomplete |
| `texture.completeness.2d.npot_size_level_1` | `texture_completeness_2d_npot_size_level_1` | 256 chain with a 127x127 level 1: incomplete |
| `texture.completeness.cube.npot_size_level_0` | `texture_completeness_cube_npot_size_level_0` | 64 chain, level 0 63x63 on all six faces: incomplete |
| `texture.completeness.cube.npot_size_level_1` | `texture_completeness_cube_npot_size_level_1` | level 1 31x31 on all six faces: incomplete |
| `texture.completeness.cube.npot_size_level_0_pos_x` | `texture_completeness_cube_npot_size_level_0_pos_x` | only +X level 0 63x63: incomplete |
| `texture.completeness.cube.npot_size_level_1_neg_x` | `texture_completeness_cube_npot_size_level_1_neg_x` | only -X level 1 31x31: incomplete |
| `negative_api.texture.generatemipmap_npot_wdt_hgt` | `negative_api_texture_generatemipmap_npot_wdt_hgt` | the extension is listed (the CTS returns early: Pass); the body it skips (256x257, 257x256, 257x257 RGB, then `glGenerateMipmap`) gives `GL_NO_ERROR` |

No test is `#[ignore]`d. The file is
`#![cfg(all(feature = "host-test", feature = "shaderc-online"))]`, because
the quad's programs are not in the offline shader table. Runs on `9390731`
(2026-10-12), each `18 passed; 0 failed; 0 ignored`:

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_npot
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es2_npot
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test es2_npot
cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es2_npot
```
