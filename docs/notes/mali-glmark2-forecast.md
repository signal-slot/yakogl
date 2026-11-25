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
