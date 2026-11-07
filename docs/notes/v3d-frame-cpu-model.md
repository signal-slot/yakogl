<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: a byte-count model of glmark2 terrain's CPU work on non-cacheable memory (Pi 4)

Study, no code change (#1209). For each place in `src/v3d`, `src/gl` and the
Zephyr V3D OSAL where the CPU reads or writes GPU-visible memory during a
terrain frame, this note gives the calls and bytes a frame, turns the bytes
into Pi 4 milliseconds with the bandwidth figures the tree already records,
and ranks the fix candidates by the share of the unexplained time each could
explain. The frame split itself is `pi4-slow-terrain.md`. Why the copies and
refusals happen is `v3d-terrain-copies.md`. Neither is repeated here.

Code read at `d83e25b5`. Its `src/v3d`, `src/gl`, `src/osal` and
`tools/hostharness` are the same as main `723a75c8`, the main tip when this
was written. Line numbers are that tree's.

## What is to be explained

Board record of main `c4af4917` (`pi4-glmark2-cpu.md`, "On the board: glmark2
from main `c4af4917`", lines 543-558; record
`records/pi4/20261022-0308-glmark2-c4af4917/`), checked against that note on
main `723a75c8`:

- terrain 161.79 ms a frame: GL 156.06 ms, swap 5.62 ms, 9 jobs a frame, the
  swap waits in 62 of 62 frames;
- timed draw steps 0.17 ms a frame;
- GPU work of frame 10: 29.1 ms (8 render jobs, plus 4 TFU jobs of 1.8 ms);
- per frame after frame 1: 3 TFU chains, 2 `unknown` refusals, 3
  `first-sample` tiled copies, about 4 `image-store`
  (`v3d-terrain-copies.md`, first table).

GL time outside the steps is 155.9 ms. The CPU can wait for the GPU inside
GL calls for at most 29.1 - 5.62 = 23.5 ms of it, the GPU time the swap did
not wait for. So **at least about 130 ms a frame is CPU work**, as
`pi4-glmark2-cpu.md:556-558` says. The shares below are of those 130 ms.

A cross-check on that split: 156 ms of GL time plus 5.6 ms of swap is almost
exactly 130 ms of CPU work plus the GPU's 29 ms. That fits a frame where the
CPU and the GPU take turns and do not overlap. The two `settle_texture` waits
below do exactly that.

## Host run

The glmark2 host harness (`tools/hostharness/glmark2/README.md`, "Profile"),
V3D build, `YAKOGL_V3D_TFU=1` and the Pi 4's other settings, which `run.sh`
sets from `rpi_4b.conf`. First terrain alone, as the README gives it:

```sh
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 GM2_ENTRIES='^terrain$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" "$TMPDIR/gm-prof" v3d
```

Result: 0.302 ms of GL time a frame, 5 TFU chains a frame, 9 jobs and 9
waits a frame, no tiled copy after frame 1. `run.sh` runs one process per
list entry, so terrain inherits no texture slots from earlier scenes. That
is the healthy case of `v3d-terrain-copies.md` ("Host run"), not the board.

To get the board's frame, the binary that `run.sh` built was run once more,
with the board's list order in **one process**: every default entry before
terrain at 3 frames each, then terrain at 20, with the settings of
`<out>/v3d-settings.txt` in the environment:

```sh
set -a; . "$TMPDIR/gm-prof/v3d-settings.txt"; set +a
YAKOGL_ERROR_LOG=1000000 GM2_PROFILE=1 GM2_FRAMES=100000 \
  "$TMPDIR/gm-prof/glmark2-host-v3d" --size 800x600 \
  -b build:use-vbo=false:nframes=3 -b build:use-vbo=true:nframes=3 ... \
  -b jellyfish:nframes=3 -b terrain:nframes=20
```

(`...` is the rest of `glmark2-host-v3d --gm2-list` before `terrain`, each
with `:nframes=3`.) **This reproduces the board's churn on the host.**
Every terrain frame after the first has the 2 CPU filters, 2 CPU tilings
and 1 TFU tiling that the board's 2 `unknown` and 3 `first-sample` a frame
give, and 3 slot evictions. So the counts in the table are host counts for
the board's case, not estimates. `v3d-terrain-copies.md` ("Host run") did
not have this mode. The one-process run is the host repro its design 1 asks
for.

The bytes come from counters added to a **scratch copy** of the tree outside
the repository (none of this is committed). There was one counter (calls,
bytes read, bytes written) at each call site of the table, read and reset at
every `eglSwapBuffers` by `shim.c`. Terrain's frames 2 to 20 were identical
to the byte. The counters sit at the call sites below; the patch is
described here so that it can be redone:

| counter at | counts |
|---|---|
| `submit_job`, `src/v3d/backend.rs:3253-3264` | `j.bcl.used`, `j.rcl.used`, `j.ind.used`, `j.data_used`, `j.tex_used`, `kick.tile_state_size` per job |
| `copy_image_for`'s fill, `backend.rs:4318-4327` | `gl_bytes` read and `image_bytes` written, split by TFU made or CPU tiled |
| `copy_image_for`'s arena path, `backend.rs:4384` | the same |
| `copy_image_for`'s wait, `backend.rs:4301` | calls |
| `rt_chain_back` `:2043-2051`, `rt_level0_back` `:2079-2089` | bytes per level untiled |
| `rt_register` eviction, `:1861-1866` | calls |
| `wait_in_flight`, `:3028` | calls |
| `glGenerateMipmap`'s CPU filter, `src/gl/texture.rs:4343-4429` | bytes read and written, and one-byte loads and stores |
| `settle_texture` busy, `texture.rs:916-917` | calls |
| `upload` `src/gl/buffer.rs:102-108`, `clear_storage` `:126-135`, `carry` `:1040-1049`, `texture_write_rect` `texture.rs:2154`, `spill` `backend.rs:3400`, shader code `:3972`, eager upload tiling `:4631` | bytes |

## Bandwidth figures in the tree

None of them is a direct measurement of the A72 against the carve-out's
Normal non-cacheable mapping (`src/osal/zephyr_v3d.c:51-56`, `:442`). Each is
a board figure from which a rate can be derived, or an assumption:

| # | figure | where | rate it gives | what it is |
|---|---|---|---:|---|
| R1 | 5.5 MB written "in 2.0 ms through Normal NC" (v3d_gl_cluster's copy bench, DEMO3N) | code comment, `driver/zephyr/lib/board/rpi4/rpi_mbox.c:210-213` | **0.36 ns/B** (2.75 GB/s) | `memcpy`, wide sequential stores. Its source was the cluster's 1920x720 window (5.5 MB, mapped Normal NC as well, `board_rpi4.c:256-257`): so NC to NC with wide loads too. The best case. Not in a note. |
| R2 | "re-tiled 28-41 textures, 8.5 MB, inside the draws (GL draws 72 ms against 14-17)" (DEMO3N) | `docs/online-shaderc/v3d-backend.md:3759-3764` | **6.5-6.8 ns per tiled byte** (55-58 ms / 8.5 MB) | Mesa's utile loop, storage to copy, both NC, at the time (2026-09-25). It predates the UIF walker (`8e05a17e`, `v3d-tiling-path.md`). It includes each tiling's allocation and bookkeeping, so it is an upper value for the loop alone. |
| R3 | "case 140: 2.2 ms against 145 ms for a 1920x720 image" (TFU against the CPU's kept copy) | code comment, `driver/samples/zephyr/glmark2/boards/rpi_4b.conf:27-29` | **26 ns per tiled byte** (145 ms / 5.53 MB) | A CPU tiled copy of one image. The source (NC storage or the caller's cached rows) and the loop it ran are not recorded. Not in a note. |
| R4 | shadow 175.28 -> 7.30 ms, GL 170 -> 0.97 ms, when the 1600x1200 depth resolve left the frame | `pi4-glmark2-cpu.md:289`, `:302-305` (main `dcbfb30b`) | **22 ns per byte of surface**, at most 88 ns per texel | One 7.68 MB D24S8 resolve a frame. A detile through `v3d_tex_load_box` into the NC texels, then a 4-byte NC load and a 4-byte NC store per texel (`backend.rs:4813-4825`; that this was the code at `dcbfb30b` is an assumption, since history before `381b0f09` is not in the tree). The best figure for **narrow** NC accesses. Refract's GL fell by about the same, 176 ms (`:290`). |
| R5 | "a pessimistic 100 MB/s for NC stores" | `pi4-terrain-uniforms.md:160` | 10 ns/B | Assumed, not measured. Used only as a pessimistic bound for writes. |

**Missing measurements**, which the model cannot do without:

- **M1**: the A72's cost of a one-byte load (`ldrb`) from Normal NC memory,
  as a dependent stream and as independent loads. That is the unit of the
  largest row. R4 is the nearest figure, and it has 4-byte loads.
- **M2**: the CPU tiling loop's rate on the board **since the UIF walker**
  (`8e05a17e`). R2 and R3 are older and 4x apart.
- **M3**: wide NC read bandwidth alone. R1 is a read and a write together.

## The table: a terrain frame on the board's path

Counts and bytes: the one-process host run, a frame after frame 1. Board ms:
bytes times the rate named. Share: of the 130 ms of CPU work.

| # | call site | calls a frame | bytes a frame | read / write | board ms a frame (rate) | share of 130 ms |
|---|---|---:|---:|---|---|---:|
| A | `glGenerateMipmap`'s CPU box filter, `src/gl/texture.rs:4343-4429`, reached when `backend::generate_mipmap` (`:4280`) is refused at `src/v3d/backend.rs:2343` (`unknown`) | 2 (bloom H, tilt-shift H) | read 2,908,544 as **2,908,544 one-byte loads**; write 727,136 as 727,136 one-byte stores | read + write, both NC | 19-76 at R2-R3 per byte read; 0.3 for the stores at R1. The bytes cannot tell where in that range it is: the unit is a byte **load**, not a byte (M1). At R4's 88 ns per narrow access it would be 290 ms, more than the frame, so the loads must overlap. | 15-58% |
| B | CPU tiling of a `first-sample` kept copy: `copy_image_for`'s fill, `backend.rs:4318-4327` -> `texture::tile` (`src/v3d/texture.rs:547-593`) -> `v3d_tex_store_level` | 2 (bloom H 9 levels, tilt-shift H 10 levels) | read 2,909,360; write 3,132,416 | read + write, both NC | 20-21 at R2; 81 at R3 (per tiled byte, 3,132,416 B) | 16-62% |
| C | `first-sample` copy given to the TFU, `tfu_tile` `backend.rs:4010-4053` (single level, 64 KiB or more) through the synchronous `OSAL_QUERY_V3D_TFU_RUN` | 1 (normal map 256x256) | 262,144 by the TFU, **0 by the CPU** | - | no CPU bytes; the CPU waits for the render in flight and the TFU job (`v3d-tfu-order.md`, section 1). Counted with the waits, row H. | in H |
| D | the tile state zeroed before each bin, `src/osal/zephyr_v3d.c:1433` (size `job_tile_state_bytes`, `backend.rs:3234-3239`, `:3262`) | 9 | write 182,784 (`memset`) | write, NC | 0.07 at R1; 1.8 at R5 | 0.05-1.4% |
| E | the job's control lists and records written while recording, flushed at `submit_job` `backend.rs:3247-3251`: BCL 1,014, RCL 3,069, indirect 5,357, texture states 584, data 0 | 9 | write 10,024 | write, NC | 0.004 at R1; 0.1 at R5 | under 0.1% |
| F | render image back into the storage on eviction or a CPU reader: `rt_chain_back` `backend.rs:2027-2057`, `rt_level0_back` `:2069-2102` | **0** on the host's churn | 0 | read + write, NC | 0. One 256x256 level 0 a frame, the guess of `v3d-terrain-copies.md` ("Eviction itself"), would be 262,144 B each way: 1.7-6.8 at R2-R3 | 0 (up to 5% if the board differs) |
| G | render-image slot evictions, `rt_register` `backend.rs:1861-1866` | 3 | 0 (the slots evicted hold no chain and no stale raster level 0 here) | - | 0 | 0 |
| H | CPU waits on the GPU: `settle_texture` `flush` + `finish` before each CPU filter (`texture.rs:916-917`), the TFU tiling's (C), `copy_image_for`'s wait for the sampled render (`backend.rs:4298-4306`) | 2 in `settle_texture`, 1 for C, 3 in all in `wait_in_flight`; `copy_image_for` 0 (linux.c's jobs are done at submit, so the host cannot show it) | - | - | GPU time, not CPU work: at most the 23.5 ms bound above | outside the 130 |
| I | tiled-copy canaries, `canary_set` and `canary_first_broken` around each kept copy, `backend.rs:4515-4525` (`CANARY_BYTES` 64, `:96`) | 2 | write 128, read 128 | NC | under 0.01 | 0 |
| J | buffer uploads, `clear_storage`, `carry` (`src/gl/buffer.rs:102-108`, `:126-135`, `:1040-1049`), texture uploads (`texture.rs:2154`), client-data spills (`backend.rs:3400`), shader code (`:3972`), eager upload tiling (`:4631`) | 0 | 0 | - | 0: all at set-up | 0 |
| K | `osal::cache_flush` and `cache_invd` around A, B, F and the arenas | as above | 0 | - | 0: no-ops on the NC carve-out (`zephyr_v3d.c:551-563`) | 0 |

The rows sum to **39-157 ms a frame (A + B + D + E)** against the 130 ms to
explain. A and B are the whole of it. Every other site together is at most
2 ms even at the pessimistic R5. On the host, terrain alone (no churn) has
only rows D, E and the 9 waits: **A, B, C and H's two `settle_texture` waits
exist only because of the churn** that `v3d-terrain-copies.md` explains.

Outside the GL time, and so outside the 130 ms: the window present. When
EGL does not show the window, `board_show` copies 800x600x4 = 1,920,000 B
NC to NC (`driver/zephyr/lib/board/rpi4/board_rpi4.c:317-320`, called from
`driver/samples/zephyr/glmark2/src/main.cpp:480-482`). That is about 0.7 ms
at R1, in the frame line's `show` column. The record's image is EGL double
buffered, so this copy may not run at all.

How A and B split inside 130 ms: if B runs at R2 (20.4 ms), A is the rest,
about 110 ms. That is about 38 ns per one-byte load, between R1's wide
stream and R4's narrow one. If B runs at R3 (81 ms), A is about 49 ms, 17 ns
a load. The board must tell which (below).

## Fix candidates, ranked

1. **Remove the churn**: `v3d-terrain-copies.md`, design 1 (clear the
   render-into mark in `free_texture`, `src/gl/texture.rs`), or design 2
   (`RT_IMAGES`, `backend.rs:286`). It removes rows A, B and C and the two
   `settle_texture` waits of H. That is all of the 39-157 ms, and so the
   whole model's explanation of the 130 ms. The model predicts that terrain's
   frame falls to its GPU work, about 30 ms (29.1 ms at frame 10, plus the 2
   TFU chains that come back, about 1 ms).
   - **Confirm on the board** (the arena lines, `backend.rs:4912-4930`,
     across terrain's window as `v3d-terrain-copies.md` reads them): `TFU
     mipmap ... chains` grows by 5 a frame, `unknown` and `first-sample` stop
     growing after terrain's frame 1, and `image-store` grows by 7 a frame.
     In the `glmark2-frame` line, terrain's `gl` falls from 156.06 ms to a few
     ms. Its frame ms then approaches the frame-10 GPU work. **If `gl` falls
     by 120 ms or more**, A + B were the cause, as modelled. If it falls by
     much less, something outside this table is (reject the model).
   - Host check first: the one-process run above gives 5 chains, 0 `unknown`
     and 0 `first-sample` a frame after the fix, and no row A, B or C.
2. **Make the CPU filter's loads wide, or read from cached memory**
   (row A). The filter reads texels one byte at a time (`texels[box_src(j,
   i)]`, `texture.rs:4391-4397`). Compiled for `aarch64-unknown-none` at the
   release profile's opt-level 2, a copy of that branch issues one `ldrb` per
   byte read and one `strb` per byte written: no wider loads. Reading each
   source row as 4-byte words (4x fewer NC loads), or copying the level into
   cached memory first (2.9 MB at R1, about 1 ms) and filtering there, cuts
   row A to a few ms wherever the CPU filter remains: chains the TFU
   refuses, the set-up's 13 `unknown`, and other applications. After fix 1
   this does not move terrain's frame. It is ranked here because it is the
   largest single unit cost found.
   - **Confirm**: a board measurement of A alone. Either a clock lap around
     `texture.rs:4343-4429` printed per scene, or terrain before and after
     with fix 1 not applied. A falls by the share above (15-58%, 19-76 ms).
3. **Read the CPU tiler's source through cached memory** (row B). It is the
   same idea for `texture::tile`'s source rows (2.9 MB a frame). It only
   matters while B exists; after fix 1, B is 0 in terrain.
   - **Confirm**: `v3d_tiling_time` (`backend.rs:5151`), the CPU tiling ns
     at draw (`ns[1]`, kept copies). It exists today but the glmark2 image
     does not print it. Printed beside the arena line, it is row B's board
     ms directly, and A = 130 ms - B - the small rows. **This one printed
     value settles M2 and splits A from B without any other change.**
4. **Tile state and arenas** (rows D, E): at most 2 ms together at the
   pessimistic R5. Not worth a change for terrain.
   - **Confirm**: the OSAL's per-job `prep` cycles (`last_job_cyc.prep`,
     `zephyr_v3d.c:1428-1436`, the memset and the submit's bookkeeping) in
     the frame line's GPU job times. Under 0.2 ms a job means the rows are as
     small as modelled.
5. **Row F** (chain and level 0 back on eviction): 0 on the host. If the
   board's `other copies` line shows `level0-back` or `chain-back` growing
   in terrain's window, which `v3d-terrain-copies.md` guesses at 1 a frame,
   the board's eviction order differs from the host's. Each one a frame is
   1.7-6.8 ms. Fix 1 removes it too.

## What the board session must read

In one Pi 4 glmark2 run, before any of the fixes, from main:

- the arena lines before terrain and at shadow's frame 1 (as `c4af4917`):
  the `unknown`, `first-sample`, `level0-back` and `chain-back` growth a
  frame. Expected 2, 3, 0 and 0 (rows A, B and C, row F);
- terrain's `glmark2-frame` `gl` and `glmark2-steps` sum (156.06 and 0.17 ms
  at `c4af4917`);
- if one board-side print is added (a later item, `src/v3d` or the sample):
  `v3d_tiling_time` and `v3d_tex_waits` (`backend.rs:5074`) differences over
  terrain's frames. The first is row B in ms; the second is how many of
  row H's waits are `copy_image_for`'s, which the host cannot count.

Then fix 1's before and after, as in candidate 1.

## Limits

- The counts are the host's in the board's scene order, with 3 frames for
  each earlier scene. The board runs each scene for about 10 s. That
  `unknown` and `first-sample` come out at the board's 2 and 3 a frame
  suggests the slots are the same. Which earlier scene leaves the stale marks
  was not looked for.
- A share range is as wide as the rate figures. Only R1 is a clean
  bandwidth, and it is from a code comment, not a note. R2, R3 and R4 are
  derived from frame times, and R5 is an assumption. M1 to M3 are missing.
- The `ldrb` finding is from a stand-alone copy of the filter's `csize == 1`
  branch compiled with `rustc --target aarch64-unknown-none -C
  opt-level=2`, not from the board image's LTO build.
- The model counts CPU bytes, not cycles spent on arithmetic. The filter's
  bounds checks and address arithmetic are in its ms on the board, as they
  are in R4's.
