<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP: why glmark2 frames take "about 24 ms" (host investigation)

**Status (2026-10-17): F1 and F2 are in the tree a second time, as a
latch thread whose default keeps the old timing; not yet run on a board.
F2 is wired in (item imx8mp-wait-vblank): the GC7000's OSAL has
`.wait_vblank`, and a frame waits once per period, not twice ("Waits per
swap").** The cause below is a prediction from the code; the board recipe
has not run.

- The first F1 (#793, merged 2026-10-16) let `board_show` return at once
  when the LCDIF already scanned the shown buffer, in every path. The owner
  saw glmark2 draw wrong on the board the same day and reverted it
  (`faee8812`, #923). Why the picture broke is not confirmed: see
  "Proposed fix" below. That form of F1 is not coming back.
- F1 now: the LCDIF's latch (the VSYNC wait inside
  `imx8mp_display_set_fb`) runs on a thread of the board layer
  (`driver/zephyr/lib/board/imx8mp/board_imx8mp.c:46-227`, `:300-379`).
  `CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS`
  (`driver/zephyr/lib/board/Kconfig:101-128`) says how many panel-sized
  screen buffers a COPIED window (glmark2's 800x600, Slint's 1024x600
  without the view) is shown through. **The default is 1: every show waits
  for its latch, the timing of `faee8812` and of every board result so
  far.** With 3, a copied window's show returns once the copy is done; with
  2 it is paced at the panel's rate. A window scanned in place (the panel's
  size, the cluster's halves, a view) waits whatever the value. 2 and 3 are
  the opt-in arms of step 2 of the board recipe, not the default, until a
  board run has checked the picture.
- F2: a strong `yakogl_zephyr_wait_vblank`
  (`driver/zephyr/lib/board/imx8mp/board_imx8mp.c`), called by the GC7000's
  OSAL (`src/osal/zephyr_gc.c`, `.wait_vblank`). Its first call after a show
  waits for that show's latch; a further call waits for the next latch. So
  a show that already waited (in place, or copied with 1 buffer) is not
  followed by a second wait in the swap. With the default 1 every row keeps
  its timing; with 2 or 3, Slint without the view is paced at the panel's
  rate by the swap ("F2 wired in", "Waits per swap").

See "Fixes applied" and "Board recipe", step 2 (the next i.MX8MP board
session). Step 3's frame-breakdown record is built (2026-10-18, "The
frame-breakdown record") and ran on the board ("Step 3 result": at 3
buffers a light frame's 14.9 to 15.6 ms are all in the swap, about 2 to
3 ms of GPU and 12.3 to 12.9 ms of the present path, no VSYNC lock). The sections up to "Proposed fix" describe the code before F1
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

## Proposed fix (2026-10-08; what became of it: "Fixes applied")

F1 (the floor; board layer only), as first proposed. In
`driver/zephyr/lib/board/imx8mp/board_imx8mp.c` `board_show`: when the LCDIF
is already on and scans `scan` (always true for the one-buffer window after
the first show), return 0 without `imx8mp_display_set_fb`. Keep the first
`imx8mp_display_on`. The picture does not change: the window is one buffer
that the LCDIF already scans, and the GPU already draws into the scanned
buffer today. This makes the i.MX8MP present like the AM62P (3 us) and the
Pi 4 (a pan without a VSYNC wait).

The board refused this form (#923, `faee8812`): #793 merged it on
2026-10-16 with a green gate and nothing run on a board, and the owner saw
glmark2 draw wrong on the i.MX8MP the same day, where the common set had
read 5 of 5 rows green at `ab43a20c5`. **Why the picture broke is not
confirmed.** The revert's reading, "with one buffer scanned in place, a
show that returns at once lets the GPU draw the next frame into the buffer
the LCDIF is scanning", holds for a window scanned in place (the panel's
size, a view). But the common GM row's 800x600 glmark2 window is COPIED
into a panel-sized screen buffer, and at #793 that copy still went into the
one buffer the LCDIF scanned, every frame, at the frame's rate instead of
once a period. So for glmark2 the candidates are (a) the copy itself,
written over the buffer being scanned several times a period, and (b)
something else the removed wait hid (the panel console's own writes, the
GPU reading the window while it is copied, a latch the LCDIF needs). No
record tells them apart: the i.MX8MP has no console, and the broken picture
was seen, not captured. What is certain is only that the LCDIF must not
scan a buffer the GPU or the copy writes into while it scans. The second
form below keeps every path that scans in place waiting, and lets a copied
window skip the wait only when the copy goes into a buffer the LCDIF does
not scan; whether that keeps the picture whole is the hypothesis step 2 of
the board recipe checks, and the default stays the old timing until it has.

F2 (keep pacing for the applications that want it). Define
`yakogl_zephyr_wait_vblank` for the i.MX8MP: wait for the LCDIF's next
VSYNC (the LCDIF driver already has `yakogl_imx8mp_lcdif_wait_vblank`,
`driver/drivers/display/display_yakogl_imx8mp_lcdif.c:178-188`; or a wait on
the shadow-load bit). Then EGL's default swap interval 1 (`src/gl/egl.rs:283`)
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

## Fixes applied (F1, F2; 2026-10-17, item check-steps, second form)

Board layer only: `driver/zephyr/lib/board/imx8mp/board_imx8mp.c` and the
option in `driver/zephyr/lib/board/Kconfig`. `src/osal/zephyr.c`,
`driver/rust/**`, `driver/src/**` and the samples are unchanged. The first
form (#793) was reverted by `faee8812` after the board showed a broken
glmark2 picture (#923, "Proposed fix" above).

- F1, the latch thread
  (`driver/zephyr/lib/board/imx8mp/board_imx8mp.c:46-227`; the `:N` ranges
  in this section are that file's). `imx8mp_display_set_fb`
  (write the address, set `SHADOW_LOAD_EN`, spin until the LCDIF clears it
  at the next VSYNC) now runs on a thread the first `board_show` creates
  (`:360-370`, lowest application priority; on another of the four A53s
  with `CONFIG_SMP=y`). A show asks the thread for a latch (`ask`, `:146-157`)
  and either waits for it (`latched`, `:160-176`) or returns.
  - A window scanned IN PLACE (`:318-326`: the panel's size, the cluster's
    two halves, a `CONFIG_YAKOGL_BOARD_SCANOUT_VIEW` view) asks for its
    latch and waits for it, as it did before #793: it is the buffer the
    GPU draws into next. The SV row and the CL row are unchanged in timing.
  - A COPIED window (`:327-359`) is copied into one of
    `CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS` panel-sized carve-out
    buffers (`screens_alloc`, `:179-206`). With the default 1 that is the
    one buffer the LCDIF scans (`:345-350`), and the show waits for its
    latch (`:358`, `:372-374`): the old timing, one panel period under
    every frame. With 2 or 3 the buffer is one the LCDIF neither scans nor
    is latching (`screen_pick`, `:209-226`), and the show returns 0 once
    the copy is done; with 2 a frame faster than the panel first waits for
    the latch in flight (`:335-343`, paced at the panel's rate). A frame the
    thread has not latched yet is replaced by the next one (the newest
    wins). The cost of a show is then the copy (the same copy as before;
    candidate 4). This is the path of the common GM row (glmark2's 800x600
    on the 1280x800 panel, `tools/cts/common-tree.txt`) and of the SO row
    (Slint 1024x600 without the view).
  - The first show of a run still calls `imx8mp_display_on` on the showing
    thread (`:360-370`). A latch that failed (the display driver's bound,
    -ETIMEDOUT) leaves the LCDIF on the buffer it had, which is free
    again (`:128-141`).
- F2, `driver/zephyr/lib/board/imx8mp/board_imx8mp.c:395-421`. A strong
  `yakogl_zephyr_wait_vblank`, the
  i.MX8MP's answer to the hook whose weak default is `src/osal/zephyr.c:603`.
  As first written it asked the thread for a latch of the address a frame
  asked for, or of the shown address again when no frame waits (the picture
  does not change), and returned when that latch completed: the LCDIF's
  next VSYNC. It was not called then ("F2 wired in" below has what changed
  when it was wired in). It runs only as often as the swap interval asks.
  - Why not the interrupt (`yakogl_imx8mp_lcdif_wait_vblank`,
    `driver/drivers/display/display_yakogl_imx8mp_lcdif.c:178-188`): that
    device exists only with `CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_LCDIF_RUST`. That
    option is default n and not board-proven (`driver/zephyr/Kconfig:317-330`),
    and no sample or image script sets it. Its interrupt is enabled only by
    a cold bring-up (`yakogl_imx8mp_lcdif_enable`), not on an adopted
    layer. The common-set images use `driver/src/display/imx8mp_lcdif.c`,
    where an interrupt-based wait would answer -ENOTSUP. The shadow-load
    wait works on both LCDIF builds.
  - The timeout. `timeout_us == 0` returns -ETIMEDOUT at once; a negative
    value waits for ever; otherwise the wait ends at the latch or at the
    deadline (`K_TIMEOUT_ABS_TICKS`, `:409-414`), -ETIMEDOUT. The latch
    itself is bounded by the display driver (`SHADOW_LOAD_TIMEOUT_US` 36 ms,
    `driver/src/display/imx8mp_lcdif.c:94`; `FRAME_WAIT_US` 83 ms,
    `driver/rust/src/ip/lcdifv3.rs:118`), also -ETIMEDOUT. EGL asks for
    100 ms (`src/gl/egl.rs:2153`), more than a 17.0 ms period, so the
    return is 0.
  - -ENOTSUP comes back before the first show (nothing is on the panel yet;
    the first swap comes before the first `board_show`) and when the latch
    failed for another reason. EGL's loop stops at the first -ENOTSUP
    (`src/gl/egl.rs:2152-2156`).
- The option, `driver/zephyr/lib/board/Kconfig:101-128`:
  `CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS`, **default 1**, range 1 to 3.
  - 1 (the default): the copy goes into the one buffer on the panel and
    every show waits for its latch: the timing of `faee8812` and of every
    board result so far (the wait moved to the thread, the show waits for
    it). Nothing runs unpaced. This stays the default until step 2 has
    checked the picture.
  - 3: a copied window's show returns once the copy is done (one buffer on
    the panel, one on its way, one to copy into). The arm whose glmark2
    light scenes should pass 58 FPS, and whose picture the board run must
    check (#923).
  - 2: a copied frame faster than the panel waits for the latch in flight:
    paced at the panel's rate, no per-frame wait below it.
  - A window scanned in place waits whatever the value.

The effect on each application:

- With the default 1, every row is timed as before this change. The rest of
  this list is the 2 and 3 arms.
- glmark2: swap interval 0. The sample compiles upstream's
  `gl-state-egl.cpp` unchanged (`driver/samples/zephyr/glmark2/CMakeLists.txt:132`),
  and its only patch is `libmatrix-util-zephyr.patch`. Upstream
  `src/gl-state-egl.cpp:426-427` at `22c527cb` calls `eglSwapInterval(dpy,
  0)` unless `--swap-mode fifo`; `GLMARK2_USE_NULL` is not defined by the
  sample. The default `CONFIG_GLMARK2_ARGS` is empty
  (`driver/samples/zephyr/glmark2/Kconfig:17-19`). Read again from the deps
  cache's glmark2 checkout (`22c527cb`) for this item. The GM row copies
  (800x600 on 1280x800, no view): with 3 buffers no swap waits and no show
  waits; the frame runs at its GPU and CPU cost plus the copy.
- Slint without the view (SO) copies too: with 3 buffers its swap
  (interval 1, F2) waits for the latch of the frame shown before, so it is
  paced at the panel's rate, every shown frame complete; with 2 it is paced
  at the panel's rate by the show or the swap, whichever comes first. Slint
  with the view (SV) is scanned in place: paced by the show's latch wait,
  as before, whatever the value.
- The cluster (CL) shows two panel-sized halves in place
  (`driver/samples/zephyr/cluster/boards/verdin_imx8mp_mimx8ml8_a53.conf`):
  each show latches the other half and waits, as before. Unchanged.
- gl_test (GT, `gc_gl_test`) keeps EGL's default interval 1
  (`src/gl/egl.rs:283`); its result does not depend on pacing.
- F2 wired in naively (`.wait_vblank` asking the thread for one more latch
  at every call) would have made interval 1 one VSYNC a swap in
  `eglSwapBuffers` for Slint and the cluster, and the in-place show a second
  one for its own latch: two periods a frame, SV and CL halved. The host
  model measured exactly that ("Waits per swap"). The wiring therefore
  counts a show's latch as the swap's period.

### F2 wired in (2026-10-17, item imx8mp-wait-vblank)

Why it was not reached before: the i.MX8MP image does not link
`src/osal/zephyr.c`. The GC7000 build takes `src/osal/zephyr_gc.c` as its
OSAL instead (`zephyr/CMakeLists.txt:86-90`), and its ops table,
`osal_zephyr_ops`, had no `.wait_vblank`, so `osal::wait_vblank` answered
-ENOTSUP from the NULL op (`src/osal/mod.rs`, `wait_vblank`). EGL's loop
stopped there, and the link map of the GM image listed
`.text.yakogl_zephyr_wait_vblank` under "Discarded input sections". "The
frame path" above credits the -ENOTSUP to zephyr.c's weak default; it came
from the missing op, with the same effect.

What changed:

- `src/osal/zephyr_gc.c`: a `__weak` `yakogl_zephyr_wait_vblank` that
  returns -ENOTSUP (zephyr.c, which has the other weak default, is not
  linked in a GC7000 build), and `.wait_vblank = z_wait_vblank` calling it,
  as zephyr.c does. An image without the board layer (gc_gl_test) keeps
  -ENOTSUP: presentation stays immediate there.
- `driver/zephyr/lib/board/imx8mp/board_imx8mp.c`: every show that asks the
  thread for a latch records its request (`s_show_seq`) and sets
  `s_show_vblank`. The first `yakogl_zephyr_wait_vblank` after it waits for
  that latch instead of asking for one more; a later call (interval 2 and
  up, or a swap with no show since) asks for the next VSYNC as before. The
  flag is cleared once that latch completed (not when the caller's timeout
  ran out first). `board_show` itself does not change: every show waits
  exactly as before, so the picture rules of "The latch" stay as they were.

Why one wait is enough for interval 1. The swap's period is counted from
the frame on the panel. The shown frame reaches the panel at its latch; the
next show's latch can only come at a later VSYNC, so the frame stays on the
panel for at least one period, which is what interval 1 asks
(EGL 1.5 3.10.3, "the minimum number of video frame periods"). A show that
already waited for its latch leaves nothing for the swap to wait for.

### Waits per swap (2026-10-17)

The panel periods a frame waits, as "show + swap" (the show's latch wait,
then the swap's `.wait_vblank`, in the order each sample calls them), with
the draw shorter than a period. An application at the panel's rate is 1;
"u" is unpaced (the frame's own cost). The order of the calls:

- Slint (`examples/slint/src/main.c`, EGL_SINGLE_BUFFER,
  interval 1): draw, swap, show.
- The cluster (`driver/samples/zephyr/cluster/src/main.c`, two halves,
  EGL_SINGLE_BUFFER and PIPELINED, interval 1): draw N+1 (recorded), show
  N, swap N+1.
- glmark2 (EGL_BACK_BUFFER, interval 0): draw, swap (shows the frame before
  through `osal::present` = `board_show`, then the interval loop). "fifo"
  is the same window at interval 1 (glmark2 `--swap-mode fifo`; dEQP's
  window surfaces).

| Row | Window | Buffers 1 (default) | Buffers 2 | Buffers 3 |
|---|---|---|---|---|
| SV, panel-size Slint | in place | 1 + 0 = 1 | 1 + 0 = 1 | 1 + 0 = 1 |
| CL | two halves in place | 1 + 0 = 1 | 1 + 0 = 1 | 1 + 0 = 1 |
| SO | copied | 1 + 0 = 1 | 0-1 + 1-0 = 1 | 0 + 1 = 1 |
| GM (interval 0) | copied | 1 + 0 = 1 | 0-1 + 0 = 1 (paced by the show) | 0 + 0 = u |
| fifo (interval 1) | copied | 1 + 0 = 1 | 0-1 + 1-0 = 1 | 0 + 1 = 1 |
| GT (gc_gl_test) | no board layer | 0 + 0 (-ENOTSUP) | the same | the same |

With buffers 2, the copied show waits only when the latch in flight holds
the buffer it needs; the swap's wait then finds that latch done, and the
other way round. Either way the frame takes one period.

The same table before this change, measured by the host model:

- without `.wait_vblank` (main before this item): every row as above,
  except SO and fifo at buffers 3, unpaced (u);
- with `.wait_vblank` wired to the first form of the hook (one more latch
  at every call): SV, panel-size Slint, CL and SO at 2 at every buffer
  count, and fifo at 2 with 1 buffer. That is the halving of SV and CL the
  note predicted ("The effect on each application").

So with the default 1 every row is timed as at `faee8812` and `b421b447`
(1 period a frame where the show paced it, GM included), and the swap adds
no wait to it.

The host model that measured both tables:
`driver/zephyr/lib/board/imx8mp/host/run.sh` compiles `board_imx8mp.c`
unchanged against Zephyr's kernel calls on POSIX threads and a simulated
LCDIF whose `imx8mp_display_set_fb` returns at the next multiple of a
4 ms period, at buffers 1, 2 and 3 (and the Slint window with and without
`CONFIG_YAKOGL_BOARD_SCANOUT_VIEW`). It plays each application's call
order with a draw of 0.3 periods, and prints periods a frame, latches a
frame and the cluster's invariant (no frame submitted into the half on the
panel). With the tree's board file it checks each row against the table
above. Given an older board file (`run.sh old.c outdir`) it prints that
file's table. It is not a `tools/check.sh` row (see the hand-over note).

## Host evidence (2026-10-17)

### F2 wired in (item imx8mp-wait-vblank)

The tree is `499b946e` plus this change (the images say
`YAKOGL_SAMPLE_TREE=499b946e-dirty`), on the deps cache's Zephyr workspace
(`tools/cts/deps-env.sh imx8mp`, as in the section below). No board ran
anything.

- The host model, `driver/zephyr/lib/board/imx8mp/host/run.sh` (period
  4 ms, draw 1.2 ms, 60 frames a row after 6 to warm up):
  - with the tree's `board_imx8mp.c`: 36 of 36 rows `ok` against "Waits per
    swap". With `.wait_vblank`: panel, SV, SO (copied), CL, GM and fifo at
    1.00 period a frame at 1, 2 and 3 buffers, except GM at 3 buffers
    (0.34, unpaced: interval 0); without it, the same except SO and fifo at
    3 buffers (0.33-0.36 over two runs, unpaced). Run again on `befdd331`
    plus this change: 36 of 36 `ok`. The cluster submitted no frame into the
    half on the panel in any row. Unwired and wired agree at buffers 1:
    the default timing is unchanged.
  - with `board_imx8mp.c` of `499b946e` (`run.sh old.c outdir`), wired:
    panel, SV, SO and CL at 2.00 at 1, 2 and 3 buffers, fifo at 2.00 with 1
    buffer; GM unchanged (1.00, 1.00, 0.34). This is the halving the wiring
    alone would have caused.
- The images, from
  `tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt build/wv-imgs GM SO SV CL GT`
  (through a small Python wrapper that sets the environment of
  `tools/cts/deps-env.sh imx8mp`, as below):
  - `GM rc=0 sha256=e83444e299e0bb236719266d0a7d61a53de54bf02cdb68ef9a27f30c650274be`
  - `SO rc=0 sha256=d19ea3a017820b25cd2cfe77d4ea9a59d63211eb9b5caa79179aebae64603850`
  - `SV rc=0 sha256=66314a4c3dc650300d73bb4466eb3fbec55313459374cb0b1fd39bc2fb37fd19`
  - `CL rc=0 sha256=a18c2c103055c78836edb7f54aae9fc607cc3fd42a3ef3e11fdbf00fb2889930`
  - `GT rc=0 sha256=9a0d11a60851063d0f2908d7046df9ee5bc1ad7c606aa31f4595efca60fda2ae`
  No warning for `board_imx8mp.c` or `zephyr_gc.c` in the five build logs.
  GM, SO, SV `.config`: `CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=1` (SV
  with `CONFIG_YAKOGL_BOARD_SCANOUT_VIEW=y`). The GM `zephyr.map` links
  `.text.yakogl_zephyr_wait_vblank` of
  `libyakogl_board.a(board_imx8mp.c.obj)` (0x184 bytes) and `.text.z_wait_vblank`
  of `libyakogl.a(zephyr_gc.c.obj)`, and lists `zephyr_gc.c`'s weak
  definition under "Discarded input sections": F2 is reached. The CL map
  links the board's definition too (0x120 bytes). The GT map (gc_gl_test,
  no board layer) links `zephyr_gc.c`'s weak -ENOTSUP default: gl_test's
  swaps do not wait, as before.
- `tools/check.sh --changed befdd331` (main's tip, the tree's base; the
  change's files are the same as above): `153 ok, 0 failed, 0 gap(s)`
  (scopes: all, since `src/osal` reaches everything) in 2124 s, on the tree
  as committed except this line of the note. The
  `imx8mp gc_gl_test image, -Werror` row compiles `zephyr_gc.c`. The images
  above were built on `499b946e` and not built again on `befdd331`: main's
  commits in between change none of the files this item touches.

### The latch thread (item check-steps)

The tree is `c854e0ae` plus this change (the images say
`YAKOGL_SAMPLE_TREE=c854e0ae-dirty`), built on the deps cache's shared Zephyr
workspace (`tools/cts/deps-env.sh imx8mp`: `west.yml`'s Zephyr `af17c0c8`,
Zephyr SDK 1.0.1, glmark2 `22c527cb`), 64 cores. What was compiled: the
whole i.MX8MP image of the three common rows that link `board_imx8mp.c`
(glmark2, Slint, cluster; `driver/zephyr/lib/board/CMakeLists.txt` compiles
it for `CONFIG_YAKOGL_BOARD`), with the default option, and the glmark2
image with the option at 3 and at 2. What was not: no board ran anything;
`tools/check.sh` compiles no Zephyr image in `--quick`, and its full
`zephyr imx8mp gc_gl_test image` row does not reach `board_imx8mp.c`
(gc_gl_test is not on the board layer), so the images below are the only
compile evidence for the C change.

- The default rows, from
  `tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt build/ff-default GM SO CL`
  with the environment of `tools/cts/deps-env.sh imx8mp` (run through a
  small Python wrapper because this turn's sandbox refused a variable
  assignment in front of the command; the script and its arguments are the
  ones above):
  - `GM rc=0 sha256=4b2cddbd740a03e25718e93bd1bd19fa2b38696eb39f8251894a0cd236c64ae8`
  - `SO rc=0 sha256=14e6e658509413b4999b0688ad21a1e8aa4b16cd71f4a4f22bd7e1b7a3a011d3`
  - `CL rc=0 sha256=e6f3e6a41daaae3ecfef4705d47915201f91d41aeea319bd5c9e426fbc6f40da`
  No warning or error for `board_imx8mp.c` in the three build logs. The GM
  `.config` has `CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=1`,
  `CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP=y`, `# CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_LCDIF_RUST
  is not set`, `CONFIG_SMP=y`, `CONFIG_MP_MAX_NUM_CPUS=4`,
  `CONFIG_MAIN_THREAD_PRIORITY=0`, `CONFIG_SYS_CLOCK_TICKS_PER_SEC=10000`
  (candidate 3's tick rate, now read from a built image) and
  `CONFIG_TIMEOUT_64BIT=y` (what `K_TIMEOUT_ABS_TICKS` needs). Its
  `zephyr.map` links `.text.latch_thread` and `.text.board_show` and lists
  `.text.yakogl_zephyr_wait_vblank` (0x118 bytes, `libyakogl_board.a(board_imx8mp.c.obj)`)
  under "Discarded input sections" (F2 not reached then; "F2 wired in").
- The opt-in arms, the GM row built with `west build` as step 2's `img`
  does, with `-DCONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=3` and `=2`:
  - `=3`: `GM-3buf rc=0 sha256=5b45d70756a85cc3187c0743d52c1266546ef03fc4f14cb59a7d31391dea9f8a`,
    `.config` `CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=3`, no warning for
    `board_imx8mp.c`; `zephyr.map` links `.text.screen_pick` and
    `.text.latch_thread`.
  - `=2`: `GM-2buf rc=0 sha256=5ca608b9a25fb32da7f9e22222e8f4dafee995c5f6691d84fe25beda8e09171e`,
    `.config` `CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=2`, no warning for
    `board_imx8mp.c`.
- `tools/check.sh --quick` on this tree: `123 ok, 0 failed, 0 gap(s)` in
  609 s. No row of it compiles `board_imx8mp.c`.
- `tools/check.sh --changed origin/main` (`origin/main` = `c854e0ae`, the
  tree's base): `15 ok, 0 failed, 0 gap(s)` (scopes: docs drivers) in
  102 s, on the tree as committed except this line of the note.

## The frame-breakdown record (built 2026-10-18; run: "Step 3 result")

**Built (item imx8mp-breakdown-record):** `driver/samples/zephyr/glmark2/src/main.cpp`
writes the page, `driver/samples/zephyr/glmark2/src/gm_perf_page.h` packs it,
`driver/samples/zephyr/glmark2/host/run.sh` checks the packing and the
decode on the host. Selectors: page 0 `0x1000`, page 1 `0x1001`, page 2
`0x1002`, page 3 `0x1003`. The board command is step 3 of the board recipe.
How the code differs from the design below, and why, is in "As built"
after the table; driver/docs/glmark2.md ("The frame breakdown on the
GC7000") is the reference of the record.

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

### As built (2026-10-18)

The code is `driver/samples/zephyr/glmark2/src/main.cpp` (nothing in
`src/vivante/**` or `driver/src/vivante/**`; it calls `gc_render_perf`
through the header `gc_render.h`, which a GC7000 build has on its include
path). It is compiled only for the GC7000 (`CONFIG_YAKOGL_DRV_GPU_VIVANTE`): the
AM62P and Pi 4 images do not contain it, and without a PERF selector the
i.MX8MP run prints and stores what it did before. Where it differs from the
design above:

- **GLMARK2 stays, PERF comes after it.** The run stores GLMARK2 as always
  and then the page, as the GM_PAGE and GL_ERROR selectors do; the SNVS
  keeps the last, so the board reads the page "instead of" GLMARK2. The
  page is the last scene's; each scene also prints a `glmark2-perf:` line.
- **The selector is refused when it names a page the image does not
  have** (`0x1004` to `0x10ff`): the run ends at once with -EINVAL.
- **T (page 2) is measured after the scene's last frame**, not at its
  start, and not by calling `imx8mp_display_set_fb`. Since F1 the latch
  thread owns the LCDIF's latch (`board_imx8mp.c`), so a second caller of
  set_fb would race it; the sample calls the board layer's VSYNC wait,
  `yakogl_zephyr_wait_vblank`, four times instead (each call after the first
  re-latches the shown address and returns at the next VSYNC), and averages
  the last three. Before the first scene's first frame nothing is on the
  panel yet (the wait answers -ENOTSUP), and a wait at the scene's start
  would also fall into the scene's own clock (upstream starts it at the end
  of setup). So the frame-to-frame times are kept in a histogram of 32 us
  buckets, and the bins are counted once T is known: an interval counts at
  its bucket's start, up to 32 us early. T = 0 says the wait failed (one
  console line says why); the bins are 0 then.
- **Bins and page 3 are over the intervals** (frames - 1 in a scene: the
  last frame has no next t0), so page 2 v0 + v1 + v2 = page 0 v0 - 1.
- **Page 2 v4 counts frames whose present took 1000 us or more, present =
  swap + show.** glmark2's window is EGL's double buffer now (`EGL_BACK_BUFFER`,
  "Waits per swap"): `eglSwapBuffers` shows the frame through
  `osal::present` = `board_show`, and the sample's own show
  (`NativeStateBoard::flip`) does nothing, so page 0's show is about 0 and
  the show's latch wait is in the swap. The design's "show >= 1000 us" would
  count nothing.
- **Page 3 values are per interval** (the design gave no unit): kcycles,
  idle kcycles, KiB read and KiB written per frame-to-frame interval. A
  scene's total does not fit 16 bits (10 s at 800 MHz is 8 000 000
  kcycles). HI_PROFILE is read only when page 3 is asked, before each
  frame's t0, so the reads are in no frame's t0..t2.
- **Page 1 leaves the picture readback out**: the last frame's
  `glReadPixels` waits for the GPU; the counters it moved are taken off.

What was checked (tree `cc1fcac8` plus this change, the deps cache's Zephyr
workspace, `tools/cts/deps-env.sh`; no board ran anything):

- `driver/samples/zephyr/glmark2/host/run.sh`: the four pages, the
  saturation, a scene without frames, T not measured, the bin edges at
  1.5 T and 2.5 T and a refused page 4 pack to the expected words, and
  `yakogl-tools record` decodes the four pages as `experiment 0 page p:
  ...` with the expected values. `gm_perf_page: ok (packed and decoded)`.
- The images, 0 compiler warnings in each build log:
  - i.MX8MP, step 3's image (`-b clear`, view, built as step 3 says):
    `rc=0 sha256=96c3cd460f0aaade7044c9e6987459a43fcdf9bb472c225181efdaafc793d14c`;
    `.config` `CONFIG_GLMARK2_ARGS="-b clear"`,
    `CONFIG_YAKOGL_BOARD_SCANOUT_VIEW=y`,
    `CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=1`,
    `CONFIG_SYS_CLOCK_TICKS_PER_SEC=10000`. Its `zephyr.map` links the
    board layer's `yakogl_zephyr_wait_vblank` (`board_imx8mp.c`, the weak
    one of `zephyr_gc.c` discarded), `gc_render_perf` and
    `osal_gc_wait_stats`, and has the page state in `.bss`.
  - The GM rows of `tools/cts/common-images.sh` (the default list):
    i.MX8MP `rc=0 sha256=ee63240b7376f6a5954d9035e151906822bd90ea1a4d7a376ca15ffb93cc0849`,
    AM62P `rc=0 sha256=01503afba393421be47ff908e2ad2959a2a527a5fe797b2e2826429640974691`,
    Pi 4 `rc=0 sha256=a3b4e29b0d4ed9d109a3b00ea68113dcc7d62653849d5baa15eaa345e9a145c7`
    (all `YAKOGL_SAMPLE_TREE=cc1fcac8-dirty`). The AM62P and Pi 4 maps have no
    symbol of the page: the code is compiled out there.

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

With this change in the tree, step 1's images are the default build
(`CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=1`): the same timing, so the
table above still holds for them.

### Step 2: the latch thread on the board (the next i.MX8MP session)

Step 2 can run without step 1. The default rows (option 1) are the old
timing from the same tree and the A/B baseline; the `-3buf` and `-2buf`
images are the opt-in arms. The environment is as in
`tools/cts/common-images.sh` ("Environment"): `ZEPHYR_WS`, `WEST_VENV`,
`ZEPHYR_SDK_INSTALL_DIR`, `GLMARK2_SRC`, `SLINT_NOSTD_DIR`,
`NOSTD_PATCH_DIR`, `AGL_SLINT_CLUSTER_DIR`, `NOTO_SANS_DIR`,
`RUSTUP_TOOLCHAIN=nightly`; `tools/cts/deps-env.sh imx8mp` prints them for
the deps cache's layout (`export $(tools/cts/deps-env.sh imx8mp)`).
`BOARD_HOST` is the leased board's ssh login (ccfarm's `ask_farm`; never in
a file). `BOARD_BOOT=uboot-mmc` as in the earlier i.MX8MP sessions. Run from
the checkout's top with a tree that has this change
(`driver/zephyr/lib/board/imx8mp/board_imx8mp.c` has `latch_thread`).

```sh
SW=$PWD
IMG=build/floor-f1f2-$(git rev-parse --short=8 HEAD)
REC=records/imx8mp/floor-f1f2-$(git rev-parse --short=8 HEAD)
# 1. the common rows as built (option 1, the old timing): glmark2 (default
#    list), Slint, Slint view, cluster
tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt "$IMG" GM SO SV CL
# 2. the opt-in arms and two light scenes
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr PATH=$WEST_VENV/bin:$PATH
MODS=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
img() { # img <dir> <sample> <cmake args...>
  d=$SW/$IMG/$1 s=$2; shift 2
  (cd "$ZEPHYR_WS" && west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d "$d" \
    "$SW/driver/samples/zephyr/$s" -- "-DZEPHYR_MODULES=$MODS" \
    -DYAKOGL_DIR="$SW" -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y "$@")
}
GM_ARGS=("-DGLMARK2_SRC=$GLMARK2_SRC")
SO_ARGS=("-DSLINT_NOSTD_DIR=$SLINT_NOSTD_DIR" "-DNOSTD_PATCH_DIR=$NOSTD_PATCH_DIR")
img GM-3buf glmark2 "${GM_ARGS[@]}" -DCONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=3
img GM-2buf glmark2 "${GM_ARGS[@]}" -DCONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=2
img GM-clear glmark2 "${GM_ARGS[@]}" '-DCONFIG_GLMARK2_ARGS="-b clear"'
img GM-clear-3buf glmark2 "${GM_ARGS[@]}" '-DCONFIG_GLMARK2_ARGS="-b clear"' -DCONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=3
img GM-build-3buf glmark2 "${GM_ARGS[@]}" '-DCONFIG_GLMARK2_ARGS="-b build:use-vbo=true"' -DCONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=3
img SO-3buf slint "${SO_ARGS[@]}" -DCONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=3
# 3. run, one image at a time (board.sh: "run <image> <logdir> [name]")
for r in GM-clear GM-clear-3buf GM-build-3buf GM GM-3buf GM-2buf SO SO-3buf SV CL; do
  BOARD_HOST=<user@host> BOARD_BOOT=uboot-mmc \
    tools/board.sh imx8mp run "$IMG/$r/zephyr/zephyr.bin" "$REC" "$r"
done
```

If the sandbox refuses a command line that starts with `BOARD_HOST=...`,
export the two variables in a small wrapper. Each run prints one
`board.sh imx8mp <name> rc=... verdict=...` line. The i.MX8MP has no
console capture, so the record is the only result. A GLMARK2 record's
second word is `fc3f` and then the score in hex (`fc3f002a` = 42, the
common-set score at `b421b447`, `tools/cts/RESULTS.md`, "Common set").

Expected:

| Run | Expected record / verdict | Reading |
|---|---|---|
| GM-clear | second word `fc3f003a` (58), or `fc3f001d` (29) | The default (option 1): locked to the panel, never above 58. The baseline of the next row. |
| GM-clear-3buf | `verdict=done result=record:GLMARK2`, second word `fc3fXXXX` with XXXX above `003a` | `clear` above 58 FPS. Expected well above 58: a full-screen clear is about 1.9 ms of GPU (candidate 2) plus the copy of 1.92 MB (candidate 4). |
| GM-build-3buf | second word above `fc3f003a` | A light model scene above 58 FPS. |
| GM | `verdict=done`, second word near `fc3f002a` (42; 37-46 earlier), `scenes ran 33, failed 0` in the tally, `GL errors raised 0` | The default list with the default option: the old timing from the same tree. Confirms the A/B; the default build is unchanged in timing. |
| GM-3buf | `verdict=done`, second word above `fc3f003a`, `scenes ran 33, failed 0`, `GL errors raised 0` | The score is no longer capped near 58, and is well above 42. Heavy scenes still run at their GPU cost; see candidate 2. |
| GM-2buf | second word at most `fc3f003a` (58), above GM's | Paced at the panel: every scene at 58 or its own slower rate, none rounded up to two periods by the copy and the wait in series. |
| SO | `verdict=done result=record:PROBES record=594b0601,08420862,00210842,c5920883` | Probes as before; paced by the show as before (the default). |
| SO-3buf | the same PROBES record as SO | The window is copied, every shown frame complete. Paced at the panel's rate by the swap (F2, "Waits per swap"): the motion as SO's. |
| SV | the same PROBES record as SO | Scanned in place, paced by the show's latch wait as before. |
| CL | `verdict=done result=record:PROBES record=594b0601,10c410c4,1905f7bf,b1076d5f` | Probes as before; two halves in place, paced as before. |

The picture, which no record shows: a person at the panel (or ccfarm's
camera, as the Pi 4 sessions used) watches GM-clear-3buf and GM-3buf for a
few scenes, and GM for comparison. Expected: each scene whole, as at
`b421b447`, no bands of another frame, no pieces of a clear. This is the
check #923 asked for before F1 comes back, and the check of the hypothesis
in "Proposed fix" (a copy into a buffer the LCDIF does not scan keeps the
picture whole). If GM-3buf shows pieces of several frames while GM is
whole, the hypothesis is wrong: the cause of #923 is not the scanned buffer
being written, and the 3-buffer arm must not become the default. Check too
that the copy path is what the GM row runs (`board: a copied window is
shown through 3 1280x800 screen buffer(s)` on the panel console, or the APP
record's `YK_RECORD_APP_SHOWN_BY_COPY`): the thread does not skip the wait
for an in-place window on purpose.

What the records cannot show: Slint and the cluster write no frame count,
and the i.MX8MP has no console, so their pacing is not in a record. A
person at the panel can check it: SO, SO-3buf, SV and CL all move as
before (one frame a panel period). SV or CL moving at half the speed would
mean the swap waits a second period after the show ("Waits per swap"; the
host model says it does not).

- If an SO-3buf probe differs from SO's, the copy-path present changed what
  is scanned: the hypothesis is wrong for Slint too.

A GT run of the common set (`tools/cts/common-images.sh ... GT`) should
still read 49 / 49 or fail only at case 44. gl_test's result does not
depend on pacing.

What the run decides:

- GM-3buf and GM-clear-3buf above 58 with a whole picture, SO-3buf's probes
  equal to SO's: a follow-up changes `default 1` to `default 3` in
  `driver/zephyr/lib/board/Kconfig` and replaces the GM row in
  `tools/cts/RESULTS.md` with GM-3buf's score.
- GM-clear-3buf at 58 or below while GM-clear is also at 58: the frame is
  still locked, and something other than the show waits. Step 3's page 0
  (show us) and page 2 (frames per bin) say where.
- A broken picture in the 3-buffer arm: the default stays 1, and the next
  item is the second window buffer EGL owns (since built:
  `include/gles_osal.h`; the panel look is #1207), with step 3's
  record to see what the LCDIF scans while the GPU draws.

### Step 2 result (2026-10-18, main `45dc44a8`, `.wait_vblank` wired)

The common set ran on the board at 1 buffer (the default, main `45dc44a8`)
and at 3 and 2 (two local commits changing only the Kconfig default; every
`.config` checked), all 15 rows `verdict=done`, `resets=absent`. The GM
half (main `bebce725`) is in the same table of `tools/cts/RESULTS.md`.

| Row | 1 buffer | 3 buffers | 2 buffers |
|---|---|---|---|
| GM score | 42 | 49 | 46 |
| SO / SV probes | `594b0601,08420862,00210842,c5920883` | same | same |
| CL probes | `594b0601,008610c4,10c4f7bf,04066d5f` | same | same |
| GT | 49 of 49 run | same | same |
| board return time SO / SV / CL | 104 / 108 / 158 s | 115 / 109 / 158 s | 115 / 105 / 168 s |

What this shows: the probes do not move (SO / SV `c592`, CL `0406`, which
are the panel-independent cluster probes, not the older `b107`); GM raises
0 GL errors in all arms (#814); no Slint or cluster row took twice as long
(a second wait per frame would have), though the return time is a
fixed-length run and a weak proxy. What it cannot show: the i.MX8MP has no
console and the record carries no frame count for Slint, so "SO-3buf and
SO-2buf are paced, no unpaced swap" is **not measured**; nobody watched the
panel, so #923's tearing and half-drawn frames are **not ruled out** at 3 or
2 buffers.

Decision: `CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS` stays **1**. The rule
above asked for a whole picture and no regression in both the GM and the
SO / SV / CL runs; the picture half has no evidence yet, and the 3-buffer
gain is small (42 to 49; a light frame still takes 15 to 16 ms). The next
session needs eyes on the panel, or a frame-count line in the Slint record.

### Step 3: the frame-breakdown record (built; the next i.MX8MP session)

The `view` image of step 1 (one scene, `clear`, the window a view into the
screen), four runs, one page each. The environment is step 2's
(`export $(tools/cts/deps-env.sh imx8mp)`; `BOARD_HOST` from ccfarm's
`ask_farm`, never in a file; `BOARD_BOOT=uboot-mmc`). From the checkout's
top:

```sh
SW=$PWD
IMG=build/breakdown-$(git rev-parse --short=8 HEAD)
REC=records/imx8mp/breakdown-$(git rev-parse --short=8 HEAD)
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr PATH=$WEST_VENV/bin:$PATH
MODS=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
(cd "$ZEPHYR_WS" && west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d "$SW/$IMG/gm-clear-view" \
  "$SW/driver/samples/zephyr/glmark2" -- "-DZEPHYR_MODULES=$MODS" \
  -DYAKOGL_DIR="$SW" -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y -DGLMARK2_SRC="$GLMARK2_SRC" \
  '-DCONFIG_GLMARK2_ARGS="-b clear"' -DCONFIG_YAKOGL_BOARD_SCANOUT_VIEW=y)
for p in 0 1 2 3; do
  SEL=$((0x1000 | p)) BOARD_HOST=<user@host> BOARD_BOOT=uboot-mmc \
    tools/board.sh imx8mp run "$IMG/gm-clear-view/zephyr/zephyr.bin" "$REC" clear-view-p$p
done
```

Each run prints `board.sh imx8mp clear-view-p<p> rc=0 verdict=done` and its
`snvs.txt` holds the page (`result=record:PERF`); `yakogl-tools record
<w0> <w1> <w2> <w3>` prints `experiment 0 page p: v0 .. v5`. Other scenes the
same way (`-b build:use-vbo=true`, ...), and at 3 buffers with
`-DCONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=3` (the next item,
imx8mp-breakdown-board).

Expected (`clear`, view, the default 1 buffer). Values are approximate
where marked. The show's latch wait is in swap when EGL shows the window
(EGL's double buffer) and in show when the window is rendered in place;
the sum is what is expected:

| Page | Record | Reading |
|---|---|---|
| 0 | `594b0c00 427a024b <swap><gl> 427a<show>` | about 587 frames (10 s x 58.76; +-2), frame and frame-to-frame about 17 018 us (0x427a, +-1 %), gl small, swap + show = about 17 018 - gl (at least 13 000 us together) |
| 1 | `594b0c01 ...` | GPU busy a few ms (expected 1-5 ms); waits per 100 frames low (the previous frame is done during the show's wait); sleeps per 100 frames near 0 |
| 2 | `594b0c02 0000024a 00000000 ....427a 2710024b` | all intervals under 1.5 T (v0 = frames - 1, v1 = v2 = 0), T about 17 018 (0x427a), v4 = frames (a present of 1 ms or more every frame), ticks 10 000 (0x2710) expected |
| 3 | `594b0c03 ...` | core MHz about 800 (0x0320); idle kcycles most of total |

Expected with a show that does not wait (3 buffers, copied): page 0 swap
+ show well under a period, frame = frame-to-frame, frames = 10 s / frame;
page 2 v4 = 0. The
GPU busy and wait values then say whether candidate 2 (GPU busy close to the
frame) or candidate 3 (sleeps per frame high, wait well above GPU busy) is
the next limit.

If page 2 v5 is not 10 000, the image was built with a different tick rate
than the pinned Zephyr's default (candidate 3): at 100, each sleep would be
10-20 ms, and the poll would be fixed before or with F1.

### Step 3 result (2026-10-18, main `a19927f3`, item imx8mp-breakdown-board)

Four images of the glmark2 sample, the window copied (no view, as step 2's
GM rows; the buffer count only acts on a copied window): `-b clear` and
`-b build:use-vbo=true`, each at `CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=1`
and `=3`. Built with the GM row of `tools/cts/common-images.sh` at tree
`a1bccdd6` (main `a19927f3` plus the cluster sample's commit, which is not
in a glmark2 image); the scene and the buffer count went into the board's
`.conf` for the build only (`YAKOGL_SAMPLE_TREE=a1bccdd6-dirty`), each
`.config` checked. One boot per page, pages 0 to 3, `BOARD_BOOT=uboot-mmc`,
one GL client, 16 boots through `tools/board.sh imx8mp tree`. Every run:
`rc=0 verdict=done boot=uboot-mmc row result=record:PERF resets=absent`.
Decoded with `yakogl-tools record` (`experiment 0 page p: v0 .. v5`):

| Arm (image sha256) | page 0 | page 1 | page 2 | page 3 |
|---|---|---|---|---|
| clear, 1 (`a4bf254b`) | 588 17064 1 17054 8 17048 | 0 0 0 1981 99 1 | 585 2 0 17014 588 10000 | 13639 12073 1908 3347 800 0 |
| clear, 3 (`06194f82`) | 672 14911 1 14902 7 14914 | 0 0 0 1989 99 1 | 670 1 0 17014 672 10000 | 11938 10365 1908 3347 800 0 |
| build, 1 (`36ca882d`) | 587 17078 38 17031 9 17065 | 0 0 0 3252 99 1 | 584 1 1 14091 587 10000 | 13652 11063 3657 4940 800 0 |
| build, 3 (`509837a0`) | 643 15592 37 15546 8 15596 | 0 0 0 3261 99 1 | 641 1 0 17018 643 10000 | 12475 9880 3657 4940 800 0 |

The records (w0..w3) are in the runs' verdict lines; for example clear at
3 buffers page 0 is `594b0c00,3a3f02a0,3a360001,3a420007`.

Where a light frame's time goes (us a frame, means over the scene):

| Arm | frame | GL | swap | show | GPU busy (page 1; page 3 agrees) | frame - GPU busy | locked to T? |
|---|---|---|---|---|---|---|---|
| clear, 1 | 17 064 | 1 | 17 054 | 8 | 1 981 | 15 083 | yes: T 17 014, 585 of 587 intervals under 1.5 T |
| clear, 3 | 14 911 | 1 | 14 902 | 7 | 1 989 | 12 922 | no: 0.88 T |
| build, 1 | 17 078 | 38 | 17 031 | 9 | 3 252 | 13 826 | yes |
| build, 3 | 15 592 | 37 | 15 546 | 8 | 3 261 | 12 331 | no: 0.92 T |

- **At 3 buffers the 15 ms are all in the swap, and the swap does not wait
  for the GPU or for a VSYNC.** GL calls are 1 to 37 us and the sample's
  own show is about 8 us (EGL's double buffer shows the window inside
  `eglSwapBuffers`, "As built"). The OSAL counts no wait for the GPU (page 1
  v0 = v1 = 0) and no `gc_render_wait` sleep (v2 = 0); one kick a frame.
  The frame is 0.88 T (clear) and 0.92 T (build), not a multiple of the
  period: no latch paces it.
- **The GPU is 2.0 ms (clear) and 3.3 ms (build) of it**, the same at 1 and
  3 buffers; HI_PROFILE agrees (busy 1 566 to 2 595 kcycles a frame at
  800 MHz, the GPU idle 79 to 89 % of the time), and the GPU's memory traffic
  (1.9 / 3.3 MiB read and written for clear, 3.6 / 4.8 for build) is the
  same in both arms: the copy into the screen buffer is not done by the GPU.
- **The rest, 12.3 to 12.9 ms a frame, is the present path inside the swap**:
  the CPU copy of the 800x600 window (1.92 MB) into a screen buffer and
  whatever it waits for. Frame minus GPU busy is nearly the same in both
  scenes (12.9 and 12.3 ms) while the GPU time differs by 1.3 ms, and the
  frame grows by 0.7 ms: the present runs mostly in series with the GPU's
  frame, not overlapped with it. These pages do not split the present
  further (copy against a wait for a free screen buffer or for the frame's
  end); a stamp around the copy in `board_show` would.
- **At 1 buffer** the same work is rounded up to one period: frame 17.06 ms
  = T (17.014 ms), every interval but 2 or 3 under 1.5 T, every present
  1 ms or more (page 2 v4 = frames). The 3-buffer arm gains 2.2 ms (clear)
  and 1.5 ms (build) a frame, what was left of the period after the copy.
- Page 2 v5 = 10 000 in all runs: the tick rate is the pinned Zephyr's, so
  candidate 3's poll is 1 ms, and it does not run here (no sleeps).
- One oddity: build at 1 buffer measured T = 14 091 us (the other three runs
  17 014 to 17 018). Its page 0 frame is 17 078 us, one period, so the
  scene was paced at T as usual; the four `yakogl_zephyr_wait_vblank` calls
  after the scene returned early once. The bins of that run use the short
  T (1.5 T = 21.1 ms) and still put all but 2 intervals in the first bin.

So the floor of a light glmark2 frame at 3 buffers is the window's copy in
the present path (about 12.5 ms), not the GPU (2 to 3 ms) and not the
latch. This is why step 2's 3-buffer score gain was small (42 to 49).
What was not done here: the picture at 3 buffers was not looked at
(#923 is still not ruled out), and the default stays 1.

## What this does not show

- The latch thread is built for the board, not run on it. The Zephyr
  builds of the glmark2, Slint and cluster images ("Host evidence") are
  host evidence that the code compiles and links, with the default and
  with the option at 3 and 2. They say nothing about timing or the picture.
- No board run was made. The per-scene split (18 / 10 / 9 scenes at one
  period) is inferred from the scores, not measured.
- The cause of #923 is not confirmed ("Proposed fix"). That the LCDIF
  scanning only complete frames keeps the picture whole rests on the copy
  path's reading of the code (the window is copied after `eglSwapBuffers`
  completed the frame, `0247ac66`; a screen buffer is never copied into
  while the LCDIF scans it or a latch of it is in flight). Step 2's look at
  the panel is the check, and the default stays 1 until then.
- With the option at 3 the screen buffers take 12 MiB of the carve-out
  instead of 4 MiB (1280x800x4 each). The glmark2 image has 240 MiB; the
  Slint image has the board's 64 MiB, of which Slint's own use is not
  measured here. If `board: the carve-out holds N of 3 ... screen buffers`
  appears, the show runs with fewer (2 paces, 1 waits).
- The Zephyr tick rate was read from the pinned Zephyr's Kconfig files, not
  from a built image's `.config`.
- Mesa etnaviv was not re-read (no Mesa tree was read for this note);
  nothing in the conclusion rests on it.
- The 46 (view) run has no row in RESULTS.md; its number is ab43a20c5:tools/cts/log/status-2026-09-25-to-10-07.md:88's.
