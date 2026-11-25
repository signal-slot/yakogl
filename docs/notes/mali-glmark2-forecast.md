<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the expected glmark2-es2 refusals, by scene

Forecast of the glmark2-es2 run of the `gpu-mali` back end on the
NanoPC-T6 (issues #20, #7, #33), made on the host from the compiler's
verdicts, current at main `b08baac7` (2026-11-25). **16 of 33 scenes
compile-clean, 17 are refused.** Nothing here was rendered: a board run
confirms or refutes it. The board's record is `tools/cts/RESULTS.md`
(section "Mali: NanoPC-T6 glmark2-es2"); this note holds the design and the
causes, not the results of a session.

- **Verdicts**: `tests/fixtures/mali_glmark2_verdicts.txt`, pinned by
  `tests/mali_glmark2_corpus.rs` (the first refusal of every shader and
  program; the test fails if one moves back). The fixture last changed in
  `17a30a20`; no commit since touches it, `src/mali/**` or
  `src/shaderc/online/mali/**`.
- **glmark2** `22c527cb0556f3a1ac4445aaa52cc532760928d5` (2023.01), the
  commit `tools/hostharness/glmark2/run.sh` pins. Its shader corpus is
  GPL-3.0 text and is never committed.
- **Feature set** `MALI_CONFIG` of `tools/check.sh`
  (`host-test,gpu-mali,shaderc-online,no-shader-table`), plus `osal-linux`
  and `report-es2` for the harness build
  (`tools/hostharness/glmark2/mali/CMakeLists.txt`).
- **Scene numbers** are the entry order of glmark2's default list, as in the
  fixture and the harness.

## How it was made

1. **The compiler's view**: the fixture. Each scene lists the first refusal
   of each of its shaders (a stage gives its first refusal only: the
   compiler stops at the first gap) and, for the texture scenes, the failed
   link. A scene is compile-clean when every shader it uses compiles and
   every program it links builds; that says nothing about the picture.
2. **The glmark2 view**: the Mali host harness,
   `GM2_WRITE=0 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-mali mali`,
   run on all 33 scenes at `eacc70f5` (an ancestor of `b08baac7` with the
   same fixture and the same `src/mali`). It runs the GL core, the online
   compiler and the back end's command generation; no stream is executed.
   Its first refusal per scene agrees with the fixture. (The committed
   `tools/hostharness/glmark2/FINDINGS-mali-allscenes.md` still reads 14
   clean scenes; a run without `GM2_WRITE=0` rewrites it.)

## 1. By scene

`clean` = every shader compiles. The codes are section 2's.

| # | scene | verdict | first refusal per shader |
|---|---|---|---|
| 0, 1 | `build:use-vbo=false`, `=true` | **clean** | — |
| 2, 3, 4 | `texture:texture-filter=nearest`, `linear`, `mipmap` | refused | FS `425b` `TEX_COORD` (the link `ace2+425b` fails) |
| 5 | `shading:shading=gouraud` | **clean** | — |
| 6 | `shading:shading=blinn-phong-inf` | refused | FS `994f` `NONFINITE_CONST` |
| 7 | `shading:shading=phong` | refused | FS `8a64` `ARITH_CONST` |
| 8 | `shading:shading=cel` | refused | FS `fea0` `NONFINITE_CONST` |
| 9 | `bump:bump-render=high-poly` | refused | FS `d442` `NONFINITE_CONST` |
| 10 | `bump:bump-render=normals` | refused | FS `487d` `NONFINITE_CONST` |
| 11 | `bump:bump-render=height` | refused | FS `77c5` `NONFINITE_CONST` |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` (3x3) | refused | FS `3c3d` `TEX_MULTI` |
| 13 | `effect2d:kernel=1,1,1,1,1;…` (5x5) | **clean** | — (`ba8e` compiles) |
| 14 | `pulsar:quads=5:texture=false:light=false` | **clean** | — |
| 15 | `desktop:…:effect=blur:…` | **clean** | — (`68af`, `7f17` compile) |
| 16 | `desktop:windows=4:effect=shadow` | **clean** | — |
| 17, 18, 19 | `buffer:…` (map, subdata, map with interleave) | **clean** | — |
| 20 | `ideas:speed=duration` | refused | VS `51b2` `CONTROL_FLOW`; FS `1aca`, `c565` `INT_VALUE` |
| 21 | `jellyfish` | refused | VS `08a2` `ARITH_CONST`; FS `8ee5` `FS_OPERAND` |
| 22 | `terrain` | refused | FS `1637`, `4d26`, `c549` `ARITH`; FS `8c86`, `bbba`, `bdfd`, `e043` `TEX_MULTI` |
| 23 | `shadow` | refused | FS `cf4b` `ARITH` |
| 24 | `refract` | refused | FS `3fae` `ARITH_CONST` |
| 25, 26, 27 | `conditionals:…` (0/0, 0/5, 5/0 steps) | **clean** | — |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | **clean** | — |
| 29 | `function:…:fragment-complexity=medium` | refused | FS `849c` `FRAGCOORD_FP16` |
| 30 | `loop:…:fragment-loop=false` | **clean** | — |
| 31 | `loop:…:fragment-uniform=false` | **clean** | — |
| 32 | `loop:…:fragment-uniform=true` | refused | FS `3678` `LOOP_FP16` |

**Compile-clean (16)**: 0, 1, 5, 13, 14, 15, 16, 17, 18, 19, 25, 26, 27, 28,
30, 31. **Refused (17)**: 2, 3, 4, 6, 7, 8, 9, 10, 11, 12, 20, 21, 22, 23,
24, 29, 32. Shaders not named in a row compile (scene 20's `df64` among
them). Against the previous measurement (14 clean at `104748bb`) scenes 13
and 15 moved to clean; no shader moved back.

On the board the 16 clean scenes are the only ones that put a command
stream of their own on the GPU (17-19 little more than their clears). The
17 refused scenes stop in glmark2's setup at `Failed to add vertex|fragment
shader`, draw nothing and cannot fault the GPU.

## 2. The causes still open

The message is `MaliError::message`'s text
(`src/shaderc/online/mali/mod.rs`). **Alone**: scenes whose only remaining
family it is, so lowering it alone makes them clean. **Item**: the queued
lowering item that covers it (named in plan #367's queue, continued by plans
#402, #387 and #400); where none is named the cell says so.

| code | message | shaders | scenes | alone | item |
|---|---|---|---|---:|---|
| `NONFINITE_CONST` | a constant operation on or to a value that is not a finite normal float is not lowered yet | FS `994f` `fea0` `d442` `487d` `77c5` | 6, 8, 9, 10, 11 | 5 | `lower-nonfinite-const`; no #402/#387/#400 item names it |
| `TEX_COORD` | a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet | FS `425b` | 2, 3, 4 | 3 | `lower-texcoord-425b` |
| `TEX_MULTI` | a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet | FS `3c3d` `8c86` `bbba` `bdfd` `e043` | 12, 22 | 1 (12) | `lower-fma` |
| `ARITH_CONST` | a constant product in a shape Mesa's algebraic rules move or fold is not lowered yet | FS `8a64` `3fae`; VS `08a2` | 7, 21, 24 | 2 (7, 24) | `lower-constprod-rest` |
| `ARITH` | arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet | FS `cf4b` `1637` `4d26` `c549` | 22, 23 | 1 (23) | `lower-fs-arith2`; none named for `cf4b` |
| `INT_VALUE` | an integer or boolean value is not lowered yet | FS `1aca` `c565` | 20 | 0 | `lower-ideas` |
| `CONTROL_FLOW` | control flow (if, a loop, a return before the end) is not lowered yet | VS `51b2` | 20 | 0 | `lower-ideas` |
| `FS_OPERAND` | a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet | FS `8ee5` | 21 | 0 | none queued |
| `FRAGCOORD_FP16` | fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y is not lowered yet | FS `849c` | 29 | 1 | `lower-mploop` |
| `LOOP_FP16` | a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge) is not lowered yet | FS `3678` | 32 | 1 | `lower-mploop` |

Three scenes need more than one family: 20 needs `INT_VALUE` and
`CONTROL_FLOW`; 21 needs `ARITH_CONST` (`08a2`) and `FS_OPERAND`; 22 needs
`ARITH` and `TEX_MULTI`.

**Order of scenes unblocked** (greedy; this is the arithmetic of the table,
not a measurement: a lowering can uncover a later refusal behind the first):

| step | family | scenes made clean | clean after |
|---:|---|---|---:|
| — | now (`b08baac7`) | 16 scenes | 16 |
| 1 | `NONFINITE_CONST` | 6, 8, 9, 10, 11 | 21 |
| 2 | `TEX_COORD` `425b` | 2, 3, 4 | 24 |
| 3 | `TEX_MULTI` | 12 | 25 |
| 4 | `ARITH_CONST` | 7, 24 | 27 |
| 5 | `ARITH` (`cf4b`) | 23 | 28 |
| 6 | `FRAGCOORD_FP16`, `LOOP_FP16` | 29, 32 | 30 |
| 7 | `INT_VALUE`, `CONTROL_FLOW` | 20 | 31 |
| 8 | `FS_OPERAND`, `ARITH_CONST` `08a2` | 21 | 32 |
| 9 | `ARITH`, `TEX_MULTI` | 22 (`terrain`) | 33 |

## 3. What the host harness shows past the compiler

From the all-scenes run at `eacc70f5` (section "How it was made"):

- **No draw was refused by the back end** in any scene, no draw sampled an
  incomplete texture, no framebuffer was incomplete, no GPU memory ran out
  and no submit was refused by the OSAL mock. The depth-only framebuffer of
  `desktop` shadow (16, 107 draws) is complete: `D-DEPTHONLY` is closed on
  the host (`DRAWS_WITHOUT_COLOUR`, `docs/notes/mali-depth-only.md`).
- **GL errors: none** in the 16 clean scenes. Scene 15 (`desktop` blur) ran
  3 frames and 56 draws with 0 errors now that its blur shaders compile
  (it raised 352 while they were refused). Scene 5 prints one
  `no uniform 'ModelViewMatrix'` line: a glmark2 look-up of a name the
  program lacks, answered with location -1 and no GL error.
- **The 17 refused scenes** fail at setup: glmark2 prints
  `Error: Failed to add vertex|fragment shader from file None:` and the
  compile log, and exits with status 3 (2-4 report the failed link of
  `ace2+425b`). Any GL error such a scene would raise is ES 2.0's answer to
  an unlinked program, not a defect of the GL core.
- **terrain (22) crashes in glmark2 itself** (SIGSEGV, rc 139, no scene-end
  line) after seven fragment shaders are refused (`1637`, `c549`, `4d26`
  `ARITH`; `e043`, `bdfd`, `bbba`, `8c86` `TEX_MULTI`). In
  `TerrainRenderer::init_program()` (`src/scene-terrain/terrain-renderer.cpp`)
  `if (!Scene::load_shaders_from_strings(...)) return;` goes on with the
  unlinked program, and `TerrainRenderer::render` reaches
  `Mesh::render_vbo` with -1 attribute locations as an index. On the board
  (main `da65355e`) libstdc++'s `std::vector::operator[]` assertion made it
  SIGABRT (rc 134); the host reads out of range and gets signal 11. The
  refused program is the cause; the crash goes with scene 22's two families
  (step 9).
- **Not reached on the host** because no scene gets there: `D-NPOTMIP`
  (refract's 1600x1200 colour target keeps one mip level,
  `npot_mip_chains` in `src/gl/texture.rs`: a picture risk, not a refusal)
  and `D-DEPTHTEX` (the depth texture `refract` and `shadow` sample goes
  through `mali_depth_resolve` to R32F; gl_test case 169 is the only board
  evidence). Both stay forecasts until scenes 23 and 24 compile. No
  texture-format or blend refusal is expected (unsized `GL_RGB`/`GL_RGBA`
  `GL_UNSIGNED_BYTE`, float32 attributes, fixed-function blends in
  `src/mali/blend.rs`).

## 4. Risks the host cannot see

- No command stream is executed: a GPU fault, hang or wrong picture is the
  board run's to find, and the 16 clean scenes are where it can happen:
  `pulsar` (15 draws), `desktop` blur and shadow (800x600 RGBA8 FBO
  textures sampled and blended with fixed function; 56 and 107 draws), the
  `loop` and `function` scenes.
- Once the loop families are lowered, `loop:…:fragment-uniform=true`
  (`3678`) runs a uniform-bounded fragment loop, the hang-risk class of
  `docs/notes/mali-gles2-forecast.md` section 3. `ideas` (540 draws) and
  `terrain` (nine passes into FBOs) are where the frame arenas and
  `SLOT_STILL_IN_FLIGHT` show first.
- Behind a first refusal the next may come: the lit shaders that call
  `reflect` or `mix` may meet `ARITH` again, and `if` steps the
  control-flow family (`docs/notes/mali-control-flow.md`).

## 5. Where each cause belongs

The compiler causes are #33 (`TEX_COORD` also #32). Three first refusals have
no lowering item of their own: `NONFINITE_CONST` (scenes 6, 8-11),
`FS_OPERAND` `8ee5` (21) and `ARITH` `cf4b` (23). `D-NPOTMIP` and
`D-DEPTHTEX` are #32 if a board picture shows them.

## 6. How to redo it

Run `GM2_WRITE=0 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-mali mali`
(without `GM2_WRITE=0` it rewrites `FINDINGS-mali-allscenes.md`). The
per-shader table is
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_corpus -- --nocapture`.
Recheck when `src/shaderc/online/mali/**`, `src/mali/backend.rs`
(`DRAWS_WITHOUT_COLOUR`, `MIP_LEVEL_ALIGN`) or `src/gl/texture.rs`
(`npot_mip_chains`) move.

## 7. Verdin iMX95 (Mali-G310)

Forecast of glmark2-es2 on the Verdin iMX95 (issue #7, B-items of
`docs/notes/pre-study-imx95.md` section 8), current at main `d6646434`
(2026-11-25). **Method: reading files and arithmetic on the host. Nothing
was built for, run on or measured on the board, and no scene was
rendered.** The compile verdicts of sections 1 and 2 carry over: the
iMX95 runs the same in-process compiler (`src/shaderc/online/mali`) on arch
major 10 as the G610, so the 16 clean and 17 refused scenes are the same
list; only frame times and memory differ.

### 7.1 Two build gaps in the Zephyr sample

`driver/samples/zephyr/glmark2` (the Zephyr image; not
`tools/cts/mali-glmark2-build.sh`, which builds the T6's Linux
`glmark2-es2-mali` over panthor and has no iMX95 arm) is not set up for
`verdin_imx95`. Two things are missing (found by reading the files; no
`west build` was run):

1. **`driver/samples/zephyr/glmark2/CMakeLists.txt`** has the board
   includes for the STM32MP257F-DK (`^stm32mp257f_dk`) and the T6
   (`^nanopc_t6`) but none for the iMX95. The dEQP sample has it
   (`driver/samples/zephyr/deqp/CMakeLists.txt:39`):
   `include("${CMAKE_CURRENT_SOURCE_DIR}/../common/verdin_imx95.cmake")`,
   before `find_package(Zephyr)`. That file puts the board layer's
   `driver/zephyr/lib/board/verdin_imx95/app.conf` and the sample's
   `verdin_imx95.conf` into `EXTRA_CONF_FILE` (in that order) and the
   board's `app.overlay` into `EXTRA_DTC_OVERLAY_FILE`, and stops with
   `FATAL_ERROR` if the sample has no `verdin_imx95.conf`. Without the
   include the image has no Mali board layer and no overlay.
2. **`driver/samples/zephyr/glmark2/verdin_imx95.conf`** (new, beside the
   `CMakeLists.txt`, modelled on `driver/samples/zephyr/deqp/verdin_imx95.conf`).
   It is not a `boards/verdin_imx95*.conf`: Zephyr merges that before
   `app.conf`, which would win over it (`common/verdin_imx95.cmake`). It
   needs `CONFIG_YAKOGL_MALI_CARVEOUT_BYTES` for glmark2 (7.2),
   `CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY=n` for a read-back first run (the
   panel is B6), and a lifetime and hold for 33 scenes in one image
   (`boards/nanopc_t6.conf` uses 900 s and 20 s; the dEQP file's 600 s and
   0 are per row), with `ALIVE_S=60` and `HOLD_FAIL_S` as the dEQP file.
   `prj.conf` is shared and already sets the 96 MiB libc heap
   (`CONFIG_COMMON_LIBC_MALLOC_ARENA_SIZE=100663296`).

### 7.2 Memory: the GL carve-out stays at 94 MiB or less

The board's RAM is the 256 MiB window at `0xd0000000` (`zephyr,sram`,
`driver/samples/zephyr/deqp/README.md`, "Memory"). The carve-out is
`.noinit`, so it counts with the image. Two differences from the T6:

- On the iMX95 `CONFIG_YAKOGL_MALI_CARVEOUT_BYTES` **includes** the board
  layer's window memory, `CONFIG_YAKOGL_MALI_WINDOW_BYTES` 9,011,200
  (8800 KiB; `driver/zephyr/lib/board/verdin_imx95/app.conf`: 32 MiB +
  9,011,200 = 42,565,632). The dEQP file adds it by hand
  (`67108864 + 9011200 = 76120064`). The T6's layer has no window, and its
  100,663,296 is all GL memory.
- The T6's glmark2 image is 261,111,808 bytes of 268,435,456
  (`boards/nanopc_t6.conf`). Without its 96 MiB heap and 96 MiB carve-out
  that is 59,785,216 bytes (57 MiB) of code, data, assets and bss. Assuming
  the iMX95 image is no larger (same sources and Mali back end; its board
  layer is the open figure): 59,785,216 + 100,663,296 (heap) + GL +
  9,011,200 <= 268,435,456 gives **GL <= 98,975,744 bytes, 94.4 MiB**.

So the GL part is **94 MiB (98,566,144) at most**, i.e.
`CONFIG_YAKOGL_MALI_CARVEOUT_BYTES` <= 107,577,344 with the heap at 96 MiB,
which leaves 409,600 bytes. **Suggested first value: 92 MiB of GL**
(`96,468,992 + 9,011,200 = 105,480,192`, 2.5 MiB left); a build's AArch64
header `image_size` then decides. The scenes need less: refract peaks at
about 56 MB and terrain needs over 30 MB (`boards/rpi_4b.conf`), build and
shading 18,932 KiB at most (T6 board run, 2026-11-23). The dEQP image's
64 MiB would also hold them by those figures, with less room for refract's
and shadow's 1600x1200 targets. Nobody has measured glmark2's heap use on
the iMX95, so do not lower the heap on this note.

### 7.3 Per-scene frame time against the T6

**The T6 baseline is thin.** Its recorded timings are the Linux pbuffer
run of 2026-11-11 (`tools/cts/RESULTS.md`, "Mali: NanoPC-T6 glmark2-es2"),
where 31 scenes were refused: desktop shadow 0.96 ms (1042 FPS) is the one
scene that compiled and drew; blur (1.15 ms) and `buffer` (0.36-0.71 ms)
ran on after refusals, so their times are not shader costs. The Zephyr T6
record (score 119, two scenes) is flip-bound, not GPU speed. Every figure
below is therefore a scaling rule applied to that one number and the
shader mix, not a prediction from a per-scene T6 measurement.

| | T6 (G610) | Verdin iMX95 (G310) | factor |
|---|---|---|---|
| shader cores | 4 (`shader_present 0x50005`, `mali-g310-vs-g610.md`) | 1 (`SHADER_PRESENT 0x1`) | 4 |
| GPU clock | 850 MHz (devfreq, 2026-11-11; 300 MHz on 2026-10-19; `driver/docs/nanopc-t6-mali.md`) | 500, 800 or 1000 MHz by SCMI performance level; the sequence of `imx95-gpu-power.md` selects index 2 = 800 MHz | 1.06 at 800 |
| per-core rates | G610 row of the model table | G310 row (`mali-g310-vs-g610.md`, table line 54) | not compared here |

- **GPU-bound scenes**: about 4 x 1.06 = **4.2x the T6's time at 800 MHz**
  (3.4x at 1000 MHz, 6.8x at 500 MHz), assuming one core and the same
  per-core pipelines.
- **CPU/driver-bound scenes** (small meshes, many draws: `build`,
  `buffer`, `ideas`, `pulsar`): bound by command generation and submit on
  an A55 under Zephyr against the T6's A76 on Linux. No A55 figure exists
  for this code; expect **1x to 3x**, not 4x.
- **Desktop shadow** (0.96 ms): about 4 ms (250 FPS) if GPU-bound, 1 to 3
  ms if CPU-bound. Either is below 16.7 ms, so with the panel (B6) the
  score is the display's flip rule, as on the T6.

| scenes | class | iMX95 vs T6 |
|---|---|---|
| 0, 1 `build`; 5 `shading=gouraud` | CPU/driver, low pixel cost | 1-3x |
| 13 `effect2d` 5x5, 14 `pulsar`, 17-19 `buffer` | CPU / upload bound | 1-3x |
| 15 blur, 16 shadow | fill and FBO (800x600), 56 and 107 draws | 3-4x |
| 25-27 `conditionals`, 28 `function` low, 30, 31 `loop` | full-screen fragment ALU | about 4.2x |
| the 17 refused scenes | stop in setup, as on the T6 | no frame time |

Once the compiler lowering (section 2) lets them compile, `terrain`,
`refract`, `shadow` and `jellyfish` would be the slowest: about 4.2x plus
whatever the CPU resolve of the 1600x1200 depth costs (`D-NPOTMIP`,
`D-DEPTHTEX`, section 3), possibly hundreds of ms a frame on an A55.

### 7.4 Scenes most at risk

- **Memory** (7.2): `refract` (1600x1200 colour target plus depth texture,
  about 56 MB), `terrain` (nine FBO passes, over 30 MB) and `shadow`
  (1600x1200 depth). They fit 92-94 MiB by those figures; `refract`'s
  out-of-memory refusal on the Pi 4 at 48 MiB (`boards/rpi_4b.conf`) is the
  precedent for a carve-out set too low. Watch `terrain` and `refract`.
- **Tile buffer budget**: `mali-g310-vs-g610.md` section 5 says `TIB_BUDGET`
  and `MEASURED_TILER_LEVELS` are gone (the budget comes from the GPU's
  `CORE_FEATURES` and `TILER_FEATURES`); the plan text that still names them
  as G610 constants is out of date against that note. On a G310v1-v3 the
  halved tile buffer matters only for 12 bytes a pixel or more (3-4 targets
  or 4x multisampling); at one sample the Z bound is 1024 pixels against
  2048 on the G610, never below the 1024 clamp. glmark2 draws one RGBA8
  target plus depth at 1x, so **no scene is expected to reach it**. The
  board's G310 variant is unread, so this stays a forecast until B2's
  `CORE_FEATURES` is known.
- **FBO and depth scenes** (`desktop` blur and shadow, `shadow`, `refract`,
  `terrain`) put second render passes and tile-size choices on the G310:
  where a G310-only difference would show first.
- **`GL_RENDERER`** reads `Arm Mali-G310vN (YakoGL)`; a record names it.
- **Time**: one core makes long scenes about four times longer; 900 s must
  cover 33 scenes at glmark2's default 10 s each plus setup.

### 7.5 What it does not say

No image was built, no memory figure measured on the iMX95, no scene ran,
and the G310's variant is unread. The T6 has no frame times for the 16
clean scenes yet, so the multipliers have only desktop shadow to act on.
Redo this section from a build's `image_size` and one `-b` run per class
once B6 and the T6's lowering allow it.
