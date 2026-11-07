<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Armadillo-X2: glmark2 behind a 1920x1080 HDMI sink

Plan #328 (items x2-gm-before, x2-gm-after; issues #1204, #1106). From the code
only: no board was used. Citations are at main `93ada074`.

## The answer

The message `the LCDIF runs 1920x1080, the devicetree expects 1920x1200` is the
old text of a check that main no longer has. Since `d6e1bd74` the adopt driver
accepts a running mode that fits inside the devicetree buffer, logs
`the LCDIF runs 1920x1080 inside the 1920x1200 buffer: cropped` at INF and
goes on. The glmark2 run does not abort, does not fall back and does not change
the size glmark2 renders at. It draws at the size of `CONFIG_GLMARK2_SIZE`
(800x600 by default) and the panel shows the top 1080 rows of the 1200-row
buffer it scans. The per-scene ms and the PERF pages stay valid and comparable
before and after, provided both runs are images from a main that has `d6e1bd74`.
No change to the X2 conf or overlay is needed for this.

## Code path

1. **Where the old message came from.** `driver/src/display/imx8mp_lcdif_adopt.c`
   `imx8mp_display_on()` reads the running scan-out from the LCDIF registers
   (`:123`, `ip/lcdifv3.rs` `read_scanout`, width and height from
   `CTRLDESCL0_1`) and compares it with the devicetree node
   `driver/boards/atmark/armadillo_x2/armadillo_x2_mimx8ml8_a53.dts:90-95`
   (`hactive = <1920>`, `vactive = <1200>`). Before `d6e1bd74` any difference
   returned -EINVAL. Now only a mode larger than the buffer does (`:133-139`,
   error `the LCDIF runs %ux%u, the devicetree buffer is %ux%u`); a smaller one
   logs `:141` and continues. The first jump ran an image built before that
   commit (`f1eae634b44d`).
2. **What stays the devicetree's.** `s_info.width`/`height` are the buffer,
   `1920x1200` (`:157-158`), and `imx8mp_display_geometry()` returns the same
   (`:87-91`). The pitch is `1920 * 4` unless the caller gives one (`:145-147`).
   The LCDIF's own window register is not written: the sink's 1080 lines are
   scanned from the start of our buffer, so the last 120 rows are never shown.
3. **The board layer sizes by that geometry.** `board_imx8mp.c:362` takes
   `pw x ph = 1920x1200`; `:366` refuses a window larger than it (-ENOTSUP). A
   window that is not the panel's size is copied into a `1920x1200` screen
   buffer (`:425-433`, `screens_alloc(pw, ph)` at `:209`); a view
   (`CONFIG_YAKOGL_BOARD_SCANOUT_VIEW`, `:305-312`) would be scanned in place.
   `board_show` calls `imx8mp_display_on(scan, pitch)` at the first show
   (`:434-441`), which is where step 1 runs; a non-zero return makes the show,
   and so the first flip, fail.
4. **glmark2.** `main.cpp:1384-1392` parses `CONFIG_GLMARK2_SIZE` and calls
   `board_window(w, h)`; the size never comes from the display. On a failure
   it returns before any scene (`:1389-1392`). glmark2 sees no 1080 anywhere.
5. **Which call shows the frame.** EGL double-buffers the window
   (`src/gl/egl.rs:1414`, `s.back`), so the swap presents it:
   `egl.rs:2331-2334` -> `osal::present` -> `src/osal/zephyr_gc.c:600`
   (`.present`) -> `zephyr_timing.c:116-122` -> `board.c:31-40` -> `board_show`.
   A refusal would be logged once as `eglSwapBuffers: the display refused the
   window's buffer`. A scan-out window is the exception (`egl.rs:1369`:
   `EGL_BAD_MATCH` without `EGL_SINGLE_BUFFER`). `flip()`
   (`main.cpp:464-492`) calls `board_show` only for a window not
   double-buffered and leaves `rc` at 0 otherwise, so `shown yes` does not tell
   the cases apart.

## Do the per-scene numbers stay valid?

- **Render size and GPU work:** unchanged. The window is 800x600 at both
  resolutions, the scene, its GPU time and its CPU fallbacks do not depend on
  the sink.
- **The copy:** a copied window writes 800x600 pixels into the top-left of the
  1920x1200 buffer (`:432-435`). That region is inside 1080 rows, so the picture
  is whole on screen. It costs the same as at 1200 lines.
- **The three slow scenes (terrain, shadow, refract):** they run at 250 ms and
  500 ms, several periods a frame, so the period does not bound them. What
  matters for them is the same before and after.
- **The 30 VSYNC-bound scenes:** `board_show` waits for the panel's latch
  (`board_imx8mp.c` "The latch"), so a scene runs at a whole number of display
  periods (`imx8mp-frame-floor.md`, 17.0 ms on the Verdin's 58.76 Hz panel). The
  X2's period is the sink's mode as Linux left it: 1920x1080 at 60 Hz is
  16.67 ms, 1920x1200 reduced blanking is about the same, but this is not
  measured: PERF page 2's `T` (`main.cpp:871-885`) is the measure. If `T`
  differs between the two X2 runs, those 30 scenes move with it, so compare
  them only within one sink. That does not matter for this plan: it measures
  the three slow scenes, whose ms is not a multiple of T.
- **The CPU depth resolve and mipmap paths** read and write the GC's own
  buffers, not the scan-out.

## Proposal for the item that owns the conf (not made here)

Nothing is required. Two optional things for `driver/samples/zephyr/glmark2/boards/armadillo_x2_mimx8ml8_a53.conf`
and its overlay: keep `CONFIG_GLMARK2_SIZE` at the default 800x600 (a size
larger than 1080 rows would be cropped), and leave `CONFIG_YAKOGL_BOARD_SCANOUT_VIEW`
off, as the Verdin's conf has it. A devicetree with `vactive = <1080>` would also work and drop the INF line,
but the dts is plan #323's, and it would break the 1200-line monitor.

## What x2-gm-before and x2-gm-after must check

- The image is built from a main that includes `d6e1bd74`.
- The boot log has neither the `imx8mp_disp` error (`imx8mp_lcdif_adopt.c:135`)
  nor `eglSwapBuffers: the display refused the window's buffer`
  (`egl.rs:2331`).
- The VSYNC-bound scenes' swap column is near one period; if not, the display
  is not being shown, and the slow scenes' ms are not a display measurement
  either.
- Both runs use the same sink and the same cable: `T` of PERF page 2 is then
  the same, and the three slow scenes' ms compare directly.
