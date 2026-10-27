<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2-es2 on the PowerVR back end: CPU ms a frame, all 33 scenes

Host profile of the 33 default scenes of glmark2-es2 on the PowerVR build (the
back end the AM62P runs), with the Linux OSAL: GPU memory is malloc'd and
every submit completes at once, so these are host CPU milliseconds of the GL
core and the back end's command generation, not the board's and no GPU wait
(`README.md`, Profile). The ranking carries over to the board; the
milliseconds do not.

## Run

Measured on main `b9130d64d878` (`docs: v3d-sampled-copies, the host table at
the main tip`), `glmark2` at `22c527cb0556f3a1ac4445aaa52cc532760928d5`:

```sh
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-prof powervr
```

The run exited 0 with all 33 entries; counted frames are frames 2 to 20 (19
frames per entry), one process per entry, one at a time. `CPU ms a frame` is
the thread CPU time of those frames (`build/glmark2-prof/profile-powervr.md`,
"cpu ms a frame"); the entry points are ranked by their wall ms a frame.
The 0.000 rows are under half a microsecond a frame.

**Harness limit: no symbols.** `GM2_PERF=1` stops at once with
`glmark2/run.sh: GM2_PERF=1 needs perf`: `perf` is not installed on this
host, and no other sampler (valgrind, gdb) is either. An own `SIGPROF`
sampler preloaded into the harness binary was refused by the session's
command permission, so the `top 3 symbols` column is empty and the
`file:line` of a cost below is where the entry point's work is done in the
source, found by reading the call path, not a sample. Where that is not
enough to name the cost, the text says so. A rerun with `GM2_PERF=1` on a
host with `perf` fills the column (`README.md`, Profile, last paragraph).

## Scenes, in glmark2's order

| # | scene | CPU ms a frame | in GL+EGL | top GL entry points (wall ms a frame) | top 3 symbols | note |
|---|---|---|---|---|---|---|
| 1 | `build:use-vbo=false` | 0.417 | 0.409 ms (96%) | `glDrawArrays` 0.408, `eglSwapBuffers` 0.001, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 2 | `build:use-vbo=true` | 0.015 | 0.006 ms (37%) | `glDrawArrays` 0.005, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 3 | `texture:texture-filter=nearest` | 0.018 | 0.006 ms (35%) | `glDrawArrays` 0.005, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 4 | `texture:texture-filter=linear` | 0.018 | 0.006 ms (34%) | `glDrawArrays` 0.005, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 5 | `texture:texture-filter=mipmap` | 0.018 | 0.006 ms (34%) | `glDrawArrays` 0.005, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 6 | `shading:shading=gouraud` | 0.014 | 0.005 ms (37%) | `glDrawArrays` 0.004, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 7 | `shading:shading=blinn-phong-inf` | 0.013 | 0.005 ms (36%) | `glDrawArrays` 0.004, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 8 | `shading:shading=phong` | 0.016 | 0.006 ms (36%) | `glDrawArrays` 0.005, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 9 | `shading:shading=cel` | 0.017 | 0.006 ms (34%) | `glDrawArrays` 0.005, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 10 | `bump:bump-render=high-poly` | 0.020 | 0.009 ms (46%) | `glDrawArrays` 0.008, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 11 | `bump:bump-render=normals` | 0.017 | 0.007 ms (39%) | `glDrawArrays` 0.006, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 12 | `bump:bump-render=height` | 0.020 | 0.007 ms (34%) | `glDrawArrays` 0.005, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 13 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | 0.013 | 0.005 ms (39%) | `glDrawArrays` 0.004, `eglSwapBuffers` 0.000, `glClear` 0.000 | perf not installed |  |
| 14 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | 0.013 | 0.005 ms (39%) | `glDrawArrays` 0.004, `eglSwapBuffers` 0.000, `glClear` 0.000 | perf not installed |  |
| 15 | `pulsar:quads=5:texture=false:light=false` | 0.042 | 0.021 ms (45%) | `glDrawArrays` 0.015, `glVertexAttribPointer` 0.004, `eglSwapBuffers` 0.000 | perf not installed |  |
| 16 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | 0.128 | 0.059 ms (46%) | `glDrawArrays` 0.053, `glClear` 0.002, `glUseProgram` 0.001 | perf not installed |  |
| 17 | `desktop:windows=4:effect=shadow` | 0.195 | 0.077 ms (39%) | `glDrawArrays` 0.066, `glClear` 0.002, `glBindFramebuffer` 0.001 | perf not installed |  |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | 0.310 | 0.060 ms (19%) | `glMapBufferOES` 0.049, `glDrawArrays` 0.008, `eglSwapBuffers` 0.001 | perf not installed |  |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | 0.332 | 0.088 ms (26%) | `glBufferSubData` 0.080, `glDrawArrays` 0.007, `eglSwapBuffers` 0.001 | perf not installed |  |
| 20 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | 0.284 | 0.066 ms (23%) | `glMapBufferOES` 0.057, `glDrawArrays` 0.007, `eglSwapBuffers` 0.001 | perf not installed |  |
| 21 | `ideas:speed=duration` | 0.464 | 0.355 ms (76%) | `glDrawElements` 0.310, `glDrawArrays` 0.039, `glUniformMatrix4fv` 0.003 | perf not installed |  |
| 22 | `jellyfish` | 0.035 | 0.011 ms (32%) | `glDrawArrays` 0.005, `glDrawElements` 0.004, `eglSwapBuffers` 0.001 | perf not installed |  |
| 23 | `terrain` | 0.202 | 0.143 ms (71%) | `glGenerateMipmap` 0.086, `glDrawArrays` 0.051, `glClear` 0.002 | perf not installed | excluded: #916 (glGenerateMipmap is the largest entry); no proposal |
| 24 | `shadow` | 2.045 | 2.021 ms (99%) | `glDrawArrays` 2.013, `glClear` 0.003, `eglSwapBuffers` 0.001 | perf not installed | excluded: kick times (the AM62P shadow scene); no proposal |
| 25 | `refract` | 1.934 | 1.913 ms (99%) | `glDrawArrays` 1.882, `glGenerateMipmap` 0.022, `glClear` 0.003 | perf not installed |  |
| 26 | `conditionals:vertex-steps=0:fragment-steps=0` | 0.011 | 0.004 ms (39%) | `glDrawArrays` 0.004, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 27 | `conditionals:vertex-steps=0:fragment-steps=5` | 0.011 | 0.005 ms (39%) | `glDrawArrays` 0.004, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 28 | `conditionals:vertex-steps=5:fragment-steps=0` | 0.011 | 0.004 ms (39%) | `glDrawArrays` 0.004, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 29 | `function:fragment-steps=5:fragment-complexity=low` | 0.012 | 0.005 ms (39%) | `glDrawArrays` 0.004, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 30 | `function:fragment-steps=5:fragment-complexity=medium` | 0.012 | 0.005 ms (40%) | `glDrawArrays` 0.004, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | 0.011 | 0.004 ms (40%) | `glDrawArrays` 0.004, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | 0.012 | 0.005 ms (40%) | `glDrawArrays` 0.004, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |
| 33 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | 0.011 | 0.004 ms (40%) | `glDrawArrays` 0.004, `eglSwapBuffers` 0.000, `glUniformMatrix4fv` 0.000 | perf not installed |  |

## Ranked by CPU ms a frame

| rank | scene | CPU ms a frame |
|---|---|---|
| 1 | `shadow` | 2.045 |
| 2 | `refract` | 1.934 |
| 3 | `ideas:speed=duration` | 0.464 |
| 4 | `build:use-vbo=false` | 0.417 |
| 5 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | 0.332 |
| 6 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | 0.310 |
| 7 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | 0.284 |
| 8 | `terrain` | 0.202 |
| 9 | `desktop:windows=4:effect=shadow` | 0.195 |
| 10 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | 0.128 |
| 11 | `pulsar:quads=5:texture=false:light=false` | 0.042 |
| 12 | `jellyfish` | 0.035 |
| 13 | `bump:bump-render=high-poly` | 0.020 |
| 14 | `bump:bump-render=height` | 0.020 |
| 15 | `texture:texture-filter=nearest` | 0.018 |
| 16 | `texture:texture-filter=linear` | 0.018 |
| 17 | `texture:texture-filter=mipmap` | 0.018 |
| 18 | `shading:shading=cel` | 0.017 |
| 19 | `bump:bump-render=normals` | 0.017 |
| 20 | `shading:shading=phong` | 0.016 |
| 21 | `build:use-vbo=true` | 0.015 |
| 22 | `shading:shading=gouraud` | 0.014 |
| 23 | `shading:shading=blinn-phong-inf` | 0.013 |
| 24 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | 0.013 |
| 25 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | 0.013 |
| 26 | `function:fragment-steps=5:fragment-complexity=low` | 0.012 |
| 27 | `function:fragment-steps=5:fragment-complexity=medium` | 0.012 |
| 28 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | 0.012 |
| 29 | `conditionals:vertex-steps=0:fragment-steps=0` | 0.011 |
| 30 | `conditionals:vertex-steps=0:fragment-steps=5` | 0.011 |
| 31 | `conditionals:vertex-steps=5:fragment-steps=0` | 0.011 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | 0.011 |
| 33 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | 0.011 |

Twenty-three scenes are at or under 0.05 ms a frame (the draw-call
overhead of one draw: `glDrawArrays` 4 to 8 us, the GL core's checks and
the back end's record of a draw). They are not worth a fix item.

**Run-to-run spread.** The table is one run of one process a scene. The
spread of the harness is in `FINDINGS-variance.md` (AMD Ryzen 9 5900HX host,
not idle; the host of this PowerVR run is not recorded, so that spread is a
guide, not a measured figure for these numbers): the CV of the wall figure is
13% to 27% at `GM2_FRAMES=10` (5 repeats a cell), and quiet runs alone vary 4%
to 37% (median about 13%). Read this ranking with that: neighbouring ranks
within 15% of each other can swap on a rerun: ranks 1 and 2 (`shadow` 2.045,
`refract` 1.934, 5%), ranks 3 and 4 (0.464, 0.417, 10%), ranks 5 to 7
(0.332, 0.310, 0.284), ranks 8 and 9 (0.202, 0.195) and every rank in the
0.011 to 0.020 ms tail. The order holds only across large gaps: ranks 1 and
2 (about 2 ms) against rank 3 (0.46 ms) is 4x; rank 4 against rank 5 is 26%;
rank 7 against rank 8 is 41%; ranks 9 to 10 and 10 to 11 are 52% and 3x.

## The top 5 costs

| # | scene | CPU ms a frame | where | plausible fix item |
|---|---|---|---|---|
| 1 | `shadow` | 2.045 | `glDrawArrays` 2.013 ms for 3 calls, 0.67 ms a draw: `draw_arrays` `src/gl/draw.rs:4048`, `build_common` `src/gl/draw.rs:3203`, then the back end's `draw_dispatch` `src/pvr/backend.rs:5365` and `draw_part` `src/pvr/backend.rs:5675`. Where inside is not narrowed (no sampler) | excluded: kick times. No proposal |
| 2 | `refract` | 1.934 | `glDrawArrays` 1.882 ms for 2 calls, 0.94 ms a draw, the same path as 1; `glGenerateMipmap` 0.022 ms (`src/gl/texture.rs:3756`, #916) | The draw: unknown until the symbols are sampled, so not proposed. The mipmap: excluded: #916 |
| 3 | `ideas:speed=duration` | 0.464 | `glDrawElements` 0.310 ms for 162 calls, 1.9 us a call: `draw_elements` `src/gl/draw.rs:4219` | No: a fixed cost per draw, 0.3% of one draw of `shadow`; only the draw count could change it, and that is the application's |
| 4 | `build:use-vbo=false` | 0.417 | `glDrawArrays` 0.408 ms: the vertex arrays are client memory, so every vertex of every array goes through `convert_attrib` and `attrib_row`, `src/gl/draw.rs:1127` (the per-vertex loop), 1092 (the function), 1057 (`attrib_row`); a VBO draw of the same mesh (`build:use-vbo=true`) takes 0.005 ms | Plausible: a client array of a native float format that is tightly packed could be copied as a block instead of row by row. 80x the VBO scene; a host test can compare the staged bytes |
| 5 | `buffer:...update-method=subdata:interleave=false` (and the two `map` ones, ranks 6 and 7) | 0.332 (0.310, 0.284) | `glBufferSubData` 0.080 ms, `src/gl/buffer.rs:445`; `glMapBufferOES` 0.049 and 0.057 ms, `src/gl/buffer.rs:1107`. They are 19 to 26% of the frame; the rest, 0.22 to 0.25 ms, is glmark2's own (glmark2 filling the vertex data ('in glmark2 itself' in the profile)), not the library | Partly: the 0.05 to 0.08 ms in the entry points could be looked at (the copy-on-write path of a buffer a kick still reads, `sub_data_copy_on_write_*` in `src/gl/buffer.rs`), but the larger share is not YakoGL's |

Not in the top 5: `terrain` (rank 8, 0.202 ms): `glGenerateMipmap` 0.086 ms
is the largest entry, excluded: #916, and its `glDrawArrays` 0.051 ms is
one draw's cost like the above; `desktop:...effect=shadow` (0.195) and
`...effect=blur` (0.128): `glDrawArrays` 0.066 and 0.053 ms, several draws
a frame of the offscreen passes, with the same unnamed inner cost as 1 and
2 at a smaller size.

## What a rerun still owes

* The `top 3 symbols` column, with `GM2_PERF=1` on a host with `perf`: it
  decides whether `shadow` and `refract` (rank 1 and 2, two or three
  draws a frame each) are the vertex staging, the
  back end's record of the draw or a copy; their lines in the table above
  are the call path, not the sample.
* `shadow` on the board is excluded for its kick times. Its host CPU ms is
  listed here because the harness measures it; nothing is proposed for it.
