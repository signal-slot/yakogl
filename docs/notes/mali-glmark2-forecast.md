<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the expected glmark2-es2 refusals, by scene

Date 2026-11-09. Plan #332 item `forecast`, for the first glmark2-es2 run of
the `gpu-mali` back end on the NanoPC-T6 (issues #20 and #7). It is a
forecast made on the host before any board run. Nothing was rendered, and
`gh` was not run.

**Board record** (added 2026-11-10): the first board run, main `da65355e`
(`tools/cts/RESULTS.md`, section "Mali: NanoPC-T6 glmark2-es2 (stage 2
criterion, first run)", landed in `08ae7a8e`), gave the per-scene outcomes
this note forecasts, except terrain's signal: SIGABRT (rc 134, a libstdc++
assertion) on the board, not SIGSEGV (section 1, row 22). `D-DEPTHONLY` and
`D-NPOTMIP` were not reached (section 2).

**Refreshed** (2026-11-21, plan #367 item `forecast-refresh`): sections 1 and 2
are re-measured at main `104748bb`: **14 of 33 scenes compile-clean**.
Sections 3 and 4 are as of `d09629d3`.

**Re-measured** (2026-11-14, plan #367 item `remeasure`) at main `d09629d3`
(main `a92775f3` differs from it in no file a Mali compile reaches):
sections 1-3 are that measurement, not the forecast of `95f20610`. View 2
is now a committed test, `tests/mali_glmark2_corpus.rs`, which pins the
first refusal of every shader and program in
`tests/fixtures/mali_glmark2_verdicts.txt` (section 1's table is that
fixture) and fails if one moves back. View 1 rewrote
`tools/hostharness/glmark2/FINDINGS-mali-allscenes.md`, and its first
refusal per scene agrees with the test's. Result: **2 of 33 scenes
compile-clean** (14 `pulsar`, 16 `desktop` shadow), 23 of the 69 corpus
shaders compile, and all 7 programs whose two stages compile link.

- **Made at** main `95f20610`; sections 1-3 re-measured at `d09629d3`.
  When `src/mali/**`, `src/shaderc/online/**` or `src/gl/**` move, the
  causes move with them (section 5).
- **glmark2** `22c527cb0556f3a1ac4445aaa52cc532760928d5` (2023.01), the
  commit `tools/hostharness/glmark2/run.sh` pins. Its shader corpus is
  GPL-3.0 text and is never committed.
- **Feature set** `MALI_CONFIG` of `tools/check.sh`
  (`host-test,gpu-mali,shaderc-online,no-shader-table`), plus `osal-linux`
  and `report-es2` for the harness build (`tools/hostharness/glmark2/mali/CMakeLists.txt`).

## How it was made

Two views, both on the host:

1. **Per scene, as glmark2 runs it**: the Mali host harness,
   `tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-mali mali`.
   It runs glmark2's 33 default entries against the `gpu-mali` build (GL
   core, the online compiler, the back end's command generation; no stream
   is executed), one process per entry. At `95f20610` it rewrote
   `tools/hostharness/glmark2/FINDINGS-mali-allscenes.md` byte for byte as
   committed (made at `c6b16f71`), so that file's per-scene table and
   findings are this forecast's record of the first refusal per scene.
2. **Per program, both stages**: glmark2 stops a scene at the first shader
   it cannot add, so view 1 shows one refusal per scene. To see what is
   behind it, the 48 distinct programs glmark2 links (read from a PowerVR
   run's logs: `run.sh "$GLMARK2_SRC" build/glmark2-host powervr`, in each
   `logs/powervr/<n>.log` a vertex shader's `gm2: shader` line followed by
   its fragment shader's) were each built from the corpus the way
   glLinkProgram builds them on the Mali (`src/gl/program.rs`
   `glLinkProgram`): both compiles, `compile_vs_linked` with no map, then the
   link, with `SHC_ONLINE` in a scratch test of the `MALI_CONFIG` build (not
   committed: it adds a test binary to every Mali row for a report that
   never fails). Result: **2 ok, 46 refused, 0 error**. The two that build are
   `desktop`'s `792b+9ce9` and `terrain`'s `8b69+895a`. Rechecked at
   `95f20610`: the 69 corpus files' compile verdicts
   (`tools/mali/corpus-verdicts.sh` over a scratch copy of the corpus under
   `tools/shaders/`, removed after) are the ones section 2 counts, and view
   1's logs have no link failure, so both pairs that compile still link.

What it can and cannot see:

- Each stage gives its **first** refusal only (the compiler stops at the
  first gap). An unblocked stage may hit the next family (section 3).
- The link-time rules (a partly read varying, constant varying lanes) run
  only when both stages compile, so for 46 programs they are not reached.
- No command stream is executed: a GPU fault, hang or wrong picture is the
  board run's to find (section 2, risks).

## 1. By scene

Measured at main `104748bb` (2026-11-21; the verdicts are
`tests/fixtures/mali_glmark2_verdicts.txt` at that commit, and the corpus is
the one generated from glmark2 `22c527cb`). Scene numbers are the harness's
(`FINDINGS-mali-allscenes.md`, entry order of glmark2's default list). Per
scene: `clean`, or the first refusal of each shader that is refused, by
family (section 2's table has the messages). A scene is compile-clean when
every one of its shaders compiles and every program it links builds; it
says nothing about the picture. **Items** are the queued lowering items
that cover the refusal (section 2).

| # | scene | verdict | first refusal per shader | items |
|---|---|---|---|---|
| 0 | `build:use-vbo=false` | **clean** | — | — |
| 1 | `build:use-vbo=true` | **clean** | — | — |
| 2 | `texture:texture-filter=nearest` | refused | FS `425b` `TEX_COORD` (link `ace2+425b`) | `lower-texcoord-425b` |
| 3 | `texture:texture-filter=linear` | refused | FS `425b` `TEX_COORD` | `lower-texcoord-425b` |
| 4 | `texture:texture-filter=mipmap` | refused | FS `425b` `TEX_COORD` | `lower-texcoord-425b` |
| 5 | `shading:shading=gouraud` | **clean** | — | — |
| 6 | `shading:shading=blinn-phong-inf` | refused | FS `994f` `NONFINITE_CONST` | `lower-nonfinite-const` |
| 7 | `shading:shading=phong` | refused | FS `8a64` `ARITH_CONST` | `lower-constprod-rest` |
| 8 | `shading:shading=cel` | refused | FS `fea0` `NONFINITE_CONST` | `lower-nonfinite-const` |
| 9 | `bump:bump-render=high-poly` | refused | FS `d442` `NONFINITE_CONST` | `lower-nonfinite-const` |
| 10 | `bump:bump-render=normals` | refused | FS `487d` `NONFINITE_CONST` | `lower-nonfinite-const` |
| 11 | `bump:bump-render=height` | refused | FS `77c5` `NONFINITE_CONST` | `lower-nonfinite-const` |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | refused | FS `3c3d` `TEX_MULTI` | `lower-fma` |
| 13 | `effect2d:kernel=1,1,1,1,1;…` | refused | FS `ba8e` `TEX_MULTI` | `lower-fma` |
| 14 | `pulsar:quads=5:texture=false:light=false` | **clean** | — | — |
| 15 | `desktop:…:effect=blur:…` | refused | FS `68af`, `7f17` `TEX_MULTI` | `lower-fma` |
| 16 | `desktop:windows=4:effect=shadow` | **clean** | — | — |
| 17 | `buffer:…:update-method=map:interleave=false` | **clean** | — | — |
| 18 | `buffer:…:update-method=subdata:interleave=false` | **clean** | — | — |
| 19 | `buffer:…:update-method=map:interleave=true` | **clean** | — | — |
| 20 | `ideas:speed=duration` | refused | VS `51b2` `CONTROL_FLOW`; FS `1aca`, `c565` `INT_VALUE`; FS `df64` `TEX_COORD` | `lower-ideas` |
| 21 | `jellyfish` | refused | VS `08a2` `ARITH_CONST`; FS `8ee5` `FS_OPERAND` | `lower-constprod-rest` (`08a2`); none for `8ee5` |
| 22 | `terrain` | refused | FS `1637`, `4d26`, `c549` `ARITH`; FS `8c86`, `bbba`, `bdfd`, `e043` `TEX_MULTI` | `lower-fs-arith2`; `lower-fma` |
| 23 | `shadow` | refused | FS `cf4b` `ARITH` | `lower-fs-arith2` |
| 24 | `refract` | refused | FS `3fae` `ARITH_CONST` | `lower-constprod-rest` |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | **clean** | — | — |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | **clean** | — | — |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | **clean** | — | — |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | **clean** | — | — |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | refused | FS `849c` `FRAGCOORD_FP16` | `lower-mploop` |
| 30 | `loop:…:fragment-loop=false` | **clean** | — | — |
| 31 | `loop:…:fragment-uniform=false` | **clean** | — | — |
| 32 | `loop:…:fragment-uniform=true` | refused | FS `3678` `LOOP_FP16` | `lower-mploop` |

The `buffer` rows stand for
`buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=…:interleave=…`,
the `loop` rows for `loop:vertex-steps=5:fragment-steps=5:…`, scene 15 for
`desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true`.
Shaders not named in a row compile.

**Tally at main `104748bb`: 14 of 33 scenes are compile-clean** (0, 1, 5,
14, 16, 17, 18, 19, 25, 26, 27, 28, 30, 31). That is what
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_corpus -- --nocapture`
prints at that commit: `mali: 69 shaders, 42 compiled; 22 pairs tried, 21
linked; 14 of 33 scenes compile-clean: [0, 1, 5, 14, 16, 17, 18, 19, 25, 26,
27, 28, 30, 31]`. 42 of the 69 corpus shaders compile (it was 23 at
`d09629d3`), and 21 of the 22 programs whose two stages were tried linked;
the one that did not is `ace2+425b`, the texture scenes' program, refused
at its fragment stage. The other 19 scenes are refused at compile
time; scene 22 (`terrain`) still carries glmark2's abort behind its
refusals (below).

Against the `d09629d3` re-measure (2 of 33 clean, 23 shaders compile):
12 scenes became clean (0, 1, 5, 17-19, 25-28, 30, 31). No shader moved
back (the test fails on one that does).

What the harness showed at `d09629d3`, for the scenes still refused (not
re-run at `104748bb`; `host-draw-probe-2` measures the setup and draw side
at the tip):

- **Setup fails**: glmark2 prints `Error: Failed to add vertex|fragment
  shader from file None:` and the compile log, the scene draws nothing and
  puts no work of its own on the GPU. A refused shader cannot fault the
  GPU.
- **Runs with GL errors** (desktop blur, 15): the GL errors are what ES 2.0
  asks for when a client uses a program that did not link, not a GL-core
  defect. The refused draws are not submitted.
- **terrain aborts** in glmark2 itself, not in YakoGL: signal 11 (rc 139)
  on the host, SIGABRT (rc 134) on the board (main `da65355e`,
  RESULTS.md). In `TerrainRenderer::init_program()`
  (`src/scene-terrain/terrain-renderer.cpp`), `if
  (!Scene::load_shaders_from_strings(...)) return;` returns when the
  compile or link fails and goes on with the unlinked program.
  `TerrainRenderer::render` then reaches `Mesh::render_vbo` with -1
  attribute locations as an index. On the board Debian trixie's g++ 14
  enables libstdc++'s `std::vector::operator[]` assertion, which fires on
  the -1 index (SIGABRT); on the host the out-of-range read gives signal
  11. The -1 locations are the symptom; the refused program is the cause.
  `18bc` (terrain's second vertex shader, two `texture2D` look-ups) now
  compiles; terrain's fragment shaders `1637`, `4d26` and `c549` (`ARITH`)
  and `8c86`, `bbba`, `bdfd`, `e043` (`TEX_MULTI`) are what is left.

## 2. The causes

### Compile refusals still open

Read from the fixture at main `104748bb`. The message is
`MaliError::message`'s text (`src/shaderc/online/mali/mod.rs`). **Shaders**
counts the distinct corpus shaders whose first refusal in their stage it is;
**scenes** are the scenes that meet it; **alone** the scenes in which it is
the only family left, so that lowering it alone makes the scene
compile-clean. The code names are this note's (the older ones are kept where
a family kept its message). **Item** is the queued lowering item.

| cause | message | shaders | hashes | scenes | alone | item |
|---|---|---:|---|---|---:|---|
| `NONFINITE_CONST` (was `CONST_FOLD`) | `Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet` | 5 FS | `994f` `fea0` `d442` `487d` `77c5` | 6, 8, 9, 10, 11 | 5 | `lower-nonfinite-const` |
| `TEX_MULTI` | `Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet` | 8 FS | `3c3d` `ba8e` `68af` `7f17` `8c86` `bbba` `bdfd` `e043` | 12, 13, 15, 22 | 3 | `lower-fma` (in review) |
| `TEX_COORD` | `Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet` | 2 FS | `425b` `df64` | 2, 3, 4, 20 | 3 | `lower-texcoord-425b` (`425b`), `lower-ideas` (`df64`) |
| `ARITH_CONST` | `Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet` | 2 FS, 1 VS | FS `8a64` `3fae`; VS `08a2` | 7, 21, 24 | 2 | `lower-constprod-rest` |
| `ARITH` | `Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet` | 4 FS | `cf4b` `1637` `4d26` `c549` | 22, 23 | 1 | `lower-fs-arith2` |
| `INT_VALUE` | `Valhall: an integer or boolean value is not lowered yet` | 2 FS | `1aca` `c565` | 20 | 0 | `lower-ideas` |
| `CONTROL_FLOW` | `Valhall: control flow (if, a loop, a return before the end) is not lowered yet` | 1 VS | `51b2` | 20 | 0 | `lower-ideas` |
| `FS_OPERAND` | `Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet` | 1 FS | `8ee5` | 21 | 0 | none queued |
| `FRAGCOORD_FP16` | `Valhall: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y is not lowered yet` | 1 FS | `849c` | 29 | 1 | `lower-mploop` |
| `LOOP_FP16` | `Valhall: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge) is not lowered yet` | 1 FS | `3678` | 32 | 1 | `lower-mploop` |

28 corpus shaders are refused (2 vertex, 26 fragment), and `ace2` (a vertex
shader that compiles) sits in the texture scenes' refused program
`ace2+425b`. Families no longer in the fixture: the vertex `CONST_FOLD`
(`ace2`, `83cd`, `6964`), `FAU_PAGE`, `LOCALS`, `NEG_ZERO`, `TEX_LANES`,
`LOOP_VERTEX`, `UNIFORM_TYPE`, `FMA` and `FRAGCOLOR_LANE`: each shader that met
one now compiles or meets a family above. `FS_OPERAND` is the only family
open without an item; `8ee5` is jellyfish's fragment shader, so scene 21
needs it as well as `lower-constprod-rest`.

**Order of scenes unblocked** (greedy: each step lowers the family that
makes the most scenes compile-clean given the steps before it; a scene can
still meet a later refusal, section 3):

| step | family (item) | scenes made compile-clean | clean after |
|---:|---|---|---:|
| — | (now, `104748bb`) | 14 scenes | 14 |
| 1 | `NONFINITE_CONST` (`lower-nonfinite-const`) | 6, 8, 9, 10, 11 | 19 |
| 2 | `TEX_COORD` `425b` (`lower-texcoord-425b`) | 2, 3, 4 | 22 |
| 3 | `TEX_MULTI` (`lower-fma`) | 12, 13, 15 | 25 |
| 4 | `ARITH_CONST` (`lower-constprod-rest`) | 7, 24 | 27 |
| 5 | `ARITH` (`lower-fs-arith2`) | 23 | 28 |
| 6 | `FRAGCOORD_FP16`, `LOOP_FP16` (`lower-mploop`) | 29, 32 | 30 |
| 7 | `INT_VALUE`, `CONTROL_FLOW`, `TEX_COORD` `df64` (`lower-ideas`) | 20 | 31 |
| 8 | `FS_OPERAND` (no item) and `ARITH_CONST` `08a2` | 21 | 32 |
| 9 | `ARITH`, `TEX_MULTI` | 22 (`terrain`) | 33 |

This is the arithmetic of the table, not a measurement: each lowering can
uncover a later refusal behind the first (section 3).

### Setup- and draw-time causes

Predicted from the code at `95f20610`, updated at `104748bb`. A scene's
setup and draw causes are reached only once its shaders compile, so none is
confirmed on the board. `shadow` (scene 23) still stops at `cf4b` and
`refract` (24) at `3fae`. The line numbers of the older rows are gone.

| code | what | where | scenes | state |
|---|---|---|---|---|
| `D-DEPTHONLY` | A framebuffer with no colour attachment was `GL_FRAMEBUFFER_UNSUPPORTED` on the Mali, and glmark2's shadow pass (`DepthRenderTarget::setup`) gives up on that status. | setup | `shadow` | **lowered in `src/mali`**: `DRAWS_WITHOUT_COLOUR` is `true` (`src/mali/backend.rs:111`), so a depth-only framebuffer is complete and drawn as panfrost draws one with no colour buffer (`docs/notes/mali-depth-only.md`). Unit tests only; the board has not drawn one (item `board-depthonly-check`). |
| `D-NPOTMIP` | refract's colour target (1600x1200 RGBA, `GL_LINEAR_MIPMAP_LINEAR`, `glGenerateMipmap` every frame). The Mali lays out no NPOT chain (`npot_mip_chains` in `src/gl/texture.rs`: `MIP_LEVEL_ALIGN` 64, `native_layered_textures` false, so `grow_npot_chain` keeps one level), so the texture samples level 0 under the mipmap filter. Not a refusal, but a picture that can differ from Mesa's where it is minified. | draw (picture) | `refract` | **risk**, open: open an issue only if the board's picture shows it (section 4) |
| `D-DEPTHTEX` | The depth texture `refract` and `shadow` sample goes through `mali_depth_resolve` to R32F. gl_test case 169 is the only board evidence for it. | draw | `refract`, `shadow` | **risk**, open; #32 (`docs/notes/mali-texture-refusals.md`) if it fails |
| `D-OK` | No texture-format or blend refusal is expected: glmark2 uploads unsized `GL_RGB`/`GL_RGBA` `GL_UNSIGNED_BYTE` (stored RGBA8), its targets are `GL_RGBA`, and its blends are fixed function (`src/mali/blend.rs`). Every attribute is float32. `glGenerateMipmap` runs on the CPU. The harness at `d09629d3` confirmed it for the scenes it reached. | — | all | — |

`host-draw-probe-2` re-runs glmark2's scenes through the Mali back end at the
tip; its result, not this table, is the record of the causes past the
compiler.


### GPU-fault and hang risks

- **Now**: low. Scenes 14-19 are the only ones with a stream of their own
  on the GPU; 17-19 submit only their clears. 14 (`pulsar`, new: 15 draws,
  5 submits in 3 frames on the host) draws five blended quads with one
  program; 15 and 16 render 800x600 RGBA8 FBO textures, sample them and
  blend with fixed function (16: 107 draws, 25 submits), which the gl_test
  sweep's PASS set covers. Their pictures are the board run's to check.
- **Once the loop families are lowered** (`LOOP_VERTEX`, `LOOP_FP16`):
  `loop:…:fragment-uniform=true` (`3678`) runs a fragment loop bounded by a
  uniform, the hang-risk class of `docs/notes/mali-gles2-forecast.md`
  section 3; `LOOP_FP16` is that loop's refusal now. `ideas` (540 draws) and
  `terrain` (nine passes into FBOs) are where the frame arenas and
  `SLOT_STILL_IN_FLIGHT` show first.

## 3. The next refusals behind the first

Each stage still gives its first refusal only. What the re-measure shows
of the forecast's predictions, and what is still a prediction:

- **Confirmed: the control-flow families are reached.** The `loop` vertex
  shader `3cb0` now stops at `LOOP_VERTEX` and the uniform-bounded fragment
  loop `3678` at `LOOP_FP16`, as the forecast expected once `ARITH` was
  gone. The `loop` fragment shader `c7ea` (a constant-bounded loop) stops
  at `FRAGCOORD_FP16` before its loop: expect `LOOP_FORM` or its `int`
  counter (`INT_VALUE`) behind it if Mesa's unroll does not take it
  (`docs/notes/mali-loop-break.md`). `e016` (`fragment-loop=false`) has no
  loop.
- **Confirmed: the link rules pass where they are reached.** All 7
  programs whose two stages compile link, among them ideas' `038e+23c8`,
  `c236+23c8`, `4f27+e6a9` and the depth program `37f4+928e`: no partly
  read varying or constant varying lane refused a link.
- **Still behind**: terrain's `18bc` (vertex) now stops at `FAU_PAGE`, not
  `ARITH_CONST`. It does **two** `texture2D` look-ups, so after `FAU_PAGE`
  expect `TEX_VERTEX` (`Valhall: a vertex shader texture look-up other than
  one 2D look-up at a level (a second one, a size query) is not lowered
  yet`; `docs/notes/mali-vs-texture-lanes.md`,
  `docs/notes/mali-texture-lookups.md`; #32). Refract's `7215` also stops
  at `FAU_PAGE`.
- **New first refusals with more behind them**: the lit fragment shaders
  now at `FRAGCOLOR_LANE` or `ARITH_CONST` that call `reflect` or `mix`
  (`8a64`, `fea0`, `9fe6`, `3fae`, `1aca`, `4d26`, `8ee5`) may meet
  `ARITH` again there behind the first gap.
  `conditionals:…fragment-steps=5` (`a5b5`) has `if` steps past its
  `FS_OPERAND`: the control-flow family (`docs/notes/mali-control-flow.md`)
  may come next.
- **Setup and draw**: `shadow` reaches `D-DEPTHONLY` once its model
  (`6964`, `CONST_FOLD`) and ground (`b9d1+cf4b`) programs build; its depth
  program already does. `refract` reaches `D-NPOTMIP` and `D-DEPTHTEX` once
  `7215+3fae` builds.

## 4. Where each cause belongs, and drafts for the new ones

Existing issues: `ARITH_CONST`, `ARITH`, `FMA`, `CONST_FOLD`, `LOCALS`,
`INT_VALUE` and `FRAGCOLOR_LANE` are #33 (plan #309). `TEX_COORD` and
`TEX_VERTEX` are #32 (plan #309). `D-DEPTHTEX` is #32 if it fails. Two
causes have no issue:

**Draft: `D-DEPTHONLY`** (lowered since: section 2; the draft is kept as written. Not reached by the first board run, main
`da65355e`: shadow stops at its compile refusal first)

> Title: Mali: a framebuffer with only a depth attachment is
> GL_FRAMEBUFFER_UNSUPPORTED (glmark2 shadow cannot set up)
>
> `src/mali/backend.rs:91` declares `DRAWS_WITHOUT_COLOUR = false`
> (`DEPTH_ONLY_TARGETS`, `src/backend.rs:761`), so `check_status`
> (`src/gl/framebuffer.rs:871-874`) reports a framebuffer with no colour attachment as
> `GL_FRAMEBUFFER_UNSUPPORTED`. glmark2's `shadow` scene renders its depth
> pass into such a framebuffer (a `GL_DEPTH_COMPONENT` texture on
> `GL_DEPTH_ATTACHMENT`, no colour; `DepthRenderTarget::setup`,
> `src/scene-shadow.cpp:73-118`) and stops at
> `DepthRenderTarget::setup: glCheckFramebufferStatus failed (0x8cdd)`, so
> the scene cannot run on the NanoPC-T6 even after its shaders build. The
> PowerVR, GC7000 and V3D back ends draw without a colour target. The frame
> builder needs a frame with no colour render target, as panfrost draws a
> framebuffer with no colour buffer (how Mesa 26.2.1 lays out that FBD is
> still to be read), and `mali_draw` must accept no colour target at the
> depth attachment's size.
> Forecast: `docs/notes/mali-glmark2-forecast.md` (`D-DEPTHONLY`). Board
> check: glmark2 `shadow`, and a gl_test case that draws depth only and
> samples the texture.

**Draft: `D-NPOTMIP`** (to open only if a board run's refract picture is
wrong where the reflection is minified; not reached by the first board run,
main `da65355e`: refract stops at its compile refusal first)

> Title: Mali: a non-power-of-two texture keeps one level under
> glGenerateMipmap (glmark2 refract samples level 0)
>
> On the Mali the GL core lays out no NPOT mip chain (`npot_mip_chains` is
> false: `MIP_LEVEL_ALIGN` is 64 and the textures are not native layered),
> so glmark2 refract's 1600x1200 colour target, mipmapped every frame and
> sampled with `GL_LINEAR_MIPMAP_LINEAR`, samples level 0. Mesa lays out
> NPOT chains on v10, so the picture can differ where the texture is
> minified. Either lay the NPOT chain out at 64-byte level alignment for
> this back end, or record why level 0 is accepted.
> Forecast: `docs/notes/mali-glmark2-forecast.md` (`D-NPOTMIP`).

## 5. How to redo it

View 1: `tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-mali
mali` rewrites `tools/hostharness/glmark2/FINDINGS-mali-allscenes.md`;
`git diff` it. View 2: run `run.sh "$GLMARK2_SRC" build/glmark2-host powervr`
(`GM2_WRITE=0` keeps `FINDINGS.md`) for the corpus and the logs, then build
each logged pair on the `MALI_CONFIG` build as "How it was made" says and
compare the first refusal of each stage with section 1. Recheck when
`src/shaderc/online/mali/**` (the causes), `src/mali/backend.rs`
(`DRAWS_WITHOUT_COLOUR`, `MIP_LEVEL_ALIGN`) or `src/gl/texture.rs`
(`npot_mip_chains`) move.
