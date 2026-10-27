<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2-es2 on the GC7000 (vivante): every default scene, host CPU profile

The CPU time of the GL and EGL calls of each of the 33 default glmark2-es2
scenes (the list `README.md` describes) on the vivante back end, at 800x600.
Nothing is rendered and linux.c's submits complete at once, so these are
**host** milliseconds a frame, without any GPU wait. They rank the costs.
They are not the i.MX8MP's times: the A53 is several times slower and reads
non-cacheable GPU memory. Never read them as fps.

## Commit and commands

* yakogl: `8f0643c20f697e602f6c7da60c121bd4b5763233` (origin/main). It
  includes `dc033dc4`, the change that offers glmark2's rendered-into RGBA8
  chains to the GC7000's GPU mipmap path (issue #1106).
* glmark2: `22c527cb0556f3a1ac4445aaa52cc532760928d5`, a checkout without
  local changes, as `README.md` documents.
* Command, run twice on the same x86-64 host (20 frames per scene, one
  process per scene, frames 2 to 20 counted; `GM2_SIZE` left at its 800x600
  default; the carve-out `YAKOGL_GC_CARVEOUT_BYTES=245642240`, run.sh's
  default for the i.MX8MP glmark2 image):

```sh
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-gc-prof vivante
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-gc-prof2 vivante
```

* The depth resolve's own cost, for the attribution below (the timing test
  of `tests/gc_depth_resolve.rs`, one 1024 x 1024 resolve, 40 runs):

```sh
cargo test --release --no-default-features --features host-test,gpu-vivante,shaderc-online \
  --test gc_depth_resolve -- --nocapture time_one
```

The tables below are `<out>/profile-vivante.md` of the runs (not committed),
reduced to the scene's GL and EGL wall ms a frame and its three costliest
entry points.

## Per scene

"GL ms" is the wall time in GL and EGL calls a frame, in run 1 and run 2.
"Frame ms" (run 1) adds glmark2's own work, such as the per-frame vertex
updates of the `buffer` scenes. The top 3 entry points are from run 1 (wall
ms a frame, calls a frame). The memory-bound scenes move a lot between the
two runs (shadow 2.4 and 4.1 ms), so treat a single reading as within a
factor of about 1.7. A call of well under a microsecond shows as about
0.0015 ms: that is the timing wrapper's own cost (`README.md`, Profile).

| # | scene | GL ms run 1 / run 2 | frame ms | top 3 entry points, run 1 (wall ms a frame, calls) |
|---|---|---|---|---|
| 0 | `build:use-vbo=false` | 0.542 / 0.887 | 0.585 | `glDrawArrays` 0.522 (1x); `glVertexAttribPointer` 0.004 (2x); `glUniformMatrix4fv` 0.003 (2x) |
| 1 | `build:use-vbo=true` | 0.025 / 0.025 | 0.074 | `glEnableVertexAttribArray` 0.004 (2x); `glDrawArrays` 0.004 (1x); `glUniformMatrix4fv` 0.003 (2x) |
| 2 | `texture:texture-filter=nearest` | 0.033 / 0.041 | 0.097 | `glEnableVertexAttribArray` 0.004 (3x); `glVertexAttribPointer` 0.004 (3x); `glBindBuffer` 0.004 (3x) |
| 3 | `texture:texture-filter=linear` | 0.033 / 0.036 | 0.096 | `glDisableVertexAttribArray` 0.004 (3x); `glBindBuffer` 0.004 (3x); `glVertexAttribPointer` 0.004 (3x) |
| 4 | `texture:texture-filter=mipmap` | 0.033 / 0.036 | 0.098 | `glEnableVertexAttribArray` 0.005 (3x); `glBindBuffer` 0.004 (3x); `glDisableVertexAttribArray` 0.004 (3x) |
| 5 | `shading:shading=gouraud` | 0.024 / 0.027 | 0.073 | `glDrawArrays` 0.004 (1x); `glUniformMatrix4fv` 0.003 (2x); `glBindBuffer` 0.003 (2x) |
| 6 | `shading:shading=blinn-phong-inf` | 0.025 / 0.027 | 0.080 | `glDrawArrays` 0.004 (1x); `glBindBuffer` 0.003 (2x); `glDisableVertexAttribArray` 0.003 (2x) |
| 7 | `shading:shading=phong` | 0.026 / 0.034 | 0.078 | `glUniformMatrix4fv` 0.004 (3x); `glDrawArrays` 0.004 (1x); `glBindBuffer` 0.003 (2x) |
| 8 | `shading:shading=cel` | 0.031 / 0.026 | 0.088 | `glUniformMatrix4fv` 0.006 (3x); `glDrawArrays` 0.005 (1x); `glBindBuffer` 0.003 (2x) |
| 9 | `bump:bump-render=high-poly` | 0.027 / 0.027 | 0.084 | `glDrawArrays` 0.004 (1x); `glUniformMatrix4fv` 0.003 (2x); `glBindBuffer` 0.003 (2x) |
| 10 | `bump:bump-render=normals` | 0.030 / 0.027 | 0.088 | `glDrawArrays` 0.004 (1x); `glEnableVertexAttribArray` 0.004 (2x); `glBindBuffer` 0.003 (2x) |
| 11 | `bump:bump-render=height` | 0.040 / 0.038 | 0.121 | `glBindBuffer` 0.006 (4x); `glVertexAttribPointer` 0.006 (4x); `glEnableVertexAttribArray` 0.006 (4x) |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | 0.020 / 0.021 | 0.062 | `glDrawArrays` 0.004 (1x); `eglSwapBuffers` 0.002 (1x); `glClear` 0.002 (1x) |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | 0.025 / 0.020 | 0.073 | `glDrawArrays` 0.006 (1x); `glActiveTexture` 0.003 (1x); `eglSwapBuffers` 0.002 (1x) |
| 14 | `pulsar:quads=5:texture=false:light=false` | 0.088 / 0.088 | 0.246 | `glDrawArrays` 0.018 (5x); `glBindBuffer` 0.014 (10x); `glEnableVertexAttribArray` 0.014 (10x) |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | 0.345 / 0.318 | 0.951 | `glDrawArrays` 0.061 (14x); `glVertexAttribPointer` 0.043 (28x); `glUseProgram` 0.043 (28x) |
| 16 | `desktop:windows=4:effect=shadow` | 0.598 / 0.615 | 1.613 | `glDrawArrays` 0.106 (26x); `glUseProgram` 0.077 (52x); `glEnableVertexAttribArray` 0.077 (52x) |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | 0.100 / 0.150 | 0.522 | `glMapBufferOES` 0.045 (4x); `glBindBuffer` 0.014 (9x); `glDrawArrays` 0.008 (1x) |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | 0.180 / 0.267 | 0.637 | `glBufferSubData` 0.133 (40x); `glBindBuffer` 0.013 (9x); `glDrawArrays` 0.007 (1x) |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | 0.078 / 0.088 | 0.313 | `glMapBufferOES` 0.035 (1x); `glBindBuffer` 0.009 (6x); `glDrawArrays` 0.007 (1x) |
| 20 | `ideas:speed=duration` | 0.751 / 0.742 | 1.648 | `glDrawElements` 0.502 (162x); `glUniformMatrix4fv` 0.065 (43x); `glVertexAttribPointer` 0.058 (40x) |
| 21 | `jellyfish` | 0.080 / 0.088 | 0.229 | `glBindBuffer` 0.009 (6x); `glEnableVertexAttribArray` 0.009 (6x); `glVertexAttribPointer` 0.008 (6x) |
| 22 | `terrain` | 0.477 / 0.498 | 0.906 | `glDrawArrays` 0.219 (8x); `glGenerateMipmap` 0.053 (5x); `glBindTexture` 0.026 (18x) |
| 23 | `shadow` | 2.430 / 4.059 | 2.579 | `glDrawArrays` 2.348 (3x); `glUniformMatrix4fv` 0.010 (5x); `glBindBuffer` 0.009 (6x) |
| 24 | `refract` | 2.314 / 3.600 | 2.467 | `glDrawArrays` 2.215 (2x); `glGenerateMipmap` 0.015 (1x); `glUniformMatrix4fv` 0.009 (5x) |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | 0.017 / 0.019 | 0.053 | `glDrawArrays` 0.004 (1x); `eglSwapBuffers` 0.002 (1x); `glDisableVertexAttribArray` 0.001 (1x) |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | 0.017 / 0.016 | 0.050 | `glDrawArrays` 0.004 (1x); `eglSwapBuffers` 0.002 (1x); `glUniformMatrix4fv` 0.002 (1x) |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | 0.017 / 0.019 | 0.051 | `glDrawArrays` 0.004 (1x); `eglSwapBuffers` 0.002 (1x); `glUniformMatrix4fv` 0.001 (1x) |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | 0.017 / 0.018 | 0.050 | `glDrawArrays` 0.004 (1x); `eglSwapBuffers` 0.002 (1x); `glUniformMatrix4fv` 0.001 (1x) |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | 0.017 / 0.018 | 0.052 | `glDrawArrays` 0.004 (1x); `eglSwapBuffers` 0.002 (1x); `glUniformMatrix4fv` 0.002 (1x) |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | 0.017 / 0.017 | 0.051 | `glDrawArrays` 0.004 (1x); `eglSwapBuffers` 0.002 (1x); `glUniformMatrix4fv` 0.002 (1x) |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | 0.018 / 0.017 | 0.054 | `glDrawArrays` 0.004 (1x); `eglSwapBuffers` 0.002 (1x); `glUniformMatrix4fv` 0.002 (1x) |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | 0.017 / 0.016 | 0.050 | `glDrawArrays` 0.004 (1x); `eglSwapBuffers` 0.002 (1x); `glUniformMatrix4fv` 0.002 (1x) |

Only `shadow` and `refract` are above 1 ms of GL time a frame in either run.
24 of the 33 scenes stay at or under 0.1 ms in both runs.

## Where shadow's and refract's draw time goes

Both scenes render depth into a depth texture of twice the canvas,
1600 x 1200 (glmark2 `scene-shadow.cpp` and `scene-refract.cpp`,
`width_ = canvas_.width() * 2`), then sample it in the next draw of the frame.
On this back end the texture unit does not sample the D24S8 surface, so that
draw resolves the depth into the texture's R32F texels on the CPU
(`gl/texture.rs` `depth_for_draw` → `resolve_depth` →
`vivante/ds_cache.rs` `resolve_depth`): 1.92 M texels, a 7.68 MB read and a
7.68 MB write each frame. The R32F texture has one level and is plain 2D, so
it is sampled as it is, with no tiled copy made after the resolve
(`vivante/tiled_copy.rs`). The timing test above gives 0.35 ms (best) and
1.10 ms (mean) for 1 M texels on this host, so the 1.92 M texels come to about
0.7 ms at best and 2.1 ms at that mean, and more with the 15 MB working
set out of the cache. Any other draw
here costs about 0.004 ms. So the 2.2 to 4.0 ms of `glDrawArrays` is the
depth resolve, and its spread between runs is that of a memory-bound copy.
No other draw-time cost was found in these two scenes. (perf was not used
for this split; `GM2_PERF=1 GM2_ENTRIES='^(shadow|refract)$'` would give the
function shares.)

## Ranked CPU costs

| rank | scene | cost, GL wall ms a frame (run 1 / run 2) | covered by running work? |
|---|---|---|---|
| 1 | `shadow` | `glDrawArrays` 2.35 / 3.96 (3 calls) | yes: the CPU depth resolve (`docs/notes/gc-depth-resolve.md`; sampling the surface instead, `docs/notes/gc-depth-sample.md`) |
| 2 | `refract` | `glDrawArrays` 2.22 / 3.48 (2 calls) | yes: the same CPU depth resolve. Its `glGenerateMipmap` is now 0.015 ms a frame: the CPU box filter is gone (issue #1106) |
| 3 | `build:use-vbo=false` | `glDrawArrays` 0.52 / 0.87 (1 call) | no; under 1 ms (candidate A below) |
| 4 | `ideas:speed=duration` | `glDrawElements` 0.50 / 0.50 (162 calls, 3 us each) | no; under 1 ms |
| 5 | `desktop:windows=4:effect=shadow` | 0.60 / 0.62 over about 360 calls, `glDrawArrays` 0.11 (26 calls) | no; under 1 ms, mostly the profile's per-call cost |
| 6 | `terrain` | `glDrawArrays` 0.22 / 0.24 (8 calls), `glGenerateMipmap` 0.05 (5 calls) | the mipmap: yes, issue #1106 (no longer the CPU filter). The draws: under 1 ms |
| 7 | `buffer ... subdata` | `glBufferSubData` 0.13 / 0.21 (40 calls) | no; under 1 ms |
| 8 | `buffer ... map` | `glMapBufferOES` 0.045 and 0.035 | no; under 1 ms |

## New candidates above 1 ms a frame

None. The only costs above 1 ms a frame are shadow's and refract's depth
resolve, which running work covers (above). That cost needs no new issue.

The two largest costs under 1 ms here, which the i.MX8MP's slower CPU
multiplies:

* **A: `build:use-vbo=false`, 0.52 / 0.87 ms a frame in one `glDrawArrays`.**
  The VBO variant of the same scene costs 0.004 ms in that call, so this is
  the client-array path. Suspected path (read in the source, not measured
  per function): `convert_attrib` in `src/gl/draw.rs` converts a client array
  one vertex at a time (`attrib_row`, `dword_bytes`, one copy per row). It
  does this also for an all-float array whose row is already the output row.
  A bulk copy for float arrays with matching component counts would remove
  most of it.
* **`ideas:speed=duration`, 0.50 ms a frame in 162 `glDrawElements`.** About
  3 us of fixed cost a draw; no single path stands out at this size.

Setup and first frame (shader compiles, `eglInitialize`, the first
`glGenerateMipmap`) are in each scene's setup line of the profile and are not
counted here. glmark2's own per-frame time (the `buffer`, `desktop` and
`ideas` scenes' updates, up to 1.0 ms) is not GL-call time.
