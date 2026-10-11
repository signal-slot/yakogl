<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Handoff

The state of the work and what comes next. Update this file in the same
commit as any result that changes it. Last update: 2026-10-07.

Board addresses, UART paths and credentials never go in this file; they come
from the environment (`BOARD_HOST`, `BOARD_UART`, ...).

## Goal, in order

1. ES 2.0 finished on the three boards (AM62P PowerVR, i.MX8MP GC7000,
   Raspberry Pi 4 V3D), each booted from its own storage by its bootloader.
2. Minimum performance.
3. A public website (`website/`). It goes live when the repository is public.
4. ES 3.1 / 3.2.

## State of main

**Plan #1 (ES 2.0 CTS, zero failures on the three boards), 2026-10-07.** Every
fix below landed on main and is proved on the host only. No board has run any
of these commits, and no dEQP tree has run since the last full trees
(AM62P `d60b9d76` 10/2: 17,141 pass / 0 fail / 19 NotSupported / 5 warnings;
i.MX8MP `d8583f20` 10/1: 0 fail; Pi 4 `fe5e8e73` 10/1: 25 fail, all cube).
The inventory with every failure's state and commit is
`docs/es2-cts-failures.md`.

| Failure | State on main | Commit |
|---|---|---|
| Pi 4, 25 cube cases (#12) | fixed on the host: non-seamless cube sampling in ES 2.0 builds (a cube look-up is a 2D-array look-up of the selected face; the V3D has no switch for seamless filtering). The owner's 2026-09-25 exclusion of these cases is replaced by this fix. Design: `docs/v3d-noseamless-cube.md` | `c3e4146` design and host reproduction, `ec5ba4f` implementation, `53ef6d7`, `d8ce931` |
| i.MX8MP, the two dEQP regressions of `72d16864` (fbo `depth_component24`; one `shaders.fragdata.*` NotSupported) | fixed on the host: in `report-es2` builds `glRenderbufferStorage(GL_DEPTH_COMPONENT24)` raises GL_INVALID_ENUM unless `GL_OES_depth24` is listed (`state::extension_listed`); `GL_MAX_DRAW_BUFFERS` and `GL_DRAW_BUFFERi` are refused and `gl_MaxDrawBuffers` is 1. It should also turn `shaders.fragdata.invalid_assign_to_1` from NotSupported into a pass on the AM62P and the Pi 4 (a prediction). Not done: other ES 3-only renderbuffer formats such as `GL_DEPTH_COMPONENT32F` are still accepted in ES 2.0 builds | `4e9397c` |
| i.MX8MP gl_test case 16 (#8) | harness fixed: it failed in the default image only because that image offers no 4x config (`check 255` is the record's none-marker, not a check). Case 16 runs its 1x half and skips the 4x half by name without a 4x config. The driver did not scale viewport, scissor and clip to the 2x scratch surface: fixed behind `gc_debug_msaa` bit 3, off by default until a board proves it | `6bddfc5` |
| i.MX8MP gl_test cases 10 and 2 (#9, #25) | not fixed. Reading our draw, resolve and read-back sync against etnaviv's finds no missing stall, flush or cache operation (`src/vivante/VIVANTE_NOTES.md`; two host tests pin it). The product is unchanged; the board recipe below separates the three remaining hypotheses | `8409419` |
| PowerVR vertex attribute registers | wrong-code defect found by the host compiler check and fixed: an attribute read only where the unroller drops code took no registers, and every later attribute was read four registers early. No dEQP-GLES2 case is known to reach it; it changes AM62P vertex code, so the AM62P tree is re-run | `c6aa6ba` |
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
   - Docs: the plans and bring-up diaries in `driver/docs` and
     `docs/direct-boot.md` move to `tools/cts/log/` (the 35 code references
     are updated in the same commits); `docs/` and `driver/docs` then
     describe only the current design.
   - Remaining C moves to Rust: the OSALs (3,740 lines), `gl_test.c` (20,291
     lines: a Rust runner calls the C cases through `extern "C"`, ported in
     batches), `driver/src` (about 16k lines).
3. **Known defects.** Each has a GitHub issue; what remains is the fixes.
   State as of 2026-10-07; none is confirmed on a board:
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
     alignment, pool placement that depends on history). "Next board
     session" runs 10 boots each of a base, a page-aligned and a no-reuse
     image and reads which one stops failing;
   - i.MX8MP case 16 (4x MSAA) (#8). **New state (`6bddfc5`):** the "failure"
     of the default image was the missing 4x config; the harness now skips
     the 4x half by name. The 4x path itself is untested on a board; the
     viewport/clip scaling to the 2x scratch is behind `gc_debug_msaa` bit 3
     (off), to be proved by the A/B in "Next board session". The 16
     `multisample.*` dEQP cases stay NotSupported at 1x;
   - the Pi 4 differs from the ES 2.0 reference in 25 cube cases (#12).
     **New state (`ec5ba4f`): fixed on the host. The owner's 2026-09-25
     exclusion of these cases from the pass count is replaced by this fix.**
     It needs the Pi 4 run (T1, T3, TF);
   - the i.MX8MP panel showed one and a half copies of the frame per line at
     the 71.1 MHz pixel clock (no DSI margin); the device tree is back at
     69.3 MHz and the margin is a checked limit (`e372545f`); the DISPLAY
     record on `c1f0e2a6` shows the link at 69.3 MHz (lane rate 426600 kHz,
     headroom 25.9 per mille, bridge IRQ_STAT clean, health 0x3f); the
     viewing by eye is still pending (#23).

## Next board session

One session per board confirms every fix that waits for hardware. Nothing
below has been run. Expected values come from the host tests and the
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

### i.MX8MP (Verdin, no UART: records are read from the SNVS)

Confirms `4e9397c` (fbo, fragdata), `6bddfc5` (case 16 and the bit 3 A/B), and
measures #9 and #25. Do steps 1 to 3 first (short), then 4 and 5.

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

   Expected: case 16 is not a failure any more and is no longer the first
   failure (without a 4x config it runs its 1x half and reports a by-name
   skip, -3). Cases 2 and 10 may still flip (#25, #9): record which runs fail
   and at which case. The earlier results (44, 45, 46 of 48 at `05f8f749`; 46
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

Confirms `ec5ba4f` (the 25 cube cases), `4e9397c` (fragdata
`invalid_assign_to_1`) and the one open question of the inventory (the 27 fbo
`attachment_combinations`). Needs `RUSTUP_TOOLCHAIN=nightly`, stage 1 for
`rpi_4b` (`DEQP_WORK`) and `BOARD_UART`, `BOARD_HDMI_TIMINGS` as the lab's.
Rows of `pi4-full-tree.txt` (`TREE_ROWS` counts from 0, over the non-comment
lines): D 1 (`shaders.fragdata.*`), FB1 4 (`fbo.api`, `fbo.completeness`), T1 19, T3 21,
TF 24.

```sh
export DEQP_WORK=<stage 1 for rpi_4b> RUSTUP_TOOLCHAIN=nightly
$SW/tools/cts/pi4-deqp-images.sh $SW/tools/cts/pi4-full-tree.txt <img> D FB1 T1 T3 TF     # <img> must not exist yet
TREE_IMAGE_DIR=<img> TREE_ROWS=1,4,19,21,24 $SW/tools/board.sh pi4 tree $SW/tools/cts/pi4-full-tree.txt <rec>/cube
$SW/tools/board.sh pi4 tally <rec>/cube
$SW/tools/cts/common-images.sh pi4 $SW/tools/cts/common-tree.txt <img-gt> GT              # v3d_gl_test
$SW/tools/board.sh pi4 run <img-gt>/GT/zephyr/zephyr.bin <rec>/gt gt-$HEAD8
```

Expected (`docs/v3d-noseamless-cube.md`, "Pi 4 verification"):

- the 25 cases pass: T1 `texture.vertex.cube.filtering.*` 10 and
  `texture.vertex.cube.wrap.*` 8, T3
  `texture.mipmap.cube.{basic,bias,projected}.{linear_linear,linear_nearest}`
  6, TF `shaders.texture_functions.vertex.texturecubelod` 1. T1, T3 and TF
  each equal their sweep-30 tally (RESULTS.md "Pi 4: sweep 30"; T1 had 18
  fail, T3 6, TF 1) with those cases moved from Fail to Pass, and no other
  case moves;
- D: `shaders.fragdata.invalid_assign_to_1` NotSupported to Pass (a
  prediction from `4e9397c`, not a measurement); everything else in D as at
  sweep 30;
- FB1: the 27 stencil-texture `attachment_combinations` cases were
  NotSupported at sweep 30 because the V3D did not claim the depth-texture
  extensions; it claims `GL_OES_depth_texture` since 10/3, so they should
  pass. If they still fail the stencil attachment, that is a V3D driver fix
  outside this plan's list;
- whole tree, if all 29 images run (`TREE_PREV=<sweep 30's tree>`): 17,100
  pass / 0 fail / 62 NotSupported / 3 QualityWarning from the cube fix alone;
  the fragdata case and the fbo 27 may move up to 28 more NotSupported to
  pass;
- `v3d_gl_test` unchanged, 21 / 21 (12 / 12 on the older record): its cube
  cases use NEAREST, which the GL core never keys.

Reading a failure (from the design document):

- all 25 fail: the TMU did not take TMUI as a float face index. Add one
  FTOIN of the face in `prepare.rs` (the caller of `Walk::cube_face`),
  rebuild, run T1 `texture.vertex.cube.wrap.clamp_clamp` alone;
- only the six `texture.vertex.cube.filtering.nearest_mipmap_{nearest,
  linear}_linear_*` fail: the c = 0.5 switch is wrong for the TMU. The remap
  is wired (`LodKey::mag_switch` in the vertex key); first check that the
  vertex key reaches the link (`V3D_VARIANTS_BUILT` in the back end's print
  grows);
- only the six `texture.mipmap.cube.*` fail, with invalid pixels along face
  edges: the fragment stage's implicit level needs the FDX / FDY fallback
  (design document, "The level of detail"); write it in `prepare.rs` for
  fragment variants only.

### AM62P (`BOARD_BOOT=uboot-tftp` or `uboot-mmc`; `BOARD_UART` set; `driver/src/fw/pvr_fw_blob.h` in the checkout)

Confirms that nothing moved after the PowerVR compiler changes since the
10/2 tree (the fusion and predication commits listed in
`docs/es2-cts-failures.md` section D, and `c6aa6ba`'s vertex attribute fix),
and that `4e9397c` turns `fragdata.invalid_assign_to_1` into a pass. The
whole GLES2 tree is re-run, since it last ran on `d60b9d76`:

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

Expected: combined mustpass 0 fail, the `fail` column of all 17 image rows 0
and of `ms4` 0, no case worse than at `d60b9d76` (17,141 pass / 0 fail / 19
NotSupported / 5 warnings of 17,165; RESULTS 1z2). If fragdata
`invalid_assign_to_1` passes (image 1, the table's first row, `TREE_ROWS=0`), that is 17,142
pass and 18 NotSupported. A case that moves from Pass to anything else is a
regression of the compiler changes since `d60b9d76`: bisect it first. If time
is short, run image 1 alone (`TREE_ROWS=0`) for the fragdata check and
images 1 to 7 (the shaders groups, `TREE_ROWS=0-6`) for the compiler. Time: at most about 11
min an image, 18 images.

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
  `ZEPHYR_WS=<west workspace>` and `HAL_NXP_DIR=<hal_nxp>` set. Without
  them the Zephyr and hal_nxp rows print `skipped: <reason>` and are
  counted in the summary; the gate is clean with them, and
  `YAKOGL_CHECK_STRICT=1` makes each skip fail (use it for this run). The
  Mesa-table reproduction row also needs `PCO_SHC=<pco_shc from
  tools/pco_shc/build_pco_shc.sh, content id 01aa62a069bb>`,
  `GLSLANG=<glslangValidator>` and, for speed, `GLSL2USC_CACHE=<dir>`;
  without them that row is skipped the same way. It takes about 45
  minutes.
- Board runs use `tools/board.sh <board> run <zephyr.bin> <record dir>
  <label>`. Run them from a detached checkout of main, and only with images
  built from commits on origin/main. Use `BOARD_BOOT=uboot-mmc` on the
  Verdins.
- A dEQP image bakes its table in: splitting rows needs a rebuilt image
  (`tools/cts/imx8mp-deqp-build.sh --table`; tools/cts/README.md).
- One GL client per board. Check that a board is idle before you use it.
