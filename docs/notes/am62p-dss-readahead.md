<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: can the DSS still read buffer N after it latched N+1?

2026-11-07. Plan #329 item `am62p-dss-readahead`, read at main `d83e25b5`.
Answers the open question of `am62p-flip-latch.md` ("Left", second bullet:
line-buffer read-ahead). Code and the TRM wording quoted in the code only;
no board run, no code change. Issue: #1199.

**Verdict: safe if the latch is at or after the end of the old frame's
active fetch (which is what the TRM wording the driver relies on says).
The worst case, if the latch could come early, is bounded: at most the last
10 lines of the old frame, 263 us, and the renderer cannot write the freed
buffer sooner than one in-order GPU frame later. Not "safe by construction":
no register or text in the tree states when the fetch of the old frame
ends relative to the latch; the board check below measures it.**

## How many buffers, and which is given back when

- Four window buffers where the OSAL has the `display_busy` op (the AM62P),
  three elsewhere: `src/gl/egl.rs:64` (`WINDOW_BUFFERS = 4`), allocation
  `src/gl/egl.rs:2252` (`n = if osal::has_display_busy() { WINDOW_BUFFERS }
  else { 3 }`). The safety layer's two buffers are a separate pair
  (`dss.rs:429` `safety_flip`) and never go to the renderer.
- The renderer asks at every swap: `swap_window_buffers`
  (`src/gl/egl.rs:2325`) presents the previous frame, then
  `next_window_buffer` (`src/gl/egl.rs:2280`) takes the first buffer after
  `cur` whose bit is clear in `osal::display_busy` (`egl.rs:2287-2294`).
  The op reaches the driver through `src/osal/zephyr.c:730`
  -> `yakogl_zephyr_display_busy` (`driver/zephyr/lib/board/common/board.c:49`)
  -> `board_display_busy` (`board_am62p.c:93`) -> `am62p_dss_app_busy`
  (`driver/drivers/display/display_am62p_dss_rust.c:123`) ->
  `soc::am62p::dss::app_busy` (`driver/rust/src/soc/am62p/dss.rs:401`).
- **What releases a buffer is the GO bit reading clear, not an interrupt
  and not a status bit of the pipeline.** `app_busy` reads `GOBIT` of
  `DSS_VP_CONTROL` (`dss::go_pending`, `ti_dss7.rs`, beside `go` at :353);
  GO clear -> `app_settled` (`dss.rs:374`): the plane scans exactly
  `app_addr_pending`, every other buffer is free. `set_app_buffer`
  (`dss.rs:304`) does the same read before it writes. The hardware clears
  GO when it copies the shadow registers at "the end of the current frame"
  (`ti_dss7.rs:22-29`, TRM 12.9.1.4.1.13 / 14.8.9.1.2.271); the model does it
  in `Am62pModel::vsync` (`model_am62p.rs:274-286`).
- The VSYNC interrupt (`dss_isr`, `dss.rs:445`; `latch_settled`, `dss.rs:467`)
  only moves `app_seq_shown`, which nothing reads for release
  (`am62p-flip-latch.md`, "Implemented"). FRAMEDONE is not enabled
  (`irq_enable`, `ti_dss7.rs:403-409`: VSYNC, SYNC LOST, buffer underflow).
- Order, for flips A then B (A being scanned): `set_app_buffer(B)` writes
  the shadow `BA_0/BA_1` (`dss.rs:317-324`) and sets GO (`commit`,
  `dss.rs:253`). Until the frame end: A and B busy (`held`, `dss.rs:411`).
  Frame end: hardware latches B, clears GO. First `app_busy`/`set_app_buffer`
  that reads GO clear: A free, B busy. A flip over an unlatched one
  (`old_pending`, `dss.rs:331-343`) drops the old address at once (never
  scanned). Doubtful cases keep `old` busy until GO is seen clear
  (`app_keep_busy`, `dss.rs:381`). So **a buffer is free only after the
  frame end that latched its successor; it is observed free no earlier
  than the next GO read after that, up to one swap later.** The report is
  late, never early.

## The read-ahead

- Each pipeline fetches through a DMA FIFO, `BUFSIZE` 128-bit words,
  reset value `0xA00` = 2560 words = 40960 bytes (`model_am62p.rs:14,140`;
  `ti_dss7.rs:516`, TRM 14.8.9.1.2.45). `pipe_fifo_init` (`ti_dss7.rs:216`)
  sets high threshold `BUFSIZE-1`, low `BUFSIZE-8`, preload = low
  (`fifo_for`, `ti_dss7.rs:201`; TRM 12.9.1.4.1.6.2): the DMA refills
  whenever the FIFO falls under the low threshold, so it runs **as far
  ahead of the output as the FIFO is deep**.
- Depth in lines: 40960 B / 4096 B (a 1024-wide XRGB8888 line, panel size)
  = **10.0 lines**; at 800x600 (3200 B a line) 12.8 lines, but the
  window is shown at 800 wide only if the plane is that size, so 10 lines
  is the figure for the panel-sized plane and 12.8 the upper bound.
  (Computed here; the TRM's FIFO width and the `BUFSIZE` of this silicon
  are read at the board, below.)
- Time: the panel is 1349 x 645 total at 51.2 MHz (`dss/tests.rs:19`,
  `board_cfg`): one line 26.35 us, the frame 16.99 ms (58.85 Hz), 45 blank
  lines (1.19 ms). So 10 lines = **263 us**, 12.8 lines = 337 us.
- Where this matters. The fetch of the old frame is ahead of the output
  by at most the FIFO. If the latch is at the frame end (after the last
  active line is output), the fetch of the old frame ended up to 263 us
  *before* it, and the next frame's fetch (preload) starts from the new
  address: no read of buffer N after N+1 is latched. If the latch were
  taken as the start of vertical blanking, the same holds (the last line
  is fetched when it leaves the FIFO ahead of the output). The only
  sequence that reads N after the latch is a latch *before* the old
  frame's last 10 lines were fetched (a latch inside the active area
  that does not re-aim the in-flight DMA). The TRM wording the driver
  quotes ("end of the current frame", `ti_dss7.rs:24-26`) says the
  configuration changes at the frame end, i.e. not inside the active area.
  Nothing in the tree contradicts this (no fault in the model keeps a
  fetch past the latch; the model has no FIFO).

## The hazard window, if it exists

Bound it by the worst case: the latch cuts the old fetch, up to 10 lines
(263 us) of N are still read after GO clears, and the renderer learns N is
free at once. The renderer is not that fast:

- `next_window_buffer` runs inside swap k, and the buffer is bound for
  frame k+1, which the GPU starts only after frame k (in order). The
  lightest scenes take 1.07-1.14 ms a frame (`am62p-flip-latch.md` per-scene
  table, rows 2-5) and they are the fastest case; that is 4x the window.
- The new frame writes the buffer top to bottom in tiles, the old frame's
  tail is the bottom 10 lines, so the first write that could land there is
  later still.
- A torn picture would need a write into the last <= 10 lines during the
  263 us: it needs the recorded frame k to be shorter than 263 us. No
  glmark2 scene is (`build` use-vbo=true is the shortest, 1.14 ms).

So the window is closed by the GPU's frame time for every scene measured,
and by the frame end for a hardware that behaves as the TRM text says.
`safe if X`: X = the latch happens no earlier than the old frame's last
active line is fetched, or the shortest frame stays above 263 us (337 us).

## The board check that confirms it

Item `am62p-board-before` (`target:am62p-camera`, with `target:am62p` and
`target:am62p-console`), on an image of an origin/main commit.

1. **Scene**: glmark2 `[build] use-vbo=true` (1.14 ms a frame, about 880
   swaps a second, about 15 swaps per vsync: the worst ratio of
   swaps to latches) and, as the second look, `[texture]
   texture-filter=nearest` (1.07 ms). Run each long enough for the camera:
   hold the scene on the panel (`tools/board.sh am62p gm` on a single-scene
   list, or the sample with `CONFIG_YAKOGL_SAMPLE_DISPLAY_DUMP`, whichever
   the item has; the existing `log_dss_stats` hold,
   `driver/zephyr/lib/board/common/sample_glue.c:212`, prints the counters
   at the end of the hold).
2. **Camera**: `take_screenshot` (`reuse_detection=true`, 10 or more
   shots at different times) and look at the bottom 10 to 13 lines of the
   picture first (1.7 to 2.2 percent of the height: the only place the
   read-ahead can tear), then the whole picture for a horizontal
   discontinuity (the model on a rotating, high-contrast scene shows a
   step). The camera exposure covers several panel frames, so a clean
   result proves less than the counters; a torn bottom strip in a still
   scene would be a hazard.
3. **Console records** (`dss stats ...` line, `sample_glue.c:221`) at the
   end of the hold:
   - `underflows 0` and `sync_lost 0` (the last board run:
     `vsyncs 20008 app_frames 152650 underflows 0 sync_lost 0 go_timeouts 0`,
     `am62p-flip-latch.md` "Board result"); an underflow means the FIFO ran
     dry (the opposite problem) and any value other than 0 is a finding
     of its own.
   - `go_timeouts 0`; `go_waits` close to `app_frames - vsyncs` (earlier
     run: 134955 against 132642, +1.7 percent): every swap that did not land
     on a fresh frame end rode along, as the mailbox assumes.
   - `vsyncs` over the hold = 58.85 per second (59 within 1 percent): the
     figure of the 16.99 ms frame this note uses.
   - `safety_flips 0` (so no second writer's GO ends a frame early).
   - `eglSwapBuffers: the display held every window buffer` at most once,
     and `still holds every window buffer` 0 times (`egl.rs:2316-2321`).
4. **Read-ahead depth**, to replace the computed 10 lines by a measured
   value: with `CONFIG_YAKOGL_SAMPLE_DISPLAY_DUMP` (or a debug read of the
   register at init) log `DSS_VID_BUF_SIZE_STATUS` (`regs.rs`
   `vid::BUF_SIZE_STATUS`, offset 0x38) and the `pipe fifo high/low/preload`
   debug line of `dss_init` (`dss.rs:227`). Expected: `BUFSIZE = 0xA00`,
   `high 2559 low 2552 preload 2552`. A different `BUFSIZE` scales the
   window: lines = BUFSIZE x 16 / 4096.

Expected result for "safe": underflows 0, sync_lost 0, go_timeouts 0, no
discontinuity in the bottom strip in 10 shots of either scene. A
discontinuity in the bottom strip with the counters at 0 is "hazard": file
a defect (title: "AM62P: DSS reads the old buffer after the latch, torn
bottom lines in light scenes") and the fix is to hold a released buffer for
one more frame end (treat `app_addr_shown` as busy until two GO-clear
reads, or one `wait_vblank`).

## A host test that would pin the release order (proposal, not added)

A new file next to `driver/rust/src/soc/am62p/dss/tests.rs` (for example
`dss/release_order.rs`, `#[cfg(test)] mod` in `dss.rs` beside `mod tests`),
on `Am62pModel`, public functions only (`dss_init`, `output_enable`,
`set_app_buffer`, `app_busy`, `Am62pModel::vsync`):

1. Show A, `vsync()`; `set_app_buffer(B)`: `app_busy(&[A, B])` has both
   bits. Without a `vsync()` it stays so for any number of `app_busy`
   calls (A is never early-free).
2. `vsync()`, then `app_busy`: A free, B busy. Pins "released by the
   frame end that latched the successor".
3. `set_app_buffer(C)` over B unlatched: B free at once, A still free
   (the dropped-in-the-mailbox case).
4. With `Am62pFaults { go_stuck: true }`: A and B stay busy through any
   number of `vsync()` calls.

The model has no FIFO, so a host test cannot pin the read-ahead itself;
that is the board's measurement. (Most of 1, 2 and 4 are covered in
`a_buffer_reported_free_is_never_scanned_whatever_happens_between_two_accesses`,
`many_flips_between_two_frame_ends_keep_a_buffer_free` and
`with_go_stuck_the_scanned_and_the_waiting_buffers_stay_busy`; a
readable four-step test names the order for the next reader.)

## Not covered

- The wait_vblank decision (`am62p-vblank-decide`).
- Whether `VPVSYNC_IRQ` falls before or after the latch inside the frame
  end: the driver only ever reads GO, never infers the latch from the
  interrupt's time.
- A second writer: the safety layer (`safety_flip`) shares the GO bit; with
  `safety_flips 0` it never acts.
