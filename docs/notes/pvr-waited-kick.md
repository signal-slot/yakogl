<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: why four waited-for kicks take about 15 ms

Written 2026-11-07 from the code alone (no board run). It follows one
waited-for prefix chain of path B (`docs/notes/am62p-pvr-mipmap-kicks.md`,
"Measured") from the back end to the render service, says what moves each
fence and where the CPU spins, puts the 15 ms against the queued price of a
kick, and proposes a fix with its host test and the board value that would
confirm it. Line numbers are at main `4370107f`. Every figure is quoted
from the record named beside it; what the records do not show is marked as
a hypothesis.

## The figures

From `am62p-pvr-mipmap-kicks.md` ("Measured") and the commit messages of
`ce3354ac` and `efa0170f`:

| What | Value | Record |
|---|---:|---|
| gl_test case 216, every level on the GPU (main `61ad38a1` + kick count) | 46.15 ms a frame, 40 kicks | records/am62p/mipchains-42/, 2026-11-01 |
| the same with path B (prefix and CPU tail) | 82.95 ms a frame, 15 kicks | same |
| GPU busy (TA\|3D) a frame, all-GPU / path B | 7.75 / 7.00 ms | same |
| one prefix `glGenerateMipmap` in the back end (pass + 3 level kicks + the wait for the last), mean of 80 calls | 15.39 ms | measurement build, 2026-10-29 |
| the CPU tail of one prefix call | 7.87 ms | same |
| submit after `ce3354ac` | 5 us a kick | `ce3354ac` message |
| kicks completing apart, every wait | about 1.11 ms, about 1 ms | `ce3354ac` message (terrain profile) |
| post to own fragment fence, light scenes, pipelined | about 3.85 ms; geometry fence about 1.09 ms after the post | `pvr-fence-wait.md` "What it says" |

So a queued kick costs 46.15 / 40 = 1.15 ms of frame, of which the GPU is
busy 7.75 / 40 = 0.19 ms; the rest is the firmware's take-up of a posted
kick (post to geometry done about 1 ms, `ce3354ac`). Four queued kicks
would be 4.6 ms. The waited four cost 15.39 ms: **about 10.8 ms a prefix
call that the queued price does not explain.**

One inconsistency in the records: 4 calls a frame x (15.39 + 7.87) ms =
93 ms, more than the 83.10 ms frame of the same day's untimed image. The
timing build's own frame time is not in the record, so the per-call
figures are the timing build's and may include its timing overhead. The
account below uses only their sizes relative to each other.

## The path, one prefix chain (read only)

1. **The back end records the chain.** `mip_chain_gpu`
   (`src/pvr/backend.rs:6241`) records levels 1..=`last`
   (`mip_gpu_last`, `:6234`, `last` = 3 for 800x600 with `mip_cpu_tail`) with
   `record_mip_level` (`:6268-6269`, `:6138`), each a frame of its own at
   level 1's HWRT size (`:6269`), so each level is one kick; the pass in
   front of it is the render to the texture the chain reads, still open
   when the chain starts (how it is closed was not followed here). `flush` (`:8493`) submits the last one
   (`:6279`). The kicks so far are posted, not waited for: `submit`
   (`driver/rust/src/gpu/pvr/render.rs:1380`) takes an HWRT member
   (`hwrt_get`, `:1388`; since `ce3354ac` a third and fourth kick of a size
   take the second pair, `:745-752`, no wait for four kicks), writes the
   client CCBs and posts the KCCB kick(s) (`:1620-1634`, `kccb_post`,
   `ccb.rs:68`), keeping a copy in `r.last_kick` for the re-post
   (`:1626-1632`). The geometry UFO throttle (`:1393-1398`) does not wait
   here (4 kicks against `GEOM_UFO_SLOTS`).
2. **The back end waits for the last kick.** `mip_chain_gpu` takes
   `p.b.mip_fence` (`:6284`) and calls `wait_fence` (`:6291`). `wait_fence`
   (`:2327`) returns at once if the OSAL's `completed` already passed it
   (`:2328`), else calls the OSAL's `wait` with `WAIT_MS` (`:2336`).
3. **The OSAL.** `z_wait` (`src/osal/zephyr.c:636`) turns the timeout into
   ms (`:643`) and calls `pvr_render_wait` (`:644`;
   `driver/src/pvr_render.c:778`), which syncs the knobs (`rr_sync_cfg`,
   `wait_repost` on by default, `pvr_render.c:347`, `:597`) and calls
   `yd_pvr_render_wait` (`render.rs:2680`) under the label "frame fence",
   i.e. `wait_for` (`render.rs:2327`). After it, `z_wait` drains the
   pending frees (`zephyr.c:648`), CPU work of microseconds.
4. **`wait_for`, one iteration per millisecond** (`render.rs:2333`):
   - `fence_reached` (`:2334`, `:2033`): invalidate the fragment UFO word
     and read it. This is the only thing that ends the wait: the fence moves
     when the firmware writes the fragment UFO at the end of the waited
     kick's 3D phase.
   - `fwccb_service_r` (`:2352`, `:2045`): serve freelist grows. It moves
     no fence of a mip kick (a chain of small levels asks for no grow).
   - the re-ring (`:2358`, off by default) and **the re-post**
     (`:2365-2382`): once, at `i == 20`, the kicks in `r.last_kick` are
     posted again unchanged. Its comment and `pvr_render.c:335-347` give the
     reason: the firmware sits on a posted kick until another command
     arrives, root cause not found. The log line is printed for the first 8
     re-posts and then every 256th (`:2371`), so a console's count of
     "re-posted" lines stops at 8; the true count is `r.reposts`
     (`pvr_render_reposts`, `pvr_render.c:348`, `:784`), which nothing
     prints.
   - the HWR check (`:2386-2436`): one invalidate and one read of the HWR
     counter; moves nothing.
   - **the spin**: `busy_wait_us(WAIT_POLL_US)` (`:2442-2453`), with
     `WAIT_POLL_US` = 1000 (`:2027-2028`, without `pvr-fine-wait`): one
     `k_busy_wait(1000)` (`driver/src/pvr_fw_boot.c:108-111`). The CPU
     spins; it never sleeps and no interrupt is connected
     (`pvr-fence-wait.md`, "Interrupts").

What moves the fence, kick by kick of the waited four:

| Kick | Moved by | The CPU meanwhile |
|---|---|---|
| the pass | the firmware taking it from the KCCB (posted at step 1), its TA then 3D; the 3D writes the fragment UFO | records the next levels (steps 1) |
| level 1, level 2 | as the pass; each is taken when the firmware reaches it | records, then spins in `wait_for` (`render.rs:2444`, `:2453`) |
| level 3, the waited one | taken by the firmware only when it notices the post; if it sits on the post (the defect of `pvr_render.c:335-347`), nothing moves until the re-post at 20 ms (`render.rs:2365`) | spins 1 ms steps until `fence_reached`; the wait ends on the first whole ms after the UFO write |

## Where the 10.8 ms can come from

- **The 1 ms poll** (`render.rs:2027-2028`): the wait ends on the first
  whole millisecond after the fence moved, so it adds 0.5 ms on average and
  1 ms at most, once a prefix call (one wait). With the fine wait the
  `ce3354ac` terrain run went from 83.08 to 81.67 ms, about 1.4 ms over 9
  waits a frame (`pvr-fence-wait.md`'s "the polling period is not the
  cost"). **At most 1 of the 10.8 ms.**
- **Losing the pipeline** (hypothesis H2): queued, kick N's geometry runs
  beside N-1's fragment phase and one kick retires per about 1.1 ms; from
  an idle firmware, a kick's post-to-fragment-fence is about 3.85 ms in the
  light scenes (`pvr-fence-wait.md`, "What it says"). If the waited chain
  starts idle and its four kicks serialise, 4 x 3.85 = 15.4 ms, the measured
  15.39 ms. Against it: the four are posted within microseconds of each
  other (5 us submits), so the firmware has the next kick before the first
  one's geometry ends, as in the queued case; nothing in the code path
  above serialises them (no wait in `hwrt_get` for four kicks, none in the
  UFO throttle).
- **The stalled post** (hypothesis H1): the last kick before a wait has no
  command behind it. If the firmware takes a posted kick only when another
  command arrives, the waited kick sits until the re-post at 20 ms and the
  wait ends about 21-22 ms after it began (`ce3354ac`: "every frame now has
  one frame fence that does not move for 20 ms until the render service
  re-posts the kick"). With a fraction f of the 80 waits stalled and the
  rest at 4.6 + 0.5 ms: 5.1 (1 - f) + 21.5 f = 15.39 gives **f of about
  0.63, about 50 re-posts in the 80 calls**. Against it: main's case 216
  ends each of its 20 frames with a `glFinish`, also a wait with nothing
  behind it, and its 46.15 ms a frame leaves no room for a 20 ms stall
  each frame; so the stall would have to be likelier after a short chain
  (the prefix's four kicks) than after a frame's ten.

The records cannot tell H1 from H2: the re-post count is not printed
(above), and the kick-times build (`pvr-kick-times`) was not run with path
B. The two have different signatures, which is the first measurement of the
fix below:

| | H1 (stalled post) | H2 (serialised from idle) |
|---|---|---|
| `r.reposts` over case 216's 20 frames, path B | about 50 | 0 to 1 |
| waits of one prefix call | two clusters, about 5 ms and about 21 ms | one, about 15 ms |
| kick times, last kick: post to geometry done | about 20 ms in the stalled ones | about 1.1 ms, but its predecessors' fragment fences 3.8 ms apart |

## The proposed fix

Make the wait itself what nudges the firmware, early and only when the
waited kick has not been taken, and make the spin finer for this wait.

1. **Re-post as soon as the waited kick is not taken**, not at 20 ms. The
   signal is the waited kick's geometry UFO slot (`geom_ufo_slot`,
   `r.gufo_off`): written when the firmware has run the kick's TA, about 1.1
   ms after the post in every record. In `wait_for`, once the fence is
   `REPOST_EARLY_MS` (3 ms) into the wait and the geometry slot has not
   reached the fence, post `r.last_kick` again (the same command the 20 ms
   re-post sends, harmless in the 60 runs of `pvr_render.c:340-345`) and
   count it in a new `r.reposts_early`. The 20 ms re-post stays as the
   backstop. A wait whose kick is taken (geometry done) is never re-posted,
   so a pipelined frame posts nothing extra. If H1 holds this turns a
   21.5 ms wait into about 3 + 4.6 ms; if H2 holds it never fires and costs
   one UFO read a millisecond.
2. **A fine poll for short waits**: the first `FINE_MS` (4) milliseconds of
   every wait read the UFO every 20 us (the `pvr-fine-wait` step, measured
   harmless in `pvr-fence-wait.md` "The fine-wait run"), then 1 ms steps as
   today. The per-ms work (FWCCB, re-post, HWR) stays per ms. Saves about
   0.5 ms a wait.
3. **Print the counters**: `pvr_render_reposts` and the early count in
   gl_test case 216's per-case line (`driver/tools/pvrlab/gl_test.c`) and
   in the render service's end-of-run line, so the board run can read H1 or
   H2 without the rate-limited console.
   Case 216 now prints `gl_test: case 216 reposts: N over F frames (R a
   frame)` from `pvr_render_reposts` (the true total, not the log lines):
   with path B on, H1 predicts R near 2.5 (about 50 over 20 frames, one
   stalled post a frame's waited chain), H2 predicts R at or under 0.05
   (0 to 1 over 20), whatever the wait's poll costs.
4. If H2 holds instead, the waited chain's cost is the firmware's take-up
   from idle and no host change to the wait removes it: path B stays off,
   and the lever for the kicks is path A (one transfer submit for the chain,
   item `pvr-tq-mip-study`). An interrupt-driven wake (connect GIC SPI 241,
   `pvr-fence-wait.md` "Interrupts") saves the same at most 1 ms as (2) and
   needs the unmeasured fact that the DDK firmware raises OS0 on each UFO
   write; it is not proposed for this cost.

Files: `driver/rust/src/gpu/pvr/render.rs` (`wait_for`, a `cfg` knob for
`REPOST_EARLY_MS` and `FINE_MS`, `YdPvrRender` gains `reposts_early`: the
layout asserts at `render.rs:397-405` and the C mirror `struct yd_pvr_render` in
`driver/rust/include/yakogl_drivers.h:865` moves with it), `driver/src/pvr_render.c` (the knob
and the counter export, `:347`, `:597`, `:784`), and
`driver/tools/pvrlab/gl_test.c` (the print). The back end
(`src/pvr/backend.rs`) does not change for (1)-(3).

### Host test design

In `driver/rust/src/gpu/pvr/render.rs`'s `wait_tests` (`:3062`): the `Sim`
already has a simulated clock, the fragment UFO (`done_at_us`) and the
geometry UFO (`geom_at_us`). Add to it a count of KCCB posts (point
`k.kccb_ctl_off` and `kccb_cmds_off` at a zeroed block of the rig, read the
write offset after the wait), and a "taken on post" mode: the GPU's
`geom_at_us` and `done_at_us` start only at the first (re-)post after the
wait began, `done_at_us` = post + 4.6 ms.

- `a_waited_kick_not_taken_is_reposted_at_3_ms`: kick never taken until a
  post; the wait re-posts once at 3 ms, ends at about 3 + 4.6 ms (assert
  under 9 ms, not 20), `reposts_early` 1, `reposts` 0.
- `a_taken_kick_is_never_reposted`: `geom_at_us` 1,100 us, `done_at_us`
  4,600 us; 0 posts, the wait ends at 4,620 us (the fine step), not 5,000.
- `the_20_ms_repost_stays_the_backstop`: kick taken only at the second
  post: one early re-post at 3 ms, the backstop at 20 ms, the wait ends
  after it; both counted.
- the existing `a_wait_ends_on_the_first_poll_step_after_the_fence_moved`
  (`:3203`) is rewritten for the two steps (20 us in the first 4 ms, 1 ms
  after).

And in `src/pvr/backend_tests.rs`, the prefix rig test
`a_chain_with_small_levels_draws_the_large_ones_and_waits_for_them`
(`:5997`) keeps its kicks and its single wait (the fix changes no command the
back end records). `tests/es2_report_guard.rs` is unaffected (no GL answer
moves).

### Expected gain and the confirming record

Predictions, valid only if the first board run reads H1:

- gl_test case 216 with path B on (`mip_cpu_tail` true): each prefix call's
  backend time from 15.39 to about 5-8 ms, the frame from 82.95 to about
  82.95 - 4 x 9 = 47 ms, still not under main's 46.15 ms because of the
  CPU tail's 7.87 ms a call (the uncached carve-out). Path B wins only with
  the tail fixed too.
- case 216 and terrain with path B off (main): the change is the early
  re-post of a stalled frame fence and the fine poll. Terrain 83.06 ms a
  frame (`tools/cts/RESULTS.md`, Common set, `6a7021ec`): about 0.5 ms a
  wait from (2), 9 waits a frame (`ce3354ac`), so about 4 ms, plus up to
  17 ms in each frame that met the 20 ms stall `ce3354ac` names. Expect 77
  to 79 ms; more only if the counter shows a stall in most frames.
- **The record value that confirms it**: case 216 with path B on,
  `reposts_early` about 50 and `reposts` (20 ms) 0 over the 20 frames, and
  ms a frame under 50; with path B off, terrain's ms a frame below 83.06
  with 0 GPU resets and the picture of column `am62p`. If the first run
  reads `reposts` 0 with path B on, H2 holds and the fix is (2) alone.

## Ordering

The fix's files are owned by running plans: `driver/rust/src/gpu/pvr/**`
and `driver/src/pvr_render.*` by #325 (PowerVR ES 3.0 host work) and
`driver/rust/src/gpu/pvr/**` by #301 (partial render);
`src/pvr/backend.rs` and `src/pvr/backend_tests.rs`, if a back-end knob is
added to turn path B on for the measurement, by #325. It goes after their
items release them, small, rebased on the current main.
