<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Handoff

The state of the work and what comes next. A result does not edit this
file: it adds a note under `docs/handoff/` (one file per result,
`docs/handoff/README.md`), and ccfarm's `handoff-fold` stage folds the notes
in after the merge; a person folds the same way. Last update: 2026-10-17.

This file holds the current state only. What happened before is in git
history; records removed on 2026-10-16 are read with
`git show ab43a20c5:<path>`.

Board addresses, UART paths and credentials never go in this file; they come
from the environment (`BOARD_HOST`, `BOARD_UART`, ...).

## Goal, in order

The five stages of `ROADMAP.md` (the public form of this list, with each
stage's criterion and state); keep the two in step.

1. ES 2.0 conformance on the three boards (AM62P PowerVR, i.MX8MP GC7000,
   Raspberry Pi 4 V3D): one whole dEQP-GLES2 mustpass tree (17,165 cases)
   per board from one main commit with 0 Fail, each board booted from its
   own storage by its bootloader. **Met** at `e9901869a` (#870).
2. The benchmarks right on the three boards: glmark2-es2's 33 scenes with
   checked pictures, the Slint sample and the AGL cluster demo with the same
   picture on every board, a frame rate recorded for each (minimum
   performance).
3. Public release: every line of `docs/release/checklist.md` met, including
   the website (`website/`). It goes live when the repository is public:
   `.github/workflows/pages.yml` deploys it once Pages is turned on
   (`website/README.md`). Every figure it shows is in `website/data.js` with
   its commit, date and source; update that file with each new board result.
   `tools/website-check.sh` checks links, metadata and `data.js`.
4. ES 3.0 on the three boards (the dEQP-GLES3 mustpass, 42,810 cases). Host
   work runs ahead of the boards and takes a board only when stages 1 to 3
   do not need it. A product image still reports ES 2.0 by default (Kconfig
   `YAKOGL_REPORT_ES2` default `y`, CMake `YAKOGL_REPORT_ES2=ON`); ES 3.0 is
   what an image that turns that off gets, and what the cargo crate reports
   without the `report-es2` feature.
5. More boards: a board joins when it meets stages 1 and 2 as the first
   three do.

Beyond the five stages, not scheduled: ES 3.1 / 3.2 (#19).

## State of main

main is `b44e9a9f` (2026-10-17). Board numbers, with the commit each image
was built from, are in `tools/cts/RESULTS.md`; the Japanese summary is
`tools/cts/STATUS.md`. Per-case detail: `docs/es2-cts-failures.md` (ES 2.0)
and `docs/es3-cts-failures.md` (ES 3.0). All three boards boot standalone
from eMMC (the two Verdins, U-Boot) or SD (Pi 4): `docs/standalone-boot.md`.

`eglSwapBuffers` on a window surface completes the frame it submitted unless
the native window carries `GLES_NATIVE_WINDOW_PIPELINED`
(`include/EGL/eglplatform.h`), the flag an application sets when it
double-buffers the window itself (the cluster sample sets it; glmark2, Slint
and triangle do not). A pbuffer's swap is unchanged. This removed the black
band of the last-drawn rows (about rows 40 to 90 of 800x600) on every
glmark2 scene of the i.MX8MP, which showed a buffer the GPU was still
writing (present since `dc6e62e4a`, 2026-10-07; not a regression). The
glmark2 scores below (AM62P 462, Pi 4 188, i.MX8MP 43 at `e9901869a`) were
measured with the pipelined swap: every board's glmark2 now waits for its
frame before showing it, so the next common set run re-measures them. The
benchmark samples stay stock: the overlap comes back from the driver.

The GL core keeps the first eight GL errors by entry point and client tag
(`yakogl_gl_error_first`); glmark2 prints them as `glmark2: GL error K of N:`
lines and, with the SNVS selector `0x800 | k`, stores error k as record kind
23 GL_ERROR (`driver/docs/glmark2.md`, "The first GL errors by name"). The
host harness models the GC7000's carve-out (`YAKOGL_GC_CARVEOUT_BYTES` in
`src/osal/linux.c`, set by `tools/hostharness/glmark2/run.sh` from the
i.MX8MP glmark2 image's overlay); the gate's glmark2 row fails on any GL
error line and on an entry whose GC peak is above 3/4 of that carve-out.

### ES 2.0 (stage 1, met)

At main `e9901869a` (2026-10-15/16), every image booted by the board's own
bootloader and run with `tools/board.sh`:

| Board | dEQP-GLES2 | Common set (GM SO SV CL GT) |
|---|---|---|
| AM62P | 17,330 pass / **0 fail** / 34 NS / 5 QW / 2 CW (17 images at 1x, at `7e1dd598`; 17,371 cases begun, every case with a result), 0 resets | 5 of 5 done (at `fb9107af`, 0 GPU resets): glmark2 462 (33 scenes, 0 failed, 0 GL errors); Slint probes pass; cluster 75.2 fps / 46.2 Mpx/s at 1024x600; gl_test 50 / 50 |
| i.MX8MP | 17,318 of 17,371 begun / **0 fail** / 34 NS / 19 other (e14 14, e27 5), 0 resets (tree re-run at `f40a2aba`, 29 selector rows, `records/imx8mp/deqp-f40a2aba/`, same totals) | 5 of 5 done: glmark2 33 scenes, 0 failed, 15+ GL errors (#814) at `e9901869a`; **0 GL errors with the glmark2 image's 240 MiB carve-out, on branch `b421b447` (2026-10-16, two common sets, `records/imx8mp/common-b421b447{-r1,}`): score 42, 33 scenes ran, 0 failed, 0 incomplete-texture draws; SO, SV, CL probes pass; gl_test 48 / 49 (case 44) then 49 / 49. Rerun from main** (`tools/cts/RESULTS.md`, "Common set", "i.MX8MP glmark2 GL errors (#814)"); gl_test case 16 skipped (#8) |
| Pi 4 | 17,126 / **0 fail** / 34 NS / 3 QW / 2 CW, every list case with a result, 0 recoveries | 5 of 5 done: glmark2 188, 0 GL errors; Slint probes pass; cluster 139.4 fps at 1920x720; gl_test 12 / 12 |

The 34 NotSupported are the same on the three boards and accepted
(`docs/es2-cts-failures.md` section C, `tools/cts/es2-accepted.txt`):
`clip_control` 8, `negative_api.texture.compressedtexsubimage2d_*` 9,
`multisampled_render_to_texture.readpixels` 1, `multisample.*` 16 at 1x.

ccfarm reruns these as post-merge board stages on each new main
(`tools/cts/board-stage.sh <board> common|deqp`, `am62p ms4`; stages
`deqp-tree-<board>` and `board-common-<board>` in `.ccfarm/project.toml`).

### ES 3.0 (goal 4)

Board numbers (`tools/cts/RESULTS.md`, "ES 3.0"):

- AM62P whole dEQP-GLES3 tree on `ad2b5ed` (same `src/`, `driver/`, `capi/`
  as `a63fcc5b`), 2026-10-14: **36,552 pass / 2,880 fail / 2,195
  NotSupported / 185 QualityWarning / 998 without a result**. Every case that
  is not Pass: `docs/es3/am62p-gles3-notpass.tsv`. The 998: image 7's
  `ResourceError` tail (987) and 11 slow `texture.filtering.3d` cases. Later
  row re-runs (`cd738b1a`, `4e62d171`) pass transform_feedback 882 / 882, ubo
  1,099 / 1,099, occlusion_query 62 / 62, `fragment_ops.blend.fbo_srgb`
  1,152 / 1,152. gl_test 196, 200 to 205 pass on the AM62P (`cd738b1a`).
- Pi 4: 7 of the 91 images of `tools/cts/pi4-gles3-tree.txt` on `cd738b1a`:
  3,807 cases, 2,798 pass / 744 fail / 265 NotSupported, 0 resets; failing
  cases in `docs/es3/pi4-gles3-fails.tsv`. gl_test 202, 203, 205 pass; 196
  and 204 fail ("Known defects").
- i.MX8MP: never run on the board (`docs/es3/board-recipe-imx8mp.md`).

Every other ES 3.0 number is HOST-ONLY: the Linux OSAL runs no command
stream, so only API errors and compile and link outcomes mean anything there
(`tools/cts/README.md` section 2b). Under `report-es2` every ES 2.0 answer
is kept and pinned by tests.

| Feature | State | Design |
|---|---|---|
| Transform feedback | built on the host for the three GPUs: the GL core runs the vertex shader on the CPU for every captured vertex, including its texture look-ups on 2D, cube, 3D and 2D-array samplers (`src/shaderc/online/eval.rs` `TexelSource`, `src/gl/tf_capture.rs` `GlTexels`). Host dEQP `transform_feedback` 882 / 882, `lifetime` 56 / 56; board: AM62P rows pass, gl_test 203 on the AM62P and the Pi 4. | `docs/es3/transform-feedback.md` |
| Uniform blocks | host: `tests/es3_ubo.rs` ports dEQP's `functional.ubo` (2,032 cases); the GC7000 refuses four at link (`KNOWN_LIMITS`) | `docs/notes/gc-ubo-const-limit.md` |
| Sized internal formats | the 35 colour formats of ES 3.0 table 3.2, converted at upload; the PowerVR samples every class but `RGB10A2`; the GC7000 and V3D refuse the integer ones by name | `docs/es3/sized-formats.md`, `docs/es3/pvr-formats.md` |
| Occlusion queries | counted on the three back ends; board: AM62P passes, Pi 4 reads 0 | `docs/es3/occlusion-query.md` |
| Integer colour render targets | the PowerVR writes all nineteen as a draw's one colour attachment (board: gl_test 200); the V3D writes all nineteen, the float ones and the ES 2.0 formats among any attachments, host only (`src/v3d/state.rs` `rt_format`; `docs/online-shaderc/v3d-backend.md` "Integer and ES 3.0 colour render targets"); the GC7000 refuses by name | `docs/es3/pvr-formats.md` section 10, `docs/es3/int-rt-gc.md`; `tests/es3_int_rt_v3d.rs` |
| `glBlitFramebuffer` | colour on the CPU, shared by the three back ends; depth on the CPU into a depth texture's texels, from a depth texture or a depth renderbuffer (host only, `tests/es3_blit_ds.rs`); depth into a renderbuffer or the window, depth from the window, and all stencil refused by name | `src/gl/blit.rs` |
| sRGB | PowerVR renders and samples (board: gl_test 204); the GC7000 and V3D sample and refuse a draw into sRGB by name | `docs/es3/srgb.md` |
| Buffer mapping and pixel buffers | host-proven; board: gl_test 205 on the AM62P and the Pi 4 | none |
| NPOT levels of arrays and storage textures | valid at every level | none |
| Framebuffer completeness | host dEQP `fbo.completeness`: 17 cases Fail to Pass on the PowerVR and the GC7000UL (`src/gl/framebuffer.rs` `check_status` under `!REPORT_ES2`); causes left in "Open work" item 4 | none (`tests/es3_fbo_fragops.rs`) |
| Stage-interface link rules | interpolation and block mismatches refused | none |
| GLSL ES 3.00 in the compiler | PowerVR 3,457 / GC7000UL 3,457 / V3D 3,457 of 3,461 expected-build dEQP programs build (`tests/es3_shaderc_corpus.rs`) | `docs/es3/shaderc-coverage.md` |
| Vertex array conversion | a buffer array the GPU cannot fetch as it lies (packed 2_10_10_10, `GL_HALF_FLOAT`, `GL_INT` / `GL_UNSIGNED_INT` through `glVertexAttribPointer`) is converted to float32 rows like a client array (`src/gl/draw.rs` `attrib_in_place_layout`); host only, `tests/es3_vertex_arrays.rs` | none |
| `layout(location)` on vertex inputs | `glBindAttribLocation` no longer moves a qualified input (`src/gl/program.rs` `ExplicitAttribs`); host only, `tests/es3_attrib_location.rs` | none |
| Multiple render targets | GL core done: the draw-buffer count is the back end's `backend::DRAW_TARGETS` (3 on every GPU); the framebuffer stores 4 colour points | none |
| PowerVR 2D-array and shadow sampling | designed, not built | `docs/es3/pvr-2darray.md` |
| ES 2.0 report guard | `tests/es2_report_guard.rs` pins what an ES 2.0 client sees; its `BUG` pins are "Open work" item 4, es2-report-gaps | none |
| ES 3.0 limits test | reverted for review (`3f78982`); es3-caps redoes it | none |

## Open work, in order

1. **Stage 2: the benchmarks right on the three boards.** The closing items
   are numbered at the end of `docs/notes/stage2-bench-gap.md`: a per-scene
   picture value in the glmark2 image and its tally (today
   `tools/cts/common-tally.py` judges console error lines only), per-scene
   evidence on the i.MX8MP (no console), fps for Slint and for the Pi 4 /
   i.MX8MP, terrain and refract checked on a board, the Slint Pi 4
   fingerprint decision, the cluster "same picture" rule, and a README table
   from one commit.
   - **AM62P: PowerVR `glGenerateMipmap` on the GPU** (#916; #1, #2). terrain
     (800x600, 10 levels) and refract (1600x1200, 11 levels) box-filter their
     chains on the CPU every frame (`src/pvr/backend.rs` `generate_mipmap:
     None`, the CPU path of `src/gl/texture.rs`): 1,467 and 2,046 ms of
     GL-call time a frame, 1 fps each. The V3D's TFU path is the model; the
     PowerVR needs a GPU chain (TQ or a per-level draw). shadow's 353 ms a
     frame is not explained by this and needs kick times
     (`CONFIG_SAFEUI_GPU_PVR_KICK_TIMES=y`) in the next GM image.
   - **i.MX8MP: glmark2 GL errors** (#814, #915): cause found (the GC's
     64 MiB carve-out), fixed in the glmark2 image and met by `b421b447` on
     the board (0 errors). Closes when a common set built from main reads
     `GL errors raised 0` in the GM row (`tools/cts/board-stage.sh imx8mp
     common`). The old "an i.MX8MP glmark2 run whose record shows them" is
     done (15 or more at 64 MiB; 0 at 240 MiB).
   - **i.MX8MP: present without a per-frame VSYNC wait** (#737). `board_show`
     (`driver/zephyr/lib/board/imx8mp/board_imx8mp.c`) waits for the next
     VSYNC every frame although glmark2 asks for swap interval 0, which fits
     the scores 37 to 46. Step 1 of `docs/notes/imx8mp-frame-floor.md`
     confirms it with one board run; fixes F1 and F2 are there.
   - **Pi 4: glmark2 refract and terrain** (#4, #5, #888): the image takes a
     96 MiB carve-out and a 24 MiB tiled-copy budget; at `e9901869a` GM ran
     33 / 0 / 0 with 0 GL errors (score 188). A colour render target is
     sampled from its render image and its mipmap chain is the TFU's
     (`docs/online-shaderc/v3d-backend.md`, "Render targets sampled from
     their render image"); shadow and refract still resolve a depth texture
     on the CPU every frame, and terrain's remaining CPU time is unexplained.
     Decide whether `e9901869a` closes #4 and #5.
   - **Double-buffering inside YakoGL's EGL window surface**, so a stock
     application gets a complete picture AND the CPU/GPU overlap: the surface
     owns a second colour buffer (OSAL GPU memory), `eglSwapBuffers` submits
     the frame into the buffer not on screen, waits for the frame before it
     and presents that one through an OSAL present hook the board layer
     implements (board_show() of a given buffer, which the Pi 4 pan, the
     AM62P plane flip and the i.MX8MP LCDIF address already support). The
     samples' flip() then only feeds the watchdog.
     `GLES_NATIVE_WINDOW_PIPELINED` stays for an application that manages its
     own halves (the cluster sample).
   - `tools/cts/common-tally.py` prints "no glmark2-mem lines" for a console
     that has them; the verdict is right, the remark is not (host only).
2. **Compile shaders at build time with our own compiler.** Shaders known at
   build time are compiled then; the run-time compiler runs only on a miss.
   An image without the run-time compiler refuses a miss by name. Steps 1 to
   6 are done (the precompiled tier stays off by default: the compiler costs
   884,201 bytes of flash and 5,244,035 of RAM on the AM62P gl_test image,
   one app's table 2,173,648 bytes of flash; `tools/precompile/measure-image.sh`).
   Open:
   - the driver's five end-of-tile programs, the boot self-test's vertex
     shader and the VDM-sync program are still Mesa's text compiled by pco;
   - four gl_test sites build GLSL at run time (149 `case_vsunits`, 150
     `shprobe`, 166 `fshprobe`'s `shp_arm` and `fshp_arm_units`) and need the
     compiler, as do gl_triangle / gc_gl_triangle (C literals), glmark2 and
     dEQP;
   - the GC7000 has no table row for the 69 gl_test sources in
     `tools/shaders/gltest/` (#22): `gen_shader_table.py --append` with
     etna_shc, then an i.MX8MP run with no compiler; the V3D table cannot be
     regenerated while `tools/v3d/glsl2v3d.py` has unresolved sites;
   - `tools/vivante/check_gl_test_blobs.py` and a key check are not in
     `tools/check.sh`;
   - steps 7 to 9: rename to "precompiled shaders" and "shader compiler"
     (features, Kconfig, C ABI, log prefixes), move `src/shaderc/online` to
     `src/shaderc/compiler` and `docs/online-shaderc` to
     `docs/shader-compiler`, optionally V3D rows.
3. **Design clean-up.** Clean-ups 1 and 2 (the GC7000's debug knobs) are on
   main.
   - Clean-up 3: the PowerVR's switches leave the product (5 hard-coded at
     their measured defaults, 2 switches and 8 options behind `test-knobs`,
     5 options retired). No branch exists; `src/pvr` has no `test-knobs`
     gate.
   - `gc_debug_msaa` goes when #8 is fixed.
   - Order from here: GL core / V3D knobs; the driver Kconfig instruments;
     samples cut (22 today -> triangle, gl_test, slint, cluster, deqp, plus
     glmark2); gl_test Rust harness; compiler knobs (after item 2's rename);
     bare metal; names; OSALs to Rust.
   - About 59 debug and experiment knobs remain (32 `static mut`, 16
     `PVR_BACKEND_OPT_*` slots, 11 others). Only `IF_LOWERING` and
     `ONE_IN_FLIGHT` stay product options; the measurement infrastructure
     stays.
   - Bare metal is made true, not removed: `src/osal/baremetal.c` is a
     skeleton with three `TODO(board)`; it needs the compiler's scratch, a
     256 KiB compile stack, a board layer, a gate row for `baremetal.mk` and
     one recorded boot. Design first, after the rename.
   - Names: `SAFEUI_*` -> `YAKOGL_DRV_*` (about 650 occurrences, 48 Kconfig
     symbols, the M7 cross-core names), `safeui,*` DT compatibles ->
     `yakogl,*`, module `powervr-bxs` -> `yakogl-drivers`, `PVR_SAMPLE_*` ->
     `YAKOGL_SAMPLE_*`.
   - C to Rust: the OSALs (3,740 lines), `gl_test.c` (about 20,000 lines,
     through a Rust runner calling the C cases), `driver/src` (about 16k
     lines).
   - The current design documents are `driver/docs/pvr-render.md`,
     `driver/docs/rpi4-v3d.md`, the "Hardware findings that still hold"
     section of `driver/docs/vivante-kernel-rust.md` and the "What the image
     does on a go entry" section of `docs/standalone-boot.md`.
4. **ES 3.0, host first.** A proposed order. Each item keeps every
   `report-es2` answer and adds a test that pins it.
   - **tf-capture**: the board next is gl_test 203 on the i.MX8MP;
     `lifetime.attach.deleted_output.buffer_transform_feedback` still fails
     on the AM62P. Still refused by name in a captured vertex shader: a
     shadow look-up, `textureGrad`, compressed textures, BASE_LEVEL other
     than 0, a sampler whose component type does not match, storage with no
     CPU view. Per-GPU code-generator refusals (`texelFetchOffset`, integer
     2D arrays, ...) are code-generator items. `src/GL_CORE_NOTES.md` lines
     394 and 634 and `docs/es3/transform-feedback.md` sections 4.1 and 5.7
     still say the capture refuses a vertex shader that samples.
   - **int-rt**: left on the PowerVR: an integer attachment beside others,
     colour masks that keep some channels of an integer target, precompiled
     rows for the integer variants (dEQP `builtin_functions.common.modf`
     needs the first). Left on the V3D: a board run (gl_test 200, dEQP
     `fragment_out`, `fbo.color`, `builtin_functions`), `tools/v3d/job_check.c`
     / `cl_golden.c` rows for a typed job, an integer clear program for a
     scissored or masked clear, the F32 TLB write for R32F, 4x on an ES 3.0
     format. The GC7000: int-rt-gc (`docs/es3/int-rt-gc.md`).
   - **formats**: the PowerVR's integer cube / 3D / 2D-array samplers and
     `RGB10A2` as its own class. `RGB10A2`, in order: a render-target class
     for the 10/10/10/2 unorm word (`docs/es3/pvr-formats.md` 2.5, with a
     host test of its PBE words); accept `GLES_TEXFMT_RGB10A2` in the
     texture-attachment guard of `src/gl/framebuffer.rs`; allow it in
     `glGenerateMipmap` with a CPU box filter for the packed word; then add
     it to `PVR_SAMPLED_TEXFMTS` and update `docs/es3/sized-formats.md` and
     `docs/es3/pvr-formats.md` 10.1.
   - **srgb**: the GC7000's and the V3D's sRGB render targets
     (`docs/es3/srgb.md` 5.1, 6.1), an sRGB attachment beside another on the
     PowerVR, compressed sRGB. On the AM62P the 2D-array, 3D and cube texture
     paths fail for every format, sRGB included (15 cases): a texture-path
     item, not sRGB.
   - **occlusion-query**: 196 on the i.MX8MP, and on the Pi 4 after its fix;
     the 4 `negative_api` cases on the AM62P. Unverified points:
     `docs/es3/occlusion-query.md` section 9.
   - **pvr-2darray**: PowerVR 2D-array and shadow sampling
     (`docs/es3/pvr-2darray.md`); the 3D and 2D-array cases of `buffer`,
     `pbo`, `read_pixels` and `texture.specification` wait for it.
   - **pvr-3d** (`docs/notes/pvr-3d-mipmap-hang.md`): (a) native 3D textures
     on the PowerVR (Mesa's `TEXTYPE_3D`), or levels and wrap modes in the
     grid emulation: the PowerVR samples a 3D texture's level 0 only and
     loses REPEAT (`src/gl/texture.rs` `texture_alloc_storage_3d`,
     `src/shaderc/online/powervr/select.rs` `vol_uv`); (b)
     `tools/cts/gen-am62p-gles3-tree.py`: name only mustpass prefixes in
     `3d.formats` and put each 3D `*_linear_mipmap_linear` case in its own
     image, so the 83 tail cases of images 20 and 21 report; (c)
     `tools/board.sh`: an AM62P dEQP image's `RUN_BOUND` at least the 600 s
     watchdog bound + 60 s + the boot.
   - **es3-shaderc**: remaining expected-pass refusals on the PowerVR: 2
     `constant_expressions.other.complex_*` (case labels) and 2
     `uniform_block.valid.long_instance_name_*` (reflection names of 63
     characters or more need a decision). `packSnorm2x16` / `packUnorm2x16`
     still round with floor(x + 0.5). A register-pressure-aware scheduler
     would build 5 more Qt3D vertex shaders on the GC7000.
   - **gc-ubo**: the GC7000's four `ubo` cases refused for constant
     registers (368 against 320). Read user blocks from memory with `load`
     as Mesa etnaviv does; design and sizes in
     `docs/notes/gc-ubo-const-limit.md`. Lifts the `take_uniforms` cap
     ("Known defects"). Needs one i.MX8MP run for a constant-offset `load`.
   - **blit-ds**: still `GL_INVALID_OPERATION`: (a) depth into a
     renderbuffer or the window, (b) all stencil, (c) depth from the window
     or a cube/array depth texture; these are all 16 AM62P
     `fbo.blit.depth_stencil` failures. (a) and (b) need two back-end
     entries in `src/backend.rs`: `ds_write(id, plane, src, x, y, w, h,
     pitch)` (write a rectangle of DEPTH f32 or STENCIL u8 into the object's
     buffer, in the layouts `depth_resolve` reads) and `stencil_resolve`;
     then `copy_depth` writes through `ds_write`. (c) needs a measurement of
     which way up each back end keeps the window's depth.
   - **es3-caps**: redo the ES 3.0 limits test (`3f78982`).
   - **es2-report-gaps**: the `BUG` pins of `tests/es2_report_guard.rs`
     (ES 3.0-only pnames and capabilities answered, ES 3.0 renderbuffer
     formats accepted, ES 3.0-only pixel types answered
     `GL_INVALID_OPERATION` instead of `GL_INVALID_ENUM`). A static audit
     lists 44 ungated ES 3.0 rows, none reached by dEQP-GLES2
     (`docs/notes/es2-error-rules.md`).
   - **es3-vertex** (causes in `docs/es3-cts-failures.md`):
     - `attribute_location.mixed_max_attributes.*` (20 AM62P cases):
       `src/shaderc/online/reflect.rs` `reflect()` must give an unqualified
       input the lowest run of locations no `layout(location)` and no
       earlier input uses; then remove the `#[ignore]` of
       `mixed_max_attributes_vec4` and
       `unqualified_unbound_inputs_avoid_every_qualifier` in
       `tests/es3_attrib_location.rs`.
     - `instanced.*.mixed.{2,4,20}_instances` (6): design chosen in
       `docs/notes/instanceid-per-draw.md`: PowerVR P1 (the PDS
       `instance_id_modifier`), then P3 (hardware divisors); GC7000UL V0
       (set `vs_instance_id_reg1` when the program reads `gl_InstanceID`)
       and V1 (the stream divisor). `src/gl/draw.rs` needs no change.
     - `vertex_arrays` (41) and `attribute_location.mixed*` (28): fixed on
       the host, not yet seen on a board.
   - **fbo-completeness**: left: separate depth and stencil with colour
     attachment 0 (12 `attachment_combinations` cases: drop the
     `fb.color.type_ == GL_NONE` term in `check_status`, un-ignore
     `separate_depth_and_stencil_with_colour_zero_are_unsupported`, edit the
     four unit tests that build ES 2.0-style D16 + S8 pairs); the 10
     `fbo.depth` clamp cases (`gl_FragDepth` on the PowerVR); `GL_R32F` as a
     render target (a decision: `GL_EXT_color_buffer_float`);
     `renderable.texture.color0.rgba32f`; the PowerVR's 19 integer texture
     attachments; the GC7000UL / V3D refused uploads; the 4 unsized depth
     textures under `!REPORT_ES2`; `completeness.layer.3d_64_15` on the
     PowerVR; a draw into colour attachments 1.. only.
   - **es3-mrt**: each GPU raises its `DRAW_TARGETS` arm in `src/backend.rs`
     to 4 once a board draws 4 targets (PowerVR: a fourth end-of-tile
     program and PBE state, `PVR_RENDER_MAX_RT` 3 in
     `driver/src/pvr_render.h`; V3D and GC7000UL: a board run into target
     3). Then `tests/es2_report_guard.rs` `draw_buffer_queries_are_invalid_enum`
     moves from 3 to 4. Also: `tools/check.sh` lacks `--test es3_mrt_gl` in
     the V3D rows.
   - **Board trees**: the remaining Pi 4 GLES3 images (84 of 91; each ran in
     under 30 s of its 240 s window, so the table could be re-split into
     fewer images), the i.MX8MP GLES3 tree, and the AM62P groups by size in
     `docs/es3/am62p-gles3-notpass.tsv` (classes in
     `docs/es3-cts-failures.md`; the texture groups `2d_array`, `mipmap.3d`,
     `filtering.3d`, `shadow.2d_array`, `swizzle`, `units` and the
     `sglrGLContext.cpp:863` compile failures are the largest).
5. **Publication (stage 3)**, `docs/release/checklist.md`:
   - Line 3 (CONTRIBUTING.md commands run as written) and line 6 (README
     walkthrough): partial until `tools/check.sh --quick` exits 0 with
     `MESA_V3D_TREE` set to a tree without `build-v3d` (item 6), and
     CONTRIBUTING.md:16's "runs two rows" for a docs-only change is checked.
   - Licence audit (`docs/release/license-audit.md`): owner decision, headers
     or allowlist rules for the ~597 missing-spdx files; then check 25 of
     `tools/cts/cts-selftest.sh` goes `--strict` or to a baseline. Re-measure
     the report's tables (they carry `79c73ea`'s 587).
   - Line 4 (SECURITY.md): the contact and two other `TODO(owner)` lines are
     the owner's.
   - Owner: CONTRIBUTING.md says a pushed branch is never rebased; the ccfarm
     `conventions` text says branches are rebased onto origin/main.
   - Website: refresh for the current GLES3 status (#411).
6. **Gate and tools.**
   - `tools/check.sh` reports a missing `build-v3d` under `MESA_V3D_TREE` as
     a failure; it should be a gap.
   - The AM62P Zephyr image rows should be a gap when
     `driver/src/fw/pvr_fw_blob.h` is absent; nested runs clobber
     `build/check-fail/<n>.log`; the `--quick` comment at `tools/check.sh:17`
     is stale; a symbolic `ZEPHYR_WS` may break the AM62P Zephyr rows
     (`tools/check.sh:2519`, not confirmed).
   - The osal-linux row list in `tools/check.sh` has no V3D entry.
   - `board.sh` does not check the shape of `BOARD_HDMI_TIMINGS` (17
     numbers) before it changes the board.
   - `tools/cts/common-images.sh` cannot set `CONFIG_GL_TEST_CASES` /
     `CONFIG_V3D_GL_TEST_CASE(S)` or pass `-pipe`.
   - i.MX8MP GLES3 tooling: failing case names print as `crc:<hex>` (map
     them with the table).
7. **Small code items.**
   - V3D `mem.cpu` NULL guard: `src/v3d/backend.rs`, the kept-image path
     does `(mem.cpu as *mut u8).add(pad)` after `gpu_alloc` without a null
     check (the other call sites check); free and return
     `KeptMiss::NoMemory`.
   - `build_tex_desc` is private, so `tests/pvr_npot_levels.rs` derives the
     texture words from the bind; exposing it under `host-test` would let the
     test compare the real words.

## Known defects

Each has a GitHub issue unless marked "no issue yet". Unless a line says
"board", the fix is proved on the host only.

- **AM62P GLES3 images 20 and 21** (no issue yet): stop at the 14th case
  (`texture.filtering.3d.formats.r11f_g11f_b10f_linear_mipmap_linear`,
  `texture.filtering.3d.sizes.3x7x5_linear_mipmap_linear`). For image 20 it
  is the 600 s image lifetime, not a GPU hang: dEQP's check of the wrong 3D
  picture is slow (the case alone completes in 158 s, Fail, no reset). Image
  21's case has not run alone. Cause and fix: item 4, pvr-3d;
  `docs/notes/pvr-3d-mipmap-hang.md` section 7 has the board check.
- **AM62P GLES3 image 7** (no issue yet): fixed on the host (`draw_dispatch`
  in `src/pvr/backend.rs` cuts a sequential draw of client arrays into
  pieces, `draw_sequential_pieces`; host `builtin_functions.precision` 0
  ResourceError), waiting for a board run. A fan, a line loop, a second
  index list or one primitive over half the GENERAL arena is still refused
  by name when it does not fit an empty frame.
- **AM62P fbo / blit / invalidate** (no issue yet, board `cd738b1a`):
  `fbo.invalidate.{sub,whole}.unbind_blit_*` 14 Fail,
  `fbo.blit.default_framebuffer` `r8_*_blit_from_default` 3,
  `fbo.color.repeated_clear.blit.tex2d.rgb16f` 1, `fbo.blit.conversion` 112
  NotSupported (each has an `r32f`, `rg32f`, `rgba32f` or `r11f_g11f_b10f`
  end), `lifetime.attach.deleted_output.buffer_transform_feedback` and
  `deleted_input.buffer_vertex_array`. Many are
  `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`.
- **pvr-msaa-split** (#890): a 4x frame split into two kicks keeps its
  samples (an unresolved sample store reloaded by the next kick's load
  quad); host tests in `src/pvr/backend_tests.rs`. Two choices are taken
  from the headers, not measured (the PBE's unresolved layout as the TPU's
  `smpcnt` image, and the per-sample fetch); the board check is in "Next
  board session", AM62P.
- **AM62P black panel under a kexec'd yakogl** (#628): every register of the
  display path equals the Linux dump and the DSI streams, but the picture
  was not seen. Next: photograph the fill image ("Next board session",
  AM62P); `driver/docs/am62p-display-cold.md`.
- **i.MX8MP gl_test 44 `es3hdr`, RGBA16F target** (#6): the 64-bit
  colour-target fix (#866) passes 49 / 49 at `e9901869a`; keep the layout
  probe recipe ("Next board session") if it comes back. Hypotheses:
  `src/vivante/VIVANTE_NOTES.md`, "RGBA16F targets". It failed once in
  `b421b447`'s first common set and passed in the second; count it with
  the flaky cases 2 and 10.
- **i.MX8MP gl_test 16, 4x MSAA** (#8): the default image skips its 4x half
  by name. On the 4x image `x4 inside` reads 0, and the 4x scratch is empty
  after the clear and after the draw. Recipe: `docs/notes/gc-msaa-record.md`.
  #183: the GC7000 resolve uses A4R4G4B4 for every 16-bit target.
- **i.MX8MP gl_test 10 (#9) and 2 (#25)**: nondeterministic. At `e40e3835`,
  40 boots per arm: case 10 failed 4 of 40, case 2 0 of 40; with the
  KICK_DIFF second read-back both failed 1 of 40 and were stable, so the
  write was lost, not late. Next: the K1 arm (etnaviv's flush rule in
  `emit_flushes`, `docs/notes/gc-kick-diff.md`); `docs/notes/gc-case10-flake.md`.
- **i.MX8MP gl_test 171 `viewwin`** (#15): the raw memory check ignores byte
  order (the GC7000 is BGRA); 171 is not in the default list and has not run
  on the i.MX8MP.
- **i.MX8MP panel** (#23): 1.5 copies of the frame per line; the 69.3 MHz
  panel's record is clean, viewing by eye pending.
- **GC7000UL: a split instanced draw that reads `gl_InstanceID` is drawn
  wrong without a message** (no issue yet): the refusal in
  `src/vivante/backend.rs` tests `vs_instance_id_reg1`, which only the
  PowerVR compiler sets. Fixed by V0 (item 4, es3-vertex).
- **GC7000 `take_uniforms` truncates** (no issue yet): it cuts a stage's
  uniform block at 1280 dwords (`src/vivante/backend.rs`); harmless while
  the block must fit the registers, silent once blocks are read from
  memory. Item 4, gc-ubo.
- **GC7000 `TEXDESC_LOG_SIZE_EXT` for NPOT sizes** (no issue yet, low):
  `log2_fixp88` (`src/vivante/texture.rs`) approximates log2 linearly; pinned
  by the ignored `bug_log_size_ext_is_a_linear_approximation_of_log2`.
- **GC7000 offline table** (#22): no `gc7000` role for `tools/shaders/gltest/`
  (item 2).
- **Pi 4 gl_test 196 `occl`** (no issue yet): the V3D occlusion counter reads
  0 for every query on the board (want 0, 1, 0, 1, 1), though it counts on
  the host. Check `OCCLUSION_QUERY_COUNTER`'s enable and read path.
- **Pi 4 gl_test 204 `srgb`** (no issue yet): the case prints that the render
  half is the PowerVR's, then FAIL with no checks; the sample half should
  run. Read case 204's V3D guard in `driver/tools/pvrlab/gl_test.c`.
- **i.MX8MP glmark2 GL errors** (#814), cause found, fixed in the glmark2
  image: every GC allocation, the board layer's 800x600 window and 1280x800
  screen copy included, comes from the one `gpu-carveout` pool, 64 MiB on
  the Verdin's device tree. On the host model terrain peaks at 51 MB and
  refract at 48 MB of the 58 MB left (one entry per process, frees returned
  at once); the board also holds frees until their kick completes and runs
  every entry in one fragmenting pool, and a refused allocation is a GL
  error. The glmark2 image now takes 240 MiB at 0x70000000 (60 of the GC
  MMU's 64 slave tables; `KERNEL_VM_SIZE` 512 MiB). The same cause and step
  as the AM62P's 2573 errors (#831). Not measured: which entry points
  raised the errors at 64 MiB (the run with the new GL_ERROR record needs
  `SEL`, which the diagnostic worker could not pass); the host model at
  64 MiB raises none, so it pins the fix by headroom, not by an error.
  Other GC images keep the board's 64 MiB (gl_test, Slint, cluster, dEQP:
  `2d.2048x2048_l8_mipmap` ended on GL_OUT_OF_MEMORY there at `4f40f622`);
  a client whose working set nears 48 MiB needs the same overlay step.
- **Pi 4 tryboot image off the network** (#905): a glmark2 tryboot image
  kept the board off the network for 15 h on 2026-10-15 and the watchdog did
  not bring it back; the same recipe ran clean at `e9901869a`.
- **`board.sh pi4 tree` as a background job** (no issue yet): killed at a
  tool's time limit it leaves `yakogl-<row>.bin` and `tryboot.txt` on
  `/boot/firmware` (a leftover refuses the next run). Run at most four rows
  at a time, or with the longest timeout.
- **Board lock and console port** (#867): the lock is a lease on the board
  itself since `11e7c66f4`; the farm's registry had named the wrong bridge
  port for the AM62P console (corrected).
- **Serial bridges drop lines**: a dEQP row whose image counters exceed the
  console is `done_lossy` (rc 4); rerun it. The fix is the bridge firmware.
- **PowerVR vertex-shader `textureSize` sampler** (no issue yet): a sampler
  a vertex shader reads only through `textureSize` has no uniform location
  (`src/shaderc/online/powervr/chain.rs` `row_sampled`).
- **`GL_HALF_FLOAT_OES`** (no issue yet): `attrib_type_size` and
  `attrib_component` read it as float32; accepted only under `report-es2`.
- **Vertex arrays** (no issue yet): the w fill of a narrow integer array
  follows the array, not the shader input (undefined in GL); the convert
  path's `gpu_read` branch has no host test.
- **Capture against draw** (by design, `tests/es3_tf_sampler.rs`): the
  capture applies texture swizzle and MIN_LOD / MAX_LOD and filters cubes
  seamlessly, which the draw does not yet.
- **Depth blit and the back end's buffer** (no issue yet): a depth blit into
  a depth texture changes its texels, not the back end's depth buffer; a
  later draw depth-tests against the old depth and the next resolve
  overwrites the texels. `glTexSubImage2D` of depth has the same gap. Item
  4, blit-ds (`ds_write`) closes both.
- **gate-lib-red-unnamed** (#188): the full gate once printed `FAIL test
  host-test,gpu-powervr,shaderc-online,precompiled-shaders` with no test
  name; not reproduced. Read the next red of this row from
  `build/check-fail/<n>.log`, never rerun it first.
- **AM62P firmware timing** (#17) and **a refused free-list grow with no 3D
  work pending** (#3, partial render missing): open.
- **Pi 4 gl_test 178 held frees** (#14): not measured on the Pi 4.

## Next board session

One GL client per board: check that the board is idle first. Images only from
commits on origin/main, built from a detached checkout; record the commit
with each result. `BOARD_BOOT=uboot-mmc` on the two Verdins; the Pi 4 uses
tryboot (board.sh's default).

The ES 2.0 sign-off runs are one command each, `tools/cts/board-stage.sh
<board> common|deqp` (and `am62p ms4`); the header of that script names the
environment. For anything else:

```sh
git worktree add --detach <sweep> main
SW=<sweep>; HEAD8=$(git -C $SW rev-parse --short=8 HEAD)
export ZEPHYR_WS=<west workspace> WEST_VENV=$ZEPHYR_WS/.venv ZEPHYR_SDK_INSTALL_DIR=<zephyr sdk>
export BOARD_HOST=<user@host>     # the board's other variables: header of tools/board.sh
$SW/tools/board.sh --dry-run <board> tree <table> <outdir>    # first: prints every command, touches nothing
```

An ES 3.0 gl_test image needs `CONFIG_GL_TEST_CASES` for `gl_test`;
`tools/cts/common-images.sh` has no case override, so append the line to
`prj.conf` for the build (or pass `-D` on the `west build` line). Expected
readings of the ES 3.0 gl_test cases are in the cases themselves
(`driver/tools/pvrlab/gl_test.c`): 196 `occl` q0 to q4 `available 1` with
0, 1, 0, 1, 1; 202 `blitconv` 12 / 12; 203 `tfcap` 9 / 9; 204 `srgb` 8 / 8;
205 `mappbo` 15 / 15. Each skips by name in an ES 2.0 context.

### i.MX8MP (Verdin, no UART: records are read from the SNVS)

```sh
export ZEPHYR_WORKSPACE=$ZEPHYR_WS ZEPHYR_VENV=$ZEPHYR_WS/.venv IMX8MP_OUT=<out> DEQP_STAGE1=<stage 1>
```

- gl_test builds need `-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y` (else `board.sh`
  refuses the image, rc=3), and a `;` inside a sweep list is written `\;`.
  An interrupted run can leave `/var/yakogl/once-*.bin`: `board.sh imx8mp
  disarm` before retrying. One boot takes about 2 to 2.5 minutes.
- Free space in the deps cache (`imx8mp-out`) before a tree run: row e27 stopped
  once on a full local disk quota (board.sh could not write its `once` token)
  and was rerun from the same records directory.
- After the #814 fix lands: `tools/cts/board-stage.sh imx8mp common` from
  main; GM must read `GL errors raised 0`. If it does not, name the errors:
  `SEL=$((0x800)) tools/board.sh imx8mp run <GM image> <logdir> gle0` (then
  `0x801`, ...), one boot of about 7 minutes each; `yakogl-tools record`
  names the entry point.
- ES 3.0 gl_test 196, 202, 203, 204, 205 (first run on this board).
- #9 / #25: the K1 arm (needs the K1 change in `src/vivante/backend.rs`),
  with the KICK_DIFF selectors of `docs/notes/gc-kick-diff.md`.
- #8: the two-arm recipe of `docs/notes/gc-msaa-record.md`.
- #737: step 1 of `docs/notes/imx8mp-frame-floor.md`.
- #6 if it returns, case 44's layout probe:

  ```sh
  $SW/tools/cts/imx8mp-gltest-build.sh gc44p-$HEAD8 44 -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y
  SEL=0 $SW/tools/board.sh imx8mp run $IMX8MP_OUT/gc44p-$HEAD8/zephyr/zephyr.bin <rec>/gc44p gc44p-$HEAD8
  ```

  Decode `case 44 check 3 ... got G want W` with the table in
  `src/vivante/VIVANTE_NOTES.md`, "RGBA16F targets".
- dEQP-GLES3 (never run): the 133 rows `e00` to `e132` of
  `tools/cts/imx8mp-gles3-tree.txt` per `docs/es3/board-recipe-imx8mp.md`
  section 3, from an image rebuilt at the commit the board runs
  (`tools/cts/imx8mp-gles3-artefacts.sh es3-<hash8>`).

### Raspberry Pi 4 (tryboot, no `BOARD_BOOT`)

`BOARD_UART` (the debug probe's serial bridge, `host:port`),
`BOARD_HDMI_TIMINGS` (the lab's 17 fields, `tools/cts/README.md`) and
`BOARD_HOST`. A leftover `/boot/firmware/yakogl-*.bin` or a non-empty
`tryboot.txt` refuses the next run (`tools/board.sh pi4 status` shows both).

- gl_test 200 `intrt` and the dEQP-GLES3 `fragment_out.basic.int`,
  `fragment_out.basic.uint` and `fbo.color.*` integer cases: the first board
  measurement of the V3D typed job and its 32-bit TLB writes.
- gl_test 196 after the counter is looked at, 204 after its guard.
- The remaining dEQP-GLES3 images (`tools/cts/pi4-gles3-tree.txt`,
  `docs/es3/board-recipe-pi4.md`; rows 0 to 6 ran, row 7 is FB4):
  `TREE_IMAGE_DIR=<dir> BOARD_MUSTPASS=<abs path of gles3-mustpass.txt>
  tools/board.sh pi4 tree tools/cts/pi4-gles3-tree.txt <outdir>` with
  `TREE_ROWS=<a>-<b>` (at most four rows a run), `DEQP_MODULE=gles3` and
  `DEQP_WORK` the gles3 stage 1. One row costs about 5 min 15 s.
- glmark2 (#4, #5): refract's console must show "the TFU made the 11-level
  chain of a 1600x1200 render image" (terrain's 10-level 800x600), no
  "tiled copy" line naming a colour render target, and 33 / 0 / 0.
- If the cube rows of the GLES2 tree fail (T3, T4): `docs/v3d-noseamless-cube.md`.

### AM62P (`BOARD_BOOT=uboot-mmc`; `BOARD_UART` set; `driver/src/fw/pvr_fw_blob.h` in the checkout)

The console must be the main_uart0 bridge port: check it with
`python3 tools/am62p_netboot_uart.py probe <host:port>` and a raw read that
shows bytes before the first image. `board.sh am62p disarm` clears a stale
`/var/yakogl/once-*.bin`; a bridge capture timeout (`verdict=scan_failed`,
rc 4) is a transport problem: rerun the row. If `board.sh` refuses (rc=3) because
a previous run left `/var/yakogl/once-img8-*.bin`, `board.sh am62p disarm` and
removing the stale image clear it (a one-shot image left in `/var/yakogl` by an
interrupted run blocks `board.sh tree`; `disarm` only cleared the U-Boot env on
2026-10-16, the file had to be removed by hand). Another session's dEQP tree may
hold the board for hours; on the AM62P `TREE_RETRIES` is 0, so wait for a
quiet board. A gles3 stage 1 is not the farm's `DEQP_WORK` (GLES2): build
one with `tools/cts/cts-build-zephyr.sh MODULE=gles3`.

- **pvr-msaa-split** on the ms4 image (`rgba8888d24s8ms4`), as a new gl_test
  case next to 16 and 173: clear black, draw case 16's white diagonal
  triangle into the 4x pbuffer, `glFinish()`, draw it again,
  `glReadPixels`. Expected: diagonal pixels `0x7f`, interior `0xff`,
  exterior `0x00`; the console shows `kick 1: ... samples` and `kick 2: ...
  rt 1, 1 draws, 1 quads, samples`. `0xbf` on the diagonal: the store held
  the resolved colour (try a 2w x 2h store with twice the pitch); `0xff` or
  `0x00`: every instance read one sample (the load shader needs
  `gl_SampleID`); a BIF fault or hang on kick 1: the second emit's address
  or extent (`pbe1` words). Then the ms4 row, 16 / 16.
- **glmark2** after a GPU mipmap path (#916): `tools/cts/cts-common.sh am62p
  build|run` with `CONFIG_SAFEUI_GPU_PVR_KICK_TIMES=y` in the GM image;
  read terrain, shadow and refract's `glmark2-frame:` lines against 1,467 /
  353 / 2,046 ms.
- **dEQP-GLES3**, by size:
  - image 7 again (`TREE_ROWS=6`, line 14 of `tools/cts/am62p-gles3-tree.txt`):
    it should run to its end; check for "does not fit an empty frame" or
    `index_base_addrlsb` lines;
  - the 11 slow `texture.filtering.3d.*_linear_mipmap_linear` cases, one
    image each; images 20 and 21 per `docs/notes/pvr-3d-mipmap-hang.md`
    section 7 (`RUN_BOUND=900 RETURN_BOUND=240`);
  - `attribute_location.mixed`, `mixed_time` (expect 0 failures) and
    `vertex_arrays.single_attribute.*` (expect the 41 failures gone);
  - after P1/P3 or V0/V1, `dEQP-GLES3.functional.instanced.*` (45 cases) on
    the AM62P and the i.MX8MP, per section 5 of
    `docs/notes/instanceid-per-draw.md`;
  - the 16 `builtin_functions.common.round` / `roundeven` highp cases (image
    7, expected to pass since shaders-fold);
  - row 19 of the sRGB rows.

  ```sh
  DEQP_MODULE=gles3 DEQP_WORK=<gles3 stage 1> \
    $SW/tools/cts/am62p-deqp-images.sh $SW/tools/cts/am62p-gles3-tree.txt <img3> $HEAD8 > <img3>/build.out
  TREE_IMAGE_DIR=<img3> TREE_TAG=$HEAD8 TREE_ROWS=<rows> \
    $SW/tools/board.sh am62p tree $SW/tools/cts/am62p-gles3-tree.txt <rec>/gles3
  $SW/tools/cts/deqp-uart-tally.py --mustpass $SW/tools/cts/gles3-mustpass.txt --statuses <rec>/gles3/*/console.txt
  ```

- **Black panel** (#628): from the node with the document camera, build
  `gl_triangle` with `CONFIG_PVR_SAMPLE_DISPLAY_FILL=0xffff0000`,
  `CONFIG_PVR_SAMPLE_DISPLAY_DUMP=y` and `CONFIG_PVR_SAMPLE_HOLD_SECONDS=40`,
  kexec it and photograph the panel 15 to 20 s after the jump. Red: the
  path is fine; repeat with the triangle alone (`DISPLAY_FILL=0`). Black:
  bisect the C-vs-Rust order differences of `driver/docs/am62p-display-cold.md`
  one at a time (the bridge EN after the DSI attach, VID_VSIZE1, ...).

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
  in this file (`docs/handoff/README.md` has the note's shape). The
  `handoff-fold` stage folds the notes in after the merge;
  `tools/handoff-notes.sh` lists what waits.
- Board results go into `tools/cts/RESULTS.md` (the newest row per suite and
  board, replaced in place) and `tools/cts/STATUS.md`, in the same push as
  the numbers. Session narratives are not committed as files: the commit
  message and the pull request carry them, and run records stay in
  `records/` (gitignored).
- Locally, run only the rows the change reaches:
  `tools/check.sh --changed <base>` maps each changed path to the parts it
  can break and skips the other rows. A shared layer (`src/gl`,
  `src/shaderc`, the OSALs, `capi`, `Cargo.toml`, the script itself) or a
  path the map does not know selects every row;
  `tools/check_scope_selftest.sh` pins the map. `tools/check.sh --local
  <base>` runs the local set only when a diff reaches everything.
- For the full run by hand: a clean detached checkout of the exact commit,
  with `driver/src/fw/pvr_fw_blob.h` copied in, and with
  `ZEPHYR_WS=<west workspace>`, `HAL_NXP_DIR=<hal_nxp>` and
  `GLMARK2_SRC=<glmark2 checkout at 22c527cb>` set. Without them the Zephyr,
  hal_nxp and glmark2 rows are gaps and the gate is red. The Mesa-table
  reproduction row also needs `PCO_SHC=<pco_shc from
  tools/pco_shc/build_pco_shc.sh>`, `GLSLANG=<glslangValidator>` and, for
  speed, `GLSL2USC_CACHE=<dir>`. `tools/setup-gate-host.sh` prepares a gate
  host, and `tools/cts/deps-env.sh` names the dependency layout the CI
  reads.
- Board runs use `tools/board.sh <board> run <zephyr.bin> <record dir>
  <label>`, from a detached checkout of main, with images built from commits
  on origin/main.
- A dEQP image bakes its table in: splitting rows needs a rebuilt image
  (`tools/cts/imx8mp-deqp-build.sh --table`; `tools/cts/README.md`).
- One GL client per board. Check that a board is idle before you use it.
