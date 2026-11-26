<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# NanoPC-T6 glmark2-es2 at origin/main 038b7d7d: report (plan #409)

The board run of plan #409 (ROADMAP stage 5, issue #7; glmark2 issue #33),
written from what is in the tree: this directory (commit `2a25a7bb`), the
`mali` column of `tools/cts/glmark2-pictures.txt` (`408c20db`, in this tree
as `9e8ea3f7`), `tests/fixtures/mali_glmark2_verdicts.txt`,
`docs/notes/mali-glmark2-forecast.md`,
`tools/hostharness/glmark2/FINDINGS-mali-allscenes.md` and glmark2
`22c527cb` source. Scene numbers are the tally's (1 to 33). The forecast
note counts from 0, so forecast # = board # - 1.

## 1. Headline

- **Build and board**: origin/main `038b7d7df63c8e5a10431dea232303512b3b4a42`,
  `dev=no`, glmark2 `22c527cb` with `mali-glmark2-pbuffer.patch`, the
  `tools/mali/panthor-gles` pair (`run/meta.txt`). The NanoPC-T6 (RK3588,
  Mali-G610, `gpu_id 0xa8670005`) ran its own Armbian 26.8.10 trixie, kernel
  6.18.55-current-rockchip64, panthor 1.5.0 (`preflight.txt`: 17 ok, 0 FAIL).
  The session ran 2026-11-25 12:44:59Z to 12:50:10Z through
  `tools/cts/mali-glmark2-session.sh`: one glmark2-es2 process a scene,
  800x600 EGL pbuffer, 180 s bound, the 33 default scenes.
- **Log**: `tools/cts/log/2026-11-25-t6-glmark2-038b7d7d/` (`tally-table.txt`,
  `tally-results-row.txt`, `state-compare.txt`, `run/scenes.tsv`,
  `run/pictures.txt`, `run/log/<n>.out|err`, `run/kmsg/<n>.log`).
- **Figures**: all 33 of 33 scenes ran. **16 clean**, 16 refused by the
  online compiler (13 at `glCompileShader`, the 3 texture scenes at
  `glLinkProgram`), and **terrain aborts in glmark2 itself**. **16 GL error lines**, all
  in terrain. 0 out-of-GPU-memory lines, 0 panthor kernel lines, 0 resets,
  0 GPU faults and 0 job timeouts in every exit line. **No score**, because
  not every scene has an FPS. The 16 clean scenes sum to 11.40 ms a frame.
- **Board state**: `/boot` unchanged (419 files), loaded modules unchanged
  (100), boot_id unchanged before and after, U-Boot env not compared (no
  `fw_printenv`; `state-compare.txt`).
- **Against the forecast**: every one of the 33 scenes agrees with the host
  fixture: the same 16 clean, the same 17 not clean, and each refusal text
  is the fixture's first refusal for that scene.
- **Pictures**: all 16 clean scenes match the `mali` column. Against `am62p`
  (the column the log's tally used), 8 match and 8 differ: timing in 5,
  fp16 mediump in 3. Against `pi4`, 11 match and 5 differ. None of the
  differences is a defect.
- **Against the old record** (main `a57d8104`, 2026-11-11): from 1 clean,
  31 refused, terrain abort and 80 GL error lines to 16 clean, 16 refused,
  terrain abort and 16 GL error lines. Desktop shadow is unchanged (0.96 ms
  then, 0.97 ms now).

## 2. Per scene

Board outcome from `run/scenes.tsv` and `tally-table.txt`. GL errors are the
`GL error` lines of `run/log/<n>.out`. "vs mali" and "vs am62p" are the
picture judgements (tolerance 8, `tools/cts/glmark2-pictures.txt:26`). The
forecast comes from `tally-host-verdicts.txt` (`--host-verdicts`, from the
fixture). Refusal texts are the board's, after `Mali(Valhall: `.

| # | scene | board outcome | GL err | FPS | ms/frame | vs mali | vs am62p | host forecast | agree | note |
|---:|---|---|---:|---:|---:|---|---|---|---|---|
| 1 | `[build] use-vbo=false` | clean (12,371 draws) | 0 | 1237 | 0.81 | match | match | clean | agree | |
| 2 | `[build] use-vbo=true` | clean (35,441 draws) | 0 | 3544 | 0.28 | match | match | clean | agree | |
| 3 | `[texture] texture-filter=nearest` | refused at link: "a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet" | 0 | - | - | absent | absent | refused, link `ace2+425b`, FS `425b` TEX_COORD | agree | |
| 4 | `[texture] texture-filter=linear` | refused at link: "a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet" | 0 | - | - | absent | absent | refused, `425b` TEX_COORD | agree | |
| 5 | `[texture] texture-filter=mipmap` | refused at link: "a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet" | 0 | - | - | absent | absent | refused, `425b` TEX_COORD | agree | |
| 6 | `[shading] shading=gouraud` | clean (29,325 draws) | 0 | 2933 | 0.34 | match | match | clean | agree | |
| 7 | `[shading] shading=blinn-phong-inf` | refused: "a constant operation on or to a value that is not a finite normal float is not lowered yet" | 0 | - | - | absent | absent | refused, FS `994f` NONFINITE_CONST | agree | |
| 8 | `[shading] shading=phong` | refused: "a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet" | 0 | - | - | absent | absent | refused, FS `8a64` ARITH_CONST | agree | |
| 9 | `[shading] shading=cel` | refused: "a constant operation on or to a value that is not a finite normal float is not lowered yet" | 0 | - | - | absent | absent | refused, FS `fea0` NONFINITE_CONST | agree | |
| 10 | `[bump] bump-render=high-poly` | refused: "a constant operation on or to a value that is not a finite normal float is not lowered yet" | 0 | - | - | absent | absent | refused, FS `d442` NONFINITE_CONST | agree | |
| 11 | `[bump] bump-render=normals` | refused: "a constant operation on or to a value that is not a finite normal float is not lowered yet" | 0 | - | - | absent | absent | refused, FS `487d` NONFINITE_CONST | agree | |
| 12 | `[bump] bump-render=height` | refused: "a constant operation on or to a value that is not a finite normal float is not lowered yet" | 0 | - | - | absent | absent | refused, FS `77c5` NONFINITE_CONST | agree | |
| 13 | `[effect2d] kernel=0,1,0;1,-4,1;0,1,0;` | refused: "a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet" | 0 | - | - | absent | absent | refused, FS `3c3d` TEX_MULTI | agree | |
| 14 | `[effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | clean (26,319 draws) | 0 | 2632 | 0.38 | match | match | clean | agree | |
| 15 | `[pulsar] light=false:quads=5:texture=false` | clean (218,620 draws) | 0 | 4373 | 0.23 | match | match | clean | agree | |
| 16 | `[desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4` | clean (101,178 draws) | 0 | 723 | 1.38 | match | mismatch (tr c0c0c0 vs ffffff, off 63) | clean | agree | picture: P2 (section 3) |
| 17 | `[desktop] effect=shadow:windows=4` | clean (267,517 draws) | 0 | 1029 | 0.97 | match | mismatch (tr b1b1b1 vs c0c0c0, off 15) | clean | agree | picture: P1 |
| 18 | `[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map` | clean (4,676 draws) | 0 | 468 | 2.14 | match | mismatch (c b4daf0 vs dcedf8, off 40) | clean | agree | picture: P3 |
| 19 | `[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata` | clean (7,575 draws) | 0 | 758 | 1.32 | match | match | clean | agree | |
| 20 | `[buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map` | clean (4,862 draws) | 0 | 487 | 2.06 | match | mismatch (c b4daf0 vs d5eaf7, off 33) | clean | agree | picture: P3 |
| 21 | `[ideas] speed=duration` | refused: "an integer or boolean value is not lowered yet" | 0 | - | - | absent | absent | refused, FS `1aca` (and `c565`) INT_VALUE | agree | |
| 22 | `[jellyfish] <default>` | refused: "a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet" | 0 | - | - | absent | absent | refused, FS `8ee5` FS_OPERAND (and VS `08a2` ARITH_CONST) | agree | |
| 23 | `[terrain] <default>` | crash: glmark2 aborts (libstdc++ `operator[]` assertion; no exit line) after 3 FS refused "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet" and 4 FS refused as TEX_MULTI | 16 | - | - | absent | absent | refused, FS `1637` ARITH (also `4d26`, `c549` ARITH; `8c86`, `bbba`, `bdfd`, `e043` TEX_MULTI) | agree | crash and GL errors: T1, T2 |
| 24 | `[shadow] <default>` | refused: "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet" | 0 | - | - | absent | absent | refused, FS `cf4b` ARITH | agree | |
| 25 | `[refract] <default>` | refused: "a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet" | 0 | - | - | absent | absent | refused, FS `3fae` ARITH_CONST | agree | |
| 26 | `[conditionals] fragment-steps=0:vertex-steps=0` | clean (38,850 draws) | 0 | 3885 | 0.26 | match | match | clean | agree | |
| 27 | `[conditionals] fragment-steps=5:vertex-steps=0` | clean (41,654 draws) | 0 | 4166 | 0.24 | match | match | clean | agree | |
| 28 | `[conditionals] fragment-steps=0:vertex-steps=5` | clean (40,980 draws) | 0 | 4098 | 0.24 | match | mismatch (tr 000000 vs fdfdfd, off 253) | clean | agree | picture: P4 |
| 29 | `[function] fragment-complexity=low:fragment-steps=5` | clean (39,243 draws) | 0 | 3924 | 0.26 | match | mismatch (tr 424242 vs 2c2c2c, off 22; br 9e9e9e vs 868686, off 24) | clean | agree | picture: P5 |
| 30 | `[function] fragment-complexity=medium:fragment-steps=5` | refused: "fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y is not lowered yet" | 0 | - | - | absent | absent | refused, FS `849c` FRAGCOORD_FP16 | agree | |
| 31 | `[loop] fragment-loop=false:fragment-steps=5:vertex-steps=5` | clean (40,504 draws) | 0 | 4050 | 0.25 | match | mismatch (tr 424242 vs 2c2c2c, off 22; br 9e9e9e vs 868686, off 24) | clean | agree | picture: P5 |
| 32 | `[loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5` | clean (40,902 draws) | 0 | 4090 | 0.24 | match | mismatch (tr 424242 vs 2c2c2c, off 22; br 9e9e9e vs 868686, off 24) | clean | agree | picture: P5 |
| 33 | `[loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5` | refused: "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge) is not lowered yet" | 0 | - | - | absent | absent | refused, FS `3678` LOOP_FP16 | agree | |

**Disagreements: none.** `tally-host-verdicts.txt:122` reads "host predicts
16 clean, 17 refused, 0 without a line; 0 flagged". Each refused scene's board
text is the fixture's first-refusal text. The fixture predicts a refusal for
terrain, and the board refuses it too. The abort that follows comes from
glmark2 (T1) and is not a separate verdict. For jellyfish the board names
only the FS `8ee5` refusal: glmark2 stops at the first program that fails,
the gradient program, so the VS `08a2` refusal the fixture also lists is
never reached on the board.

## 3. Causes of every GL error, wrong picture and the terrain abort

glmark2 paths are in glmark2 `22c527cb` (`$GLMARK2_SRC`). YakoGL paths are
at this tree's tip. The compiler sites were found by compiling the three
terrain fragment shaders, plus shadow's, jellyfish's and ideas', in a scratch
copy of the tree with every refusal return instrumented. The tree was not
changed. The sources were glmark2's files with `ShaderSource::str()`'s
preamble rebuilt (`src/libmatrix/shader-source.cc:470-520`). The rebuilt
terrain shaders are 3252, 693 and 3998 bytes, the sizes the board log prints
for `glCompileShader(5)`, `(7)` and `(13)` (`run/log/23.out`).

| id | what | cause | file:line | owning plan | files the fix belongs in |
|---|---|---|---|---|---|
| T1 | terrain (23) abort: `stl_vector.h:1130 ... operator[] ... Assertion '__n < this->size()' failed` (`run/log/23.err`), no `yakogl-mali-exit` line | The main terrain program does not link: its FS `terrain.frag` is refused. `TerrainRenderer::init_program` returns early, at glmark2 `src/scene-terrain/terrain-renderer.cpp:91-92`, before `mesh_.set_attrib_locations` at `:136`, so the mesh's `attrib_locations_` (`src/mesh.h:104`, a `std::vector<int>`) stays empty. `TerrainRenderer::render` (`:52`) calls `Mesh::render_vbo`, and at `src/mesh.cpp:575` `attrib_locations_[i]` reads element 0 of the empty vector. Debian trixie's g++ 14 libstdc++ asserts there (SIGABRT). The trigger is the compiler refusal. glmark2 is not at fault for this run (its missing check is upstream's and gets no fix here). Refusal sites, measured: `terrain.frag` and `terrain-normalmap.frag` stop at `src/shaderc/online/mali/nir_fs.rs:7320` (the `Frcp` arm: `1.0 / x` of anything but a constant, here `length(lVector) / pointLightDistance[i]`, `gl_FragCoord.z / gl_FragCoord.w` and `1.0 / resolution.x`). `terrain-noise.frag` stops at `nir_fs.rs:7242` (the `Ffloor` arm: `floor` of a computed lane, simplex noise's `floor(v + dot(v, C.yy))`). The four TEX_MULTI shaders stop at the message `src/shaderc/online/mali/mod.rs:479-480` (`MaliError::TexCount`). The fixture names the three ARITH shaders `1637`, `4d26` and `c549`; which hash is which file was not established here (no corpus on this host). | `nir_fs.rs:7320`, `nir_fs.rs:7242`; glmark2 `src/mesh.cpp:575`, `src/scene-terrain/terrain-renderer.cpp:92` | plan #387 (compiler): `intval-rest` train, arith step 1a "division" (`docs/notes/mali-arith-lowering.md` section 4.1, the `Frcp` arm); `arith-lower-2`, arith step 3 "FS rounding" (`floor`); TEX_MULTI: the fmaloc design (`docs/notes/mali-fmaloc-lowering.md`, queue name `lower-fma`), with no open #387 lowering item | `src/shaderc/online/mali/nir_fs.rs` (the `Frcp` arm at 7313-7327, the `Ffloor`/`Fceil` arm at 7229-7246, the texture look-up arms), `tests/fixtures/mali_glmark2_verdicts.txt` (regenerated in the same commit), the arith `tests/mali_deqp_*.rs` pins |
| T2 | terrain (23): 16 GL error lines, 8 of `0x501 in glGetAttribLocation` and 8 of `0x501 in glGetUniformLocation` | Two of terrain's helper programs did not link: the simplex-noise one (`src/scene-terrain/simplex-noise-renderer.cpp:47-53`) and the normal-from-height one (`normal-from-height-renderer.cpp:70-73`). Their setup still looks up symbols, and each new name is `Program::operator[]` (`src/libmatrix/program.cc:340-360`), which calls `glGetAttribLocation` (`:258`) and then `glGetUniformLocation` (`:246`): 4 names after each of the 2 refusals, 2 calls a name, 16 lines. ES 2.0 requires `GL_INVALID_OPERATION` for a program that is not linked, and YakoGL gives it: `src/gl/program.rs:3440-3441` and `:2488-2489` (`if !p.linked`). This is correct GL behaviour, not a defect. | `src/gl/program.rs:3440`, `src/gl/program.rs:2488`; glmark2 `src/libmatrix/program.cc:346,351` | no fix; it goes away with T1's lowering (plan #387) | none (`src/gl/program.rs` is correct as it is) |
| P1 | desktop shadow (17): tr b1b1b1 against am62p and pi4 c0c0c0 (off 15) | Timing. The windows move with wall-clock `dt` (glmark2 `src/scene-desktop.cpp:928-929`), and the picture is read after the scene's last `update()` (`tools/cts/mali-glmark2-pbuffer.patch:133-134`), so a moving window's shadow edge lands elsewhere. The effect2d-finish item's board rerun with `--frame-end finish` confirmed timing (that rerun is not part of this log). The 2026-11-11 record saw the same probe at `959595`. The judgement itself is `tools/cts/common-tally.py:366-370` (a probe off by more than the tolerance). | glmark2 `src/scene-desktop.cpp:928`; `tools/cts/common-tally.py:366` | reference column, no fix | none; the `mali` column (`tools/cts/glmark2-pictures.txt:169`) holds this run's value |
| P2 | desktop blur (16): tr c0c0c0 against am62p ffffff (off 63) | Probably timing (a moving window at the last frame, `src/scene-desktop.cpp:928-929`). The T6 value equals the pi4 column (`glmark2-pictures.txt:144`, tr c0c0c0, and the run matches `pi4` in this scene). No `--frame-end finish` rerun reproduced the am62p value, so timing is not proven. The am62p column header (`glmark2-pictures.txt:94`) already notes that moving probes differ between runs. | `tools/cts/glmark2-pictures.txt:110` (am62p row), `:144` (pi4 row); glmark2 `src/scene-desktop.cpp:928` | reference column, no fix | none |
| P3 | buffer map (18, 20): probe c b4daf0 against am62p dcedf8 / d5eaf7 (off 40 / 33) | The am62p reference was sampled at another frame. The grid is displaced by elapsed time (glmark2 `src/scene-buffer.cpp:113-121`), and the am62p header (`glmark2-pictures.txt:94`) records that buffer probe c moves between runs of one image (off 16, 36 and 33 against `bc43ea09`). The T6 value is exactly the pi4 column's (`b4daf0f0`, `glmark2-pictures.txt:146,148`), and the means agree (42 57 65 against 42 57 66). | `tools/cts/glmark2-pictures.txt:94,112,114`; glmark2 `src/scene-buffer.cpp:113` | reference column, no fix | none |
| P4 | conditionals vertex-steps=5 (28): tr 000000 against fdfdfd (off 253) | Timing. The grid rotates by `36 * elapsed` degrees (glmark2 `src/scene-grid.cpp:94`), so the tr probe sits on the rotating grid's edge: black on one frame, white on another. The effect2d-finish item's `--frame-end finish` rerun confirmed timing. Note that the `mali` column header (`glmark2-pictures.txt:162`) puts this probe under P4 "mediump fp16". That wording disagrees with the rerun and should say timing (new issue N4). | glmark2 `src/scene-grid.cpp:94`; `tools/cts/glmark2-pictures.txt:162` | reference column, no fix (header wording: N4) | `tools/cts/glmark2-pictures.txt` (header comment only) |
| P5 | function low (29), loop fragment-loop=false (31), loop fragment-uniform=false (32): tr 424242 against 2c2c2c (off 22), br 9e9e9e against 868686 (off 24), bl 5e5e5e against 636363 (off 5, within tolerance); identical in all three | mediump runs at fp16. `function.frag` and `loop.frag` compute `float d = fract(FragCoord.x * FragCoord.y * 0.0001)` with a highp `FragCoord` copy but a mediump `d`, then apply `d = fract(3.0 * d)` five times (`function-step-low.all`, `loop-step-simple.all`). Each step triples the rounding error of fp16's 2^-11 ulp near 0.5, so 3^5 = 243 times that error is up to about 0.12, about 30 levels. The AM62P and Pi 4 evaluate mediump at fp32. The Mali compiler, like Mesa's, lowers mediump fragment arithmetic to fp16 (`src/shaderc/online/mali/mod.rs:1011-1020`, the width rule; `nir_fs.rs:828-836`, the highp `FragCoord` copy kept at fp32). ES 2.0 permits this. | `src/shaderc/online/mali/mod.rs:1011`, `src/shaderc/online/mali/nir_fs.rs:828` (read only) | reference column, no fix (not #406 or #402/#387: the fp16 path is Mesa's behaviour) | none; the `mali` column is the reference for these scenes |

No GL error comes from any other scene. No draw was refused in a clean
scene (`refused=0` in every exit line), and no kernel line or GPU fault
appeared.

## 4. Refusal map (17 scenes that are not clean)

Classes as in `docs/notes/mali-glmark2-forecast.md` section 2. "Item" is the
plan #402/#387 item that lowers it: item ids and titles were read from the
connected plans, and the design notes from `docs/notes/`. Where neither plan
has an open lowering item, the cell says so, with the forecast's queue name
from plan #367.

| # | scene | class | shaders (first refusal per stage) | measured site | unblocking item |
|---:|---|---|---|---|---|
| 3, 4, 5 | texture nearest, linear, mipmap | TEX_COORD | FS `425b` (link `ace2+425b`) | message `mod.rs:482-483` (`MaliError::TexCoord`) | plan #387 fmaloc design step 7 `tc_2d_scalar` (`docs/notes/mali-fmaloc-lowering.md:314-318`). Not in the `intval-rest` train (its T1 is the cube coordinate path); no open lowering item (queue name `lower-texcoord-425b`) |
| 7 | shading blinn-phong-inf | NONFINITE_CONST | FS `994f` | message `mod.rs:464-465` (`MaliError::ConstantFold`) | plan #402 `lower-vsconst`, only if its FS step covers this shape (the glmark2-impact item's caveat); otherwise **new issue N1** |
| 8 | shading phong | ARITH_CONST | FS `8a64` | message `mod.rs:458-459` (`MaliError::ArithConstant`) | plan #387 `intval-rest` train, step "negzero/constprod" (`docs/notes/mali-constprod-lowering.md`; queue name `lower-constprod-rest`) |
| 9 | shading cel | NONFINITE_CONST | FS `fea0` | `mod.rs:464-465` | as 7: #402 `lower-vsconst` if FS, else N1 |
| 10, 11, 12 | bump high-poly, normals, height | NONFINITE_CONST | FS `d442`, `487d`, `77c5` | `mod.rs:464-465` | as 7: #402 `lower-vsconst` if FS, else N1 |
| 13 | effect2d 3x3 | TEX_MULTI | FS `3c3d` | message `mod.rs:479-480` (`MaliError::TexCount`) | plan #387 fmaloc design (`docs/notes/mali-fmaloc-lowering.md`, `docs/notes/mali-tex-multi.md`); no open lowering item (queue name `lower-fma`) |
| 21 | ideas | INT_VALUE | FS `1aca`, `c565` (`ideas-lamp-lit.frag`, `ideas-logo.frag`: `v1.w == 0.0 && v2.w == 0.0`, a boolean `&&` of two compares) | `nir_fs.rs:2802` (the integer arm's catch-all `_ => Err(MaliError::IntValue)`) | plan #402 `lower-fsint` (FS integer and boolean values) |
| 22 | jellyfish | FS_OPERAND + ARITH_CONST | FS `8ee5` (`gradient.frag`: `mix(color1, color2, uv.x * uv.y)`, a uniform first in the sum `mix` makes); VS `08a2` ARITH_CONST | `nir_fs.rs:11783` (`MaliError::FsOperandForm`, the operand-form check of the vectorised colour) | FS: **new issue N2** (no #402/#387 item); VS: #387 `intval-rest` constprod step |
| 23 | terrain | ARITH + TEX_MULTI | FS `1637`, `4d26`, `c549` ARITH; FS `8c86`, `bbba`, `bdfd`, `e043` TEX_MULTI | `nir_fs.rs:7320` (`terrain.frag`, `terrain-normalmap.frag`), `nir_fs.rs:7242` (`terrain-noise.frag`) | #387 `intval-rest` arith step 1a (division) and `arith-lower-2` arith step 3 (`floor`), plus the TEX_MULTI lowering (none open). The abort (T1) goes when the main program links |
| 24 | shadow | ARITH | FS `cf4b` (`shadow.frag`: `ShadowCoord / ShadowCoord.w`, division by a varying lane) | `nir_fs.rs:7320` (the same `Frcp` arm as terrain) | #387 `intval-rest` arith step 1a (division), **if** its `Frcp` of a varying lane (`other_frcp_fs_none` shape, `mali-arith-lowering.md:406-407`) covers a vec4 divided by its own `.w`. The forecast note says no item covers `cf4b`, but the measured site belongs to step 1a; N3 records the check |
| 25 | refract | ARITH_CONST | FS `3fae` | `mod.rs:458-459` | #387 `intval-rest` constprod step |
| 30 | function medium | FRAGCOORD_FP16 | FS `849c` (`fract(sqrt(d))`, no one-lane fp16 `Fsqrt`, `docs/notes/mali-fragcoord-fp16.md:151`) | message `mod.rs:446-447` (`MaliError::FragCoordMediump`) | plan #387 texmisc design (`docs/notes/mali-texmisc-lowering.md`) or arith step 3 (`sqrt`, `arith-lower-2`); no open lowering item names `849c` (queue name `lower-mploop`) |
| 33 | loop fragment-uniform=true | LOOP_FP16 | FS `3678` | message `mod.rs:503-504` (`MaliError::LoopMediump`) | plan #387 `intval-rest` train, step "loops" (`docs/notes/mali-loops-lowering.md:573`) |

A lowering can reveal a later refusal behind the first one (the fixture pins
only the first), so the counts below are an upper bound. Lowering division
(step 1a) alone frees shadow (24) and two of terrain's three ARITH shaders.
NONFINITE_CONST frees five scenes (7, 9-12), the largest single gain.

## 5. Replacement texts

Paste-ready. The files are not edited by this item.

### 5.1 `tools/cts/RESULTS.md` (plan #387), summary-table row "NanoPC-T6 (Mali)"

```
| NanoPC-T6 (Mali) | no score (16 of 33 scenes have an FPS), 33 of 33 scenes ran, 17 failed (16 refused by the online compiler, terrain aborts in glmark2), 16 GL error lines (all terrain), 0 out of GPU memory; main `038b7d7d`, 2026-11-25, on the board's own Armbian trixie (kernel 6.18.55-current-rockchip64, panthor 1.5.0), `tools/cts/mali-glmark2-session.sh` over the `tools/cts/mali-glmark2-build.sh` tree (`dev=no`; one glmark2-es2 process a scene, 800x600 on an EGL pbuffer of the `tools/mali/panthor-gles` pair); **16 scenes clean (build x2, shading gouraud, effect2d 5x5, pulsar, desktop x2, buffer x3, conditionals x3, function low, loop x2), every picture matching the `mali` column; 16 refused at compile or link (#33); terrain aborts in glmark2 after its refusals**; every scene as the host forecast; log `tools/cts/log/2026-11-25-t6-glmark2-038b7d7d/`; per scene under "Mali: NanoPC-T6 ... glmark2-es2" below | see "Mali: NanoPC-T6 (RK3588, Valhall v10, panthor), gl_test sweep" | 0 (0 panthor kernel lines in every scene; same boot_id before and after; /boot and loaded modules unchanged, U-Boot env not compared: no fw_printenv on the board) |
```

### 5.2 `tools/cts/RESULTS.md` (plan #387), section "Mali: NanoPC-T6 glmark2-es2 (stage 2 criterion)", whole body

```
### Mali: NanoPC-T6 glmark2-es2 (stage 2 criterion)

Main `038b7d7d`, 2026-11-25, the board's own Armbian trixie (kernel
6.18.55-current-rockchip64, panthor 1.5.0): `tools/cts/mali-glmark2-session.sh`
(`tools/cts/mali-glmark2-run.sh` between two `tools/cts/mali-board-state.sh`
snapshots) over the `tools/cts/mali-glmark2-build.sh` tree (`dev=no`, glmark2
`22c527cb` with `mali-glmark2-pbuffer.patch`, the `tools/mali/panthor-gles`
pair; every process's `yakogl-mali: loaded` line names the staged
libGLESv2.so.2, `build=038b7d7d`). Board state, before against after: `/boot`
unchanged (419 files), loaded modules unchanged (100), U-Boot env not compared
(the board has no `fw_printenv`), boot_id unchanged. One glmark2-es2 process a
scene, 800x600 on an EGL pbuffer, 180 s bound, the 33 default scenes in order.
Tallied by `tools/cts/glmark2-mali-tally.py`. The redacted log is
`tools/cts/log/2026-11-25-t6-glmark2-038b7d7d/` (its `report.md` has the
causes and the refusal map).

| Scenes | ok | refused (compile or link) | crash | GL error lines | out of GPU memory | panthor kernel lines | boot_id |
|---:|---:|---:|---:|---:|---:|---:|---|
| 33 of 33 ran | **16** | 16 | 1 | 16 | 0 | 0 | unchanged |

- The outcome is the host forecast's, scene for scene
  (`tests/fixtures/mali_glmark2_verdicts.txt`,
  `docs/notes/mali-glmark2-forecast.md` section 1: 16 clean, 17 not). No
  clean scene refused a draw, faulted or timed out (`refused=0
  gpu_faults=0 job_timeouts=0` in every exit line). The 16 clean scenes sum
  to 11.40 ms a frame. No score: terrain and the 16 refused scenes have no
  FPS.
- Pictures: all 16 clean scenes match the `mali` column of
  `tools/cts/glmark2-pictures.txt` (made from this run, not looked at by
  eye). Against the AM62P column 8 match. The 8 that differ are timing
  (desktop shadow and conditionals vertex-steps=5, confirmed by a
  `--frame-end finish` rerun; desktop blur probably, where the T6 equals the
  Pi 4 column; buffer map with interleave=false and true, where the T6
  equals the Pi 4 column and the AM62P column's probe c moves between runs)
  or mediump at fp16 (function low and loop fragment-loop=false and
  fragment-uniform=false: five `d = fract(3.0 * d)` steps triple the fp16
  error each step, off 22 and 24 at tr and br). None is a defect.
- The 16 refused scenes stop at the online compiler's first refusal (#33):
  5 a constant operation on a value that is not a finite normal float
  (shading blinn-phong-inf and cel, bump x3), 3 a texture coordinate other
  than a whole vec2 varying (texture x3, at the link), 2 a constant product
  Mesa moves or folds (shading phong, refract), 1 fragment arithmetic
  (shadow: a division by a varying lane), 1 a texture look-up beside another
  (effect2d 3x3), 1 an integer or boolean value (ideas), 1 a fragment
  operand form (jellyfish's gradient program), 1 fp16 arithmetic on
  gl_FragCoord (function medium), 1 a kept loop in a mediump fragment
  shader (loop fragment-uniform=true). Each prints `Set up failed` and
  draws nothing.
- terrain: `glmark2-es2-mali` aborts in libstdc++'s
  `std::vector<int>::operator[]` assertion. Three of its fragment shaders are
  refused as arithmetic (division and `floor` of a computed value) and four
  as texture look-ups beside another. The main terrain program therefore
  never links, `TerrainRenderer::init_program` returns before
  `set_attrib_locations`, and `Mesh::render_vbo` reads the empty location
  vector (glmark2 `src/mesh.cpp:575`). It is a consequence of the refusals,
  in glmark2, not a YakoGL crash. Its 16 GL error lines are `0x501` from
  `glGetAttribLocation`/`glGetUniformLocation` on the two helper programs
  that did not link, which is what ES 2.0 requires.
- Against the previous record (main `a57d8104`, 2026-11-11: 1 clean, 31
  refused, terrain abort, 80 GL error lines), 15 scenes moved to clean,
  and desktop blur and the three buffer scenes no longer run on without
  their program. Desktop shadow went from 0.96 to 0.97 ms a frame.
- `D-DEPTHONLY` passes in desktop shadow (17). `D-NPOTMIP` (refract) and
  `D-DEPTHTEX` (shadow, refract) are still not reached: both scenes stop at
  a compile refusal first.

Per scene (ms a frame is glmark2's FrameTime; a refused scene has none;
picture against the `mali` column, then against `am62p`):

| # | Scene | ms/frame | FPS | Verdict | Picture (mali / am62p) | panthor lines |
| ---: | --- | ---: | ---: | --- | --- | ---: |
| 1 | `[build] use-vbo=false` | 0.81 | 1237 | **ok** | match / match | 0 |
| 2 | `[build] use-vbo=true` | 0.28 | 3544 | **ok** | match / match | 0 |
| 3 | `[texture] texture-filter=nearest` | - | - | refused at link: FS texture coordinate | absent | 0 |
| 4 | `[texture] texture-filter=linear` | - | - | refused at link: FS texture coordinate | absent | 0 |
| 5 | `[texture] texture-filter=mipmap` | - | - | refused at link: FS texture coordinate | absent | 0 |
| 6 | `[shading] shading=gouraud` | 0.34 | 2933 | **ok** | match / match | 0 |
| 7 | `[shading] shading=blinn-phong-inf` | - | - | refused: FS constant on a non-finite float | absent | 0 |
| 8 | `[shading] shading=phong` | - | - | refused: FS constant product | absent | 0 |
| 9 | `[shading] shading=cel` | - | - | refused: FS constant on a non-finite float | absent | 0 |
| 10 | `[bump] bump-render=high-poly` | - | - | refused: FS constant on a non-finite float | absent | 0 |
| 11 | `[bump] bump-render=normals` | - | - | refused: FS constant on a non-finite float | absent | 0 |
| 12 | `[bump] bump-render=height` | - | - | refused: FS constant on a non-finite float | absent | 0 |
| 13 | `[effect2d] kernel=0,1,0;1,-4,1;0,1,0;` | - | - | refused: FS texture look-up beside another | absent | 0 |
| 14 | `[effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | 0.38 | 2632 | **ok** | match / match | 0 |
| 15 | `[pulsar] light=false:quads=5:texture=false` | 0.23 | 4373 | **ok** | match / match | 0 |
| 16 | `[desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4` | 1.38 | 723 | **ok** | match / mismatch (timing) | 0 |
| 17 | `[desktop] effect=shadow:windows=4` | 0.97 | 1029 | **ok** | match / mismatch (timing) | 0 |
| 18 | `[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map` | 2.14 | 468 | **ok** | match / mismatch (timing) | 0 |
| 19 | `[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata` | 1.32 | 758 | **ok** | match / match | 0 |
| 20 | `[buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map` | 2.06 | 487 | **ok** | match / mismatch (timing) | 0 |
| 21 | `[ideas] speed=duration` | - | - | refused: FS integer or boolean value | absent | 0 |
| 22 | `[jellyfish] <default>` | - | - | refused: FS operand form | absent | 0 |
| 23 | `[terrain] <default>` | - | - | crash: glmark2 aborts after its refusals (16 GL errors) | absent | 0 |
| 24 | `[shadow] <default>` | - | - | refused: FS arithmetic (division) | absent | 0 |
| 25 | `[refract] <default>` | - | - | refused: FS constant product | absent | 0 |
| 26 | `[conditionals] fragment-steps=0:vertex-steps=0` | 0.26 | 3885 | **ok** | match / match | 0 |
| 27 | `[conditionals] fragment-steps=5:vertex-steps=0` | 0.24 | 4166 | **ok** | match / match | 0 |
| 28 | `[conditionals] fragment-steps=0:vertex-steps=5` | 0.24 | 4098 | **ok** | match / mismatch (timing) | 0 |
| 29 | `[function] fragment-complexity=low:fragment-steps=5` | 0.26 | 3924 | **ok** | match / mismatch (fp16) | 0 |
| 30 | `[function] fragment-complexity=medium:fragment-steps=5` | - | - | refused: FS fp16 gl_FragCoord | absent | 0 |
| 31 | `[loop] fragment-loop=false:fragment-steps=5:vertex-steps=5` | 0.25 | 4050 | **ok** | match / mismatch (fp16) | 0 |
| 32 | `[loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5` | 0.24 | 4090 | **ok** | match / mismatch (fp16) | 0 |
| 33 | `[loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5` | - | - | refused: FS kept loop in mediump | absent | 0 |
```

### 5.3 `tools/cts/STATUS.md`, the Japanese T6 glmark2 line (replaces line 17)

```
- **NanoPC-T6（RK3588、Mali-G610、panthor）の glmark2-es2、origin/main 038b7d7d。** ボード自身の Armbian trixie（kernel 6.18.55-current-rockchip64）で tools/cts/mali-glmark2-session.sh により 1 シーン 1 プロセス、800x600 の EGL pbuffer（tools/mali/panthor-gles の対）で既定の 33 シーンを実行した（2026-11-25、記録は tools/cts/log/2026-11-25-t6-glmark2-038b7d7d）。実行 33/33、正常 16（build 2、shading gouraud、effect2d 5x5、pulsar、desktop 2、buffer 3、conditionals 3、function low、loop 2）、オンラインコンパイラの拒否 16（非有限定数 5、テクスチャ座標 3（リンク時）、定数積 2、fragment の除算 1（shadow）、並んだテクスチャ参照 1、整数・真偽値 1（ideas）、fragment のオペランド形 1（jellyfish）、gl_FragCoord の fp16 算術 1、mediump の残るループ 1。いずれも #33）、terrain は拒否の後に glmark2 自身が abort（リンクしなかったプログラムで空の属性位置の vector を引く）。GL エラー 16 行（すべて terrain、リンクしていないプログラムに ES 2.0 が返す 0x501）、GPU メモリ不足 0、panthor のカーネルメッセージ 0 件、GPU フォールト 0。実行の前後で /boot（419 ファイル）と読み込み済みモジュール（100）は不変、boot_id も不変。U-Boot の環境変数はボードに fw_printenv が無いため比較していない。ホストの予測（tests/fixtures/mali_glmark2_verdicts.txt、docs/notes/mali-glmark2-forecast.md）とシーンごとにすべて一致。正常な 16 シーンの絵はすべて glmark2-pictures.txt の mali 列（この実行から作成、目では未確認）と一致し、AM62P 列とは 8 シーンが一致、8 シーンが違う（動くシーンの最後のフレームの時刻の差 5、mediump を fp16 で計算する差 3。どれも不具合ではない）。正常 16 シーンの 1 フレームの時間の合計は 11.40 ms（desktop shadow は 0.97 ms）。FPS の無いシーンがあるのでスコアは無し。前の記録（a57d8104、11/11: 正常 1、拒否 31、GL エラー 80 行）から 15 シーンが正常になった。
```

### 5.4 `README.md` Status (plan #392): a paragraph after the glmark2 score table

```
The fourth back end's board, the NanoPC-T6 (Mali-G610, `gpu-mali`, not one
of the three boards above), runs glmark2-es2 only on its own Linux through
panthor so far. At main `038b7d7d` (2026-11-25) 16 of the 33 scenes run
clean, with every picture matching its own reference column. 16 are refused
by the online shader compiler and terrain aborts in glmark2 after its
refusals, so it has no score yet (`tools/cts/RESULTS.md`, "Mali: NanoPC-T6
glmark2-es2"; issue #33).
```

### 5.5 `ROADMAP.md` stage-5 glmark2 bullet (plan #389), replacing the `08ae7a8e` / `4bf0e7ab` bullet

```
- glmark2-es2 (main `038b7d7d`, 2026-11-25,
  `tools/cts/log/2026-11-25-t6-glmark2-038b7d7d/`): no score, 33 of 33
  scenes ran, 16 clean (every picture matching the `mali` column), 16
  refused by the online compiler and terrain aborting in glmark2 after its
  refusals; 16 GL error lines, all terrain's; every scene as the host
  forecast (#33). The earlier records (`da65355e`, `a57d8104`) had 1 clean
  scene.
```

## 6. Issue comments

### 6.1 For #33

```
T6 glmark2-es2 re-measured at origin/main 038b7d7d (2026-11-25, plan #409; log tools/cts/log/2026-11-25-t6-glmark2-038b7d7d/, commit 2a25a7bb, report.md there).

- 33 of 33 scenes ran: 16 clean, 16 refused by the online compiler, terrain aborts in glmark2 after its refusals. 16 GL error lines, all terrain (0x501 on unlinked programs). No score. 0 panthor lines, 0 GPU faults, boot_id unchanged.
- Every scene agrees with tests/fixtures/mali_glmark2_verdicts.txt (16 clean, 17 not, same first refusal texts).
- The 16 clean scenes' pictures all match the new `mali` column. Against am62p, 8 differ: timing (desktop x2, buffer map x2, conditionals vertex-steps=5) and mediump at fp16 (function low, loop x2). None is a defect.
- Against a57d8104 (1 clean, 31 refused, 80 GL error lines), 15 scenes moved to clean.

Still refused, by class, with the measured site and the item that unblocks it:
- NONFINITE_CONST (shading blinn-phong-inf, cel; bump x3): mod.rs:464 ConstantFold. Plan #402 `lower-vsconst` only if its FS step covers it, otherwise a new issue.
- TEX_COORD (texture x3, link ace2+425b): fmaloc design step 7 `tc_2d_scalar`, no open lowering item.
- ARITH_CONST (shading phong, refract, jellyfish VS 08a2): plan #387 intval-rest constprod step.
- ARITH: shadow cf4b and terrain's terrain.frag/terrain-normalmap.frag stop at nir_fs.rs:7320 (1.0/x of a run-time value), which is plan #387 arith step 1a (division). terrain-noise.frag stops at nir_fs.rs:7242 (floor of a computed lane), plan #387 arith step 3 (arith-lower-2).
- TEX_MULTI (effect2d 3x3, terrain x4): fmaloc design, no open lowering item.
- INT_VALUE (ideas 1aca, c565: a boolean && of two compares, nir_fs.rs:2802): plan #402 `lower-fsint`.
- FS_OPERAND (jellyfish 8ee5 = gradient.frag, nir_fs.rs:11783): no item.
- FRAGCOORD_FP16 (function medium 849c): no open lowering item.
- LOOP_FP16 (loop fragment-uniform=true 3678): plan #387 intval-rest loops step.

The terrain abort is glmark2's own (src/mesh.cpp:575 reads an empty attribute-location vector because TerrainRenderer::init_program returned after the refusal). It goes away when terrain.frag compiles.
```

### 6.2 For #7

```
NanoPC-T6, stage 5 benchmark criterion (glmark2-es2), re-measured at origin/main 038b7d7d on the board's own Armbian through panthor (2026-11-25, plan #409; tools/cts/log/2026-11-25-t6-glmark2-038b7d7d/).

33 of 33 scenes ran. 16 are clean, with every picture matching the T6's `mali` reference column. 16 are refused by the online compiler (#33), and terrain aborts in glmark2 after its refusals. No score yet. No kernel line, GPU fault or reset; board state unchanged. Every outcome is the host forecast's. This replaces the 2026-11-11 record (a57d8104: 1 clean scene).

Still open for the glmark2 criterion: the compiler refusals of #33 (plans #402 and #387), then a second rerun that has all 33 scenes with an FPS. After that, the T6's glmark2 under Zephyr and from U-Boot (after #408), as the other boards record it.
```

## 7. New issues (causes with no owner)

### N1

Title: `gpu-mali: FS constant operations on non-finite values block glmark2 shading and bump (5 scenes)`

```
glmark2-es2 on the NanoPC-T6 at origin/main 038b7d7d (tools/cts/log/2026-11-25-t6-glmark2-038b7d7d/) refuses five scenes with "Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet" (MaliError::ConstantFold, src/shaderc/online/mali/mod.rs:464): shading blinn-phong-inf (FS 994f), shading cel (fea0), bump high-poly (d442), normals (487d), height (77c5). This is the largest single class left in glmark2: lowering it frees 5 of the 17 scenes that are not clean, if no later refusal sits behind it.

Plan #402's `lower-vsconst` covers the vertex-shader form. It covers these fragment shaders only if its FS step reaches glmark2's mediump `pow(dot-bounded base, ...)` shape with a byte-equal Mesa reference. If it stays VS-only, these need their own item: Mesa 26.2.1 references of the five shaders, the raise site of each (one of the 14 `MaliError::ConstantFold` returns in src/shaderc/online/mali/), the lowering, and the moved lines of tests/fixtures/mali_glmark2_verdicts.txt in the same commit.
```

### N2

Title: `gpu-mali: jellyfish's gradient FS refused as an operand form (mix of two uniforms)`

```
glmark2's jellyfish scene stops at its first program, GradientRenderer (glmark2 22c527cb src/scene-jellyfish.cpp:96-110, shaders/gradient.frag): `gl_FragColor = vec4(mix(color1, color2, uv.x * uv.y), 1.0)` with two vec3 uniforms. The Mali online compiler refuses it with "a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first)". The site, measured by compiling the shader in an instrumented scratch copy at the tip, is src/shaderc/online/mali/nir_fs.rs:11783 (`MaliError::FsOperandForm`, the admitted-product check of the vectorised colour). Fixture line: `21 jellyfish fs 8ee5`. No item of plans #402/#387 covers it.

Needed: a Mesa 26.2.1 reference of gradient.frag (and a dEQP shape with a uniform first in a mediump sum, if one exists), the admitted form at nir_fs.rs:11760-11784, the moved fixture line. Behind it, jellyfish's VS 08a2 (ARITH_CONST) waits for the constprod step of plan #387's intval-rest train, and jellyfish.frag itself already compiles.
```

### N3

Title: `gpu-mali: check that arith step 1a (division) covers glmark2 shadow's ShadowCoord / ShadowCoord.w and terrain's divisions`

```
The forecast note (docs/notes/mali-glmark2-forecast.md section 5) lists shadow's FS cf4b as an ARITH refusal with no lowering item. Measured at the tip by compiling glmark2 22c527cb shaders/shadow.frag in an instrumented scratch copy, cf4b stops at src/shaderc/online/mali/nir_fs.rs:7320: the `Frcp` arm refuses `1.0 / x` of anything but a constant. Here x is a varying's w (`vec4 sc_perspective = ShadowCoord / ShadowCoord.w`). terrain.frag (`length(lVector) / pointLightDistance[i]`, `gl_FragCoord.z / gl_FragCoord.w`) and terrain-normalmap.frag (`1.0 / resolution.x`, a uniform) stop at the same line. terrain-noise.frag stops at nir_fs.rs:7242 (`floor` of a computed lane).

That is the site of plan #387's arith step 1a (division, docs/notes/mali-arith-lowering.md section 4.1, references other_frcp_fs_none and random_frcp_fs_mediump). Step 1a's references are scalar `1.0 / x` of a varying lane. Whether they cover a vec4 divided by its own lane, a uniform array element and gl_FragCoord.w needs one host check after the train lands: rerun tests/mali_glmark2_corpus.rs and read lines `23 shadow fs cf4b` and `22 terrain fs 1637/4d26/c549`. If any stays at nir_fs.rs:7320, it needs its own reference and lowering. Shadow is then the only scene left behind this class (terrain also needs TEX_MULTI).
```

### N4

Title: `tools/cts: glmark2-pictures.txt mali header puts conditionals vertex-steps=5 under fp16, a rerun says timing`

```
The `mali` column header of tools/cts/glmark2-pictures.txt (line 162) explains the am62p mismatch of `[conditionals] fragment-steps=0:vertex-steps=5` (probe tr 000000 against fdfdfd, off 253) as mediump fp16 ("P4 ... tr of conditionals vertex-steps=5 off 253 where an edge falls on another pixel"). Plan #409's effect2d-finish board rerun with `--frame-end finish` attributes it to timing instead. The grid rotates by 36 * elapsed degrees (glmark2 22c527cb src/scene-grid.cpp:94), so the corner probe sits on the rotating grid's edge. The precision explanation applies to function low and loop x2 (five `d = fract(3.0 * d)` steps), not to the conditionals scene, whose fragment-steps=0 shader has no such chain. The fix is to move scene 28 from P4 to the timing group in that header comment, with no change to any value line. The owner of glmark2-pictures.txt should take it in its own item, with tools/cts/glmark2-pictures-column-selftest.sh.
```

### N5

Title: `tools/cts: glmark2-mali-tally.py prints refusal texts with their numbers masked ("more than # look-ups")`

```
The per-scene verdict text in tally-table.txt and tally-results-row.txt (tools/cts/log/2026-11-25-t6-glmark2-038b7d7d/) reads "... or more than # look-ups, is not lowered yet", while run/scenes.tsv and the compiler say "64". The cause key `normalize()` (tools/cts/glmark2-mali-tally.py:240-243) replaces every number with `#`. That is right for grouping causes, but the key leaks into the text that `--results-row` prints and that RESULTS.md pastes. The fix is to keep the raw first-refusal text for display and use normalize() only as the grouping key, pinned in tools/cts/glmark2-mali-tally-selftest.sh. The draft-issue text of tally-causes.txt also calls this "the first glmark2-es2 run on the NanoPC-T6", but it is the third. A tooling fix lands in its own item with its selftest.
```

## Notes on the evidence

- `/root` appears in 35 files of this directory, in three forms: `/root`
  and `/root/yakogl-mali` (`preflight.txt:14`, the stage-space check) and
  `/root/yakogl-mali-glmark2/038b7d7d-<stamp>/lib/libGLESv2.so.2` (the
  `yakogl-mali: loaded` lines). It is the board superuser's home and names
  no person, so no redaction is needed. The one build-host path in the log
  is already redacted as `<home>` (`run/log/23.err`).
- What this report did not establish: which fixture hash (`1637`, `4d26`,
  `c549`) is which terrain file, because the generated glmark2 corpus is not
  on this host. The `--frame-end finish` reruns of P1 and P4 come from the
  effect2d-finish item and are not in this log.
