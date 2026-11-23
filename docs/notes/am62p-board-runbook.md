<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: one board session, every terrain arm and the camera tear look

Plan #329, item `am62p-board-runbook` (#1199). Written on the host, from merged
notes and code only. **Nothing here ran on a board and no code changed.** It
is the checklist for the one lease (`target:am62p`, `target:am62p-console`,
`target:am62p-camera`) that the items `am62p-board-before`,
`pvr-mip-tail-e1-board` and `am62p-board-after` share, so that one session
reads as much as possible. Every figure is cited to a note or to code, or
marked **(prediction)** with its arithmetic.

Checked against origin/main `9f5f6e76` (the fetched ref
`refs/connected/origin/remotes/origin/main`). Line numbers below are that tip's
unless a file is named otherwise. No board address, host name or UART path is
in this file: they come from the environment (`tools/board.sh` header,
`ask_farm`), never from a note.

## What is on main, and what is not

| piece | on `9f5f6e76`? | where |
|---|---|---|
| the window's dead clear gone (51 kicks a frame) | yes, `4255adf1` | `am62p-terrain-host-after.md` |
| the 20 us fence poll by default | yes, `1e67474f` | `driver/zephyr/Kconfig:183` |
| `glmark2-counters:` line (V3D and PowerVR) | yes | `driver/samples/zephyr/glmark2/src/main.cpp:1336` |
| re-post line names `site=` and `last_posted=` | yes | `driver/rust/src/gpu/pvr/render.rs:2455` |
| gl_test cases 216, 219, 220, 248 | yes | `driver/tools/pvrlab/gl_test.c:20400` (216), `:20668` area (219, `kickpair`), `:20767` (220), `:20925` (248) |
| `parse.py` reads the counters line | yes | `tools/hostharness/terrain-records/README.md` |
| the re-ring knob `pvr_render_wait_rering` | yes, a plain global, off, **no setter** | `driver/src/pvr_render.c:352`, read at `driver/rust/src/gpu/pvr/render.rs:2438` |
| E1 (tail route) | **no**: a patch inside `pvr-mip-tail-e1.md`, never merged | section (1) of that note, written against `ea371491` |
| three levels a render | **no** (`pvr-mip-three-levels`, ready) | `pvr-mip-three-levels.md` |
| up to 4 renders a KCCB command | **no** (`pvr-chain-submit-knob`, ready) | `pvr-chain-one-submit.md`, "If it is per command" |
| HWPerf timestamps | **no**: a design (`e3fe58d4`); needs code, not an arm | `pvr-hwperf.md` |
| `am62p_dss_wait_vsync` as `wait_vblank` | **decided: not wired** | `am62p-flip-latch.md`, "Decision: wait_vblank (2026-11)" |

The last row means the display needs **no image of its own**: the camera look
reads the tree as it is.

## The images, and why this order needs the fewest

Every arm builds from the **same** origin/main commit `<main>` (take the tip
when the lease starts, `git fetch origin`, note the sha8). Each arm differs
by at most one commit, so a difference is that commit's. Each image is two
builds (gl_test `GT`, glmark2 `GM`), run as two boots through `board.sh`
(`tools/cts/cts-common.sh` and `tools/cts/pr-session.sh` do that).

| image | built from | carries | arms it serves |
|---|---|---|---|
| **A** | `<main>` | GT with cases `216 219 220 248` and `-DGL_TEST_TEAR_FRAMES=6000`; GM, the default scene list | baseline, camera tear look, gl_test probes, the re-ring knob's **off** arm |
| **B** | `<main>` + the one-line re-ring commit | GT `216`; GM | re-ring knob (on) |
| **C** | `<main>` + E1's patch (and its regenerated tables) | GT `216 219`; GM | E1 tail route |
| **D** | the commit of `pvr-chain-submit-knob`, option on | GT `216 219`; GM | chain submit, **when on main** |
| **E** | the commit of `pvr-mip-three-levels`, knob on | GT `216`; GM | three levels, **when on main** |

Five images, ten boots at most. Reuse: A answers four arms (baseline, camera,
probes, rering-off) and A's case-216 and case-219 lines are the "before" of
B, C, D and E. No arm needs a second baseline image. B, C, D and E each have a
**single changed variable**; do not stack them in one image (the figures would
not separate). Case 248 sits last in A's list so its 6000 frames (about 100 s,
`am62p-flip-latch.md`, "Camera look procedure") run after the timed cases and
the camera window is the end of the run.

**Run order: A, B, C, then D and E if they are on main.** A first because the
other images compare to it and because the camera look needs only A.
B before C: B is a one-line edit with the smallest prediction (-12 ms) and
settles whether the 20 ms fence is the stall before E1's -20 kicks is read.
Run each image's GT before its GM (GT is short; it shows a hang before
glmark2's long run). Within one image, run it twice if the lease allows:
terrain's figure moves by about 1 ms between runs (`pvr-rering-knob.md`).

## Cut-off order (what to drop first if the session must end early)

Drop from the bottom of this list up; keep the top.

1. **E** (three levels): not on main, off by default, a design with the
   largest ES 2.0 risk (`pvr-mip-three-levels-es2.md`).
2. **D** (chain submit): not on main; its own c/s numbers also come from the
   `kickpair` lines of image A (case 219 as landed does not decide it,
   `pvr-chain-one-submit.md`, "Why case 219 as landed does not decide it").
3. E1's dEQP mipmap read-back (step 3 of its procedure): a second whole
   board stage, `board-stage.sh am62p deqp`.
4. **C's GM** (E1 terrain): keep C's GT (216 kicks 24.00 is the cheap proof).
5. **B's GM**: keep B's GT (case 216 `reposts 20 -> 0` is the cheap proof).
6. Image A's case 220 (log only, a V3D question).
7. A's second run of each image.
8. **Never drop**: A's GM baseline and GT `216 219` (the "before" of every
   other arm) and the camera look on A's case 248 (the display question's
   only open board check). The camera look is last in the keep list because
   it needs the least lease time (about 100 s plus shots).

## The commands

Environment as `tools/cts/common-images.sh` lists (`ZEPHYR_WS`, `WEST_VENV`,
`GLMARK2_SRC`, the firmware blob `driver/src/fw/pvr_fw_blob.h`, never
committed). The board address, boot backend and UART come from the
environment (`tools/board.sh` header), not from here. `--dry-run` prints every
command first.

### Arm 1: baseline (image A, the glmark2 half)

```text
tools/cts/board-stage.sh am62p common
python3 -I tools/hostharness/terrain-records/parse.py records/am62p/common-<sha8>
```

`board-stage.sh am62p common` is `cts-common.sh am62p build build/common/am62p-<sha8>`
then `cts-common.sh am62p run ... records/am62p/common-<sha8>`
(`tools/cts/board-stage.sh:363-364`): glmark2 `GM` and the default gl_test.
That is the **record** of the baseline (the verdict, the picture split, the
per-scene ms). The case list (216 219 220 248) is a separate gl_test image,
below.

Read, per scene:

| record line | predicted (confirms) |
|---|---|
| `verdict=` in the INDEX.txt line | `done`, 0 GL errors, 0 resets (`am62p-slow-terrain.md`) |
| terrain `FPS: N FrameTime: X ms` | **79.9 to 82.0 ms, 12.2 to 12.5 FPS** (prediction, `am62p-terrain-host-after.md`, "Board values to expect": 83.06 less the dead window clear, less the poll if the old image lacked it). The kick count is the verdict, not the ms |
| `glmark2-counters: ... [terrain] ...: frames N kicks K waits W reposts R ...` | `kicks` 51 to 52 a frame, `waits` about 51 (the frame-set waits, `pvr-frameset-wait.md` figure, a prediction), `reposts` about 1 or 0, `wait_timeouts` 0 (`glmark2-scene-counters.md`, PowerVR table) |
| `glmark2-frame: [terrain] ...: gl G, swap S, show H, other O, frame F` | gl about 78.7 to 80.8 ms and swap under 1 ms (prediction, `am62p-terrain-host-after.md`). A swap near 20 ms moves the 20 ms fence to `finish_previous` |
| `render: fence N (frame fence) not moved at 20 ms, re-posted 1 kick(s) (M so far) site=<site> last_posted=<k>` | 8 lines printed over terrain if the stall is once a frame; M of about 110 to 125 at the last line. `site=` now names the wait (`render.rs:2455`): **read it**, it settles which wait meets the 20 ms fence (`am62p-slow-terrain.md`, candidate 1) |
| shadow, refract `FrameTime` | 10.6 ms and 35 to 37 ms (`am62p-slow-terrain.md`); unchanged |
| pictures, column `am62p` of `tools/cts/glmark2-pictures.txt` | the same split as the last record (28 match, 5 animated mismatches) |

`parse.py` prints these fields (commit from the console's `sample: build
tree=` line, verdict line, records path, terrain FPS and ms, `glmark2-frame`,
`reposts_so_far`, the `glmark2-counters` keys a frame). Run its `--selftest`
once (`python3 -I tools/hostharness/terrain-records/parse.py --selftest`) on
the host before the lease: it needs no board.

### Arm 2: the gl_test probes and the tear case (image A, the gl_test half)

```text
tools/cts/common-images.sh --dry-run --cases '216 219 220 248' am62p tools/cts/common-tree.txt build/rb/A GT
# add -DEXTRA_CFLAGS=-DGL_TEST_TEAR_FRAMES=6000 to the printed west build line, then
tools/cts/common-images.sh --cases '216 219 220 248' am62p tools/cts/common-tree.txt build/rb/A GT
tools/board.sh am62p run build/rb/A/GT/zephyr/zephyr.bin records/am62p/rb-A-gt
```

`--cases` is `CONFIG_GL_TEST_CASES` (`driver/tools/pvrlab/README.md`, case 248
section). `tools/cts/pr-session.sh <name> --knob off --cases '216 219 220 248'
--glmark2` does the same with logs in `tools/cts/log/<date>-pr-<name>/`
(`{gltest,glmark2}.log`) and adds the glmark2 image with the same overlay; use
it for B to E (it takes `--allow-local` for a commit not on main). For image A
use `common-images.sh` when the tear frames are raised, since `pr-session.sh`
passes no `EXTRA_CFLAGS`.

Read, from the gl_test log (all in `gl_test.c`, case 216 at `:20400`, prints
at `:20511-20528`):

| record line | predicted (confirms) |
|---|---|
| `gl_test: mipchains: 20 frames, X ms a frame` | **46.15** at `61ad38a1` (the last record, `terrain-board-readout.md`); no record at a later main, so A supplies it |
| `gl_test:   per frame: N kicks (M over 20 frames)` (case 216) | **40.00** (4 chains of 9 less 1 plus 5 passes: `am62p-slow-terrain.md`); the dead-clear fix does not touch 216 |
| `gl_test:   per frame: TA busy ..., 3D busy ..., TA|3D ... us, wall ... us` | TA\|3D about 7.75 ms (`61ad38a1`) |
| `gl_test: mipchains: reposts R over 20 frames` (`:20528`) | **R about 20** if the 20 ms fence is once a frame (then 46.15 is 40 kicks at about 0.65 ms plus one fence); **R about 0** if every kick is about 1.15 ms (`terrain-board-readout.md`, candidate 1). Both are priors: this is the one number A decides |
| `gl_test: case 216 mipchains: PASS` | PASS (the four quadrant probes) |
| `gl_test: kickpair: two: 100 reps, X ms a rep, 2.00 kicks a rep, reposts R` and `one: ... 1.00 kicks a rep` (`:20643`) | `c = two - one`, the marginal cost of a queued render: about **1.15 ms** (`pvr-mip-fewer-kicks.md`, a prior). `reposts 0` or the run is void (`pvr-chain-one-submit.md`, "The decision") |
| `gl_test: kickpair: <arm>: last kick, mean us over N reps: submit; post->geom; post->frag; prev frag->frag; wait` (`:20655`) | post to geometry fence 120 to 140 us on an idle GPU (`pvr-fence-wait.md` via the readout); about 1.09 ms when queued. `not built` unless the image has `CONFIG_YAKOGL_DRV_GPU_PVR_KICK_TIMES` |
| case 220 `gl_test: lodtime: LINEAR: ...`, `LINEAR_MIPMAP_LINEAR: ...`, `mipmap/linear time ratio R` | log only. No AM62P record and no prediction: the case was made for the V3D blur jobs (`driver/tools/pvrlab/README.md`, case 220). Record it, do not judge it |
| case 248 `gl_test: tear: frames F, ms T, swaps S, display_busy waits n/a` and `gl_test: tear: dss vsyncs V` | F = 6000, T about 100 s; V over T about **59 a second** (`am62p-dss-readahead.md`: 58.85, 16.99 ms frame). T well under F/60 s means swaps did not wait for the display |

Not printed: the re-post count of a terrain run in the GM (use `glmark2-counters
reposts`), HWPerf timestamps (`pvr-hwperf.md` is a design).

### Arm 3: the camera tear look (image A, case 248, `target:am62p-camera`)

Same lease, same boot as the case list above. Procedure: `am62p-flip-latch.md`,
"Camera look procedure" (case 248: eight black bars on white moving 64 pixels a
frame, a 16-bit frame counter in the top 32 rows). In short:

1. `calibrate_camera` with the first bars on the panel (white is bright, which
   the detector wants).
2. 15 to 20 `take_screenshot` with `reuse_detection=true` through the run; keep
   every picture. A shorter exposure holds fewer frames; take a second set at
   the longest exposure, so a blur is not mistaken for a tear.
3. A torn frame: bars at one phase above one horizontal line and at the other
   (64 pixels away) below it, the same row for all bars; the counter says which
   two frames. Read the top row's counter against the bar phase (even frame:
   bar 0 at x = 0; odd: x = 64).
4. Light-scene look (`am62p-dss-readahead.md`, "The board check that confirms
   it", the same note's camera item): glmark2 `[build] use-vbo=true`, then
   `[texture] texture-filter=nearest`, held on the panel, 10 or more shots,
   **the bottom 10 to 13 lines first**. Needs a single-scene GM (the sample's
   `CONFIG_GLMARK2_ARGS`, `driver/samples/zephyr/glmark2/Kconfig:17`): a
   short extra GM image. If the lease is short, drop it (the case 248 look is
   the stronger one: moving bars show a tear that glmark2's dark, slow scenes
   would not).

Records (the readout's table, `terrain-board-readout.md`, "AM62P display"):

| record | predicted (confirms: no tear) |
|---|---|
| `dss stats <when> t=.. ms: vsyncs .. app_frames .. underflows .. sync_lost .. go_timeouts .. go_waits .. safety_flips ..` (`driver/zephyr/lib/board/common/sample_glue.c:221`) | `underflows 0`, `sync_lost 0`, `go_timeouts 0`, `safety_flips 0`; `go_waits` within a few percent of `app_frames - vsyncs` (last record: 134955 against 132642). Printed only by a PowerVR sample built with `CONFIG_YAKOGL_SAMPLE_SHOW_ON_DSS`; the GM and gl_test images do not print it: if absent, say so and keep `gl_test: tear: dss vsyncs V` |
| the pictures | no discontinuity in the bottom strip with the counters at 0. A discontinuity there with the counters at 0 is **"hazard"**: a new defect, "AM62P: DSS reads the old buffer after the latch, torn bottom lines in light scenes". A run with no torn picture in 15 to 20 shots excludes only frequent tears: say that in those words |
| decision it feeds | `am62p-flip-latch.md`, "Decision": `wait_vblank` stays **not wired**; the look can only add a defect, it cannot reopen the wiring |

### Arm 4: E1, the tail route (image C)

Build: `<main>` plus the diff of `pvr-mip-tail-e1.md`, section (1) (written
against `ea371491`; it may need a rebase: run `git apply --check` first, on the
host). Regenerate `src/pvr/internal.rs` and `tests/reference/pvr_internal.rs`
as that section orders (the precompile step, then `--verify`). It is an
experiment commit that never goes to main, hence `--allow-local`:

```text
git checkout -b exp-e1 <main>
# apply the diff and regenerate, as in pvr-mip-tail-e1.md (1)
git commit -am 'EXPERIMENT: E1 tail route (never merge)'
tools/cts/pr-session.sh e1-tail --knob off --cases '216 219' --glmark2 --allow-local
```

Image A's case 216 and 219 lines are the "before" (same `<main>`).
Terrain: `parse.py records/am62p/rb-A-gm <C's glmark2 record>` prints the two
and the difference table.

| record line | before (A) | after, predicted |
|---|---|---|
| `per frame: N kicks` (216) | 40.00 | **24.00** (3 chains of 9 -> 5, one of 8 -> 4, plus 5 passes) |
| `mipchains: 20 frames, X ms a frame` | about 46 | 27.8 + the 4 tail kicks' shading if a queued kick is 1.15 ms (16 fewer); 35.7 + shading if a kick is 0.65 ms plus one 20 ms fence a frame. **Confirmed** when the kicks read 24.00, PASS, and X falls by at least 16c - 2 ms (about 16 ms at c = 1.15) |
| terrain `FrameTime` | A's | A's less 20c: about **60 ms** at c = 1.15, about **70 ms** at c = 0.65; `glmark2-counters kicks` about 31 to 32 a frame (51 less 20) |
| `reposts` / re-post lines | A's | not more |
| `verdict=`, GL errors, resets, picture split | done, 0, 0, the same | **the same** (mean terrain pixel within 1) |
| kicks fall, ms do not | | the frame is not kick-bound: it is the waits (`pvr-waited-kick.md`, `HWRT_MEMBERS` = 4), as path B showed |

Optional (cut order 3): `tools/cts/board-stage.sh am62p deqp` on both commits;
expected the same pass and fail per `functional.texture.mipmap.2d.*` case.

### Arm 5: the re-ring knob (image B)

Build: `pvr-rering-knob.md` recipe A, one line in `driver/src/pvr_render.c`
(`bool pvr_render_wait_rering;` is at `:352` on this tip; the note's `:331` is
older):

```text
git checkout -b exp-rering <main>
sed -i 's/^bool pvr_render_wait_rering;$/bool pvr_render_wait_rering = true;/' driver/src/pvr_render.c
git diff --stat                 # one file, one line
git commit -am 'EXPERIMENT: pvr_render_wait_rering on (never merge)'
tools/cts/pr-session.sh rering-on --knob off --cases '216' --glmark2 --allow-local
```

The knob prints nothing when it fires: the commit subject and `git diff` are
the proof, so keep both in the record. The "off" arm is image A (its record,
not a new run).

| record line | A (off) | B (on), predicted |
|---|---|---|
| `render: ... not moved at 20 ms, re-posted ... (M so far) site=...` | about 8 lines | R falls toward 0 if a doorbell is enough |
| `glmark2-counters ... reposts` a frame | about 1 | falls toward 0 |
| terrain `FrameTime` | A's | **about 71 ms**, i.e. about -12 ms (re-ring at 7 ms replaces the 20 ms wait: 20 - 8). The upper bound is -19 ms (the re-post at once). `pvr-rering-knob.md` |
| `gl_test: mipchains: reposts R over 20 frames` | about 20 or 0 | 20 -> 0, and X ms falls by about 12 ms from A's if A's R was 20 (arithmetic) |
| kicks | 40.00 / 51 | **unchanged** (the knob changes no command) |

Reading table: `pvr-rering-knob.md`, "What each outcome means for candidate 1"
(doorbell enough; re-post needed; not the stall; partly; hang). A hang,
`verdict` other than `done`, or an HWR rules the knob out and is a new defect
with the log.

### Arm 6: chain submit, up to 4 renders a KCCB command (image D, when on main)

**When on main** (`pvr-chain-submit-knob`). Build `<main>` at that commit with
its Backend option on (the option's name is the item's; read it from its
commit). Not the experiment of `pvr-chain-one-submit.md` (never merged: its
hold-a-render arms); the product step 1 of that note.

Command: `tools/cts/pr-session.sh chain4 --knob off --cases '216 219' --glmark2`
(no `--allow-local` once on main).

| record line | A (off) | D, predicted |
|---|---|---|
| `kicks a frame` (216, `glmark2-counters kicks`) | 40.00 / 51 | **the same** (renders are unchanged; commands fall, `pvr-chain-one-submit.md`: 13 commands a frame instead of 43) |
| terrain `FrameTime` | A's | **-30 s** with s the per-command cost: at s = 0.9 ms about -27 ms; a gain only if the decision of that note reads "per command" (`s >= 0.5 ms and s >= 0.6c`, from A's `kickpair` lines at 400x300) |
| `kickpair` `post->geom` | about 1.09 ms queued | much sooner on the 2nd to 4th render of a command (the take-up is per command) |
| `verdict=`, pictures, errors | | **the same** (no ES 2.0 answer or picture changes) |

If A's `(four - one) / 3 < 0.3 ms`, the per-kick price is not in the submit
and this arm is worth nothing: drop it.

### Arm 7: three levels a render (image E, when on main)

**When on main** (`pvr-mip-three-levels`, knob off by default; `Backend`
option, name from its commit). Command: `tools/cts/pr-session.sh three --knob off --cases '216' --glmark2`.

| record line | A (off) | E, predicted (`pvr-mip-three-levels.md`, (f)) |
|---|---|---|
| `per frame: N kicks` (216) | 40.00 | **21.00** |
| `mipchains: 20 frames, X ms a frame` | about 46.15 | 46.15 - 19c: about **24 ms** at c = 1.15; about 34 ms at the other reading. Confirmed when the fall is at least 19c - 2 ms |
| terrain `FrameTime`, `kicks` | A's, 51 | A's - 23c: about **57 ms**; kicks about 29 a frame (51 less 23) at c = 1.15 (prediction; see `pvr-mip-three-levels-es2.md` for the picture change this knob would bring) |
| refract `FrameTime` | 35 ms | about 28 ms (-6c) |
| pictures, `verdict=` | the same | **the same**, within tolerance 8 (`pvr-mip-three-levels-es2.md`: the knob is the one that may move bytes; read that note's table before calling a mismatch) |
| kicks fall, ms do not | | not kick-bound (`am62p-pvr-mipmap-kicks.md`) |

## After the session

Per arm: `docs/notes/` record with `<main>` and each experiment commit, the
`verdict=` line, the records path (`records/am62p/...` or
`tools/cts/log/<date>-pr-<name>/`), the table above filled in. Do not touch
`tools/cts/RESULTS.md`, `STATUS.md`, `README.md` (the glmark2 rows are #317's
one-commit table). A figure that is a defect is a new issue; the outcome rows
go into the #1199 comment. The `am62p-board-before` item writes the baseline
and camera result; `pvr-mip-tail-e1-board` the E1 and re-ring rows;
`am62p-board-after` the final comment.

## What this note does not settle

- The exact option names of arms 6 and 7: they are not on main.
- Whether `common-images.sh` accepts `EXTRA_CFLAGS` for the tear frames
  directly: `am62p-flip-latch.md` says to append it to the printed west line.
  Run `--dry-run` first.
- The HWPerf split of the 0.9 ms (`pvr-hwperf.md`) needs code first.
