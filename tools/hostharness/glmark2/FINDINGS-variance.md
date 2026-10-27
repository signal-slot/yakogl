<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# How much `GM2_PROFILE` ms a frame varies from run to run

Plan items compare the host harness's `GM2_PROFILE=1` ms a frame before and
after a change ([README.md](README.md), "Profile"). This file measures how
stable that number is, so a before/after table knows how many frames and
repeats it needs. It claims no speed-up.

## Method

* Commit measured: main `a2f612cb3`, glmark2 at `22c527cb0556f3a1ac4445aaa52cc532760928d5`.
* Host: AMD Ryzen 9 5900HX, 8 cores, 16 threads, Linux. **Not idle**: the
  host is shared with other build jobs. Its load average was 5 to 9 when the
  runs began and about 24 to 27 (16 threads) at the end. The numbers below
  are therefore a noisy-host upper bound on the spread, not an idle-host
  figure (see "What this does not show").
* Scenes: `terrain`, `shadow`, `refract` and two cheap ones,
  `build:use-vbo=true` and `texture:texture-filter=linear`. GPUs: v3d and
  vivante, 5 repeats each at `GM2_FRAMES` 3, 10 and 20; then once more on v3d
  with `YAKOGL_V3D_TFU=1`, 5 repeats at `GM2_FRAMES=10`. `run.sh` already sets
  the TFU on for v3d (README, "The V3D's settings"), so the `v3d` rows and the
  `v3d+tfu1` rows are the same configuration run in two batches: a measure of
  batch-to-batch drift as well.
* Command, per repeat (one out directory reused; its `profile-<gpu>.md`
  copied away after each run):

```sh
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=<3|10|20> \
GM2_ENTRIES='^(terrain|shadow|refract|build:use-vbo=true|texture:texture-filter=linear)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2var/w v3d vivante
YAKOGL_V3D_TFU=1 GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=10 GM2_ENTRIES=<same> \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2var/w v3d
```

* The figure is the entry's "wall ms a frame" over its counted frames, which
  are frames 2 to `GM2_FRAMES`: 2, 9 and 19 frames. Wall time as
  `profile-<gpu>.md` prints it (3 decimals; the cheap scenes are around
  0.1 ms, so their CV includes about 1% of rounding).
* Columns: mean, min, max and CV (sample standard deviation over mean) of
  the 5 wall figures; the CV of the thread-CPU-time figure; and the *quiet
  runs*, those whose wall time is within 5% (+0.003 ms) of their CPU time,
  with their CV. A run whose wall time is well above its CPU time was
  descheduled by the host, not slower in the library.

## Table (ms a frame, 5 repeats each)

| GPU | scene | GM2_FRAMES (counted) | mean | min | max | CV % | CPU-time CV % | quiet runs | quiet CV % |
|---|---|---|---|---|---|---|---|---|---|
| v3d | `terrain` | 3 (2) | 3.096 | 1.363 | 7.065 | 74.9 | 51.3 | 4/5 | 36.9 |
| v3d | `terrain` | 10 (9) | 2.824 | 1.747 | 5.266 | 50.4 | 15.4 | 2/5 | 9.5 |
| v3d | `terrain` | 20 (19) | 3.442 | 1.901 | 7.263 | 67.0 | 10.1 | 3/5 | 3.9 |
| v3d | `shadow` | 3 (2) | 0.418 | 0.349 | 0.527 | 18.7 | 18.2 | 5/5 | 18.7 |
| v3d | `shadow` | 10 (9) | 0.516 | 0.293 | 1.158 | 70.7 | 29.5 | 4/5 | 22.2 |
| v3d | `shadow` | 20 (19) | 0.565 | 0.291 | 0.974 | 60.8 | 17.1 | 3/5 | 11.7 |
| v3d | `refract` | 3 (2) | 3.204 | 1.387 | 4.706 | 48.2 | 48.1 | 5/5 | 48.2 |
| v3d | `refract` | 10 (9) | 1.265 | 0.932 | 1.638 | 22.8 | 21.0 | 4/5 | 19.7 |
| v3d | `refract` | 20 (19) | 1.133 | 0.840 | 1.871 | 37.4 | 23.6 | 4/5 | 11.9 |
| v3d | `build:use-vbo=true` | 3 (2) | 2.083 | 0.148 | 9.731 | 205.2 | 102.8 | 4/5 | 14.1 |
| v3d | `build:use-vbo=true` | 10 (9) | 0.115 | 0.100 | 0.135 | 13.3 | 13.5 | 5/5 | 13.3 |
| v3d | `build:use-vbo=true` | 20 (19) | 0.116 | 0.091 | 0.132 | 13.4 | 13.0 | 5/5 | 13.4 |
| v3d | `texture:texture-filter=linear` | 3 (2) | 0.239 | 0.172 | 0.384 | 35.5 | 35.7 | 5/5 | 35.5 |
| v3d | `texture:texture-filter=linear` | 10 (9) | 0.164 | 0.132 | 0.192 | 17.4 | 17.9 | 5/5 | 17.4 |
| v3d | `texture:texture-filter=linear` | 20 (19) | 0.229 | 0.134 | 0.550 | 78.7 | 9.8 | 4/5 | 9.3 |
| v3d+tfu1 | `terrain` | 10 (9) | 1.492 | 1.389 | 1.615 | 7.5 | 6.3 | 4/5 | 7.2 |
| v3d+tfu1 | `shadow` | 10 (9) | 0.306 | 0.286 | 0.336 | 7.1 | 7.0 | 5/5 | 7.1 |
| v3d+tfu1 | `refract` | 10 (9) | 1.116 | 0.730 | 2.147 | 53.3 | 34.6 | 4/5 | 19.9 |
| v3d+tfu1 | `build:use-vbo=true` | 10 (9) | 0.116 | 0.097 | 0.145 | 19.7 | 19.4 | 5/5 | 19.7 |
| v3d+tfu1 | `texture:texture-filter=linear` | 10 (9) | 0.161 | 0.133 | 0.193 | 16.4 | 16.7 | 5/5 | 16.4 |
| vivante | `terrain` | 3 (2) | 14.345 | 10.275 | 23.914 | 39.5 | 26.4 | 4/5 | 18.2 |
| vivante | `terrain` | 10 (9) | 14.548 | 12.164 | 18.266 | 19.2 | 15.7 | 4/5 | 15.8 |
| vivante | `terrain` | 20 (19) | 22.978 | 11.348 | 40.892 | 53.1 | 25.1 | 3/5 | 19.8 |
| vivante | `shadow` | 3 (2) | 4.317 | 2.359 | 6.531 | 36.4 | 36.4 | 5/5 | 36.4 |
| vivante | `shadow` | 10 (9) | 6.071 | 4.733 | 7.005 | 14.9 | 14.5 | 5/5 | 14.9 |
| vivante | `shadow` | 20 (19) | 9.552 | 3.614 | 18.570 | 77.9 | 72.3 | 3/5 | 11.3 |
| vivante | `refract` | 3 (2) | 16.706 | 12.398 | 23.683 | 27.0 | 27.1 | 5/5 | 27.0 |
| vivante | `refract` | 10 (9) | 22.354 | 16.108 | 30.227 | 26.6 | 26.4 | 5/5 | 26.6 |
| vivante | `refract` | 20 (19) | 33.269 | 17.586 | 84.160 | 86.5 | 32.6 | 3/5 | 3.9 |
| vivante | `build:use-vbo=true` | 3 (2) | 2.969 | 0.101 | 14.372 | 214.7 | 57.3 | 4/5 | 18.8 |
| vivante | `build:use-vbo=true` | 10 (9) | 0.092 | 0.078 | 0.108 | 13.5 | 13.6 | 5/5 | 13.5 |
| vivante | `build:use-vbo=true` | 20 (19) | 0.093 | 0.076 | 0.108 | 12.6 | 12.5 | 5/5 | 12.6 |
| vivante | `texture:texture-filter=linear` | 3 (2) | 0.177 | 0.134 | 0.278 | 32.9 | 22.4 | 4/5 | 11.3 |
| vivante | `texture:texture-filter=linear` | 10 (9) | 0.123 | 0.100 | 0.140 | 13.3 | 13.5 | 5/5 | 13.4 |
| vivante | `texture:texture-filter=linear` | 20 (19) | 0.204 | 0.099 | 0.352 | 63.9 | 17.2 | 3/5 | 8.8 |

## What the numbers say

* The spread is large. At `GM2_FRAMES=10` the CV is 13% to 27% for every
  scene except the batch that happened to run quietly (`v3d+tfu1` terrain and
  shadow, 7%). A 5% change is well inside the noise of a 5-repeat mean.
* Slow outliers dominate the mean. Where a run's wall time is far above its
  CPU time (vivante refract at 20 frames: 84.2 ms wall, 34.9 ms CPU) the
  host descheduled the process. The mean and the CV are pulled up by such
  runs, the minimum is not. The minimum of the five is the stable end:
  v3d terrain's three minima are 1.36, 1.75 and 1.90 ms for 2, 9 and 19
  counted frames, against means of 3.1, 2.8 and 3.4 ms.
* `GM2_FRAMES=3` counts 2 frames. The first-use costs that land after the
  first swap show up as 10-fold outliers (`build` 9.7 ms and 14.4 ms a frame
  in one repeat each, against 0.1 ms) and the CV there is 15% to 215%. It is
  not usable for a comparison.
* More frames do not by themselves cut the spread here: from 10 to 20 frames
  the CV moves both ways. Every extra frame only lengthens the stretch in
  which the host can interfere, so the gain from more frames is limited by
  the load, not by the frame count.
* Wall and CPU time agree except in the descheduled runs, so the profile's
  wall column is fine once those are recognised: a run whose wall time is
  more than about 5% above its CPU time is a descheduled run.
* Even the quiet runs vary by 4% to 37% (median about 13%), so some of the
  spread is not preemption: CPU frequency, SMT siblings and shared caches
  under the other jobs' load. That part this host cannot separate from the
  library's own time.

## Recommendation

* **Report the minimum**, with the CPU-time figure beside it, not the mean.
  The mean is moved by whichever repeat the host disturbed; the minimum
  tracks the library's own time.
* **Frame count: `GM2_FRAMES=20`** (19 counted) for terrain, shadow and
  refract and **never 3**. The minima at 10 and 20 frames agree within
  about 15% (v3d terrain 1.75 and 1.90 ms, vivante terrain 12.2 and 11.3,
  vivante refract 16.1 and 17.6), which this data cannot rank, so 20 is
  chosen for averaging over more frames, not because it measured better;
  `GM2_FRAMES=10` is enough for the cheap scenes.
* **Repeat count: at least 10 per side**, run alternately (before, after,
  before, after, ...) so that load drift hits both sides alike, with the min
  of each side compared. Five was not enough on this host: with a CV of
  10% to 25%, resolving a 5% change in the *mean* would take about
  `2 * (1.96 + 0.84)^2 * (CV / 5%)^2` = 63 repeats a side at CV 10% and 250
  at CV 20% (95% confidence, 80% power), which is not practical. The
  minimum needs far fewer, but how many this data cannot say: the five
  repeats here give one minimum a cell, not a spread of minima.
* **State the noise floor in the table**: print the CV of the "before" repeats
  beside each before/after pair and call a change resolved only if it is
  larger than that spread. With the figures above, call nothing under about
  15% resolved when the run was made on a loaded host. Discard (or mark) a
  repeat whose wall time is above its CPU time by more than 5%.
* **Run on an idle host** (load average under 1; check `uptime` before and
  after) and say so in the table. The best case seen here, 7% CV in the
  quiet `v3d+tfu1` batch for terrain and shadow, suggests that on an idle
  host with 10 repeats a 5% change in the minimum is within reach, but that
  is a guess this file does not show.

## What this does not show

* The host was shared and loaded the whole time, so these are not idle-host
  figures. A repeat of this matrix on an idle host would give the floor
  directly; it is the one measurement this file lacks.
* 5 repeats a cell is too few to measure the spread of a minimum, and the
  CV of five samples is itself uncertain (about +-35% of its value).
* Only wall and CPU time of the whole frame were taken, not a per-entry-point
  figure; an entry point's own ms (the profile's ranked table) is noisier
  than the frame it is part of.
