<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2-es2 on the V3D: host CPU time of every default scene

The CPU time the GL core and the V3D back end spend in GL and EGL calls, per
default scene of glmark2-es2 (the 33 entries of `src/default-benchmarks.h`),
at 800x600, with the TFU on (the Pi 4 board's setting,
`CONFIG_YAKOGL_V3D_TFU=y`) and off. Milliseconds a frame, never frames a
second. Host time on the machine that ran it, not the Pi 4's: the Cortex-A72
at 1.5 GHz is several times slower and reads non-cacheable GPU memory. The
ranking carries over, the milliseconds do not. The GPU's own time and the
board's waiting for it are not in the numbers (`src/osal/linux.c` completes
every submit at once); the CPU waits are counted instead (below).

## What was run

* YakoGL `4b057224` (the integration tip this item started from). `origin/main`
  was one commit ahead, `b4e5f46e`, which changes only `tools/check.sh`: no
  driver code differs.
* glmark2 `22c527cb0556f3a1ac4445aaa52cc532760928d5`, the commit the Zephyr
  image pins (a git checkout, verified by `run.sh`), as `$GLMARK2_SRC`.
* One process per scene, 20 frames; the counted frames are 2 to 20 (the
  first is setup), one run at a time on the host (README.md, "Profile").

```sh
# TFU on: run.sh passes the Pi 4 board file's settings (96 MiB carve-out,
# 24 MiB tiled copies, TFU on) for what the environment does not set
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 YAKOGL_V3D_TFU=1 \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2-tfu v3d
# TFU off: the CPU path
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 YAKOGL_V3D_TFU=0 \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2-notfu v3d
```

`build/<out>/profile-v3d.md` of each run holds the full per-scene tables the
ones below are cut from. The columns:

* GL ms/frame: wall time in GL and EGL calls, per counted frame (the
  profile's "in GL and EGL calls").
* top 3 entry points: by wall ms a frame, the entry point and its ms.
* CPU waits/frame: calls of the V3D OSAL's wait (`l_wait` in
  `src/osal/linux.c`, the host's stand-in for `osal_v3d` waiting on a fence;
  every `wait_in_flight` in `src/v3d/backend.rs` ends in one). The profile
  does not count them and the host takes no time in them, so they were
  counted with gdb (`break l_wait`, hit count) on the same binary with the
  same settings: the count of a 21-frame run minus a 1-frame run, over 20
  frames. This is every wait, not only the glGenerateMipmap ones: the
  swap's wait, a render-to-texture sampled by the next pass, the arena
  waits. On the board each one is a CPU stall until the GPU has finished the
  job in flight; the host cannot show how long.

Noise: two runs of one scene differ by tens of percent for calls under
0.1 ms, and `build:use-vbo=false` showed 4.0 and 2.1 ms (below). Read the
rank, not the third digit.

## Per scene

### TFU on (YAKOGL_V3D_TFU=1)

| # | scene | GL ms/frame | top 3 entry points (wall ms/frame) | CPU waits/frame |
|---|---|---|---|---|
| 0 | `build:use-vbo=false` | 4.042 | `glDrawArrays` 3.993, `eglSwapBuffers` 0.042, `glFinish` 0.002 | 1.00 |
| 1 | `build:use-vbo=true` | 0.042 | `eglSwapBuffers` 0.023, `glDrawArrays` 0.016, `glFinish` 0.001 | 1.00 |
| 2 | `texture:texture-filter=nearest` | 0.095 | `eglSwapBuffers` 0.049, `glDrawArrays` 0.039, `glFinish` 0.001 | 1.00 |
| 3 | `texture:texture-filter=linear` | 0.068 | `glDrawArrays` 0.033, `eglSwapBuffers` 0.029, `glFinish` 0.001 | 1.00 |
| 4 | `texture:texture-filter=mipmap` | 0.070 | `glDrawArrays` 0.035, `eglSwapBuffers` 0.028, `glFinish` 0.001 | 1.00 |
| 5 | `shading:shading=gouraud` | 0.044 | `eglSwapBuffers` 0.023, `glDrawArrays` 0.018, `glFinish` 0.001 | 1.00 |
| 6 | `shading:shading=blinn-phong-inf` | 0.069 | `eglSwapBuffers` 0.032, `glDrawArrays` 0.031, `glFinish` 0.001 | 1.00 |
| 7 | `shading:shading=phong` | 0.051 | `eglSwapBuffers` 0.024, `glDrawArrays` 0.023, `glUniformMatrix4fv` 0.001 | 1.00 |
| 8 | `shading:shading=cel` | 0.049 | `eglSwapBuffers` 0.025, `glDrawArrays` 0.021, `glFinish` 0.001 | 1.00 |
| 9 | `bump:bump-render=high-poly` | 0.058 | `eglSwapBuffers` 0.028, `glDrawArrays` 0.025, `glFinish` 0.001 | 1.00 |
| 10 | `bump:bump-render=normals` | 0.045 | `eglSwapBuffers` 0.021, `glDrawArrays` 0.021, `glFinish` 0.001 | 1.00 |
| 11 | `bump:bump-render=height` | 0.077 | `glDrawArrays` 0.039, `eglSwapBuffers` 0.030, `glFinish` 0.002 | 1.00 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | 0.045 | `eglSwapBuffers` 0.022, `glDrawArrays` 0.020, `glFinish` 0.001 | 1.00 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | 0.060 | `glDrawArrays` 0.029, `eglSwapBuffers` 0.027, `glFinish` 0.002 | 1.00 |
| 14 | `pulsar:quads=5:texture=false:light=false` | 0.107 | `glDrawArrays` 0.067, `eglSwapBuffers` 0.031, `glUniformMatrix4fv` 0.002 | 1.00 |
| 15 | `desktop:windows=4:effect=blur (radius 5, passes 1, separable)` | 0.625 | `glDrawArrays` 0.577, `eglSwapBuffers` 0.024, `glClear` 0.006 | 11.00 |
| 16 | `desktop:windows=4:effect=shadow` | 0.402 | `glDrawArrays` 0.341, `eglSwapBuffers` 0.022, `glUseProgram` 0.006 | 3.00 |
| 17 | `buffer:update-method=map:interleave=false (fraction 0.5, dispersion 0.9, columns 200)` | 0.473 | `glMapBufferOES` 0.335, `glDrawArrays` 0.070, `eglSwapBuffers` 0.054 | 1.00 |
| 18 | `buffer:update-method=subdata:interleave=false (fraction 0.5, dispersion 0.9, columns 200)` | 0.927 | `glBufferSubData` 0.780, `glDrawArrays` 0.072, `eglSwapBuffers` 0.060 | 1.00 |
| 19 | `buffer:update-method=map:interleave=true (fraction 0.5, dispersion 0.9, columns 200)` | 0.402 | `glMapBufferOES` 0.289, `glDrawArrays` 0.055, `eglSwapBuffers` 0.046 | 1.00 |
| 20 | `ideas:speed=duration` | 1.761 | `glDrawElements` 1.482, `glDrawArrays` 0.195, `eglSwapBuffers` 0.050 | 1.00 |
| 21 | `jellyfish` | 0.093 | `eglSwapBuffers` 0.025, `glDrawArrays` 0.018, `glFinish` 0.001 | 1.00 |
| 22 | `terrain` | 1.518 | `glDrawArrays` 0.732, `eglSwapBuffers` 0.051, `glClear` 0.014 | 9.00 |
| 23 | `shadow` | 0.133 | `glDrawArrays` 0.096, `eglSwapBuffers` 0.022, `glClear` 0.006 | 3.00 |
| 24 | `refract` | 0.494 | `glDrawArrays` 0.315, `eglSwapBuffers` 0.054, `glClear` 0.013 | 3.00 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | 0.044 | `eglSwapBuffers` 0.023, `glDrawArrays` 0.018, `glFinish` 0.001 | 1.00 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | 0.052 | `eglSwapBuffers` 0.024, `glDrawArrays` 0.024, `glFinish` 0.001 | 1.00 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | 0.050 | `eglSwapBuffers` 0.027, `glDrawArrays` 0.020, `glFinish` 0.001 | 1.00 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | 0.055 | `eglSwapBuffers` 0.029, `glDrawArrays` 0.022, `glFinish` 0.001 | 1.00 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | 0.068 | `eglSwapBuffers` 0.033, `glDrawArrays` 0.029, `glFinish` 0.002 | 1.00 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | 0.052 | `eglSwapBuffers` 0.026, `glDrawArrays` 0.023, `glFinish` 0.001 | 1.00 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | 0.041 | `eglSwapBuffers` 0.020, `glDrawArrays` 0.018, `glFinish` 0.001 | 1.00 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | 0.050 | `eglSwapBuffers` 0.025, `glDrawArrays` 0.022, `glFinish` 0.001 | 1.00 |

### TFU off (YAKOGL_V3D_TFU=0)

| # | scene | GL ms/frame | top 3 entry points (wall ms/frame) | CPU waits/frame |
|---|---|---|---|---|
| 0 | `build:use-vbo=false` | 2.122 | `glDrawArrays` 2.071, `eglSwapBuffers` 0.043, `glFinish` 0.002 | 1.00 |
| 1 | `build:use-vbo=true` | 0.042 | `eglSwapBuffers` 0.021, `glDrawArrays` 0.018, `glFinish` 0.001 | 1.00 |
| 2 | `texture:texture-filter=nearest` | 0.055 | `eglSwapBuffers` 0.026, `glDrawArrays` 0.025, `glFinish` 0.001 | 1.00 |
| 3 | `texture:texture-filter=linear` | 0.058 | `glDrawArrays` 0.028, `eglSwapBuffers` 0.024, `glFinish` 0.001 | 1.00 |
| 4 | `texture:texture-filter=mipmap` | 0.054 | `glDrawArrays` 0.027, `eglSwapBuffers` 0.023, `glFinish` 0.001 | 1.00 |
| 5 | `shading:shading=gouraud` | 0.045 | `eglSwapBuffers` 0.024, `glDrawArrays` 0.018, `glFinish` 0.001 | 1.00 |
| 6 | `shading:shading=blinn-phong-inf` | 0.045 | `eglSwapBuffers` 0.024, `glDrawArrays` 0.018, `glFinish` 0.001 | 1.00 |
| 7 | `shading:shading=phong` | 0.047 | `eglSwapBuffers` 0.024, `glDrawArrays` 0.019, `glFinish` 0.001 | 1.00 |
| 8 | `shading:shading=cel` | 0.050 | `eglSwapBuffers` 0.023, `glDrawArrays` 0.023, `glFinish` 0.001 | 1.00 |
| 9 | `bump:bump-render=high-poly` | 0.044 | `eglSwapBuffers` 0.024, `glDrawArrays` 0.017, `glFinish` 0.001 | 1.00 |
| 10 | `bump:bump-render=normals` | 0.049 | `glDrawArrays` 0.023, `eglSwapBuffers` 0.023, `glFinish` 0.001 | 1.00 |
| 11 | `bump:bump-render=height` | 0.049 | `glDrawArrays` 0.023, `eglSwapBuffers` 0.022, `glFinish` 0.001 | 1.00 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | 0.037 | `eglSwapBuffers` 0.018, `glDrawArrays` 0.017, `glFinish` 0.001 | 1.00 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | 0.041 | `eglSwapBuffers` 0.020, `glDrawArrays` 0.018, `glFinish` 0.001 | 1.00 |
| 14 | `pulsar:quads=5:texture=false:light=false` | 0.094 | `glDrawArrays` 0.060, `eglSwapBuffers` 0.026, `glUniformMatrix4fv` 0.002 | 1.00 |
| 15 | `desktop:windows=4:effect=blur (radius 5, passes 1, separable)` | 0.310 | `glDrawArrays` 0.280, `eglSwapBuffers` 0.014, `glClear` 0.003 | 11.00 |
| 16 | `desktop:windows=4:effect=shadow` | 0.459 | `glDrawArrays` 0.389, `eglSwapBuffers` 0.027, `glClear` 0.006 | 3.00 |
| 17 | `buffer:update-method=map:interleave=false (fraction 0.5, dispersion 0.9, columns 200)` | 0.587 | `glMapBufferOES` 0.450, `glDrawArrays` 0.069, `eglSwapBuffers` 0.053 | 1.00 |
| 18 | `buffer:update-method=subdata:interleave=false (fraction 0.5, dispersion 0.9, columns 200)` | 0.671 | `glBufferSubData` 0.507, `glDrawArrays` 0.086, `eglSwapBuffers` 0.063 | 1.00 |
| 19 | `buffer:update-method=map:interleave=true (fraction 0.5, dispersion 0.9, columns 200)` | 0.534 | `glMapBufferOES` 0.413, `glDrawArrays` 0.064, `eglSwapBuffers` 0.046 | 1.00 |
| 20 | `ideas:speed=duration` | 1.723 | `glDrawElements` 1.461, `glDrawArrays` 0.187, `eglSwapBuffers` 0.046 | 1.00 |
| 21 | `jellyfish` | 0.107 | `glDrawElements` 0.067, `eglSwapBuffers` 0.019, `glDrawArrays` 0.014 | 1.00 |
| 22 | `terrain` | 59.370 | `glGenerateMipmap` 41.542, `glDrawArrays` 17.686, `eglSwapBuffers` 0.047 | 9.00 |
| 23 | `shadow` | 0.196 | `glDrawArrays` 0.144, `eglSwapBuffers` 0.030, `glClear` 0.008 | 3.00 |
| 24 | `refract` | 46.706 | `glGenerateMipmap` 32.306, `glDrawArrays` 14.306, `eglSwapBuffers` 0.046 | 3.00 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | 0.043 | `eglSwapBuffers` 0.022, `glDrawArrays` 0.018, `glFinish` 0.001 | 1.00 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | 0.039 | `eglSwapBuffers` 0.020, `glDrawArrays` 0.016, `glFinish` 0.001 | 1.00 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | 0.040 | `eglSwapBuffers` 0.021, `glDrawArrays` 0.017, `glFinish` 0.001 | 1.00 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | 0.046 | `eglSwapBuffers` 0.024, `glDrawArrays` 0.019, `glFinish` 0.001 | 1.00 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | 0.046 | `eglSwapBuffers` 0.025, `glDrawArrays` 0.018, `glFinish` 0.001 | 1.00 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | 0.053 | `eglSwapBuffers` 0.028, `glDrawArrays` 0.021, `glFinish` 0.001 | 1.00 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | 0.044 | `eglSwapBuffers` 0.022, `glDrawArrays` 0.018, `glFinish` 0.001 | 1.00 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | 0.043 | `eglSwapBuffers` 0.022, `glDrawArrays` 0.018, `glFinish` 0.001 | 1.00 |

Terrain with the TFU on: `glGenerateMipmap` is 0.656 ms a frame of CPU for 5
calls, and the TFU model's host time for those jobs (116.8 ms a frame) is
outside the figures, as the board's GPU time is. With the TFU off, terrain
is 59.4 ms and refract 46.7 ms a frame, 41.5 and 32.3 ms of it in
`glGenerateMipmap`'s CPU box filter: the board does not take that path
(`docs/notes/pi4-glmark2-cpu.md`), so it is not a candidate.

## Ranked CPU costs

Ranked by host ms a frame, TFU on (the board's setting), above 0.3 ms. An
issue is opened only for a new candidate above 1 ms a frame.

| rank | scene | cost | ms a frame | covered by | issue |
|---|---|---|---|---|---|
| 1 | terrain | 5 `glGenerateMipmap` a frame, each submits the open render job and waits for it before the TFU job: 9 CPU waits a frame in all (host CPU for the calls: 0.656 ms; the waits are the board's cost, 178.9 ms a frame on the Pi 4) | 1.518 GL, 9 waits | running item v3d-tfu-mipmap-nowait | none |
| 2 | build:use-vbo=false | the one `glDrawArrays` converts the client-side vertex arrays at draw time (`convert_attrib`, `src/gl/draw.rs`) | 4.0 (TFU on), 2.1 (TFU off) | no | ISSUE-1 |
| 3 | ideas | 162 `glDrawElements` a frame, about 9 us each, from VBOs | 1.761 (1.723 off) | no | ISSUE-2 |
| 4 | buffer, update-method=subdata | `glBufferSubData` | 0.780 of 0.927 (0.507 of 0.671 off) | no: below 1 ms and it moved between the runs | none |
| 5 | desktop, blur | `glDrawArrays` 0.577; 11 CPU waits a frame, the passes sample each other's render targets | 0.625, 11 waits | wait: the same `wait_in_flight` family as terrain, not the mipmap one | none |
| 6 | refract | `glDrawArrays` 0.315 (depth resolve and the mipmap are the running items'); 3 waits | 0.494, 3 waits | gc-depth-resolve / v3d-tfu-mipmap-nowait | none |
| 7 | buffer, update-method=map (both) | `glMapBufferOES`, 4 calls a frame | 0.335 and 0.289 (0.450 and 0.413 off) | no: below 1 ms | none |
| 8 | desktop, shadow | `glDrawArrays` 0.341; 3 waits | 0.402 | no: below 1 ms | none |

Every other scene is under 0.11 ms a frame, with 1 wait (the swap's). The
scenes that carry waits: terrain 9, desktop blur 11, desktop shadow 3,
shadow 3, refract 3. The wait counts are the same with the TFU off. The
causes were not separated (no per-call-site counter is printed by the back
end); terrain's 9 a frame are 5 `glGenerateMipmap` and the render-to-texture
passes that sample a target the job in flight is still writing.

What the planner should read from this: apart from the mipmap wait that
v3d-tfu-mipmap-nowait is already removing, no scene's GL time on the host is
above 2 ms a frame except `build:use-vbo=false`, which the board's glmark2
list runs as its first entry. The other waits (desktop blur's 11 a frame,
the 3 of shadow, refract and desktop shadow) are what is left to look at on
the board after the no-wait mipmap lands; the host cannot give them a time.
No fix was tried: this item changes no code.

## Issues

Both candidates above 1 ms a frame, with the numbers and the suspected code
path, as issue texts. `gh issue create` was refused in the session that
wrote this file, so they are **not yet opened** and carry no link: open them
with the texts below and put the numbers in place of "ISSUE-n" above.

* ISSUE-1, "v3d glmark2 build:use-vbo=false: glDrawArrays 2.1-4.0 ms a frame
  on the host (client-side vertex arrays)": the scene is one `glDrawArrays`,
  3.993 ms (TFU on) and 2.071 ms (TFU off); the VBO variant costs 0.016 ms.
  Suspected: `convert_attrib` and the client-array staging in
  `src/gl/draw.rs` convert every attribute's vertices into the scratch each
  draw; float32 arrays in the fetch's layout could be copied by memcpy.
  Confirm with `GM2_PERF=1`.
* ISSUE-2, "v3d glmark2 ideas: 162 glDrawElements a frame cost 1.5 ms on the
  host": 1.482 ms (TFU on) and 1.461 ms (TFU off) of 1.761 and 1.723 ms, VBOs
  with 16-bit indices, no client arrays, about 9 us a draw. Suspected: the
  per-draw fixed cost in `src/gl/draw.rs` `glDrawElements` and the back end's
  per-draw state packing. Confirm with `GM2_PERF=1`.

`perf` was not permitted on the host that ran this (`perf_event_paranoid`
4), so the suspected paths come from reading the code, not from samples.
