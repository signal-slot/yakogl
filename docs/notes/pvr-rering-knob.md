<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: trying `pvr_render_wait_rering` on glmark2 terrain (candidate 1's first try)

Status: recipe, written on the host at `434d2695` (origin/main of 2026-11-07).
Nothing here was run on a board. It belongs to candidate 1 of
`docs/notes/am62p-slow-terrain.md`, "Fix candidates, ranked", 1 ("The
once-a-frame 20 ms fence").

**Short answer: "no new code, only a knob" does not hold.** The knob is a
plain C global with no Kconfig symbol, no build define, no setter and no
shell. No image sets it. The cheapest way to flip it is a one-line local
patch, below, that serves both images (GM and gl_test). It is an experiment
commit that never goes to main, so the session builds it with
`--allow-local`, as `docs/notes/pvr-mip-tail-e1.md` ("(3) The board
procedure") does for E1.

## What the knob is and where it is read

- Defined in `driver/src/pvr_render.c:331`: `bool pvr_render_wait_rering;`
  (zero-initialised: off). The comment at `:328-330` says what it is: "Re-ring
  the KCCB doorbell every 8 ms while waiting for a fence. A probe for the 'the
  last kick is never applied' invariant". Declared `extern` in
  `driver/src/pvr_render.h:243`.
- Its sibling `pvr_render_wait_repost` (`pvr_render.c:347`) is **on** by
  default, since 2026-09-24, as a mitigation (comment `:333-346`).
  `pvr_render_reposts` (`:348`) counts its firings.
- Copied into the Rust state on every entry, not once: `rr_sync_cfg()`
  (`pvr_render.c:593-596`, `RR.cfg.wait_rering = pvr_render_wait_rering;`) is
  called at init (`:640`), in the submit entry (`:746`) and in
  `pvr_render_wait` (`:782`, the "frame fence" wait). A write made at any time
  before a kick or wait is seen by the next one. The field is
  `wait_rering` in `driver/rust/src/gpu/pvr/render.rs:269` (C layout
  `driver/rust/include/yakogl_drivers.h:809`).
- Who writes it today: only the host record tools,
  `tools/pvr/km/record_c.c:134` and `tools/pvr/km/km_check.c:207`, from a
  scenario's config word (`tools/pvr/km/scenario.h:602`). No file under
  `driver/samples/`, `driver/tools/pvrlab/`, `driver/zephyr/` or `src/` names
  it (grep of `wait_rering`, 2026-11-07). The `pvr_debug_*` test-knob
  contract of `docs/notes/pvr-test-knobs.md` does not cover it: the symbol is
  not in the `pvr_debug_` namespace, so the gate's "no test knob in the
  artefact" row is not involved, and there is no `CONFIG_YAKOGL_TEST_KNOBS`
  path for it.

## What it does on each wait, in code terms

`wait_for` (`driver/rust/src/gpu/pvr/render.rs:2327`, one loop turn `i` a
millisecond up to `timeout_ms`) does, after the fence test and the stop check:

1. `fwccb_service_r` (`:2352`): serves the firmware's requests.
2. **`if r.cfg.wait_rering != 0 && (i & 0x7) == 0x7 { kccb_doorbell(k) }`**
   (`render.rs:2356-2360`): with the knob on, at `i` = 7, 15, 23 ... ms writes
   the KCCB doorbell register again. No command is posted, `last_kick` is not
   resent, nothing is logged, no counter moves.
3. `if r.cfg.wait_repost != 0 && i == 20` (`:2365-2382`): once, at 20 ms, posts
   `last_kick[..last_kick_n]` again (`kccb_post`), `r.reposts += 1`, and logs
   `render: fence N (who) not moved at 20 ms, re-posted K kick(s) (M so far)`
   for the first 8 and every 256th (`:2371-2380`). `who` is `"frame fence"` for
   the swap's wait (`pvr_render.c:783`).
4. `busy_wait_us` for the rest of the millisecond, 20 us steps with
   `pvr-fine-wait` (`:2026`), else one 1000 us step (`:2028`, `:2437-2453`).
   The re-ring is once per ms turn either way, so it does not depend on
   fine-wait.

Both calls run in **every** wait of the run (the submit paths' waits too,
labelled otherwise). The knob can only change a wait that lasts 7 ms or more:
the first re-ring is at 7 ms. A wait that ends sooner is the same bytes as
now. A wait that the firmware only delays until the *next command* is not
helped by a doorbell alone (that is what `wait_repost` was added for,
`pvr_render.c:333-346`); the knob asks whether the firmware merely missed the
notification.

## Expected effect, to compare with the record

`am62p-slow-terrain.md` says the knob "would cut the stall to about 8 ms if a
doorbell is enough" and calls the candidate "up to -19 ms (83 -> about 64)".
The two figures differ: the 64 is the re-post at once (stall to about 1 ms).
With the re-ring the wait ends at about 8 ms (the re-ring at 7 ms plus the
firmware's reaction), so the saving is about 20 - 8 = **12 ms a frame**, terrain
83.1 -> about **71 ms**. Read -19 as the upper bound, -12 as the prediction of
this knob.

## How to set it, per image

Neither image can set it without an edit. Pick one (the first is the
recommendation):

**A. One line in the library, both images (recommended).**
`driver/src/pvr_render.c:331` from `bool pvr_render_wait_rering;` to
`bool pvr_render_wait_rering = true;`. `driver/src/**` and
`driver/rust/src/gpu/pvr/**` are #325's, so this is a local experiment commit
on top of the main commit under test, never merged. One commit, one build line
for both images. Nothing else changes: the glmark2 sample and gl_test both link
this file (`pvr_render.h` is included by `driver/tools/pvrlab/gl_test.c:856`
and `driver/samples/zephyr/glmark2/src/main.cpp:147`).

**B. A Kconfig symbol per sample (what a merged knob would look like).** For
the record of the effort, not for the session:

- glmark2: a `config GLMARK2_PVR_WAIT_RERING` in
  `driver/samples/zephyr/glmark2/Kconfig` (beside `GLMARK2_PVR_ONE_KICK_IN_FLIGHT`,
  `:27`) and, in `driver/samples/zephyr/glmark2/src/main.cpp` after
  `canvas.init()` (beside the `#if defined(CONFIG_GLMARK2_PVR_ONE_KICK_IN_FLIGHT)`
  block, `:1438-1442`), `pvr_render_wait_rering = true;` with a `printk` line
  that says so. `driver/samples/zephyr/glmark2/**` is #328's.
- gl_test: a `config GL_TEST_PVR_WAIT_RERING` in
  `driver/samples/zephyr/gl_test/Kconfig` and the same assignment at the top of
  the case run in `driver/tools/pvrlab/gl_test.c` (its PowerVR-only helpers are
  at `:880-905`). `gl_test.c` is the running items' `gltest-repost-count` and
  `gltest-lod-time`.
- `tools/cts/pr-session.sh` passes only the PARTIAL_RENDER, probe and
  one-in-flight symbols (`:105`, `OVERLAY`; `:119` for the glmark2 one), so a symbol would also need a
  flag there (`tools/cts/**` is #325's).

There is no runtime route: no Zephyr shell in either image (no `CONFIG_SHELL`
under `driver/samples/zephyr/{glmark2,gl_test}` or `driver/zephyr/Kconfig`),
no debugger on the farm's board path, and the gate's knob row does not apply.

## The session

Board: `target:am62p` with `target:am62p-console`. Both arms from the **same
origin/main commit** `<main>`: the "off" arm is `<main>` itself, the "on" arm
is `<main>` plus the one-line commit of A. The board-before session's
baseline may serve as the off arm if it is the same commit (its records, not a
new run).

```text
git checkout <main>                       # a detached origin/main tip
tools/cts/pr-session.sh rering-off --knob off --cases '216' --glmark2

git checkout -b exp-rering <main>
sed -i 's/^bool pvr_render_wait_rering;$/bool pvr_render_wait_rering = true;/' driver/src/pvr_render.c
git diff --stat                           # one file, one line
git commit -am 'EXPERIMENT: pvr_render_wait_rering on (never merge)'
tools/cts/pr-session.sh rering-on  --knob off --cases '216' --glmark2 --allow-local
```

- `--knob off` is `CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=n`, main's default
  (`tools/cts/pr-session.sh:16`, `:76`). `--cases '216'` is
  `CONFIG_GL_TEST_CASES` (mipchains). `--glmark2` adds the glmark2 image with
  the same overlay, `GLMARK2_SRC` as `tools/cts/common-images.sh` (`:51-58`).
  The session writes `tools/cts/log/<date>-pr-<name>/{gltest,glmark2}.log`
  (`:95`, `:191-200`); the board-run line is
  `tools/board.sh am62p run build/pr/<name>-<sha8>/<label>/zephyr/zephyr.bin ...`.
  `--dry-run` prints the exact west and board commands without a board.
- Check the sed took before building: `grep -n 'pvr_render_wait_rering'
  driver/src/pvr_render.c` shows `= true;` on the second branch only. The
  knob prints nothing when it fires, so the record cannot show it. The
  `.config` check in the script covers PARTIAL_RENDER only, not this; the
  commit subject and `git diff` of the session branch are the proof, so keep
  both in the note.
- Run the pair back to back, off then on, and again on, off (4 runs per
  image) if the boards allow: the AM62P's terrain figure moves by about 1 ms
  between runs, and the effect looked for is 12.
- The default glmark2 list runs 33 scenes of 10 s. Terrain's lines are in it.
  For terrain alone add `-DCONFIG_GLMARK2_ARGS="-b terrain"` to the printed
  glmark2 west line and run that by hand (`tools/board.sh am62p run ...`).
- Optional, same build: `CONFIG_YAKOGL_DRV_GPU_PVR_KICK_TIMES=y` and
  `CONFIG_YAKOGL_DRV_GPU_PVR_FINE_WAIT=y` change the wait itself
  (`driver/zephyr/Kconfig:183`, `:194`; `pvr-fence-wait.md`). Leave them off in
  this pair; `pvr-fine-wait-default` is a separate item and would move the
  baseline.

## The record lines to read

From the glmark2 log (`glmark2.log`), per scene, terrain only:

| line | where it comes from | read |
|---|---|---|
| `render: fence N (frame fence) not moved at 20 ms, re-posted 1 kick(s) (M so far)` | `render.rs:2374-2380` | the count of these lines and M's growth over the scene. M is a running total over the run (all scenes before terrain included), so subtract the value at terrain's first line, or use the first-8 and every-256th rule: after 8 only the 256th line prints, so read M from the **last** line, or from `pvr_render_reposts` if printed |
| `glmark2: terrain` result and `FPS:` | glmark2's scene result line | ms a frame = 1000 / FPS; 83.1 at `6a7021ec` (`RESULTS.md` terrain row of the AM62P) |
| `glmark2-frame: [terrain]: N frames; us per frame: gl G, swap S, show H, other O, frame F` | `driver/samples/zephyr/glmark2/src/main.cpp:1208-1212` | the swap's waits are in `swap`; the 20 ms stall is expected there (`am62p-slow-terrain.md`, the call-site table) |
| `verdict=done`, GL errors, resets, HWR | `pr-session.sh:192-193` | must be `done` and 0; a doorbell must not break anything |

From the gl_test log (`gltest.log`), case 216 (`gl_test.c:20028-20200`):

| line | read |
|---|---|
| `gl_test: mipchains: 20 frames, X ms a frame` | the case's frame; 46.15 at `61ad38a1` (`pvr-mip-tail-e1.md`'s table) |
| `gl_test:   per frame: N kicks (M over 20 frames)` | unchanged: 40.00. The knob must not change a kick count |
| `gl_test:   per frame: TA busy ..., 3D busy ..., TA\|3D ... us, wall ... us` | the GPU's share; unchanged by the knob |
| `gl_test: mipchains: reposts N over 20 frames` (`gl_test.c:20196`) | N = 20 means one 20 ms fence a frame, N = 0 means none |
| `gl_test: case 216 mipchains: PASS` | PASS |

Compare off and on pair by pair. The `reposts` line needs the case-216
re-post print, merged with `pvr-gltest-kick-probes`; this tree has it
(`gl_test.c:20153`, `:20170`, `:20196`).

## What each outcome means for candidate 1

Let R be the re-post lines a terrain frame (terrain's M growth over its
frames) and D the drop in ms a frame, off minus on.

| outcome | reading | candidate 1 |
|---|---|---|
| R falls to 0 (or near it), D about 10-13 ms, swap us falls by the same, case 216 reposts 20 -> 0, ms a frame falls by about 12 | the stall ended at the re-ring at 7 ms, before the 20 ms re-post: a doorbell is enough. The firmware had the kick and missed the notification | **Doorbell enough.** Ship it as a design: re-ring every ms (or at 1, 2, 4 ms) in `wait_for`, then re-measure for the other 8 ms. A merged change is #325's (`render.rs:2356`, `pvr_render.c:331`); the AM62P gets about -12 ms, -19 with a ring at 1 ms if the kick is applied at once |
| R stays at about 1 a frame, D within +-1 ms, case 216 reposts still about 20 | a doorbell at 7 and 15 ms did not make the firmware apply the kick; only the posted command at 20 ms did | **Re-post needed.** The firmware wants a command, as `pvr_render.c:333-346` found. Try the earlier re-post: a repost at 1-2 ms (`i == 20` to a small value, one line in `render.rs:2365`) or a no-op command after the last kick of a frame; expected -19 ms, same record lines to read |
| R falls to 0 but ms a frame and swap us do not fall (D < 3 ms) | the doorbell moved the work the 20 ms re-post did, but the frame did not get shorter: the 20 ms was not the stall, or another wait of 15-20 ms takes its place | **Not the stall.** Read `glmark2-frame` swap us against the per-wait call sites (`am62p-slow-terrain.md`, the waits table) and the kick-times build; the once-a-frame 20 ms fence is not the frame's cost. Move to candidate 2 |
| R falls, D between 3 and 9 ms | part of the stall: the doorbell ended some waits (before 20 ms) but others still reached the re-post, or the firmware needed the second ring (15 ms) | **Partly.** Read the run's lines one by one: lines that remain are the waits the doorbell did not end. Count them a frame; D should be about 12 x (1 - R_on/R_off) |
| R rises, or a hang, `verdict` is not `done`, HWR | the extra doorbell races the firmware | **Rule the knob out** and stop; a new defect for the issue list, with the log |

Two checks that the arms are comparable: the `FPS:` of the scenes that do
not wait on the GPU past 7 ms (shadow 10.6 ms, `am62p-slow-terrain.md`) must
be the same in both arms within noise, and the picture split must be the same
(`tools/cts/glmark2-pictures.txt`, column `am62p`). The knob changes no
command's bytes, only when a doorbell is rung.

## What to write down afterwards

In a new `docs/notes/` record, not in this note: the two commits (`<main>`
and the experiment commit), the verdict lines, the records path, the table
above filled in. The RESULTS.md, STATUS.md and README.md glmark2 rows stay
#317's. The comment for #1199 gets the outcome row and the figures; a hang or
a regression is a new issue with the log.
