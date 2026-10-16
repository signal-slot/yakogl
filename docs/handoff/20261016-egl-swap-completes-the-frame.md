# eglSwapBuffers completes the frame on a window, 2026-10-16, work/egl-swap-completes-the-frame

## State of main
- Every glmark2 scene on the i.MX8MP showed a black band of the last-drawn
  rows (about rows 40 to 90 of the 800x600 window), in the copy path, the
  scan-out view and at 1024x600 and 800x640 alike, back to `dc6e62e4a`
  (2026-10-07): not a regression. Cause: `eglSwapBuffers` submitted the
  frame and waited only for the frame before it (double-buffering was left
  to the application), while the glmark2, Slint and triangle samples show
  their one window right after the swap, so the board showed a buffer the
  GPU was still writing. A 5 ms wait before the board's copy made the band
  vanish; stalls and flushes added to the kick epilogue did not; a 240 MiB
  carve-out (#922) and the panel console off did not either.
- Fix: a window surface's swap completes the frame it submitted unless the
  native window carries `GLES_NATIVE_WINDOW_PIPELINED` (new,
  `include/EGL/eglplatform.h`), the flag an application sets when it
  double-buffers the window itself (two halves, the half shown is never the
  half being drawn): the cluster sample sets it; glmark2, Slint and triangle
  do not and get complete frames. A pbuffer's swap is unchanged.
- The pipelined swap was what the glmark2 scores of stage 2 were measured
  with (AM62P 462, Pi 4 188, i.MX8MP 43 at `e9901869a`): every board's
  glmark2 now waits for its frame before showing it, so the next common set
  run re-measures them. The benchmark samples stay stock: the overlap comes
  back from the driver, not from the samples (owner, 2026-10-16).

## Open work
- Stage 2: double-buffering inside YakoGL's EGL window surface, so a stock
  application gets a complete picture AND the CPU/GPU overlap: the surface
  owns a second colour buffer (OSAL GPU memory), `eglSwapBuffers` submits
  the frame into the buffer not on screen, waits for the frame before it and
  presents that one through an OSAL present hook the board layer implements
  (board_show() of a given buffer, which the Pi 4 pan, the AM62P plane flip
  and the i.MX8MP LCDIF address already support). The samples' flip() then
  only feeds the watchdog. `GLES_NATIVE_WINDOW_PIPELINED` stays for an
  application that manages its own halves (the cluster sample).

## Known defects
- Not an issue number: the black band on the i.MX8MP's glmark2 (reported by
  the owner on 2026-10-16) is fixed by this result; #814 (the GL errors) is
  separate and still open.
