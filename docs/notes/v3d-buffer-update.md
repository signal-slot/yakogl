<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D host: glmark2's buffer scenes cost what the other back ends cost, attributed

`gl-buffer-update.md` ("Per-call breakdown", fix 6) listed the V3D build's
buffer-update calls at 0.29 to 0.78 ms a frame against 0.05 to 0.13 ms on the
PowerVR and the GC7000, and suspected the host OSAL's allocator (bitmap search,
the memset of the arena), page faults, or `v3d_kick_pending` flushing the open
job. This note measures all of them. Milliseconds are host CPU time a frame,
never fps; the host is not the board.

**Result.** The 5 to 10 times does not reproduce. At the commit measured the
V3D's minimum over 11 runs is 1.06 to 1.2 times the PowerVR's for the three
`buffer` scenes. The earlier V3D figures are single readings of a host whose
reading of one scene varies by up to 18 times between runs on both back ends
(table 1). What is real is smaller: the update call copies, and on subdata
also memsets, the whole 288 KB buffer four times a frame. None of the
suspects is the cause: the allocator's search is 3% of subdata, there are no
page faults per frame, and the buffer path causes no job flush, no kick and
no CPU wait.

## Commit, commands, method

* Measured: main `41e951696810` (`hostharness: compare.py ...`), a clean tree
  except the temporary, uncommitted timing laps below. glmark2
  `22c527cb0556f3a1ac4445aaa52cc532760928d5`. Host: x86-64 Linux, shared with
  other build jobs (the spread below is that sharing).
* `perf` is not installed on this host, so `GM2_PERF=1` could not run
  (`run.sh` stops: "GM2_PERF=1 needs perf"). `GM2_PROFILE=1` plus laps took
  its place. The command, run 11 times (a build directory each, or the same):

  ```sh
  GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=60 GM2_ENTRIES='^buffer:' \
    tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-buf v3d powervr
  ```

  (`GLMARK2_SRC` was the checkout of the commit above in the dependency
  cache.) Five of the 11 runs had the laps compiled in but off, six had them
  on.
* The laps (a few `rdtsc` pairs around: `l_gpu_alloc` split into the
  allocator's search and its memset, `l_gpu_free`, `detach_from_pending_kick`'s
  alloc / copy / free, `upload`, `map_range`'s `carry`, `retire`,
  `unmap_buffer`'s flush, `v3d_kick_pending`, and the body of
  `glBufferSubData` and `glMapBufferOES`) are in `src/gl/buffer.rs` and
  `src/osal/linux.c` only in the working tree and were reverted: they are not
  part of this change. A first version with `clock_gettime` laps cost 0.9 us
  a lap and inflated the 40 calls a frame several times over; its readings
  are not used.
* Each reading is the minimum over the runs, per FINDINGS-variance.md. The
  counted frames are frames 2 to 60 of a 60-frame process; the laps cover all
  60 (the first frame's allocations are 4 map allocations or the setup, noted
  where it matters), divided by 60.

## 1. The update call, V3D against PowerVR (`glBufferSubData`, `glMapBufferOES`)

ms a frame, the profile's "wall ms a frame" of the update call; min of 11
runs, and the worst run for the spread.

| scene | V3D min | PowerVR min | V3D / PowerVR | V3D worst run | PowerVR worst run |
|---|---:|---:|---:|---:|---:|
| subdata, no interleave (40 calls) | 0.125 | 0.114 | 1.10 | 2.209 | 1.305 |
| map, no interleave (4 calls) | 0.035 | 0.033 | 1.06 | 0.562 | 1.088 |
| map, interleave (1 call) | 0.030 | 0.025 | 1.20 | 1.045 | 0.144 |

The spread of one reading (worst over best) is 11 to 33 times on either back
end: in the same run that read the V3D's subdata at 2.209 the PowerVR's read
1.305. Read singly, either back end can look 10 times the other. The old
note's V3D row (0.780, 0.335, 0.289) and PowerVR row (0.080, 0.049, 0.057)
came from different runs of different moments, so a ratio of them is a ratio
of two draws from this spread. The medians of the 11 runs are 0.29 (V3D
subdata) and 0.17 (PowerVR).

All 11 readings of the update call, V3D subdata: 0.125, 0.128, 0.159, 0.194,
0.228, 0.293, 0.332, 0.335, 0.392, 0.406, 2.209; PowerVR subdata: 0.114, 0.132,
0.148, 0.152, 0.160, 0.165, 0.200, 0.213, 0.242, 0.265, 1.305. (Four more runs
had the first version's `clock_gettime` laps on; they read 0.47 to 0.68 on
both back ends and are not used.)

## 2. Per step, V3D against PowerVR (the quietest run, which held every step's minimum)

ms a frame. Subdata: 4 buffers of 288 000 bytes, 40 `glBufferSubData` calls.
Each frame's first write to a buffer finds the open kick still reading it
(`last_use` newer than the retired kick), so it copies on write.

| step (file:line at the commit) | V3D | PowerVR | share of V3D body |
|---|---:|---:|---:|
| `gpu_alloc` of 288 000 bytes, 4 a frame (`detach_from_pending_kick`, `src/gl/buffer.rs:182`) | 0.053 | 0.038 | 48% |
| - of it the allocator's search: `v3d_granules_place` + mark (`src/osal/linux.c:749-758`); PowerVR `linux_heap_alloc` + `aligned_alloc` (`:254`, `:260`) | 0.005 | 0.004 | 5% |
| - of it the memset of the new block (`src/osal/linux.c:762`; PowerVR `:265`) | 0.048 | 0.035 | 43% |
| copy of the whole buffer into it: `cache_invd`, `copy_nonoverlapping` of all bytes, `cache_flush` (`buffer.rs:191-195`), 4 a frame | 0.041 | 0.037 | 37% |
| the 36 range writes that follow: `upload` (`buffer.rs:102-106`), a copy and a flush of each range | 0.018 | 0.017 | 16% |
| `gpu_free` of the old block (`buffer.rs:222`, deferred; `linux.c:773`) | 0.0004 | 0.001 | 0% |
| `kick_pending` (`src/v3d/backend.rs:5705`), 40 a frame | 0.0004 | 0.0004 | 0% |
| **named steps, total (body of `glBufferSubData`)** | **0.110** | **0.092** | 100% |
| the profile's wrapper (`shim.c`: two clock reads a call, 40 calls) | 0.049 | 0.056 | |
| **the profile's figure for the update call, same run** | **0.159** | **0.148** | |

The named steps are 69% of the profile's figure and the wrapper the other
31%; the wrapper is a measurement cost, not the library's, and it checks:
`glBindBuffer` (9 calls, 0.3 us of work each) reads 1.4 us a call in the same
profile, so 40 calls cost about 0.045. In this run the steps add to 0.113 and
the body measured whole is 0.110, so the steps are 100% of the body.
(Table 1's minima are other runs, where the wrapper is the same.)

Map, no interleave (`glMapBufferOES`, 4 a frame) and map, interleave (1):

| step | V3D map | PowerVR map | V3D interleave | PowerVR interleave |
|---|---:|---:|---:|---:|
| `carry` of the whole buffer into the retired storage (`buffer.rs:1046-1047`, `carry` at `:920`) | 0.043 | 0.045 | 0.031 | 0.032 |
| `retire`, `kick_pending`, the unmap flush (a no-op on the host, `:161`) | 0.001 | 0.001 | 0.001 | 0.001 |
| the profile's wrapper (4 or 1 call) | 0.004 | 0.004 | 0.001 | 0.001 |
| profile figure, same run | 0.048 | 0.049 | 0.030 | 0.030 |

The copy is 90% of the map call and the same on both back ends. The map path
takes `take_retired` (`:1017`), so after the first frame it makes no
allocation and no memset (laps: 4 allocations in 60 frames, the first
frame's).

## 3. Counts a frame (subdata unless stated)

| count | per frame | source |
|---|---:|---|
| `gpu_alloc` calls caused by the buffer path | 4 (map: 0 after frame 1) | laps: 240 in 60 frames |
| bytes memset by them | 1 152 000 (map: 0) | 4 x 288 000 |
| bytes copied whole by copy on write | 1 152 000 (all three scenes) | laps |
| bytes copied again by `upload` / the application | about 0.66 MB (subdata) | 36 range writes |
| distinct arena blocks used for the 288 000-byte allocations | 8 in 244 allocations | laps: two sets of 4, reused, so the memset hits warm cache |
| job flushes, kicks caused by the buffer path | 0 | `gm2: v3d-frames counted=59 kicks=59`: the one kick a frame is `eglSwapBuffers`'s |
| CPU waits caused by the buffer path | 0 | `gm2: v3d-waits`: only `eglSwapBuffers` (58), `eglMakeCurrent` (1), `glFinish` (1); none in a buffer entry point |
| page faults a frame | 0 | `ru_minflt` 7351 at 20 frames, 7355 at 60: all in setup |
| allocator search | 1.3 us a call (min 0.13 us) | 261 calls, 0.335 ms |

## 4. The attributed cause

1. **Whole-buffer copy on write, every frame, four times**
   (`detach_from_pending_kick`, `src/gl/buffer.rs:175-226`; `map_range` with
   OES, `:1046-1047`): 0.041 ms of a V3D subdata body of 0.110 and 0.043 of a
   map call of 0.048. It is the library's, it is the same on every back end,
   and it exists on the Pi 4 (below).
2. **The host OSAL's memset of every new block** (`src/osal/linux.c:762` for
   the V3D, `:265` for the others): 0.048 ms of 0.110, 43% of subdata, none of
   map. It is a property of the harness; the PowerVR build has the same
   memset (0.035). **It is not why the V3D differed**: the V3D's and the
   PowerVR's costs match.
3. **The 40 per-range writes** (`upload`, `buffer.rs:102-106`): 0.018 ms.
4. Not causes: the allocator's bitmap search (0.005 ms, 5% of subdata, 1.3 us a
   call), page faults (none a frame), `v3d_kick_pending` (0.4 us a frame; the
   function is five comparisons), a job flush, a CPU wait (none). The 96 MiB
   memset is a one-off at init (not in the counted frames).

Where the earlier V3D numbers came from: single readings of a loaded host
(table 1: the same code reads 0.125 to 2.209). A reading is meaningful only
as the minimum of repeats (FINDINGS-variance.md); the `4b057224` tables, which
this note did not rerun, state one reading each.

## 5. Does the Pi 4 board have each cause? (`src/osal/zephyr_v3d.c`)

| cause | on the Pi 4's Zephyr OSAL? |
|---|---|
| whole-buffer copy on write | **Yes**, and dearer: the library code is the same, but the carve-out is mapped Normal non-cacheable (`zephyr_v3d.c:221`), so the read of the old 288 KB (`cache_invd` is a no-op on it, `:539-543`) is an uncached read, and the write is write-combined. Not measurable on the host. The board's cost is expected to be a larger share of its subdata and map calls than the host's 37% and 90% |
| memset of a new block | **No.** `z_gpu_alloc` (`zephyr_v3d.c:393-428`) hands out the granules and does not clear them; the GL core's `clear_storage` is not on the subdata or map path. The host's 0.048 ms (43% of subdata) is harness-only, so host subdata over-reports the board's allocation cost |
| allocator search | Yes, the same `v3d_granules_place` (`:407`); it also polls the GPU's completion counter when frees are pending (`poll_and_drain`, `:405`), a register read per allocation and per free (`:449`) that the host does not have |
| per-range `upload` copies and flushes | Yes; `cache_flush` is a no-op on the NC mapping (`:533-537`), so only the uncached writes remain |
| `v3d_kick_pending`, job flush, CPU wait | No flush or wait in the buffer path, host or board (the same library code); the frame's one wait is at the swap (`v3d-swap-wait-study`) |
| page faults | None on the board (a fixed carve-out, no paging) |

## 6. Ranked fixes

None is implemented here. Gains are on the host numbers above and say what
they remove from the V3D's 0.110 (subdata) and 0.048 (map) ms.

| # | fix | files | gain |
|--:|---|---|---|
| 1 | **Copy only the dirty ranges.** Reuse a retired storage of the same buffer and copy to it only the ranges written since it was retired (in `detach_from_pending_kick` and `map_range`), instead of all 288 000 bytes four times. This is the item `gl-buffer-retired-reuse` (running; `src/gl/buffer.rs`, `docs/notes/gl-buffer-update.md` fix 1) | `src/gl/buffer.rs` | host: removes the allocation, the memset (0.053) and about half the copy (0.02): subdata 0.110 -> about 0.04; map 0.043 -> about 0.02. Board: also half the uncached 288 KB reads a buffer a frame, expected larger than the host's |
| 2 | **Subdata takes `take_retired` as map does**, before any dirty-range work: the V3D subdata is 0.110 and the map 0.055 for the same bytes only by the alloc and memset (0.053). Part of fix 1 | `src/gl/buffer.rs:175-226` | host: 0.053 of 0.110. Board: the allocator's register poll only, not the memset: smaller |
| 3 | **The harness: do not memset in `l_gpu_alloc` when the caller overwrites it all** (or time the harness with it off) so host subdata is not 0.048 ms above the board. A flag in the OSAL ops, or a harness-only environment variable | `src/osal/linux.c:762`, `:265` | changes the baseline only: host subdata 0.110 -> about 0.062 with fix 1 absent |
| 4 | Flush on unmap only the bytes the application wrote (needs fix 1's dirty list) | `src/gl/buffer.rs:155-168` | none on the host; board: half of a 288 KB flush a buffer, but the flush is a no-op on the NC mapping, so none there either |
| 5 | Merge the 40 `glBufferSubData` calls' flushes | `src/gl/buffer.rs:102-106`, `:463-` | under 0.01 ms, and zero on the board (no flush to merge) |
| - | Not a fix: the allocator (`v3d_granules_place`), `v3d_kick_pending`, a page-fault cure | | 0.005 ms / 0 / 0 |

## What to post

Nothing new is board-relevant that fix 1 of `gl-buffer-update.md` does not
already hold. For the issue that holds `gl-buffer-retired-reuse`: "Host
re-measure at 41e95169: the V3D's buffer scenes cost 1.06 to 1.2 times the
PowerVR's (minimum of 11 runs, 0.125 / 0.035 / 0.030 ms), not 5 to 10 times;
the old figures were single readings of a loaded host. The cost of a
`glBufferSubData` frame is memset 43%, whole-buffer copy 37%, range writes
16%; the memset is host-only. docs/notes/v3d-buffer-update.md."
