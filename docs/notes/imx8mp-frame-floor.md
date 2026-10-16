<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP: why glmark2 frames take "about 24 ms" (host investigation)

**Status (2026-10-14): F1 and F2 applied, not yet run on a board. F2 is
not reached yet.** The cause below is a prediction from the code; the board
recipe has not run.

- F1: `board_show` returns 0 without `imx8mp_display_set_fb` when the
  LCDIF already scans the shown buffer
  (`driver/zephyr/lib/board/imx8mp/board_imx8mp.c:158-179`).
- F2: a strong `yakogl_zephyr_wait_vblank` waits for the next VSYNC
  (`board_imx8mp.c:186-229`). Nothing calls it yet. The GC7000's OSAL,
  `src/osal/zephyr_gc.c`, has no `.wait_vblank` (see "F2 is not reached
  yet"). Until a change in `src/` wires it in, no swap waits on the
  i.MX8MP. That is what glmark2 asks for, but Slint, the cluster and
  gl_test run unpaced.
- `CONFIG_YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC=y`
  (`driver/zephyr/lib/board/Kconfig:101-113`) brings back the old
  behaviour, for an A/B run or for a paced Slint or cluster image until
  then.

See "Fixes applied" and "Board recipe", step 2 (the next i.MX8MP board
session). The sections up to "Proposed fix" describe the code before F1
and F2, at `fdff1147`.

Written 2026-10-08 from the code at `fdff1147`. Line
numbers are that commit's. Host-only: this item reads code and earlier
records. It ran nothing on a board and changed no code.

## Answer in short

The most likely cause: **every glmark2 frame on the i.MX8MP waits for the
panel's VSYNC**, and glmark2 asked for no wait.

- `board_show()` calls `imx8mp_display_set_fb()` every frame
  (`driver/zephyr/lib/board/imx8mp/board_imx8mp.c:153-154`).
- That is `lcdifv3::set_fb`, which sets `SHADOW_LOAD_EN` and then spins until
  the LCDIF clears it. The LCDIF clears it "on VSYNC of next frame"
  (`driver/rust/src/ip/lcdifv3.rs:21-27`, `:239-251`, `:284-292`).
- The panel runs at 58.76 Hz (69.3 MHz pixel clock,
  `driver/samples/zephyr/gc_common/verdin_imx8mp_display.dtsi:169`;
  `driver/src/display/NOTES.md:180-186`, measured 17 018 us).
- glmark2 calls `eglSwapInterval(dpy, 0)`
  (`$GLMARK2_SRC/src/gl-state-egl.cpp:426-427`, upstream 22c527cb): it asks
  the platform not to wait for VSYNC. The board layer waits anyway.

So each scene runs at a whole number of display periods: 17.0 ms (58 FPS),
34.0 ms (29 FPS), 51.1 ms (19 FPS), ... The "24 ms" is not a floor. It is the
**mean** of scenes at 17.0 ms and scenes at 34.0 ms. The scores 46, 39 and 37
fit this with 18 of 30, 10 of 30 and 9 of 33 scenes at one period (the
arithmetic below). No scene can run faster than 58 FPS, which is why the
score stays far below the AM62P (550) and the Pi 4 (842). Their `board_show`
does not wait for VSYNC.

This is a prediction from the code. No per-scene number exists for the
i.MX8MP (it has no UART). The recipe below confirms or rejects it with one
board run and no code change.

## What is known (inputs)

| Run | Score | Scenes | Source |
|---|---|---|---|
| common set round 2, copy path | 39 | 30 ran, 1 failed, 2 unsupported | `tools/cts/RESULTS.md:457`, record `594b120e 00000027 021e0102 03200258` |
| the same with the scan-out view (no copy) | 46 | (not in RESULTS.md) | `ab43a20c5:tools/cts/log/status-2026-09-25-to-10-07.md:88` |
| common set round 4, copy path | 37 | 33 ran, 0 failed, 0 unsupported | `tools/cts/RESULTS.md:490-501`, record `594b120e fc3f0025 2184003f 03200258` |

- The common-set glmark2 row is built without the view
  (`tools/cts/common-tree.txt:22-23`, `tools/cts/common-images.sh:110`). So 39
  and 37 are copy runs.
- glmark2's score is the mean of the scenes' FPS. A scene's FPS is
  `unsigned(frames / elapsed)` (`$GLMARK2_SRC/src/scene.cpp:139-142`,
  `main-loop.cpp:53-58`, `:118`).
- The i.MX8MP's glmark2 prints no `glmark2-frame:` line: no UART, and the
  per-kick times are "absent on the GC7000"
  (`driver/samples/zephyr/glmark2/src/main.cpp:24-40`).
- "1000 / 42 = 24 ms" is the score turned into a frame time. It is a mean
  over scenes, not a measured minimum.

## The frame path, end to end

One glmark2 frame (`driver/samples/zephyr/glmark2/src/main.cpp:379-391`,
upstream `MainLoop::draw`):

1. `canvas_.clear(); scene_->draw(); scene_->update();` The GL calls record
   the frame into the GC back end's command stream (`gl` in `glmark2-frame`).
2. `canvas_.update()` = `eglSwapBuffers` then `flip()`:
   - `eglSwapBuffers` (`src/gl/egl.rs:2086-2136`):
     - `backend::flush()` -> `gc_flush` -> `frame_submit`
       (`src/vivante/backend.rs:2531-2533`, `:1733-1792`): pad, clean the CPU
       cache over the stream and arena (`:1766-1774`), OSAL submit.
     - OSAL `z_submit` (`src/osal/zephyr_gc.c:371-420`) ->
       `gc_render_submit` (`driver/src/vivante/gc_render.c:578-621`) ->
       `ring_submit` (`driver/rust/src/gpu/vivante/cmdbuf.rs:131-205`):
       prologue flush + FE/PE semaphore, LINK to the stream, epilogue flush +
       semaphore + EVENT, new WAIT/LINK idle loop, then the old WAIT becomes a
       LINK.
     - `backend::finish_previous()` -> `gc_finish_previous`
       (`src/vivante/backend.rs:2569-2579`): waits only for the frame
       submitted at the previous swap (pipelined, since 2026-10-01).
     - The wait: `z_wait` (`src/osal/zephyr_gc.c:499-515`) -> `timed_wait` ->
       `gc_render_wait` (`driver/src/vivante/gc_render.c:886-1002`). It polls
       `fence_done` and sleeps `k_sleep(K_MSEC(1))` between polls (`:978`).
       The ISR moves `fence_done` (`driver/rust/src/gpu/vivante/irq.rs:53-92`).
     - The vertical-sync loop: `for _ in 0..e.swap_interval`
       (`src/gl/egl.rs:2130-2134`). glmark2 set the interval to 0, so it does
       nothing. (Even at 1 it would not wait: no i.MX8MP code defines
       `yakogl_zephyr_wait_vblank`, so the weak default returns -ENOTSUP,
       `src/osal/zephyr.c:602-611`.)
   - `flip()` = `board_show(&win_)` (`main.cpp:211-224`) ->
     `board_imx8mp.c:112-163`:
     - copy path (no view): `memcpy` of 800x600x4 = 1.92 MB, row by row, from
       the window into a panel-sized buffer (`:137-151`). Both are in the GPU
       carve-out, which is Normal non-cacheable (`:5`).
     - then, always: `imx8mp_display_set_fb(scan)` (`:153-154`) ->
       `lcdif_set_fb` (`driver/src/display/imx8mp_lcdif_rust.c:66-72`) ->
       `yd_lcdif_flip` (`driver/rust/src/ffi/imx8mp_dsi.rs:381-384`) ->
       `lcdifv3::set_fb` (`driver/rust/src/ip/lcdifv3.rs:284-292`): write the
       address, set `SHADOW_LOAD_EN`, and `wait_shadow_loaded` busy-waits in
       50 us steps until the next VSYNC (`:239-251`, `:116-119`).

The window is one buffer (`board_window`, `board_imx8mp.c:41-110`, `taken`).
So the address written every frame is the same one the LCDIF already scans.
The write changes nothing on the screen. Only the wait remains.

## Candidate causes, ranked

### 1. The flip waits for VSYNC every frame (most likely; explains the numbers)

Evidence:

- `driver/zephyr/lib/board/imx8mp/board_imx8mp.c:153-154`: `set_fb` on every
  `board_show`, with the same address.
- `driver/rust/src/ip/lcdifv3.rs:21-27`: the shadow load completes on the next
  VSYNC. `:239-251`: `wait_shadow_loaded` spins until then. `:284-292`:
  `set_fb` always waits.
- `driver/samples/zephyr/gc_common/verdin_imx8mp_display.dtsi:169`:
  `pixel-clock-frequency = <69300000>`. `driver/src/display/NOTES.md:180-186`:
  htotal 1404, vtotal 840, so 58.761 Hz; measured 17 018 us a frame.
- `$GLMARK2_SRC/src/gl-state-egl.cpp:426-427`: glmark2 asks for swap interval
  0 (unless `--swap-mode fifo`).
- Earlier GC7000 measurement of the same mechanism: gc_slint's frame-time
  histogram had "none under 17 (the flip waits for the latch, so 48 ms of GPU
  becomes three vsync periods)" (`tools/cts/RESULTS.md:15827-15828`).
- The other boards do not wait in `board_show`. The AM62P's `show` is 3 us a
  frame (`docs/notes/pvr-fence-wait.md`, "What was measured"). The Pi 4
  presents by a pan of about 0.1 ms and does not use SET_VSYNC
  (`tools/cts/RESULTS.md:10044-10060`, FBFLIP5, "a 60 fps present pans and does not wait
  on SET_VSYNC").

The arithmetic. Let T = 17.018 ms. One frame is
`gl + swap + copy + (wait to the next VSYNC)`. The GPU draws frame N during
the wait and the next frame's `gl`; the swap of N+1 waits for N. So a scene
whose `gl + swap + copy` stays under T runs at exactly one period. A scene
that needs more runs at two periods, and so on.

- A scene's FPS is `unsigned(frames / elapsed)`: 58 at one period (58.76,
  truncated), 29 at two, 19 at three, 14 at four.
- The score is the mean over the scenes. With `a` scenes at 58 and the rest
  at 29 (n scenes): `score = 29 + 29 a / n`, so `a = n (score - 29) / 29`.

| Run | n | a (one period) | check | mean frame (1000 / score) |
|---|---|---|---|---|
| view, 46 | 30 | 17.6, so 18 | (18 x 58 + 12 x 29) / 30 = 46.4 | 21.7 ms |
| copy, 39 | 30 | 10.3, so 10 | (10 x 58 + 20 x 29) / 30 = 38.7 | 25.6 ms |
| copy, 37 | 33 | 9.1, so 9 | (9 x 58 + 24 x 29) / 33 = 36.9 | 27.0 ms |

So "about 24 ms" is the mean of 17.0 ms and 34.0 ms frames. Some scenes at
three periods would lower `a` a little; the conclusion is the same. Under
this model the score can never exceed 58 on this panel. All three runs are
below it.

Why the copy costs 7 points (39 against 46) but is "not the floor": the copy
reads 1.92 MB of non-cacheable memory in every frame. That moves about 8 of 30
scenes from one period to two. It does not set the floor. The VSYNC wait
does, with or without the copy. This matches ab43a20c5:tools/cts/log/status-2026-09-25-to-10-07.md:88 ("the floor stays
when the view removes the copy").

What would reject this candidate: a single light scene (`-b clear`) that
scores above 58 (59 or more). Then the frame is not locked to the panel.

### 2. GPU time per frame (decides which scenes need two periods)

This is not the floor, but it decides where each scene lands, and it is the
next limit after candidate 1 is fixed.

- glClear is a full-screen quad with colour and depth, not a tile-status fast
  clear (`src/vivante/backend.rs:1796-1846`).
- The colour target is linear, so there is no early depth test
  (`src/vivante/backend.rs:1584-1590`): every covered pixel is shaded before
  the depth test.
- Measured on this GPU (gc_slint, 1280x800, `tools/cts/RESULTS.md:15802-15816`):
  per-draw flushes and stalls cost nothing measurable (47.4 against 47.9 ms);
  with every fragment shader cut to 1 instruction the frame still took 8.4 ms
  for about 2.15 M rasterised pixels (RA 2.1x the screen), about 3.9 ns a
  pixel. The clock was checked at 799 MHz (`:15765`).
- At 3.9 ns a pixel, one 800x600 full-screen pass is about 1.9 ms. So a
  light scene (clear + one model) is a few ms of GPU, well under T. A scene
  with full-screen shading of 30-50 instructions per pixel (effect2d,
  desktop, jellyfish, terrain, refract) can pass 17 ms. That is consistent
  with about a third to two thirds of the scenes at two periods. The exact
  split is what the record below measures.

### 3. The fence-wait poll: `k_sleep(K_MSEC(1))` (small)

- `driver/src/vivante/gc_render.c:978`: the wait sleeps 1 ms between polls.
  A wait that is not already done at the swap ends up to about 1 ms after the
  fence (plus one 0.1 ms tick of Zephyr's rounding).
- This is the same class as the AM62P's 1 ms poll, which cost about 0.1 ms a
  frame there (303 -> 316 with a 20 us poll; commit `78c4c208`,
  `docs/notes/pvr-fence-wait.md`, "The fine-wait run").
- At most about 1 ms a frame here. It matters only when it pushes a scene
  over T, and after candidate 1 is fixed.
- The tick rate is 10 000 a second, so the sleep is not tick-rounded to a
  large value. This repository does not set `SYS_CLOCK_TICKS_PER_SEC` (not in
  `driver/boards/toradex/verdin_imx8mp/verdin_imx8mp_mimx8ml8_a53_defconfig`
  or the glmark2 confs). The pinned Zephyr `af17c0c8` (`west.yml`), read
  upstream for this item:
  - `kernel/Kconfig`: `SYS_CLOCK_TICKS_PER_SEC` is `default 10000 if
    TICKLESS_KERNEL`; `TICKLESS_KERNEL` is `default y if TICKLESS_CAPABLE`.
  - `soc/nxp/imx/imx8m/Kconfig`: `SOC_MIMX8ML8_A53` selects `ARM_ARCH_TIMER
    if SYS_CLOCK_EXISTS`. No file of `soc/nxp/imx/imx8m/` sets the tick rate
    or `TICKLESS_KERNEL`.
  - `drivers/timer/Kconfig.arm_arch`: `ARM_ARCH_TIMER` selects
    `TICKLESS_CAPABLE`.
  - `soc/nxp/imx/imx8m/Kconfig.defconfig.mimx8ml8_a53`: sets only
    `SYS_CLOCK_HW_CYCLES_PER_SEC` (8 MHz), not the tick rate.
  - `kernel/timeout.c` `z_add_timeout`: a relative timeout of n ticks expires
    after n + 1 ticks (`timeout.ticks + ticks_elapsed + 1`).

  So `k_sleep(K_MSEC(1))` is 10 + 1 ticks: 1.0-1.1 ms. Page 2 of the record
  below still reports the built value, as a check on the built image.

### 4. The copy in `board_show` (only without the view)

- `driver/zephyr/lib/board/imx8mp/board_imx8mp.c:137-151`: 1.92 MB `memcpy`
  per frame from Normal non-cacheable memory (`:5`).
- Explains 39 (copy) against 46 (view) through candidate 1 (see above). Not
  the floor.

### Ruled out or negligible (read, not measured here)

- The front end's idle loop: WAIT of `800 MHz >> 15` = 24 414 cycles = 30.5 us
  (`driver/rust/src/gpu/vivante/cmdbuf.rs:54-56`, `:28-29`). At most 30 us of
  latency when the FE is idle.
- The GPU clock: core, shader and AXI at 800 MHz from SYSTEM_PLL1
  (`driver/rust/src/gpu/vivante/power.rs:9-15`, `:53-58`), and the HI_PROFILE
  clock check read 799 MHz (`tools/cts/RESULTS.md:15765`).
- Full flushes: per-draw (`src/vivante/backend.rs:1597-1602`) and per-submit
  (`src/osal/zephyr_gc.c:399-402`). Measured negligible on the same GPU
  (RESULTS.md:15808).
- A full-GPU idle wait per frame: none. `gc_finish_previous` waits for the
  previous frame only (`src/vivante/backend.rs:2559-2579`); `frame_open`
  waits only when it reuses one of the 4 stream slots
  (`:73`, `:1682-1694`).
- The EGL vertical-sync loop: interval 0 from glmark2; and the hook is not
  wired on the i.MX8MP, see "The frame path". With interval 1 the weak
  default's -134 equals -ENOTSUP (`src/util.rs:17-29` reads Zephyr's
  numbers in Zephyr builds; host test `zephyr_osal_reads_zephyr_numbers`,
  `src/util.rs:376-384`), so the loop stops after its first call to the
  hook and does not wait.

## Comparison with the other drivers

- AM62P: its floor was the 1 ms fence poll (candidate 3's class), then the
  kick path. Its `board_show` is 3 us. The i.MX8MP's floor is a different
  thing: the display wait.
- Pi 4: the scan-out view removed a per-frame copy (201 -> 842); its present
  is a pan without SET_VSYNC.
- galcore / etnaviv: the WAIT/LINK ring, its WAIT length and the flushes
  follow galcore as the repo records (`cmdbuf.rs:1-29`). Mesa etnaviv and the
  Linux etnaviv driver were not re-read in this item: there is no Mesa tree
  here and a clone was not possible. They do not change the conclusion. Under Linux
  the page flip belongs to KMS and the compositor, not to the GL driver; the
  published i.MX8MP scores (`driver/docs/glmark2.md:233-239`) are off-screen
  or Wayland runs, not a GL driver that spins on the LCDIF.

## Proposed fix (F1 and F2 applied: see "Fixes applied")

F1 (the floor; board layer only). In
`driver/zephyr/lib/board/imx8mp/board_imx8mp.c` `board_show`: when the LCDIF
is already on and scans `scan` (always true for the one-buffer window after
the first show), return 0 without `imx8mp_display_set_fb`. Keep the first
`imx8mp_display_on`. The picture does not change: the window is one buffer
that the LCDIF already scans, and the GPU already draws into the scanned
buffer today. This makes the i.MX8MP present like the AM62P (3 us) and the
Pi 4 (a pan without a VSYNC wait).

F2 (keep pacing for the applications that want it). Define
`yakogl_zephyr_wait_vblank` for the i.MX8MP: wait for the LCDIF's next
VSYNC (the LCDIF driver already has `yakogl_imx8mp_lcdif_wait_vblank`,
`driver/drivers/display/display_yakogl_imx8mp_lcdif.c:178-188`; or a wait on
the shadow-load bit). Then EGL's default swap interval 1 (`src/gl/egl.rs:276`)
paces Slint and the cluster through EGL, and glmark2's interval 0 is honoured.
Check the Slint and cluster rows after F2:
today they are paced by the wait in `board_show`.

F3 (after F1, smaller). A fence wait woken by the interrupt instead of the
1 ms sleep (the ISR already runs per event, `irq.rs:53-92`), or a finer
poll, as the AM62P did.

F4 (after F1, large). The GPU fill cost (candidate 2): a tiled colour target
with tile status and fast clear, and early depth. That is back-end work in
`src/vivante/**`.

Expected effect of F1: light scenes stop at 58 FPS and run at their GPU or
CPU cost. A scene now at two periods also speeds up (to its own cost, no
longer rounded up to 34.0 ms). The score should rise well above 58. The
exact value is not predicted: it is the sum of candidates 2 and 3, which the
record below measures.

## Fixes applied (F1, F2; 2026-10-14, item check-steps)

Board layer only. `src/osal/zephyr.c`, `driver/rust/**` and the samples are
unchanged.

- F1, `driver/zephyr/lib/board/imx8mp/board_imx8mp.c:158-179`. `board_show`
  keeps the address the LCDIF scans in `s_scan` (`:40-44`). It is set by the
  first `imx8mp_display_on` and by every `imx8mp_display_set_fb` that
  latched. Once the display is on, a show of the buffer in `s_scan` returns
  0 at once. This is every show of the one-buffer window after the first, in
  the scan-in-place and view paths and in the copy path (always `screen`).
  The first show still calls `imx8mp_display_on`. A buffer the LCDIF does
  not scan yet still goes through `imx8mp_display_set_fb` and its wait. A
  failed latch leaves `s_scan` as it was, so the next show tries again.
- F2, `board_imx8mp.c:186-229`. A strong `yakogl_zephyr_wait_vblank`, the
  i.MX8MP's answer to the hook whose weak default is `src/osal/zephyr.c:603`.
  It is not called yet: see "F2 is not reached yet" below. It calls
  `imx8mp_display_set_fb(s_scan)`: it latches the address already scanned
  again, so the picture does not change. It returns when the LCDIF clears
  `SHADOW_LOAD_EN` at its next VSYNC. This is the wait `board_show` did
  every frame before. Once it is called, it runs only as often as the swap
  interval asks.
  - Why not the interrupt (`yakogl_imx8mp_lcdif_wait_vblank`,
    `driver/drivers/display/display_yakogl_imx8mp_lcdif.c:178-188`): that
    device exists only with `CONFIG_SAFEUI_DISPLAY_IMX8MP_LCDIF_RUST`. That
    option is default n and not board-proven (`driver/zephyr/Kconfig:317-330`),
    and no sample or image script sets it. Its interrupt is enabled only by
    a cold bring-up (`yakogl_imx8mp_lcdif_enable`), not on an adopted
    layer. The common-set images use `driver/src/display/imx8mp_lcdif.c`,
    where an interrupt-based wait would answer -ENOTSUP, and Slint and the
    cluster would lose their pacing. The shadow-load wait works on both
    LCDIF builds.
  - The timeout. `timeout_us == 0` gives -ETIMEDOUT at once. Otherwise the
    wait cannot be cut short: it ends at the VSYNC, or at the LCDIF
    driver's own bound (`SHADOW_LOAD_TIMEOUT_US` 36 ms,
    `driver/src/display/imx8mp_lcdif.c:94`; `FRAME_WAIT_US` 83 ms,
    `driver/rust/src/ip/lcdifv3.rs:118`), which is -ETIMEDOUT. A VSYNC that
    came later than `timeout_us` is also -ETIMEDOUT. EGL asks for 100 ms
    (`src/gl/egl.rs:2132`), more than a 17.0 ms period, so the return is 0.
  - -ENOTSUP comes back before the first show (the display is not on yet;
    the first swap comes before the first `board_show`), with the A/B
    option below, and when `set_fb` fails for another reason. EGL's loop
    stops at the first -ENOTSUP (`src/gl/egl.rs:2131-2135`).
- A/B, `driver/zephyr/lib/board/Kconfig:101-113`:
  `CONFIG_YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC` (default n). With `y`,
  `board_show` calls `imx8mp_display_set_fb` on every show, and
  `yakogl_zephyr_wait_vblank` answers -ENOTSUP. That is exactly the code
  before F1 and F2.

The effect on each application:

- glmark2: swap interval 0. The sample compiles upstream's
  `gl-state-egl.cpp` unchanged (`driver/samples/zephyr/glmark2/CMakeLists.txt:132`),
  and its only patch is `libmatrix-util-zephyr.patch`. Upstream
  `src/gl-state-egl.cpp:426-427` at `22c527cb` calls `eglSwapInterval(dpy,
  0)` unless `--swap-mode fifo`. The default `CONFIG_GLMARK2_ARGS` is empty
  (`driver/samples/zephyr/glmark2/Kconfig:17-19`). Read from the deps
  cache's glmark2 checkout for this item. So no swap waits and no show
  waits: the frame runs at its GPU and CPU cost.
- Slint and the cluster keep EGL's default interval 1
  (`src/gl/egl.rs:276`). With F2 reached, that is one VSYNC a swap, as
  before, now in `eglSwapBuffers` instead of `board_show`. Today (F2 not
  reached), no swap waits: they run unpaced. Their output does not change,
  because the window is the one scanned buffer either way. Only the rate
  changes.
- gl_test (GT) also keeps interval 1, so it is the same as Slint.

### F2 is not reached yet (found in this item)

The i.MX8MP image does not link `src/osal/zephyr.c`. The GC7000 build takes
`src/osal/zephyr_gc.c` as its OSAL instead (`zephyr/CMakeLists.txt:86-90`).
Its ops table, `osal_zephyr_ops` (`src/osal/zephyr_gc.c:558-577`), has no
`.wait_vblank`, so `osal::wait_vblank` answers -ENOTSUP from the NULL op
(`src/osal/mod.rs:469-474`). EGL's loop stops there. The link map of the GM
image built below lists `.text.yakogl_zephyr_wait_vblank` of
`libyakogl_board.a(board_imx8mp.c.obj)` under "Discarded input sections":
nothing references it. The same holds for the code before F1. "The frame
path" above credits the -ENOTSUP to zephyr.c's weak default; it actually
comes from the missing op, with the same effect.

Wiring it in is a change to `src/` and outside this item's scope. In
`src/osal/zephyr_gc.c`, add a `yakogl_zephyr_wait_vblank` declaration (with
a `__weak` -ENOTSUP default there, because zephyr.c is not linked) and
`.wait_vblank = z_wait_vblank` calling it, as `src/osal/zephyr.c:603-612`
does. Until then:

- glmark2 gets what it asks for (interval 0, no wait). F1 alone removes the
  floor.
- Slint, the cluster and gl_test run unpaced on the default build. To keep
  them paced as before, build those images with
  `CONFIG_YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC=y`.

## Host evidence (2026-10-14)

- The Zephyr build of the common GM row from this change:
  `tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt build/ck-8f7d396d GM`
  on the deps cache's shared workspace (Zephyr SDK 1.0.1). It printed `GM rc=0
  sha256=5f0c0a911b7f22c4c7afda8a5b8d239cd4d53f98288479f55ce9977a16a56a18
  build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=8f7d396d`. Its `.config`
  has `CONFIG_SAFEUI_DISPLAY_IMX8MP=y`, `# CONFIG_SAFEUI_DISPLAY_IMX8MP_LCDIF_RUST
  is not set` and `# CONFIG_YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC is not
  set`. The later commits change only comments and this doc. Built again
  from `9562c58a` (the same command, `build/ck-head`): `GM rc=0
  sha256=0c4d71d27f5df3b66fcc158869e2534115564d1efe8016434c752d853d28fba9
  build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=9562c58a`, the same
  `.config` lines, and `.text.yakogl_zephyr_wait_vblank` again under
  "Discarded input sections" in `zephyr.map`.
- The A/B arm compiled: the build's own compile command for
  `board_imx8mp.c` (aarch64-zephyr-elf-gcc, from `compile_commands.json`),
  run again with `-fsyntax-only -Wall -Wextra -Wno-unused-parameter`, once
  as built and once with `-DCONFIG_YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC=1`.
  Both rc=0 with no diagnostics. The full image of the A/B arm, from
  `9562c58a`: step 2's `gm GM-waits -DCONFIG_YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC=y`
  west build (into `build/ck-head/GM-waits`), rc=0, `.config` with
  `CONFIG_YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC=y`, `zephyr.bin` sha256
  `b394bd1641c57cb727a68e9403034968b5eee7bf4c55ac658292da23f3f5dcdc`.
- `tools/check.sh --quick` on this tree (system76): 116 ok, 1 failed. The
  failure is from the environment: "v3d tex_check, cl_golden, job_check
  (MESA_V3D_TREE is not a Mesa tree)", because that machine's
  `MESA_V3D_TREE` has no `build-v3d`. That row covers the V3D only.

## The frame-breakdown record (design, not built)

One glmark2 image on the i.MX8MP, one scene per image
(`CONFIG_GLMARK2_ARGS="-b <scene>"`), reports one page of a kind 12 PERF
record instead of its GLMARK2 record when the selector asks for it. This is
the existing form (`driver/zephyr/lib/board/imx8mp/gc_result.h:277-283`,
`YK_RECORD_PERF_SEL` 0x1000 in `driver/zephyr/lib/board/include/yakogl_record.h:99`;
decoded by `yakogl-tools record` as "experiment e page p: v0 .. v5",
`driver/rust/src/board/record.rs:1190-1201`). The SNVS keeps only the last
record, so one run gives one page.

- Selector: `0x1000 | page` (experiment 0). Read once with `board_selector()`
  (`yakogl_board.h:204`) before the benchmark loop. Without a selector the run
  ends with the GLMARK2 record as today.
- w0 = `0x594b0c00 | page`; w1..w3 = six 16-bit values v0..v5, v0 in w1's low
  half (`w1 = v1 << 16 | v0`, `w2 = v3 << 16 | v2`, `w3 = v5 << 16 | v4`).
- Times are microseconds per frame, means over the scene's frames,
  saturating at 0xffff. Counts saturate at 0xffff.
- The scene's counters reset where `s_cost` resets today
  (`RecordingLoop::log_scene_info`, `main.cpp:402-416`); the page is computed
  at `log_scene_result` (`main.cpp:418-422`).

| Page | v0 | v1 | v2 | v3 | v4 | v5 |
|---|---|---|---|---|---|---|
| 0: the frame | frames | frame (t2 - t0) | gl (t1 - t0) | swap | show | frame to frame (t0 to the next t0) |
| 1: the swap | wait us | waits per 100 frames | `gc_render_wait` sleeps per 100 frames | GPU busy us | kicks per 100 frames | submit us |
| 2: the display | frames under 1.5 T | frames in [1.5 T, 2.5 T) | frames at 2.5 T or more | T, us | frames with show >= 1000 us | `CONFIG_SYS_CLOCK_TICKS_PER_SEC` |
| 3: the GPU | HI total kcycles | HI idle kcycles | KiB read | KiB written | core MHz | 0 |

Where each value comes from:

- t0, t1, t2: the existing stamps in `RecordingLoop::draw`
  (`main.cpp:381-390`, `k_cycle_get_64`). swap: `GLStateTimed::swap`
  (`main.cpp:243-247`). show: `NativeStateBoard::flip` (`main.cpp:211-216`).
  Frame to frame: new, the difference of consecutive t0 (it also holds any
  time outside `draw()`).
- Page 1: differences over the scene of `osal_gc_wait_stats` (wait us, waits;
  `src/osal/zephyr_gc.c:473-483`), `gc_render_perf` (`wait_sleeps`,
  `gpu_busy_ns`, `kicks_done`; `driver/src/vivante/gc_render.c:623-646`) and
  `osal_gc_gpu_stats` (submit us; `zephyr_gc.c:460-471`). GPU busy is the
  union of submit-to-retire spans (`irq.rs:80-87`).
- Page 2: T is measured at scene start: four `imx8mp_display_set_fb(scan)`
  calls (each returns at a VSYNC), timed, the last three averaged. After F1
  `board_show` no longer waits, but this direct call still does. The bins use
  the frame-to-frame time of each frame. The tick rate is the Kconfig value.
- Page 3: `HI_PROFILE_TOTAL_CYCLES`, `_IDLE_CYCLES`, `_READ_BYTES8`,
  `_WRITE_BYTES8` (`gc_render.c:638-641`). They are 32-bit and wrap in about
  5 s at 800 MHz, so read them every frame and add the differences (not one
  difference over the 10 s scene). Bytes = BYTES8 x 8. Core MHz = total
  cycles / frame-to-frame us.

Code owner for a later item: `driver/samples/zephyr/glmark2/src/main.cpp`
(and nothing in `src/vivante/**`). It needs a host check that packs a known
page into known words (as `record.rs` tests do for other kinds).

## Board recipe

Environment, as in `tools/cts/common-images.sh:47-58`: `ZEPHYR_WS`,
`WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`, `GLMARK2_SRC` (glmark2 at
`22c527cb0556f3a1ac4445aaa52cc532760928d5`), `IMX8MP_OUT`; `SW` = this
checkout; `<rec>` = a record directory. The board is reached as
`tools/board.sh` describes (U-Boot TFTP by default).

### Step 1: confirm candidate 1 with today's code (no change)

Two images of the glmark2 sample, one scene (`clear`: only glClear and the
swap), with and without the view:

```sh
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr PATH=$WEST_VENV/bin:$PATH
cd "$ZEPHYR_WS"
MODS=$("$SW/tools/cts/zephyr-modules.sh" "$ZEPHYR_WS")
for arm in view copy; do
  V=; [ $arm = view ] && V=-DCONFIG_YAKOGL_BOARD_SCANOUT_VIEW=y
  west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d "$IMX8MP_OUT/gm-clear-$arm" \
    "$SW/driver/samples/zephyr/glmark2" -- "-DZEPHYR_MODULES=$MODS" \
    -DYAKOGL_DIR="$SW" -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y -DGLMARK2_SRC="$GLMARK2_SRC" \
    '-DCONFIG_GLMARK2_ARGS="-b clear"' $V
  "$SW/tools/board.sh" imx8mp run "$IMX8MP_OUT/gm-clear-$arm/zephyr/zephyr.bin" <rec>/floor clear-$arm
done
```

Expected (kind GLMARK2, `driver/rust/src/board/record.rs:147-157`, `:658-686`;
`ss` is the sequence byte):

| Arm | Record | Reading |
|---|---|---|
| view | `594b12ss fc3f003a 2104003f 03200258` | score 58 (0x3a), 1 ran, 0 failed, 0 unsupported, 800x600: locked to 58.76 Hz |
| copy | `594b12ss fc3f003a 2104003f 03200258` or `... fc3f001d ...` | 58 if the copy fits in one period with the clear, 29 if not; never above 58 |

Verdict lines: `verdict=done`, `result=record:GLMARK2`, `resets=0`.

- view = 58: candidate 1 is confirmed (a clear of 800x600 is a few ms of
  GPU; only the VSYNC wait can make it exactly one period).
- view = 59 or more: candidate 1 is wrong. Go to step 3 (the record) before
  any fix.
- view = 29 or less: one clear takes more than 17 ms. Then candidate 2 or 3
  is large; step 3 page 1 and page 2 say which.

### Step 2: F1 and F2 on the board (the next i.MX8MP session)

Step 2 can run without step 1. Its A/B arm (`GM-waits`) is the old
behaviour, built from the same tree. The environment is as in
`ab43a20c5:docs/board-sessions/imx8mp-common-int.md` ("Commands"): `ZEPHYR_WS`,
`WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`, `GLMARK2_SRC`, `SLINT_NOSTD_DIR`,
`NOSTD_PATCH_DIR`, `AGL_SLINT_CLUSTER_DIR`, `RUSTUP_TOOLCHAIN=nightly`.
`BOARD_HOST` is the leased board's ssh login. `BOARD_BOOT=uboot-mmc` as in
that session. Run from the checkout's top with a tree that has F1 and F2.

```sh
SW=$PWD
IMG=build/floor-f1f2-$(git rev-parse --short=8 HEAD)
REC=records/imx8mp/floor-f1f2-$(git rev-parse --short=8 HEAD)
# 1. the common rows with F1+F2: glmark2 (default list), Slint, Slint view, cluster
tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt "$IMG" GM SO SV CL
# 2. the extra glmark2 images: the A/B arm (old wait) and two light scenes
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr PATH=$WEST_VENV/bin:$PATH
MODS=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
gm() { # gm <dir> <cmake args...>
  d=$SW/$IMG/$1; shift
  (cd "$ZEPHYR_WS" && west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d "$d" \
    "$SW/driver/samples/zephyr/glmark2" -- "-DZEPHYR_MODULES=$MODS" \
    -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y "-DGLMARK2_SRC=$GLMARK2_SRC" "$@")
}
gm GM-waits -DCONFIG_YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC=y
gm GM-clear '-DCONFIG_GLMARK2_ARGS="-b clear"'
gm GM-clear-waits '-DCONFIG_GLMARK2_ARGS="-b clear"' -DCONFIG_YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC=y
gm GM-build '-DCONFIG_GLMARK2_ARGS="-b build:use-vbo=true"'
# 3. run, one image at a time (board.sh: "run <image> <logdir> [name]")
for r in GM-clear GM-clear-waits GM-build GM GM-waits SO SV CL; do
  BOARD_HOST=<user@host> BOARD_BOOT=uboot-mmc \
    tools/board.sh imx8mp run "$IMG/$r/zephyr/zephyr.bin" "$REC" "$r"
done
```

As in that session, if the sandbox refuses a command line that starts with
`BOARD_HOST=...`, export the two variables in a small wrapper. Each run
prints one `board.sh imx8mp <name> rc=... verdict=...` line. The i.MX8MP has
no console capture, so the record is the only result. A GLMARK2 record's
second word is `fc3f` and then the score in hex (`fc3f002b` = 43, as in
`imx8mp-common-int.md`).

Expected:

| Run | Expected record / verdict | Reading |
|---|---|---|
| GM-clear | `verdict=done result=record:GLMARK2`, second word `fc3fXXXX` with XXXX above `003a` | `clear` above 58 FPS. Before F1, every scene stopped at 58 (one period). Expected a few hundred: a full-screen clear is about 1.9 ms of GPU (candidate 2). |
| GM-clear-waits | second word `fc3f003a` (58), or `fc3f001d` (29) | The A/B arm: locked to the panel, never above 58. |
| GM-build | second word above `fc3f003a` | A light model scene above 58 FPS. |
| GM | `verdict=done`, second word above `fc3f003a`, `scenes ran 33, failed 0` in the tally | The default list. The score is no longer capped near 58, and is well above the 43 of `imx8mp-common-int.md`. Heavy scenes still run at their GPU cost; see candidate 2. |
| GM-waits | second word near `fc3f002b` (43; 37-46 earlier) | The old behaviour from the same tree, which confirms the A/B. |
| SO, SV | `verdict=done result=record:PROBES record=594b0601,08420862,00210842,c5920883` | Probes as before. Unpaced while F2 is not reached (no swap waits); paced at 58.76 Hz once `zephyr_gc.c` calls the hook. |
| CL | `verdict=done result=record:PROBES record=594b0601,10c410c4,1905f7bf,b1076d5f` | Probes as before; paced as SO is. |

What the records cannot show: Slint and the cluster write no frame count,
and the i.MX8MP has no console, so their pacing is not in a record. A
person at the panel can check it.

- While F2 is not reached, SO, SV and CL from the default build swap
  without waiting. The probes must still pass, with the same records. If a
  probe differs from the table, the unpaced present changed what is
  scanned: rerun that row built with the A/B option (`gm`-style, or
  `west build ... -DCONFIG_YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC=y` on the
  slint or cluster sample) and compare.
- Once `zephyr_gc.c` calls `yakogl_zephyr_wait_vblank`, the motion in SO,
  SV and CL should have the speed of the earlier session, without tearing.
  Repeat these three rows then.

A GT run of the common set (`tools/cts/common-images.sh ... GT`) should
still fail only at case 44. gl_test's result does not depend on pacing.

Reject F1 if: GM-clear is at 58 or below while GM-clear-waits is also at 58.
The frame is then still locked, and something other than `board_show` waits.
Step 3's page 0 (show us) and page 2 (frames per bin) say where.

### Step 3: the frame-breakdown record (after it is built)

The `view` image of step 1 (or step 2), four runs, one page each:

```sh
for p in 0 1 2 3; do
  SEL=$((0x1000 | p)) "$SW/tools/board.sh" imx8mp run \
    "$IMX8MP_OUT/gm-clear-view/zephyr/zephyr.bin" <rec>/floor clear-view-p$p
done
```

Expected before F1 (`clear`, view). Values are approximate where marked:

| Page | Record | Reading |
|---|---|---|
| 0 | `594b0c00 427a024b <swap><gl> 427a<show>` | about 587 frames (10 s x 58.76; +-2), frame and frame-to-frame about 17 018 us (0x427a, +-1 %), gl + swap small, show = about 17 018 - gl - swap (at least 0x32c8, 13 000 us) |
| 1 | `594b0c01 ...` | GPU busy a few ms (expected 1-5 ms); waits per 100 frames low (the previous frame is done during the show wait); sleeps per 100 frames near 0 |
| 2 | `594b0c02 0000024b 00000000 ....427a 2710....` | all frames under 1.5 T (v0 = frames, v1 = v2 = 0), T about 17 018 (0x427a), v4 = frames (show >= 1 ms every frame), ticks 10 000 (0x2710) expected |
| 3 | `594b0c03 ...` | core MHz about 800 (0x0320); idle kcycles most of total |

Expected after F1: page 0 show under 50 us (v4 < 0x0032), frame =
frame-to-frame = gl + swap, frames = 10 s / frame; page 2 v4 = 0. The
GPU busy and wait values then say whether candidate 2 (GPU busy close to the
frame) or candidate 3 (sleeps per frame high, wait well above GPU busy) is
the next limit.

If page 2 v5 is not 10 000, the image was built with a different tick rate
than the pinned Zephyr's default (candidate 3): at 100, each sleep would be
10-20 ms, and the poll would be fixed before or with F1.

## What this does not show

- F1 and F2 are built for the board, not run on it. The Zephyr builds of
  the glmark2 image, with and without the A/B option, are host evidence
  that the code compiles and links, and that the strong
  `yakogl_zephyr_wait_vblank` replaces the weak one. They say nothing about
  timing.
- No board run was made. The per-scene split (18 / 10 / 9 scenes at one
  period) is inferred from the scores, not measured.
- The Zephyr tick rate was read from the pinned Zephyr's Kconfig files, not
  from a built image's `.config` (no Zephyr workspace on this machine).
- Mesa etnaviv was not re-read (no Mesa tree here, and cloning was not
  possible); nothing in the conclusion rests on it.
- The 46 (view) run has no row in RESULTS.md; its number is ab43a20c5:tools/cts/log/status-2026-09-25-to-10-07.md:88's.
