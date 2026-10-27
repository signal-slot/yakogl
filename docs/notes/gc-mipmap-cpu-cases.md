<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: which glmark2 glGenerateMipmap calls take the CPU filter, and why

Measured on 2026-10-20 with the host harness, GC7000 build. The library
sources are those of main `dc033dc4` (#1106: the GL core offers the
GC7000's chains to the back end). They are the same at the main tip
`6a157ddd`: no later commit touches `src/`, `include/`, `Cargo.toml`, the
CMake files or the harness scripts. Goes with issue #1106.

**Result: at this main, no glmark2 glGenerateMipmap call on the GC7000 takes
the CPU filter.** All 28 calls of the three scenes that call it, in setup and
3 frames each (1 in texture mipmap, 24 in terrain, 3 in refract), are offered to `src/vivante/mipmap.rs`
`gc_generate_mipmap`, which draws the chain. No call is refused. Before
`dc033dc4`, every one of them took the CPU box filter. `chain_pitch != 0`
refused all of them, and `pe_order` also refused the rendered-into targets.
The table gives both states and the CPU ms a frame of each.

## How it was measured

- **Scene list and ms a frame:** `GM2_PROFILE=1 GM2_WRITE=0
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm-gc-mipcases
  vivante`. glmark2 is at the pinned `22c527cb`. The run covers all 33
  entries, 3 frames each, 800x600, with the i.MX8MP glmark2 carve-out
  (`YAKOGL_GC_CARVEOUT_BYTES=245642240`). Every entry ended ok, with 0 GL
  errors, 0 refused draws and 0 allocations refused. Only 3 frames are
  taken (2 counted), so the three scenes that call glGenerateMipmap were run
  again with `GM2_FRAMES=20` (19 counted frames), three runs each. The ms
  below are the median of those three runs, wall time, from
  `profile-<gpu>.md`.
- **Per-call attributes:** a temporary print in `src/gl/texture.rs`
  `glGenerateMipmap`, not committed. It printed one line per call with the
  texture's size, `target`, `texfmt`, `es3_fmt`, `layers`, `chain_levels`,
  `chain_pitch`, `bgra`, `pe_order`, `level_above_zero_attached`, the path
  taken, and the call's time. A matching print in `src/vivante/mipmap.rs`
  gave the back end's reason for each refusal. The print was run on all 33
  entries. Lines came only from entries 4 (`texture:texture-filter=mipmap`),
  22 (`terrain`) and 24 (`refract`). The source agrees: glmark2 calls
  `glGenerateMipmap` only at `texture.cpp:95` (`Texture::load` with a
  mipmap min filter), `scene-terrain/base-renderer.cpp:89`
  (`update_mipmap`) and `scene-refract.cpp:430`.
- **Before:** `dc033dc4`'s parent differs in `src/` only by the two offer
  conditions `t.chain_pitch == 0` and `!t.pe_order`. For the "before"
  columns, those two lines were put back on this tree, temporarily. The runs
  were the same: the print over all 33 entries, and three 20-frame profile
  runs of the three entries. The tree was then restored. `git status` is
  clean apart from this note.

Host CPU time on an x86-64 machine, not the i.MX8MP's: the ranking carries
over, the milliseconds do not (`tools/hostharness/glmark2/README.md`,
"Profile"). These "before" milliseconds are about four times those recorded
in `docs/notes/imx8mp-glmark2-cpu.md` (terrain 10.32, refract 10.95). That
run used another host and a crate built by hand, so compare the figures
here with each other only.

## The 30 entries that never call it

`build` (2), `shading` (4), `conditionals` (3), `function` (2), `loop` (3),
`buffer` (3) and `ideas` load no mipmapped texture. Neither do `texture`
with nearest or linear, `bump` (3), `effect2d` (2), `pulsar`, `desktop` (2),
`jellyfish` and `shadow` (its depth texture is NEAREST). Confirmed by the
print: none of these 30 entries printed a line.

## The calls, per scene

All rows: `target` GL_TEXTURE_2D, `texfmt` RGBA8 (`GLES_TEXFMT_RGBA8`),
`es3_fmt` 0, `layers` 1, `bgra` false, no level above 0 attached to a
framebuffer (glmark2 attaches level 0 only). So the conditions that remain
in the offer (`src/gl/texture.rs`, "A backend that keeps level 0 ...") hold
for every call. `chain_pitch` is the GC7000 layout's level-0 pitch in
texels (`mip_level_align` 64), never 0. `pe_order` is true when a draw has
rendered into the texture since its last upload.

"Before" is the parent's offer. "Now" is this main. "ms a frame" is the
scene's `glGenerateMipmap` wall time over the counted frames (median of
three 20-frame runs). The per-call ms come from the print run, frames 2 and
3.

| scene (entry) | call site, texture | size, levels | `chain_pitch` | `pe_order` | before: refusing condition | before: path, ms a call | now: path, ms a call |
|---|---|---|---|---|---|---|---|
| `texture:texture-filter=mipmap` (4) | `texture.cpp:95`, `crate-base` image upload, once at load | 512x512, 10 | 512 | false | `chain_pitch != 0` | CPU filter, 6.5 (setup) | GPU, 0.26 (setup) |
| `terrain` (22) | `base-renderer.cpp:89`, `specular_map_renderer` target, at setup (still empty) | 512x512, 10 | 512 | false | `chain_pitch != 0` | CPU filter, 4.7 (setup) | GPU, 0.48 (setup) |
| `terrain` (22) | `texture.cpp:95`, uploads `terrain-grasslight-512`, `terrain-backgrounddetailed6`, `terrain-grasslight-512-nm`, once each | 512x512, 10 | 512 | false | `chain_pitch != 0` | CPU filter, 6.9 to 7.1 each (setup) | GPU, 0.10 to 0.13 each (setup) |
| `terrain` (22) | `base-renderer.cpp:89`, the terrain, bloom_h, bloom_v and tilt_h targets, at setup (still empty) | 800x600 (10), 256x256 (9) | 800, 256 | false | `chain_pitch != 0` | CPU filter, 14.6 and 21.0 (800x600, the NPOT chain grown), 3.1 and 2.1 (256x256) (setup) | GPU, 3.8 and 6.5 (800x600, the NPOT chain grown), 0.10 and 0.09 (256x256) (setup) |
| `terrain` (22) | `base-renderer.cpp:89`, `specular_map_renderer` target after its one render, frame 1 only | 512x512, 10 | 512 | true | `chain_pitch != 0`, `pe_order` | CPU filter, 6.9 (frame 1) | GPU, 0.08 (frame 1) |
| `terrain` (22) | `base-renderer.cpp:89`, every frame, 5 calls: the terrain target twice (its own `update_mipmap` and the overlay's `target_renderer_.update_mipmap()`), bloom_h, bloom_v, tilt_h | terrain and tilt_h 800x600 (10); bloom 256x256 (9) | 800, 256 | true | `chain_pitch != 0`, `pe_order` | CPU filter: 800x600 12.1 to 13.0 a call, 256x256 1.5 to 1.9 a call | GPU: 0.03 to 0.08 a call |
| `refract` (24) | `scene-refract.cpp:430`, every frame, 1 call: `tex_[COLOR]`, the colour target of the depth pass (twice the canvas) | 1600x1200, 11 | 1600 | true | `chain_pitch != 0`, `pe_order` | CPU filter, 52.0 to 52.6 a call (frame 1: 85.6, the NPOT chain grown) | GPU, 0.05 to 0.07 a call (frame 1: 25.3, the NPOT chain grown) |

Per scene, ms a frame (counted frames, wall, median of three 20-frame runs;
the three runs in brackets):

| entry | calls a frame | glGenerateMipmap before | now | frame before | now |
|---|---:|---:|---:|---:|---:|
| `texture:texture-filter=mipmap` | 0 (1 at setup) | - | - | 0.067 (0.065, 0.234, 0.067) | 0.054 (0.051, 0.054, 0.267) |
| `terrain` | 5 | 40.42 (40.42, 59.17, 40.14) | 0.153 (0.140, 0.187, 0.153) | 41.57 (41.57, 61.11, 41.28) | 1.012 (0.885, 2.141, 1.012) |
| `refract` | 1 | 47.38 (47.38, 50.87, 46.99) | 0.037 (0.036, 0.037, 0.041) | 52.01 (51.35, 53.59, 52.01) | 3.187 (4.408, 2.697, 3.187) |

In the 33-entry run (3 frames, 2 counted) at this main, glGenerateMipmap is
0.142 ms a frame in terrain and 0.030 ms in refract. Each of the three
entries logs `yakogl/vivante: the GPU made the ... chain` once. No entry
logs `left to the CPU`, so `gc_backend_mip_chains` counted no refusal. What
is left in glGenerateMipmap is the CPU recording the chain's draws (one
kick per level). Refract's remaining frame is `glDrawArrays`, the depth
resolve (`docs/notes/gc-depth-resolve.md`).

## Which cases the GPU path takes, and which need more

- **Taken once the offer moved (done in `dc033dc4`):** every call above.
  These are the 2D RGBA8 uploads (512x512) and the rendered-into targets
  (800x600 and 1600x1200 NPOT, 512x512 and 256x256 POT), at the chain's
  pitch, in either channel order. Dropping `chain_pitch == 0` admitted the
  uploads. Dropping `!pe_order` as well admitted the targets.
- **Needs more (another format, cube or array):** none of glmark2's calls.
  No call is a cube, an array, another format or `bgra`, and none has a
  level above 0 attached. Outside glmark2, the GL core still keeps the CPU
  filter for an uploaded BGRA/XRGB image (`t.bgra`), and the back end still
  refuses other formats, cubes and array stacks (`chain_gpu` in
  `src/vivante/mipmap.rs`).
- **Not the filter, but still CPU in glGenerateMipmap:** the first call on
  an NPOT texture grows its storage to a chain (`grow_npot_chain`, after
  `settle_texture`), on both paths. With the GPU chain, that call takes 3.8 and 6.5 ms for terrain's
  800x600 targets and 25.3 ms for refract's 1600x1200 target, in setup or
  frame 1 only, never in the counted frames.
