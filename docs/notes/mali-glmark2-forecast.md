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

Measured at main `d09629d3` (2026-11-14). Scene numbers are the harness's
(`FINDINGS-mali-allscenes.md`, entry order of glmark2's default list).
Programs are `<vs>+<fs>`, the first four hex digits of each corpus file's
hash (`<scene>-<hash>.{vert,frag}.glsl`), in the order glmark2 links them
(`PROGRAMS` in `tests/mali_glmark2_corpus.rs`). Per program: `ok` when both
stages compile and the program links, else the first refusal of each stage
that is refused (section 2's codes). This column is
`tests/fixtures/mali_glmark2_verdicts.txt`, the fixture the test pins: no
program whose two stages compile fails to link. **Harness**: what glmark2
did on the host (`setup fails`: exit 3 after a shader is refused, nothing
drawn; `runs, GL errors`: the scene goes on without the refused program;
`crash`: glmark2 itself, signal 11 on the host). **Issue**: where the
families of the row belong (section 4).

| # | scene | programs: first refusal per stage | harness | issue |
|---|---|---|---|---|
| 0 | `build:use-vbo=false` | `ace2+4748` VS `CONST_FOLD` | setup fails | #33 |
| 1 | `build:use-vbo=true` | `ace2+4748` VS `CONST_FOLD` | setup fails | #33 |
| 2 | `texture:texture-filter=nearest` | `ace2+425b` VS `CONST_FOLD` | setup fails | #33 |
| 3 | `texture:texture-filter=linear` | `ace2+425b` VS `CONST_FOLD` | setup fails | #33 |
| 4 | `texture:texture-filter=mipmap` | `ace2+425b` VS `CONST_FOLD` | setup fails | #33 |
| 5 | `shading:shading=gouraud` | `83cd+4748` VS `CONST_FOLD` | setup fails | #33 |
| 6 | `shading:shading=blinn-phong-inf` | `78e3+994f` FS `FRAGCOLOR_LANE` | setup fails | #33 |
| 7 | `shading:shading=phong` | `e5f2+8a64` FS `ARITH_CONST` | setup fails | #33 |
| 8 | `shading:shading=cel` | `e5f2+fea0` FS `FRAGCOLOR_LANE` | setup fails | #33 |
| 9 | `bump:bump-render=high-poly` | `78e3+d442` FS `FRAGCOLOR_LANE` | setup fails | #33 |
| 10 | `bump:bump-render=normals` | `51d0+487d` FS `CONST_FOLD` | setup fails | #33 |
| 11 | `bump:bump-render=height` | `270a+77c5` FS `FRAGCOLOR_LANE` | setup fails | #33 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | `4623+3c3d` FS `CONST_FOLD` | setup fails | #33 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | `4623+ba8e` FS `ARITH_CONST` | setup fails | #33 |
| 14 | `pulsar:quads=5:texture=false:light=false` | `9d75+4748` ok | **runs, clean** | — |
| 15 | `desktop:…:effect=blur:…` | `792b+9ce9` ok; `792b+7f17` FS `ARITH_CONST`; `792b+68af` FS `ARITH_CONST` | runs, 352 GL errors | #33 |
| 16 | `desktop:windows=4:effect=shadow` | `792b+9ce9` ok | **runs, clean** | — |
| 17 | `buffer:…:update-method=map:interleave=false` | `7234+9fe6` VS `ARITH`, FS `ARITH_CONST` | runs, 15 GL errors | #33 |
| 18 | `buffer:…:update-method=subdata:interleave=false` | `7234+9fe6` VS `ARITH`, FS `ARITH_CONST` | runs, 15 GL errors | #33 |
| 19 | `buffer:…:update-method=map:interleave=true` | `7234+9fe6` VS `ARITH`, FS `ARITH_CONST` | runs, 15 GL errors | #33 |
| 20 | `ideas:speed=duration` | `bf4b+23c8` VS `INT_VALUE`; `c265+23c8` VS `INT_VALUE`; `a09e+23c8` VS `ARITH_CONST`; `038e+23c8` ok; `51b2+c565` VS `UNIFORM_TYPE`, FS `INT_VALUE`; `4f27+e6a9` ok; `4f27+df64` FS `TEX_COORD`; `51b2+1aca` VS `UNIFORM_TYPE`, FS `FRAGCOLOR_LANE`; `c236+23c8` ok | setup fails | #32, #33 |
| 21 | `jellyfish` | `14ac+8ee5` FS `FRAGCOLOR_LANE`; `08a2+22fc` VS `FMA`, FS `ARITH` | setup fails | #33 |
| 22 | `terrain` | `8b69+1637` FS `NEG_ZERO`; `8b69+c549` FS `ARITH`; `8b69+d697` FS `TEX_LANES`; `18bc+4d26` VS `FAU_PAGE`, FS `FRAGCOLOR_LANE`; `8b69+e043` FS `ARITH_CONST`; `8b69+bdfd` FS `ARITH_CONST`; `8b69+895a` ok; `8b69+bbba` FS `ARITH_CONST`; `8b69+8c86` FS `ARITH_CONST` | **crash** (glmark2; host rc 139, board rc 134) | #32, #33 |
| 23 | `shadow` | `6964+4748` VS `CONST_FOLD`; `37f4+928e` ok; `b9d1+cf4b` VS `ARITH_CONST`, FS `ARITH` | setup fails | #33 |
| 24 | `refract` | `7215+3fae` VS `FAU_PAGE`, FS `ARITH_CONST`; `37f4+928e` ok | setup fails | #33 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | `3107+6c61` FS `FRAGCOORD_FP16` | setup fails | #33 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | `3107+a5b5` FS `FS_OPERAND` | setup fails | #33 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | `a21c+6c61` FS `FRAGCOORD_FP16` | setup fails | #33 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | `0162+c8ce` FS `FRAGCOORD_FP16` | setup fails | #33 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | `0162+849c` FS `FRAGCOLOR_LANE` | setup fails | #33 |
| 30 | `loop:…:fragment-loop=false` | `3cb0+e016` VS `LOOP_VERTEX`, FS `FRAGCOORD_FP16` | setup fails | #33 |
| 31 | `loop:…:fragment-uniform=false` | `3cb0+c7ea` VS `LOOP_VERTEX`, FS `FRAGCOORD_FP16` | setup fails | #33 |
| 32 | `loop:…:fragment-uniform=true` | `3cb0+3678` VS `LOOP_VERTEX`, FS `LOOP_FP16` | setup fails | #33 |

The `buffer` rows stand for
`buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=…:interleave=…`,
the `loop` rows for `loop:vertex-steps=5:fragment-steps=5:…`, scene 15 for
`desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true`.

**Tally: 2 of 33 scenes are compile-clean** (14 `pulsar`, new since the
forecast; 16 `desktop:windows=4:effect=shadow`). 26 fail at setup, 4 run
with GL errors and a wrong picture (15, 17, 18, 19), 1 crashes in glmark2
(22, terrain). All 31 come from compile refusals. Of the 48 programs, 7
build (`9d75+4748`, `792b+9ce9`, ideas' `038e+23c8`, `c236+23c8` and
`4f27+e6a9`, terrain's `8b69+895a`, and `37f4+928e`, the depth program of
shadow and refract). `shadow` still carries `D-DEPTHONLY` behind its
refusals, `refract` `D-NPOTMIP` (section 2).

Against the forecast (`95f20610`), per corpus shader: 23 of 69 compile
(16 of 29 vertex, 7 of 40 fragment; 11 at the forecast). 12 shaders that
were refused now compile, all vertex: the lit ones `78e3`, `e5f2`, `51d0`,
`270a`, `9d75`, `37f4`; `conditionals`' and `function`'s `3107`, `a21c`,
`0162`; `ideas`' `038e`, `c236`, `4f27`. Most of the rest moved to a later
refusal (section 2); no shader moved back.

What the 31 do on the board, from the harness's logs:

- **Setup fails** (26: 0-13, 20, 21, 23-32): glmark2 prints `Error: Failed
  to add vertex|fragment shader from file None:` and the compile log, the
  scene draws nothing and puts no work of its own on the GPU. A refused
  shader cannot fault the GPU.
- **Runs with GL errors** (15, 17-19): the GL errors are what ES 2.0 asks
  for when a client uses a program that did not link
  (`0x501 in glGetAttribLocation`/`glGetUniformLocation`, `0x502` at the
  draw, `current_drawable_program`), not a GL-core defect. The refused draws
  are not submitted. 17-19 draw nothing but the clear (3 draws over 3
  frames, all refused); 15 draws its windows without the blur.
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
  `init_program()` loads `terrain.vert` and `terrain.frag`; that this is
  program `18bc+4d26` is inferred from the table (row 22: the only
  terrain program whose vertex shader is not `8b69`, refused at both
  stages), not checked by hash (the corpus text was not at hand). Its two blockers now are VS
  `18bc` at `FAU_PAGE` (a push word past FAU page 0, no lowering yet:
  section 2) and FS `4d26` at `FRAGCOLOR_LANE`. `terrain.vert` does two
  `texture2D` look-ups (checked in the source), so `TEX_VERTEX` is
  forecast to follow `FAU_PAGE` for `18bc` (section 3); it has not been
  measured.

## 2. The causes

### Compile refusals

The message is `MaliError::message`'s text (`src/shaderc/online/mali/mod.rs`;
the code's variant in brackets). **Shaders** counts the distinct corpus
shaders whose first refusal in their stage it is. **Scenes** are the scenes
with a program that meets it; **alone** the scenes in which it is the only
family left, so that lowering it alone makes the scene compile-clean.
Codes of the forecast are kept; the ones marked new are new messages.

| cause | message | shaders | VS / FS hashes | scenes | alone | note, issue |
|---|---|---:|---|---|---:|---|
| `CONST_FOLD` (`ConstantFold`) | `Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet` | 3 VS, 2 FS | VS `ace2` `83cd` `6964`; FS `487d` `3c3d` | 0-5, 10, 12, 23 | 8 | `docs/notes/mali-algebraic-folds.md`, `docs/notes/mali-mixed-precision.md`; #33 |
| `FRAGCOLOR_LANE` | `Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet` | 8 FS | FS `994f` `fea0` `d442` `77c5` `1aca` `8ee5` `4d26` `849c` | 6, 8, 9, 11, 20, 21, 22, 29 | 5 | `docs/notes/mali-varying-constants.md`, `docs/notes/mali-fragment-outputs.md`; #33 |
| `ARITH_CONST` (`ArithConstant`) | `Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet` | 2 VS, 10 FS | VS `a09e` `b9d1`; FS `8a64` `ba8e` `7f17` `68af` `9fe6` `e043` `bdfd` `bbba` `8c86` `3fae` | 7, 13, 15, 17-20, 22-24 | 3 | `docs/notes/mali-constant-products.md`, `docs/notes/mali-algebraic-folds.md`; #33 |
| `FRAGCOORD_FP16` (`FragCoordMediump`), new | `Valhall: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y is not lowered yet` | 4 FS | FS `6c61` `c8ce` `e016` `c7ea` | 25, 27, 28, 30, 31 | 3 | `docs/notes/mali-fragcoord-mediump.md`, `docs/notes/mali-fragcoord-fp16-ops.md`; #33 |
| `FS_OPERAND` (`FsOperandForm`), new here | `Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet` | 1 FS | FS `a5b5` | 26 | 1 | `docs/notes/mali-gles2-forecast.md` (`FS_OPERAND`); #33 |
| `ARITH` (`Arithmetic`) | `Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet` | 1 VS, 3 FS | VS `7234`; FS `22fc` `c549` `cf4b` | 17-19, 21-23 | 0 | `docs/notes/mali-alu-ops.md`, `docs/notes/mali-vs-alu-ops.md`; #33 |
| `LOOP_VERTEX` (`LoopVertex`), new here | `Valhall: a loop in a vertex shader is not lowered yet` | 1 VS | VS `3cb0` | 30-32 | 0 | `docs/notes/mali-vs-loops.md`; #33 |
| `FAU_PAGE` (`FauPage`), new | `Valhall: a push word past FAU page 0 is not lowered yet` | 2 VS | VS `18bc` `7215` | 22, 24 | 0 | none yet (`src/shaderc/online/mali/fau.rs` `repair_fau`: Mesa's `va_select_fau_page` is not transcribed); #33 |
| `INT_VALUE` (`IntValue`) | `Valhall: an integer or boolean value is not lowered yet` | 2 VS, 1 FS | VS `bf4b` `c265`; FS `c565` | 20 | 0 | `docs/notes/mali-int-and-size.md`; #33 |
| `UNIFORM_TYPE` (`UniformType`), new here | `Valhall: a uniform other than a float, vec2, vec3, vec4 or mat4 is not lowered yet` | 1 VS | VS `51b2` | 20 | 0 | `docs/notes/mali-uniform-types.md`; #33 |
| `TEX_COORD` | `Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet` | 1 FS | FS `df64` | 20 | 0 | `docs/notes/mali-texture-refusals.md`, `docs/notes/mali-tex-coords.md`; #32 |
| `FMA` | `Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet` | 1 VS | VS `08a2` | 21 | 0 | `docs/notes/mali-vs-alu-ops.md`, `docs/notes/mali-vs-fma.md`; #33 |
| `NEG_ZERO` (`NegZeroShared`), new | `Valhall: a -0.0 that a comparison reads through a variable, or that another value reads too (Mesa's rules make it 0.0 there), is not lowered yet` | 1 FS | FS `1637` | 22 | 0 | `docs/notes/mali-algebraic-folds.md` (the -0.0 rules); #33 |
| `TEX_LANES` (`TexLanes`), new | `Valhall: a texture result's lanes other than its own pairs in order (a swizzle, a lane alone) is not lowered yet` | 1 FS | FS `d697` | 22 | 0 | `docs/notes/mali-texture-refusals.md`; #32 |
| `LOOP_FP16` (`LoopMediump`), new | `Valhall: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge) is not lowered yet` | 1 FS | FS `3678` | 32 | 0 | `docs/notes/mali-loop-break.md`, `docs/notes/mali-precision.md`; #33 |
| `LOCALS` | `Valhall: a shader with more than 32 local variables is not lowered yet` | 0 | — (was FS `ba8e`, `1637`) | — | — | lowered since the forecast; #33 |

13 vertex and 33 fragment shaders are refused (46 of 69). "new here" marks
a code that `docs/notes/mali-gles2-forecast.md` already uses for the same
message.

**Order of scenes unblocked** (greedy: each step lowers the family that
makes the most scenes compile-clean given the steps before it; a scene can
still meet a later refusal, section 3):

| step | family | scenes made compile-clean | clean after |
|---:|---|---|---:|
| — | (now) | 14, 16 | 2 |
| 1 | `CONST_FOLD` | 0-5, 10, 12 | 10 |
| 2 | `FRAGCOLOR_LANE` | 6, 8, 9, 11, 29 | 15 |
| 3 | `ARITH_CONST` | 7, 13, 15 | 18 |
| 4 | `ARITH` | 17, 18, 19, 23 (`shadow`: then `D-DEPTHONLY`) | 22 |
| 5 | `FRAGCOORD_FP16` | 25, 27, 28 | 25 |
| 6 | `LOOP_VERTEX` | 30, 31 | 27 |
| 7 | `FS_OPERAND` | 26 | 28 |
| 8 | `FAU_PAGE` | 24 (`refract`: then `D-NPOTMIP`, `D-DEPTHTEX`) | 29 |
| 9 | `FMA` | 21 (`jellyfish`) | 30 |
| 10 | `LOOP_FP16` | 32 | 31 |
| 11 | `INT_VALUE` + `UNIFORM_TYPE` + `TEX_COORD` | 20 (`ideas`) | 32 |
| 12 | `NEG_ZERO` + `TEX_LANES` | 22 (`terrain`) | 33 |

What moved since the forecast, per shader (forecast code → now):

- **Vertex `ARITH_CONST` is nearly gone** (13 → 2 shaders). Of the lit and
  transformed vertex shaders, `78e3`, `e5f2`, `51d0`, `270a`, `9d75` and
  `37f4` compile; `ace2`, `83cd` and `6964` (glmark2's `light-basic` with
  constants added) moved to `CONST_FOLD`; `7234` (buffer) to `ARITH`;
  `18bc` (terrain) and `7215` (refract) to `FAU_PAGE`; `b9d1` (shadow's
  ground) stays `ARITH_CONST`.
- **`ARITH` in fragment shaders** (25 → 3): the lit fragment shaders moved
  to `FRAGCOLOR_LANE` (`994f`, `fea0`, `d442`, `77c5`, `1aca`, `4d26`,
  `849c`), `ARITH_CONST` (`8a64`, `9fe6`, `3fae`, terrain's `e043`, `bdfd`,
  `bbba`, `8c86`), `CONST_FOLD` (`487d`), `TEX_LANES` (`d697`); the step
  shaders of `conditionals`, `function` and `loop` to `FRAGCOORD_FP16`
  (`6c61`, `c8ce`, `e016`, `c7ea`), `FS_OPERAND` (`a5b5`) and `LOOP_FP16`
  (`3678`). `22fc`, `c549` and `cf4b` stay `ARITH`.
- **`ARITH` in vertex shaders** (4 → 1): `3107`, `a21c` and `0162`
  compile; `3cb0` (loop) moved to `LOOP_VERTEX`.
- **`FMA`** (8 → 1): of `ideas`' vertex shaders `038e`, `c236` and `4f27`
  compile, `bf4b` and `c265` moved to `INT_VALUE`, `a09e` to
  `ARITH_CONST`, `51b2` to `UNIFORM_TYPE`; jellyfish's `08a2` stays `FMA`.
- **`CONST_FOLD`** in fragment shaders: the desktop blur passes `7f17` and
  `68af` moved to `ARITH_CONST`; effect2d's Laplace kernel `3c3d` stays.
- **`LOCALS`** is gone: `ba8e` moved to `ARITH_CONST`, `1637` to
  `NEG_ZERO`.
- Unchanged: `c565` (`INT_VALUE`), `df64` (`TEX_COORD`), `8ee5`
  (`FRAGCOLOR_LANE`).

What is behind each family, read from the corpus (built-in calls, as a
guide: the compiler names only the family):

- **`CONST_FOLD`**: `light-basic`'s vertex shaders with glmark2's added
  constants (`ace2` build and texture, `83cd` gouraud, `6964` shadow's
  model: `normalize` of a constant light direction, `max(dot(N, L), 0.0)`,
  constant material lanes) and the fragment shaders `487d` (bump normals)
  and `3c3d` (effect2d's Laplace kernel, weights of 0.0): a fold on or to a
  value that is not a finite normal float (a zero).
- **`FRAGCOLOR_LANE`**: lit fragment shaders that write
  `gl_FragColor = vec4(<computed>.xyz, 1.0)` (blinn-phong `994f`), a
  computed colour rather than a varying's own lane or a constant.
- **`ARITH_CONST`**: in fragment shaders now: the per-fragment lighting of
  `phong` (`8a64`), the blur and 5x3 box kernels' constant weights
  (`7f17`, `68af`, `ba8e`), buffer's and terrain's colour arithmetic.
- **`FRAGCOORD_FP16`**: the step shaders read `gl_FragCoord` in mediump
  arithmetic past the forms lowered.
- **`FAU_PAGE`**: terrain's `18bc` and refract's `7215` push more uniform
  words than FAU page 0 holds.

### Setup- and draw-time causes (predicted from the code at `95f20610`)

None of these is reached on the host at `d09629d3` either: each scene is
still stopped by a compile refusal first (`shadow` at its model program
`6964+4748`, before `DepthRenderTarget::setup`; `refract` at `7215+3fae`).
The first board run (main `da65355e`) did not reach `D-DEPTHONLY` or
`D-NPOTMIP` either, for the same reason; both stay open risks, to check
once the compile refusals are gone. The line numbers below are those of
`95f20610`.

| code | what | where | scenes | owner |
|---|---|---|---|---|
| `D-DEPTHONLY` | A framebuffer with no colour attachment is `GL_FRAMEBUFFER_UNSUPPORTED` on the Mali (`DRAWS_WITHOUT_COLOUR = false`, `src/mali/backend.rs:91`, read as `DEPTH_ONLY_TARGETS` at `src/backend.rs:761`; `check_status` returns `GL_FRAMEBUFFER_UNSUPPORTED` for a framebuffer with no colour attachment at `src/gl/framebuffer.rs:871-874`). glmark2's shadow pass (`DepthRenderTarget::setup`, `src/scene-shadow.cpp:73-118`) attaches only a `GL_DEPTH_COMPONENT` texture and gives up on any other status: `DepthRenderTarget::setup: glCheckFramebufferStatus failed (0x8cdd)`. | setup | `shadow` (not reached by the first board run) | none yet: new issue (section 4) |
| `D-NPOTMIP` | refract's colour target (`DistanceRenderTarget`, 1600x1200 RGBA, `GL_LINEAR_MIPMAP_LINEAR`, `glGenerateMipmap` every frame). The Mali lays out no NPOT chain (`npot_mip_chains` in `src/gl/texture.rs`: `MIP_LEVEL_ALIGN` 64, `native_layered_textures` false, so `grow_npot_chain` keeps one level), so the texture samples level 0 under the mipmap filter. Not a refusal, but a picture that can differ from Mesa's where it is minified. | draw (picture) | `refract` (not reached by the first board run) | a risk: new issue only if the board's picture shows it (section 4) |
| `D-DEPTHTEX` | The depth texture `refract` (and `shadow`, once `D-DEPTHONLY` is gone) samples goes through `mali_depth_resolve` to R32F. gl_test case 169 is the only board evidence for it. | draw | `refract`, `shadow` | a risk; #32 (`docs/notes/mali-texture-refusals.md`) if it fails |
| `D-OK` | No texture-format or blend refusal is expected: glmark2 uploads unsized `GL_RGB`/`GL_RGBA` `GL_UNSIGNED_BYTE` (stored RGBA8), its targets are `GL_RGBA`, and its blends (`SRC_ALPHA, ONE_MINUS_SRC_ALPHA`; desktop's separate `ZERO, ONE` alpha; terrain's `SRC_ALPHA, ONE`) are fixed function (`src/mali/blend.rs`). Every attribute is float32. `glGenerateMipmap` runs on the CPU (`generate_mipmap: None`). The harness confirms it for the scenes it reaches: no refused draw, no incomplete texture, no incomplete framebuffer, no OSAL refusal in any scene. | — | all | — |

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

**Draft: `D-DEPTHONLY`** (not reached by the first board run, main
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
