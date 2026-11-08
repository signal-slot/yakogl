<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the expected glmark2-es2 refusals, by scene

Date 2026-11-08. Plan #332 item `forecast`, for the first glmark2-es2 run of
the `gpu-mali` back end on the NanoPC-T6 (issues #20 and #7). It is a
forecast made on the host before any board run. Nothing was rendered, and
`gh` was not run.

- **Made at** the integration tip `78c917aabbdf`. `src/mali/**`,
  `src/shaderc/online/**` and `tests/glmark2_corpus.rs` are byte-identical
  in origin/main `15f23afc`, so the forecast holds there too. When those
  trees move, the causes move with them.
- **glmark2** `22c527cb0556f3a1ac4445aaa52cc532760928d5` (2023.01), the commit
  `tools/hostharness/glmark2/run.sh` pins. The corpus is GPL-3.0 text and is
  never committed. It is generated into `build/glmark2-host/shaders/`.
- **Feature set** `MALI_CONFIG` of `tools/check.sh`:
  `host-test,gpu-mali,shaderc-online,no-shader-table`.

## How it was made

```text
tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-host powervr   # the corpus and the per-scene logs
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table \
    --test glmark2_corpus -- --nocapture
```

(Run with `GM2_WRITE=0` so that the run leaves `FINDINGS.md` as it is.) The
Mali arm of `tests/glmark2_corpus.rs` reads the programs glmark2 linked from
the run's PowerVR logs. In each `logs/powervr/<n>.log` (entry `n` of
`entries.txt`), a vertex shader's `gm2: shader` line is followed by its
fragment shader's line. There are **48 distinct programs over the 33
entries**. Each program is built the way glLinkProgram builds it on the Mali
(`src/gl/program.rs` `link_program`): the two compiles, then
`compile_vs_linked` with no map (the vertex shader for the fragment shader),
then the link. The arm prints one `mali: <vs>+<fs> <ok|refused: …|error: …>`
line per program and one `mali-entry:` line per entry. It fails only on a
corpus that is not one, never on a refusal.

Result: **2 ok, 46 refused, 0 error**. The two that build are `desktop`'s
`792b+9ce9` and `terrain`'s `8b69+895a`.

What the result can and cannot see:

- Each stage gives its **first** refusal only, because the compiler stops at
  the first gap. A stage that is unblocked may hit the next family. Section 3
  lists the next refusals that can be predicted.
- The link-time rules (a partly read varying, constant varying lanes) and
  `compile_vs_linked` run only when both stages compile. So for 46 programs
  they are not reached.
- Draw-time refusals (`src/mali/backend.rs`, `src/mali/frame.rs`) are not
  run here. No Mali host harness exists (`run.sh` builds powervr, vivante and
  v3d only). Section 2 predicts them from the code and from glmark2's GL
  calls.

A scene with a refused program fails its setup. glmark2's
`Scene::load_shaders_from_strings` returns false, and glmark2 prints the
scene as `Set up failed` (`src/main-loop.cpp`). It draws nothing and puts no
work on the GPU. A refused shader therefore cannot fault the GPU.

## 1. By scene

Programs are `<vs>+<fs>`, written as the first four hex digits of each
corpus file's hash (`<scene>-<hash>.{vert,frag}.glsl`). **Forecast**:
`setup fails` (a program is refused), `runs` (every program builds, and no
draw refusal is predicted), `+ setup` / `+ draw` (a further non-compile cause,
section 2). The **cause** codes are those of section 2. The **owner** is plan
#309 for every refusal in `src/mali/**` or `src/shaderc/online/mali/**` (its
scope), plus the issue its note names.

| scene | programs | forecast | cause | owner |
|---|---|---|---|---|
| `build:use-vbo=false` | `ace2+4748` | setup fails | VS `ARITH_CONST` | #309, #33 |
| `build:use-vbo=true` | `ace2+4748` | setup fails | VS `ARITH_CONST` | #309, #33 |
| `texture:texture-filter=nearest` | `ace2+425b` | setup fails | VS `ARITH_CONST` | #309, #33 |
| `texture:texture-filter=linear` | `ace2+425b` | setup fails | VS `ARITH_CONST` | #309, #33 |
| `texture:texture-filter=mipmap` | `ace2+425b` | setup fails | VS `ARITH_CONST` | #309, #33 |
| `shading:shading=gouraud` | `83cd+4748` | setup fails | VS `ARITH_CONST` | #309, #33 |
| `shading:shading=blinn-phong-inf` | `78e3+994f` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| `shading:shading=phong` | `e5f2+8a64` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| `shading:shading=cel` | `e5f2+fea0` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| `bump:bump-render=high-poly` | `78e3+d442` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| `bump:bump-render=normals` | `51d0+487d` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| `bump:bump-render=height` | `270a+77c5` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | `4623+3c3d` | setup fails | FS `CONST_FOLD` | #309, #33 |
| `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | `4623+ba8e` | setup fails | FS `LOCALS` | #309, #33 |
| `pulsar:quads=5:texture=false:light=false` | `9d75+4748` | setup fails | VS `ARITH_CONST` | #309, #33 |
| `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | `792b+9ce9` ok, `792b+7f17`, `792b+68af` | setup fails | FS `CONST_FOLD` (both blur passes) | #309, #33 |
| `desktop:windows=4:effect=shadow` | `792b+9ce9` ok | **runs** | none predicted; picture to check (section 2, `D-OK`) | — |
| `buffer:…:update-method=map:interleave=false` | `7234+9fe6` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| `buffer:…:update-method=subdata:interleave=false` | `7234+9fe6` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| `buffer:…:update-method=map:interleave=true` | `7234+9fe6` | setup fails | VS `ARITH_CONST`, FS `ARITH` | #309, #33 |
| `ideas:speed=duration` | 9: `bf4b c265 a09e 038e c236 +23c8`, `51b2+c565`, `51b2+1aca`, `4f27+e6a9`, `4f27+df64` | setup fails | VS `FMA` (all 9); FS `INT_VALUE` (`c565`), `ARITH` (`1aca`), `TEX_COORD` (`df64`) | #309, #33; `TEX_COORD` #32 |
| `jellyfish` | `08a2+22fc`, `14ac+8ee5` | setup fails | VS `ARITH` + FS `ARITH`; FS `FRAGCOLOR_LANE` | #309, #33 |
| `terrain` | 9: `8b69+895a` ok, `8b69+{1637,c549,d697,e043,bdfd,bbba,8c86}`, `18bc+4d26` | setup fails | FS `ARITH` (6), FS `LOCALS` (`1637`), VS `ARITH_CONST` + FS `ARITH` (`18bc+4d26`; next VS `TEX_VERTEX`, section 3) | #309, #33; `TEX_VERTEX` #32 |
| `shadow` | `6964+4748`, `37f4+928e`, `b9d1+cf4b` | setup fails + setup | VS `ARITH_CONST` (all 3), FS `ARITH` (`cf4b`); `D-DEPTHONLY` | #309, #33; `D-DEPTHONLY` new (section 4) |
| `refract` | `7215+3fae`, `37f4+928e` | setup fails + draw | VS `ARITH_CONST` (both), FS `ARITH` (`3fae`); `D-NPOTMIP` risk | #309, #33; `D-NPOTMIP` new if the board shows it (section 4) |
| `conditionals:vertex-steps=0:fragment-steps=0` | `3107+6c61` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| `conditionals:vertex-steps=0:fragment-steps=5` | `3107+a5b5` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| `conditionals:vertex-steps=5:fragment-steps=0` | `a21c+6c61` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| `function:fragment-steps=5:fragment-complexity=low` | `0162+c8ce` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| `function:fragment-steps=5:fragment-complexity=medium` | `0162+849c` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | `3cb0+e016` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | `3cb0+c7ea` | setup fails | VS `ARITH`, FS `ARITH` | #309, #33 |
| `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | `3cb0+3678` | setup fails | VS `ARITH`, FS `ARITH`; hang risk once unblocked (section 3) | #309, #33 |

The `buffer` rows stand for the entries
`buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=…:interleave=…`.

**Tally: 32 scenes setup fails, 1 runs** (`desktop:windows=4:effect=shadow`).
Two of the 32 also carry a non-compile cause: `shadow` (`D-DEPTHONLY`) and
`refract` (`D-NPOTMIP`, a risk).

## 2. The causes

### Compile refusals (all plan #309's scope)

The message is the exact `MaliError::message` text
(`src/shaderc/online/mali/mod.rs`). "Shaders" counts the distinct corpus
shaders whose first refusal it is.

| cause | message | shaders | scenes | note, issue |
|---|---|---:|---:|---|
| `ARITH_CONST` (`ArithConstant`) | `Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet` | 13 VS | 19 | `docs/notes/mali-algebraic-folds.md`, `docs/notes/mali-remaining-families.md` F10; #33 |
| `ARITH` (`Arithmetic`) | `Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet` | 5 VS, 25 FS | 22 | `docs/notes/mali-alu-ops.md`, `docs/notes/mali-vs-alu-ops.md`; #33 |
| `FMA` (`FusedMultiplyAdd`) | `Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet` | 7 VS | 1 (`ideas`) | `docs/notes/mali-vs-alu-ops.md`; #33 |
| `CONST_FOLD` (`ConstantFold`) | `Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet` | 3 FS | 2 | `docs/notes/mali-mixed-precision.md` (non-finite folds); #33 |
| `LOCALS` | `Valhall: a shader with more than 32 local variables is not lowered yet` | 2 FS | 2 | `docs/notes/mali-remaining-families.md` F4; #33 |
| `INT_VALUE` (`IntValue`) | `Valhall: an integer or boolean value is not lowered yet` | 1 FS | 1 | `docs/notes/mali-int-and-size.md`; #33 |
| `TEX_COORD` (`TexCoord`) | `Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet` | 1 FS | 1 | `docs/notes/mali-texture-refusals.md` (b); #32 |
| `FRAGCOLOR_LANE` | `Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet` | 1 FS | 1 | `docs/notes/mali-remaining-families.md` F8, `docs/notes/mali-varying-constants.md`; #33 |

The shaders and their uses, read from the corpus (built-in calls, as a guide
to which op is behind each `ARITH`; the compiler names only the family):

- **`ARITH_CONST`, vertex** is every lit or transformed vertex shader of
  `build`, `texture`, `shading`, `bump`, `pulsar`, `buffer`, `shadow`,
  `refract` and terrain's `18bc`. It is glmark2's
  `ModelViewProjectionMatrix * vec4(position, 1.0)` plus a constant-scaled
  normal or lighting term. The four vertex shaders that compile are
  `desktop-792b`, `effect2d-4623`, `jellyfish-14ac` and `terrain-8b69`.
- **`ARITH`, fragment**: `normalize`, `max(dot(…), 0.0)`, `pow` and `reflect`
  in the lit shaders (`shading`, `bump`, `ideas-1aca`, `refract-3fae`);
  `fract`, `sqrt` and the steps' branches in `conditionals`, `function` and
  `loop`; `abs`, `exp2`, `mix`, `smoothstep` and `clamp` in `terrain` and
  `buffer`. **`ARITH`, vertex**: `fract` in `conditionals`, `function` and
  `loop`; `sin`, `pow`, `smoothstep`, `distance` and `abs` in `jellyfish-08a2`.
- **`FMA`** is every `ideas` vertex shader (a non-matrix `a * b + c`).
- **`CONST_FOLD`**: the desktop blur's two passes (`7f17`, `68af`) and
  effect2d's Laplace kernel (`3c3d`), whose kernel weights include 0.0. A
  zero is not a normal float, so a fold on it is refused.
- **`LOCALS`**: effect2d's 5x3 box kernel (`ba8e`, 15 look-ups) and terrain's
  simplex-noise height map (`1637`).

### Draw- and setup-time causes (predicted from the code, not run)

| code | what | where | scenes | owner |
|---|---|---|---|---|
| `D-DEPTHONLY` | A framebuffer with no colour attachment is `GL_FRAMEBUFFER_UNSUPPORTED` on the Mali (`DRAWS_WITHOUT_COLOUR = false`, `src/mali/backend.rs:91`; `src/gl/framebuffer.rs` `check_status`, ES 2.0 4.4.5). glmark2's shadow pass (`DepthRenderTarget::setup`, `src/scene-shadow.cpp`) attaches only a `GL_DEPTH_COMPONENT`/`GL_UNSIGNED_INT` texture, at 1600x1200, and gives up on any status other than complete: `DepthRenderTarget::setup: glCheckFramebufferStatus failed (0x8cdd)`. | setup | `shadow` | none yet: new issue (section 4) |
| `D-NPOTMIP` | refract's colour target (`DistanceRenderTarget`, 1600x1200 RGBA, `GL_LINEAR_MIPMAP_LINEAR`, `glGenerateMipmap` every frame). On the Mali the GL core lays out no NPOT chain (`npot_mip_chains`: `MIP_LEVEL_ALIGN` 64, not layered, `src/gl/texture.rs`), so the texture keeps one level and samples level 0 under the mipmap filter (`src/gl/draw.rs`, the NPOT rule). This is not a refusal (`NOT_BUILT_TEXTURE_NPOT_CHAIN` needs `mip_levels > 1`), but a picture that may differ from Mesa's. | draw (picture) | `refract` | a risk: new issue only if the board's picture shows it (section 4) |
| `D-DEPTHTEX` | The depth texture sampled by `refract` (and by `shadow` once `D-DEPTHONLY` is gone) goes through `depth_resolve` (`mali_depth_resolve`) to R32F, a sampled format. gl_test case 169 is the only board evidence for it. | draw | `refract`, `shadow` | a risk; #32 (`docs/notes/mali-texture-refusals.md` (c)) if it fails |
| `D-OK` | No texture-format refusal is expected. glmark2 uploads every image as unsized `GL_RGB` or `GL_RGBA` `GL_UNSIGNED_BYTE` (`setup_texture`, `src/texture.cpp`), and its render-target textures are `GL_RGBA`. The GL core stores both as RGBA8 (`texfmt_of`), so the `RGBA8, sRGB8_A8, R32F or ETC2` rule of `NOT_BUILT_TEXTURE_FORMAT` holds. The blends are `SRC_ALPHA, ONE_MINUS_SRC_ALPHA` (desktop, jellyfish), the separate `…, ZERO, ONE` alpha (desktop), and `SRC_ALPHA, ONE` (terrain's overlay). All are fixed function (`src/mali/blend.rs` `can_fixed_function`: equal bases up to an invert, or ONE), so no `NOT_BUILT_BLEND_SHADER` is expected. Every attribute is float32. Every image glmark2 mipmaps is POT, except refract's (`D-NPOTMIP`). `glGenerateMipmap` runs on the CPU (`generate_mipmap: None`). | — | all | — |

### GPU-fault and hang risks

- **Now**: low. Only `desktop:windows=4:effect=shadow` reaches the GPU with
  a scene's own draws. Every other scene puts only its clear and swap on the
  GPU, because a failed setup draws nothing. That scene renders to 800x600
  RGBA8 FBO textures, samples them, and blends with fixed function, which the
  gl_test sweep's PASS set covers.
- **Once `ARITH`/`ARITH_CONST` are lowered**: `loop:…:fragment-uniform=true`
  runs a fragment loop whose bound is a uniform. That is the hang-risk class
  of `docs/notes/mali-gles2-forecast.md` section 3. `ideas` (540 draws) and
  `terrain` (nine passes into FBOs) are where frame arena size and
  `SLOT_STILL_IN_FLIGHT` would show first.

## 3. The next refusals behind the first

Read from the shader texts and the families the notes list. These are
predictions, not run:

- terrain `18bc` (vertex) does **two** `texture2D` look-ups in the vertex
  shader. After `ARITH_CONST`, expect `TEX_VERTEX` (`Valhall: a vertex shader
  texture look-up other than one 2D look-up at a level (a second one, a size
  query) is not lowered yet`; `docs/notes/mali-vs-texture-lanes.md`,
  `docs/notes/mali-texture-lookups.md`; #32).
- Every shader with `if`/`for` past the first refusal (`conditionals`,
  `loop`, `function`, `ideas`, `terrain-4d26`) may then meet the control-flow
  and loop families (`docs/notes/mali-control-flow.md`,
  `docs/notes/mali-int-loops.md`; #33). The `loop` shaders have `int`
  counters (`INT_VALUE`).
- A program whose two stages compile may still hit the link rules: a
  varying read in part (`reads part of a varying`, gl_test case 19), or a
  varying lane computed from no attribute (`docs/notes/mali-varying-constants.md`;
  #31/#33).

## 4. Where each cause belongs, and drafts for the new ones

Existing issues: `ARITH_CONST`, `ARITH`, `FMA`, `CONST_FOLD`, `LOCALS`,
`INT_VALUE` and `FRAGCOLOR_LANE` are #33 (plan #309). `TEX_COORD` and
`TEX_VERTEX` are #32 (plan #309). `D-DEPTHTEX` is #32 if it fails. Two causes
have no issue:

**Draft: `D-DEPTHONLY`**

> Title: Mali: a framebuffer with only a depth attachment is
> GL_FRAMEBUFFER_UNSUPPORTED (glmark2 shadow cannot set up)
>
> `src/mali/backend.rs` declares `DRAWS_WITHOUT_COLOUR = false`, so the GL
> core reports a framebuffer with no colour attachment as
> `GL_FRAMEBUFFER_UNSUPPORTED` (ES 2.0 4.4.5). glmark2's `shadow` scene
> renders its depth pass into such a framebuffer: a 1600x1200
> `GL_DEPTH_COMPONENT` texture on `GL_DEPTH_ATTACHMENT`, no colour. It stops
> at `DepthRenderTarget::setup: glCheckFramebufferStatus failed (0x8cdd)`, so
> the scene can never run on the NanoPC-T6, even after its shaders build.
> The PowerVR, GC7000 and V3D back ends draw without a colour target
> (`DRAWS_WITHOUT_COLOUR = true`). The frame builder needs a frame with no
> colour render target, as panfrost draws a framebuffer with no colour
> buffer. How Mesa 26.2.1 lays out that frame's FBD is still to be read.
> `mali_draw` must also accept `target_gpu == 0` at the depth attachment's
> size.
> Forecast: `docs/notes/mali-glmark2-forecast.md` (`D-DEPTHONLY`). Board
> check: glmark2 `shadow`, and a gl_test case that draws depth only and
> samples the texture.

**Draft: `D-NPOTMIP`** (to open only if the first board run's refract
picture is wrong where the reflection is minified)

> Title: Mali: a non-power-of-two texture keeps one level under
> glGenerateMipmap (glmark2 refract samples level 0)
>
> On the Mali the GL core lays out no NPOT mip chain (`npot_mip_chains` is
> false: `MIP_LEVEL_ALIGN` is 64 and the textures are not native layered),
> so glmark2 refract's 1600x1200 colour target, mipmapped every frame and
> sampled with `GL_LINEAR_MIPMAP_LINEAR`, samples level 0. Mesa lays out
> NPOT chains on v10 (pan_layout), so the picture can differ where the
> texture is minified. Either lay the NPOT chain out at 64-byte level
> alignment for this back end, or record why level 0 is accepted.
> Forecast: `docs/notes/mali-glmark2-forecast.md` (`D-NPOTMIP`).

## 5. How to redo it

Regenerate the corpus at the pin, then run the test line of "How it was made"
and diff the `mali:` lines. `GLMARK2_CORPUS` may name the shaders directory.
Without the logs beside it (`<corpus>/../logs/powervr/`), the arm falls back
to every vertex/fragment pair of an entry. That gives more programs than
glmark2 links, and the summary line says which source it used.
