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

- **Made at** main `95f20610`. When `src/mali/**`, `src/shaderc/online/**`
  or `src/gl/**` move, the causes move with them (section 5).
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

Scene numbers are the harness's (`FINDINGS-mali-allscenes.md`, entry order
of glmark2's default list). Programs are `<vs>+<fs>`, the first four hex
digits of each corpus file's hash (`<scene>-<hash>.{vert,frag}.glsl`).
**Harness**: what glmark2 did on the host (`setup fails`: exit 3 after a
shader is refused, nothing drawn; `runs, GL errors`: the scene goes on
without the refused program; `crash`: glmark2 itself, signal 11). The
**cause** codes are section 2's; `VS`/`FS` give both stages per program
(view 2). The **owner** is plan #309 for every refusal (`src/mali/**`,
`src/shaderc/online/mali/**`), with the issue its note names.

| # | scene | programs | harness | cause | owner |
|---|---|---|---|---|---|
| 0 | `build:use-vbo=false` | `ace2+4748` | setup fails | VS `ARITH_CONST` | #309, #33 |
| 1 | `build:use-vbo=true` | `ace2+4748` | setup fails | VS `ARITH_CONST` | #309, #33 |
| 2 | `texture:texture-filter=nearest` | `ace2+425b` | setup fails | VS `ARITH_CONST` | #309, #33 |
| 3 | `texture:texture-filter=linear` | `ace2+425b` | setup fails | VS `ARITH_CONST` | #309, #33 |
| 4 | `texture:texture-filter=mipmap` | `ace2+425b` | setup fails | VS `ARITH_CONST` | #309, #33 |
| 5 | `shading:shading=gouraud` | `83cd+4748` | setup fails | VS `ARITH_CONST` | #309, #33 |
| 6 | `shading:shading=blinn-phong-inf` | `78e3+994f` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| 7 | `shading:shading=phong` | `e5f2+8a64` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| 8 | `shading:shading=cel` | `e5f2+fea0` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| 9 | `bump:bump-render=high-poly` | `78e3+d442` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| 10 | `bump:bump-render=normals` | `51d0+487d` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| 11 | `bump:bump-render=height` | `270a+77c5` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | `4623+3c3d` | setup fails | FS `CONST_FOLD` | #309, #33 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | `4623+ba8e` | setup fails | FS `LOCALS` | #309, #33 |
| 14 | `pulsar:quads=5:texture=false:light=false` | `9d75+4748` | setup fails | VS `ARITH_CONST` | #309, #33 |
| 15 | `desktop:…:effect=blur:…` | `792b+9ce9` ok, `792b+7f17`, `792b+68af` | runs, 352 GL errors | FS `CONST_FOLD` (both blur passes) | #309, #33 |
| 16 | `desktop:windows=4:effect=shadow` | `792b+9ce9` ok | **runs, clean** | none; picture to check (`D-OK`) | — |
| 17 | `buffer:…:update-method=map:interleave=false` | `7234+9fe6` | runs, 15 GL errors | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| 18 | `buffer:…:update-method=subdata:interleave=false` | `7234+9fe6` | runs, 15 GL errors | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| 19 | `buffer:…:update-method=map:interleave=true` | `7234+9fe6` | runs, 15 GL errors | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| 20 | `ideas:speed=duration` | 9: `bf4b c265 a09e 038e c236 +23c8`, `51b2+c565`, `51b2+1aca`, `4f27+e6a9`, `4f27+df64` | setup fails | VS `FMA` (all 9); FS `INT_VALUE` (`c565`), `ARITH` (`1aca`), `TEX_COORD` (`df64`) | #309, #33; `TEX_COORD` #32 |
| 21 | `jellyfish` | `08a2+22fc`, `14ac+8ee5` | setup fails | VS `FMA` + FS `ARITH`; FS `FRAGCOLOR_LANE` | #309, #33 |
| 22 | `terrain` | 9: `8b69+895a` ok, `8b69+{1637,c549,d697,e043,bdfd,bbba,8c86}`, `18bc+4d26` | **crash** (glmark2; host rc 139, board SIGABRT rc 134, libstdc++ assertion) | FS `ARITH` (6), FS `LOCALS` (`1637`), VS `ARITH_CONST` + FS `ARITH` (`18bc+4d26`; next VS `TEX_VERTEX`, section 3) | #309, #33; `TEX_VERTEX` #32 |
| 23 | `shadow` | `6964+4748`, `37f4+928e`, `b9d1+cf4b` | setup fails | VS `ARITH_CONST` (all 3), FS `ARITH` (`cf4b`); then `D-DEPTHONLY` | #309, #33; `D-DEPTHONLY` new (section 4) |
| 24 | `refract` | `7215+3fae`, `37f4+928e` | setup fails | VS `ARITH_CONST` (both), FS `ARITH` (`3fae`); then `D-NPOTMIP` risk | #309, #33; `D-NPOTMIP` new if the board shows it (section 4) |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | `3107+6c61` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | `3107+a5b5` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | `a21c+6c61` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | `0162+c8ce` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | `0162+849c` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| 30 | `loop:…:fragment-loop=false` | `3cb0+e016` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| 31 | `loop:…:fragment-uniform=false` | `3cb0+c7ea` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| 32 | `loop:…:fragment-uniform=true` | `3cb0+3678` | setup fails | VS `ARITH`, FS `ARITH`; hang risk once unblocked (section 3) | #309, #33 |

The `buffer` rows stand for
`buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=…:interleave=…`,
the `loop` rows for `loop:vertex-steps=5:fragment-steps=5:…`, scene 15 for
`desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true`.

**Tally: 1 of 33 scenes is forecast clean** (16,
`desktop:windows=4:effect=shadow`). 27 fail at setup, 4 run with GL errors
and a wrong picture (15, 17, 18, 19), 1 crashes in glmark2 (22, terrain).
All 32 come from compile refusals that plan #309 owns. Two of the 32 also
carry a non-compile cause behind the refusal: `shadow` (`D-DEPTHONLY`, a
certain setup failure) and `refract` (`D-NPOTMIP`, a picture risk).

What the 32 do on the board, from the harness's logs:

- **Setup fails** (27): glmark2 prints `Error: Failed to add vertex|fragment
  shader from file None:` and the compile log, the scene draws nothing and
  puts no work of its own on the GPU. A refused shader cannot fault the GPU.
- **Runs with GL errors** (15, 17-19): the GL errors are what ES 2.0 asks
  for when a client uses a program that did not link
  (`0x501 in glGetAttribLocation`/`glGetUniformLocation`, `0x502` at the
  draw, `current_drawable_program`), not a GL-core defect. The refused draws
  are not submitted. 17-19 draw nothing but the clear (3 draws over 3
  frames, all refused); 15 draws its windows without the blur.
- **terrain crashes** (signal 11) in glmark2's `Mesh::render_vbo`, from
  `TerrainRenderer::render`, using an unlinked program's -1 attribute
  locations as an index. A crash of the glmark2 process, not of YakoGL;
  the board runner's process per scene must contain it (plan #332 item 2).
  On the board (main `da65355e`, RESULTS.md) the same cause ends as SIGABRT,
  rc 134: Debian trixie's g++ 14 enables libstdc++'s
  `std::vector<int>::operator[]` assertion, which fires in `Mesh::render_vbo`
  on the -1 index before the read that gives signal 11 on the host. No
  panthor kernel line, the runner's probe after it passed, boot_id unchanged.

## 2. The causes

### Compile refusals (all plan #309's scope)

The message is `MaliError::message`'s text (`src/shaderc/online/mali/mod.rs`).
"Shaders" counts the distinct corpus shaders (view 2) whose first refusal
in their stage it is.

| cause | message | shaders | scenes (first refusal, view 1) | note, issue |
|---|---|---:|---:|---|
| `ARITH_CONST` | `Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet` | 13 VS | 19 | `docs/notes/mali-algebraic-folds.md`, `docs/notes/mali-remaining-families.md`; #33 |
| `ARITH` | `Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet` | 4 VS, 25 FS | 9 (and behind 13 others) | `docs/notes/mali-alu-ops.md`, `docs/notes/mali-vs-alu-ops.md`; #33 |
| `FMA` | `Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet` | 8 VS | 1 (`ideas`; also jellyfish's `08a2`) | `docs/notes/mali-vs-alu-ops.md`; #33 |
| `CONST_FOLD` | `Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet` | 3 FS | 2 (12, 15) | `docs/notes/mali-mixed-precision.md` (non-finite folds); #33 |
| `LOCALS` | `Valhall: a shader with more than 32 local variables is not lowered yet` | 2 FS | 2 (13, and in 22) | `docs/notes/mali-remaining-families.md`; #33 |
| `INT_VALUE` | `Valhall: an integer or boolean value is not lowered yet` | 1 FS | behind `FMA` (20) | `docs/notes/mali-int-and-size.md`; #33 |
| `TEX_COORD` | `Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet` | 1 FS | behind `FMA` (20) | `docs/notes/mali-texture-refusals.md`; #32 |
| `FRAGCOLOR_LANE` | `Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet` | 1 FS | 1 (21) | `docs/notes/mali-varying-constants.md`, `docs/notes/mali-remaining-families.md`; #33 |

Against the earlier reading of the same corpus (plan #332's first try,
2026-11-08, at `15f23afc`), one shader moved: jellyfish's vertex shader
`08a2` is now refused for `FMA`, not `ARITH` (the transcendental ops of
`3d937fe5` are lowered; its non-matrix `a * b + c` is next). Nothing else
changed, and no shader that was refused builds yet.

What is behind each family, read from the corpus (built-in calls, as a
guide: the compiler names only the family):

- **`ARITH_CONST`, vertex**: every lit or transformed vertex shader of
  `build`, `texture`, `shading`, `bump`, `pulsar`, `buffer`, `shadow`,
  `refract` and terrain's `18bc`: glmark2's
  `ModelViewProjectionMatrix * vec4(position, 1.0)` with a constant-scaled
  normal or lighting term. The vertex shaders that compile are
  `desktop-792b`, `effect2d-4623`, `jellyfish-14ac` and `terrain-8b69`.
- **`ARITH`, fragment**: `normalize`, `max(dot(…), 0.0)`, `pow`, `reflect`
  in the lit shaders; `fract`, `sqrt` and the steps' branches in
  `conditionals`, `function` and `loop`; `abs`, `exp2`, `mix`, `smoothstep`,
  `clamp` in `terrain` and `buffer`. **`ARITH`, vertex**: `fract` in
  `conditionals`, `function` and `loop`.
- **`FMA`**: every `ideas` vertex shader and jellyfish's `08a2`.
- **`CONST_FOLD`**: the desktop blur's two passes (`7f17`, `68af`) and
  effect2d's Laplace kernel (`3c3d`), whose weights include 0.0: a zero is
  not a normal float, so a fold on it is refused.
- **`LOCALS`**: effect2d's 5x3 box kernel (`ba8e`, 15 look-ups) and
  terrain's simplex-noise height map (`1637`).

### Setup- and draw-time causes (predicted from the code at `95f20610`)

None of these is reached on the host today: each scene is stopped by a
compile refusal first. The first board run (main `da65355e`) did not reach
`D-DEPTHONLY` or `D-NPOTMIP` either, for the same reason; both stay open
risks, to check once #309's refusals are gone.

| code | what | where | scenes | owner |
|---|---|---|---|---|
| `D-DEPTHONLY` | A framebuffer with no colour attachment is `GL_FRAMEBUFFER_UNSUPPORTED` on the Mali (`DRAWS_WITHOUT_COLOUR = false`, `src/mali/backend.rs:91`, read as `DEPTH_ONLY_TARGETS` at `src/backend.rs:761`; `check_status` returns `GL_FRAMEBUFFER_UNSUPPORTED` for a framebuffer with no colour attachment at `src/gl/framebuffer.rs:871-874`). glmark2's shadow pass (`DepthRenderTarget::setup`, `src/scene-shadow.cpp:73-118`) attaches only a `GL_DEPTH_COMPONENT` texture and gives up on any other status: `DepthRenderTarget::setup: glCheckFramebufferStatus failed (0x8cdd)`. | setup | `shadow` (not reached by the first board run) | none yet: new issue (section 4) |
| `D-NPOTMIP` | refract's colour target (`DistanceRenderTarget`, 1600x1200 RGBA, `GL_LINEAR_MIPMAP_LINEAR`, `glGenerateMipmap` every frame). The Mali lays out no NPOT chain (`npot_mip_chains` in `src/gl/texture.rs`: `MIP_LEVEL_ALIGN` 64, `native_layered_textures` false, so `grow_npot_chain` keeps one level), so the texture samples level 0 under the mipmap filter. Not a refusal, but a picture that can differ from Mesa's where it is minified. | draw (picture) | `refract` (not reached by the first board run) | a risk: new issue only if the board's picture shows it (section 4) |
| `D-DEPTHTEX` | The depth texture `refract` (and `shadow`, once `D-DEPTHONLY` is gone) samples goes through `mali_depth_resolve` to R32F. gl_test case 169 is the only board evidence for it. | draw | `refract`, `shadow` | a risk; #32 (`docs/notes/mali-texture-refusals.md`) if it fails |
| `D-OK` | No texture-format or blend refusal is expected: glmark2 uploads unsized `GL_RGB`/`GL_RGBA` `GL_UNSIGNED_BYTE` (stored RGBA8), its targets are `GL_RGBA`, and its blends (`SRC_ALPHA, ONE_MINUS_SRC_ALPHA`; desktop's separate `ZERO, ONE` alpha; terrain's `SRC_ALPHA, ONE`) are fixed function (`src/mali/blend.rs`). Every attribute is float32. `glGenerateMipmap` runs on the CPU (`generate_mipmap: None`). The harness confirms it for the scenes it reaches: no refused draw, no incomplete texture, no incomplete framebuffer, no OSAL refusal in any scene. | — | all | — |

### GPU-fault and hang risks

- **Now**: low. Scenes 15, 16 and 17-19 are the only ones with a stream of
  their own on the GPU; 17-19 submit only their clears. 15 and 16 render
  800x600 RGBA8 FBO textures, sample them and blend with fixed function
  (16: 107 draws, 25 submits in 3 frames on the host), which the gl_test
  sweep's PASS set covers. Their pictures are the board run's to check.
- **Once `ARITH`/`ARITH_CONST` are lowered**: `loop:…:fragment-uniform=true`
  runs a fragment loop bounded by a uniform, the hang-risk class of
  `docs/notes/mali-gles2-forecast.md` section 3. `ideas` (540 draws) and
  `terrain` (nine passes into FBOs) are where the frame arenas and
  `SLOT_STILL_IN_FLIGHT` show first.

## 3. The next refusals behind the first

Predictions from the shader texts and the families the notes list, not run:

- terrain `18bc` (vertex) does **two** `texture2D` look-ups. After
  `ARITH_CONST`, expect `TEX_VERTEX` (`Valhall: a vertex shader texture
  look-up other than one 2D look-up at a level (a second one, a size query)
  is not lowered yet`; `docs/notes/mali-vs-texture-lanes.md`,
  `docs/notes/mali-texture-lookups.md`; #32).
- Every shader with `if`/`for` past its first refusal (`conditionals`,
  `loop`, `function`, `ideas`, `terrain-4d26`) may then meet the
  control-flow and loop families (`docs/notes/mali-control-flow.md`,
  `docs/notes/mali-int-loops.md`; #33). The `loop` shaders have `int`
  counters (`INT_VALUE`).
- A program whose two stages compile may still hit the link rules: a
  varying read in part, or a varying lane computed from no attribute
  (`docs/notes/mali-varying-constants.md`; #31/#33).

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
