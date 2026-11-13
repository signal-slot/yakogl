<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: W13, the window tiled and resolved at the swap (design)

Design only, no code and no board access (plan #355, item w13-window-design).
It works out `gc-tiled-wire.md` W13 and `gc-tiled-targets.md` step 6 against
the W4 code that landed (`src/vivante/tiled_rt.rs`, `backend.rs`) and
`src/gl/egl.rs`, read at the tip of this item (86b4af69). W13 is optional and
last: it needs W8 (`gc-tiled-wire.md`, "Order, dependencies and risk"), and no
number below is measured. Line numbers are those of this tip.

## What W4 does today, and what stops it at the window

- One stream tracks one shadow: `tiled_rt::Track { spec, shadow, phase }`
  (`src/vivante/tiled_rt.rs:89-95`), `g.trt` in the backend. `Phase` is `None`,
  `Active` or `InStep` (`:77-86`).
- A draw gets a shadow from `tiled_rt_spec` (`src/vivante/backend.rs:618-632`).
  It asks the knob, one 4-byte target, not 64-bit, not through the 4x scratch,
  A8R8G8B8 or X8R8G8B8 in the PE, the depth test on, and
  `ds_cache::pe_private` (`:626`). Then `tiled_rt::spec_of`
  (`tiled_rt.rs:140-147`) wants width a multiple of 16 and height of 4.
- `tiled_rt_enter` (`backend.rs:2594-2626`) tiles the storage in with the RS
  unless the draw opens with a whole colour clear (`clears_whole_colour`,
  `:637-644`), and keeps `RESERVE_WORDS` (48, `tiled_rt.rs:53`) free for the
  write-back.
- `frame_submit` (`backend.rs:2498`) ends an `Active` shadow with the RS
  tiled→linear copy (`tiled_rt_untile`, `:2575-2578`; called at `:2523-2525`)
  before the stream's `pad` and `submit`. So the copy is in the same kick as
  the draws, and the kick's fence covers it.
- **`pe_private` refuses the window**: bit 31 of the depth id is the window's
  (`src/vivante/ds_cache.rs:263-270`; the id is made at
  `src/gl/draw.rs:2685-2688`, one D24S8 for depth and stencil, keyed by the
  buffer's page number). The test `only_a_depth_renderbuffer_is_private_to_the_pixel_engine`
  keeps it so (`backend_tests_tiled_rt.rs:495-506`). So with W4 as built, no
  window draw gets a shadow, whatever the knob says.

W13 therefore changes two decisions (the depth rule, and the spec of the
target) and adds a lifetime rule (the shadow follows the swapped buffers). It
adds no new stream element: the frame-end resolve is W4's.

## (1) The EGL back buffer gets a 32-bit TILED shadow

**What a "window target" is.** A double-buffered window has three or four
buffers (`window_buffers_ready`, `src/gl/egl.rs:2248-2267`: `WINDOW_BUFFERS`
where the display says which it still holds, else 3; buffer 0 is the window's
own). Each swap rebinds the context to another one
(`swap_window_buffers` → `gl_context_bind_surface(&mut s.bufs[next], ...)`,
`egl.rs:2337-2340`), so `GlesDraw::target_gpu` takes one of three or four
values in turn, and the depth id (page number of `target_gpu`) with it
(`draw.rs:2685`).

**Shadow per buffer or one for the window.** The shadow is keyed by
`Spec { gpu, w, h, pitch }` (`tiled_rt.rs:57-65`), and `shadow_of`
(`tiled_rt.rs:171-211`) keeps `ENTRIES` = 4 (`:50`) of them, giving up a
different shadow of the same `gpu` and the least recently used one. Three
window buffers would take three of four entries and starve the FBO shadows
(terrain's pass), and each would cost a window-sized allocation. The design:

- The shadow is **per window surface, not per buffer**. The rule is that a
  window's frame is drawn into one buffer and resolved at its own end, so the
  shadow is dead between frames (see (2): nothing in the shadow survives a
  swap, because every frame's stream ends with the resolve, and a frame
  begins with a clear or a tile-in). The key is therefore the window's
  *size and pitch*, not the buffer: `tiled_rt::shadow_of` gets a window
  entry class whose `gpu` field is a stable per-surface token (the surface's
  `bufs[0].gpu`, which the backend sees as `GlesDraw::target_gpu` only for
  buffer 0, so the token has to come with the draw: one new field,
  `GlesDraw::shadow_token`, set by `resolve_ds_storage`'s caller when
  `bound_framebuffer == 0`; an FBO leaves it 0 and `spec_of` keeps
  `target_gpu`).
- The `Spec` equality of `tiled_rt_enter` (`backend.rs:2600`) then compares
  token, size and pitch. The *destination* of the resolve is still
  `Spec::gpu`'s buffer of the frame, so `Spec` gets a second field,
  `dst: u64` (the buffer), that is not part of the shadow's key but is part of
  `Track`'s identity: a draw into another buffer in the same stream ends the
  old copy first, exactly as a draw into another target does today
  (`backend.rs:2607-2609`).
- Memory: one 4-byte shadow per window, `stride(w) * h` bytes
  (`tiled_rt.rs:72-74`, `:200`), see (4).

**The depth side.** The shadow is useless for early depth (the reason to do
W13 beyond the clears) without a super-tiled depth, and a TILED colour target
under SINGLE_BUFFER(2) wants one (`gc-tiled-wire.md` W4). The window's D24S8
is `pe_private`-refused because the CPU and the GL core can read it:

- It persists across kicks and swaps: `surface_swapped` is `None` for
  this backend (`backend.rs:3871`), so EGL's "undefined after swap"
  (`egl.rs:2367-2373`) is not exploited and the contents stay.
- A layout change is refused while a plane holds contents
  (`ds_cache.rs:249-251`, `select_in` at `:314-352`).
- CPU readers of a non-linear surface refuse (`resolve_depth` `:614-618`,
  `:710`).

The rule W13 adds to `pe_private`: the window's depth qualifies when the
window is double-buffered (`EglSurface::back`)
and the context has **no CPU reader of the default framebuffer's depth**:
ES 2.0 has none (glReadPixels reads colour, `framebuffer.rs:2931`); ES 3.0's
glBlitFramebuffer of depth from the window, and `glInvalidateFramebuffer` on
the window, go through `gpu_idle_for_cpu` (`blit.rs:1240`) and
`resolve_depth`. So the predicate takes the window's depth only when the
draw's `GlesDraw` says the default framebuffer's depth is never read: a flag
the GL core sets (one `GlesDraw` bit, cleared by a blit with a depth
source or a read of the default framebuffer's depth), and a `layout_free`
check (`ds_cache.rs:249`) that refuses the change while the plane holds
contents. When the check refuses, the draw keeps the linear path; that is the
same fallback W4 already has for an FBO.

A window that draws without the depth test (a 2D UI, Slint's cluster) never
qualifies (`tiled_rt_spec`'s `!gl.depth_test`, `:627`), so only 3D windows
change. Windows scanned in place (`GLES_NATIVE_WINDOW_SCANOUT`, a view,
`board_imx8mp.c:318-346`) and `pipelined` ones are **excluded**: they are
shown right after the swap, and `complete` (`egl.rs:2405`) finishes the frame
at the swap, so for them the resolve would write into the buffer the LCDIF
scans (see (3)). They keep the linear path.

## (2) Where the frame-end resolve goes

**It is already where W13 needs it.** The order in `eglSwapBuffers`
(`egl.rs:2352-2437`) is:

1. `flush_pending_clear()` (`:2369`): a `glClear` no draw followed goes out
   as a clear-only draw. With W13 that clear may be a shadow draw that opens
   the frame (no tile-in, `backend.rs:2610-2612`).
2. `backend::flush()` (`:2396`) → `gc_flush` (`backend.rs:3658`) →
   `frame_submit`: the open stream, which ends with
   `tiled_rt_untile` (`backend.rs:2523-2525`) while `g.trt.phase ==
   Active`, then `submit`. This is the kick the window's frame ends in.
3. `kick_id` and the `idle` test (`:2411-2412`).
4. `finish_previous` (`:2413`) or, for `complete` windows and the
   no-new-kick case, `finish`: it waits for **every kick up to the previous
   swap**, not the one just submitted (`gc_finish_previous`,
   `backend.rs:3710-3720`, the doc comment at `:3700-3709`).
5. `swap_window_buffers` (`:2419`, fn at `:2325`): presents `pending`, the
   **previous** frame's buffer, then binds the next buffer.

So "the last kick before `finish_previous`" is step 2's kick, and it carries
the untile because `frame_submit` records it. Nothing new is placed in the
swap path. What W13 must guarantee:

- **The window's draws are in one `Active` stretch ending at `flush`.** A
  draw that does not take the shadow (a linear draw, a draw into an FBO, a
  sampler of the window as texture) ends the stretch with an untile
  (`tiled_rt_enter`, `backend.rs:2615-2622`) and the shadow is `InStep`; a
  later window draw sets it `Active` again with no tile-in (`:2600-2604`). The
  stream's final state is written back by `frame_submit`. Count: one
  untile per stretch, so a frame of window draws alternating with FBO draws
  costs as many window-sized copies as it has stretches (the FBO-first
  scenes, terrain's shadow pass before the window, have one).
- **The stream may split before the swap** (`frame_submit` is also called by
  a full buffer and by `finish`): each split ends with the untile, and the
  next kick's first window draw tiles in unless it clears. Only the case
  "split mid-frame, no clear" pays the extra tile-in, and `RESERVE_WORDS` keeps
  room for the untile (`tiled_rt.rs:53`).
- **`swap_window_buffers` reads resolved pixels** only through `present`, and
  it presents `pending`. `pending` is the buffer whose kick was submitted at
  the *previous* swap and was waited for by *this* swap's `finish_previous`
  (the fence `gc_finish_previous` waits for is `newest` of the previous call,
  `backend.rs:3712-3713`). The untile is in that kick's stream, so by the
  time `osal::present` copies or scans it the resolve is done. The buffer
  the current kick's resolve writes (`cur`) is not touched until the next
  swap. This is the ordering of the linear path today: its colour writes
  are in the same kick.
- **`idle`** (`egl.rs:2411-2412`, a swap with no new kick) waits for the
  last kick with `finish`: nothing was recorded, so no shadow is `Active`
  and the previous frame's resolve is in that kick. Unchanged.
- **`complete` windows** (single-buffered or pipelined are excluded, above);
  if a future change includes them, `finish` (`:2413`) waits for the resolve
  and the copy follows, which is why only the LCDIF case in (3) matters.
- **glReadPixels on the window** calls `read_pixels_sync` →
  `gpu_idle_for_cpu` (`framebuffer.rs:2931`, `:3002-3010`):
  `flush_pending_clear`, then `backend::flush()` (which submits the stream
  *with the untile*) and `backend::finish()` (`backend.rs:3678-3691`: submit,
  then wait for **all** frames). The CPU read comes after
  both, so it reads the resolved storage. Likewise glBlitFramebuffer from or
  to the window (`blit.rs:1240`) and glCopyTexImage2D. Nothing that reads the
  window's storage uses the shadow, because the shadow is only ever read by
  the PE and the RS (`tiled_rt.rs:17-18`).
- **A texture of the window** (a render-to-window then sample trick) is not
  a thing in EGL window surfaces: the texture classes see only FBO
  attachments.

What W13 must not do: leave a window shadow `Active` across `flush`. The
backend's `finish`/`flush`/`kick_pending` paths all go through
`frame_submit`, and `g.trt = Track::NONE` there (`:2526`), so the invariant
is the one W4 already has; the build item adds a debug assertion that
`g.trt.phase == None` at the return of `frame_submit`.

## (3) Why the LCDIF can never scan a buffer being written

The invariants, in the order the frame meets them:

1. **The RS writes the window buffer of frame N only inside frame N's kick**
   (the untile is in the same stream, (2)).
2. **Frame N's buffer is not given to the display before the kick has
   completed.** Swap N submits frame N's kick and sets `s.pending = s.cur`
   (`egl.rs:2345-2347`); the buffer is presented by swap N+1, after swap
   N+1's `finish_previous` has waited for frame N's fence (`:2413`,
   `backend.rs:3710-3720`), and swap N+1 itself only *submits* frame N+1.
3. **The buffer frame N+1 draws and resolves into is never one the display
   holds**: `next_window_buffer` (`egl.rs:2280-2318`) picks, right after
   presenting `pending`, the first buffer after `cur` that
   `osal::display_busy` does not report (`:2288-2295`), or the next in the
   ring of three where the display has no such report; it waits vertical
   periods (up to `BUSY_WAIT_PERIODS`, `:2300-2316`, `:69-70`) when none is
   free. The RS resolve of frame N+1 targets that buffer.

For the glmark2 / X2 window (800x600 copied into the panel's screen buffer),
the LCDIF does not scan the window buffers at all. `board_show` copies the
window with the CPU into a panel-sized carve-out buffer
(`driver/zephyr/lib/board/imx8mp/board_imx8mp.c:441-446`, `memcpy` row by row
from `win->cpu`) and gives the LCDIF that buffer's address; the window
buffer is read only by that `memcpy`, in the swap, after `finish_previous`
(1)-(2). The RS writes only window buffers, never the screen buffer. The
copy's source, `pending`, is complete; the RS resolve in flight writes
`cur`. (`imx8mp-frame-floor.md:296-335` and `:360-390` for the latch thread
and the screen buffers; `x2-lcdif-1080.md:60-66` for the 800x600 copy into
the 1920x1200 buffer.)

What #923 forbids and how W13 keeps clear of it
(`imx8mp-frame-floor.md:308-327`): the broken picture of `faee8812` came from
a show that returned at once while the GPU (or the copy) wrote the buffer the
LCDIF scanned. W13 adds **a writer, the RS, into buffers that are never the
LCDIF's** in the copied case, and it makes the window's colour writes
**later in the kick** (a burst at the end instead of spread through the
draws), so the sole risk it could add is a window **scanned in place**
(`GLES_NATIVE_WINDOW_SCANOUT`, a view: `board_imx8mp.c:318-346`, its
single buffer is the one the LCDIF scans). There the draws already write the
scanned buffer; with the resolve they would write it *only at the end of the
kick*, a 1.9 MB (or 8.3 MB, (4)) burst in one stretch, with the old
"band of the last-drawn rows" failure (`egl.rs:2383-2390`, the comment's
2026-10-16 observation) now being a band of the *un*-resolved rows. So the
design **excludes in-place windows from the shadow**, a one-line test on
`EglSurface::back` that the backend sees as a `GlesDraw` flag
(`window_shadow_ok`), not a board property. The sole window class W13 takes is
the copied or the ring-buffered one, where (1)-(3) hold by construction.

The frame floor is untouched: the floor is the latch wait in `board_show`
(`imx8mp-frame-floor.md:147-195`), not the GPU's time, and W13 changes only
GPU time of the three slow scenes, which run at 250 to 500 ms
(`x2-lcdif-1080.md:67-69`), several periods a frame. A VSYNC-bound scene
that takes the shadow could only get slower by the RS copy, within its
period, and its pictures do not change.

## (4) What it costs, what it saves

**Cost per frame**: one RS resolve at window size (tiled→linear,
`tiled_rt::untile`, one 40-word block, `tiled_rt.rs:107-118`,
`backend.rs:1085-1106`) plus, in a frame whose first window draw does not
clear, one RS tile-in of the same size (`tiled_rt.rs:122-133`), and a
window-sized shadow allocation once (`tiled_rt.rs:200-206`).

| window | pixels | bytes of one RS copy (read tiled, write linear: each) | shadow |
|---|---|---|---|
| 800x600 (glmark2 on the X2 and the Verdin) | 480,000 | 1,920,000 | 1,920,000 (`layout::stride` pads the width to a tile, `layout.rs:127-135`, which is 800 here: 800 % 16 = 0, 600 % 4 = 0, `tiled_rt.rs:140-147`) |
| 1920x1080 (the X2's mode, `x2-lcdif-1080.md:10-17`) | 2,073,600 | 8,294,400 | 8,294,400 (1920 % 16 = 0, 1080 % 4 = 0) |

The X2's LCDIF buffer is 1920x1200 (9,216,000 bytes) and only the 800x600
window of glmark2 or the full 1080 of a 1080 application is drawn into it. An
application at 1920x1200 would add 360 kB per extra 100 rows. A size with
`w % 16 != 0` or `h % 4 != 0` is refused by `spec_of`: the 1366x768 panel of
other boards (768 % 4 = 0, 1366 % 16 = 6) never takes the shadow.

The RS rate is unmeasured (`gc-tiled-wire.md:1080-1081`: "Session A
measures it"); the model used in `gc-tiled-targets.md:683` for 800x600 is "a
1.92 MB RS copy" against the window clear's 1.87 ms
(`gc-tiled-targets.md:325`, `:336`). A tile-in doubles the copy bytes; the
glmark2 scenes clear the window first, so no tile-in.

**What it saves**: the window's draws are tiled (early depth for window
draws, the 4x4 tile writes of the PE instead of linear rows), and
`gc-tiled-targets.md:683-686` models it at terrain about 2 ms (the two window
clears, 3.7 ms, minus one copy), shadow 0 to 1 ms and refract 0 to 1 ms.
That is a model, with W8 (tile-status fast clear of the window) behind it
(`gc-tiled-wire.md` "Order, dependencies and risk": W13 depends on W8, "an
X2 run, every scene"). Without W8 the window clear is a quad (a draw, not a
fast clear) and the gain is only what early depth and the tiled writes give
(W5's range, terrain 0 to 14 ms at most). Break-even: the copy must cost
less than the draws save, which is only measured on the board; if the RS
copy of 1.92 MB costs more than about 1 ms, the shadow and refract scenes
lose, and the window class narrows (the build item's knob: a separate
`gc_tiled_window`, default off).

## (5) Host tests a build item would write

On the TestGl/backend rigs (`backend_tests_tiled_rt.rs`: `Rig`,
`shadow_target`, `rs_jobs`, `rs_kicks_at`, `rs_run`, `:28-130`), with
`gc_tiled_rt` and the new `gc_tiled_window` knob on:

1. `a_double_buffered_windows_last_stream_ends_with_the_rs_resolve`: draw a
   depth-tested frame into the window of an `Egl` rig
   (`src/gl/egl.rs` tests, `Egl::new`, `egl.current`), call
   `eglSwapBuffers`, take the captured streams (`rs_jobs`,
   `rs_kicks_at`): the **last stream submitted before** the swap's
   `finish_previous` holds the RS tiled→linear copy after its last draw, with
   the destination of the buffer that frame drew into and `GC_RS_SOURCE_TILED`
   (no `TILING`, `tiled_rt.rs:107-118`).
2. `the_swap_presents_the_buffer_whose_resolve_has_completed`: with the mock
   fence (`set_kick_pending`, `with_platform(|p| p.fence = ...)`), frame N+1's
   swap presents buffer N only after the kick of frame N is complete: the
   order of present and fence wait in the platform log; and the buffer the
   RS of frame N+1 writes is never one `presents()` (`src/testutil.rs:1867`)
   lists or `display_busy` reports.
3. `glreadpixels_on_the_window_reads_after_the_resolve`: draw, `glReadPixels`
   with no swap: the stream is flushed with the copy at its end, the finish
   precedes the CPU's read (the call order in the mock backend's log:
   `flush`, `finish`), and the bytes equal the linear reference of the same
   draws (the knob-off picture).
4. `the_window_picture_is_the_same_with_the_knob_off_and_on` (pixel compare
   over a depth-tested frame, a clear, an alternation with an FBO draw).
5. `a_scanout_window_and_a_pipelined_window_take_no_shadow`, and
   `a_window_without_the_depth_test_takes_no_shadow` (the knob changes
   nothing: the stream is the knob-off stream byte for byte).
6. `the_window_depth_stays_linear_for_a_blit_from_the_default_framebuffer`
   (the depth rule of (1)) and `a_layout_change_with_contents_is_refused`
   (`ds_cache.rs:249-251`).
7. `the_window_shadow_is_one_per_surface_not_per_buffer`: three swaps, one
   `tiled_rt::held()` (`tiled_rt.rs:236`) of 1, three distinct destinations.
8. Counts in `gc-scene-kicks` style (`tests/gc_scene_kicks_tiled.rs`, owned by
   another item): a window scene's kicks per frame unchanged, RS jobs +1
   (+2 with a tile-in).
9. Every ES 2.0 answer stays: `tests/es2_report_guard.rs` and the
   `tests/es2_*.rs` that reach vivante, with the knob off (the default), as
   for W1-W5.

The host cannot show a tear. These tests show the order of the stream and of
the waits; (3) holds on the board only.

## (6) What a board session has to show

One image from an origin/main commit with W13 behind `gc_tiled_window`,
`gc_wdog_arm: armed`, the rules of plan #323/#328 (no
`NETBOOT_NO_WATCHDOG`, nothing persisted, the board left as found):

- **Before and after, same commit, knob off and on**: ms a frame of terrain,
  shadow and refract (`tools/cts/x2-glmark2-scenes.sh`), and of the 30
  VSYNC-bound scenes, to show none moves by more than its period
  (`x2-lcdif-1080.md:67-69`).
- **The pictures**: every glmark2 scene's picture value (the X2 console
  record words) and, where it helps, the KVM, with the knob on equal to the
  knob off. The tear is the failure of #923: a band of the last rows, or half
  frames. A picture check that passes once proves little; run each scene for
  a few hundred frames and compare the console words at several frames.
- **The RS copy's cost** at 800x600 (the unmeasured rate), by the scene with
  the clear and without.
- **The orders (3)**: that `display_busy` never reports the buffer the next
  frame resolves into (the two once-only log lines of the wait, `egl.rs:2305` and `:2316`,
  must not appear in a run that did not print them with the knob off).
- **Stop**: any picture difference, a hang or a reset ends the session, no
  retry; the board needs a person.
- Not to be done on the X2 as the first step: in-place (view) windows, the
  1080 sink at full size (8.3 MB copies); those are for a later session
  after 800x600.
- **A gate before the session**: session A (`gc-tiled-wire.md`) must be
  green for the FBO shadows, and the tile-status probe (W7-W9) answered, as
  W13 needs W8.

Results go to the notes named in the plan (`x2-glmark2-slow-scenes.md`,
`gc-slow-*.md`), never to `tools/cts/RESULTS.md`, `STATUS.md` or `README.md`.
