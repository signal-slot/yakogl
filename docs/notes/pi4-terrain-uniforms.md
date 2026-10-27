<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 terrain: the 7.8 ms a frame in "uniforms"

`pi4-glmark2-cpu.md` ("On the board: glmark2 from main `dcbfb30b`") has
terrain at 178.9 ms a frame on the Pi 4 and a `glmark2-steps` line with
7.8 ms a frame in uniforms, while the host with `YAKOGL_V3D_TFU=1` shows
0.25 ms of GL-call time for the whole scene. The 5 TFU CPU waits a frame are
another cost (item v3d-tfu-mipmap-nowait). This note is about the 7.8 ms.

**Status of the measurements.** The session that wrote this note was refused
every run of `tools/hostharness/glmark2/run.sh` (the command in (b), issued
four ways: plain, under `env`, under `bash`, `--help`). The per-frame call
counts in (b) are therefore **counted from glmark2's source at the commit the
image pins** (`22c527cb`), not read from a `gm2: gl-time` line, and the host
ms per call is not measured. The command that fills both in is in (b). The
ranking below rests on the counts, which the source fixes, not on host
milliseconds.

## Result in one paragraph

Terrain draws 8 times a frame and sets 2 uniforms a frame. Nothing in the
timed "uniforms" lap of a draw can cost 1 ms (7.8 ms / 8 draws) of CPU work:
the lap copies a few hundred bytes. The one thing in the lap that can is a
**console print**: `dump_small_fs_block` (`src/gl/draw.rs:1939`) writes four
`fs uniform[i] = ...` lines through the OSAL log for each of the first 24
qualifying draws of a run, and the Pi 4 run of `dcbfb30b` shows exactly those
lines for terrain ("25 groups of four `fs uniform[0..3]` lines",
`pi4-glmark2-cpu.md`). Over terrain's ~56 frames, 96 lines of ~55 bytes on a
115200-baud console is ~8 ms a frame, which is the observed figure.

## (a) What the board's "uniforms" step measures

Not the `glUniform*` calls. `glmark2-steps` is the sample's print of
`osal_draw_stats` (`driver/samples/zephyr/glmark2/src/main.cpp:1148`,
`print_steps`): the total, over the scene, of each step the library times
*inside a draw*, divided by the scene's frames and printed in microseconds
(`main.cpp:1170`-`1173`). The baseline `steps0` is read in `log_scene_info`
(`main.cpp:1029`), which glmark2's `MainLoop::step` calls **after** the
scene's `setup()` (`main-loop.cpp:84`-`98`): the setup's draws are not in
the figure, every draw from frame 1 on is. The steps are `draw_steps.rs`
`Step`; "uniforms" is `Step::GlUniforms`, the lap that ends at
`src/gl/draw.rs:3486` in `build_common` and starts at the `GlProgram` lap
(`draw.rs:3381`). It is wall time on the OSAL clock (`zephyr_timing.c`, the
54 MHz counter) of this section of each `glDrawArrays` / `glDrawElements`,
summed over the frame's draws:

1. `fill_sampler_layer_uniforms` (`draw.rs:2736`) and
   `fill_depth_range_uniforms` (`draw.rs:2815`): a scan of the program's
   uniform table, both stages. The first skips every row that is not a
   sampler3D or sampler2DArray, so for terrain it does no work.
2. `fill_uniform_blocks` (`program.rs:663`): returns at once for ES 2.0.
3. The blend-colour copy (`draw.rs:3395`-`3404`).
4. The upload: `staging_reserve` (`draw.rs:3430`-`3437`, only when
   `pg.uniforms.cpu` is null), `put` of the VS and FS blocks
   (`draw.rs:3444`, `3459`, `3462`), and **`dump_small_fs_block`**
   (`draw.rs:3466`).

The step is wall time: a print, an interrupt or another thread inside the
lap is in it. A `glUniform*` call (`program.rs:2490` `set_uniform`: table
lookup, `store_dwords` into a heap array) is not in any step; it is in the
frame's "gl" figure, outside the steps.

## (b) Terrain's uniform calls per frame

glmark2's terrain (`src/scene-terrain.cpp`, `src/scene-terrain/*.cpp`) per
frame, from the source:

| what | per frame |
|---|---:|
| draws (`Mesh::render_vbo`, one `glDrawArrays` each): height map, normal map, terrain, bloom H, bloom V, bloom overlay, tilt-shift H, tilt-shift V | 8 |
| `glUniform*` calls: `update()` sets `uvOffset` on the height-map program and `uOffset` on the terrain program (two `glUniform2fv`) | 2 |
| `glUseProgram` (`Program::start`) | 8 |
| `glGenerateMipmap` (`update_mipmap`, the TFU waits of the other item) | 5 |

Every other uniform (matrices, fog, light, sampler units, `uvOffset`,
`uvScale`, `opacity`) is set once in `setup()` and in the renderers'
constructors. So terrain makes no `glUniform*` calls in its draw loop beyond
two a frame; the 7.8 ms is not made of calls. The 8 draws each run the lap
above, over blocks that are small: the blur, luminance and noise programs'
fragment blocks are 4 dwords (the "four `fs uniform` lines"), the terrain
program's vertex block a few tens of dwords.

To replace the counts by a measurement and get the host ms per call:

```
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 GM2_ENTRIES='^terrain$' YAKOGL_V3D_TFU=1 \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-prof v3d
```

`build/glmark2-prof/profile-v3d.md`, rows `glUniform2fv` and `glDrawArrays`:
calls over the 19 counted frames give calls a frame (expect 2 and 8), the
wall ms a frame over calls a frame the host ms per call (expect
microseconds). The run's log in `build/glmark2-prof/logs/v3d/` shows whether
the host also prints the `fs uniform[` lines in terrain's first frames; if it
does not, the print is board-only, since its condition (`fs_usc_dw > 900`)
reads the compiled shader size.

## (c) What can be slow on the board and not on the host

Each item is in the lap (`draw.rs:3381`-`3486`) or called by it.

1. **`dump_small_fs_block`, a console print inside the lap**
   (`draw.rs:1939`-`1956`, called at `draw.rs:3466`; budget
   `FS_DUMP_BUDGET` = 24, `draw.rs:1959`, per run). For an FS block of 8
   dwords or less in a shader of more than 900 USC dwords it prints one
   `glog!` line per dword (`draw.rs:1954`). `glog!` goes to the OSAL log
   (`osal/mod.rs:546`), which on Zephyr is the console UART: a synchronous
   byte-by-byte write. The host writes the same lines to stderr (a memory
   pipe), so it does not pay for them. Terrain draws 4 blur or noise passes
   a frame whose FS block is 4 dwords, so the 24-block budget is spent in the
   first ~6 frames, inside the counted window (the baseline is taken after
   `setup()`, see (a)). The budget is per run, so a whole-list run spends it
   on the first scene that qualifies, which the Pi 4 record shows to be
   terrain; refract's steps sum (0.09 ms) shows nothing like it.
2. **The upload `put` writes to non-cacheable memory** (`draw.rs:834`-`850`,
   called at `draw.rs:3444`, `3459`, `3462`). `z_gpu_alloc`
   (`osal/zephyr_v3d.c:393`) maps GPU memory Normal non-cacheable;
   `z_cache_flush` (`zephyr_v3d.c:533`) does nothing for it. Per draw it is
   a `copy_nonoverlapping` of the VS block and the FS block, capped at
   `MAX_UNIFORM_DWORDS` = 1536 (`gl/context.rs:83`). **Dead work on the
   V3D:** the V3D back end never reads `vs_uniform_gpu` / `fs_uniform_gpu`
   (`grep uniform_gpu src/v3d`: no hit; only `pvr/backend.rs:6241`, `6271`
   and the test double `testutil.rs:630`); it reads the host arrays
   `d.vs_uniform_cpu` / `d.fs_uniform_cpu` (`v3d/backend.rs:3181`-`3182`) and
   copies them into its own arena (`stage`, `v3d/backend.rs:2752`).
3. **A wait for an in-flight job in `staging_reserve`**
   (`draw.rs:873`-`882`, `kick_pending`). Reached only when `pg.uniforms.cpu`
   is null (`draw.rs:3430`). The V3D allocation has a CPU view, so the
   uniform storage does not stall behind the TFU or the render job.
4. **Wall time that is not the draw's**: the V3D completion interrupt, the
   display thread and the console thread preempt the glmark2 thread, and the
   open step takes it. The host has one thread and completes every job at
   submit.
5. **The table scans** `fill_stage_layer_uniforms` (`draw.rs:2765`) and
   `fill_stage_depth_range` (`draw.rs:2832`): pure CPU on cached memory,
   microseconds.
6. **The timer** (`zephyr_timing.c:34`-`52`, ~7 laps a draw): under a
   microsecond each.
7. **A recompile or relink**: none in the lap; a program's code is uploaded
   in the `GlProgram` lap (`draw.rs:1875`-`1890`).

## (d) Ranking against 7.8 ms

The scene is 8 draws a frame, so 7.8 ms is ~1 ms a draw, ~1.5 million cycles
of the 1.5 GHz A72 per draw.

1. **The console print (item 1): fits.** 24 blocks x 4 lines = 96 lines of
   ~55 bytes (`  fs uniform[0] = 0x3f800000  int 1065353216  float 1`) is
   ~5 KB. On a 115200-baud console (11.5 KB/s; assumed, not read from the
   board's device tree) that is ~0.45 s. Terrain at 178.9 ms a frame runs a
   10 s scene in ~56 frames: 0.45 s / 56 = ~8 ms a frame, against 7.8. It is
   the only item of the right order, explains why it is terrain and not
   another scene, and why the host does not show it.
2. **Wall time taken by interrupts and threads (item 4)**: fits any figure
   and names nothing; a 1 ms loss on each of 8 draws is not what the
   interrupts of a 6 fps scene look like.
3. **The NC upload (item 2): does not fit.** Terrain's blocks are tens of
   dwords. Even at the 1536-dword cap, 8 draws x 2 blocks x 6 KB = 98 KB a
   frame, which at a pessimistic 100 MB/s for NC stores is 1 ms, not 7.8.
   It is real dead work, and the second fix below removes it, but it is not
   this.
4. **Scans, timer, `staging_reserve` wait, relink (items 5, 6, 3, 7)**: far
   too small, or not taken on the V3D.

**The board observation that decides.** Count the `fs uniform[` lines in the
UART log between terrain's `scene-begin` and its first ~6 frames (or add the
number the budget used, `FS_DUMP_BUDGET - fsdump_left`, to the frame-1 arena
line, `v3d/backend.rs:4202`). Then:

- 96 lines in terrain's window: item 1. Prediction to check: with the print
  off, terrain's uniforms figure falls to ~0.1 ms.
- no such lines in the window and the step still 7.8 ms: item 4; separate
  it with Zephyr's thread runtime stats over the step.
- ns spent in `put` alone (an OSAL clock read around `draw.rs:3444`-`3462`)
  close to the step: item 2.

## Proposed fix

For the cause: the dump is a bring-up aid for a hang on a uniform-bounded
loop and has no business in a timed lap of a benchmark run.

- `src/gl/draw.rs:1939`-`1959`: take the print out of the lap, either behind
  a build or environment switch that the glmark2 images leave off (budget 0
  by default), or move `dump_small_fs_block` after
  `clock.lap(Step::GlUniforms)` (`draw.rs:3486`). The second only moves the
  time into another step; the first removes it.
- A host test in `tests/` that, with the switch off, no `fs uniform[` line is
  logged for a 4-dword FS block of a >900-dword shader, and with it on four
  are (grep `tests/` for how the OSAL log is captured).

Independent dead work, to do after the TFU item merges (it owns `src/v3d`):
stop the V3D from paying the NC upload (item 2).

- `src/backend.rs`: a capability on the backend table, "reads uniform blocks
  from the host arrays only", set by the V3D back end. When adding the field,
  grep `tests/` for every literal of the table
  (`grep -rl 'YakoglBackend {' tests src`), as `CLAUDE.md` says.
- `src/gl/draw.rs:3430`-`3475`: with it set, skip `staging_reserve` and the
  `put` calls; keep `vs_uniform_dwords`, `fs_uniform_dwords` and the
  `*_uniform_cpu` pointers.
- `src/v3d/backend.rs`: set the field.

Neither changes a rendered pixel.

## Done: the FS dump is off by default

`dump_small_fs_block` (`src/gl/draw.rs`) now runs only when
`yakogl_fs_dump_budget` is non-zero; it defaults to 0, so a draw pays one
relaxed load and prints nothing. Bring-up sets the budget (the old fixed
value was 24 blocks) by writing the exported global, as for
`yakogl_trace_log_first`; no OSAL reads a `YAKOGL_FS_DUMP` variable into it
yet (as `YAKOGL_ERROR_LOG` is in `src/osal/linux.c`). Each dumped block takes
one off it.
`tests/gl_fs_dump_switch.rs` pins both sides: no `fs uniform[` line with the
switch unset for a 4-dword block of a >900-dword shader, the lines with it
set. The prediction (terrain's uniforms figure falls to ~0.1 ms on the
Pi 4) is for the next board session. Commit: `git log -- tests/gl_fs_dump_switch.rs`.

## Done: the V3D skips the NC uniform upload

`YakoglBackend::uniforms_host_only` (`src/backend.rs`), set by the V3D table
(`src/v3d/backend.rs`): with it set, `draw::draw` skips `staging_reserve` and
the `put` calls for the VS and FS blocks and still sets `vs_uniform_dwords`,
`fs_uniform_dwords`, the `*_uniform_gpu` addresses and the `*_uniform_cpu`
pointers, which is all `v3d_job_draw` reads. The program's staging buffer is
still allocated once with the program (`upload_prog_gpu`); no draw writes a
byte into it. Every other back end leaves the field false and uploads as
before. `tests/v3d_uniform_host_only.rs` pins both sides on the real V3D back
end (no staged byte with the capability, the same words staged without it);
`tests/v3d_uniform_host.rs` pins the values each draw reads.

Host before/after for terrain on the V3D build (the command of section (b):
`GM2_FRAMES=20`, `YAKOGL_V3D_TFU=1`, entry `terrain`; three runs a side, the
field false then true, `compare.py --before ... --after ...`; time in GL and
EGL calls, ms per frame, minimum of the repeats):

| | upload (before) | host arrays only (after) | delta |
|---|---|---|---|
| terrain, all GL calls | 0.156 | 0.144 | -0.011 (within the noise, CV 37-44%) |
| `glDrawArrays` | 0.098 | 0.091 | -0.007 |

The difference is inside the run-to-run noise: the host's cacheable memory
makes the removed `put` cost well under a microsecond a draw. The pixel
output is the same (crc16 7282 in both runs' `glmark2-picture` line). The
board figure (uncached memory) is the next Pi 4 session's.
