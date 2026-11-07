<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: how the CPU waits for a job, and what a wait costs past the GPU's end

Study for the Pi 4 item `v3d-wait-poll-study` of plan #329 (the Pi 4
counterpart of `docs/notes/pvr-fence-wait.md`). Read at main `ea371491`
(line numbers are that tree's). Read-only: no code changed, nothing ran on a
board. Every microsecond figure below is an **estimate from the code**, not a
measurement; the record values that would turn each into a measurement are
listed in the last section.

## Answer in four lines

- A V3D wait is a **busy poll** of the core and hub interrupt-status registers,
  one `k_busy_wait(1)` between reads. No `k_sleep`, no semaphore, no tick is
  involved, so the Zephyr tick rate (10 000 a second, see 2) does not touch it.
- FLDONE (bin finished) has an interrupt, which starts the render. FRDONE
  (render finished) has none: the CPU learns of it only inside a wait,
  `completed()`, the next submit, or a gpu_free drain.
- The poll's own overshoot is small, about 1 to 3 us a wait. What follows
  FRDONE on the CPU (cache clean, queued TFU jobs, starting the waiting job)
  is larger, about 10 to 60 us, and it is GPU idle time.
- Terrain: 9 waits a frame, so about 0.1 to 0.6 ms a frame in total: under
  0.4 % of the 161.8 ms frame, 0.3 to 2 % of the ~30 ms frame expected once
  `pi4-target-mark-clear` lands. **The poll is not where terrain's Pi 4 time
  is.** Nothing here is worth a code change before a board counter says so.

## 1. The mechanism, call by call

1. The back end waits in `wait_job` (`src/v3d/backend.rs:3051`-`3071`): it
   takes the OSAL's `wait` hook and calls `wait(f.fence, JOB_TIMEOUT_US)`
   (`:3056`; `JOB_TIMEOUT_US` is 1 s, `:443`), then counts `g.waits += 1`
   (`:3057`). Its callers: `wait_in_flight` (`:3028`) and `wait_queued`
   (`:3040`). `wait_in_flight` waits for the older job first, then the newer
   (`:3029`-`3036`).
2. The Zephyr OSAL's hook is `z_wait` (`src/osal/zephyr_v3d.c:1510`). For a
   fence that is the job in flight or the one waiting behind it, it calls
   `finish_inflight(timeout_us == 0)` (`:1516`-`1525`); the back end's timeout
   is not 0, so it blocks. A fence older than that answers from two saved
   results (`:1526`-`1528`) with no wait at all. The comment at `:1490`-`1509`
   says the bound is `JOB_HANG_US` = 4 s (`:598`) whatever the caller passes.
3. `finish_inflight` (`:1256`-`1334`):
   a. if the job's binner is still running, `bin_finish` (`:1162`): under
      `irq_lock` it checks the ISR's verdict, the MMU fault bits and the 4 s
      bound, and otherwise loops on `k_busy_wait(10)` (`:1222`). The ISR is
      what normally ends the binning (below), so this loop waits for an
      interrupt that fires on its own;
   b. `v3d_wait(..., FRDONE, ...)` (`:1281`) -> `wait_bits`
      (`driver/src/v3d/v3d_hw.c:449`-`506`): each pass reads
      `V3D_CTL_0_INT_STS` (core) and `V3D_HUB_CTL_INT_STS` (hub) (`:466`-`467`),
      tests the MMU fault bits and the out-of-memory bit, returns 0 when the
      wanted bit is up (`:493`), else `k_busy_wait(1)` (`:501`);
   c. `v3d_caches_clean` (`:1301`; `v3d_hw.c:377`-`389`): a TMU write-combiner
      flush and an L2T clean, each a `poll32` loop on a register with
      `k_busy_wait(1)` between reads (`v3d_hw.c:32`-`41`);
   d. `tfu_queue_run` (`:1313`, `:1113`): the TFU jobs queued behind this job
      run now, each `tfu_now` -> `v3d_tfu_run` (`v3d_hw.c:521`-`578`), the same
      shape of poll (`k_busy_wait(1)` at `:572`);
   e. `complete` (`:1315`, `:1146`): the fence completes and `drain()` gives
      back the frees held for the job;
   f. `start_pending` (`:1318`, `:1468`): the job waiting behind starts now.
      `start_job` (`:1424`) zeroes that job's tile state with `memset` into
      non-cacheable memory (`:1433`), then `v3d_submit_bin`
      (`v3d_hw.c:401`-`419`).
4. What has an interrupt. `init` connects `v3d_isr` to the V3D line
   (`IRQ_CONNECT(V3D_IRQ, ...)`, `:263`-`264`; `V3D_IRQ` = 74 + 32, `:171`).
   The mask holds only `V3D_BIN_IRQS` = FLDONE | OUTOMEM (`:172`, unmasked in
   `start_job`, `:1458`). `v3d_isr` (`:1012`) feeds the binner an overflow
   block on OUTOMEM and, on FLDONE, starts the render
   (`v3d_submit_render`, `:1050`). The comment at `:1273`-`1276` says it:
   "FRDONE has no interrupt". So **no wait sleeps on an interrupt**: the CPU
   spins on registers while the ISR runs inside the spin. The ISR also
   clears `bin_pending`, which `bin_finish` reads.
5. The TFU: `v3d_tfu_run` spins on the hub status for TFUC (`v3d_hw.c:555`-`573`).
   `osal_v3d_tfu` first runs `finish_all` (`zephyr_v3d.c:1346`-`1349`), so a
   TFU job is never beside a render.
6. `z_completed` (`:1484`-`1491`) is the non-blocking look: if
   `inflight_ready()` (`:1230`) sees FRDONE, it runs `finish_inflight(false)`
   (so it, too, ends with `start_pending`). The back end's `fence_done`
   (`src/v3d/backend.rs:3075`) calls it. `z_submit` finishes the job in flight
   when one is also waiting (`zephyr_v3d.c:1397`-`1398`).

There is **no** `k_sleep`, `k_msleep`, `k_usleep`, `k_sem` or `k_timer` in
`src/osal/zephyr_v3d.c` or `driver/src/v3d/*.c` (grep at this commit). The
only time function is the busy wait and `k_cycle_get_32` stamps.

## 2. The Zephyr tick rate on the Pi 4

- `driver/zephyr/lib/board/rpi4/app.conf` has no tick or timer line, and
  neither does `driver/samples/zephyr/glmark2/prj.conf` or its
  `boards/rpi_4b.conf` (grep for TICK, SYS_CLOCK, TIMER, SLEEP: none).
- The pinned Zephyr (`/home/orion/connected-deps/zephyr-ws/zephyr`) does the
  rest: `soc/brcm/bcm2711/Kconfig` selects `ARM_ARCH_TIMER`;
  `drivers/timer/Kconfig.arm_arch` makes that `TICKLESS_CAPABLE` and
  `ARCH_HAS_CUSTOM_BUSY_WAIT`; `kernel/Kconfig:1043` makes
  `TICKLESS_KERNEL` default y then; `kernel/Kconfig:697`-`701` gives
  `SYS_CLOCK_TICKS_PER_SEC` 10 000 for a tickless kernel. **So 10 000 ticks a
  second, 0.1 ms a tick.** This is read from the Kconfig sources, not from a
  board build's `.config` (check `build/.../zephyr/.config` on the next image).
  `docs/notes/imx8mp-frame-floor.md:234`-`240` derives the same for the
  i.MX8MP by the same route.
- It does not enter the wait: `k_busy_wait` is the arch timer's own
  (`arch_busy_wait`, `drivers/timer/arm_arch_timer.c:160`), a counter loop, not
  a tick sleep. A `k_sleep` would round to 0.1 ms; there is none.

So the AM62P's problem (a 1 ms granularity, `pvr-fence-wait.md`) has no
counterpart here: the Pi 4's poll was always fine-grained.

## 3. The overshoot per wait: from FRDONE rising to the CPU acting

Two parts. The first is time the CPU is late; the second is CPU work that
delays the *next* GPU job (the GPU idles while it runs). Neither is
measured; each is built from the loop above.

**(a) Noticing.** The loop period is one `k_busy_wait(1)` (>= 1 us, the
counter loop, `arm_arch_timer.c:180`-) plus two device-memory register reads.
A read of a V3D register from the A72 is assumed 0.3 to 1 us (device memory,
one bus hop; no measurement in the tree). Period about 1.6 to 3 us, so a
uniform arrival costs half of it on average: **0.8 to 1.5 us, worst 3 us.**

**(b) Work after noticing, before the GPU gets its next job.**

| step | cost, assumed | basis |
|---|---|---|
| `v3d_caches_clean`: TMU flush then L2T clean | 2 to 20 us | two `poll32` loops, 1 us steps (`v3d_hw.c:377`-`389`); the L2T clean writes back dirty lines, so it grows with what the job wrote |
| `tfu_queue_run` | 0 unless TFU jobs are queued behind this job; each is GPU time anyway | `:1113` |
| `complete` + `drain` | a few us | frees held for the job (`v3d_frees.h`) |
| `start_job`: tile-state `memset` (non-cacheable) + bin submit | 5 to 40 us | tile state of an 800x600 job is some KB; about 40 ns a non-cacheable load was measured for reads (`pi4-slow-terrain.md`, "about 40 ns per non-cacheable"); stores are cheaper, so take 5 to 40 us |

(b) happens **only when a job is waiting behind**; with a single job in
flight `start_pending` returns at `:1469`. Total (a)+(b): **about 10 to 60 us
per wait that has a job behind it, 2 to 25 us per wait that does not.**
Then GPU idle between two back-to-back jobs is that same figure; the code's
own name for it is `idle`: "the cycles from the end of the previous job's GPU
work ... to this job's bin submit" (`zephyr_v3d.c:741`-`746`).

A caveat that is larger than any of these: **FRDONE is noticed only when the
CPU asks.** If the CPU is busy elsewhere (a CPU mipmap filter, the CPU tiler)
when the render ends, the waiting job does not start until the next wait,
`completed()`, submit or allocation. That is not poll granularity; it is
the absence of an interrupt. In terrain today it is hidden: `settle_texture`
drains the GPU before the CPU filter (`pi4-slow-terrain.md`, call sites),
so the GPU is idle through the filter anyway, by the render-image thrash,
not by the poll.

## 4. Terrain: waits a frame and what they sum to

Waits a frame: **9** (9 jobs, each waited for once, on host and board;
`pi4-slow-terrain.md` section "CPU waits on the GPU, with call sites").
Their split depends on the thrash: with it, 7 in `glGenerateMipmap`, 1 in
`glDrawArrays`, 1 in `eglSwapBuffers`; without it 5, 1 and 3
(`pi4-slow-terrain.md:189`-`192`). `v3d-terrain-copies.md:166` and `:174` give
"up to 4 CPU waits" for the copies and the TFU; up to 6 plus the swap is the
task's figure and is the same set seen through the copy paths. The swap's
wait is one of the 9 either way.

| | waits a frame | per wait | per frame | share of the frame |
|---|---:|---|---:|---|
| before `pi4-target-mark-clear` (161.8 ms) | 9 | 10 to 60 us if a job is behind, else 2 to 25 | **0.1 to 0.5 ms** | 0.06 to 0.3 % |
| after it (expected ~30 ms, GPU-bound) | 9 | 10 to 60 us (CPU idle in waits, jobs queue behind) | **0.1 to 0.6 ms** | 0.3 to 2 % |

Before the fix, the CPU spends ~130 ms in non-cacheable loops with the GPU
drained, so most waits find FRDONE already up (`render_late` = 1, so `~`
in `glmark2-jobs`) and cost only the clean and the next start. After the
fix the frame is GPU-bound, the CPU sits in the polls, and nearly every wait
has a job behind it; that is the case of the high end, per job `idle` is the
gap it measures.

For scale, the AM62P's 20 us poll gained 1.4 ms (`pvr-fence-wait.md`, "The
fine-wait run") because the poll was 1 ms. The Pi 4 poll is 1 us: the same
change here is worth at most ~1 us x 9 = 0.01 ms a frame.

## 5. Alternatives, ranked

Gains are ms a frame on terrain after `pi4-target-mark-clear`; the base is ~30 ms.

1. **Wait only at the swap, i.e. fewer CPU waits** (the queue of
   `v3d-swap-wait.md` section 4, the no-wait TFU mipmap of `v3d-tfu-order.md`,
   the `wait_kick` instead of a drain in `pi4-slow-terrain.md` candidate 4).
   Not a poll change: it removes GPU idle and CPU-before-GPU serialisation,
   which are a few ms where the poll is ~0.3 ms. **Gain: bounded by the
   overlap, a few ms (those notes); it also removes the overshoot of every
   wait it removes.** Risk: the single job state, arenas and unordered
   cross-job reads (`v3d-swap-wait.md` section 3); needs hazard tracking and
   an order test. Owner: `src/v3d/**`, `src/osal/zephyr_v3d.c`.
   **Confirm with:** `glmark2-frame` gl and swap for terrain, and the
   per-scene wait count (`v3d_counters`' waits), falling.
2. **An FRDONE interrupt that starts the waiting job** (unmask FRDONE in the
   ISR, run `v3d_caches_clean`'s start and `start_pending` there or in a
   deferred thread). It removes the (b) idle gap when a job waits behind,
   **even when the CPU is not in a wait**, which the polls cannot. **Gain:
   0.1 to 0.5 ms for terrain; more in a scene with CPU loops between jobs.**
   Risk: medium. `start_job` zeroes tile state with `memset` and `finish_inflight`
   runs the TFU queue and `drain()`, so those cannot all move into an ISR;
   the ISR and `bin_finish`/`finish_inflight` would race on `s.inflight` and
   `s.pending`, today separated by `irq_lock` only in `bin_finish` (`:1167`);
   the hang bound and `recover` rely on the CPU looking. Hardware FRDONE
   mask bit exists (the same register as FLDONE, `V3D_CTL_0_INT_STS`).
   **Confirm with:** `idle` in `glmark2-jobs:` falling toward the clean's
   figure and the frame's gl+swap falling by about the sum of the gaps.
3. **A finer poll** (drop `k_busy_wait(1)`, read back to back; or read only
   the core register in the loop, the hub register on exit). **Gain: at most
   ~1 us a wait, 0.01 ms a frame.** Risk: low (the same registers), though
   a read-only spin hammers the register bus; the hub status read could be
   moved to every 8th pass. Not worth a commit alone. **Confirm with:** the
   `wait time` counter of item `v3d-wait-time` divided by waits: a mean over
   ~5 us means the loop itself is slower than assumed here.
4. **Sleeping or an interrupt with a semaphore for the wait itself.**
   **Gain: negative.** The GIC wake and thread switch cost 5 to 15 us on an
   A72 (assumed), more than a spin's 1 to 3 us, and the CPU has nothing else
   to run in a wait (one application thread). It saves power, not frame
   time. Risk: medium (needs an FRDONE interrupt as in 2 plus a k_sem and
   its own timeout path). Do not do it for speed.

## 6. What a board record has, and what it needs

Existing, printed by the GM image (`driver/samples/zephyr/glmark2/src/main.cpp:1255`-`1270`):
`glmark2-jobs:` lines list each completed job as `total`, `render` (with `~`
when FRDONE was already up, i.e. a lower bound), `idle` (the gap above), and
`tfu K/Tus`. They are printed once a scene at a fixed frame, from the ring
(`zephyr_v3d.c:748`-`758`). The fields (`prep`, `bin`, `render`, `clean`,
`total`) are in `done_job_cyc` (`:728`-`740`).

| to confirm | value | where |
|---|---|---|
| the poll's overshoot is a few us | `wait time` mean per wait < 10 us on a job whose `render_late` is 0 | item `v3d-wait-time` (not merged at this commit) |
| (b) is the gap | `idle` of a job that followed another directly (not after a CPU filter) = 10 to 60 us | `glmark2-jobs:` terrain |
| the sum is under 1 ms | terrain `glmark2-frame:` gl + swap minus the GPU work (29.1 ms at frame 10) < 1 ms after the mark fix | the "after" record of `pi4-board-after` |
| waits a frame | `v3d_counters` waits / frames = 9 | the per-scene line `pi4-slow-terrain.md` asks for |
| tick rate | `CONFIG_SYS_CLOCK_TICKS_PER_SEC=10000` | `ticks_per_sec` in `main.cpp:912`, or the build's `.config` |

If the `wait time` per frame comes out above ~2 ms after the mark fix, this
note's estimate is wrong and the likely cause is slower register reads than
assumed (candidate 3) or the L2T clean (3b); look at `clean` in the job
ring first.

## Not in this note

- V3D 7.1 (Pi 5) shares `wait_bits` and the same OSAL shape; the Pi 5 has
  its own plan (#321) and is not read here.
- Any code change. The candidates are for the planner, with the file
  owners named above (`src/v3d/**` is held by #308 and `v3d-rt-evict`).
