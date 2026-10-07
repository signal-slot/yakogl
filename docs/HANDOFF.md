<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Handoff

The state of the work and what comes next. Update this file in the same
commit as any result that changes it. Last update: 2026-10-06.

Board addresses, UART paths and credentials never go in this file; they come
from the environment (`BOARD_HOST`, `BOARD_UART`, ...).

## Goal, in order

1. ES 2.0 finished on the three boards (AM62P PowerVR, i.MX8MP GC7000,
   Raspberry Pi 4 V3D), each booted from its own storage by its bootloader.
2. Minimum performance.
3. A public website (`website/`). It goes live when the repository is public.
4. ES 3.1 / 3.2.

## State of main

Results are from main as of `186cbb56`: AM62P gl_test on `05f8f749`, Slint and cluster on `05f8f749` (`7dada1ff` for the fill_color figures), i.MX8MP gc_gl_test on `05f8f749`, the Pi 4 on `afa12e9b`. All three boards boot standalone from eMMC (the two Verdins, U-Boot) or SD (Pi 4).
`docs/standalone-boot.md` has the procedure.

| Board | gl_test | Slint | Cluster |
|---|---|---|---|
| AM62P | 48 / 48, 0 resets (the set now has 192 `predlive`); 48 / 48 in the compiler-only and precompiled + compiler images (`05f8f749`) | pass on `7dada1ff` (705 frames in 10007 ms; also with the precompiled tier, same records, `17605c36`) | 91.8 fps, 56.4 Mpx/s, 16.28 ms/Mpx on `7dada1ff` (also with the precompiled tier, same records, `17605c36`; `39bd6983`: 91.2 fps, 56.0 Mpx/s, 16.11 ms/Mpx) |
| i.MX8MP | 46 / 48 on `72d16864` and `112dd4fb`, first failure case 16 (#8). On `05f8f749`: 44 / 48 (first failure case 2), 45 / 48 (case 2), 46 / 48 (case 16): case 2 is intermittent (#25) | not run | not run |
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
2026-10-07 on the lepton host (`// toolchain: pco_shc=01aa62a069bb
glslang=2e8c31daf6b2`): all 2476 USC rows are byte-identical to the
`0fe778f6` build, so the board results recorded against that table carry
over. The `0fe778f6` commit also
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
   reflection appeared about 10.7 times. The table stores it per source now: a
   `ShcPrecompiledRow` points at a `ShcPrecompiledReflection`, and the generator
   refuses a source whose rows disagree on a field the source owns. Measured on
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
     options retired; no image changes behaviour) and the two dEQP fixes (an
     ES 2.0 surface refuses a DEPTH_COMPONENT24 renderbuffer and has no
     GL_MAX_DRAW_BUFFERS / GL_DRAW_BUFFERi query).
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
   - Docs: the plans and bring-up diaries in `driver/docs` and
     `docs/direct-boot.md` move to `tools/cts/log/` (the 35 code references
     are updated in the same commits); `docs/` and `driver/docs` then
     describe only the current design.
   - Remaining C moves to Rust: the OSALs (3,740 lines), `gl_test.c` (20,291
     lines: a Rust runner calls the C cases through `extern "C"`, ported in
     batches), `driver/src` (about 16k lines).
3. **Known defects.** Each has a GitHub issue; what remains is the fixes:
   - i.MX8MP gl_test case 10 fails about 1 run in 3 (nondeterminism; the
     cause is not found) (#9);
   - i.MX8MP case 16 (4x MSAA) fails (#8);
   - i.MX8MP gl_test case 2 (`texture`) fails intermittently since
     `05f8f749` (2 of 3 runs: 44 / 48 and 45 / 48 with case 2 first, 46 / 48
     with case 16 first; `112dd4fb`, without commits 4 and 5, 46 / 48) (#25).
     The command streams of 24 `cs_ours` modes are byte-identical between
     `72d16864` and `05f8f749` and the two images differ only in
     `record_key`, `gl_test_run` and the clear tables, so it is the
     nondeterminism class of #9, not a deterministic regression;
   - the Pi 4 differs from the ES 2.0 reference in 25 cube cases (#12);
   - the i.MX8MP panel showed one and a half copies of the frame per line at
     the 71.1 MHz pixel clock (no DSI margin); the device tree is back at
     69.3 MHz and the margin is a checked limit (`e372545f`); the DISPLAY
     record on `c1f0e2a6` shows the link at 69.3 MHz (lane rate 426600 kHz,
     headroom 25.9 per mille, bridge IRQ_STAT clean, health 0x3f); the
     viewing by eye is still pending (#23).

## How to work here

- The records prove the framebuffer, not the panel: a display change needs
  someone looking at the panel (#23).
- The gate is `tools/check.sh`. Run it from a clean detached checkout of
  the exact commit, with `driver/src/fw/pvr_fw_blob.h` copied in, and with
  `ZEPHYR_WS=<west workspace>` and `HAL_NXP_DIR=<hal_nxp>` set. Without
  them the Zephyr and hal_nxp rows are gaps and the gate is red. The
  Mesa-table reproduction row also needs `PCO_SHC=<pco_shc from
  tools/pco_shc/build_pco_shc.sh, content id 01aa62a069bb>`,
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
