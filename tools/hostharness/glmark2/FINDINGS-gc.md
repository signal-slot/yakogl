<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2 on the GC7000 host build: CPU time of the GL calls per scene

Hand-written results of the harness's Profile mode on the vivante build
(README.md, "Profile"). Nothing in `run.sh`, `shim.c` or `report.py` was
changed. The note that reads these numbers is
[`docs/notes/imx8mp-glmark2-cpu.md`](../../../docs/notes/imx8mp-glmark2-cpu.md).

## How it was run

```sh
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gc-prof vivante
GM2_WRITE=0 GM2_PERF=1    GM2_FRAMES=20 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gc-perf vivante
```

- Tree: main `f691d33a`. glmark2 `22c527cb` (a git checkout, verified by
  `run.sh`). 33 default entries, 800x600, 20 frames each, one process per
  entry; the counted frames are frames 2 to 20 (19 frames).
- Board settings: `run.sh` set `YAKOGL_GC_CARVEOUT_BYTES=245642240` (the glmark2
  image's 251658240-byte `gpu-carveout` less 6016000 for the board layer's
  window and screen copy). 0 allocations refused, 0 GL errors, 0 refused
  draws in all 33 entries.
- The milliseconds are the `GM2_PROFILE=1` run (timed around each GL and EGL
  call). The split inside an entry point is the `GM2_PERF=1` run (perf at
  2000 Hz, counted frames only), as its share times the profile run's cpu ms.
- Host time on an x86-64 desktop, not the i.MX8MP's. The board's Cortex-A53
  is several times slower and its GPU memory is Normal non-cacheable (the
  host's malloc'd memory is cached). The ranking carries over; the
  milliseconds are a lower bound. GPU time and GPU waits are not in them:
  `src/osal/linux.c` completes every submit at once, and cache maintenance
  is a no-op there (on the board `gc_render_cache_flush` and `_invd` run
  over the range).
- Pictures are black on the host (nothing is rendered); that does not
  matter for the timing.

## Per scene

"GL ms" is the time in GL and EGL calls; the rest of the frame is glmark2's
own code (mesh updates, matrices). Sorted by the order of glmark2's default
list; the rank is by wall ms a frame.

| # | entry | rank | host ms a frame | GL ms | top entry points (wall ms a frame) |
|---|---|---:|---:|---:|---|
| 0 | `build:use-vbo=false` | 6 | 0.461 | 0.441 | `glDrawArrays` 0.440 |
| 1 | `build:use-vbo=true` | | 0.025 | 0.003 | `glDrawArrays` 0.002 |
| 2 | `texture:texture-filter=nearest` | | 0.032 | 0.003 | `glDrawArrays` 0.002 |
| 3 | `texture:texture-filter=linear` | | 0.039 | 0.004 | `glDrawArrays` 0.002 |
| 4 | `texture:texture-filter=mipmap` | | 0.041 | 0.004 | `glDrawArrays` 0.003 |
| 5 | `shading:shading=gouraud` | | 0.024 | 0.003 | `glDrawArrays` 0.002 |
| 6 | `shading:shading=blinn-phong-inf` | | 0.024 | 0.003 | `glDrawArrays` 0.002 |
| 7 | `shading:shading=phong` | | 0.025 | 0.003 | `glDrawArrays` 0.002 |
| 8 | `shading:shading=cel` | | 0.026 | 0.003 | `glDrawArrays` 0.002 |
| 9 | `bump:bump-render=high-poly` | | 0.028 | 0.003 | `glDrawArrays` 0.002 |
| 10 | `bump:bump-render=normals` | | 0.027 | 0.003 | `glDrawArrays` 0.002 |
| 11 | `bump:bump-render=height` | | 0.037 | 0.004 | `glDrawArrays` 0.002 |
| 12 | `effect2d` 3x3 kernel | | 0.025 | 0.003 | `glDrawArrays` 0.002 |
| 13 | `effect2d` 5x5 kernel | | 0.024 | 0.003 | `glDrawArrays` 0.002 |
| 14 | `pulsar:quads=5:texture=false:light=false` | | 0.081 | 0.010 | `glDrawArrays` 0.007 (5 calls) |
| 15 | `desktop:windows=4:effect=blur` | 9 | 0.282 | 0.037 | `glDrawArrays` 0.027 (14 calls) |
| 16 | `desktop:windows=4:effect=shadow` | 5 | 0.471 | 0.050 | `glDrawArrays` 0.036 (26 calls) |
| 17 | `buffer` map, no interleave | 8 | 0.352 | 0.053 | `glMapBufferOES` 0.048 |
| 18 | `buffer` subdata | 7 | 0.414 | 0.072 | `glBufferSubData` 0.067 (40 calls) |
| 19 | `buffer` map, interleave | 4 | 0.519 | 0.346 | `glMapBufferOES` 0.342 |
| 20 | `ideas:speed=duration` | 3 | 0.603 | 0.217 | `glDrawElements` 0.188 (162 calls), `glDrawArrays` 0.021 |
| 21 | `jellyfish` | | 0.078 | 0.009 | `glDrawElements` 0.003, `glDrawArrays` 0.002 |
| 22 | `terrain` | 2 | **8.988** | 8.791 | `glGenerateMipmap` **8.306** (5 calls), `glDrawArrays` 0.473 (8 calls) |
| 23 | `shadow` | 3 | **2.191** | 2.125 | `glDrawArrays` **2.118** (3 calls) |
| 24 | `refract` | 1 | **12.437** | 12.362 | `glGenerateMipmap` **10.267** (1 call), `glDrawArrays` 2.082 (2 calls) |
| 25 | `conditionals` 0 / 0 | | 0.018 | 0.002 | `glDrawArrays` 0.002 |
| 26 | `conditionals` 0 / 5 | | 0.018 | 0.002 | `glDrawArrays` 0.002 |
| 27 | `conditionals` 5 / 0 | | 0.025 | 0.004 | `glDrawArrays` 0.003 |
| 28 | `function` low | | 0.029 | 0.005 | `glDrawArrays` 0.003 |
| 29 | `function` medium | | 0.027 | 0.004 | `glDrawArrays` 0.003 |
| 30 | `loop` fragment-loop=false | | 0.027 | 0.005 | `glDrawArrays` 0.004 |
| 31 | `loop` fragment-uniform=false | | 0.031 | 0.007 | `glDrawArrays` 0.005 |
| 32 | `loop` fragment-uniform=true | | 0.051 | 0.007 | `glDrawArrays` 0.005 |

Ranks 1 to 9 hold everything above 0.25 ms; the other 24 scenes are under
0.1 ms a frame (a draw is built in 2 to 7 microseconds). Scene 32's wall
time is above its cpu time (0.051 against 0.034): a scheduling gap of one run,
not GL time.

## What the perf run says, per costly entry point

Own samples of the counted frames, as ms of the profile run's cpu time:

| scene | function | cpu ms a frame | what it is |
|---|---|---:|---|
| refract | `glGenerateMipmap` (`src/gl/texture.rs`, the CPU box filter after `backend::generate_mipmap` answered 0) | 10.43 (83%) | 1600x1200 RGBA8 colour target, the whole chain, per texel and channel |
| refract | `yakogl::vivante::backend::gc_depth_resolve` (`src/vivante/ds_cache.rs:resolve_depth` -> `ds_retention::write_depth_texels`) | 2.06 (16%) | D24S8 plane read back into an R32F depth texture |
| terrain | `glGenerateMipmap` | 8.74 (92%) | 5 calls a frame: terrain target twice, `bloom_h`, `bloom_v`, `tilt_h` |
| terrain | `yakogl::vivante::tiled_copy::tile` (`src/vivante/tiled_copy.rs`) | 0.29 (3%) | tiled copy of a 2D texture the vertex stage samples (`wanted_vs`) |
| shadow | `gc_depth_resolve` | 2.19 (96.5%) | the same depth resolve, once a frame, before the ground's draw |
| build:use-vbo=false | `yakogl::gl::draw::attrib_row`, `convert_attrib`, `memmove` | 0.30 + 0.08 | client vertex arrays copied into the frame arena at every draw |
| buffer, map + interleave | `glMapBufferOES` (`src/gl/buffer.rs:map_range`) | 0.34 | a fresh GPU allocation plus a copy of the buffer (`cache_invd`, `copy_nonoverlapping`, `cache_flush`) per map; on the host this includes page faults of a new malloc |

Setup (once a scene, not a frame): terrain 60 to 67 ms in GL calls before its
first swap returns (`glGenerateMipmap` 22 to 26 ms for 14 calls, `glCompileShader`
14 to 16 ms, `glBufferData` 8 to 9 ms); refract 47 to 54 ms
(`glGenerateMipmap` 21 to 26 ms); jellyfish 21 ms (`glTexImage2D` 10 ms for
33 calls). Not part of the score.

## After: glGenerateMipmap on the GPU (#1106)

The GL core now offers glmark2's rendered-into RGBA8 chains (`pe_order`,
at level 0's pitch) to `src/vivante/mipmap.rs`, which draws them
(docs/notes/imx8mp-glmark2-cpu.md, "After: the GL core's offer"). Profile
mode, `terrain` and `refract`, 20 frames, ms a frame over frames 2 to 20,
wall, the median of three interleaved runs, on another x86-64 host than
the table above (so the before column is not that table's). Before: main
`b4e5f46e`; after: the same plus the `src/gl/texture.rs` change of #1106.
Built with run.sh's sources and flags by hand (run.sh itself could not be
started in that session; the `yakogl` crate linked as an rlib, release
without LTO, instead of the assembled `libyakogl.a`; the note says how):

| entry | frame before | frame after | `glGenerateMipmap` before | after | `glDrawArrays` before | after |
|---|---:|---:|---:|---:|---:|---:|
| `terrain` | 11.39 | 0.87 | 10.32 (5 calls) | 0.054 | 0.40 | 0.20 |
| `refract` | 13.79 | 2.20 | 10.95 (1 call) | 0.013 | 2.28 | 1.96 |

glGenerateMipmap is out of the CPU box filter: every after run logs "the
GPU made the ... chain" and no chain left to the CPU; 0 GL errors and 0
refused draws in all runs. Refract's frame is now its depth resolve in
`glDrawArrays` (`gc_depth_resolve`, above).
