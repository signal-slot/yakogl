<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: a flip that never scans the buffer being drawn

2026-10-18. Plan #150 item `am62p-flip-latch-design`, read at main
`3adf39e3`. The then hand-over ("New: the AM62P DSS flip does not
wait"; board run now #1211) asks for a choice between a fourth buffer and a `present` that
reports the latched buffer. Option C is implemented (item
`am62p-flip-latch-impl`); "Implemented" below says where the code differs
from the steps. No board run yet.

## The finding in one paragraph

The AM62P `present` does not wait and does not say what the panel scans.
The library rotates its buffers blindly (`(cur + 1) % 3`, `src/gl/egl.rs`
`swap_window_buffers`) and assumes the buffer it draws into next was given
up by the display two presents ago. On the DSS that holds only when the
display latched the flips within two swaps. **A fourth buffer in the same
blind ring does not fix this**: a ring of N buffers is safe up to N-1 swaps
per vsync, and the light glmark2 scenes run at hundreds of swaps per second
against about 59 vsyncs per second (5 to 10 or more swaps per vsync, no
bound). The fix is to let the library know which buffers the display may
still scan, and to pick the next buffer from the others. That needs four
buffers (not three) to stay non-blocking. **Recommended: option C.**

## How the DSS latches a new address

Code read: `driver/rust/src/soc/am62p/dss.rs`,
`driver/rust/src/ip/ti_dss7.rs`, `driver/rust/src/soc/am62p/regs.rs`,
`driver/rust/src/model_am62p.rs`,
`driver/drivers/display/display_am62p_dss_rust.c`.

- The flip is `board_show` (`driver/zephyr/lib/board/am62p/board_am62p.c`)
  -> `pvr_sample_flip_window` (`driver/zephyr/lib/board/common/sample_glue.c`)
  -> `am62p_dss_set_app_buffer` -> `soc::am62p::dss::set_app_buffer`. The
  first flip also enables the plane and the port (`s_shown`).
- `set_app_buffer` writes the VID pipeline registers (`DSS_VID_BA_0`,
  `DSS_VID_BA_EXT_0`, sizes, `ATTRIBUTES`) through `plane_configure`. They
  are shadowed (TRM 12.9.1.4.1.13). Then `commit` sets `GOBIT` in
  `DSS_VP_CONTROL` and returns. It **never waits**.
- The hardware copies the shadow to the active registers at the next frame
  end of the running port and clears `GOBIT`. Until then the panel scans the
  old address. After it, the new one.
- Two writers share one `GOBIT` (the application plane and the safety
  layer). If `GOBIT` is still set, `commit` returns `Go::RodeAlong`: the new
  values are written over the pending ones and latch together (`go_waits`
  counts it). So **flips between two vsyncs overwrite each other: only the
  last one is ever scanned.** The earlier ones are dropped. This is a
  mailbox, which is what makes option C possible.
- `write_and_go` rewrites the commit once if a vsync went by while it was
  being written (the latch may have taken a mix).
- The interrupt (`dss_isr` -> `latch_settled`) sets
  `app_seq_shown = app_seq_pending` when it finds `GOBIT` clear. The seq
  comes from `am62p_dss_plane_cfg.seq`; `plane_init` in `sample_glue.c`
  never sets it (always 0), and nothing in the board layer reads
  `am62p_dss_app_seq_shown`. The driver keeps no "scanned address".
- Panel: 1024x600 at about 58.8 Hz (`dss/tests.rs` `board_cfg`: pixel clock
  51.2 MHz, 1349 x 645 total). One XRGB8888 buffer of the panel size is
  2400 KiB. glmark2 windows are 800x600 by default (1875 KiB) and up to
  1920x1080 (8100 KiB, `CONFIG_YAKOGL_BOARD_WINDOW_MAX_KB`).
- There is no `wait_vblank` on the AM62P OSAL (the weak
  `yakogl_zephyr_wait_vblank` in `src/osal/zephyr.c`), so `eglSwapInterval`
  is accepted and ignored and `present` returns 0. The driver has
  `am62p_dss_wait_vsync`, not wired to EGL.

## Why the panel can scan the buffer being drawn

At swap k (`egl.rs`): frame k's buffer is on the GPU; the swap presents the
buffer of frame k-1; it binds `bufs[(cur + 1) % 3]` for frame k+1. That is
the buffer of frame k-2. It is safe only if the display has left it, which
needs the flip of k-1 (made during this swap) to be latched. On the DSS the
flip is latched at the next vsync. Swaps run at CPU/GPU speed, so with more
than two swaps per vsync the flips of k-2 and k-1 are both still shadow
only, the panel still scans an older buffer, and the ring comes round to it
at the third swap. The read-back pixels are right (the GPU draws what it
was told); only the panel can show a half-drawn frame. The i.MX8MP does not
have this: its `present` waits for the LCDIF latch, so the display has
always given up k-2.

## The options

"Light scenes" are the glmark2 scenes at hundreds of fps.

### A. A fourth buffer in the same blind ring

- Memory: +1 buffer (1875 KiB at 800x600, 2400 KiB at the panel size, 8100
  KiB at 1920x1080) from the render carve-out.
- Safe up to 3 swaps per vsync. A light scene makes 5 to 10 or more. **Does
  not remove the defect**, only moves it to higher rates. Rejected.

### B. `present` waits for the latch (the i.MX8MP way), three buffers

- `board_show` blocks until the flip is latched (`app_seq_shown` reaches the
  flip's seq, or `am62p_dss_wait_vsync`) and returns 1, so that swap
  interval 1 does not wait twice.
- Memory: none. Correct at any rate.
- ms/frame: every swap takes up to one vsync period, so **no scene is
  faster than about 17 ms/frame** (59 fps). The light scenes are 2 to 5 ms
  today. The per-scene ms/frame table would read the refresh period for
  them and the 462 score would collapse towards 59 per scene lighter than a
  vsync: the benchmark becomes a vsync meter. Rejected for the benchmark.
  (It is what swap interval 1 means on a real display; see "Left".)

### C. Four buffers and a query of what the display may still scan (recommended)

The display tells the library which of its buffers it may still scan. The
library draws only into others. The flip stays non-blocking and uses the DSS
mailbox: a flip that replaces an unlatched one drops that frame, and the
dropped buffer is free at once.

Roles at the moment the buffer for frame k+1 is chosen (inside swap k):
`D`, frame k, on the GPU; `P`, frame k-1, just presented (shadow only or
latched); `S`, the buffer the panel scans, when it is not `P`. The buffer
for frame k+1, `R`, must be none of them: **four different buffers**. When
swap k+1 then presents `D`, an unlatched `P` is overwritten in the mailbox
and is free again for frame k+2, and a latched `P` has made `S` free, so
one buffer is free at every swap and nothing waits. Three buffers are enough
only if the display waits (option B); four is the minimum for a
non-blocking mailbox.

- Memory: +1 buffer, on the AM62P only. Boards without the query keep three.
- ms/frame: no wait is added in any path. The swap is today's swap (no end
  wait, no vsync wait), so the light scenes keep their frame time. Expected
  score: the 462 of `e9f562d8`, within noise (the host cannot show it; the
  board cell check does).
- Timeline (`docs/notes/egl-swap-timeline.md`, timeline 3): unchanged. The
  swap still submits k, waits for k-1, shows k-1, returns. Only the choice
  of the next buffer changes.
- The panel shows at most one new frame per vsync, always a complete one
  (the newest finished before the latch). Dropped frames are not seen;
  glmark2 measures drawing, so its numbers do not change.
- Correctness needs the report to be **exact or too large, never too
  small** ("The race the report must close").

### D. `present` returns the latched buffer in its return value

The same information as C packed into the `int` that `present` returns
(0, 1 or an errno). It cannot name a set of buffers, and ties the answer to
the call while the panel latches a moment later. A separate query is exact
at the moment the library needs it. D is C with a worse interface. Rejected.

## The race the report must close

`latch_settled` (the ISR) treats "`GOBIT` clear at a vsync" as "the pending
commit was latched". That is true only if `GOBIT` was set and then
consumed. `commit` is entered with `GOBIT` clear (the previous one was
consumed). A vsync that arrives after the commit has marked itself pending
(`app_commit_pending`, set in the closure of `set_app_buffer`) and before
`commit` sets `GOBIT` sees "clear" and settles: the new address is reported
as shown while the panel scans the old one. `app_seq_shown` has this hole
today (harmless: nothing reads it). A report that is too large is safe; one
that is too small is not (the scanned buffer would look free).

The driver therefore keeps, in `DssData` next to `app_seq_pending`:

- `app_addr_pending`: the address written by the commit in progress or
  waiting for a latch;
- `app_addr_shown`: the address of the last commit the ISR knows was
  latched;
- `app_addr_unsure`: non-zero while a vsync fell between "marked pending"
  and "`GOBIT` set": that commit's address may or may not have latched.
  The retry in `write_and_go` (it loops when `vsyncs` changed) arms `GOBIT`
  again, and the next settled vsync clears it;
- `go_armed`: set by `commit` **after** the `GOBIT` write returned `Armed`
  or `RodeAlong`; the ISR reads and clears it when it settles. An ISR that
  runs between the `GOBIT` write and the `go_armed` store finds both clear:
  that is "unsure", never "settled".

The query takes the addresses of the surface's buffers and returns a mask
of those that are busy: equal to `app_addr_shown`, `app_addr_unsure` or
`app_addr_pending`. A buffer is free when it is none of these.

## Recommended: option C, steps

1. **DSS driver** (`driver/rust/src/soc/am62p/dss.rs`):
   - `DssData`: add `app_addr_pending`, `app_addr_shown`, `app_addr_unsure`
     (atomics, as `app_addr` is) and `go_armed`. Update the layout pins in
     `driver/rust/src/ffi/am62p.rs` (`offset_of!` asserts) and regenerate
     `driver/rust/include/yakogl_drivers.h` in the same commit (the gate
     compares the bytes; row "yakogl_drivers.h layout pins" of
     `tools/check.sh`).
   - `set_app_buffer`: store `app_addr_pending` inside the closure, where
     `app_seq_pending` is stored, so a retry rewrites it.
   - `commit`: set `go_armed` after `Armed` / `RodeAlong`.
   - `latch_settled`: settle only when `go_armed` is set and `GOBIT` is
     clear: `app_addr_shown = app_addr_pending`, clear `app_addr_unsure`.
     When a commit is pending and `go_armed` is clear, set
     `app_addr_unsure = app_addr_pending`.
   - New `app_addr_busy(d, addrs: &[u64]) -> u32`: the mask above. It reads
     atomics only, so the GL thread can call it without a lock.
   - `ffi/am62p.rs`: `yd_am62p_dss_app_busy`.
     `driver/drivers/display/display_am62p_dss_rust.c` and
     `driver/include/yakogl_drv/am62p_dss.h`: `am62p_dss_app_busy(dev,
     const uint64_t *pa, size_t n)`.
2. **Board layer**: a `board_display_busy(const uint64_t *phys, size_t n)` in
   `driver/zephyr/lib/board/am62p/board_am62p.c` over `am62p_dss_app_busy`
   (declared in the board header `yakogl_board.h`); `common/board.c` gets
   `yakogl_zephyr_display_busy`, next to `yakogl_zephyr_present`, which calls
   it. A board that does not define the hook gets a weak default, and the
   OSAL then reports "no query".
3. **OSAL ABI**: add `uint32_t (*display_busy)(const uint64_t *phys, size_t
   n)` as the last op of `struct osal_ops` (`include/gles_osal.h`, with its
   comment beside `present`: "which of these buffers the display may still
   scan; NULL: `present` waits, or the display does not scan them, and the
   library rotates its buffers as before"), `OsalOps` in `src/osal/mod.rs`
   (`has_display_busy()` and `display_busy(&[u64]) -> u32`, beside
   `has_present`), and the offset pin in `src/abi_layout.rs` and
   `src/abi_layout_check.c`. `src/osal/zephyr.c` supplies it over the board
   hook; `zephyr_v3d.c`, `zephyr_gc.c`, `linux.c`, `baremetal.c` and
   `MOCK_OSAL` in `src/testutil.rs` leave it NULL.
4. **EGL** (`src/gl/egl.rs`; owned by the running item `pi4-end-wait`, so
   do this after it merges): `WINDOW_BUFFERS` becomes the maximum, 4, and
   the surface keeps `nbufs` (3 or 4). `window_buffers_ready` allocates by
   `osal::has_display_busy()`. In `swap_window_buffers` the line
   `next = (cur + 1) % WINDOW_BUFFERS` becomes `pick_free(s)`: after
   `osal::present` of `pending`, ask `osal::display_busy` for every
   `bufs[k].phys` and take the first `k`, searching from `cur + 1`, that is
   not `cur`, not busy. Without the op the search is the ring over three
   buffers, so the pick equals today's. If none is free (cannot happen
   with four), call `osal::wait_vblank(100_000)` and ask again; with no
   vblank source take the ring's buffer and `glog_once!` it. Nothing else
   in the swap changes: no new wait, `finish_previous` as now.
5. **Docs**: the AM62P row of the board table in
   `docs/notes/egl-swap-timeline.md`, and the issue of the
   implementing result (#1211).

## Host test plan

All on the host; no board.

1. **Reproduce first** (new `tests/egl_swap_latch.rs`, not
   `tests/egl_swap_timeline.rs`, which `pi4-end-wait` is changing). A fake
   display that models the DSS: `present` writes a shadow address; every
   17 ms of fake time a vsync copies the shadow to the scanned address (a
   flip over an unlatched one overwrites it). A fake GPU whose frames take
   0.3 ms. Assert for every frame: the buffer it draws into is never the
   scanned address and never the shadow address, from its kick to its end.
   With the current ring this fails once there are 3 swaps per vsync (200
   swaps cover 11 vsyncs). The failure is shown before the fix.
2. **The fix**: the same test with `display_busy` supplied by the fake
   display. The assertion holds for 5000 swaps at swap-to-vsync ratios of
   0.5, 1, 2, 3, 10 and 100, with the vsyncs at random offsets. Also: no
   swap waits for a vsync (zero `wait_vblank` calls), no swap waits for its
   own frame's end (the checks `double_buffer_timeline` already makes).
3. **Without the op**: three buffers are allocated and the sequence of drawn
   buffers equals today's ring. This pins the Pi 4 and the i.MX8MP.
4. **Memory**: with the op, `gpu_alloc` is called for 3 buffers besides the
   window's own; without it, for 2. A failed allocation of the fourth falls
   back to the existing "rendered in place" path.
5. **Driver** (`driver/rust/src/soc/am62p/dss/tests.rs` on `model_am62p.rs`,
   which has the shadow, `GOBIT` and the vsync): inject a vsync at each
   step of `set_app_buffer` (before the closure, between the closure and
   `GOBIT`, after `GOBIT`, during the retry) and assert that `app_addr_busy`
   never calls free the address the model's active registers scan. Cover
   `Am62pFaults.go_stuck` (the shadow address stays busy) and two commits
   riding one `GOBIT` (the first address is no longer busy: it was
   dropped).
6. **Gate**: `src/abi_layout*` and `src/gl` reach every row, so
   `tools/check.sh --local <base>` for the implementing result.

## Implemented

Option C, as the steps above, except:

- **The driver's report is kept by the GL thread, not the interrupt.** The
  `go_armed` scheme of "The race the report must close" still has a hole: a
  `go_armed` left set by the previous commit lets the interrupt settle on
  an address whose registers are not written yet, and an interrupt that
  runs after the thread's next commit misses the address it latched. Both
  make the report too small. `set_app_buffer` (`soc/am62p/dss.rs`) instead
  reads the GO bit itself before it writes, and `commit` counts every GO
  write of every writer (`go_enter`, `go_exit`). A frame end latches only
  when it finds GO set and clears it; only a GO write sets it again. So,
  with no other GO write in between: GO clear before a commit means the
  last commit was latched (the plane scans exactly its address); GO set
  before and still set at the commit's GO read, and set since the last
  commit's own GO, means nothing was latched since that commit wrote its
  address, so that buffer was never scanned and is dropped (the mailbox);
  GO set before and clear at the GO read means a frame end latched the old
  or the new address during the write. Anything else (another writer's GO,
  a commit written twice, the port off) keeps the old address busy until
  GO is seen clear. The report is `app_busy` (`app_addr_pending`, `_shown`,
  `_unsure`, and `app_busy_all` when more than three could be scanned). The
  interrupt is unchanged; `app_seq_shown` keeps its old meaning.
- **When the report holds every buffer** (a frame end inside a commit, or
  the safety layer's GO between two application commits, can leave one
  buffer too many busy until the next frame end), `next_window_buffer`
  (`src/gl/egl.rs`) waits up to 6 vertical periods where EGL has a
  vertical-sync source, else asks again for up to 100 ms (the OSAL clock),
  then takes the ring's buffer; each is logged once. The AM62P has no
  vertical-sync source for EGL, so there it asks again until the next
  frame end settles it.
- **The op's wiring.** `src/osal/zephyr.c` puts the weak, undefined symbol
  `yakogl_zephyr_display_busy` in its table: NULL unless the board library
  defines it. `common/board.c` defines it over `board_display_busy` only
  with `CONFIG_YAKOGL_BOARD_DISPLAY_BUSY` (set by the AM62P board with
  `YAKOGL_SAMPLE_SHOW_ON_DSS`); `board_am62p.c` answers through
  `sample_glue.c` `pvr_sample_window_busy` -> `am62p_dss_app_busy`.
- `docs/notes/egl-swap-timeline.md` is not changed: its AM62P row still
  describes the flip, which did not change.

Host tests: `tests/egl_swap_latch.rs` (the plan above, items 1 to 4: the
blind ring of three draws into a held buffer at 3 and 10 swaps a period;
with the op, 5000 swaps at 0.5 to 100 swaps a period, three vsync offsets,
no violation, no vertical-period wait, no wait for the frame's own end),
and `soc/am62p/dss/tests.rs` (item 5: a frame end, a late interrupt or the
safety layer's GO before every register access of an EGL-like run; the
scanned buffer is never reported free and never one being drawn; two
commits on one GO drop the first; `go_stuck`).

## Cost summary

| | memory | light-scene ms/frame | panel can show a half-drawn frame |
|---|---|---|---|
| today (3, blind ring) | 2 buffers beyond the window's | as measured (462) | yes, above 2 swaps per vsync |
| A (4, blind ring) | +1 | same | yes, above 3 |
| B (3, `present` waits) | 0 | about 17 for every scene | no |
| C (4, busy query) | +1 (2400 KiB at the panel size) | same as today | no, if the report is never too small |
| D | as C | as C | as C, worse interface |

## Left

- The board cell check for C: one glmark2 run on the AM62P, ms/frame per
  scene against the 462 run, and a look at the panel (or the display dump)
  for a torn frame. Not done yet (the board item after the
  implementation).
- Whether the DSS has finished fetching the old buffer when the frame end
  latches the new address (line-buffer read-ahead). `ti_dss7.rs` quotes the
  TRM: the new configuration is used "at the end of the current frame", so
  the old frame is complete; the board check can confirm.
- Honouring `eglSwapInterval(1)` on the AM62P (wiring `am62p_dss_wait_vsync`
  as `wait_vblank`) caps every scene at the refresh rate, like option B. It
  is a separate decision and changes the benchmark; not part of this.
- `am62p_dss_plane_cfg.seq` stays unset by `plane_init`; the new fields use
  addresses, not seq.

## Board result

2026-10-19. Plan #150 item `am62p-flip-latch-board`. The cell check for
option C, on the AM62P (Verdin AM62P, one GL client).

- **Main commit:** `2483a146` (`2483a1469ae6074b66505a2be4e2a22385b54e3c`),
  which includes the four screen buffers and the `display_busy` op.
- **Image:** the GM image of `tools/cts/common-images.sh` (glmark2, its default
  set), sha256 `c61426b30c1c85252f04f742ba3f5d498bcaa1b1aeafc3b29ea0f2d84f7e923a`.
  Run with `tools/board.sh am62p tree` on the GM row only, `BOARD_UART`-style
  console through the leased serial adapter, `BOARD_BOOT=uboot-mmc`.
- **Record:** `records/am62p/flip-latch-2483a146/20261019-1826-GM-c61426b3/`
  (gitignored; `INDEX.txt`, `console.txt`). Verdict line: `board.sh am62p GM rc=0
  verdict=done boot=uboot-mmc row boot_verdict=done result=record:GLMARK2 resets=0
  record=594b1201,fc3f01cd,6184003f,00258320 back_after=426s unsupported=0`.
- **Score:** `GLMARK2 (18) seq 1: score 461 status 1 (pass) scenes ran 33 failed 0
  unsupported 0 size 800x600 on-screen; ... GL errors raised 0, draws that sampled
  an incomplete texture 0`. Score history, in commit order: 465 at `cd738b1a`,
  289 at `1d54a4eb` (in-place swap), then the EGL double buffer (`1eb9783d`)
  and 462 at `e9f562d8` / `bc43ea09`. This run, 461, is the 462 level: the
  score is back, within noise. The console says `glmark2: the window is
  double-buffered by EGL (shown by the swap)`.

### Per scene, ms per frame

"now" is `common-tally.py --gm-table` (ms/frame from `FrameTime`) on this
record. The per-scene console of the 462 run
(`records/am62p/common-bc43ea09/20261018-1739-GM-47b4a10a`) is not on the node
that ran this session, so no `--gm-diff` could be made. The "before" column
is therefore **not** the 462 run for 30 of the 33 scenes: it is 1000 / fps of
the `cd738b1a` run (score 465; table in `docs/notes/stage2-bench-gap.md`).
`1eb9783d` is not an ancestor of `cd738b1a`, so that run used the in-place
swap and the 64 MiB carve-out, and it raised 2573 GL errors
(`docs/notes/stage2-bench-gap.md`). Its fps are whole numbers, so those ratios
carry up to 1 percent of rounding. Only terrain, shadow and refract are the
figures `tools/cts/RESULTS.md` cites for the 462 run (93.9, 353, 375).

| # | scene | now ms/frame | before ms/frame | ratio |
|---|---|---|---|---|
| 1 | [build] use-vbo=false | 9.98 | 9.80 | 1.02 |
| 2 | [build] use-vbo=true | 1.14 | 1.14 | 1.00 |
| 3 | [texture] texture-filter=nearest | 1.07 | 1.06 | 1.00 |
| 4 | [texture] texture-filter=linear | 1.07 | 1.07 | 1.00 |
| 5 | [texture] texture-filter=mipmap | 1.09 | 1.09 | 1.00 |
| 6 | [shading] shading=gouraud | 1.42 | 1.42 | 1.00 |
| 7 | [shading] shading=blinn-phong-inf | 1.66 | 1.65 | 1.01 |
| 8 | [shading] shading=phong | 2.07 | 2.06 | 1.00 |
| 9 | [shading] shading=cel | 2.43 | 2.42 | 1.01 |
| 10 | [bump] bump-render=high-poly | 3.03 | 3.04 | 1.00 |
| 11 | [bump] bump-render=normals | 1.19 | 1.18 | 1.01 |
| 12 | [bump] bump-render=height | 1.42 | 1.41 | 1.01 |
| 13 | [effect2d] kernel=0,1,0;1,-4,1;0,1,0; | 5.81 | 5.78 | 1.01 |
| 14 | [effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1; | 8.99 | 8.93 | 1.01 |
| 15 | [pulsar] light=false:quads=5:texture=false | 1.13 | 1.12 | 1.00 |
| 16 | [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4 | 21.32 | 21.28 | 1.00 |
| 17 | [desktop] effect=shadow:windows=4 | 6.92 | 6.90 | 1.00 |
| 18 | [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map | 16.14 | 16.13 | 1.00 |
| 19 | [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata | 16.38 | 16.13 | 1.02 |
| 20 | [buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map | 14.45 | 14.49 | 1.00 |
| 21 | [ideas] speed=duration | 5.74 | 5.56 | 1.03 |
| 22 | [jellyfish] <default> | 6.30 | 6.33 | 1.00 |
| 23 | [terrain] <default> | 93.97 | 93.90 | 1.00 |
| 24 | [shadow] <default> | 353.08 | 353.00 | 1.00 |
| 25 | [refract] <default> | 375.44 | 375.00 | 1.00 |
| 26 | [conditionals] fragment-steps=0:vertex-steps=0 | 1.06 | 1.05 | 1.01 |
| 27 | [conditionals] fragment-steps=5:vertex-steps=0 | 1.83 | 1.83 | 1.00 |
| 28 | [conditionals] fragment-steps=0:vertex-steps=5 | 1.06 | 1.05 | 1.01 |
| 29 | [function] fragment-complexity=low:fragment-steps=5 | 1.27 | 1.27 | 1.00 |
| 30 | [function] fragment-complexity=medium:fragment-steps=5 | 1.79 | 1.79 | 1.00 |
| 31 | [loop] fragment-loop=false:fragment-steps=5:vertex-steps=5 | 1.35 | 1.35 | 1.00 |
| 32 | [loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5 | 1.36 | 1.35 | 1.01 |
| 33 | [loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5 | 3.89 | 3.88 | 1.00 |

Sum of the 33: 966.85 ms/frame. Every scene is within 3 percent of the
`cd738b1a` figure (the largest: ideas 1.03, build use-vbo=false 1.02, buffer
subdata 1.02, all scenes whose fps is one or two counts different in the old
table), and the three scenes that carry the 462 run's own figures are within
0.1 percent. The frame time did not move against the in-place-swap run, and
the 462 level (score) is kept, as the design expected for option C.

### The four-buffer path on the console

- `eglSwapBuffers: the display held every window buffer; the swap waited for
  one to be free`: **present, once** (at the start of texture `nearest`,
  18:27:13). The line is logged once per run (`glog_once!` in `src/gl/egl.rs`),
  so it says that the rare wait happened at least once, not how often; the
  console has no count. After that scene the frame times did not move, so it
  was not frequent enough to show.
- `still holds every window buffer`: **0** lines. The 100 ms fallback to the
  ring never ran.
- `dss stats` at the end: `vsyncs 20008 app_frames 152650 underflows 0 sync_lost 0
  go_timeouts 0 go_waits 134955 safety_flips 0`. Swaps minus vsyncs is 132642;
  `go_waits` is 134955, 2313 more (1.7 percent), so the GO wait count follows the
  swaps that did not land on a fresh vsync, about as expected. No underflow, no
  lost sync, no timeout, no safety flip.
- The console has no line that names the op by name; the use of the busy
  query is shown by the "held every window buffer" line (printed only from the
  `display_busy` path in `src/gl/egl.rs`) and the GO count above. A reader who wants
  a direct line needs a one-time log in the swap; not added (no code change in
  this item).

### Pictures

`common-tally.py --gm-pictures <record> am62p`: `33 scenes, match 29, mismatch 4`.
The tool prints a line for each scene whose crc16 differs from the reference or
whose probe differs; four scenes have the reference crc16 exactly (shading
blinn-phong-inf 1c8a, effect2d 2116 and e4c5, loop fragment-loop=false 787d)
and print no line, so they are also a match.

```
scene [build] use-vbo=false: picture match (crc16 ed63, reference 5d19)
scene [build] use-vbo=true: picture match (crc16 11ce, reference 160d)
scene [texture] texture-filter=nearest: picture match (crc16 9209, reference 7781)
scene [texture] texture-filter=linear: picture match (crc16 97ab, reference b89f)
scene [texture] texture-filter=mipmap: picture match (crc16 3dc6, reference 2e9d)
scene [shading] shading=gouraud: picture match (crc16 63a4, reference 6e03)
scene [shading] shading=phong: picture match (crc16 e5d7, reference 5171)
scene [shading] shading=cel: picture match (crc16 f09a, reference 0f93)
scene [bump] bump-render=high-poly: picture match (crc16 4cea, reference 6761)
scene [bump] bump-render=normals: picture match (crc16 e006, reference efb3)
scene [bump] bump-render=height: picture match (crc16 f6d8, reference 1ee6)
scene [pulsar] light=false:quads=5:texture=false: picture match (crc16 e07a, reference cef1)
scene [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4: picture match (crc16 d4a8, reference 4f6d)
scene [desktop] effect=shadow:windows=4: picture match (crc16 93ae, reference 5362)
scene [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map: picture mismatch (probe c b4d9f0 vs cce5f5 (off 24))
scene [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata: picture mismatch (probe c b4d9f0 vs d8ebf7 (off 36))
scene [buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map: picture mismatch (probe c cde6f5 vs b4d9f0 (off 25))
scene [ideas] speed=duration: picture match (crc16 d74f, reference 3561)
scene [jellyfish] <default>: picture match (crc16 4755, reference 0e1e)
scene [terrain] <default>: picture mismatch (probe c a7b223 vs 676d13 (off 69))
scene [shadow] <default>: picture match (crc16 06b0, reference e53f)
scene [refract] <default>: picture match (crc16 0cd3, reference d9d9)
scene [conditionals] fragment-steps=0:vertex-steps=0: picture match (crc16 6361, reference b5cd)
scene [conditionals] fragment-steps=5:vertex-steps=0: picture match (crc16 193c, reference a532)
scene [conditionals] fragment-steps=0:vertex-steps=5: picture match (crc16 f0ab, reference 7984)
scene [function] fragment-complexity=low:fragment-steps=5: picture match (crc16 188f, reference f5e7)
scene [function] fragment-complexity=medium:fragment-steps=5: picture match (crc16 20de, reference 08a8)
scene [loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5: picture match (crc16 4fd2, reference 787d)
scene [loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5: picture match (crc16 bf21, reference 787d)
```

The four mismatches are the centre probe of animated scenes: the three
`buffer` scenes (the wave, probe c off by 24, 36 and 25 of 255) and terrain
(probe c off by 69). The reference column notes that "animated scenes are
sampled at the last frame, so one probe moves between runs (e.g. ... buffer,
terrain ...)". The other four corner probes and the mean of each of these four
equal the reference (buffer 42 57 65 against 41 56 66 / 42 57 66; terrain 119
129 44 against 119 129 44), and none is black. They were not judged by eye: the
run was a one-shot boot that ended before a camera look, so **no tear check on the
panel was made**; torn frames are not excluded by this record. What it does show
is that no scene is black and none lost its content under four buffers. The
reference table `tools/cts/glmark2-pictures.txt` is not changed here.

### What this shows, and what is left

- The score history is 465 (`cd738b1a`), 289 (`1d54a4eb`, in-place swap), and
  462 (`e9f562d8`, after the EGL double buffer). With four buffers and
  `display_busy` the score stays at 461, the 462 level, and no scene moved
  against the `cd738b1a` figures beyond 3 percent: option C costs no speed at
  800x600. A scene-by-scene comparison with the 462 run's own console is not
  made (that console is not on this node); `common-tally.py --gm-diff` on
  `records/am62p/common-bc43ea09/20261018-1739-GM-47b4a10a` would make it.
- The display held every buffer at least once (the log-once line), the frame
  times show no cost, and the run ended with 0 underflows and 0 sync losses.
- Not done: a look at the panel (or the display dump) for a torn frame in a
  light scene (the first "Left" item above); the line-buffer read-ahead question.
