<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Handoff

The state of the work and what comes next. A result does not edit this
file: it adds a note under `docs/handoff/` (one file per result,
`docs/handoff/README.md`), and ccfarm's `handoff-fold` stage folds the notes
in after the merge; a person folds the same way. Last update: 2026-10-15.

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

main is `cb93f638` (2026-10-15). The numbers below are read from
`tools/cts/RESULTS.md` and `tools/cts/STATUS.md`; the commit next to each is
the image's. Where a number has no RESULTS.md entry, the line says so. What
this section said before 2026-10-11, with every closed row and the
measurements behind the decisions, is
`tools/cts/log/handoff-2026-10-07-to-10-11.md`.

Per-case detail: `docs/es2-cts-failures.md` (ES 2.0) and
`docs/es3-cts-failures.md` (ES 3.0). Every board number with its record:
`tools/cts/RESULTS.md`. The short status in Japanese: `tools/cts/STATUS.md`.
All three boards boot standalone from eMMC (the two Verdins, U-Boot) or SD
(Pi 4): `docs/standalone-boot.md`.

### ES 2.0, per board

| Board | dEQP-GLES2 mustpass (17,165) | gl_test | glmark2 | Slint, cluster |
|---|---|---|---|---|
| AM62P | whole tree on `32778b57` (2026-10-10): 17,070 pass / 54 fail / 34 NotSupported / 5 QualityWarning / 2 CompatibilityWarning. Of the 54: the 9 `shaders.invariance` cases pass on the board since `e17fb6a7` (image 1, 1,379 / 0); the 45 `texture.vertex.2d.*` cases (image 9) are fixed on the host (`a6fc6f14`) and wait for the board. The ms4 image: 14 / 2 on `32778b57`; 16 / 16 on `452d6c31` and `a27d69c0` after the PDS fix (`a4f64950`; no RESULTS.md entry: the commit message of `a80b297f5` and the HANDOFF text of `bd8df7a49`). | 49 / 49, 0 resets, on `c4ccc966` (197 `vtxgrid` in the set; no RESULTS.md entry); 48 / 48 on `32778b57` (RESULTS.md). Cases 198 to 201 pass on `a27d69c0`, 0 resets (no RESULTS.md entry: the HANDOFF text of `bd8df7a49`); 198 hung the fragment DM on `452d6c31` before `a80b297f5`. 50 / 50 on `cd738b1a` (2026-10-14, common set, build id `d5cb878`; `docs/board-sessions/am62p-common-int.md`). | 465, 33 scenes ran, 0 failed, on `cd738b1a` (2026-10-14, common set), but terrain (1 fps) and shadow (3 fps) run out of GPU memory and refract raises 1694 `framebuffer 1 incomplete` errors ("Known defects"); 465, 33 ran, 0 failed, 0 resets, on `f2a13877` (2026-10-11): pvr-depth-resolve confirmed. | SO pass (crc16 c592), SV empty (0 console bytes, unresolved; same image as SO), CL pass (crc16 110c, 75.3 fps, 46.3 Mpx/s at 1024x600) on `cd738b1a` (2026-10-14, common set); cluster 91.8 fps (56.4 Mpx/s, 16.28 ms/Mpx) on `7dada1ff` (RESULTS.md, 2026-10-05). |
| i.MX8MP | whole tree on `835cf9cc` (2026-10-14, 29 entries, `records/imx8mp/deqp-835cf9cc`): 17,272 pass / 46 fail / 34 NotSupported / 19 other of the 17,371 begun, 0 no-result. All 46 are `GL_BGRA_EXT` renderbuffer cases of entry 21 (`fbo`); every other entry equals the `32778b57` run (17,318 pass / 0 fail, 2026-10-10). | 48 / 49 run passed, 1 skipped (case 16, #8), case 44 (#6) fails, on `a63fcc5b` (2026-10-14, common set; `tools/cts/RESULTS.md`); the same reading on `b568940` (`docs/board-sessions/imx8mp-common-int.md`), where cases 2, 10 and 171 passed. The same reading on `e0eb883d` (2026-10-14, two boots, identical; `docs/board-sessions/imx8mp-gltest-final.md`); 0 GPU resets. Case 16 passes when run alone (`SEL=16`). Cases 10 (#9) and 2 (#25) fail now and then. | 43, 33 ran, 0 failed, 0 unsupported, on `a63fcc5b` (2026-10-14, common set; also on `b568940`, record `records/imx8mp/common-b5689400`); 42 on `8abeaabb` (`tools/cts/log/benchmarks-baseline/imx8mp.md`); no console, so no per-scene score. | SO, SV and CL probes pass on `a63fcc5b` (on `b568940`: SO and SV crc16 c592, CL crc16 b107); no fps (no console). |
| Pi 4 | 17,126 pass / 0 fail / 34 NotSupported / 5 warnings, composed of three records: the tree on `a27d69c0` (17,083 / 9 / 34, 34 lost in capture), the six cube rows on `1cf946a9` (934 / 934) and the F1 rerun (2026-10-10 and 10-11). One tree from one commit has not run. | 12 / 12, 0 failed, 0 MMU faults, 0 recoveries, on `a63fcc5b` (2026-10-14; `docs/board-sessions/pi4-gltest-final.md`); 12 / 12 on `1cf946a9` (2026-10-11). The ES 3.0 cases on `cd738b1a` (2026-10-14; `docs/board-sessions/pi4-gles3-baseline.md`): 202 `blitconv` 12 / 12, 203 `tfcap` 9 / 9 and 205 `mappbo` 15 / 15 pass; 196 `occl` fails (all five queries available, all five results 0) and 204 `srgb` fails (the render half is refused by name, no sample-half checks ran); 0 MMU faults, 0 recoveries. | 183, 33 ran, 0 failed, on `b7170d35` (2026-10-04, a commit of the history before `afb16335`). | Slint probes pass on `1fa941e3` (2026-10-04, same history); the cluster's last row in RESULTS.md is "Batch 106" (`a8b11720`). |

The expected reading of every image or row of the final dEQP-GLES2 trees is
in `tools/cts/log/board-sessions/es2-final-expected.md`: one table per board
(AM62P 17 images and the ms4 image, i.MX8MP 29 entries, Pi 4 29 rows), with
the case count of each filter, the last recorded reading with its commit and
record, the expected final reading (Fail 0; the NotSupported and
QualityWarning cases only in the buckets of `docs/es2-cts-failures.md`
section C) and the fix commit where a row changes. Each board's rows add up
to 17,165. The invariance fix is in: `tests/es2_invariance.rs` reads 12
passed, 0 failed, 0 ignored in all six feature sets (PowerVR, GC7000, V3D,
each with and without `report-es2`). Whole-tree records of 2026-10-12 exist
for every board and read 0 Fail: AM62P `63efed8e`
(`docs/board-sessions/am62p-es2-final-r2.md`), i.MX8MP `c345ab74`
(`docs/board-sessions/imx8mp-es2-final-r2.md`), Pi 4 `d56e79e5`
(`docs/board-sessions/pi4-es2-final-r2.md`); `docs/es2-cts-failures.md` and
`tools/cts/STATUS.md` do not carry them yet (`es2-docs-r1`).

The NotSupported 34 are the same on the three boards: `clip_control` 8,
`negative_api.texture.compressedtexsubimage2d_*` 9,
`multisampled_render_to_texture.readpixels` 1 and `multisample.*` 16 (the
1x config; the AM62P measures them on its ms4 image).

### Waiting for a board

Each item waits for hardware. The commands and expected values are in
"Next board session".

- AM62P image 9 of the GLES2 tree: the 45 `texture.vertex.2d.*` cases
  (`a6fc6f14`).
- AM62P: one whole GLES2 tree from one main commit, with the ms4 image.
- Pi 4: one whole GLES2 tree from one main commit (it replaces the composed
  row).
- gl_test 196 `occl`, 202 `blitconv`, 203 `tfcap`, 204 `srgb` and 205
  `mappbo` (the `work/land9` cases, on main since `c4dce9125`) on the
  i.MX8MP (the AM62P ran them at `cd738b1a` and again at `9b525867`, 5 of 5;
  the Pi 4 ran them at `cd738b1a`: 202, 203 and 205 pass, 196 and 204 wait
  for their defects, "Known defects"); 199 to 201 on the i.MX8MP and the
  Pi 4 where the GPU runs them.
- i.MX8MP dEQP-GLES3: the 133 table rows `e00` to `e132`
  (`docs/es3-board-recipe-imx8mp.md`), from an image rebuilt at the commit
  the board runs.
- AM62P final GLES2 tree (`am62p-es2-final-a`):
  `texture.completeness.2d.npot_size` must still read Pass and glmark2
  terrain must still render (`docs/investigations/pvr-npot-levels.md`).
- i.MX8MP case 44's layout probe (#6, step 6 of the i.MX8MP section).
- i.MX8MP #8 (4x MSAA) and #9 / #25 (cases 10 and 2): the next instruments
  are in "Known defects".
- AM62P dEQP-GLES3: the "hang" of images 20 and 21, and image 7's
  `ResourceError` (Known defects).
- The Pi 4 glmark2 refract and terrain (#4, #5) on the 96 MiB carve-out.
- AM62P: SV once more (empty at `cd738b1a`), and glmark2 after the render
  pool fix (terrain and shadow out of memory, Known defects).
- The i.MX8MP panel by eye (#23).

### ES 3.0 (goal 4)

The AM62P dEQP-GLES3 tree on `ad2b5ed` (main `e51de0af` plus docs; same
`src/`, `driver/` and `capi/` as main `a63fcc5b`), 2026-10-14: of 42,810,
**36,552 pass / 2,880 fail / 2,195 NotSupported / 185 QualityWarning / 998
without a result** (`f2a13877`, 2026-10-11: 32,669 / 5,549 / 2,318 / 185 /
2,089). It is the first board run of `c4dce9125`: `fragment_ops` 1,109 -> 0
fail, `transform_feedback` 640 -> 0, `fbo` 872 -> 367. The 998: image 7's
`builtin_functions.precision` tail 987 (the `ResourceError`) and 11 slow
`texture.filtering.3d` cases not started. Per case:
`tools/cts/log/board-sessions/am62p-gles3-percase-2026-10-13.md`; every case
that is not Pass: `am62p-gles3-percase-2026-10-13.notpass.tsv` beside it.
Rows re-run on `cd738b1a` (2026-10-14; images 2, 3, 5, 25, all `done`, 0
resets; section "dEQP-GLES3 rows" of `docs/board-sessions/am62p-es3-gltest.md`)
replace the `f2a13877` numbers of their groups: transform_feedback 882 / 882
and ubo 1,099 / 1,099 pass (image 25 lost 51 result lines on `f2a13877`, none
now); image 5 completes (1,662 cases; the capture no longer aborts it) with
`occlusion_query` 62 / 62 pass; `fbo.blit.conversion` 325 pass / 0 fail / 112
NotSupported; `fragment_ops.scissor` 26 / 26 pass. This section's whole-tree
row, RESULTS.md and STATUS.md stay as they are until a whole tree runs. The
gl_test cases 196 `occl`, 200 `intrt`, 201 `usamp`, 202 `blitconv`, 203
`tfcap`, 204 `srgb` and 205 `mappbo` are board-proven on the AM62P at
`cd738b1a` (7 of 7 pass, 0 resets; 196 q0 to q4 `available 1` with 0, 1, 0,
1, 1; 200 16 / 16, 201 8 / 8, 202 12 / 12, 203 9 / 9, 204 8 / 8, 205 15 /
15); the ES 2.0 default set still passes there, 0 of 50 failed. On the same
commit the default list passes 50 of 50 with cases 198 `a2cconst`, 197
`vtxgrid` and 171 `viewwin` 3 of 3 (0 resets; uboot-mmc, console on
main_uart0; `docs/board-sessions/am62p-gltest-final.md`), and 196 and 202 to
205 pass again at the integration tip `9b525867` (5 of 5, 0 resets, same
values; image sha256 `4acb9d77...`; run record
`records/am62p/20261014-0910-es3gl-4acb9d77`), so the AM62P part of the
stage-1 gl_test gap is closed. Fail by group
and the 2,089 by group: `tools/cts/RESULTS.md`, "AM62P: the
whole dEQP-GLES3 tree on `f2a13877`". The Pi 4 GLES3 tree
(`tools/cts/pi4-gles3-tree.txt`, 91 images; recipe
`docs/es3-board-recipe-pi4.md`) ran its first baseline on `cd738b1a`
(2026-10-14): 7 of the 91 images (A1, B1, C1, D1, FB1, FB2, FB3) ran, 3,807
cases, 2,798 pass / 744 fail / 265 NotSupported, every case with a result, 0
resets; B1 (`buffer`, 695) and C1 (`clipping`, 602) pass in full
(`docs/board-sessions/pi4-gles3-baseline.md`, per-case failures in
`pi4-gles3-baseline-fails.tsv`). The i.MX8MP's table
(`tools/cts/imx8mp-gles3-tree.txt`) covers every `gles3-mustpass.txt` case
once (`check-imx8mp-gles3-tree.py`); its GLES3 stage 1 and table image were
built at `ba8b242a` (`docs/board-sessions/imx8mp-deqp-stage1-gles3.md`), and
only `--gles3` turns `REPORT_ES2` off, so ES 2.0 stays the default; it has
never run on the board. Every
other ES 3.0 number is HOST-ONLY: the Linux OSAL runs no command stream, so
only API errors and compile and link outcomes mean anything there
(`tools/cts/README.md` section 2b). Under `report-es2` every ES 2.0 answer
is kept and pinned by tests. The design for `gl_InstanceID` in
instance-by-instance draws (the six `instanced.*.mixed.{2,4,20}_instances`
cases) is chosen, with no source change yet:
`tools/cts/log/investigations/instanceid-per-draw.md` ("Open work" item 4,
es3-vertex).

| Feature | State | Design |
|---|---|---|
| Transform feedback | built on the host for the three GPUs: the GL core runs the vertex shader on the CPU for every captured vertex (`c4dce9125`; API `16ac864`). Host dEQP-GLES3 `transform_feedback` 882 / 882, `lifetime` 56 / 56. The capture also evaluates a vertex shader's texture look-ups (`texture`, `textureLod`, `textureOffset`, `textureProj`, `texelFetch`, `texelFetchOffset`, `textureSize`) on 2D, cube, 3D and 2D-array samplers (float, `isampler*`, `usampler*`) from the GL texture storage (`TexelSource` in `src/shaderc/online/eval.rs`, `GlTexels` in `src/gl/tf_capture.rs`); host only, `tests/es3_tf_sampler.rs`. | `docs/es3-transform-feedback.md` |
| Uniform blocks | host: `tests/es3_ubo.rs` ports dEQP's `functional.ubo` (2,032 cases); the GC7000 refuses four at link (`KNOWN_LIMITS`) (`1b0e431`, `b0c0fa6`) | none |
| Sized internal formats | the 35 colour formats of ES 3.0 table 3.2, converted at upload; the PowerVR samples every class but `RGB10A2`; the GC7000 and V3D refuse the integer ones by name (`888af5d`, `e5c4a18`; PowerVR `25267d17`, merged in `a27d69c0`) | `docs/es3-sized-formats.md`, `docs/es3-pvr-formats.md` |
| Occlusion queries | counted on the three back ends, host only (`2241cd84` GL core, `d9573ab9` PowerVR, `dcac96cb` GC7000, `7526b084` V3D; merged in `452d6c31`) | `docs/es3-occlusion-query.md` |
| Integer colour render targets | GL half built; the PowerVR writes all nineteen as a draw's one colour attachment (`fc658e64`, merged in `a27d69c0`); the GC7000 and V3D refuse by name | `docs/es3-pvr-formats.md` section 10 |
| `glBlitFramebuffer` | colour on the CPU, shared by the three back ends (`c4dce9125`); depth on the CPU into a depth texture's texels, from a depth texture or a depth renderbuffer (blit-ds, host only, `tests/es3_blit_ds.rs`); depth into a renderbuffer or the window, depth from the window, and all stencil refused by name | `src/gl/blit.rs` |
| sRGB | PowerVR renders and samples; the GC7000 and V3D sample and refuse a draw into sRGB by name (`c4dce9125`) | `docs/es3-srgb.md` |
| Buffer mapping and pixel buffers | host-proven (`c4dce9125`) | none |
| NPOT levels of arrays and storage textures | valid at every level (`c40414b`) | none |
| Framebuffer completeness | host dEQP `fbo.completeness`: 17 cases Fail to Pass on the PowerVR and the GC7000UL, 0 regressions (depth larger than colour, colour on attachments 1.. alone, separate depth and stencil with no colour); `src/gl/framebuffer.rs` `check_status` under `!REPORT_ES2`; HOST-ONLY (reads no pixels); causes left in "Open work" item 4 | none (`tests/es3_fbo_fragops.rs`) |
| Stage-interface link rules | interpolation and block mismatches refused (`b88fea5`, `03ff7a6`) | none |
| GLSL ES 3.00 in the compiler | measured over dEQP's 5,596 programs (`4ab482e`, `79fe171`, `ed0c81d`); the GC7000UL now builds `large_constant_arrays.indexing.{float,vec4}_128_vertex` (gc-const-array, 2026-10-14), so its expected-build count equals the PowerVR's and the V3D's | `docs/es3-shaderc-coverage.md` |
| Vertex array conversion | a buffer array of a type the GPU cannot fetch as it lies is converted to float32 rows like a client array (`src/gl/draw.rs`, `attrib_in_place_layout` decides): packed `GL_INT_2_10_10_10_REV` / `GL_UNSIGNED_INT_2_10_10_10_REV`, `GL_HALF_FLOAT`, and `GL_INT` / `GL_UNSIGNED_INT` set with `glVertexAttribPointer` (ES 3.0 2.9). The conversion reads the buffer's CPU view, or the OSAL `gpu_read` if there is none; element size and stride come from the real type (`attrib_elem_bytes`). An integer array narrower than its `ivec4` / `uvec4` input fills w with the integer 1 (`attrib_row`, ES 3.0 2.8; only `glVertexAttribIPointer` arrays feed integer inputs). HOST-ONLY: `tests/es3_vertex_arrays.rs`, 10 tests, needs `host-test,gpu-powervr,shaderc-online` and no `report-es2`; `tools/check.sh` runs it next to `es3_occlusion_gl` | none |
| PowerVR 2D-array and shadow sampling | designed, not built | `docs/es3-pvr-2darray.md` |
| ES 2.0 report guard | `tests/es2_report_guard.rs` pins what an ES 2.0 client sees; two pins are `#[ignore = "BUG: ..."]` (framebuffer and texture rows; the two state pins run since the state/program tranche, `tests/es2_error_rules_state.rs`) (`6c9e9ee`) | none |
| ES 3.0 limits test | reverted for review (`3f78982`); es3-caps redoes it | none |

## Open work, in order

1. **Compile shaders at build time with our own compiler.** Shaders known
   at build time are compiled then; the run-time compiler runs only on a
   miss. An image without the run-time compiler refuses a miss by name.
   Mesa's `pco_shc` / `etna_shc` then compile nothing. Steps 1 to 5 are on
   main (`68563f54`, `dea95deb`, `84b5761e`, `17605c36` with `17e7d34c`,
   `05f8f749`; board proof on the AM62P at `05f8f749`); the step-by-step
   account is the record file named under "State of main".
   - Step 6 was measured and decided: the precompiled tier stays off by
     default. On the AM62P gl_test image (2026-10-07,
     `tools/precompile/measure-image.sh`) the compiler costs 884,201 bytes
     of flash and 5,244,035 bytes of RAM and serves any shader; one app's
     table (`gl_test_es100`, 176 sources) costs 2,173,648 bytes of flash
     and grows with the app's shader set. The tier is what an image pays
     when it wants no compiler.
   - Open: the driver's five end-of-tile programs, the boot self-test's
     vertex shader and the VDM-sync program are still Mesa's text, so pco
     compiles them for a product image. The branch that made them come
     from our encoder (`work/pco-driver-programs`, `f5e4aa57`) is not in
     this repository; the work is open.
   - Open: four gl_test sites link a named source beside a buffer they fill
     with GLSL at run time (149 `case_vsunits`, 150 `shprobe`, 166
     `fshprobe`'s `shp_arm` and `fshp_arm_units`); no table can name those
     programs, so those cases need the compiler.
   - Open: the GC7000 has no table row for the 69 gl_test sources in
     `tools/shaders/gltest/`, whose `dir` line has no `gc7000` role (issue
     #22, in "Known defects"); that role needs
     `gen_shader_table.py --append` with etna_shc, then an i.MX8MP run with
     no compiler. The V3D table cannot be regenerated while the pair reader
     has unresolved sites (`tools/v3d/glsl2v3d.py`).
   - A limit, not a gap: sampler-state (lod-key) variants are not
     precompiled. A missing variant is never a refusal: the draw gets the
     program's own shaders, as every draw did before the tier existed.
   - Open: an image that turns the tier on must name its apps
     (`YAKOGL_PRECOMPILED_APPS`); an empty value stops the build.
   - Open: three kinds of shader no table can serve: gl_triangle and
     gc_gl_triangle write their GLSL as C literals in their own main.c, and
     glmark2 and dEQP bring their shaders from outside. Those images need the
     compiler.
   - Remaining steps: 7 rename to "precompiled shaders" and "shader
     compiler" (features `precompiled-shaders` and `shader-compiler` for
     `shaderc-online` and `shaderc-offline`; Kconfig symbols, the C ABI,
     log prefixes); 8 the source moves from `src/shaderc/online` to
     `src/shaderc/compiler` and `docs/online-shaderc` to
     `docs/shader-compiler`; 9 optionally, V3D rows. The words online,
     offline and remote go away.
2. **Design clean-up.** Decided 2026-10-06. Clean-up 1 (`260711b9`) and 2
   (`72d16864`), the GC7000's debug knobs, are on main.
   - Clean-up 3 (the PowerVR's switches leave the product: 5 hard-coded at
     their measured defaults, 2 switches and 8 options behind
     `test-knobs`, 5 options retired) is open and has no branch: the
     branch once named for it (`work/cleanup-1-vivante-knobs`) is on no
     remote and in no local ref (checked 2026-10-11), and `src/pvr` has no
     `test-knobs` gate. Its two dEQP fixes are on main already (`4e9397c`).
   - One more knob to remove: `gc_debug_msaa` (the GC7000's 4x MSAA switch:
     bit 0 the 4x path, bits 1 and 2 etnaviv's extras, bit 3 the viewport
     and clip scaling). The bit 3 A/B failed in both arms (#8), so bit 3
     is not the design; the knob goes when #8 is fixed.
   - Order from here (4 to 15): GL core / V3D knobs; the driver Kconfig
     instruments; samples cut; gl_test Rust harness; (after the
     shader-compiler rename, item 1) compiler knobs; samples stage 2; bare
     metal; names; OSALs to Rust; docs.
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
   - Docs: done. The board-session records, investigations and benchmark
     baselines are under `tools/cts/log/`; a design statement in one of
     them counts only once a `docs/` file or this file says it. The plans
     and bring-up diaries of `driver/docs` and `docs/direct-boot.md` (now
     `tools/cts/log/x2-direct-boot.md`) moved there too. The current design documents are `driver/docs/pvr-render.md`,
     `driver/docs/rpi4-v3d.md`, the "Hardware findings that still hold"
     section of `driver/docs/vivante-kernel-rust.md` and the "What the image
     does on a go entry" section of `docs/standalone-boot.md`.
   - Remaining C moves to Rust: the OSALs (3,740 lines), `gl_test.c` (20,291
     lines: a Rust runner calls the C cases through `extern "C"`, ported in
     batches), `driver/src` (about 16k lines).
3. **Known defects.** Each has a GitHub issue unless marked "no issue yet".
   Unless a line says "board", the fix is proved on the host only.
   - **GLES3 hang** (no issue yet): two dEQP-GLES3 images stop, and the
     AM62P comes back only with the RTI reset:
     `texture.filtering.3d.formats.r11f_g11f_b10f_linear_mipmap_linear`
     (image 20) and `texture.filtering.3d.sizes.3x7x5_linear_mipmap_linear`
     (image 21), each the 14th case of its image (`f2a13877`, 2026-10-11).
     Both are 3D textures filtered `linear_mipmap_linear`. The cause is most
     likely **not a GPU hang** (hypothesis H1 of
     `tools/cts/log/investigations/pvr-3d-mipmap-hang.md`). On the host, the
     PowerVR back end gets bit-identical texture and sampler words for the
     14th case (`linear_mipmap_linear`) and the 13th (`linear`, which
     finished), in both images, from the same program. A 3D texture is a
     one-level slice grid there, so the min filter never reaches the TPU. The
     board picture of a 3D mipmap case is wrong: level 0 only, and REPEAT
     lost on all three axes. dEQP's check of a wrong 3D
     `LINEAR_MIPMAP_LINEAR` picture is a six-deep search per pixel and took
     337 s for one case on `c4ccc966` (`tools/cts/README.md` 2e). "The 14th
     case" in both images follows from dEQP's sorted case order (each
     image's third prefix, second filter). `no_reset` at 602/612 s cannot
     tell this from a hang, because the RTI resets up to 660 s after arming.
     Measured on the host with the PowerVR's picture (host Mesa forced to
     level 0 and clamped coordinates): every 3D `linear_mipmap_linear` case
     takes 8.8 to 18.8 s of dEQP checking, every other case under 0.07 s,
     and plain Mesa passes all in milliseconds. The board's 337 s case takes
     18.8 s on the host (factor about 18), so both images reach case 14 at
     about 560 s and 600 s. Not yet confirmed on the board: the
     investigation's section 7 has the check. Until it is fixed the two
     images' tails (83 cases) have no result. Image 11 needs 4 resets
     (`done_with_resets`) to complete.
     Update 2026-10-14 (`ad2b5ed`, per-case record named under "State of
     main"): for image 20 it is the 600 s image lifetime, not a GPU hang. The
     two non-mustpass `bgra*_linear_mipmap_linear` cases take about 4.5 min
     each, so `r11f_g11f_b10f_linear_mipmap_linear` starts at 549 s; alone it
     completes in 158 s (Fail, no reset). Next: narrow
     `tools/cts/am62p-gles3-tree.txt`'s `3d.formats` and `3d.sizes` rows to
     the mustpass names, one image per `*_linear_mipmap_linear` case (not
     done). Image 21's `3x7x5_linear_mipmap_linear` has not run alone yet.
     Image 11's 4 resets are `shaders.metamorphic.bubblesort_flag.variant_1`
     (3) and `variant_2` (1).
   - **Depth-PBO uploads** (`tools/cts/RESULTS.md` row "AM62P GLES3 (depth
     uploads)", 10 cases): pass on the board at `ad2b5ed`, with a call trace
     that raises no GL error. The row can close.
   - **PowerVR 3D textures sample level 0 only and lose REPEAT** (no issue
     yet; the defect behind the GLES3 hang): `src/gl/texture.rs`
     `texture_alloc_storage_3d`, `src/shaderc/online/powervr/select.rs`
     `vol_uv`. Mesa's path is the TPU's own 3D type (`TEXTYPE_3D`,
     3D-twiddled layout, IMAGE_WORD1 `depth`, the full chain, `addrmode_w`).
     The investigation's section 3 compares the fields one by one. Work:
     "Open work" item 4, pvr-3d.
   - **AM62P GLES3 image 7** (no issue yet): a 24,384-vertex `glDrawArrays`
     in `shaders.builtin_functions.precision.clamp.highp_fragment.vec3`
     does not fit the 1 MiB general arena, and dEQP stops at the
     `ResourceError` (1,397 cases never run). A non-indexed draw needs
     cutting into pieces as `draw_indexed_pieces` cuts an indexed one.
   - **AM62P fbo / blit / invalidate failures** (no issue yet; findings 6 and
     7 of `docs/board-sessions/am62p-es3-gltest.md`, `cd738b1a`):
     `fbo.invalidate.{sub,whole}.unbind_blit_*` 14 Fail (the expected
     `unbind_blit_color*` cases did not all pass);
     `fbo.blit.default_framebuffer` `r8_*_blit_from_default` 3 Fail;
     `fbo.color.repeated_clear.blit.tex2d.rgb16f` Fail (not in the expected
     integer list); `fbo.blit.conversion` 112 NotSupported (every one has an
     `r32f`, `rg32f`, `rgba32f` or `r11f_g11f_b10f` end);
     `lifetime.attach.deleted_output.buffer_transform_feedback` and
     `deleted_input.buffer_vertex_array` still Fail. Many fbo failures are
     `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`. `blit-ds` stays: 16 Fail on
     `fbo.blit.depth_stencil`.
   - **ES 3.0 framebuffer completeness, causes left** (no issue yet; host):
     the 12 colour-0 `attachment_combinations` cases, R32F, RGBA32F, the
     integer texture attachments, the unsized depth textures, the PowerVR 3D
     grid slice and the draw into colour-1-only. Each, with its file, is in
     "Open work" item 4, `fbo-completeness`.
   - **pvr-msaa-split** (no issue yet): a 4x frame the PowerVR splits into
     two kicks (a full arena, the 1024-draw cap, or a glFinish /
     glReadPixels before more drawing) loses its samples: the next kick's
     colour-load quad writes the resolved colour to all four samples.
     `a_split_multisampled_frame_reloads_each_sample_not_the_resolved_colour`
     (`src/pvr/backend_tests.rs`) is `#[ignore = "BUG: ..."]` and asserts
     the right answer. The fix keeps each sample across kicks and reloads
     it at the full sample rate; the PBE's unresolved layout and the
     per-sample load are unmeasured on the board.
   - AM62P: an indexed draw whose staged data cannot be cut (a fan or a
     line loop, a second index list, or one primitive whose vertex window
     is over half the arena) is still refused by name when it does not fit
     an empty frame. The GC7000 gives such a draw memory of its own
     (`src/vivante/state.rs`, `GcFrame::extra`); the PowerVR keeps its fixed render pool.
   - i.MX8MP gl_test case 44 `es3hdr`, an RGBA16F render target (#6):
     still fails at `b568940` (2026-10-14, common set), on `f2a13877` at check 3 (texel (5,0)'s colour stored at
     (7,0) of an 8-wide linear target), and in the default list at
     `e0eb883d` (`failing 1: first case 44`; the same on both boots). `466e1b00`'s two fixes did not move
     it. The ranked hypotheses: `src/vivante/VIVANTE_NOTES.md`, "RGBA16F
     targets". The layout probe (case 44's check 3 on the GC7000,
     `hdr_layout_probe`, decode pinned by `tests/gltest_hdr_probe.c`) is
     the next board run (i.MX8MP step 6).
   - i.MX8MP gl_test case 16 no longer fails in the common set: the set
     skips it by name (#8 tracks the skip).
   - i.MX8MP gl_test case 10 fails about 1 run in 3 (#9), and case 2 now
     and then (#25), the same nondeterminism class. Cause not found
     (`8409419`): our sync sequence has a counterpart for every etnaviv
     step, and repeat runs on integration images (`b20e4d11`, `0175bac1`)
     did not separate the three surface hypotheses
     (`tools/cts/log/investigations/gc-case10-flake.md`). Rate without an
     instrument (integration image `e3efeaad`, 2026-10-11, 32 boots): case
     10 failed 4 of 32, case 2 0 of 32
     (`tools/cts/log/board-sessions/imx8mp-gltest-r2.md`). gl_test now
     calls the KICK_DIFF instrument (a second read-back 2 ms after the
     fence) for cases 2 and 10 when the selector asks for its page; the
     next step is the board run: about 40 interleaved boots each of the
     base arm and the KICK_DIFF arm from one image
     (`tools/cts/log/investigations/gc-kick-diff.md`, "How to run it on the
     board"). The instrument was read end to end at main `a63fcc5b` and is
     complete for that run; none of its seven gaps (G1 to G7) needs a code
     change. Selectors: base `SEL=512`; KICK_DIFF case 10 `SEL=4586`
     (`0x11ea`); case 2 `SEL=4578` (`0x11e2`); pages decode with
     `yakogl-tools record kickdiff`. Corrections to `gc-kick-diff.md`, not
     yet made: the two commands' comment "list 0" is list 1 (`0x11ea`);
     case 10 is bit 10 of the sweep pass word, not bit 9; the sample
     output's "10000 us" is 2000 us.
   - **GC7000 `TEXDESC_LOG_SIZE_EXT` for NPOT sizes** (no issue yet, low
     severity): `log2_fixp88` (`src/vivante/texture.rs`) is a linear
     approximation of log2; Mesa writes the exact value. The CTS sizes are
     within one unit; a size like 90 is 0.086 octave low. Pinned by the
     ignored test `bug_log_size_ext_is_a_linear_approximation_of_log2`; the
     fix is one function and removes the `ignore`.
   - **GC7000 `take_uniforms` truncates** (no issue yet):
     `take_uniforms` (`src/vivante/backend.rs:2287`) cuts a stage's uniform
     block at `GC_MAX_UNIFORM_DWORDS` (1280 dwords). Harmless while the
     whole block must fit the 320 registers; it becomes a silent truncation
     as soon as blocks are read from memory. The fix in "Open work" item 4
     (the GC7000 `ubo` cases) lifts it.
   - **GC7000UL: a split instanced draw that reads `gl_InstanceID` is drawn
     wrong without any message, not refused** (no issue yet). The refusal at
     `src/vivante/backend.rs:2353` tests `vs_instance_id_reg1`, and only the
     PowerVR compiler sets that field (`src/shaderc/online/powervr/chain.rs:212`).
     On the host, the dEQP mixed shader compiles to `viv.vs_id_in_reg` 3 and
     `vs_instance_id_reg1` 0. `gc_draw` takes instances 1..3, and
     `gc_refused_draws` stays 0. The six `instanced.*.mixed` cases still
     fail, but because the image is wrong. The unit test
     `an_instance_drawn_on_its_own_that_reads_gl_instance_id_is_refused`
     (`src/vivante/backend_tests.rs`) sets the field by hand, so it does not
     catch this. Fixed by V0 (or by V1 for divisors up to 0xffff; "Open
     work" item 4, es3-vertex). The PowerVR refuses as designed, confirmed
     on the host: instances 1..3 of the dEQP mixed shader return `-EINVAL`
     with `yakogl/pvr: draw refused: an instance drawn on its own (a divisor
     past 1) whose program reads gl_InstanceID`
     (`tools/cts/log/investigations/instanceid-per-draw.md`).
   - GC7000 REPEAT on NPOT sizes (stage 1 residual risk,
     `docs/es2-cts-failures.md` H.2/H.3): `tests/gc_npot_repeat.rs` pins the
     texture descriptor and sampler registers the Vivante back end emits for
     the 17 NPOT wrap and size cases against the rnndb headers and Mesa
     26.2.1. All 17 pass; nothing is wrong at the CTS sizes
     (`docs/investigations/gc-npot-repeat.md`). No board check is needed; a
     texel read-back across a REPEAT seam would show more than the
     solid-colour CTS cases do.
   - i.MX8MP case 16, 4x MSAA (#8): the default image has no 4x config, so
     the case skips its 4x half by name (`6bddfc5`). On the 4x image the
     path fails at check 4 (`x4 inside` reads 0x00000000) with and without
     `gc_debug_msaa` bit 3, for a reason still unknown. The `x4 scratch`,
     `x4 clear only` and `x4 draw only` lines that would tell a failed
     resolve from a failed draw are console output, and the i.MX8MP has no
     console. They now also go into the result record, one page of three
     words a boot (`SEL=0x1000|tag`, kind PERF; `yakogl-tools record msaa`
     decodes them): `tools/cts/log/investigations/gc-msaa-record.md` has the
     two-arm recipe. Run once on the integration image `e3efeaad`
     (2026-10-11, `tools/cts/log/board-sessions/imx8mp-gltest-r2.md`): arms
     1 and 9 both fail check 4, and the 4x scratch is empty after the frame,
     after the clear alone and after the draw alone, so nothing the frame
     does reaches the surface the probes read. The 16
     `multisample.*` dEQP cases stay NotSupported at 1x.
   - **i.MX8MP glmark2 frames wait for VSYNC** (no issue yet). `board_show`
     (`driver/zephyr/lib/board/imx8mp/board_imx8mp.c`) calls
     `imx8mp_display_set_fb(scan)` every frame, which spins until the LCDIF
     takes the new address on the next VSYNC, although glmark2 asks for
     swap interval 0. So every scene runs at a whole number of 17.0 ms
     panel periods, and the scores 37 to 46 fit that. A prediction from
     the code; step 1 of the recipe in
     `tools/cts/log/investigations/imx8mp-frame-floor.md` confirms it with
     one board run and no code change. Fix F1 there: skip the flip when the
     LCDIF already scans the buffer (and F2: honour the swap interval
     through `yakogl_zephyr_wait_vblank`).
   - The GC7000 Mesa table (`tests/reference/vivante_blobs.rs`) no longer
     lags its sources (#22, 2026-10-13): 550 rows, every source in a
     `gc7000` directory has a row under its current SHA-256 or is refused
     by the toolchain (168, by class in `tools/vivante/README.md`, "The
     table against its sources"). `tools/vivante/check_gl_test_blobs.py`
     exits 0: it reports the three `uniidx` sources as refused (named in
     its `GC7000_REFUSED` with the toolchain's message), and lists `gltest/` (69)
     and `offarm/` (13) as left out by `precompiled.list`. Open: the script
     and a key check are not in `tools/check.sh`, and `check_table_absent`
     under LTO. It matters only for images that use the table; the dEQP
     images use the compiler.
   - glmark2 terrain black (#1) and the AM62P refract depth pass (#2) are
     fixed on the host (NPOT chains, `2673197`, `77693ff`, `cc78859`; the
     pool's top region, `541c9d3`); on the AM62P at `f2a13877` all 33
     scenes ran, but no capture shows the pictures. They are not closed on
     the AM62P: at `cd738b1a` terrain and shadow run out of GPU memory and
     refract raises `framebuffer 1 incomplete` (the next item). The Pi 4 carve-out (#4,
     #5): the Pi 4 glmark2 image takes 96 MiB (`ca4074a`); the host harness
     (`tools/hostharness/glmark2/FINDINGS-v3d.md`, 2026-10-12) shows no
     refused draw and no refused allocation since the image's tiled-copy
     budget is 24 MiB (refract's 1600x1200 mip chain was over the 8 MiB
     default: `tools/cts/log/hostharness/glmark2/FINDINGS-refract-budget.md`);
     not yet run on the board.
     Since v3d-rt-sample a colour render target is sampled from its render
     image (the tile list stores RT 0 a second time, in the TMU's layout) and
     its glGenerateMipmap is the TFU's, so refract's and terrain's per-frame
     copies and CPU box filters are gone on the host
     (`docs/online-shaderc/v3d-backend.md`, "Render targets sampled from their
     render image"). Run on the Pi 4 on 2026-10-14
     (`tools/cts/log/board-sessions/pi4-rt-sample.md`): 33 ran / 0 failed /
     0 unsupported, score 188 (183 before), the TFU made terrain's chain.
     CPU time in GL calls per frame went from 711 to 188 ms in refract, from
     442 to 177 ms in terrain, from 228 to 170 ms in shadow, and from 90 to 7
     ms in desktop blur. refract, terrain and shadow still show 6 FPS. shadow
     and refract still resolve a depth texture on the CPU every frame (option
     (b), not done). Terrain's remaining CPU time is not explained yet.
   - **AM62P glmark2 terrain and shadow out of GPU memory** (no issue yet;
     `cd738b1a`, 2026-10-14, common set; `docs/board-sessions/am62p-common-int.md`):
     the score passes (465), but terrain (1 fps) and shadow (3 fps) hit
     `render pool exhausted` (5464 KiB and 21848 KiB requested, 3648 and
     2744 KiB left), and refract raises 1694 `framebuffer 1 incomplete
     (0x8cd6)` errors. The render pool (59200 KiB, about 38 MB live at the
     end of jellyfish) does not fit terrain's 800x600 NPOT mip chain (5.6 MB)
     after jellyfish; the `pvr-pool-npot` replay item is the host side of
     this.
   - The glmark2 record carries the GL core's counters (#13): GL errors
     raised and draws that sampled an incomplete texture, 4 bits each in
     w3 (`driver/docs/glmark2.md`, "The record"). The host harness shows
     both per scene. Next: an i.MX8MP glmark2 run, whose record shows them
     (also #1's check on that board).
   - The i.MX8MP panel at 69.3 MHz (`c1f0e2a6`; its DISPLAY record is
     clean): the viewing by eye is still pending (#23).
   - ES 2.0 builds still accept other ES 3-only renderbuffer formats such
     as `GL_DEPTH_COMPONENT32F` (a static audit lists 44 ungated ES 3.0
     rows, none reached by a dEQP-GLES2 case:
     `tools/cts/log/investigations/es2-error-rules.md`).
   - **gate-lib-red-unnamed** (no issue yet, 2026-10-11): the full gate on
     `05d0d8df` printed `FAIL test
     host-test,gpu-powervr,shaderc-online,precompiled-shaders` with `error:
     1 target failed: --lib` and no test name. The row's binary then passed
     20 plain runs, 20 shuffled runs and the row's own command twice, so
     the failure is not reproduced and its test is unknown. Hazards read
     in the code, none shown to be the cause: `cached_or_precompiled`
     (`src/shaderc/online/backend.rs`) runs outside the store's test lock,
     and `a_hit_runs_no_compile_and_is_what_the_compiler_makes`
     (`src/shaderc/precompiled.rs`) asserts exact deltas of the global
     `HITS` counter while the GL-level tests serialise on another mutex.
     `check.sh` now keeps a failed row's whole output in
     `build/check-fail/<n>.log`. Open until a red names its test; the next
     red of this row is read from that file, never rerun first.
   - **`GL_BGRA_EXT` renderbuffers** (no issue yet): the 46 failures of entry
     21 (`fbo`) on the i.MX8MP at `835cf9cc` (and the same 46 on the Pi 4 at
     `20a48791`, `1344748`). The report-es2 renderbuffer allowlist of
     `8bbf9ba2` names `GL_BGRA8_EXT` but not the unsized `GL_BGRA_EXT`
     (`docs/board-sessions/imx8mp-es2-final.md`, `da842c6`). Fixing the
     allowlist should bring the i.MX8MP back to 0 fail.
   - **Pi 4 gl_test 196 `occl`** (no issue yet): the V3D occlusion counter
     reads 0 for every query on the board (q0..q4 `available 1`, results
     0,0,0,0,0; want 0,1,0,1,1), though it counts on the host (`cd738b1a`).
     Check `OCCLUSION_QUERY_COUNTER`'s enable and read path on hardware.
   - **Pi 4 gl_test 204 `srgb`** (no issue yet): the case prints "the render
     half is the PowerVR's (this GPU refuses an sRGB target by name)" and
     then FAIL, with no checks, although the sample half is meant to run on
     all three boards. Read case 204 in `driver/tools/pvrlab/gl_test.c` for
     the V3D guard.
   - **`board.sh pi4 tree` as a background job** (no issue yet): run as a
     session's background job it is killed at the tool's time limit; it died
     while FB3 was on the board, left the staged `yakogl-FB3.bin` and
     `tryboot.txt` on `/boot/firmware` (a leftover refuses the next run) and
     printed no tally. Run the tree in rows of at most four, or in a job with
     the longest timeout.
   - **`tools/check_scope_selftest.sh` fails at the tip** (no issue yet; lines
     88 and 145): full `tools/check.sh --jobs 3` at `cd738b1a` read 148 rows,
     145 ok, 3 failed, 0 gaps (`docs/investigations/gate-red-tip.md`). The
     self-test pins the row count at 149 (real 148); the other two red rows
     are the AM62P Zephyr image rows, which need the git-ignored
     `driver/src/fw/pvr_fw_blob.h` (absent on that worker). The clippy lints,
     the `tf.rs` / `tf_tests.rs` lints and the `report-es2` `--lib` failure of
     earlier reviews do not exist at the tip: `clippy-gate-fix` has nothing
     to fix on main. The tools-only item is in "Open work" item 7.
   - **i.MX8MP GLES3 stage-1 tooling** (no issue yet): failing GLES3 case
     names print as `crc:<hex>`, not as dEQP names (map them with the table);
     `tools/cts/am62p-deqp-images.sh` lacks `-DCONFIG_YAKOGL_REPORT_ES2=n`;
     `cts-build-zephyr.sh` now drops a stale CMake cache and prints the
     failing ninja step, and the same fix applies to the AM62P and Pi 4
     stage 1 builds.
   - **PowerVR vertex-shader `textureSize` sampler** (no issue yet): a
     `sampler2DArray` that a vertex shader reads only through `textureSize`
     has no uniform location (`glGetUniformLocation` answers -1), so the
     application cannot set its unit.
     `src/shaderc/online/powervr/chain.rs` keeps only the sampler rows a `Tex`
     op samples (`row_sampled`); `tests/es3_tf_sampler.rs`
     (`texture_size_of_each_target`) works around it with an extra
     `texture()` read.
   - **`GL_HALF_FLOAT_OES`** (0x8D61, no issue yet): `attrib_type_size` and
     `attrib_component` do not handle it, so it is read as float32 words.
     Left as is: the ES 2.0 answer is unchanged, and the type is accepted
     only under `report-es2`. It is wrong if an OES half-float array is ever
     used; a separate item should decide.
   - **Vertex array integer fill follows the array** (no issue yet): the
     w fill of a narrow integer array follows `ga.integer`, not the type of
     the shader input. A float array bound to an `ivec4` input is undefined
     in GL, so this agrees with ES 3.0 for every defined case.
   - **Vertex array convert path with no CPU view** (no issue yet): the
     `gpu_read` branch is not covered by the host test, because its OSAL
     stub has none.
   - Capture against draw, differences by design (`tests/es3_tf_sampler.rs`):
     the capture applies the texture swizzle and GL_TEXTURE_MIN_LOD / MAX_LOD,
     which the draw does not yet (`GlTexture` comment "the draw does not apply
     them yet"); it filters cube maps seamlessly (ES 3.0 3.8.9.1); it computes
     a 3D texture's chain length from max(w, h, d) where `draw.rs`
     `mip_levels_needed` uses w and h. A sampler object's WRAP_R and LOD range
     are not exposed by `es3.rs` (`sampler_state` returns min, mag, wrap S,
     wrap T), so the capture uses the texture's, as the draw does.
   - **Depth blit and the back end's buffer** (no issue yet; host):
     A depth blit into a depth texture changes the texels, which is what a
     sampler reads. It does not change the back end's depth/stencil buffer
     for that texture. A draw that later depth-tests against the texture,
     with the texture as its depth attachment, still sees the depth it had
     before the blit. The next resolve after such a draw also overwrites
     the texels with that buffer. `glTexSubImage2D` of depth has the same
     gap today (`src/gl/texture.rs` `resolve_depth`: "Not the other way
     round"). The `ds_write` entry of blit-ds (item 4) closes both.
4. **ES 3.0, host first.** Goal 4. A proposed order, not a ranking anyone
   measured. Each item keeps every `report-es2` answer and adds a test that
   pins it. What each item built is in the ES 3.0 table above; here is what
   remains. The full wording as of 2026-10-11 is in the record file.
   - **tf-capture**: gl_test 203 `tfcap` passes on the AM62P and the GLES3
     `transform_feedback` rows pass (882 / 882, `cd738b1a`); the board next:
     203 passes on the Pi 4 (9 / 9, `cd738b1a`); the i.MX8MP next.
     `lifetime.attach.deleted_output.buffer_transform_feedback` still fails
     on the AM62P. Left: the V3D's hardware path
     (`docs/es3-transform-feedback.md` section 4.3) only if an application
     needs the speed. A vertex shader that samples a texture is evaluated
     now (host only); still refused, each by name in the log: a shadow
     look-up, `textureGrad`, a compressed texture (its format named), a
     texture with GL_TEXTURE_BASE_LEVEL other than 0 (the draw does not apply
     it either), a sampler whose component type does not match the texture's,
     storage with no CPU view. What each code generator lets reach a capture
     (the capture is the same on the three GPUs; a program the code generator
     or the link refuses never draws): the PowerVR refuses `texelFetchOffset`,
     `textureGrad` and an integer result from a 2D array (and cube, 3D); the
     GC7000 refuses `textureOffset`, `texelFetchOffset`, `textureProj`,
     `textureGrad`, shadow samplers, integer results and `textureSize` at a
     level other than a literal 0; the V3D refuses 3D and 2D-array samplers,
     `texelFetch`, offsets, `textureGrad`, shadow samplers and integer
     results. `texelFetchOffset` and an integer 2D array reach no capture on
     any GPU yet (`the_texel_source_reads_what_no_code_generator_selects_yet`
     in `tf_capture_tests.rs` asks the texel source for them directly): these
     are code-generator items, not capture items. Stale text for the next
     item that owns the files: `src/GL_CORE_NOTES.md` lines 394 and 634 and
     `docs/es3-transform-feedback.md` section 4.1 "Vertex textures" (line
     228) and section 5.7 still say the capture refuses a vertex shader that
     samples.
   - **int-rt**: the PowerVR's is built; its proof is the AM62P (gl_test
     200, and the `texture.specification`, `fragment_out` and
     `shaders.builtin_functions` buckets). Left on the PowerVR: an integer
     attachment beside other attachments, colour masks that keep some of an
     integer target's channels, precompiled rows for the integer variants.
     The GC7000 and the V3D: none.
   - **formats**: each back end samples the new storage classes of
     `docs/es3-sized-formats.md` and gains isampler / usampler. Left on the
     PowerVR: integer cube / 3D / 2D-array samplers and `RGB10A2` as a
     class. The GC7000 and the V3D: none.
   - **srgb**: the PowerVR's is built (gl_test 204 and the dEQP-GLES3 cases
     at the top of `docs/es3-srgb.md`). Left: the GC7000's and the V3D's
     sRGB render targets (`docs/es3-srgb.md` 5.1, 6.1), an sRGB attachment
     beside another on the PowerVR, compressed sRGB.
   - **occlusion-query**: gl_test 196 and `functional.occlusion_query` (62 /
     62) pass on the AM62P (`cd738b1a`); 196 fails on the Pi 4 (the counter
     reads 0, "Known defects"); the board next: 196 on the i.MX8MP and the
     Pi 4 after its fix, and the 4 `negative_api` cases on the AM62P. The unverified
     points are `docs/es3-occlusion-query.md` section 9.
   - **pvr-2darray**: PowerVR 2D-array and shadow sampling, designed in
     `docs/es3-pvr-2darray.md`.
   - **buffer-pbo**: gl_test 205 `mappbo` passes on the AM62P (15 / 15,
     `cd738b1a`), and on the Pi 4 (15 / 15, `cd738b1a`); the board next: 205
     on the i.MX8MP, then the AM62P GLES3 tree's images 1 `buffer.*`, 5 `pbo.*`, 6 `read_pixels.*` and 23
     `texture.specification.*`). Left: the 3D and 2D-array cases of those
     groups need `pvr-2darray`, and the depth ones the PowerVR's depth
     sampling.
   - **es3-shaderc**: the online compiler's GLSL ES 3.00 gaps
     (`docs/es3-shaderc-coverage.md`: PowerVR 3,457 / GC7000UL 3,457 / V3D
     3,457 of 3,461 expected-build programs build, `tests/es3_shaderc_corpus.rs`
     with `DEQP_GLES3_TEST_DIR` = VK-GL-CTS `93bca018`; the V3D's base was
     not run). Backslash-newline inside a preprocessing token is closed (the
     4 `line_continuation.preprocessing_token_*` / `.token_*` programs on each
     GPU), but the cause was the harness, not the compiler: dEQP's
     `parseShaderSource` removes the extra indentation, so the line after the
     backslash starts in column 0, and the corpus now has
     `remove_extra_indentation`; `lexer.rs` `splice_lines` already spliced
     (`tests/es3_shaderc_pp_splice.rs` pins it, ES 1.00 unchanged; the
     `preprocessor` row is 292 built, 0 refused). Remaining expected-pass
     refusals on the PowerVR: 2 `constant_expressions.other.complex_*` (case
     labels) and 2 `uniform_block.valid.long_instance_name_*` (reflection
     name). Open: the rest of its "Work that closes the table"; one needs a
     decision first: names of 63
     characters or more (an instance block's default-block rows are named
     after the instance, so a 1,024-character instance name fills the
     64-byte field).
     - The indexed element passed as an out/inout argument (RC13, 8 / 0 / 8)
       needs no work: at `cd738b1ae7cf` the 8 programs
       (`indexing.moredynamic.{builtin_fncall_out_parameter,
       user_defined_fncall_out_parameter, user_defined_fncall_inout_parameter,
       user_defined_fncall_inout_parameter_with_index_with_side_effects}`,
       both stages) build on the PowerVR, the GC7000UL and the V3D. They were
       refused for the missing `trunc` (`modf`), fixed by `b1cf9e4`; the
       indexed copy-in/out (`LRef` in `lower.rs`) was already correct
       (`tools/cts/log/investigations/shaderc-inout-indexed.md`).
       `docs/es3-cts-failures.md` sets the RC13 row to 0 / 0 / 0 and no
       longer lists "indexed out/inout arguments" in the "no item yet" line.
     - Item 4 of that table, the GC7000UL's 128-element constant array, is
       done (gc-const-array, 2026-10-14; `alloc.rs` `sink_definitions`,
       `tests/es3_gc_const_array.rs`), and not the way the item said: the
       cause was register pressure made by a scheduling pass, not a missing
       relative addressing into the constant file. The front end reads
       `array[i]` as the chain `acc = (i == k) ? c_k : acc` (`lower.rs`
       `select_among`), which needs 3 temporaries for any length;
       `sink_definitions` moved each select past the next compare, so all
       127 compares ended up before all selects (35 and 34 temporaries,
       over `abi.rs`'s 21). The pass now does not move an instruction past
       a sibling when the move keeps two or more of its own sources alive;
       both programs need 3. Over the 809 shaders under `tools/shaders` and
       `tests` on the GC7000: no compile verdict changed, 26 need fewer
       temporaries, 4 need one more. The precompiled GC7000 tables lost 80
       code bytes each (`es2=0 app=all` 741,888, `es2=1 app=all` 245,232);
       `src/vivante/internal.rs` is unchanged.
     - Why not the address register: `src/vivante/VIVANTE_NOTES.md` (gl_test
       108) measured that `movar` then `u[base][a.x]` reads element 0 on the
       i.MX8MP whatever the index. Mesa etnaviv reads constant data from
       memory with `load` (`load_constant` from
       `ETNA_UNIFORM_CONSTANT_DATA_ADDR`); if an array ever needs memory,
       that is the path to copy, like the `SHC_VIV_CONST_UBO_ADDR`
       uniform-block copy.
     - Possible follow-up, not done: a stricter rule also builds 5 more
       Qt3D vertex shaders
       (`tools/shaders/qt3d/{0d972b456ed599d1,56105a9f4a2e536b,928dc4e0891ab537,a9805e68997ed2ee,c667c97598a82c1b}.vert`)
       but raises 17 others by up to 3 (particle vertex shaders 17 → 19,
       against the limit of 21). A real fix is a scheduler that tracks
       register pressure, not another rule in this pass.
     - shaders-fold (2026-10-14; `tests/es3_shaderc_fold.rs`): `round` was
       floor(x + 0.5), which rounds before the floor (0.49999997 + 0.5 is
       1.0; above 2^23 an odd integer + 0.5 goes to the even one above), and
       `roundEven` built on it. `builtin_call.rs` now takes t = trunc(x) and
       d = x - t, both exact, and adds `+ 0.0` so a zero is +0.0 (ES 3.00
       only; no ES 2.0 answer changes). Host: 4 / 4 on the PowerVR, the
       GC7000UL and the V3D; `es3_shaderc_corpus` unchanged (3457, 3455,
       3457); no fixture moved. Expected on the AM62P, not run on a board:
       the 16 `builtin_functions.common.round` / `roundeven`
       `{float,vec2,vec3,vec4}_highp_{vertex,fragment}` cases turn green
       (image 7, line 14 of `tools/cts/am62p-gles3-tree.txt`), and the same
       lowering serves `builtin_functions.precision.round*` /
       `roundeven*` among image 7's 987 cases without a result. The 16 were
       the largest bucket whose cause is in `src/shaderc/online/{fold,
       fmath,builtin_call,loop_count}.rs` in the 2026-10-13 per-case list
       (`git show cd238b6:tools/cts/log/board-sessions/am62p-gles3-percase-2026-10-13.notpass.tsv`);
       no other bucket is there. The next value bucket of `shaders` is
       `builtin_functions.common.modf` (24 cases, "Result comparison
       failed"), not in those files: the IR evaluator gives the exact modf,
       and dEQP's executor binds two integer colour attachments for modf's
       two outputs, which the PowerVR back end does not draw
       (`src/pvr/backend.rs`, "an integer colour attachment beside
       another"; owner: int-rt-pvr follow-up; why the case reads "Result
       comparison failed" and not "Error in draw" is not traced). Not on
       the board list: `packSnorm2x16` / `packUnorm2x16` still round with
       floor(x + 0.5) (`builtin_call.rs` `pack2x16`; only an input just below
       .5 of a step goes the wrong way; dEQP's pack_unpack cases pass on the
       AM62P).
   - **blit-ds**: the depth/stencil half of `glBlitFramebuffer`. Done on
     the host for the planes the GL core holds: depth copied on the CPU
     into the R32F texels of a 2D depth texture, read from a 2D depth
     texture's texels or from a depth renderbuffer's depth
     (`backend::depth_resolve`), with the colour path's mapping, flips,
     cut and scissor (`src/gl/blit.rs` `copy_depth`). Still
     GL_INVALID_OPERATION, each cause logged once: (a) depth into a
     renderbuffer or the window, (b) all stencil, (c) depth from the
     window or from a cube/array depth texture. These are all 16 AM62P
     `fbo.blit.depth_stencil` failures, because dEQP blits between
     renderbuffers. (a) and (b) need two new back-end entries
     (`src/backend.rs`, one per back end):
     - `ds_write(id, plane, src, x, y, w, h, pitch) -> i32`: write a
       rectangle of plane DEPTH (f32 in [0, 1]) or STENCIL (u8) of
       object `id`'s buffer from the CPU. Take a buffer for `id` if it
       has none (refuse as `ds_retention::refuse` does when the table is
       full), and leave the plane `Plane::LIVE`. If the plane is
       `CLEARED(v)`, fill the buffer with `v` before the rectangle is
       written. The layouts are the ones `depth_resolve` reads: on the
       PowerVR, packed D24S8 rows at the ZLS-tile-aligned width; on the
       GC7000, D24S8 row-linear at the surface stride, depth in bits
       31..8 and stencil in 7..0; on the V3D, UIF D24S8 without XOR
       (the store side of `v3d_tex_load_box`).
     - `stencil_resolve(id, dst: *mut u8, w, h, pitch) -> i32`: the
       stencil half of `depth_resolve`, with the same answers (0 / 1 /
       negative errno).
     With these two entries, `copy_depth` writes through `ds_write`
     instead of into texels. A stencil copy is the same loop over u8.
     (c) also needs a measurement: which way up each back end keeps the
     window's depth (the window's colour is stored top-down;
     `default_y_up`).
   - **es3-caps**: redo the ES 3.0 limits test, reverted for review
     (`3f78982`).
   - **es2-report-gaps**: the four `BUG` pins of `tests/es2_report_guard.rs`
     (ES 3.0-only pnames and capabilities answered, four ES 3.0
     renderbuffer formats accepted, ES 3.0-only pixel types
     GL_INVALID_OPERATION instead of GL_INVALID_ENUM under `report-es2`).
   - **es3-vertex**: the AM62P vertex groups, `attribute_location` 48,
     `vertex_arrays` 41, `instanced` 6 fail on `f2a13877`; the task list
     and causes are in `docs/es3-cts-failures.md`.
     - `attribute_location`: `glBindAttribLocation` no longer moves a vertex
       input that has `layout(location = N)` (`src/gl/program.rs`:
       `ExplicitAttribs::scan` reads the qualifiers of a `#version 300 es`
       vertex shader, `attrib_layout` and `build_attrib_indices` honour them,
       `link_attrib_layout` recompiles with the decided layout when it
       differs; ES 2.0 and `#version 450` sources read nothing, so
       `report-es2` answers do not change). `tests/es3_attrib_location.rs`
       reproduces `mixed` and `mixed_time.*`; expected effect on the AM62P:
       the 28 `mixed` / `mixed_time` failures (task T4), not run on a board.
       Still open: `mixed_max_attributes.*` (20 AM62P cases). When the
       compile puts two inputs on one location the record has one row for it
       (`src/shaderc/online/powervr/abi.rs`, the `find(|d| d.location ==
       ...)` merge), so the link keeps the compile's placement for such
       shaders (`layout_differs` returns false when an active input has no
       row). **`src/shaderc/online/reflect.rs` must change** (held by
       tf-infolog when written): in `reflect()`, the
       `StorageQualifier::Attribute` branch, an input without
       `layout(location)` must get the lowest run of free locations that no
       `layout(location)` of the shader uses and no earlier input took,
       instead of `attrib_loc`. Sketch (proven on a throw-away patch, not
       committed): collect a bit mask `explicit` of `l .. l + locations(base)
       * array_len` for every top-level attribute with `io.location ==
       Some(l)`; keep a mask `placed` of the locations earlier unqualified
       inputs took; for an unqualified input of `n = locations(base) * count`
       locations start at 0, advance while any of `loc .. loc + n` is in
       `explicit | placed`, use `loc` and add it to `placed`; an input with a
       qualifier keeps `attrib_loc = l`. A shader with no qualifier reflects
       as before. With it `mixed_max_attributes_vec4` and
       `unqualified_unbound_inputs_avoid_every_qualifier` in
       `tests/es3_attrib_location.rs` pass: remove their `#[ignore]` in the
       same change and add the reflect.rs unit test. Limit of the source
       scan: a location written through a macro, or `#if`-ed out qualifiers,
       are not seen (the input then counts as unqualified); dEQP writes the
       plain form.
     - `vertex_arrays` (tasks T2 and T3, 41 AM62P failures: 20
       int2_10_10_10 and 21 components{2,3}_{i,u}vec4): fixed on the host
       ("Vertex array conversion" in the ES 3.0 table above). Not yet seen on
       a board.
     - `instanced`: the 6 failures are `{draw_arrays_instanced,
       draw_elements_instanced}.mixed.{2,4,20}_instances`; the other 39 pass.
       The cause is in the back ends, not in `src/gl/vertex.rs` (divisors 0,
       1, 2 are stored and returned correctly, `tests/es3_vertex_api.rs`).
       The PowerVR and the GC7000UL take divisor 1 only, so `draw_instanced`
       (`src/gl/draw.rs`) draws instance by instance and feeds each
       instance's divisor-array value as a constant, with
       `GlesDraw::instance_id` set to the instance number, while the PDS /
       FE starts `gl_InstanceID` at 0 in every draw. The PowerVR then
       refuses the draw (`src/pvr/backend.rs` draw entry ~4590 and chunking
       exception ~4826, `-EINVAL`); the GC7000UL draws it wrong without a
       message ("Known defects"). The V3D takes any divisor (`0xffff`),
       draws once and is predicted to pass. `tests/es3_instanced.rs` (host
       only) pins the part the GL core owns. The design is chosen
       (`tools/cts/log/investigations/instanceid-per-draw.md`):
       - PowerVR (plan #18): **P1**, Mesa's PDS `instance_id_modifier` set
         to `GlesDraw::instance_id`. It is a new parameter of
         `pvr_pds_render_vertex_program_ids` (`driver/src/pvr_pds_render_gen.{c,h}`,
         `src/pvr/ffi.rs`, `src/pvr/ffi_layout_check.c`), the refusal at
         `src/pvr/backend.rs:4585-4599` is removed, and `VDATA_DWORDS`
         grows. About 60 lines plus tests. Then **P3**: hardware divisors
         through the PDS stream `multiplier`/`shift`, with
         `max_instance_divisor` 0xffff.
       - GC7000UL (plan #18, compiler part plan #19): **V0**, the GC7000UL
         online compiler sets `ShcShader::vs_instance_id_reg1` when the
         program reads `gl_InstanceID` (`src/shaderc/online/select.rs:3983-3987`,
         `src/shaderc/online/abi.rs`). Plus **V1**, the stream divisor from
         `GlesAttrib::divisor` (`src/vivante/backend.rs:2261`) and
         `max_instance_divisor` 0xffff (`:2886`). The FE already gets the
         divisor register 0x14680 on every draw. This is what Mesa etnaviv
         does on HALTI2+.
       - `src/gl/draw.rs` (plan #19) needs no change for any of these.
         dEQP was not run.
   - The GC7000 refuses four `ubo` cases at link for constant registers:
     `functional.ubo` `single_nested_struct_array.{per_block_buffer,single_buffer}.{shared,std140}_instance_array_both`
     (the four `KNOWN_LIMITS` of `tests/es3_ubo.rs`). Each stage needs 184
     constant registers (108 for the copied block, 76 for literals), 368
     against the 320 the i.MX8MP's GC7000UL has (`NumberOfConstants` 0x140
     in Mesa 26.2.1's hwdb). The 320 is real; the cases are fixable. Mesa
     etnaviv reads every user UBO from memory with `LOAD` and puts only
     default-block uniforms in the constant file
     (`etnaviv_nir_lower_ubo_to_uniform.c`, `block > 0` stays a load).
     Reading block-member rows with `load` here (the path of
     `LoadUniformIdx`, `SHC_VIV_CONST_UBO_ADDR`, `ubo_copy`) would need
     about 77 registers per stage. The fix design (files and sizes) is in
     `tools/cts/log/investigations/gc-ubo-const-limit.md`:
     `src/shaderc/online/codegen.rs` (~25 lines), `select.rs`
     (`uniform_block`, `uniform_src`, ~80), `src/vivante/backend.rs`
     (`take_uniforms` cap of 1280 dwords, ~30; "Known defects"). gc-const-array
     has merged, so nothing else holds the vivante code generator now. One
     i.MX8MP board run is needed for a constant-offset `load` from a user
     block; it is not yet done.
   - **pvr-3d** (new, not in any plan;
     `tools/cts/log/investigations/pvr-3d-mipmap-hang.md`): (a) native 3D
     textures on the PowerVR, or levels and wrap modes in the grid
     emulation (fixes most 3D filtering failures and the slow check;
     "Known defects", GLES3 hang); (b) `tools/cts/gen-am62p-gles3-tree.py`:
     name only the mustpass prefixes in `3d.formats` (image 20 runs 12 BGRA
     cases and `rgba16f_linear_mipmap_linear` outside the mustpass), and put
     the 3D `*_linear_mipmap_linear` cases in images of their own, so the 83
     tail cases report; (c) `tools/board.sh`: an AM62P dEQP image's
     `RUN_BOUND` should be at least the 600 s watchdog bound + 60 s + the
     boot, so the RTI reset is inside the capture.
   - **fbo-completeness** (plan #19 item `fbo-fragops`): done on the host
     for the three rules in the ES 3.0 table (base integration tip `20a4879`).
     Left, each with its file: separate depth and stencil images with a
     colour attachment 0 (12 `attachment_combinations` cases: drop the
     `fb.color.type_ == GL_NONE` term in `check_status`, delete the ignored
     marker of `separate_depth_and_stencil_with_colour_zero_are_unsupported`,
     and edit four unit tests that build ES 2.0-style separate D16 + S8 pairs:
     `src/gl/draw.rs` `fbo_depth_and_stencil_renderbuffers_both_name_the_ds_buffer`
     and `one_depth_stencil_texture_on_both_points_is_one_object`,
     `src/v3d/backend_tests.rs` `fbos_sharing_a_depth_renderbuffer_with_their_own_stencil`
     and `a_recreated_stencil_renderbuffer_keeps_the_fbos_depth`; the two V3D
     rigs need one D24S8 renderbuffer or `report-es2`); the 10 `fbo.depth`
     clamp cases (`gl_FragDepth`, PowerVR shader back end) and the 5 `basic`
     ones (a board); `GL_R32F` as a render target is a decision (ES 3.0
     wants `GL_EXT_color_buffer_float`); `renderable.texture.color0.rgba32f`
     (texture.rs must remember the sized format); the PowerVR's 19 integer
     texture attachments (the texture half of int-rt) and the GC7000UL / V3D
     75 refused uploads (`formats-gc`, `formats-v3d`); the 4 unsized depth
     textures (texture.rs must refuse them under `!REPORT_ES2`);
     `completeness.layer.3d_64_15` on the PowerVR (the slice's offset and
     pitch in `attachment_storage` and `loc_pitch`, then every consumer); a
     draw into colour on attachments 1.. only
     (`GL_INVALID_FRAMEBUFFER_OPERATION`: `target_loc` and
     `fb_target_drawable` must follow `draw_buf`, item `mrt4`). The board
     sessions should run `fbo.completeness.size.distinct` and `fbo.depth.*`
     with a watch for MMU faults (a depth image of another size than the
     colour has not been drawn into on a board).
   - **es3-mrt** (plan #19 item `mrt4`, RC14 in `docs/es3-cts-failures.md`):
     the GL-core half is done (integration tip `2769e77d`). The draw-buffer
     count is the back end's `backend::DRAW_TARGETS` (3 on every GPU):
     GL_MAX_DRAW_BUFFERS, GL_MAX_COLOR_ATTACHMENTS, GL_DRAW_BUFFERi,
     glClearBuffer's range, the attachment-point checks and the shader's
     `gl_MaxDrawBuffers` read it, so nothing is advertised that a back end
     cannot draw. The framebuffer stores 4 colour points
     (`GL_CAP_COLOR_ATTACHMENTS` 4; `struct gl_framebuffer` 92 -> 108 bytes,
     +8 KiB `.bss` with 2 contexts); a draw and a clear hand a 4-target back
     end all 4 slots (host test, fake back end); `tools/yakogl-tools/src/es100.rs`
     reads `max_draw_buffers()`. glDrawBuffers now answers
     GL_INVALID_OPERATION (was GL_INVALID_ENUM) for `GL_COLOR_ATTACHMENTm`
     with m below 32 and at or past GL_MAX_COLOR_ATTACHMENTS (ES 3.0 4.2.1).
     Left, the back-end half: each GPU raises its `DRAW_TARGETS` arm in
     `src/backend.rs` to 4 once a board draws 4 targets; the queries,
     attachment points and `gl_MaxDrawBuffers` then follow. The same change
     moves `tests/es2_report_guard.rs` `draw_buffer_queries_are_invalid_enum`
     and `tests/es3_caps.rs` (caps-land, not on main yet) from 3 to 4.
       - PowerVR (AM62P): the end-of-tile programs and the kick's PBE words
         exist for 3 targets (`PVR_RENDER_MAX_RT` 3 in
         `driver/src/pvr_render.h`, `eot_doutu_src0_n[]`;
         `src/pvr/backend.rs` `PVR_BACKEND_MAX_RT` clamps to 3). A fourth
         needs PBE state per render target (`docs/es3-pvr-formats.md` 1.2,
         2.5) and a fourth end-of-tile program; the USC has 4 output
         registers (1.8) and the compiler writes 4 (`select.rs`
         `MAX_COLOUR_OUTPUTS`).
       - V3D 4.2 (Pi 4): the TLB takes targets 0-3, the compiler writes 4
         (`shaderc/online/v3d/prepare.rs` `MAX_RTS`), the backend takes up
         to 4 (`docs/online-shaderc/v3d-backend.md`, "Multiple render
         targets", step 4 pending). Left: a board run that draws into
         target 3, then `DRAW_TARGETS` 4.
       - GC7000UL (i.MX8MP): the PE has 4 render targets (etnaviv `num_rts`,
         halved for 64-bit formats) and `vivante::backend::mrt_targets`
         takes up to 4. Check on the board that the shader back end writes a
         fourth output and the PE programs RT 3, then `DRAW_TARGETS` 4. A
         64-bit format may halve the count: that needs a per-format check at
         framebuffer completeness, not a lower `DRAW_TARGETS`.
     Found on the way, not done: `tools/check.sh` lacks `--test es3_mrt_gl`
     in the `$V3D_CONFIG` and `$V3D_ES2_CONFIG` rows (it passes there by
     hand); the GL core does not check a fragment output at a location at or
     past GL_MAX_DRAW_BUFFERS (`src/gl/program.rs`,
     `src/shaderc/online/sema.rs`; each back-end compiler refuses it its own
     way; whether ES 3.0 3.9.2 makes it a link error is unchecked);
     glFramebufferTexture*/Renderbuffer and the attachment query answer
     GL_INVALID_ENUM for a colour point past the limit where Mesa answers
     GL_INVALID_OPERATION (GL 4.5 9.2.8), both readings exist, unchanged.
   - Then the dEQP-GLES3 tree on each board, after its ES 2.0 tree. The Pi 4
    has 84 of its 91 images left (FB4 is built, `build/` only); each ran in
    under 30 s of its 240 s window, so the table could be re-split into far
    fewer images. Its failures so far, not yet classified case by case, are
    `fbo.blit.conversion` 261, `fbo.completeness.renderable` 105,
    `fbo.color.*` about 190, `blit.default_framebuffer` 31,
    `attribute_location.mixed*` 48, `implementation_limits.*` 11 and
    `draw.random.*` 14: the integer, MRT, 2D-array and layout items of the
    plan. `tools/cts/common-images.sh` cannot set `CONFIG_V3D_GL_TEST_CASE(S)`;
    a case option would save editing the Kconfig for every build. The
    i.MX8MP's board run follows `docs/es3-board-recipe-imx8mp.md`, with an
    image named `es3-<hash8>` rebuilt at the commit the board runs
    (`tools/cts/imx8mp-gles3-artefacts.sh es3-<hash8>` prints its size and
    sha256).
5. **ES 2.0 final sessions** (plan #20). `es2-invariance` is done: the
   PowerVR fix (`f9688bc5`, merged by `e17fb6a7`) needs no further source
   change. The final sessions (`am62p-es2-final-a`, `am62p-es2-final-b`,
   `imx8mp-es2-final`, `pi4-es2-final-a`, `pi4-es2-final-b`) and ccfarm's
   post-promote `deqp-tree` stages compare their per-image or per-row
   counters with `es2-final-expected.md` and name every difference by case.
   - Stage 1 residual risk "the PowerVR TPU's level arithmetic for
     `2d.npot_size`" (`docs/es2-cts-failures.md` H.5) is host-pinned by
     `tests/pvr_npot_levels.rs` (5 tests pass; the code and Mesa's layout
     rules agree, no bug found; `docs/investigations/pvr-npot-levels.md`).
     The board is open: see "Waiting for a board". The old scope of
     `am62p-es2-r1c` (images 8 to 17, ms4, gl_test) is not repeated; its
     record is `tools/cts/log/board-sessions/am62p-es2-r1c.md`.
   - Follow-up for a `src/` change: `build_tex_desc` is private, so the test
     derives the texture words from the bind. Exposing it under `host-test`
     would let the test compare the real words.
6. **Publication (stage 3)**, release checklist `docs/release/checklist.md`
   (walkthrough at `a63fcc5b`, `docs/release/readme-walkthrough.md`):
   - Line 5 (DESIGN.md, every named path exists): **met** (three stale
     references fixed; 132 paths checked).
   - Line 3 (CONTRIBUTING.md commands run as written): **partial**. The
     README commands ran at `a63fcc5b`: the CMake host build and the five
     `cargo test` rows pass; `--quick` is clean (119 ok, 0 failed, 0 gaps,
     236 s) only with `MESA_V3D_TREE` unset, otherwise 2 rows fail. The full
     gate was not re-run. CONTRIBUTING.md says so and names the `env -u` form.
   - Line 6 (walkthrough): **partial**. README findings 1 to 10 are open
     (steps 4 to 6 of its Building block fail as written), listed with line
     and proposed text in the walkthrough doc; the item that holds README.md
     (`pub-roadmap`) applies them. The line becomes met when they are applied
     and a `tools/` item makes `--quick` exit 0 with `MESA_V3D_TREE` set to a
     tree without `build-v3d`.
   - Line 4 (SECURITY.md): unchanged; the contact and two other
     `TODO(owner)` lines are the owner's.
   - For a `tools/` item: `tools/check.sh:17` (`--quick` wording) and
     `tools/check.sh:2519` (a symbolic `ZEPHYR_WS` breaks the AM62P Zephyr
     rows; cause not confirmed by a build).
   - Owner: CONTRIBUTING.md says a pushed branch is never rebased; the ccfarm
     `conventions` text says branches are rebased onto origin/main.
7. **Gate rows at the tip** (tools only; `docs/investigations/gate-red-tip.md`):
   fix the two row-count pins in `tools/check_scope_selftest.sh` (pinned 149,
   real 148); make the AM62P Zephyr image rows a `gap` when
   `driver/src/fw/pvr_fw_blob.h` is absent; stop nested runs clobbering
   `build/check-fail/<n>.log`; correct the `--quick` comment.
8. **i.MX8MP common-set follow-ups** (`b568940`, 2026-10-14; record
   `records/imx8mp/common-b5689400`, session doc
   `docs/board-sessions/imx8mp-common-int.md`):
   - Apply the i.MX8MP cell text once the files are free (another plan held
     them); each cites `b568940` and the record:
     - `tools/cts/RESULTS.md`, i.MX8MP common-set cells: `GM score 43, 33 ran,
       0 failed, 0 unsupported; SO pass (crc16 c592); SV pass (crc16 c592); CL
       pass (crc16 b107); GT 48 of 49 run, case 16 skipped, failing case 44
       only (cases 2, 10, 171 passed)`, on `b568940` (2026-10-14).
     - `tools/cts/STATUS.md` (Japanese): `i.MX8MP 共通セット (b568940,
       2026-10-14, records/imx8mp/common-b5689400): GM スコア 43 (33 シーン実行、失敗 0)、SO・SV・CL は
       pass (crc16 c592 / c592 / b107)、GT は 49 件中 48 件 pass (case 16 は skip、失敗は case 44
       のみ。case 2・10・171 は pass)。`
     - `README.md` Status table, i.MX8MP row: `GM 43 / SO pass / SV pass / CL
       pass / GT 48 of 49 run (case 16 skipped, case 44 fails)` (`b568940`).
   - Case 171 (`viewwin`) passed at `b568940`; it can leave the queue (confirm
     once more on the next GT run, since 2 and 10 are random). Cases 2 and 10
     pass or fail at random (`docs/investigations/gc-case10-flake.md`); they
     passed in this one run, which settles nothing.
   - glmark2 terrain and refract on this board are still not judged per scene
     (no console on the i.MX8MP): the record gives counts only.
   - V3D `mem.cpu` NULL guard (`src/v3d/backend.rs`): the kept-image path at
     about line 3087 does `(mem.cpu as *mut u8).add(pad)` after `gpu_alloc`
     without a null check; lines 2150, 4089, 4110 and 4225 do check. Add the
     same guard, free the allocation and return `KeptMiss::NoMemory` (or
     `Address`).
   - osal-linux V3D row in `tools/check.sh`: the host row list (about line
     2274) has `osal-linux,gpu-powervr,...` and `osal-linux,gpu-vivante,...`
     but no V3D entry; add one so the host-harness V3D build stays checked.
   - Re-check `docs/benchmarks.md` code references with
     `python3 -I tools/docs/check-refs.py docs/benchmarks.md`. At `b568940`
     `tools/docs/check-refs.py` does not exist (no `tools/docs/` directory),
     so the check could not run; it is probably on an unmerged branch.
9. **AM62P common-set follow-ups** (integration `cd738b1a`, build id
   `d5cb878`, 2026-10-14; session doc `docs/board-sessions/am62p-common-int.md`):
   - glmark2: the score passes, but terrain and shadow run out of GPU memory
     and refract raises `framebuffer 1 incomplete` ("Known defects"), so #1
     and #2 are not closed on this board. Fix the render pool (or the order
     of allocations); the `pvr-pool-npot` replay item is the host side.
   - SV returned no console bytes: repeat it once to tell a boot or bridge
     flake from a real failure.

## Next board session

After the `GL_BGRA_EXT` allowlist fix, re-run i.MX8MP entry 21 (`fbo`, with
`TREE_ROWS` as in `tools/board.sh`) and expect 711 of 711.

One session per board confirms what waits for hardware ("Waiting for a
board"). Only steps not yet run are here; the steps that ran, with their
readings, are in the record file named under "State of main". Expected
values come from the host tests and the committed records named in each
line; a result that differs is a finding, not a reason to edit the
expectation. One GL client per board: check that the board is idle first.
`BOARD_BOOT=uboot-mmc` on the two Verdins.

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

The gl_test cases below passed on the AM62P at `cd738b1a` (and 196, 202 to 205
again at `9b525867`). On the Pi 4 they ran at `cd738b1a`: 202, 203 and 205
pass, 196 and 204 fail ("Known defects"); they are still to run on the
i.MX8MP, and 196 and 204 on the Pi 4 after their fixes. An ES 3.0 gl_test image needs
`CONFIG_GL_TEST_CASES` for `gl_test`; `tools/cts/common-images.sh` has no
case override, so append the line to `prj.conf` for the build and remove it
(a `-D` option on the `west build` line does the same).

On the Pi 4 and the i.MX8MP, gl_test 203 `tfcap` too (the capture is the GL
core's, the same on the three). Expected: case 203 tfcap 9/9 checks (A:
PRIMITIVES_WRITTEN 2, the six interleaved records, the guard, the window
green at (16, 16); B: 2, the strip's positions and colours in 0 1 2, 2 1 3; C:
the discarded draw's positions, the window red), `PASS`; on an ES 2.0 image
it SKIPs.

On the Pi 4 and the i.MX8MP: gl_test 196 `occl` alone
(`CONFIG_GL_TEST_CASES="196"` in the board's gl_test sample; not in case 7's
regression set until it has passed on all three). Expected: the five lines `gl_test: occl: q0 ... q4`
read `available 1` and results 0, 1, 0, 1, 1, the white quad in the centre
and the red occluder in a corner, `PASS`. q0 or q1 wrong: the counter does
not count (PowerVR: the general-heap result area or `GETVISRESULTS`; V3D:
`OCCLUSION_QUERY_COUNTER`; GC7000: ADDR/CONTROL); q3 0: the V3D's TLB does
not add across jobs; q4 0 on the PowerVR: the frame split at `glEndQuery`
is not in effect.

On the Pi 4 and the i.MX8MP, gl_test 202 `blitconv`, 204 `srgb` and 205
`mappbo` as well. Case 204's render half is the PowerVR's only
(`GL_TEST_PVR`); its sample half runs on all three. Each of these cases skips
by name in an ES 2.0 context.

- 202 `blitconv` (any shader set: the case draws with clears only). Expected
  12/12 checks (the RGB565 target's quadrants 0xff0000ff, 0xff00ff00,
  0xffff0000 and 0xff9c6531 with the source's top half at the bottom, the
  RGBA16F target's four quadrant colours as floats within 0.004, and after a
  scissored clear of the RGB565 corner the blit still beside it).
- 204 `srgb` (the online compiler's image: no table has an sRGB variant).
  Expected 8/8 checks: the SRGB8_ALPHA8 clear to linear 0.2 reads 124 124 124
  255 (two checks), ONE/ONE with 0.2 reads 170 (248 would be a blend of the
  encoded bytes), SRC_ALPHA/ONE_MINUS_SRC_ALPHA reads 203 89 160 191, a frame
  with no clear blending 0.1 reads 188 (227: the load did not decode) and
  keeps 203 89 160 191, an sRGB texel (188, 124, 0, 128) samples as 128 51 0
  128, and a black/white pair filtered LINEAR at 0.516 reads 131 (58:
  filtered before the decode).
- 205 `mappbo`: 15/15 on the AM62P; the checks are in the case.

### i.MX8MP (Verdin, no UART: records are read from the SNVS)

A board can hold a staged one-shot image (`/var/yakogl/once-*.bin`) after an
interrupted run. If a run is refused with "holds a one-shot image of a run
that did not clean up", run `board.sh disarm` (or remove the file) before
retrying; `board.sh` does not clean up such an image after a killed run.

`imx8mp-gltest-r2` can run its #9/#25 part (the KICK_DIFF arms, "Known
defects") as it is, with one condition: build the image first and read `rc=`.
The Zephyr arm of `gc_gl_test` and `gl_test.c` has never been built from this
code (`gc-kick-diff.md`, "What was checked"), so a compile error shows at the
build, not at a boot. Use the selector values in "Known defects", not the
comments in `gc-kick-diff.md`. The loop that interleaves the two arms is the
session's to write. Image recipe:
`tools/cts/imx8mp-gltest-build.sh kd-<sha8> 0 '-DCONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10\;10\;2"'`.

The gl_test builds need `-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y`, or
`tools/board.sh` refuses the image (rc=3), and a `;` inside a sweep list
must be written `\;` (`tools/cts/log/investigations/gc-case10-flake.md`,
"Board recipe").

```sh
export ZEPHYR_WORKSPACE=$ZEPHYR_WS ZEPHYR_VENV=$ZEPHYR_WS/.venv IMX8MP_OUT=<out> DEQP_STAGE1=<stage 1>
```

6. **Case 44's layout probe (#6; one boot, any time).** Case 44 alone:

   ```sh
   $SW/tools/cts/imx8mp-gltest-build.sh gc44p-$HEAD8 44 -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y
   SEL=0 $SW/tools/board.sh imx8mp run $IMX8MP_OUT/gc44p-$HEAD8/zephyr/zephyr.bin <rec>/gc44p gc44p-$HEAD8
   ```

   The record is `CASE ... case 44 check 3 status 2 got G want W` (check 3 is
   the probe on the GC7000). Decode G and W with the table in
   `src/vivante/VIVANTE_NOTES.md`, "RGBA16F targets": G[31:24] the arms that
   came out right (bit 0 RGBA16F 8 wide, 1 = 4, 2 = 16, 3 = 24, 4 = 32, 5 the
   RGBA8 control), the rest where the first wrong arm's pixels went.
   Expected: `20008106 00c2873e` (pixel x stored at x ^ 2 in every RGBA16F
   width) or `20102006 00d0073e` (a 2-pixel shift) -> render 64-bit targets
   tiled, as Mesa does; G = `34......` or `3c......` (16 and 32 wide right)
   -> a 16-texel row pitch rule for 64-bit texels. A CASE pass or a failure
   at check 4 or later means the probe's targets all came out right and the
   case's own did not. Copy the record into the notes' table and #6's row.

(Steps 1 to 5 ran: 1 and 4 on `32778b57`, 2 on `a0f439d6`, 3 and 5 on
integration images. Their recipes are in the record file.)

The common set (GM SO SV CL GT) ran at `b568940`
(`docs/board-sessions/imx8mp-common-int.md`); nothing more is required from
it. If run again, use `BOARD_BOOT=uboot-mmc` (the default is `uboot-tftp`) and
rerun GT once if 2, 10 or 16 fail; the session doc's "To rerun it" has the
commands.

The gl_test default list ran at `e0eb883d` (`docs/board-sessions/imx8mp-gltest-final.md`).
`common-images.sh` cannot pass `-pipe`, and the worker's `/tmp` quota
stopped the compile of `gl_test.c`; on a worker with enough `/tmp` the GT
image builds without the workaround. Cases 10 and 2 are flaky
(`gc-kick-diff.md`): more boots are needed before calling them fixed.

Optional, not needed to accept gc-const-array: a run of
`dEQP-GLES3.functional.shaders.large_constant_arrays.*` would confirm that
the two newly built programs draw (the host interpreter says they compute
every element: `tests/es3_gc_const_array.rs` `every_element_reads_back`).

dEQP-GLES3 (never run on the board): the 133 table rows `e00` to `e132` of
`tools/cts/imx8mp-gles3-tree.txt`, as in `docs/es3-board-recipe-imx8mp.md`
section 3, from an image rebuilt at the commit the board runs
(`tools/cts/imx8mp-gles3-artefacts.sh es3-<hash8>`).

### Raspberry Pi 4 (tryboot, no `BOARD_BOOT`)

Still to run on the Pi 4:

- one whole GLES2 tree from one main commit, which replaces the composed
  row; `TREE_PREV=records/pi4/deqp-1cf946a9/rows` compares the six cube
  rows. Expected: 17,126 pass / 0 fail / 34 NotSupported / 5 warnings (3
  QualityWarning, 2 CompatibilityWarning). A row whose verdict is
  `done_lossy` has lost result lines in the bridge: read it against the
  image's counters and rerun the row (the bridge's fix is firmware work);
- gl_test 196 `occl` after the V3D counter is looked at, and 204 `srgb` after
  its guard (above; 202, 203 and 205 passed at `cd738b1a`);
- the remaining dEQP-GLES3 tree images (`tools/cts/pi4-gles3-tree.txt`, 91
  images, `docs/es3-board-recipe-pi4.md`; rows 0 to 7 ran, row 7 is FB4,
  built and not run):
  `TREE_IMAGE_DIR=<dir> BOARD_MUSTPASS=<abs path of gles3-mustpass.txt>
  tools/board.sh pi4 tree tools/cts/pi4-gles3-tree.txt <outdir>` with
  `TREE_ROWS=<a>-<b>` (at most four rows a run, "Known defects"),
  `DEQP_MODULE=gles3` and `DEQP_WORK` set to the gles3 stage 1 for the image
  build. One row costs about 5 min 15 s (the run itself under 30 s). Run the
  five GLES2 regression rows (D, FB1, T1, T3, TF) in the same session; they
  did not run in the baseline session.

The whole tree needs `RUSTUP_TOOLCHAIN=nightly`, stage 1 for `rpi_4b`
(`DEQP_WORK`), `BOARD_UART` (the debug probe's device, or `host:port` of
its serial bridge) and `BOARD_HDMI_TIMINGS` as the lab's:

```sh
export DEQP_WORK=<stage 1 for rpi_4b> RUSTUP_TOOLCHAIN=nightly
$SW/tools/cts/pi4-deqp-images.sh $SW/tools/cts/pi4-full-tree.txt <img>   # <img> must not exist yet
TREE_IMAGE_DIR=<img> TREE_PREV=records/pi4/deqp-a27d69c0/full $SW/tools/board.sh pi4 tree $SW/tools/cts/pi4-full-tree.txt <rec>/full
$SW/tools/board.sh pi4 tally <rec>/full
```

If the cube rows fail again (T3 21, T4 22, counted from 0 over the
non-comment lines): check that the fragment variant was built
(`V3D_VARIANTS_BUILT` grows) and has `fdx`, `fdy`, `tmuslod`; a draw
refused on a bad field means the face size word found no size in the job
builder's table (`texture_units`, v3d_job.c). `docs/v3d-noseamless-cube.md`
has the rest.

glmark2 on the Pi 4 after v3d-rt-sample. The host harness
(`tools/hostharness/glmark2/run.sh <glmark2> <out> v3d`, run at this commit
on 2026-10-14, 3 frames per scene, 33 ok) shows the image path for
one-level targets only: desktop's 800x600 and terrain's 256x256 targets
print "is sampled from its render image (...): no tiled copy". The Linux OSAL
has no TFU (`src/osal/linux.c`, "What is not"), so there refract's and
terrain's glGenerateMipmap stay on the CPU, the CPU filter moves the stamp,
and refract's tiled-copy high-water stays at 19,742,720 bytes (its
7,684,096-byte depth copy and the colour chain copied again): the TFU chain
is tested on the board only. The "render images: ..." shutdown line is not
printed in either place, because glmark2 ends without tearing the context
down. On the board, the glmark2 image's console must show "the TFU made the
11-level chain of a 1600x1200 render image" in refract (the 10-level
800x600 one in terrain), no "tiled copy" line naming a colour render
target, and 33 / 0 / 0. Terrain's remaining CPU cost is still unexplained:
on the host its tiled-copy high-water climbs to 12,433,408 bytes before its
first frame ends, and which textures those copies are (uploads, or targets
sampled before their first render) is not read from the console.

### AM62P (`BOARD_BOOT=uboot-mmc`; `BOARD_UART` set; `driver/src/fw/pvr_fw_blob.h` in the checkout)

The console must be the main_uart0 bridge port: a session leased the
mcu_uart0 port and captured nothing
(`tools/cts/log/investigations/am62p-console-empty.md`). On 2026-10-14 the
port the lease and the task's `BOARD_UART` named was not it either (both gave
no bytes). Probe the port with
`python3 tools/am62p_netboot_uart.py probe <host:port>` and check that the
console shows bytes before the first image.

`board.sh am62p tree` refuses (rc=3) when a dead run left a one-shot image in
the eMMC staging directory (a stale `/var/yakogl/once-*.bin`): `board.sh am62p
disarm` clears it. A bridge capture timeout (`verdict=scan_failed`, rc 4) is a
transport problem, not a case failure: rerun the row. An image build with the
image directory under `/tmp` failed once with rc=101 (cause unknown, not
reproduced with the directory in the dependency cache).

After the `layout(location)` change (`src/gl/program.rs`), re-run
`functional.attribute_location.mixed` and `mixed_time` on the AM62P (expect 0
failures), and `mixed_max_attributes` after the `reflect.rs` change ("Open
work" item 4, es3-vertex). After a back-end fix for `gl_InstanceID` in an
emulated instance (P1/P3, V0/V1), run the `dEQP-GLES3.functional.instanced.*`
group (45 cases) on the AM62P and the i.MX8MP; the commands and the expected
verdict table are in section 5 of
`tools/cts/log/investigations/instanceid-per-draw.md`. The six
`{draw_arrays_instanced,draw_elements_instanced}.mixed.{2,4,20}_instances`
cases must pass, and the 39 others must stay passing; the Pi 4 should pass
already. Watch the `attrib_divisor` cases after P3 or V1, because they then
use the hardware divide. If they fail on the i.MX8MP after V1, the GC7000UL
FE ignores divisors > 1: set `max_instance_divisor` back to 1 and use V2 (a
per-draw uniform).

After the vertex array conversion (`src/gl/draw.rs`), re-run
`dEQP-GLES3.functional.vertex_arrays.single_attribute.*` on the AM62P (expect
the 41 failures gone; the other 920 pass as before).

Common set (ran at `cd738b1a`, `docs/board-sessions/am62p-common-int.md`):
re-run SV alone (it was empty). After the render pool fix ("Open work" item
9), re-run GM and watch the console for `render pool exhausted` and `out of
GPU memory`; the tally's "GL errors raised" reads `15 or more` and does not
name the scene. The console is the bridge's main_uart0 port, not the lease's
mcu_uart0 port.

dEQP-GLES2: image 9 (`TREE_ROWS=8`) confirms the 45 `texture.vertex.2d.*`
cases (`a6fc6f14`); then the whole tree and the ms4 image replace the
`32778b57` row.

```sh
export DEQP_WORK=<stage 1>
$SW/tools/cts/am62p-deqp-images.sh $SW/tools/cts/am62p-full-tree.txt <img> $HEAD8 > <img>/build.out       # 17 images
$SW/tools/cts/am62p-deqp-images.sh $SW/tools/cts/am62p-ms4-tree.txt <img-ms4> ms4 > <img-ms4>/build.out   # 1 image
TREE_IMAGE_DIR=<img> TREE_TAG=$HEAD8 $SW/tools/board.sh am62p tree $SW/tools/cts/am62p-full-tree.txt <rec>/full
TREE_IMAGE_DIR=<img-ms4> TREE_TAG=ms4 $SW/tools/board.sh am62p tree $SW/tools/cts/am62p-ms4-tree.txt <rec>/ms4
$SW/tools/board.sh am62p tally <rec>/full
$SW/tools/board.sh am62p tally <rec>/ms4
$SW/tools/cts/deqp-uart-tally.py --mustpass $SW/tools/cts/gles2-mustpass.txt <console logs of both trees>
```

Expected: combined mustpass 0 fail, the `fail` column of all 17 image rows
0 and of `ms4` 0, no case worse than at `d60b9d76` (17,141 pass / 0 fail /
19 NotSupported / 5 warnings of 17,165; RESULTS 1z2) except
`fragdata.invalid_assign_to_1`, which passes since `4e9397c`. A case that
moves from Pass to anything else is a regression: bisect it first. Time:
at most about 11 min an image, 18 images.

dEQP-GLES3 (the tree ran on `ad2b5ed`; "State of main"). Next, by size:

- the 11 slow `texture.filtering.3d.*_linear_mipmap_linear` mustpass cases,
  one image each (up to about 4.5 min each);
- the "hang" of images 20 and 21 (Known defects, most likely dEQP's slow
  check): before booting anything, read the `f2a13877` consoles of images
  20 and 21 (`records/am62p/deqp-gles3-f2a13877/full/20261011-0232-img20-*`,
  `full2/20261011-0251-img21-*`): the time of every `Test case` line and
  any `render: HWR during fence` or `pvr_backend: kick N did not complete`
  line. Gaps of about 270 to 315 s at cases 2 and 8 (the earlier
  `linear_mipmap_linear` cases), and no HWR or timeout line, confirm H1
  without a board run. If a board run is still needed: three one-case
  images (`3x7x5_linear_mipmap_linear`,
  `r11f_g11f_b10f_linear_mipmap_linear`, control `3x7x5_linear`) with
  `RUN_BOUND=900 RETURN_BOUND=240`; the commands and the expected output
  per hypothesis are in section 7 of
  `tools/cts/log/investigations/pvr-3d-mipmap-hang.md`;
- image 7's tail (Known defects): re-run image 7 (`TREE_ROWS=6`) after the
  non-indexed draw is cut into pieces;
- the failing groups, by size: texture 1,752 (and 1,646 NotSupported),
  fragment_ops 1,109, fbo 872, transform_feedback 640 (191 NotSupported),
  fragment_out 595, shaders 412. The capture, blit and sRGB rows below are
  the first claim on those; the classes of the rest are in
  `docs/es3-cts-failures.md`.

(Image 5's tail and image 25's capture loss ran clean on `cd738b1a`.)

Transform feedback, blit, occlusion and the gl_test cases 196 and 200 to 205
ran on `cd738b1a` (above); the dEQP rows `TREE_ROWS=4,24` (`lifetime`,
`transform_feedback`) and `1,2,4` (`fbo.blit`, `fbo.color`, `fbo.invalidate`,
`fragment_ops.scissor`) ran too. What did not pass is in "Known defects"
("AM62P fbo / blit / invalidate failures"); a blit case that fails with
"Image comparison failed" is a finding for `src/gl/blit.rs`.

ES 3.0 formats and integer render targets (`25267d17`, `fc658e64`, merged in
`a27d69c0`): gl_test 200 and 201 pass (above). The dEQP-GLES3
`texture.specification`, `fragment_out` and `shaders.builtin_functions`
buckets (736 + 522 + 531 + 1,461 cases) have not run since `f2a13877`: run
them against their last numbers (the online compiler's image; a table-only
image refuses the integer variants by name); `fragment_out.array.*`/`random.*`
with an integer attachment beside another stay refused by name.

ES 3.0 sRGB (`docs/es3-srgb.md`): gl_test 204 passes (above). The dEQP-GLES3
rows of `tools/cts/am62p-gles3-tree.txt` that hold sRGB cases have not run
beyond rows 1, 2 and 4: run `TREE_ROWS=1-4,18,19,21,22` minus those, against
the `c4ccc966` records (`records/am62p/deqp-gles3-c4ccc966`):

```sh
DEQP_MODULE=gles3 DEQP_WORK=<gles3 stage 1> \
  $SW/tools/cts/am62p-deqp-images.sh $SW/tools/cts/am62p-gles3-tree.txt <img3> $HEAD8 > <img3>/build.out
TREE_IMAGE_DIR=<img3> TREE_TAG=$HEAD8 TREE_ROWS=1-4,18,19,21,22 \
  $SW/tools/board.sh am62p tree $SW/tools/cts/am62p-gles3-tree.txt <rec>/gles3-srgb
$SW/tools/cts/deqp-uart-tally.py --mustpass $SW/tools/cts/gles3-mustpass.txt --statuses <rec>/gles3-srgb/*/console.txt | grep -i srgb
```

Row 3 is `fragment_ops.blend.fbo_srgb` (1,106 of 1,152 failed): expected
all 1,152 to pass. Rows 1, 2 (`fbo.color`, `fbo.completeness`, `fbo.render`),
4 (`fragment_out`, `pbo`), 18 (`texture.filtering.2d`/`2d_array`), 19, 21
(`texture.filtering.cube`, `texture.format`) and 22 (`texture.specification`):
the srgb8 / srgb8_alpha8 cases of the table at the top of
`docs/es3-srgb.md` pass, except the 12 `fragment_out.array.*srgb8_alpha8*`
(refused by name: an sRGB attachment beside another); the 32
`fbo.blit.conversion.*srgb8_alpha8*` cases and the sRGB
`fbo.color.repeated_clear.blit` ones pass through `src/gl/blit.rs`'s sRGB
decode and encode (`fbo.blit.conversion.*32f*` sRGB pairs and
`fbo.msaa.*srgb8_alpha8` stay NotSupported / as the multisample blit is).
Every other case of those rows
as at `c4ccc966` (a Pass that moves is a regression). A filtering case that
fails by a step or two in dark texels is the TPU's decode precision, a
finding for `docs/es3-srgb.md` 4.3.

The final dEQP-GLES2 sessions: compare the counters with
`tools/cts/log/board-sessions/es2-final-expected.md` (the Begun column is the
basis of the board's own counter; the mustpass-list tally is the Totals
table). A row that differs from "Expected final" is news.
`flush_finish.{flush, flush_wait}` may read CompatibilityWarning (0 to 2 per
board) and is not a difference. A capture with Lost cases is a rerun.

When the sessions are done, replace each board's row in
`docs/es2-cts-failures.md` (the summary table and section A),
`tools/cts/STATUS.md`, `tools/cts/RESULTS.md` and "State of main" above with
the new numbers and record names.

## How to work here

- The records prove the framebuffer, not the panel: a display change needs
  someone looking at the panel (#23).
- main moves only through pull requests: a branch, a pull request, the CI,
  a review, then the merge. The CI is the gate: `.github/workflows/gate.yml`
  runs `tools/check.sh` on every pull request to main, on every push to
  main and nightly, on self-hosted runners that carry the environment
  below; the file's header says which rows each event runs. The review
  reads the commit; it runs no check.
- A result's hand-over goes in `docs/handoff/<YYYYMMDD>-<topic>.md`, never
  in this file: two results landing in the same days always changed the
  same lines here, so every second pull request conflicted with main and
  was redone (`docs/handoff/README.md` has the note's shape). The
  `handoff-fold` stage folds the notes in after the merge;
  `tools/handoff-notes.sh` lists what waits.
- Locally, run only the rows the change reaches:
  `tools/check.sh --changed <base>` maps each changed path to the parts it
  can break (a GPU back end, gl_test, the tools, the precompiled tables,
  the drivers crate, the samples) and skips the other rows, printing
  `skip` for each. A shared layer (`src/gl`, `src/shaderc`, the OSALs,
  `capi`, `Cargo.toml`, the script itself) or a path the map does not know
  selects every row; `tools/check_scope_selftest.sh` pins the map.
  `tools/check.sh --local <base>` is the same, except that a diff reaching
  everything runs the local set only: fmt, clippy and the suite in the
  three GPUs' compiler configurations, the gl_test.c arms and the
  self-test. Of the gate's 138 rows (2026-10-11, `YAKOGL_CHECK_PLAN=1`),
  the local set is 13, a V3D-only change selects 36 and a docs change 2.
  The full run takes about 45 minutes. `tools/land.sh` runs the local set
  (`LAND_GATE=changed` for every row the diff reaches, `LAND_GATE=full` for
  every row) with a cargo target directory shared between landings.
- For the full run by hand: a clean detached checkout of the exact commit,
  with `driver/src/fw/pvr_fw_blob.h` copied in, and with
  `ZEPHYR_WS=<west workspace>`, `HAL_NXP_DIR=<hal_nxp>` and
  `GLMARK2_SRC=<glmark2 checkout at 22c527cb>` set. Without them the Zephyr,
  hal_nxp and glmark2 rows are gaps and the gate is red. The glmark2 shader
  corpus is not committed: `tools/hostharness/glmark2/run.sh` generates it
  from `GLMARK2_SRC` into `build/glmark2-host/shaders`, and
  `tests/glmark2_corpus.rs` reads `GLMARK2_CORPUS` (default that path) on
  the three GPUs. `tools/setup-gate-host.sh` prepares a gate host, and
  `tools/cts/deps-env.sh` names the dependency layout the CI reads. The
  Mesa-table reproduction row also needs `PCO_SHC=<pco_shc from
  tools/pco_shc/build_pco_shc.sh, content id f537fc86b41f>`,
  `GLSLANG=<glslangValidator>` and, for speed, `GLSL2USC_CACHE=<dir>`;
  without them that row is a gap and the gate is red. The offline PowerVR
  table (`tests/reference/blobs.rs`) records the toolchain that reproduces
  it byte for byte; the history of its regenerations is in the record file
  named under "State of main".
- Board runs use `tools/board.sh <board> run <zephyr.bin> <record dir>
  <label>`. Run them from a detached checkout of main, and only with images
  built from commits on origin/main. Use `BOARD_BOOT=uboot-mmc` on the
  Verdins.
- A serial bridge (the Pi 4's TCP bridge, the AM62P's) can stall and drop a
  burst of console lines. A dEQP run whose image counters (`deqp: summary`)
  say more cases had a result than the console shows has the verdict
  `done_lossy` (`lost=<n>`, rc 4), and `board.sh <board> tally` and
  `deqp-uart-tally.py` name the lost ranges and call those cases Lost, never
  NoResult (a case the image never began). Rerun the row until the capture is
  whole; do not count a `done_lossy` row as a result (tools/cts/README.md,
  section 6). The bridge's fix is firmware work.
- A dEQP image bakes its table in: splitting rows needs a rebuilt image
  (`tools/cts/imx8mp-deqp-build.sh --table`; tools/cts/README.md).
- One GL client per board. Check that a board is idle before you use it.
