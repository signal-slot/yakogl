<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 terrain: the first board record with each job's bin and prep

One board run of glmark2 terrain on the Pi 4 (#1209), from the first image
whose `glmark2-jobs` line prints each job's `bin` and `prep` (`3becc843`).
It reads the values that `pi4-terrain-cpu-left.md` ("Fix candidates, ranked",
0 to 3) and `v3d-pending-start.md` (e) name, and compares them with the last
record, `e07d066a` (`pi4-slow-terrain.md`, "Third run"). No code changed.

## The run

- Commit: `a4b9c551` (origin/main as of 2026-11-22; `git ls-remote origin`
  failed here with "Host key verification failed", so
  `refs/connected/origin/heads/main` was used). `3becc843` is an ancestor.
  Image: the GM image of `tools/cts/board-stage.sh pi4 common`, build line
  `YAKOGL_SAMPLE_TABLE=6c5b8a70 YAKOGL_SAMPLE_TREE=a4b9c551`, tree not dirty,
  sha256 `afffb41dfb550692f9b592bb3c838efa2cb21c626ce1120412f33b75176f2b5e`.
- Run with `tools/board.sh pi4 run <image> records/pi4 glmark2-a4b9c551`.
- Verdict line:

```
board.sh pi4 glmark2-a4b9c551 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:GLMARK2 resets=0 record=594b1201,fc3f00c3,6184003f,00258320 back_after=430s unsupported=0
```

- Records path: `records/pi4/20261122-2039-glmark2-a4b9c551-afffb41d/`
  (gitignored, on the machine that ran it). `records/pi4/20261122-2008-...`
  and `20261122-2029-...` are two earlier tries of the same image that were
  cut off (the task's background time limit killed the runner at scene 25 of
  33, and then the leftover staged image made the next try refuse); they
  carry no verdict and are not used. `parse.py` read this record
  (`tools/hostharness/terrain-records/parse.py`).

## Terrain

**34.722 ms a frame, 29 FPS** (`glmark2-frame` frame 34687 us over 289
frames: gl 26414, swap 8270, other 3). 8.003 jobs a frame, every end waited
for (289 of 289). `waits` 8.003 a frame. **`wait.ns` 28.349 ms a frame**
(`wait.untimed` 0). **Frame less `wait.ns`: 34.687 - 28.349 = 6.34 ms.**
Other counters a frame: `copy.image-store` 6.979, `mip.queued` 4.986,
`rt.direct` 7.976, `rt.chains` 4.986, `tfu.jobs` 0.007, `tile.ns` 47.35 ms
over the run (0.16 ms a frame), `tiled.n` 0, `mip.refused.unknown` 0.014.

## Frame 10, job by job (`glmark2-jobs`, us)

```
glmark2-jobs: [terrain] <default>: frame 10: 8 job(s): total 8093 render 4436~ idle 14 bin 42 prep 3609; total 7948 render 7931 idle 80 bin 9 prep 1; total 8060 render 191 idle 1 bin 6 prep 7856; total 13843 render 9374 idle 0 tfu 1/805us bin 3529 prep 127; total 14259 render 460 idle 0 tfu 1/113us bin 7 prep 13672; total 953 render 287 idle 0 tfu 1/113us bin 7 prep 539; total 2965 render 1754 idle 0 tfu 1/795us bin 36 prep 374; total 6199 render 2801 idle 0 tfu 1/806us bin 36 prep 2549
```

| # | total | render | `~` | idle | tfu | bin | prep |
|---|---|---|---|---|---|---|---|
| 1 | 8093 | 4436 | `~` | 14 | | 42 | 3609 |
| 2 | 7948 | 7931 | | 80 | | 9 | 1 |
| 3 | 8060 | 191 | | 1 | | 6 | 7856 |
| 4 | 13843 | 9374 | | 0 | 1/805 | 3529 | 127 |
| 5 | 14259 | 460 | | 0 | 1/113 | 7 | 13672 |
| 6 | 953 | 287 | | 0 | 1/113 | 7 | 539 |
| 7 | 2965 | 1754 | | 0 | 1/795 | 36 | 374 |
| 8 | 6199 | 2801 | | 0 | 1/806 | 36 | 2549 |
| sum | | **27234** | 1 of 8 | **95** | 5 jobs, 2632 | **3672** | 28727 |

`total` is submit to completion, so it holds the queue time behind the
previous job (`prep` is the same stamp, from the submit to the bin start,
`pi4-terrain-cpu-left.md` answer 2): the `prep` column is mostly queueing and
is not CPU time. Job 4 (the one that bins the 393,216-vertex terrain) has the
only large `bin`, 3529 of the 3672 us. Jobs 2 and 4, the 800x600 blurs, render
in 7.9 and 9.4 ms. The render sum, 27.23 ms, is the later figure of
`pi4-slow-terrain.md` (27.26 ms), not the 23.49 of the older images.

## Readout, candidate by candidate

GPU work in the frame: render 27.23 + TFU 2.63 = 29.87 ms, against 34.69
ms: **4.8 ms** of the frame is not the GPU's printed work.

- **Candidate 0, `wait.ns`: refuted as predicted, branch undecided.**
  Predicted 31 to 34 ms; read 28.35 ms. It is not under 28 ms (the branch
  that said the budget was wrong), so the "CPU work is 1.7 to 4.7 ms" claim
  is not confirmed either: **6.34 ms** a frame is outside `wait_job`. Some of
  that is GPU time the wait cannot see (the 4.8 ms above is the part not in
  `render` and TFU: bin, cache clean, start-up and idle between jobs). `gl`
  less `wait.ns` cannot be formed: `wait.ns` (28.35) is larger than `gl`
  (26.41) because swap (8.27 ms) holds waits too, so the CPU time of the GL
  calls is not split out by this record.
- **Candidate 1, overlap the next bin with the render: bin is real, the gain
  is small, refuted as a 1 to 4 ms item.** The sum of `bin` is 3.67 ms and
  job 4's is 3.53 ms (above the 1 ms mark of the note), so bin exists.
  But overlap hides a bin only behind the render of the job before it, and
  the one large bin follows job 3, which renders for 191 us. The bound is
  the sum over the pairs of min(bin of N+1, render of N): 9 + 6 + 191 + 7 +
  7 + 36 + 36 = **about 0.29 ms**. (Job 1's 4436 us is an upper bound, it is
  `~`; it only feeds the pair 1 to 2, bin 9 us.) The 3.5 ms bin is serial
  with whatever job precedes it unless that job is longer, i.e. unless the
  order of the jobs changes.
- **Candidate 2, start a parked job without the CPU in a wait (and
  `v3d-pending-start.md` (e)): undecided, leaning to a small gain.** The
  frame-time value that confirms it (a fall of 0.4 to 1.6 ms) needs the
  change; this record has only the "before" half. What it does show:
  only **1 of 8** jobs is `~` (job 1), so the OSAL saw 7 of 8 ends in time;
  (e) puts the lag inside the previous job's render, flagged `~`, and the
  four short jobs it expected (jobs 3, 5, 6 and 7 here: 191, 460, 287 and
  1754 us) are not flagged. The `idle` sum is 95 us, which (e) says is a
  control and not the gain. The jobs' `prep` is queue time and cannot split
  the lag from it. So the exposure is at most job 1's lag, which the line
  cannot size.
- **Candidate 3, skip the tile-state `memset`: refuted.** The only job that
  started with the GPU idle at its submit (`idle` 80 us, the only idle over
  14 us) is job 2, with **`prep` 1 us**: well under the 0.1 ms that the
  note says closes the question. Caveat: the line does not give that job's
  tile count. The other `prep` values are queue time behind a running job.
- **Candidate 4, shorten the CPU's recording:** not decided here (no
  `gl` less `wait.ns`), its expected gain is under 0.3 ms.
- **`v3d-pending-start.md` (e) values:** frame 34.687 ms and `wait.ns`
  28.349 ms are the "before" values; frame less `wait.ns` is 6.34 ms (the
  note expected a rise of at most 2 ms after the hook); `waits` 8.003 (must
  not change); `idle` sum 95 us (control); `~` jobs 1; render sum 27.234 ms.

## Against `e07d066a`

| | `e07d066a` | `a4b9c551` |
|---|---|---|
| terrain FrameTime | 34.776 ms (29 FPS) | 34.722 ms (29 FPS) |
| gl / swap / frame us | 26436 / 8278 / 34717 | 26414 / 8270 / 34687 |
| `wait.ns` a frame | 28.38 ms | 28.349 ms |
| frame less `wait.ns` | 6.34 ms | 6.34 ms |
| jobs, `waits` a frame | 8.003, 8.003 | 8.003, 8.003 |
| frame-10 render sum | 27.26 ms | 27.234 ms |
| frame-10 TFU jobs | 5, 2.64 ms | 5, 2.632 ms |
| `copy.image-store` a frame | 6.979 | 6.979 |

Everything is within the 0.1 ms run-to-run spread: nothing between the two
commits moved terrain on the Pi 4 (the commits in between touch other
boards and the shader work). One run, no repeat.

## What the record says

The frame is GPU-bound: 28.35 of 34.69 ms is the CPU inside `wait_job`, and
the jobs' render and TFU time (29.87 ms) is the largest figure. Of the four
CPU-side candidates this record refutes 3 as sized (0 as predicted, 1 and 3)
and leaves 2 undecided; none of them is more than about 0.3 ms by what is
printed. The 3.53 ms `bin` of job 4 is the one new large item and no
overlap hides it. What remains is the GPU cost of the jobs (jobs 2 and 4 at
7.9 and 9.4 ms): `v3d-terrain-gpu.md`.
