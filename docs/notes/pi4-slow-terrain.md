<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 terrain: where the 161.8 ms frame goes (host breakdown)

The host breakdown of glmark2 terrain on the Pi 4 (#1209): at main `c4af4917`
the board takes 161.79 ms a frame, 156.06 ms of it in GL calls, while the
GPU work of a frame is 29.1 ms and the timed draw steps sum to 0.17 ms
(`docs/notes/pi4-glmark2-cpu.md`, "On the board: glmark2 from main
`c4af4917`"). Code read and host runs at main `aaf8e53e` (2026-11-07; the
V3D, GL core and OSAL sources are those of `c4af4917`'s successors, no change
since that touches the paths below). Nothing here ran on a board.

## Result

**The 130 ms is a render-image table that thrashes, and it thrashes because
of a stale GL-core table entry, not because of terrain.** The host harness
runs each glmark2 entry in its own process; the board runs the list in one.
Run in one process after the two desktop scenes, terrain registers 9 render
targets for the V3D's 8 render-image slots (`RT_IMAGES`,
`src/v3d/backend.rs:286`) instead of its 7: two of its *uploaded* 512x512
textures inherit a nonzero `target_kick` from the desktop scenes' deleted
render targets, because `free_texture` (`src/gl/texture.rs:5009`) never
clears the texture's `TARGET_USE` entry (`src/gl/framebuffer.rs:96`). The
least-recently-used eviction (`rt_register`, `src/v3d/backend.rs:1856`) then
loses three of terrain's targets every frame. Of terrain's 5
`glGenerateMipmap` calls, 2 find no slot and are filtered by the GL core's
CPU box filter, a byte at a time, over non-cacheable memory, after a full
GPU drain. 3 samples a frame find no slot and are copied, two of them as
whole mip chains tiled on the CPU. The host reproduces the board's counters
exactly (3 TFU chains, 2 `unknown` refusals and 3 `first-sample` copies a
frame). Its own GL time for the frame rises from 0.33 ms to 4.85 ms
(14.6x). With the board's non-cacheable memory that is an estimated 95 to
140 ms a frame, which is the unexplained ~130 ms. Clearing the entry on
delete was tried on the host in a scratch tree (not in this change). Terrain
then registers its 7 targets, nothing is evicted, and all 5 chains go to the
TFU. Expected board frame after that fix: about 30 ms, GPU-bound.

## How it was measured

Host: x86-64, glmark2 at `22c527cb` (`$GLMARK2_SRC`, verified by `run.sh`),
the V3D build with the Pi 4 image's settings (`v3d-settings.txt`:
`YAKOGL_V3D_CARVEOUT_BYTES=100663296`, `YAKOGL_V3D_TILED_COPIES_BYTES=25165824`,
`YAKOGL_V3D_TFU=1` or `0`). The scripts are in `tools/hostharness/pi4-terrain/`
(new; `tools/hostharness/glmark2/` is unchanged):

```sh
# (1) the item's commands: terrain alone, TFU model on and off, GM2_PROFILE=1 and GM2_PERF=1
tools/hostharness/pi4-terrain/run.sh "$GLMARK2_SRC" build/pi4-terrain
#   = GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 GM2_ENTRIES='^terrain$' YAKOGL_V3D_TFU=1|0 \
#       tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/pi4-terrain/prof-tfu1|0 v3d
#     and the same with GM2_PERF=1, then summarize.py over each
# (2) the board's order: list entries FIRST..LAST in one process (terrain is entry 22)
tools/hostharness/pi4-terrain/one-process.sh build/pi4-terrain/prof-tfu1 0 23 20
# (3) terrain's own frames in that state: two runs differing only in terrain's frame count
tools/hostharness/pi4-terrain/one-process.sh --profile build/pi4-terrain/prof-tfu1 15 22 20 20 > a.txt
tools/hostharness/pi4-terrain/one-process.sh --profile build/pi4-terrain/prof-tfu1 15 22 20 40 > b.txt
python3 -I tools/hostharness/pi4-terrain/framediff.py a.txt b.txt 20
```

- `GM2_PERF=1` did not run on this host: `perf record` failed with
  "`perf_event_paranoid setting is 4`", "Failure to open any events for
  recording" (`build/pi4-terrain/perf-tfu1/logs/v3d/0.log`). The per-function
  split below comes from the counters, the per-entry-point times and the
  code, not from perf samples.
- The host completes every job inside `submit()` (`src/osal/linux.c`), so
  its waits cost nothing; on the host the wait *counts* are the metric.
  The TFU model's time is GPU work and is taken out of the call times by
  `shim.c`.

## Host figures

### (1) Terrain alone, TFU model on (`prof-tfu1`): 0.33 ms of GL time a frame

`summarize.py build/pi4-terrain/prof-tfu1`, frames 2..20:

```
frame: wall 591.1 us, cpu 586.0 us, in GL/EGL calls 328.0 us; TFU model jobs 5.00 a frame, model host time 29369.9 us a frame (GPU, not CPU)
draw steps: 8.0 draws a frame; us a frame: GlProgram 10.2, GlAttribs 6.5, GlTextures 5.0, GlUniforms 11.7, GlState 1.3; sum 34.8
v3d: jobs 9.00 a frame, CPU waits for a job 9.00 a frame
glDrawArrays | 8.00 | 201.6 | 207.9 | 1.00 | 0.00
glGenerateMipmap | 5.00 | 74.3 | 73.1 | 5.00 | 5.00
eglSwapBuffers | 1.00 | 15.9 | 16.6 | 2.84 | 0.00
yakogl/v3d: TFU mipmap at frame 1: 1 chains (0 queued behind a job, 0 waited), refused: ... unknown 13, ...
yakogl/v3d: tiled copies for draws at frame 1, by reason: plain 3, r32f 0, first-sample 7, ...
```

(columns: calls, wall us, cpu us, V3D CPU waits, TFU jobs, a frame.)
`glDrawArrays` 201.6 us here and 44.1 us in the clean one-process run below
are one-run noise (the CV of these host figures is 37-44 %,
`pi4-terrain-uniforms.md`). The isolated run is the state the earlier notes
measured: every `glGenerateMipmap` a TFU chain, no tiled copy after frame 1.

### (1') Terrain alone, TFU model off (`prof-tfu0`): 12.57 ms

```
frame: wall 12841.1 us, cpu 12753.6 us, in GL/EGL calls 12573.9 us; TFU model jobs 0.00 a frame
glGenerateMipmap | 5.00 | 8801.5 | 8753.5 | 7.00 | 0.00
glDrawArrays | 8.00 | 3726.5 | 3702.1 | 1.00 | 0.00
yakogl/v3d: TFU mipmap at frame 2: 0 chains ..., refused: ... unknown 13, ..., no-tfu 6, ...
yakogl/v3d: tiled copies for draws at frame 2, by reason: plain 3, r32f 0, first-sample 7, cpu-write 6, ...
yakogl/v3d: other copies at frame 2: ... image-store 8, level0-back 4, chain-back 0
```

The CPU path of all 5 chains: 8.8 ms of box filter and 3.7 ms of chain
tiling a frame on the host. This is the bound for what the CPU fallback
costs when every chain takes it.

### (2) The board's order: terrain after the other scenes, in one process

`one-process.sh build/pi4-terrain/prof-tfu1 0 23 20` (entries 0..23, 20
frames each). The cumulative lines at terrain's frame 1 and shadow's frame 1:

```
yakogl/v3d: TFU mipmap at frame 441: 1 chains (0 queued behind a job, 0 waited), refused: ... unknown 14, ...
yakogl/v3d: tiled copies for draws at frame 441, by reason: plain 11, r32f 0, first-sample 20, ...
yakogl/v3d: other copies at frame 441: upload 7, arena 0, stamp-replaced 0, stale-given-back 0, image-store 204, level0-back 0, chain-back 0
gm2: scene-end [terrain] <default>: ok frames=20 draws=161 refused=0 gl_errors=0 incomplete_draws=0
yakogl/v3d: TFU mipmap at frame 461: 58 chains (0 queued behind a job, 0 waited), refused: ... unknown 52, ...
yakogl/v3d: tiled copies for draws at frame 461, by reason: plain 11, r32f 0, first-sample 77, ...
yakogl/v3d: other copies at frame 461: upload 7, ..., image-store 280, level0-back 0, chain-back 0
```

Over terrain's 19 frames after frame 1: 57 chains (3 a frame), 38 `unknown`
refusals (2 a frame), 57 `first-sample` copies (3 a frame), 76 image stores
(4 a frame). The board's record, over
61 frames: 183 chains (3 a frame), 122 `unknown` (2 a frame), 183
`first-sample` copies (3 a frame), 245 image stores. **The host reproduces
the board.**

Which earlier scene does it (`one-process.sh ... FIRST 23 20`, counts over
terrain's frames):

| first entry | entries before terrain | chains a frame | `unknown` a frame | `first-sample` a frame |
|---|---|---:|---:|---:|
| 22 (terrain alone) | none | 5 | 0 | 0 |
| 20 (ideas) | ideas, jellyfish | 5 | 0 | 0 |
| 15 (desktop blur) | desktop x2, buffer x3, ideas, jellyfish | 3 | 2 | 3 |
| 9 (bump) | bump x3 .. jellyfish | 3 | 2 | 3 |
| 0 (the list) | all 22 | 3 | 2 | 3 |

The two desktop scenes are what changes it. A scratch build with a log line
in `rt_register` and `rt_drop` (entries 15..22, 6 frames each; not part of
this change) shows the table. The desktop scenes' slots are all dropped at
their teardown (`v3d_texture_released` → `rt_drop`, `:4706`). Terrain then
registers 9 targets: three 512x512 10-level ones, two 256x256 one-level, two
256x256 9-level and two 800x600 10-level. Run alone it registers 7, with only
one 512x512, the specular map it renders at set-up. From terrain's second
frame on, every registration evicts:

```
DBG rt_register evict slot 4 gpu 0x2ea1000 800x600 lv 10 used 152 img false for gpu 0x2886000 256x256 lv 9 now 158 keep Some(7)
DBG rt_register evict slot 6 gpu 0x1258000 256x256 lv 1 used 156 img false for gpu 0x2ea1000 800x600 lv 10 now 161 keep None
DBG rt_register evict slot 4 gpu 0x2886000 256x256 lv 9 used 158 img false for gpu 0x1258000 256x256 lv 1 now 165 keep Some(5)
```

The three keys of the cycle are a one-level 256x256 map (height or normal), a
256x256 bloom target and the 800x600 tilt-shift target. The two extra
512x512 keys are `0x2484000` and `0x2730000`; the isolated run's one 512x512
is `0x12ae000`, registered in both runs. Each is evicted before it is
rendered into again (`img false`): no image is ever untiled back, the
`level0-back` and `chain-back` counters stay 0.

**Why two uploaded textures look like render targets.** A V3D render image
is kept for a texture "rendered into at some point" (`rt_eligible`,
`src/v3d/backend.rs:1804`: `b.target_kick != 0`). `target_kick` is
`framebuffer::texture_target_use` (`src/gl/draw.rs:3952`), a per-context
table indexed by the texture's slot (`TARGET_USE`,
`src/gl/framebuffer.rs:96`, written by `set_target_use`, `:203`). The table
is moved by `orphan_moved` (`:1379`, the move at `:1387`-`1399`) and reset only when every
context is (`:3762`). `glDeleteTextures` → `free_texture`
(`src/gl/texture.rs:4958`, `:5009`-`5019`) clears the texture record but
not its `TARGET_USE` entry. glmark2's desktop scene deletes its window
textures at teardown (`scene-desktop.cpp` `RenderObject::release`). Terrain's
`glGenTextures` gets the same slots back, and two of its uploaded 512x512
textures carry a desktop render's kick from their first sample on. A scratch
build that clears the entry in `free_texture`
(`framebuffer::set_target_use(c.index, h, 0)`) registers 7 targets in the
same run and evicts none. (`DEPTH_TARGET`, `:108`, is not cleared either:
see "Not in this note".)

### (3) Terrain's own frames, thrashing and not

`framediff.py`, 40-frame run less 20-frame run, over the 20 frames between
(TFU model on):

| | thrashing: entries 15..22 | clean: entries 20..22 |
|---|---:|---:|
| GL/EGL time a frame | **4849.6 us** | **332.8 us** |
| `glGenerateMipmap` (5 calls) | 3511.9 us, 7.00 waits, 3.00 TFU jobs | 69.6 us, 5.00 waits, 5.00 TFU jobs |
| `glDrawArrays` (8 calls) | 1445.0 us, 1.00 waits, 1.00 TFU jobs | 44.1 us, 1.00 waits, 0 TFU jobs |
| `eglSwapBuffers` | 17.8 us, 1.00 waits | 15.8 us, 3.00 waits |
| jobs, CPU waits a frame | 9.00, 9.00 | 9.00, 9.00 |
| TFU jobs a frame | 4.00 | 4.95 |

The 4.5 ms the thrash adds on the host is all in the two entry points that
run the CPU loops below. The TFU job in `glDrawArrays` is the first-sample
copy of the evicted one-level 256x256 map, tiled by the TFU (one level, 64
KiB or more: `tfu_tile`, `src/v3d/backend.rs:4017`). The board's frame 10
shows the same 4 TFU jobs a frame (3 chains and 1 tiling).

## The board frame, split five ways

Board figures are the `c4af4917` record's (62 frames). "Estimated" rows use
the conversion in the next section.

| part | ms a frame | how known |
|---|---:|---|
| CPU loops over GPU memory (the thrash) | **~128 (95-140)** | estimated, below |
| CPU waits on the GPU inside GL calls | ~23 (at most 29.1 - 5.6) | the board's GPU work less what the swap waited; no board counter times waits |
| CPU waits in the swap | 5.62 | `glmark2-frame` swap mean, `end waited for in 62 of 62` |
| CPU GL-call work outside the loops | ~1 | host 0.33 ms clean, A72 assumed about 3x slower |
| logging | ~0 a frame (~1.2 averaged) | below |
| **sum** | **~158 (125-170)** | board: 161.79 |

Jobs and kicks: 9 render jobs a frame (560 over 62 on the board, 9.00 on the
host) and 4 TFU jobs a frame (3 chains, 1 tiling). GPU time: 29.1 ms at frame
10 (8 jobs, the 4 TFU jobs 1.8 ms of it).

### CPU loops over GPU memory, per frame (thrashing)

| loop | call site | bytes per frame | board estimate |
|---|---|---|---:|
| box filter of the 800x600 tilt-shift chain | `glGenerateMipmap` → `src/gl/texture.rs:4344`-`4427`, the `csize == 1` path `:4391`-`4401`; reached after `generate_mipmap` refused it as `unknown` (`src/v3d/backend.rs:2343`) | reads 2.56 M single bytes (4 per output byte), writes 640 KB | ~102 ms |
| box filter of the 256x256 bloom chain | same | reads 0.35 M bytes, writes 87 KB | ~14 ms |
| CPU tiling of the 800x600 chain on its first sample | `glDrawArrays` → `copy_image_for` (`src/v3d/backend.rs:4278`) → `tiled_copy` fill → `texture::tile` (`:4324`); `tfu_tile` refuses a chain (`layout.levels != 1`, `:4017`) | reads 2.56 MB, writes 2,780,160 B (the 10-level layout) | ~8-10 ms |
| CPU tiling of the 256x256 9-level chain | same | ~0.35 MB each way | ~1.5 ms |
| one-level 256x256 map, first sample | same, by the TFU (GPU) | 0 on the CPU | 0 |
| image stores (`image-store`, 4 a frame) | `flush_job`, `src/v3d/backend.rs:2955`-`2958`: the job's TLB stores RT 0 into the render image | 0 on the CPU (GPU stores) | 0 |
| uploads, depth or colour resolves, chain/level-0 write-backs | `upload` +0, `level0-back` and `chain-back` 0 over terrain's frames | 0 | 0 |

The image stores are GPU work. The 245 `image-store` copies of the record are
not a CPU cost.

### CPU waits on the GPU, with call sites

Every job is waited for once (9 a frame on host and board). With the thrash,
the waits on the host fall as: 7 in `glGenerateMipmap`, 1 in `glDrawArrays`,
1 in `eglSwapBuffers`. Without it: 5, 1 and 3. The call sites:

- `flush_job` → `wait_queued` (`src/v3d/backend.rs:2922`): a submit with two
  jobs outstanding waits for the older. Each `glGenerateMipmap` submits the
  job rendering into its target (`:2358`), and so does a draw on a target
  change.
- A refused chain: `settle_texture` (`src/gl/texture.rs:915`-`917`) finds
  the target's render kick pending and calls `backend::flush` and
  `backend::finish` = `v3d_finish` (`src/v3d/backend.rs:6206`): a **full
  drain** before the CPU filter. On the board the GPU then idles through the
  ~100 ms of filtering, and the frame's GPU work cannot overlap the CPU
  loops. That is why the parts add up instead of overlapping. These are the
  2 extra waits in `glGenerateMipmap`.
- A first-sample copy of a target whose render is still running:
  `copy_image_for` → `wait_in_flight` (`src/v3d/backend.rs:4298`-`4305`,
  counted in `tex_waits`).
- The swap: `v3d_finish_previous` (`:6258`) and the frame's last flush.
- `choose_arenas` (`:2994`-`3015`) when the arena set is held.
- Every wait polls on the board: `z_wait` → `finish_inflight`
  (`src/osal/zephyr_v3d.c:1510`, `:1256`) → `v3d_wait` on FRDONE (`:1281`).
  The CPU spins and the time is in the GL-call figure. A TFU job also
  finishes the job in flight first (`osal_v3d_tfu` → `finish_all`, `:1346`).

### Logging

0 in the steady state. No `fs uniform[` line (the dump budget is 0 by
default, `pi4-terrain-uniforms.md`). The back end's counter lines are printed
only when an arena high-water grows (`end_frame`,
`src/v3d/backend.rs:4886`-`4931`), so at terrain's frame 1: five lines, about
850 bytes. At 115200 baud (assumed, as in `pi4-terrain-uniforms.md`) that is
~75 ms once, ~1.2 ms a frame averaged over 62 frames. It falls in frame 1's
swap.

## Host bytes to board time

The Pi 4's CPU is a Cortex-A72 at 1.5 GHz (Raspberry Pi 4 Model B
specifications, raspberrypi.com: "Broadcom BCM2711, Quad core Cortex-A72
(ARM v8) 64-bit SoC @ 1.5GHz"). On the board every GPU allocation, including
the GL core's texture storage, the tiled copies and the render images, is
Normal non-cacheable (`src/osal/zephyr_v3d.c:51`-`56`; `z_gpu_alloc`,
`:413`-`416`). No load from it is cached or prefetched, so a loop over it
pays a bus round trip per load, and the cost is per *access*, not per byte.

No streaming bandwidth to non-cacheable memory has been measured on this
board, so the rate comes from the repository's own board records. shadow
before the depth sampling (`pi4-glmark2-cpu.md`) spent about 170 ms a frame
in GL calls (175.28 ms a frame at `33a0f264`). Almost all of it was two CPU
passes over the 1600x1200 depth texture: the resolve and the tiling. Each
pass is 1.92 M texels, one 4-byte non-cacheable load and one store each
(Mesa's per-pixel `v3d_move_pixels_general_percpp`). Less shadow's ~7 ms of
GPU work, ~162 ms / 3.84 M texel moves gives **about 40 ns per non-cacheable
load** (stores are write-combined and much cheaper). The same two passes took
11.2 ms on the host: **about 15x host-to-board** for such loops.

Two ways to convert:

- **Per load** (the model of the tables above): the box filter's reads are
  single-byte loads (`texels[box_src(j, i)] as u32`,
  `src/gl/texture.rs:4396`). 2.56 M + 0.35 M loads at ~40 ns is ~116 ms.
  The chain tiling reads level 0 in 16-byte rows through the UIF walker
  (`docs/notes/v3d-tiling-path.md`), so ~10 ms. Total ~128 ms; with 30-45 ns
  per load, 95-140 ms.
- **Host ratio**: 4.5 ms added on the host x 15 = ~68 ms. This is a floor. On
  the host a byte load and a 4-byte load cost the same, so the ratio
  measured on 4-byte loads underrates a loop of byte loads.

Either way the thrash is most or all of the unexplained ~130 ms, and nothing
else on the CPU side of the frame is above a few ms.

## Fix candidates, ranked

Gains are ms a frame on the board, against 161.8.

1. **Clear a texture's target-use entry when the texture is freed.**
   `src/gl/texture.rs:5009` `free_texture`: call
   `framebuffer::set_target_use(c.index as usize, h as usize, 0)`
   (`src/gl/framebuffer.rs:203`). Check whether `DEPTH_TARGET` (`:108`)
   needs the same (see "Not in this note"). Host test: two scenes in one
   context, the first renders into an FBO texture and deletes it, the second
   gets the name again with `glGenTextures`, uploads and samples it with a
   mipmap filter. Assert `texture_target_use` is 0 and the V3D registers no
   render image for it (`v3d_rt_images` registered count). Then a 9-target
   frame like terrain's makes 5 TFU chains (`tests/`, on the V3D test
   double). **Expected gain: ~125-130 ms** (frame ~30-35 ms, GPU-bound like
   refract: 3 jobs, 17.3 ms frame, 16.8 ms GPU). Risk: low. The stale value
   only sends a texture down the render-target copy and image paths, which
   give the same texels, so no picture or ES 2.0 answer depends on it
   (`tests/es2_report_guard.rs` unchanged). The GC7000 tiled copy and the
   GL core's `settle_texture` wait on the same value for a kick that has long
   completed, so they lose nothing. **Board value that confirms it:** the
   terrain `glmark2-frame:` line, gl from 156 ms to under ~3 ms and the
   frame near 30 ms. The `yakogl/v3d: TFU mipmap at frame N:` lines from
   terrain's frame 1 to shadow's frame 1 grow by 5 chains a terrain frame
   with `unknown` unchanged, and the `tiled copies for draws` line by
   `first-sample` +0 after terrain's frame 1. All of these lines are already
   printed. Files: `src/gl/texture.rs` (plan #310's list) and
   `src/gl/framebuffer.rs` (plan #325's): order it after them.

2. **Make the V3D's render-image table count and survive more than 8
   targets.** `src/v3d/backend.rs:1856`-`1881` `rt_register`: count
   evictions (no counter exists today) and print them on the `TFU mipmap at
   frame` line and at shutdown. Then one or both of: (a) `RT_IMAGES` 8 → 12
   or 16 (`:286`; each live image is a second copy of its target, 2.78 MB for
   an 800x600 chain, inside the 96 MiB carve-out whose high-water is 69 MB in
   refract); (b) do not register a texture whose `target_kick` predates its
   storage (`rt_eligible`, `:1804`, given a storage stamp in the bind).
   **Expected gain: the same ~125 ms for terrain if candidate 1 is not
   done; 0 after it.** It keeps an application with more than 8 sampled
   targets (Slint layers, the cluster) out of the same cliff. Risk: (a)
   memory, a refused image falls back to the copy path, as now; (b) none for
   pictures. **Board value that confirms it:** the new eviction count, 0 over
   terrain, with the candidate-1 values above. Owner: `src/v3d/**` (plan
   #308).

3. **Make the CPU fallback cheap on non-cacheable memory.** For any chain the
   TFU cannot make (no TFU, a failed job, no slot):
   (a) `src/gl/texture.rs:4344`-`4427`: read each source level once with
   wide loads into a cached scratch (or filter a texel at a time as `u32`)
   instead of four single-byte loads per output byte;
   (b) `src/v3d/backend.rs:4017`: let `tfu_tile` tile a chain level by level
   (one TFU job a level), so a first-sample chain copy is GPU work.
   **Expected gain: 0 for terrain after candidate 1; up to ~100 ms (a)
   and ~10 ms (b) a frame wherever the fallback runs.** Risk: (a) must stay
   byte-identical (`(sum + 2) / 4` per channel). The ES 2.0 answers do not
   change if it is, and a host test compares the bytes with the old filter
   over odd sizes. (b) needs a board check of the TFU's per-level output,
   with gl_test case 140's method. **Board value:** none printed today.
   Add the CPU mipmap filter's time and bytes (an OSAL clock lap around
   `texture.rs:4344`-`4427`, a counter beside `v3d_tiling_time`) and print
   `v3d_tiling_time` (`src/v3d/backend.rs:5151`, exported, read by nothing
   on the board) per scene.

4. **Do not drain the GPU before a CPU filter of a target.** The
   `settle_texture` drain (`src/gl/texture.rs:915`-`917`) is needed for a
   CPU write. The filter reads level 0 and writes levels 1.., so waiting for
   the target's render kick alone (`backend::wait_kick`) would let the
   other jobs run. **Expected gain: up to the GPU work overlapped, a few ms;
   0 after candidate 1.** Risk: ordering; it needs the
   `tests/v3d_tfu_mipmap_nowait.rs` style order test. **Board value:** the
   per-scene CPU wait count, not printed yet (below).

## Counters a board run needs

What the GM image prints today is enough to confirm candidate 1: the
`glmark2-frame:` line and the cumulative `TFU mipmap at frame N` and `tiled
copies for draws at frame N` lines at terrain's and shadow's frame 1. Not
printed, and needed for a clean per-scene split:

- a per-scene line at each scene's end with the differences of
  `v3d_copy_counters`, `v3d_mip_counters`, `v3d_rt_images`, `v3d_counters`
  (jobs and waits) and `v3d_tiling_time`. These are all exported by
  `src/v3d/backend.rs`, so it is a print in
  `driver/samples/zephyr/glmark2/src/main.cpp` beside `glmark2-steps`
  (plan #328 owns that file). Today the scene's share is read from lines
  printed only when an arena grows;
- the render-image eviction count (candidate 2, `rt_register`);
- the time the CPU spends waiting, not only the count: an OSAL clock lap
  in `wait_job` (`src/v3d/backend.rs:3051`-`3057`) summed per scene;
- the CPU mipmap filter's time and bytes (candidate 3).

## Board runs

Two GM images, run on 2026-11-08 one after the other on the same Pi 4,
each with `tools/board.sh pi4 run <image> records/pi4 glmark2-<commit>`
(the GM image of `tools/cts/cts-common.sh pi4 build`, label `GM`). The
runner was main's `tools/board.sh` at the after commit. Records are kept on
the runner host (gitignored), as every board record is.

- **Before**: `c707598d`, the first parent of `31a4cf8d` (v3d-rt-evict). On
  main's first-parent chain, `31a4cf8d` is the first commit that contains
  itself, so the commit before it is its parent. It has none of this plan's
  V3D or GL fixes.
- **After**: `17e91019`, the origin/main tip at the session. It has
  `31a4cf8d` (v3d-rt-evict, `RT_IMAGES` 16), `21a236e5` (mip-filter-wide)
  and `8d1488cd` (v3d_wait_time). Between the two commits, those three are
  the only commits that touch `src/gl/texture.rs`, `src/gl/framebuffer.rs`
  or `src/v3d/backend.rs`. **The render-into mark fix
  (`gl: clear a texture slot's render-into mark when the slot is freed`,
  plan item pi4-target-mark-clear, `9b6b99d1` on its own branch) is not on
  that tip.** `free_texture` still leaves the entry, so this pair measures
  candidate 2 without candidate 1.

Before, commit `c707598d`, `records/pi4/20261108-1522-glmark2-c707598d-dbcff325/`, terrain 161.923 ms a frame:

```
board.sh pi4 glmark2-c707598d rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:GLMARK2 resets=0 record=594b1201,fc3f00c3,6184003f,00258320 back_after=420s unsupported=0
```

After, commit `17e91019`, `records/pi4/20261108-1513-glmark2-17e91019-fc6f17b0/`, terrain 35.720 ms a frame:

```
board.sh pi4 glmark2-17e91019 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:GLMARK2 resets=0 record=594b1201,fc3f00c4,6184003f,00258320 back_after=488s unsupported=0
```

(The last eight hex digits of a records directory name are the image's
sha256, not the commit.)

| | before `c707598d` | after `17e91019` |
|---|---|---|
| boot build line | `sample: build tree=c707598d table=6c5b8a70` | `sample: build tree=17e91019-dirty table=6c5b8a70` (*) |
| **terrain** | **161.923 ms a frame, 7 FPS** | **35.720 ms a frame, 28 FPS** |
| `glmark2-frame` gl / swap / frame us | 156087 / 5619 / 161708 over 62 frames | 27374 / 8283 / 35659 over 281 frames |
| jobs a frame, end waited for | 560 over 62 (9.0), 62 of 62 | 2531 over 281 (9.0), 281 of 281 |
| `glmark2-steps` sum | 176 us | 111 us |
| `glmark2-jobs` frame 10: render sum, TFU | 8 jobs, 23.49 ms render; 4 TFU jobs (`2/911us`, `1/113us`, `1/804us`) 1.83 ms | 8 jobs, 23.49 ms render; 5 TFU jobs (`806`, `114`, `114`, `804`, `804` us) 2.64 ms |
| TFU chains a frame (terrain frame 1 to shadow frame 1) | 1 to 184: 3.0 a frame (183 over 61) | 1 to 1401: 5.0 a frame (1400 over 280) |
| `unknown` refusals a frame | 14 to 136: 2.0 | 14 to 14: 0 |
| `first-sample` copies a frame | 29 to 212: 3.0 | 29 to 29: 0 |
| `image-store` a frame | 9584 to 9828: 4.0 | 9581 to 11541: 7.0 |
| `level0-back`, `chain-back` | 0, 0 | 0, 0 |
| glmark2 score, scenes, GL errors, resets | 195, 33 ran 0 failed, 0, 0 | 196, 33 ran 0 failed, 0, 0 |
| shadow / refract | 7.342 / 17.275 ms | 7.342 / 17.247 ms |

(*) `-dirty` is the runner patch below, uncommitted in the after commit's
worktree when the image was built. `tools/board.sh` is not part of the
image.

The counter rows are differences of the cumulative `yakogl/v3d:` lines
printed at terrain's frame 1 (frame 44400 before, 44544 after) and shadow's
frame 1 (44462, 44825). The arena lines are printed only when a high-water
grows. Terrain printed nothing between those two lines in either run, so
the steady frame logs nothing (the "Logging" row above: 0 a frame).

**Not readable from either image**: the `v3d_wait_time` sums and the CPU
mipmap filter's time and bytes (`gl_mip_cpu_counters`). `8d1488cd` and
`21a236e5` export them, but nothing in the GM image prints them
(`driver/samples/zephyr/glmark2/src/main.cpp` has no caller). The CPU
filter's share is read from the `unknown` count instead: a refused chain is
the only way into the filter, and after frame 1 the count is flat.

`tools/hostharness/terrain-records/parse.py <before> <after>` was checked
against these two consoles. The verdict line, the `glmark2-frame` fields
and the highest-frame `TFU mipmap` and `tiled copies` lines are right; its
`diff` gives gl -128713 us and frame -126049 us. Two fields are wrong on a
real console. `fps`/`ms` read `absent`: on the board a back-end line is
printed between `[terrain] <default>:` and ` FPS: 28 FrameTime: 35.720 ms`,
so the scene name and the FPS are on different lines. `FPS` in parse.py
wants them on one. `commit` reads the image's sha256 prefix (`dbcff325`,
`fc6f17b0`), not the commit. The values above were read from the console.

### Measured cause and gain, per ranked candidate

- **The 130 ms.** Before: the arena lines grow by 2 `unknown`, 3
  `first-sample` and 3 chains a frame. These are the rates the host model
  predicted for a thrashing table ("(2) The board's order"). The `c4af4917`
  record had the same rates and frame (161.79 ms). After: the same lines are
  flat and the frame is 126.2 ms shorter (gl -128.7 ms). **The measured
  cause is the render-image table thrash**, the CPU box filter and CPU chain
  tiling it caused (section "CPU loops over GPU memory").
- **Candidate 2 (render-image table, `31a4cf8d`): measured gain 126.2 ms a
  frame** (161.92 to 35.72 ms, 4.5x). Every confirming value from
  `terrain-board-readout.md` was read: chains 5 a frame, `unknown` and
  `first-sample` flat after frame 1, `image-store` 7 a frame
  (`v3d-frame-cpu-model.md` 1), `level0-back` and `chain-back` 0 (so the
  board's eviction order matches the host's), 5 TFU jobs at 2.64 ms
  (`v3d-terrain-gpu.md`: 2.64 ms) and 9.0 jobs a frame. The eviction counter
  the candidate asked for does not exist, so the zero evictions are inferred
  from those counters.
- **Candidate 1 (render-into mark): not measured, not on the tip.** With 16
  slots, terrain's 9 targets fit even with the two stale entries, so its
  confirming counters are already met without it. Expected speed gain on
  terrain after candidate 2: about 0 (no `unknown` or `first-sample` left
  to remove). It remains a correctness fix: a stale kick sends an uploaded
  texture down the render-target paths. A board run of a main that has it
  should read the same counters and a terrain frame within run-to-run
  spread of 35.7 ms.
- **Candidate 3a (`21a236e5`, wide loads in the CPU filter): measured no-gain
  on terrain, because the path is not taken.** `unknown` is flat after
  frame 1, so no chain reaches the filter. Its time and bytes are not
  printed. Its gain is for frames that still fall back. Both changes are in
  the after image, so the 126.2 ms belongs to candidate 2, not 3a.
- **Candidate 3b (`tfu_tile` level by level): not built; nothing to gain
  here.** `first-sample` is flat after frame 1.
- **Candidate 4 (no full drain before a CPU filter): not on the tip; nothing
  to gain here.** No filter runs.
- **What is left (new).** The after frame is 35.7 ms against about 26.1 ms
  of GPU work at frame 10 (23.49 ms render plus 2.64 ms TFU). Predicted:
  30 to 35 ms here, 30 to 31 in `v3d-terrain-gpu.md`. The split is gl 27.4
  ms and swap 8.3 ms. It is closer to `v3d-terrain-gpu.md`'s "gl about 20,
  swap about 9" than to this note's "gl under 3 ms". The 9 waits a frame
  sit in the calls that end a job (`terrain-board-readout.md`,
  "Discrepancies"). The other ~9.6 ms is CPU time that does not overlap the
  GPU, and no counter splits it on the board. The timed draw steps are 0.11
  ms. The last job's `prep` is 2.56 ms, before and after. The next session
  needs the `v3d_wait_time` sum printed per scene (item
  `glmark2-scene-counters-design`) to tell waiting from CPU work. The
  GPU-side candidates of `v3d-terrain-gpu.md` (the blur passes sampling one
  level, the dead window clear) are now the larger part. The second and fourth
  entries of the frame-10 `glmark2-jobs` line (`render` 7930 and 9352 us)
  are 74 % of the render sum.

### How the console was captured (session-specific)

connected's serial MCP server held the console bridge's one client for this
lease, so the runner's own bridge capture got `busy` (`probe: busy (another
client holds the bridge)`, rc 3, first attempt
`records/pi4/20261108-1511-glmark2-17e91019-fc6f17b0/`). For both runs the
runner was given a local pty as `BOARD_UART` (`pi4_capture_start`'s local
device path). The console text read from the MCP server was written into
that pty while the run was going, from the firmware's image-load line to
`board: holding 20 s, then the reset`. The Linux shutdown and boot text
around the image was not passed on. All of it reached the runner inside the
420 s bound (after: last chunk 15:19:46 for a 15:13:36 tryboot; before:
15:28:23 for 15:22:14), so `end_seen`, the `yk-record:` decode and the
verdict are the runner's own. The `HH:MM:SS` stamps in `console.txt` are
when each chunk was passed on, not when the board printed the line. CRs are
dropped as the runner drops them.

The runner also ran with an uncommitted guard patch in `common_prechecks`.
The board's `who` listed one remote session, and its address was the
client address of the board's own `SSH_CONNECTION` for this connection:
our own ssh session through the lease forwarder, not one of this host's
`ip -o addr` addresses. The patch adds that address to `mine`. No other
session was on the board.

## Not in this note

- `DEPTH_TARGET` (`src/gl/framebuffer.rs:108`) has the same lifetime as
  `TARGET_USE`. A new depth texture that gets a deleted depth texture's slot
  inherits its level and resolved kick. Whether that can make a resolve
  skip or read the wrong plane was not checked. It is a correctness
  question for the fix of candidate 1.
- The AM62P's terrain (plan item am62p-breakdown).
