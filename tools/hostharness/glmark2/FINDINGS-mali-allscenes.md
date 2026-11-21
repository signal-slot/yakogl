<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2-es2 on the Mali back end: refusals, GL errors and incomplete textures, all 33 scenes

The 33 default scenes of glmark2-es2 on the `gpu-mali` build (the back end
the NanoPC-T6's Mali-G610 runs) on the host harness: the GL core, the
in-process compiler (`src/shaderc/online/mali`) and the back end's command
generation (`src/mali`) run; nothing is rendered and no command stream is
executed (`README.md`, GPUs, gpu-mali). What it gives per scene is the
frame-side half of a board forecast: which shader the compiler refuses,
which draw the back end refuses, which GL error the client sees and from
which entry point, which draw samples an incomplete texture.

## Run

On main `7b2246c0` (`tools/mali: Mesa 26.2.1 drm-shim references for the
remaining constant products of glmark2 (gm2_cprod_1..6)`), glmark2 at
`22c527cb0556f3a1ac4445aaa52cc532760928d5` (a git checkout, verified by
`run.sh`):

```sh
tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-mali mali
```

The build: `MALI_CONFIG`'s features of `tools/check.sh` (`gpu-mali`,
`shaderc-online`, `no-shader-table`) with `osal-linux` and `report-es2`
(`mali/CMakeLists.txt`). `shim.c`'s `GM2_MALI` table answers the Mali OSAL
contract (the T6's counts, a tiler heap in linux.c's memory, submits that
complete at once) and lends the compiler its scratch (4 MiB) and keep
(512 KiB) regions, as the panthor pair the board runs does: a
`no-shader-table` build without `host-test` has no built-in ones, and
without the lend every compile is refused ("no keep region"). The back end
came up in every process (`mali: gpu_id 0xa8670005, ..., 92 user
registers`) and built its clear and preload programs; nothing in it needed
a change to run here.

The previous record of this file was made at main `d09629d3` (2 clean
scenes). Plans #309, #336 and #367's lowering items moved the compiler since:
the constant-product, fragment-arithmetic, vertex-arithmetic and FMA
families, locals, texture swizzle, the vertex texture look-ups, the FAU page,
the int-counter loops and the gl_FragCoord fp16 forms no longer stop a
scene. `tests/mali_glmark2_corpus.rs` pins the first refusal of every
shader and program of the corpus (`tests/fixtures/mali_glmark2_verdicts.txt`);
every scene's row below agrees with it, and
`docs/notes/mali-glmark2-forecast.md` sections 1 and 2 give the per-shader
moves.

## Result

**14 of 33 scenes are clean** (status ok, no refused draw, no GL error, no
incomplete texture): 0 and 1 (`build`), 5 (`shading=gouraud`), 14 (`pulsar`),
16 (`desktop` shadow), 17, 18 and 19 (`buffer`), 25, 26 and 27
(`conditionals`), 28 (`function` low), 30 and 31 (`loop`). Their one line in
the table is the `glmark2-picture` line every GPU's run logs, black as
nothing is rendered; each ran 3 frames (3 draws for the one-object scenes,
15 for `pulsar`, 107 for `desktop` shadow). 15 (`desktop` blur) finishes
with 352 GL errors over 56 draws, so it is not clean. 17 scenes fail at
setup (glmark2 exits with status 3 after a shader it needs is refused: 2-4,
6-13, 20, 21, 23, 24, 29, 32), and terrain (22) crashes in glmark2 itself.
No draw was refused by the back end, no draw sampled an incomplete texture,
no framebuffer was incomplete, no GPU memory ran out, and no submit was
refused by the OSAL mock, in any scene. The depth-only framebuffer of
`desktop` shadow (16, the shadow-map pass) is complete and its 107 draws
run (`D-DEPTHONLY` is closed on the host).

The one deviation from "refusals only" is scene 5's single finding,
`yakogl: program <n> has no uniform 'ModelViewMatrix'`: a glmark2 uniform
look-up of a name the linked program does not have, answered with location
-1 and no GL error; the scene is clean.

Every failure has one root cause: **the Mali code generator
(`src/shaderc/online/mali`) refuses a shader**. That refusal work is plan
#367's lowering items (`src/shaderc/online/mali`); this file only lists
which glmark2 scene each refusal stops. The GL errors and the crash are
what glmark2 does next with a program that did not link.

### Compiler refusals, by cause

The first refusal glmark2 meets in each failing scene (the corpus fixture's
first refusal per shader; a vertex shader before its fragment shader; 15 and
terrain go on after a refusal and meet several), with the text after
`Mali(Valhall: ` in the compile log and the family of
`docs/notes/mali-glmark2-forecast.md` section 2; scenes are entry numbers of
the table below.

| code | cause (the compiler's words) | stage, shaders | scenes | count |
|---|---|---|---|---|
| `TEX_COORD` | a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet | fragment `425b` (the program fails to link) | 2, 3, 4 | 3 |
| `CONST_FOLD` | a constant operation on or to a value that is not a finite normal float is not lowered yet | fragment `994f`, `fea0`, `d442`, `487d`, `77c5` | 6, 8, 9, 10, 11 | 5 |
| `ARITH_CONST` | a constant product in a shape Mesa's algebraic rules move or fold is not lowered yet | fragment `8a64` (phong), `3fae` (refract) | 7, 24 | 2 |
| `TEX_MULTI` | a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet | fragment `3c3d`, `ba8e` (effect2d); `68af`, `7f17` (blur, 16 refusals each); terrain `8c86`, `bbba`, `bdfd`, `e043` | 12, 13, 15, 22 | 4 |
| `ARITH` | arithmetic other than +, -, * and abs is not lowered yet | fragment `cf4b` (shadow); terrain `1637`, `4d26`, `c549` | 22, 23 | 2 |
| `CONTROL_FLOW` | control flow (if, a loop, a return before the end) is not lowered yet | vertex `51b2` (ideas) | 20 | 1 |
| `FS_OPERAND` | a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet | fragment `8ee5` (jellyfish) | 21 | 1 |
| `FRAGCOORD_FP16` | fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y is not lowered yet | fragment `849c` (function medium) | 29 | 1 |
| `LOOP_MEDIUMP` | a loop Mesa keeps (a run-time bound) in a mediump fragment shader is not lowered yet | fragment `3678` | 32 | 1 |

Behind them, in shaders glmark2 does not reach yet: ideas `1aca` and `c565`
(`INT_VALUE`) and `df64` (`TEX_COORD`), jellyfish vertex `08a2`
(`ARITH_CONST`). Gone since the previous record: `FRAGCOLOR_LANE`,
`LOCALS`, `FAU_PAGE`, `TEX_LANES`, `NEG_ZERO`, the vertex `ARITH_CONST`,
`CONST_FOLD`, `ARITH` and `LOOP_VERTEX` families (both build/texture/
gouraud/shadow vertex shaders, `buffer`, `refract`, `terrain`'s vertex
shaders and the loop scenes' vertex shader `3cb0` compile). Section 1 of the
forecast note lists every program's both stages.

### What follows a refusal

* **Setup failure** (17 scenes: 2-4, 6-13, 20, 21, 23, 24, 29, 32): glmark2
  prints `Error: Failed to add vertex|fragment shader from file None:` and
  the log (scenes 2-4: the link failure of the program whose fragment shader
  was refused), the scene's setup fails, and the process exits with status 3
  (`run`, `scene` findings). Scene 20 (`ideas`) stops at its vertex shader
  `51b2`; scene 23 (`shadow`) at its fragment shader `cf4b` before any
  depth-only framebuffer is made.
* **GL errors with entry point** (15, and 22 before its crash): scenes that
  carry on with the program that did not link. Each is the error ES 2.0 asks
  for, not a defect of the GL core:
  * `GL error 0x501 in glGetAttribLocation`, `0x501 in glGetUniformLocation`
    (and, in 15, `glEnableVertexAttribArray`, `glVertexAttribPointer`,
    `glDisableVertexAttribArray`): glmark2 asks a program object that is
    not one, or passes the -1 location back.
  * `GL error 0x502 in current_drawable_program`: a draw with no usable
    program (the GL core's draw-time check names itself, not glDraw*).
  * 15 (`desktop` blur) raised 352 of them over its 3 frames with 56 draws:
    its 32 blur fragment shaders (`68af`, `7f17`, 16 each) are refused and
    the window draws go on with the rest; glmark2 read one `0x501` back
    (`gm2: glGetError`). The `buffer` scenes (17-19) raise none any more.
* **terrain (22) crashes** (signal 11, exit 139, no scene-end line):
  in glmark2's `Mesh::render_vbo` called from `TerrainRenderer::render`
  (the `gm2: mali-crash` frames, resolved against the binary's symbols),
  after 7 of its fragment shaders were refused (`1637`, `c549`, `4d26`,
  `e043`, `bdfd`, `bbba`, `8c86`; its vertex shaders compile): glmark2 uses
  the -1 attribute locations of the unlinked program as an index. A
  host-side crash of glmark2, not of YakoGL; on the board the same cause
  ends as SIGABRT (rc 134, libstdc++'s assertion; `tools/cts/RESULTS.md`).

### New causes not owned by the compiler work

None. The GL errors are the client's consequence of a refused shader and
the errors ES 2.0 specifies; the crash is glmark2's. No refused draw, no
incomplete texture, no incomplete framebuffer and no OSAL refusal came out
of the back end. The setup and draw causes the forecast note names
(`D-DEPTHONLY`, `D-NPOTMIP`, `D-DEPTHTEX`): `D-DEPTHONLY` is passed by
scene 16; `refract` (24, `D-NPOTMIP`) and the shadow scene (23,
`D-DEPTHTEX`) still stop at a fragment shader before their render targets
are made, so the host run does not reach them; the board run does.

### Limits of this record

* No command stream is executed: what the GPU would do with the streams of
  the 14 clean scenes and scene 15 (fault, hang, wrong picture) is the board
  run's to find. The picture lines are black on every GPU here (nothing is
  rendered).
* One process per scene from an empty heap (`README.md`, How it works): a
  scene does not inherit the previous scene's GPU memory, as one board run
  of the list does.
* The OSAL mock models no memory limit and no tiler heap growth; the heap
  is one 2 MiB chunk the back end is told about and nothing ever reads.

The table below is generated by `report.py` from the run's logs; a rerun
of the command above rewrites it.

<!-- generated by tools/hostharness/glmark2/report.py: begin -->
Run: glmark2 `22c527cb0556f3a1ac4445aaa52cc532760928d5`, the default benchmark list (33 entries), `--size 800x600`, 3 frames per entry (`-b :nframes=3`), one process per entry; GPUs: mali.

### EGL config chosen

| GPU | surface | config |
|---|---|---|
| mali | window | `id=0x1 buf=0x20 r=0x8 g=0x8 b=0x8 a=0x8 depth=0x18 stencil=0x8 samples=0x0 surface=0x405 renderable=0x44 visual=0x0` (33 of 33 runs) |

### Scenes

| # | entry | mali: status, frames, draws, refused, GL errors, incomplete-texture draws |
|---|---|---|
| 0 | `build:use-vbo=false` | ok, 3, 3, 0, 0, 0 |
| 1 | `build:use-vbo=true` | ok, 3, 3, 0, 0, 0 |
| 2 | `texture:texture-filter=nearest` | failed, 0, 0, 0, 0, 0 |
| 3 | `texture:texture-filter=linear` | failed, 0, 0, 0, 0, 0 |
| 4 | `texture:texture-filter=mipmap` | failed, 0, 0, 0, 0, 0 |
| 5 | `shading:shading=gouraud` | ok, 3, 3, 0, 0, 0 |
| 6 | `shading:shading=blinn-phong-inf` | failed, 0, 0, 0, 0, 0 |
| 7 | `shading:shading=phong` | failed, 0, 0, 0, 0, 0 |
| 8 | `shading:shading=cel` | failed, 0, 0, 0, 0, 0 |
| 9 | `bump:bump-render=high-poly` | failed, 0, 0, 0, 0, 0 |
| 10 | `bump:bump-render=normals` | failed, 0, 0, 0, 0, 0 |
| 11 | `bump:bump-render=height` | failed, 0, 0, 0, 0, 0 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | failed, 0, 0, 0, 0, 0 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | failed, 0, 0, 0, 0, 0 |
| 14 | `pulsar:quads=5:texture=false:light=false` | ok, 3, 15, 0, 0, 0 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | ok, 3, 56, 0, 352, 0 |
| 16 | `desktop:windows=4:effect=shadow` | ok, 3, 107, 0, 0, 0 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | ok, 3, 3, 0, 0, 0 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | ok, 3, 3, 0, 0, 0 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | ok, 3, 3, 0, 0, 0 |
| 20 | `ideas:speed=duration` | failed, 0, 0, 0, 0, 0 |
| 21 | `jellyfish` | failed, 0, 0, 0, 0, 0 |
| 22 | `terrain` | **no end** (rc 139) |
| 23 | `shadow` | failed, 0, 0, 0, 0, 0 |
| 24 | `refract` | failed, 0, 0, 0, 0, 0 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | ok, 3, 3, 0, 0, 0 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | ok, 3, 3, 0, 0, 0 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | ok, 3, 3, 0, 0, 0 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | ok, 3, 3, 0, 0, 0 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | failed, 0, 0, 0, 0, 0 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | ok, 3, 3, 0, 0, 0 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | ok, 3, 3, 0, 0, 0 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | failed, 0, 0, 0, 0, 0 |

### Findings

Every GL error (the library's own line, with the entry point), compile or link failure, refused draw, incomplete-texture line, error the client read back, glmark2 error line, failed run, and every other line the library logged that is not start-up or exit bookkeeping (kinds `depth resolve`, `no such uniform`, `other`), per entry and GPU. A count is the number of times the line came in the entry's setup and 3 frames, one per GPU in the GPU column's order. Ids are replaced by `<n>`.

| # | entry | GPU | kind | finding | count |
|---|---|---|---|---|---|
| 0 | `build:use-vbo=false` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 0 | `build:use-vbo=false` | mali | other | `mali arena 'data': a chunk of 262144 bytes for a block of 258192` | 2 |
| 0 | `build:use-vbo=false` | mali | other | `glmark2-picture: [build] use-vbo=false: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 1 | `build:use-vbo=true` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 1 | `build:use-vbo=true` | mali | other | `glmark2-picture: [build] use-vbo=true: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 2 | `texture:texture-filter=nearest` | mali | other | `glLinkProgram(2): link error: the vertex shader for the program's fragment shader: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet` | 1 |
| 2 | `texture:texture-filter=nearest` | mali | link failure | `gm2: link-failed <id>: link error: the vertex shader for the program's fragment shader: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet)` | 1 |
| 2 | `texture:texture-filter=nearest` | mali | glmark2 error | `Error: Failed to link program created from files None and None:  link error: the vertex shader for the program's fragment shader: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet)` | 1 |
| 2 | `texture:texture-filter=nearest` | mali | run | `the process ended with exit status 3` | 1 |
| 2 | `texture:texture-filter=nearest` | mali | scene | `glmark2: scene failed` | 1 |
| 3 | `texture:texture-filter=linear` | mali | other | `glLinkProgram(2): link error: the vertex shader for the program's fragment shader: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet` | 1 |
| 3 | `texture:texture-filter=linear` | mali | link failure | `gm2: link-failed <id>: link error: the vertex shader for the program's fragment shader: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet)` | 1 |
| 3 | `texture:texture-filter=linear` | mali | glmark2 error | `Error: Failed to link program created from files None and None:  link error: the vertex shader for the program's fragment shader: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet)` | 1 |
| 3 | `texture:texture-filter=linear` | mali | run | `the process ended with exit status 3` | 1 |
| 3 | `texture:texture-filter=linear` | mali | scene | `glmark2: scene failed` | 1 |
| 4 | `texture:texture-filter=mipmap` | mali | other | `glLinkProgram(2): link error: the vertex shader for the program's fragment shader: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet` | 1 |
| 4 | `texture:texture-filter=mipmap` | mali | link failure | `gm2: link-failed <id>: link error: the vertex shader for the program's fragment shader: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet)` | 1 |
| 4 | `texture:texture-filter=mipmap` | mali | glmark2 error | `Error: Failed to link program created from files None and None:  link error: the vertex shader for the program's fragment shader: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet)` | 1 |
| 4 | `texture:texture-filter=mipmap` | mali | run | `the process ended with exit status 3` | 1 |
| 4 | `texture:texture-filter=mipmap` | mali | scene | `glmark2: scene failed` | 1 |
| 5 | `shading:shading=gouraud` | mali | no such uniform | `yakogl: program <n> has no uniform 'ModelViewMatrix'` | 1 |
| 5 | `shading:shading=gouraud` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 5 | `shading:shading=gouraud` | mali | other | `glmark2-picture: [shading] shading=gouraud: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 6 | `shading:shading=blinn-phong-inf` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1747 bytes]` | 1 |
| 6 | `shading:shading=blinn-phong-inf` | mali | compile failure | `gm2: compile-failed <id> 994f0197ada4c9fa: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 1 |
| 6 | `shading:shading=blinn-phong-inf` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 6 | `shading:shading=blinn-phong-inf` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 1 |
| 6 | `shading:shading=blinn-phong-inf` | mali | run | `the process ended with exit status 3` | 1 |
| 6 | `shading:shading=blinn-phong-inf` | mali | scene | `glmark2: scene failed` | 1 |
| 7 | `shading:shading=phong` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fus` | 1 |
| 7 | `shading:shading=phong` | mali | compile failure | `gm2: compile-failed <id> 8a641531f03295fb: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is` | 1 |
| 7 | `shading:shading=phong` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 7 | `shading:shading=phong` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is` | 1 |
| 7 | `shading:shading=phong` | mali | run | `the process ended with exit status 3` | 1 |
| 7 | `shading:shading=phong` | mali | scene | `glmark2: scene failed` | 1 |
| 8 | `shading:shading=cel` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 2737 bytes]` | 1 |
| 8 | `shading:shading=cel` | mali | compile failure | `gm2: compile-failed <id> fea0d079a13967da: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 1 |
| 8 | `shading:shading=cel` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 8 | `shading:shading=cel` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 1 |
| 8 | `shading:shading=cel` | mali | run | `the process ended with exit status 3` | 1 |
| 8 | `shading:shading=cel` | mali | scene | `glmark2: scene failed` | 1 |
| 9 | `bump:bump-render=high-poly` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1730 bytes]` | 1 |
| 9 | `bump:bump-render=high-poly` | mali | compile failure | `gm2: compile-failed <id> d4424181c7796637: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 1 |
| 9 | `bump:bump-render=high-poly` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 9 | `bump:bump-render=high-poly` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 1 |
| 9 | `bump:bump-render=high-poly` | mali | run | `the process ended with exit status 3` | 1 |
| 9 | `bump:bump-render=high-poly` | mali | scene | `glmark2: scene failed` | 1 |
| 10 | `bump:bump-render=normals` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 2268 bytes]` | 1 |
| 10 | `bump:bump-render=normals` | mali | compile failure | `gm2: compile-failed <id> 487d34975c3c8a1e: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 1 |
| 10 | `bump:bump-render=normals` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 10 | `bump:bump-render=normals` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 1 |
| 10 | `bump:bump-render=normals` | mali | run | `the process ended with exit status 3` | 1 |
| 10 | `bump:bump-render=normals` | mali | scene | `glmark2: scene failed` | 1 |
| 11 | `bump:bump-render=height` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 2676 bytes]` | 1 |
| 11 | `bump:bump-render=height` | mali | compile failure | `gm2: compile-failed <id> 77c536a2dad6a903: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 1 |
| 11 | `bump:bump-render=height` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 11 | `bump:bump-render=height` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 1 |
| 11 | `bump:bump-render=height` | mali | run | `the process ended with exit status 3` | 1 |
| 11 | `bump:bump-render=height` | mali | scene | `glmark2: scene failed` | 1 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | mali | compile failure | `gm2: compile-failed <id> 3c3d57ba2c0ece72: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet)` | 1 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet)` | 1 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | mali | run | `the process ended with exit status 3` | 1 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | mali | scene | `glmark2: scene failed` | 1 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | mali | compile failure | `gm2: compile-failed <id> ba8e6925e955f7f2: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet)` | 1 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet)` | 1 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | mali | run | `the process ended with exit status 3` | 1 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | mali | scene | `glmark2: scene failed` | 1 |
| 14 | `pulsar:quads=5:texture=false:light=false` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 14 | `pulsar:quads=5:texture=false:light=false` | mali | other | `glmark2-picture: [pulsar] light=false:quads=5:texture=false: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(9): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | compile failure | `gm2: compile-failed <id> 7f177aebfac60da2: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet)` | 16 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 32 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet)` | 32 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(11): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | compile failure | `gm2: compile-failed <id> 68afdb5a2dcf8400: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet)` | 16 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | GL error | `GL error 0x501 in glGetAttribLocation` | 64 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | GL error | `GL error 0x501 in glGetUniformLocation` | 64 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | GL error | `GL error 0x501 in glEnableVertexAttribArray` | 64 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | GL error | `GL error 0x501 in glVertexAttribPointer` | 64 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | GL error | `GL error 0x502 in current_drawable_program` | 32 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | GL error | `GL error 0x501 in glDisableVertexAttribArray` | 64 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(13): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(15): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(17): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(19): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(21): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(23): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(25): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(27): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(29): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(31): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(33): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(35): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(37): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(39): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(41): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(43): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(45): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(47): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(49): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(51): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(53): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(55): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(57): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(59): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(61): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(63): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(65): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(67): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(69): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(71): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | glGetError | `gm2: glGetError 0x501` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glmark2-picture: [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 16 | `desktop:windows=4:effect=shadow` | mali | other | `glmark2-picture: [desktop] effect=shadow:windows=4: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | mali | other | `glmark2-picture: [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | mali | other | `glmark2-picture: [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | mali | other | `glmark2-picture: [buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 20 | `ideas:speed=duration` | mali | other | `glCompileShader(19): online shaderc: the code generator refused this vertex shader: Mali(Valhall: control flow (if, a loop, a return before the end) is not lowered yet) [vertex stage, 927 bytes]` | 1 |
| 20 | `ideas:speed=duration` | mali | compile failure | `gm2: compile-failed <id> 51b217c5b7686924: online shaderc: the code generator refused this vertex shader: Mali(Valhall: control flow (if, a loop, a return before the end) is not lowered yet)` | 1 |
| 20 | `ideas:speed=duration` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 20 | `ideas:speed=duration` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: control flow (if, a loop, a return before the end) is not lowered yet)` | 1 |
| 20 | `ideas:speed=duration` | mali | glmark2 error | `Error: No valid program for normal logo rendering` | 1 |
| 20 | `ideas:speed=duration` | mali | run | `the process ended with exit status 3` | 1 |
| 20 | `ideas:speed=duration` | mali | scene | `glmark2: scene failed` | 1 |
| 21 | `jellyfish` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet) [fragment stage, 431` | 1 |
| 21 | `jellyfish` | mali | compile failure | `gm2: compile-failed <id> 8ee578801bfa8bdc: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet)` | 1 |
| 21 | `jellyfish` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 21 | `jellyfish` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet)` | 1 |
| 21 | `jellyfish` | mali | run | `the process ended with exit status 3` | 1 |
| 21 | `jellyfish` | mali | scene | `glmark2: scene failed` | 1 |
| 22 | `terrain` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [fragment stage, 3252 bytes]` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> 1637be2eef2e8a5e: online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 22 | `terrain` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 7 |
| 22 | `terrain` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 3 |
| 22 | `terrain` | mali | GL error | `GL error 0x501 in glGetAttribLocation` | 30 |
| 22 | `terrain` | mali | GL error | `GL error 0x501 in glGetUniformLocation` | 30 |
| 22 | `terrain` | mali | other | `glCompileShader(7): online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [fragment stage, 693 bytes]` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> c549a79aadfd095b: online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 22 | `terrain` | mali | other | `glCompileShader(13): online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [fragment stage, 3998 bytes]` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> 4d266cda76461dc0: online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 22 | `terrain` | mali | other | `glCompileShader(15): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> e043e1e89bacfac4: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet)` | 1 |
| 22 | `terrain` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet)` | 4 |
| 22 | `terrain` | mali | other | `glCompileShader(17): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> bdfd8dd627aa0bf2: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet)` | 1 |
| 22 | `terrain` | mali | other | `glCompileShader(23): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> bbba1ac2cdc22b1a: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet)` | 1 |
| 22 | `terrain` | mali | other | `glCompileShader(25): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups,` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> 8c8662245addb598: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet)` | 1 |
| 22 | `terrain` | mali | other | `[terrain] <default>:GL error 0x502 in current_drawable_program` | 1 |
| 22 | `terrain` | mali | GL error | `GL error 0x502 in current_drawable_program` | 1 |
| 22 | `terrain` | mali | other | `gm2: mali-crash signal 11: glmark2-host-mali+0x97d71 libc.so.6+0x45cb0 glmark2-host-mali+0xcc3f8 glmark2-host-mali+0x157908 glmark2-host-mali+0x1563fa glmark2-host-mali+0x14e9cd glmark2-host-mali+0x93db6 glmark2-host-mali+0xc82b4 glmark2-host-mali+0x94d0a libc.so.6+0x2a601 libc.so.6+0x2a718 glmark2-host-mali+0x93515` | 1 |
| 22 | `terrain` | mali | run | `the process ended with exit status 139` | 1 |
| 22 | `terrain` | mali | run | `no scene-end line: the scene never finished` | 1 |
| 23 | `shadow` | mali | other | `glCompileShader(13): online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [fragment stage, 724 bytes]` | 1 |
| 23 | `shadow` | mali | compile failure | `gm2: compile-failed <id> cf4bf899d84af04d: online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 23 | `shadow` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 23 | `shadow` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 23 | `shadow` | mali | glmark2 error | `Error: Failed to set up the ground renderer` | 1 |
| 23 | `shadow` | mali | run | `the process ended with exit status 3` | 1 |
| 23 | `shadow` | mali | scene | `glmark2: scene failed` | 1 |
| 24 | `refract` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fus` | 1 |
| 24 | `refract` | mali | compile failure | `gm2: compile-failed <id> 3fae89c9ab969ea3: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is` | 1 |
| 24 | `refract` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 24 | `refract` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is` | 1 |
| 24 | `refract` | mali | run | `the process ended with exit status 3` | 1 |
| 24 | `refract` | mali | scene | `glmark2: scene failed` | 1 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | mali | other | `glmark2-picture: [conditionals] fragment-steps=0:vertex-steps=0: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | mali | other | `glmark2-picture: [conditionals] fragment-steps=5:vertex-steps=0: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | mali | other | `glmark2-picture: [conditionals] fragment-steps=0:vertex-steps=5: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | mali | other | `glmark2-picture: [function] fragment-complexity=low:fragment-steps=5: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y is not lowered yet) [fragment stage, 9` | 1 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | mali | compile failure | `gm2: compile-failed <id> 849c78149ac0211c: online shaderc: the code generator refused this fragment shader: Mali(Valhall: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y is not lowered yet)` | 1 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y is not lowered yet)` | 1 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | mali | run | `the process ended with exit status 3` | 1 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | mali | scene | `glmark2: scene failed` | 1 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | mali | other | `glmark2-picture: [loop] fragment-loop=false:fragment-steps=5:vertex-steps=5: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | mali | other | `mali: depth/stencil buffers: high-water 1 of 32` | 1 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | mali | other | `glmark2-picture: [loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge) is not lowered yet) [fragment stage, 879 b` | 1 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | mali | compile failure | `gm2: compile-failed <id> 3678074e44f2c92c: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge) is not lowered yet)` | 1 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge) is not lowered yet)` | 1 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | mali | run | `the process ended with exit status 3` | 1 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | mali | scene | `glmark2: scene failed` | 1 |

Entries with no finding on any GPU: none.

### Shaders

61 distinct shader texts (26 vertex, 35 fragment) passed to glShaderSource, in `<out>/shaders/` of the run (`index.txt` maps each to its entries; not committed, tools/shaders/glmark2/README.md).
<!-- generated by tools/hostharness/glmark2/report.py: end -->
