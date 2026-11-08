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

On main `c6b16f71` (`gl: clear a texture slot's render-into mark when the
slot is freed`), glmark2 at `22c527cb0556f3a1ac4445aaa52cc532760928d5` (a git
checkout, verified by `run.sh`):

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

## Result

**1 of 33 scenes is clean**: 16, `desktop:windows=4:effect=shadow` (3 frames,
107 draws, 25 submits, 30,864 stream bytes; its one line in the table is
the `glmark2-picture` line every GPU's run logs, black as nothing is
rendered). 27 scenes fail at
setup (glmark2 exits with status 3 after a shader it needs is refused),
4 finish with GL errors (15, 17, 18, 19), and terrain (22) crashes in
glmark2 itself. No draw was refused by the back end, no draw sampled an
incomplete texture, no framebuffer was incomplete, no GPU memory ran out,
and no submit was refused by the OSAL mock, in any scene.

Every failure has one root cause: **the Mali code generator
(`src/shaderc/online/mali`) refuses a shader**. That refusal work is plan
#309's (it owns `src/mali` and `src/shaderc/online/mali`); this file only
lists which glmark2 scene each refusal stops. The GL errors and the crash
are what glmark2 does next with the program that did not link.

### Compiler refusals, by cause (#309)

The text after `Mali(Valhall: ` in the compile log; the scenes (entry
numbers of the table below) whose shader it refused.

| cause (the compiler's words) | stage | scenes | count |
|---|---|---|---|
| a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, ...) | vertex | 0-11, 14, 17, 18, 19, 22, 23, 24 | 19 |
| arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet | vertex | 25-32 | 8 |
| the same | fragment | 22 | 1 |
| a constant operation on or to a value that is not a finite normal float is not lowered yet | fragment | 12, 15 | 2 |
| a shader with more than 32 local variables is not lowered yet | fragment | 13, 22 | 2 |
| a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet | vertex | 20 | 1 |
| a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet | fragment | 21 | 1 |

The first one alone stops 19 scenes: every scene whose vertex shader is
glmark2's common lighting/transform one (build, texture, shading, bump,
pulsar, buffer, terrain, shadow, refract). Fixing it and the vertex
arithmetic one (conditionals, function, loop) reaches 27 of the 32 failing
scenes' first refusal; a scene may then meet the next one.

### What follows a refusal

* **Setup failure** (27 scenes: 0-14, 20, 21, 23-32): glmark2 prints
  `Error: Failed to add vertex|fragment shader from file None:` and the log,
  the scene's setup fails, and the process exits with status 3 (`run`,
  `scene` findings). Scene 20 (`ideas`) also prints `No valid program for
  table rendering.`
* **GL errors with entry point** (15, 17, 18, 19, and 22 before its crash):
  scenes that carry on with the program that did not link. Each is the
  error ES 2.0 asks for, not a defect of the GL core:
  * `GL error 0x501 in glGetAttribLocation`, `0x501 in glGetUniformLocation`
    (and, in 15, `glEnableVertexAttribArray`, `glVertexAttribPointer`,
    `glDisableVertexAttribArray`): glmark2 asks a program object that is
    not one, or passes the -1 location back.
  * `GL error 0x502 in current_drawable_program`: a draw with no usable
    program (the GL core's draw-time check names itself, not glDraw*).
  * 15 (`desktop` blur) raised 352 of them over its 3 frames with 56 draws:
    its 32 blur fragment shaders are refused (`a constant operation on ...
    not a finite normal float`) and the window draws go on with the rest;
    17-19 (`buffer`) 15 each, and glmark2 read one `0x501` back
    (`gm2: glGetError`).
* **terrain (22) crashes** (signal 11, exit 139, no scene-end line):
  in glmark2's `Mesh::render_vbo` called from `TerrainRenderer::render`
  (the `gm2: mali-crash` frames, resolved against the binary's symbols),
  after 8 of its shaders were refused: glmark2 uses the -1 attribute
  locations of the unlinked program as an index. A host-side crash of
  glmark2, not of YakoGL; on the board the same scene would stop the same
  way (the board runner's per-scene process keeps it from taking the other
  scenes with it).

### New causes not owned by #309

None. The GL errors are the client's consequence of a refused shader and
the errors ES 2.0 specifies; the crash is glmark2's. No refused draw, no
incomplete texture and no OSAL refusal came out of the back end.

### Limits of this record

* No command stream is executed: what the GPU would do with the streams of
  scenes 15-19 (fault, hang, wrong picture) is the board run's to find. The
  picture lines are black on every GPU here (nothing is rendered).
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
| 0 | `build:use-vbo=false` | failed, 0, 0, 0, 0, 0 |
| 1 | `build:use-vbo=true` | failed, 0, 0, 0, 0, 0 |
| 2 | `texture:texture-filter=nearest` | failed, 0, 0, 0, 0, 0 |
| 3 | `texture:texture-filter=linear` | failed, 0, 0, 0, 0, 0 |
| 4 | `texture:texture-filter=mipmap` | failed, 0, 0, 0, 0, 0 |
| 5 | `shading:shading=gouraud` | failed, 0, 0, 0, 0, 0 |
| 6 | `shading:shading=blinn-phong-inf` | failed, 0, 0, 0, 0, 0 |
| 7 | `shading:shading=phong` | failed, 0, 0, 0, 0, 0 |
| 8 | `shading:shading=cel` | failed, 0, 0, 0, 0, 0 |
| 9 | `bump:bump-render=high-poly` | failed, 0, 0, 0, 0, 0 |
| 10 | `bump:bump-render=normals` | failed, 0, 0, 0, 0, 0 |
| 11 | `bump:bump-render=height` | failed, 0, 0, 0, 0, 0 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | failed, 0, 0, 0, 0, 0 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | failed, 0, 0, 0, 0, 0 |
| 14 | `pulsar:quads=5:texture=false:light=false` | failed, 0, 0, 0, 0, 0 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | ok, 3, 56, 0, 352, 0 |
| 16 | `desktop:windows=4:effect=shadow` | ok, 3, 107, 0, 0, 0 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | ok, 3, 3, 0, 15, 0 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | ok, 3, 3, 0, 15, 0 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | ok, 3, 3, 0, 15, 0 |
| 20 | `ideas:speed=duration` | failed, 0, 0, 0, 0, 0 |
| 21 | `jellyfish` | failed, 0, 0, 0, 0, 0 |
| 22 | `terrain` | **no end** (rc 139) |
| 23 | `shadow` | failed, 0, 0, 0, 0, 0 |
| 24 | `refract` | failed, 0, 0, 0, 0, 0 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | failed, 0, 0, 0, 0, 0 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | failed, 0, 0, 0, 0, 0 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | failed, 0, 0, 0, 0, 0 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | failed, 0, 0, 0, 0, 0 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | failed, 0, 0, 0, 0, 0 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | failed, 0, 0, 0, 0, 0 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | failed, 0, 0, 0, 0, 0 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | failed, 0, 0, 0, 0, 0 |

### Findings

Every GL error (the library's own line, with the entry point), compile or link failure, refused draw, incomplete-texture line, error the client read back, glmark2 error line, failed run, and every other line the library logged that is not start-up or exit bookkeeping (kinds `depth resolve`, `no such uniform`, `other`), per entry and GPU. A count is the number of times the line came in the entry's setup and 3 frames, one per GPU in the GPU column's order. Ids are replaced by `<n>`.

| # | entry | GPU | kind | finding | count |
|---|---|---|---|---|---|
| 0 | `build:use-vbo=false` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 0 | `build:use-vbo=false` | mali | compile failure | `gm2: compile-failed <id> ace204f6171a58b1: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 0 | `build:use-vbo=false` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 0 | `build:use-vbo=false` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 0 | `build:use-vbo=false` | mali | run | `the process ended with exit status 3` | 1 |
| 0 | `build:use-vbo=false` | mali | scene | `glmark2: scene failed` | 1 |
| 1 | `build:use-vbo=true` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 1 | `build:use-vbo=true` | mali | compile failure | `gm2: compile-failed <id> ace204f6171a58b1: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 1 | `build:use-vbo=true` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 1 | `build:use-vbo=true` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 1 | `build:use-vbo=true` | mali | run | `the process ended with exit status 3` | 1 |
| 1 | `build:use-vbo=true` | mali | scene | `glmark2: scene failed` | 1 |
| 2 | `texture:texture-filter=nearest` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 2 | `texture:texture-filter=nearest` | mali | compile failure | `gm2: compile-failed <id> ace204f6171a58b1: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 2 | `texture:texture-filter=nearest` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 2 | `texture:texture-filter=nearest` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 2 | `texture:texture-filter=nearest` | mali | run | `the process ended with exit status 3` | 1 |
| 2 | `texture:texture-filter=nearest` | mali | scene | `glmark2: scene failed` | 1 |
| 3 | `texture:texture-filter=linear` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 3 | `texture:texture-filter=linear` | mali | compile failure | `gm2: compile-failed <id> ace204f6171a58b1: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 3 | `texture:texture-filter=linear` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 3 | `texture:texture-filter=linear` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 3 | `texture:texture-filter=linear` | mali | run | `the process ended with exit status 3` | 1 |
| 3 | `texture:texture-filter=linear` | mali | scene | `glmark2: scene failed` | 1 |
| 4 | `texture:texture-filter=mipmap` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 4 | `texture:texture-filter=mipmap` | mali | compile failure | `gm2: compile-failed <id> ace204f6171a58b1: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 4 | `texture:texture-filter=mipmap` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 4 | `texture:texture-filter=mipmap` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 4 | `texture:texture-filter=mipmap` | mali | run | `the process ended with exit status 3` | 1 |
| 4 | `texture:texture-filter=mipmap` | mali | scene | `glmark2: scene failed` | 1 |
| 5 | `shading:shading=gouraud` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 5 | `shading:shading=gouraud` | mali | compile failure | `gm2: compile-failed <id> 83cd81fbb702ee85: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 5 | `shading:shading=gouraud` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 5 | `shading:shading=gouraud` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 5 | `shading:shading=gouraud` | mali | run | `the process ended with exit status 3` | 1 |
| 5 | `shading:shading=gouraud` | mali | scene | `glmark2: scene failed` | 1 |
| 6 | `shading:shading=blinn-phong-inf` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 6 | `shading:shading=blinn-phong-inf` | mali | compile failure | `gm2: compile-failed <id> 78e3ba59e507a5cc: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 6 | `shading:shading=blinn-phong-inf` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 6 | `shading:shading=blinn-phong-inf` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 6 | `shading:shading=blinn-phong-inf` | mali | run | `the process ended with exit status 3` | 1 |
| 6 | `shading:shading=blinn-phong-inf` | mali | scene | `glmark2: scene failed` | 1 |
| 7 | `shading:shading=phong` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 7 | `shading:shading=phong` | mali | compile failure | `gm2: compile-failed <id> e5f21674435c0404: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 7 | `shading:shading=phong` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 7 | `shading:shading=phong` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 7 | `shading:shading=phong` | mali | run | `the process ended with exit status 3` | 1 |
| 7 | `shading:shading=phong` | mali | scene | `glmark2: scene failed` | 1 |
| 8 | `shading:shading=cel` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 8 | `shading:shading=cel` | mali | compile failure | `gm2: compile-failed <id> e5f21674435c0404: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 8 | `shading:shading=cel` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 8 | `shading:shading=cel` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 8 | `shading:shading=cel` | mali | run | `the process ended with exit status 3` | 1 |
| 8 | `shading:shading=cel` | mali | scene | `glmark2: scene failed` | 1 |
| 9 | `bump:bump-render=high-poly` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 9 | `bump:bump-render=high-poly` | mali | compile failure | `gm2: compile-failed <id> 78e3ba59e507a5cc: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 9 | `bump:bump-render=high-poly` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 9 | `bump:bump-render=high-poly` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 9 | `bump:bump-render=high-poly` | mali | run | `the process ended with exit status 3` | 1 |
| 9 | `bump:bump-render=high-poly` | mali | scene | `glmark2: scene failed` | 1 |
| 10 | `bump:bump-render=normals` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 10 | `bump:bump-render=normals` | mali | compile failure | `gm2: compile-failed <id> 51d0a6207c34d50c: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 10 | `bump:bump-render=normals` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 10 | `bump:bump-render=normals` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 10 | `bump:bump-render=normals` | mali | run | `the process ended with exit status 3` | 1 |
| 10 | `bump:bump-render=normals` | mali | scene | `glmark2: scene failed` | 1 |
| 11 | `bump:bump-render=height` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 11 | `bump:bump-render=height` | mali | compile failure | `gm2: compile-failed <id> 270aae3cfcae71a2: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 11 | `bump:bump-render=height` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 11 | `bump:bump-render=height` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 11 | `bump:bump-render=height` | mali | run | `the process ended with exit status 3` | 1 |
| 11 | `bump:bump-render=height` | mali | scene | `glmark2: scene failed` | 1 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1889 bytes]` | 1 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | mali | compile failure | `gm2: compile-failed <id> 3c3d57ba2c0ece72: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 1 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 1 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | mali | run | `the process ended with exit status 3` | 1 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | mali | scene | `glmark2: scene failed` | 1 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a shader with more than 32 local variables is not lowered yet) [fragment stage, 2653 bytes]` | 1 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | mali | compile failure | `gm2: compile-failed <id> ba8e6925e955f7f2: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a shader with more than 32 local variables is not lowered yet)` | 1 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a shader with more than 32 local variables is not lowered yet)` | 1 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | mali | run | `the process ended with exit status 3` | 1 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | mali | scene | `glmark2: scene failed` | 1 |
| 14 | `pulsar:quads=5:texture=false:light=false` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 14 | `pulsar:quads=5:texture=false:light=false` | mali | compile failure | `gm2: compile-failed <id> 9d75bb1847b5cc25: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 14 | `pulsar:quads=5:texture=false:light=false` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 14 | `pulsar:quads=5:texture=false:light=false` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 14 | `pulsar:quads=5:texture=false:light=false` | mali | run | `the process ended with exit status 3` | 1 |
| 14 | `pulsar:quads=5:texture=false:light=false` | mali | scene | `glmark2: scene failed` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(9): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | compile failure | `gm2: compile-failed <id> 7f177aebfac60da2: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 16 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 32 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 32 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(11): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | compile failure | `gm2: compile-failed <id> 68afdb5a2dcf8400: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet)` | 16 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | GL error | `GL error 0x501 in glGetAttribLocation` | 64 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | GL error | `GL error 0x501 in glGetUniformLocation` | 64 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | GL error | `GL error 0x501 in glEnableVertexAttribArray` | 64 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | GL error | `GL error 0x501 in glVertexAttribPointer` | 64 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | GL error | `GL error 0x502 in current_drawable_program` | 32 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | GL error | `GL error 0x501 in glDisableVertexAttribArray` | 64 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(13): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(15): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(17): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(19): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(21): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(23): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(25): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(27): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(29): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(31): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(33): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(35): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(37): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(39): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(41): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(43): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(45): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(47): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(49): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(51): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(53): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(55): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(57): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(59): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(61): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(63): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(65): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(67): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(69): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glCompileShader(71): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet) [fragment stage, 1518 bytes]` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | glGetError | `gm2: glGetError 0x501` | 1 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | mali | other | `glmark2-picture: [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 16 | `desktop:windows=4:effect=shadow` | mali | other | `glmark2-picture: [desktop] effect=shadow:windows=4: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | mali | compile failure | `gm2: compile-failed <id> 723443fdbaf22e8b: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | mali | GL error | `GL error 0x501 in glGetAttribLocation` | 6 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | mali | GL error | `GL error 0x501 in glGetUniformLocation` | 6 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | mali | GL error | `GL error 0x502 in current_drawable_program` | 3 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | mali | glGetError | `gm2: glGetError 0x501` | 1 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | mali | other | `glmark2-picture: [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | mali | compile failure | `gm2: compile-failed <id> 723443fdbaf22e8b: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | mali | GL error | `GL error 0x501 in glGetAttribLocation` | 6 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | mali | GL error | `GL error 0x501 in glGetUniformLocation` | 6 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | mali | GL error | `GL error 0x502 in current_drawable_program` | 3 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | mali | glGetError | `gm2: glGetError 0x501` | 1 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | mali | other | `glmark2-picture: [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | mali | compile failure | `gm2: compile-failed <id> 723443fdbaf22e8b: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | mali | GL error | `GL error 0x501 in glGetAttribLocation` | 6 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | mali | GL error | `GL error 0x501 in glGetUniformLocation` | 6 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | mali | GL error | `GL error 0x502 in current_drawable_program` | 3 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | mali | glGetError | `gm2: glGetError 0x501` | 1 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | mali | other | `glmark2-picture: [buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map: 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 7282, crc16q 7282; mean 0 0 0; black yes` | 1 |
| 20 | `ideas:speed=duration` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet) [vertex stage, 877 bytes]` | 1 |
| 20 | `ideas:speed=duration` | mali | compile failure | `gm2: compile-failed <id> bf4bdbb017a1985a: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet)` | 1 |
| 20 | `ideas:speed=duration` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 20 | `ideas:speed=duration` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet)` | 1 |
| 20 | `ideas:speed=duration` | mali | glmark2 error | `Error: No valid program for table rendering.` | 1 |
| 20 | `ideas:speed=duration` | mali | run | `the process ended with exit status 3` | 1 |
| 20 | `ideas:speed=duration` | mali | scene | `glmark2: scene failed` | 1 |
| 21 | `jellyfish` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet) [fragment stage, 431 bytes]` | 1 |
| 21 | `jellyfish` | mali | compile failure | `gm2: compile-failed <id> 8ee578801bfa8bdc: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet)` | 1 |
| 21 | `jellyfish` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 1 |
| 21 | `jellyfish` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet)` | 1 |
| 21 | `jellyfish` | mali | run | `the process ended with exit status 3` | 1 |
| 21 | `jellyfish` | mali | scene | `glmark2: scene failed` | 1 |
| 22 | `terrain` | mali | other | `glCompileShader(5): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a shader with more than 32 local variables is not lowered yet) [fragment stage, 3252 bytes]` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> 1637be2eef2e8a5e: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a shader with more than 32 local variables is not lowered yet)` | 1 |
| 22 | `terrain` | mali | glmark2 error | `Error: Failed to add fragment shader from file None:` | 7 |
| 22 | `terrain` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: a shader with more than 32 local variables is not lowered yet)` | 1 |
| 22 | `terrain` | mali | GL error | `GL error 0x501 in glGetAttribLocation` | 34 |
| 22 | `terrain` | mali | GL error | `GL error 0x501 in glGetUniformLocation` | 34 |
| 22 | `terrain` | mali | other | `glCompileShader(7): online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [fragment stage, 693 bytes]` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> c549a79aadfd095b: online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 22 | `terrain` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 6 |
| 22 | `terrain` | mali | other | `glCompileShader(9): online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [fragment stage, 507 bytes]` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> d697bb5000d6955f: online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 22 | `terrain` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> 18bca803a331552c: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 22 | `terrain` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 22 | `terrain` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 22 | `terrain` | mali | other | `glCompileShader(13): online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [fragment stage, 1055 bytes]` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> e043e1e89bacfac4: online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 22 | `terrain` | mali | other | `glCompileShader(15): online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [fragment stage, 1055 bytes]` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> bdfd8dd627aa0bf2: online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 22 | `terrain` | mali | other | `glCompileShader(21): online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [fragment stage, 1405 bytes]` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> bbba1ac2cdc22b1a: online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 22 | `terrain` | mali | other | `glCompileShader(23): online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [fragment stage, 1405 bytes]` | 1 |
| 22 | `terrain` | mali | compile failure | `gm2: compile-failed <id> 8c8662245addb598: online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 22 | `terrain` | mali | GL error | `GL error 0x502 in current_drawable_program` | 2 |
| 22 | `terrain` | mali | other | `[terrain] <default>:GL error 0x502 in current_drawable_program` | 1 |
| 22 | `terrain` | mali | other | `gm2: mali-crash signal 11: glmark2-host-mali+0x92d31 libc.so.6+0x45cb0 glmark2-host-mali+0xc73b8 glmark2-host-mali+0x1528c8 glmark2-host-mali+0x1513ba glmark2-host-mali+0x14998d glmark2-host-mali+0x8ed76 glmark2-host-mali+0xc3274 glmark2-host-mali+0x8fcca libc.so.6+0x2a601 libc.so.6+0x2a718 glmark2-host-mali+0x8e4f5` | 1 |
| 22 | `terrain` | mali | run | `the process ended with exit status 139` | 1 |
| 22 | `terrain` | mali | run | `no scene-end line: the scene never finished` | 1 |
| 23 | `shadow` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 23 | `shadow` | mali | compile failure | `gm2: compile-failed <id> 6964fcf6aaac9739: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 23 | `shadow` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 23 | `shadow` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 23 | `shadow` | mali | run | `the process ended with exit status 3` | 1 |
| 23 | `shadow` | mali | scene | `glmark2: scene failed` | 1 |
| 24 | `refract` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused` | 1 |
| 24 | `refract` | mali | compile failure | `gm2: compile-failed <id> 7215f4f5052a383c: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 24 | `refract` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 24 | `refract` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no` | 1 |
| 24 | `refract` | mali | run | `the process ended with exit status 3` | 1 |
| 24 | `refract` | mali | scene | `glmark2: scene failed` | 1 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [vertex stage, 876 bytes]` | 1 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | mali | compile failure | `gm2: compile-failed <id> 310736a638617746: online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | mali | run | `the process ended with exit status 3` | 1 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | mali | scene | `glmark2: scene failed` | 1 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [vertex stage, 876 bytes]` | 1 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | mali | compile failure | `gm2: compile-failed <id> 310736a638617746: online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | mali | run | `the process ended with exit status 3` | 1 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | mali | scene | `glmark2: scene failed` | 1 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [vertex stage, 1291 bytes]` | 1 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | mali | compile failure | `gm2: compile-failed <id> a21c916ffe4265bc: online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | mali | run | `the process ended with exit status 3` | 1 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | mali | scene | `glmark2: scene failed` | 1 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [vertex stage, 970 bytes]` | 1 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | mali | compile failure | `gm2: compile-failed <id> 016263068a7fa619: online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | mali | run | `the process ended with exit status 3` | 1 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | mali | scene | `glmark2: scene failed` | 1 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [vertex stage, 970 bytes]` | 1 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | mali | compile failure | `gm2: compile-failed <id> 016263068a7fa619: online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | mali | run | `the process ended with exit status 3` | 1 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | mali | scene | `glmark2: scene failed` | 1 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [vertex stage, 971 bytes]` | 1 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | mali | compile failure | `gm2: compile-failed <id> 3cb095bbd6bfdb49: online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | mali | run | `the process ended with exit status 3` | 1 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | mali | scene | `glmark2: scene failed` | 1 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [vertex stage, 971 bytes]` | 1 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | mali | compile failure | `gm2: compile-failed <id> 3cb095bbd6bfdb49: online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | mali | run | `the process ended with exit status 3` | 1 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | mali | scene | `glmark2: scene failed` | 1 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | mali | other | `glCompileShader(3): online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet) [vertex stage, 971 bytes]` | 1 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | mali | compile failure | `gm2: compile-failed <id> 3cb095bbd6bfdb49: online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | mali | glmark2 error | `Error: Failed to add vertex shader from file None:` | 1 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | mali | glmark2 error | `Error:   online shaderc: the code generator refused this vertex shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` | 1 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | mali | run | `the process ended with exit status 3` | 1 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | mali | scene | `glmark2: scene failed` | 1 |

Entries with no finding on any GPU: none.

### Shaders

34 distinct shader texts (20 vertex, 14 fragment) passed to glShaderSource, in `<out>/shaders/` of the run (`index.txt` maps each to its entries; not committed, tools/shaders/glmark2/README.md).
<!-- generated by tools/hostharness/glmark2/report.py: end -->
