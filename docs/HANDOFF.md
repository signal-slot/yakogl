<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Handoff

The state of the work and what comes next. Update this file in the same
commit as any result that changes it. Last update: 2026-10-11.

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

main is `5ad2fa0e` (2026-10-11). The numbers below are read from
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
| AM62P | whole tree on `32778b57` (2026-10-10): 17,070 pass / 54 fail / 34 NotSupported / 5 QualityWarning / 2 CompatibilityWarning. Of the 54: the 9 `shaders.invariance` cases pass on the board since `e17fb6a7` (image 1, 1,379 / 0); the 45 `texture.vertex.2d.*` cases (image 9) are fixed on the host (`a6fc6f14`) and wait for the board. The ms4 image: 14 / 2 on `32778b57`; 16 / 16 on `452d6c31` and `a27d69c0` after the PDS fix (`a4f64950`; no RESULTS.md entry: the commit message of `a80b297f5` and the HANDOFF text of `bd8df7a49`). | 49 / 49, 0 resets, on `c4ccc966` (197 `vtxgrid` in the set; no RESULTS.md entry); 48 / 48 on `32778b57` (RESULTS.md). Cases 198 to 201 pass on `a27d69c0`, 0 resets (no RESULTS.md entry: the HANDOFF text of `bd8df7a49`); 198 hung the fragment DM on `452d6c31` before `a80b297f5`. | 465, 33 scenes ran, 0 failed, 0 resets, on `f2a13877` (2026-10-11): pvr-depth-resolve confirmed. | Slint pass, cluster 91.8 fps (56.4 Mpx/s, 16.28 ms/Mpx) on `7dada1ff` (RESULTS.md, 2026-10-05). |
| i.MX8MP | whole tree on `32778b57` (2026-10-10): 17,318 pass / 0 fail / 34 NotSupported / 19 QualityWarning of the 17,371 begun; every selector equals `d8583f20`'s. | 47 / 48 run, 1 skipped (case 16, #8), first failure case 44 (#6), on `a0f439d6` (2026-10-10; no RESULTS.md entry). Case 44 alone still fails on `f2a13877` (RESULTS.md). Cases 2 and 10 fail now and then (#25, #9). | 42, 33 ran, 0 failed, on `8abeaabb` (`tools/cts/log/benchmarks-baseline/imx8mp.md`); no console, so no per-scene score. | probes pass on `8abeaabb`; no fps (no console). |
| Pi 4 | 17,126 pass / 0 fail / 34 NotSupported / 5 warnings, composed of three records: the tree on `a27d69c0` (17,083 / 9 / 34, 34 lost in capture), the six cube rows on `1cf946a9` (934 / 934) and the F1 rerun (2026-10-10 and 10-11). One tree from one commit has not run. | 12 / 12 on `1cf946a9` (2026-10-11). | 183, 33 ran, 0 failed, on `b7170d35` (2026-10-04, a commit of the history before `afb16335`). | Slint probes pass on `1fa941e3` (2026-10-04, same history); the cluster's last row in RESULTS.md is "Batch 106" (`a8b11720`). |

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
- gl_test 196 `occl` on every board; 202 `blitconv`, 203 `tfcap`, 204
  `srgb` and 205 `mappbo` (the `work/land9` cases, on main since
  `c4dce9125`) on every board; 199 to 201 on the i.MX8MP and the Pi 4 where
  the GPU runs them.
- i.MX8MP case 44's layout probe (#6, step 6 of the i.MX8MP section).
- i.MX8MP #8 (4x MSAA) and #9 / #25 (cases 10 and 2): the next instruments
  are in "Known defects".
- AM62P dEQP-GLES3 rows that `c4dce9125` changes: transform feedback,
  blit, buffer and PBO, sRGB.
- AM62P dEQP-GLES3: the GPU hang of images 20 and 21, and image 7's
  `ResourceError` (Known defects).
- The Pi 4 glmark2 refract and terrain (#4, #5) on the 96 MiB carve-out.
- The i.MX8MP panel by eye (#23).

### ES 3.0 (goal 4)

The AM62P dEQP-GLES3 tree, all 28 images on `f2a13877` (2026-10-11): of
42,810, **32,669 pass / 5,549 fail / 2,318 NotSupported / 185
QualityWarning / 2,089 without a result** (`c4ccc966`, 2026-10-10:
30,431 / 7,011 / 2,274 / 5 / 3,088). The 2,089: image 5's tail 558 (it
ran before transform-feedback capture landed), image 7's tail 1,397 (a
`ResourceError`), the two hanging images' tails 83, and 51 lost in capture.
Fail by group and the 2,089 by group: `tools/cts/RESULTS.md`, "AM62P: the
whole dEQP-GLES3 tree on `f2a13877`". The Pi 4 GLES3 tree
(`tools/cts/pi4-gles3-tree.txt`, 91 images; recipe
`docs/es3-board-recipe-pi4.md`) and the i.MX8MP's have never run. Every
other ES 3.0 number is HOST-ONLY: the Linux OSAL runs no command stream, so
only API errors and compile and link outcomes mean anything there
(`tools/cts/README.md` section 2b). Under `report-es2` every ES 2.0 answer
is kept and pinned by tests.

| Feature | State | Design |
|---|---|---|
| Transform feedback | built on the host for the three GPUs: the GL core runs the vertex shader on the CPU for every captured vertex (`c4dce9125`; API `16ac864`). Host dEQP-GLES3 `transform_feedback` 882 / 882, `lifetime` 56 / 56. | `docs/es3-transform-feedback.md` |
| Uniform blocks | host: `tests/es3_ubo.rs` ports dEQP's `functional.ubo` (2,032 cases); the GC7000 refuses four at link (`KNOWN_LIMITS`) (`1b0e431`, `b0c0fa6`) | none |
| Sized internal formats | the 35 colour formats of ES 3.0 table 3.2, converted at upload; the PowerVR samples every class but `RGB10A2`; the GC7000 and V3D refuse the integer ones by name (`888af5d`, `e5c4a18`; PowerVR `25267d17`, merged in `a27d69c0`) | `docs/es3-sized-formats.md`, `docs/es3-pvr-formats.md` |
| Occlusion queries | counted on the three back ends, host only (`2241cd84` GL core, `d9573ab9` PowerVR, `dcac96cb` GC7000, `7526b084` V3D; merged in `452d6c31`) | `docs/es3-occlusion-query.md` |
| Integer colour render targets | GL half built; the PowerVR writes all nineteen as a draw's one colour attachment (`fc658e64`, merged in `a27d69c0`); the GC7000 and V3D refuse by name | `docs/es3-pvr-formats.md` section 10 |
| `glBlitFramebuffer` | colour on the CPU, shared by the three back ends (`c4dce9125`); depth and stencil refused by name (blit-ds) | `src/gl/blit.rs` |
| sRGB | PowerVR renders and samples; the GC7000 and V3D sample and refuse a draw into sRGB by name (`c4dce9125`) | `docs/es3-srgb.md` |
| Buffer mapping and pixel buffers | host-proven (`c4dce9125`) | none |
| NPOT levels of arrays and storage textures | valid at every level (`c40414b`) | none |
| Stage-interface link rules | interpolation and block mismatches refused (`b88fea5`, `03ff7a6`) | none |
| GLSL ES 3.00 in the compiler | measured over dEQP's 5,596 programs (`4ab482e`, `79fe171`, `ed0c81d`) | `docs/es3-shaderc-coverage.md` |
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
   - Open: the GC7000 has no table row for most gl_test sources (issue
     #22, in "Known defects"); a `gc7000` role for `dir gltest` needs
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
   - **GLES3 hang** (no issue yet): two dEQP-GLES3 cases hang the AM62P's
     GPU, and the board comes back only with the RTI reset:
     `texture.filtering.3d.formats.r11f_g11f_b10f_linear_mipmap_linear`
     (image 20) and `texture.filtering.3d.sizes.3x7x5_linear_mipmap_linear`
     (image 21), each the 14th case of its image (`f2a13877`, 2026-10-11).
     Both are 3D textures filtered `linear_mipmap_linear`. Cause not found:
     nothing has decoded the hang yet (the HWR record, as for gl_test 198).
     Until it is fixed the two images' tails (83 cases) have no result.
     Image 11 needs 4 resets (`done_with_resets`) to complete.
   - **AM62P GLES3 image 7** (no issue yet): a 24,384-vertex `glDrawArrays`
     in `shaders.builtin_functions.precision.clamp.highp_fragment.vec3`
     does not fit the 1 MiB general arena, and dEQP stops at the
     `ResourceError` (1,397 cases never run). A non-indexed draw needs
     cutting into pieces as `draw_indexed_pieces` cuts an indexed one.
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
     still fails on `f2a13877` at check 3 (texel (5,0)'s colour stored at
     (7,0) of an 8-wide linear target). `466e1b00`'s two fixes did not move
     it. The ranked hypotheses: `src/vivante/VIVANTE_NOTES.md`, "RGBA16F
     targets". The layout probe (case 44's check 3 on the GC7000,
     `hdr_layout_probe`, decode pinned by `tests/gltest_hdr_probe.c`) is
     the next board run (i.MX8MP step 6).
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
     board").
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
   - The GC7000 Mesa table (`tests/reference/vivante_blobs.rs`) lags the
     gl_test source list (#22): the four `matattr` keys are current, but
     `tools/vivante/check_gl_test_blobs.py` exits 1 with 146 of gl_test's
     192 shaders without a row (325 rows, 2026-10-11). The script is not in
     `tools/check.sh`. It matters only for images that use the table; the
     dEQP images use the compiler.
   - glmark2 terrain black (#1) and the AM62P refract depth pass (#2) are
     fixed on the host (NPOT chains, `2673197`, `77693ff`, `cc78859`; the
     pool's top region, `541c9d3`); on the AM62P at `f2a13877` all 33
     scenes ran, but no capture shows the pictures. The Pi 4 carve-out (#4,
     #5): the Pi 4 glmark2 image takes 96 MiB (`ca4074a`); the host harness
     (`tools/hostharness/glmark2/FINDINGS-v3d.md`, 2026-10-12) shows no
     refused draw and no refused allocation since the image's tiled-copy
     budget is 24 MiB (refract's 1600x1200 mip chain was over the 8 MiB
     default: `tools/cts/log/hostharness/glmark2/FINDINGS-refract-budget.md`);
     not yet run on the board.
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
4. **ES 3.0, host first.** Goal 4. A proposed order, not a ranking anyone
   measured. Each item keeps every `report-es2` answer and adds a test that
   pins it. What each item built is in the ES 3.0 table above; here is what
   remains. The full wording as of 2026-10-11 is in the record file.
   - **tf-capture**: the board (gl_test 203 `tfcap` on each board, then the
     AM62P GLES3 rows with `transform_feedback` and `lifetime`). Left: a
     vertex shader that samples a texture refuses its capturing draw; the
     V3D's hardware path (`docs/es3-transform-feedback.md` section 4.3)
     only if an application needs the speed.
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
   - **occlusion-query**: the board (gl_test 196 on each board, then
     `functional.occlusion_query` (62) and the 4 `negative_api` cases on the
     AM62P). The unverified points are `docs/es3-occlusion-query.md`
     section 9.
   - **pvr-2darray**: PowerVR 2D-array and shadow sampling, designed in
     `docs/es3-pvr-2darray.md`.
   - **buffer-pbo**: the board next (gl_test 205 `mappbo`, then the AM62P
     GLES3 tree's images 1 `buffer.*`, 5 `pbo.*`, 6 `read_pixels.*` and 23
     `texture.specification.*`). Left: the 3D and 2D-array cases of those
     groups need `pvr-2darray`, and the depth ones the PowerVR's depth
     sampling.
   - **es3-shaderc**: the online compiler's GLSL ES 3.00 gaps
     (`docs/es3-shaderc-coverage.md`: PowerVR 3,453 / GC7000UL 3,451 / V3D
     3,453 of 3,461 expected-build programs build). Open: the rest of its
     "Work that closes the table"; two need a decision first: names of 63
     characters or more (an instance block's default-block rows are named
     after the instance, so a 1,024-character instance name fills the
     64-byte field), and the GC7000UL's 128-element constant array (it
     wants the constant file with relative addressing, as etnaviv takes it).
   - **blit-ds**: the depth/stencil half of `glBlitFramebuffer` (the 16
     AM62P `fbo.blit.depth_stencil` failures). It needs a back-end entry
     that copies a rectangle between two objects' planes
     (`src/ds_retention.rs`) with the blit's mapping, NEAREST only.
   - **es3-caps**: redo the ES 3.0 limits test, reverted for review
     (`3f78982`).
   - **es2-report-gaps**: the four `BUG` pins of `tests/es2_report_guard.rs`
     (ES 3.0-only pnames and capabilities answered, four ES 3.0
     renderbuffer formats accepted, ES 3.0-only pixel types
     GL_INVALID_OPERATION instead of GL_INVALID_ENUM under `report-es2`).
   - **es3-vertex**: the AM62P vertex groups, `attribute_location` 48,
     `vertex_arrays` 41, `instanced` 6 fail on `f2a13877`; the task list
     and causes are in `docs/es3-cts-failures.md`.
   - The GC7000 refuses four `ubo` cases at link for constant registers
     (`tests/es3_ubo.rs` `KNOWN_LIMITS`).
   - Then the dEQP-GLES3 tree on each board, after its ES 2.0 tree.

## Next board session

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

On every board, gl_test 203 `tfcap` too (the capture is the GL core's, the
same on the three; expected as in the AM62P section below).

On every board: gl_test 196 `occl` alone (`CONFIG_GL_TEST_CASES="196"` in
the board's gl_test sample; not in case 7's regression set until it has
passed on all three). Expected: the five lines `gl_test: occl: q0 ... q4`
read `available 1` and results 0, 1, 0, 1, 1, the white quad in the centre
and the red occluder in a corner, `PASS`. q0 or q1 wrong: the counter does
not count (PowerVR: the general-heap result area or `GETVISRESULTS`; V3D:
`OCCLUSION_QUERY_COUNTER`; GC7000: ADDR/CONTROL); q3 0: the V3D's TLB does
not add across jobs; q4 0 on the PowerVR: the frame split at `glEndQuery`
is not in effect.

On every board, gl_test 202 `blitconv`, 204 `srgb` and 205 `mappbo` as
well (expected as in the AM62P section below). Case 204's render half is
the PowerVR's only (`GL_TEST_PVR`); its sample half runs on all three. Each
of these cases skips by name in an ES 2.0 context.

### i.MX8MP (Verdin, no UART: records are read from the SNVS)

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

### Raspberry Pi 4 (tryboot, no `BOARD_BOOT`)

Still to run on the Pi 4:

- one whole GLES2 tree from one main commit, which replaces the composed
  row; `TREE_PREV=records/pi4/deqp-1cf946a9/rows` compares the six cube
  rows. Expected: 17,126 pass / 0 fail / 34 NotSupported / 5 warnings (3
  QualityWarning, 2 CompatibilityWarning). A row whose verdict is
  `done_lossy` has lost result lines in the bridge: read it against the
  image's counters and rerun the row (the bridge's fix is firmware work);
- the gl_test cases of `c4dce9125` (`v3d_gl_test` sets: 203 `tfcap`, 202
  `blitconv`) and 196 `occl` (above);
- the dEQP-GLES3 tree (`tools/cts/pi4-gles3-tree.txt`, 91 images; never
  run, `docs/es3-board-recipe-pi4.md`), after the transform-feedback and
  blit rows pass on the Pi 4.

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

### AM62P (`BOARD_BOOT=uboot-mmc`; `BOARD_UART` set; `driver/src/fw/pvr_fw_blob.h` in the checkout)

The console must be the main_uart0 bridge port: a session leased the
mcu_uart0 port and captured nothing
(`tools/cts/log/investigations/am62p-console-empty.md`).

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

dEQP-GLES3 (the tree ran on `f2a13877`; "State of main"). Next, by size:

- the GPU hang of images 20 and 21 (Known defects): decode the hang first
  (the HWR record and trace through `driver/tools/sfdecode`, as for gl_test
  198), since the board needs the RTI reset to return and each run costs
  about ten minutes; a gl_test case that draws one 3D
  `linear_mipmap_linear` texture would be the smaller reproduction (not
  written). Run those images with `RUN_BOUND=720 RETURN_BOUND=240`;
- image 7's tail (Known defects): re-run image 7 (`TREE_ROWS=6`) after the
  non-indexed draw is cut into pieces;
- image 5's tail (558 cases never ran): it aborted after
  `lifetime.attach.deleted_output.buffer_transform_feedback`, where the
  capture was missing. The capture is on main (`c4dce9125`): re-run it
  (`TREE_ROWS=4`, below);
- the failing groups, by size: texture 1,752 (and 1,646 NotSupported),
  fragment_ops 1,109, fbo 872, transform_feedback 640 (191 NotSupported),
  fragment_out 595, shaders 412. The capture, blit and sRGB rows below are
  the first claim on those; the classes of the rest are in
  `docs/es3-cts-failures.md`;
- capture loss: image 25 lost 51 result lines (`done`, not `done_lossy`, on
  the runner of `f2a13877`). Re-run image 25 (`TREE_ROWS=24`) with the
  current runner so a lossy run says so.

Transform feedback (tf-capture; host only so far): a gl_test image with
`GL_TEST_CASES="203"`. Expected: case 203 tfcap 9/9 checks (A: PRIMITIVES_WRITTEN
2, the six interleaved records, the guard, the window green at (16, 16); B:
2, the strip's positions and colours in 0 1 2, 2 1 3; C: the discarded
draw's positions, the window red), `PASS`; on an ES 2.0 image it SKIPs.
Then the dEQP-GLES3 images of `tools/cts/am62p-gles3-tree.txt` lines 12
(`lifetime`, 56, with `negative_api` and the rest of its image) and 32
(`transform_feedback`, 882, with its `ubo` groups): `TREE_ROWS=4,24`
(counted from 0 over the non-comment lines), against the host's 882 + 56; a buffer mismatch there and not on the host is the board's
memory path (the capture writes through the CPU mapping and flushes).

ES 3.0 formats and integer render targets (`25267d17`, `fc658e64`, merged in
`a27d69c0`): gl_test 200 and 201 pass on the AM62P on `a27d69c0` (no
RESULTS.md entry), and the dEQP buckets below ran in the `f2a13877` tree,
whose numbers are now theirs. For a re-run: a gl_test image with
`GL_TEST_CASES="200,201"` (the
online compiler's image; a table-only image refuses the integer variants by
name). Expected: case 200 intrt 16/16 checks (RGBA8UI and RGBA32UI: the
cleared integers, the drawn uvec4 with the column in alpha, a scissored draw
after a clear, a draw with no clear over the reloaded pixels), case 201
usamp 8/8 (RGBA8UI through usampler2D, RGBA8I sign-extended through
isampler2D). Then the dEQP-GLES3 `texture.specification`, `fragment_out`
and `shaders.builtin_functions` buckets (736 + 522 + 531 + 1,461 cases)
against their last numbers: `fragment_out.array.*`/`random.*` with an
integer attachment beside another stay refused by name.

`glBlitFramebuffer` (`c4dce9125`; host only so far): a gl_test image with
`GL_TEST_CASES="202"` (any shader set: the case draws with clears only).
Expected: case 202 blitconv 12/12 checks (the RGB565 target's quadrants
0xff0000ff, 0xff00ff00, 0xffff0000 and 0xff9c6531 with the source's top
half at the bottom, the RGBA16F target's four quadrant colours as floats
within 0.004, and after a scissored clear of the RGB565 corner the blit
still beside it). Then the dEQP-GLES3 tree's rows 1, 2 and 4 of
`tools/cts/am62p-gles3-tree.txt` (`TREE_ROWS=1,2,4`: `fbo.blit`,
`fbo.color`, `fbo.invalidate`, `fragment_ops.scissor`) against `c4ccc966`
(`records/am62p/deqp-gles3-c4ccc966`): `fbo.blit.conversion` 324 fail,
`default_framebuffer` 84, `rect` 40, `repeated_clear.blit` 64,
`invalidate.{sub,whole}.unbind_blit_color*` 8 and
`fragment_ops.scissor.framebuffer_blit_*` 3 should pass but for the integer `tex2d`
names of `repeated_clear.blit` (no integer texture targets; the 32
`srgb8_alpha8` conversions are the sRGB item's, below); the 16
`fbo.blit.depth_stencil` failures stay (blit-ds). A blit case that fails
with "Image comparison failed" is a finding for `src/gl/blit.rs`.

ES 3.0 sRGB (`docs/es3-srgb.md`; host only so far): a gl_test image with
`GL_TEST_CASES="204"` (the online compiler's image: no table has an sRGB
variant). Expected: case 204 srgb 8/8 checks -- the SRGB8_ALPHA8 clear to
linear 0.2 reads 124 124 124 255 (two checks), ONE/ONE with 0.2 reads 170
(248 would be a blend of the encoded bytes), SRC_ALPHA/ONE_MINUS_SRC_ALPHA
reads 203 89 160 191, a frame with no clear blending 0.1 reads 188 (227: the
load did not decode) and keeps 203 89 160 191, an sRGB texel (188, 124, 0,
128) samples as 128 51 0 128, and a black/white pair filtered LINEAR at
0.516 reads 131 (58: filtered before the decode). Then the dEQP-GLES3 rows
of `tools/cts/am62p-gles3-tree.txt` that hold sRGB cases, against the
`c4ccc966` records (`records/am62p/deqp-gles3-c4ccc966`):

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
