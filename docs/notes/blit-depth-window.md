<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glBlitFramebuffer depth: what is open (the window, cube and array sources)

`src/gl/blit.rs` copies depth and stencil on the CPU. It reads and writes a
renderbuffer's plane through `backend::depth_resolve`, `stencil_resolve` and
`ds_write` (the PowerVR implements all three: `pvr_backend_ds_write`,
`pvr_backend_stencil_resolve`), and it reads and writes a 2D depth texture's
R32F texels. Three source cases still answer `GL_INVALID_OPERATION`
(`REFUSED_DEPTH_SRC`):

1. depth read from the window;
2. depth read from a cube depth texture;
3. depth read from an array depth texture.

Writing into the window is refused for the same reason as (1) (`REFUSED_DEPTH_DST`).

**Status (item gl-ds-window):** the window path of the design below is
implemented in `src/gl/blit.rs` (`DepthSource::Window`, `PlaneDst::Window`,
the rows mirrored when `!default_y_up`; host tests
`tests/es3_blit_ds_window.rs`, none ignored; over the real tables
`tests/es3_blit_ds_window_pvr.rs` and `_v3d.rs`, none ignored, and in
`_gc.rs` and `_mali.rs` depth from the window into a depth texture, the one
blit those tables' `depth_resolve` alone allows), so (1) and the window
destination are lifted on a back end with the entries. Still refused: the
window to the window (4.3.3), a multisampled window as a source, window depth
without `depth_resolve` or into the window without `ds_write`, window stencil
without `stencil_resolve` (the PowerVR and, since `582b201c`, the V3D have all
three; the GC7000UL and the Mali have `depth_resolve` only), and (2), (3) and
stencil into or out of a texture (gl-ds-layers). The row order is still read
from the code, not measured (the board checks below).

## What the code says: window orientation per back end (read from the code, not measured)

Every line below was read from the source at `b5f882ea`. Nothing was run, on a
board or on the host; no back end has read a window's depth plane back yet.
Each finding is a hypothesis for the board checks under "Open".

**The GL core's side.** A window draw carries `GlesDraw::flip_y`
(`framebuffer::fb_flip_y`: the default framebuffer, unless it is a pbuffer,
`GlContext::default_y_up`: `src/gl/framebuffer.rs:1360-1362`; set at
`src/gl/draw.rs:2839` and `:4566`; asserted by the tests at `:7021` "the window
surface is stored top-down" and `:7212` "FBO textures are in texel order"). Its
depth/stencil is `GlesDraw::ds_id = depth_id = stencil_id = 0x8000_0000 |
(target_gpu >> 12)`, the page number of the colour buffer the context is bound
to (`resolve_ds_storage`, `src/gl/draw.rs:2699-2711`; the test at `:7111`). It
is one D24S8 object for both planes, and the id changes with every back buffer
a swap rebinds (three or four of them, `docs/notes/gc-tiled-window.md`). The
colour blit already mirrors on the same fact: `Image::flip = !c.default_y_up`
(`src/gl/blit.rs:154`). `ds_format(fb 0)` gives the window's depth image
`target_w` x `target_h` (`blit.rs:339-349`).

| Back end | The window's depth/stencil | Row 0 in memory | Evidence (read from the code, not measured) |
| --- | --- | --- | --- |
| PowerVR (AM62P) | a `Backend::ds` slot keyed `DS_PACKED`, `id = ds_id`; packed D24S8 words, row-linear at the ZLS-tile-aligned width (`pvr_backend_depth_resolve`, `src/pvr/backend.rs:4910-4965`; `owners[DEPTH] = d.depth_id`, `:4510`) | the top of the window | the frame's target is top-down for `flip_y` (`target_flip_y`, `:5235`; the pixel box is "top-down in the target", `:7204-7232`; the winding swap uses `!flip_y`, `:7861`); the ZLS buffer is addressed with the frame's own pixel order, so it follows the target. The read layout was measured for an FBO only (2026-10-01, gl_test 169, comment at `:4896-4900`). |
| V3D (Pi 4) | a `ZsBuf` of the target (`zs_record(target)`, `owners = [depth_id, stencil_id]`, `src/v3d/backend.rs:6309`, `:2793-2794`), UIF D24S8 read back with `v3d_load_tiled_image` (`v3d_depth_resolve`, `:4960-5010`) | the top of the window | the window's viewport carries the flip: `scale[1] = -scale[1]`, `translate[1] = target_h - translate[1]` (`src/v3d/state.rs:242-247`), gl_FragCoord uses `(-1, target_h)` (`:253-259`; the test at `:733-750`: "memory row 0 of a window is the top one"). The doc of `v3d_depth_resolve` says "Row 0 is GL's y 0: an FBO is drawn without flip_y" (`backend.rs:4943`): the FBO half only. |
| GC7000UL (i.MX8MP) | a `ds_cache` entry keyed by both objects (the window's id, bit 31); a window is never PE-private (`pe_private`, `src/vivante/ds_cache.rs:260-270`), so its layout stays `Linear`, the layout `resolve_depth` reads (`:690-740`) | the top of the window | `mirrors(flip_y)` (`src/vivante/backend.rs:1171-1175`) puts the flip in the viewport (`shader.rs:543-559` for gl_FragCoord and the point sprite); only the colour surface can run upwards (`negative_stride`, `:1177-1180`, a `test-knobs` study, `FLIP_STUDY_NEGATIVE_STRIDE`, off by default: `flip_mode()` is 0 without `test-knobs`, `:1158-1169`). The depth surface is addressed in the same pixel order. The `resolve_depth` doc says row 0 is GL's y 0 for an FBO (measured 2026-09-30, `ds_cache.rs:644-650`); the window is not measured. The W13 tiled window (`gc-tiled-window.md`) is a design and leaves depth alone. |
| Mali (NanoPC-T6) | a `ZsBuf` keyed by `ds_id` (the window's id) and a `PlaneEntry` per object (`src/mali/backend.rs:498-524`, `:2625-2711`); linear D24S8 at `4 * w` bytes a row (`mali_depth_resolve`, `:3505-3574`) | the top of the window | the frame builder draws a window (top-down) target "with the flipped viewport" (`frame::DrawCall::flip_y`, `:1449-1450`, set from `d.flip_y` at `:1820`); `mali_depth_resolve` documents "The frame renders a texture target unflipped, so buffer row y is texel row y" (`:3516-3518`), the FBO half only. `frame_flip_tests.rs` compares the colour box only. |

All four read the same way: the window is stored top-down, so memory row `r`
of its plane is GL row `target_h - 1 - r`. Two caveats, also from the code:

- A pbuffer (`default_y_up`) is drawn y-up; its depth is in GL order and needs
  no mirror. The mirror follows `!c.default_y_up`, as the colour blit's does.
- `eglSwapBuffers` leaves the ancillary buffers undefined: the V3D drops the
  window's depth to `Untouched` (`v3d_surface_swapped`, `src/v3d/backend.rs:5025-5050`)
  and the PowerVR drops its buffer (`pvr_backend_surface_swapped`,
  `src/pvr/backend.rs:9173-9200`); the GC7000UL and the Mali have no
  `surface_swapped` hook (`src/vivante/backend.rs:4001`, `src/mali/backend.rs:3692`).
  A window depth blit reads what the frame drew since the swap, under the id
  of the buffer the context is bound to now. A plane held for no id answers 1
  ("holds nothing").

### Which id and entry the window needs

| Need | Window | Today |
| --- | --- | --- |
| read depth | `backend::depth_resolve(id, ..)` with `id = 0x8000_0000 \| (default_surface.gpu >> 12) as u32` (the formula of `draw.rs:2707`), `w = target_w`, `h = target_h` | all four accept an id of that form: PowerVR (`b.ds` by `owners[DEPTH] == id`), V3D (`owners[0] == id`, but only while the buffer is `Stored` and not 4x: `ZsKnown::Stored if !b.msaa`), Mali (`planes` by id), GC7000UL (`mine(key.plane(DEPTH))`, scale 1). A multisampled window (`default_samples > 1`) answers 1 on the PowerVR and V3D, so its blit stays refused until #339's resolve covers it. |
| read stencil | `backend::stencil_resolve(id, ..)`, same id (one object for both planes) | the PowerVR and the V3D (`v3d_stencil_resolve`, `582b201c`) |
| write depth or stencil | `backend::ds_write(id, plane, src, x, y, w, h, pitch)`, same id | the PowerVR and the V3D; `pvr_backend_ds_write` finds the window's buffer by the same `owners` lookup, `v3d_ds_write` by `zs_plane_buf` (the same `owners`; -ENOTSUP for a 4x buffer, so a multisampled window stays refused as unheld) |

No new handle is needed: the GL core builds the id itself. A helper beside
`resolve_ds_storage` that both call keeps the two from drifting (that is
`src/gl/draw.rs`, outside this item).

## Design: the window in `depth_source` and `copy_depth`

1. `DepthSource::Window { id }` and `PlaneDst::Window { id, w, h }` beside the
   renderbuffer variants (`src/gl/blit.rs:828-890`). `depth_source` returns the
   window variant for `r.att.type_ == GL_NONE` (the window's own `DsImage`), and
   the destination match at `:1244-1260` does the same for `d`, gated like a
   renderbuffer by `backend::can_ds_write()`.
2. The source plane is read as for a renderbuffer (`read_plane` into a zeroed
   scratch of `w * h` elements, pitch `w`), then, when `!c.default_y_up`, the
   rows of the scratch are reversed so that it is in GL order. The copy loop of
   `copy_depth` (`:1001-1080`) then stays as it is: it indexes GL rows on both
   sides.
3. A window destination is read whole into its scratch, mirrored to GL order,
   written by the loop, mirrored back, and the cut rectangle written with
   `ds_write` at the memory rows `target_h - (y + h) .. target_h - y`
   (`dst_cut` is cut to `0..target_w` x `0..target_h`, so the rectangle is
   inside the buffer).
4. `dst_writable` probes the window as it does a renderbuffer (`ds_write` of an
   empty rectangle answering 0). A back end without `ds_write` keeps
   `REFUSED_DEPTH_DST`; a source whose back end has no `depth_resolve` keeps
   `REFUSED_DEPTH_SRC`; a plane that answers 1 ("holds nothing") is today's
   `rc == 1`: no error, the destination stays.
5. Stencil through the window follows the same rules: the `rb_src`/`rb_dst`
   helpers (`:1244-1258`) gain the window id, using `stencil_resolve` (the
   PowerVR today; the V3D and GC7000UL entries are the `v3d-ds-write` item and
   its GC7000UL counterpart). The depth-format match at `:1229-1236` already
   treats the window as D24S8.
6. The mirror is the GL core's decision, not the back end's: all four back
   ends draw the window with `flip_y` and, as far as the code says, store it
   top-down.

### Is a `YakoglBackend` flag needed?

Not if the four board checks answer "top-down". The mirror is a function of
`default_y_up`, like the colour blit's. A flag is needed only if a check finds a
back end that keeps the window bottom-up (or depth and stencil in different
orders). Then it is a field of `YakoglBackend` in `src/backend.rs` (plan #368's
file, so only described here), for example `window_ds_top_down: bool` (true:
reverse the rows; false: do not), read through a `backend::` accessor like
`can_ds_write` (`src/backend.rs:2134`). The default `false` keeps the refusal.

Every struct literal to update (`grep -rn 'YakoglBackend {' tests src`, 163
lines at `b5f882ea`; re-run both greps on the latest main when the field is
added, CLAUDE.md "Two green pull requests, red main"):

- the four back ends' tables: `src/pvr/backend.rs:9478` (`yakogl_backend_pvr`),
  `src/v3d/backend.rs:6697` (`yakogl_backend_v3d`), `src/mali/backend.rs:3665`
  (`yakogl_backend_mali`), `src/vivante/backend.rs:3957` (`yakogl_backend_vivante`);
- `src/testutil.rs`: `:1429`, `:1473`, `:1515`, `:1557`, `:1613` spell every
  field; `:1598`, `:1602`, `:1607` use `..MOCK_BACKEND` and need nothing;
- `tests/`: every literal that spells every field. `grep -rl 'stencil_resolve:'
  tests src` finds them (138 files, 142 literals counting `src`); a literal
  with a `..` base needs nothing.

## Open: four single-case board checks (issue texts)

Each is one gl_test case (the next free number; `driver/tools/pvrlab/gl_test.c`
and the per-board mains are plan #368's, so writing the case is the issue's
work, not this note's). The case does the same thing twice. It draws a depth
ramp with the same GL calls, once into the window and once into an FBO with a
D24S8 renderbuffer of the window's size: a quad that covers the target,
`glDepthRange(0, 1)`, the depth test on with `GL_ALWAYS`, and a vertex z equal
to the NDC y, so that the depth at GL row `y` is `(y + 0.5) / H` and no two
rows agree. Then it calls the back end's `depth_resolve` for each plane (the
FBO's object id; the window's `0x8000_0000 | (surface gpu >> 12)`) before any
swap, and prints the depth at memory rows 0 and `H - 1` and whether the plane
is monotonic. The verdict line is `WINDOWDEPTH <backend> fbo_row0=<f>
win_row0=<f> order=top-down|bottom-up|none`. `fbo_row0` is about `0.5 / H`.
The expected line for the design above is `order=top-down` with `win_row0`
about `1 - 0.5 / H`; `bottom-up` means the window is stored like the FBO and
the mirror must not be applied; `none` (a plane answered 1 or an error) is not
a pass. One GL client on the board, the board idle first, an image from a
commit on origin/main (tools/cts/README.md, "Board rules for every run"). The
board is chosen by connected (`target:<kind>`); no address, port or credential
belongs in the issue.

**Issue 1: AM62P PowerVR: single-case check of the window's depth row order**

> glBlitFramebuffer depth from or into the window (docs/notes/blit-depth-window.md)
> needs to know which way up the PowerVR keeps the window's depth. Read from the
> code, not measured: top-down (`flip_y`, `src/pvr/backend.rs:5235`), row 0 =
> the top of the window.
>
> Case: the depth-ramp case above on `target:am62p`, built with
> `tools/cts/common-images.sh --cases <n> am62p ...`. It draws the ramp into the
> window and into a D24S8 FBO, reads both through `pvr_backend_depth_resolve`
> (its layout was measured for the FBO on 2026-10-01, case 169) and prints
> `WINDOWDEPTH powervr fbo_row0=<f> win_row0=<f> order=<...>`.
> Expected: `order=top-down`, `fbo_row0` about 0.5/H, `win_row0` about
> 1 - 0.5/H, both planes monotonic. Also read the window plane after
> `eglSwapBuffers`: expected answer 1 (the PowerVR drops the buffer).
> Result: the verdict line and the image's commit, in this issue and in
> docs/notes/blit-depth-window.md. If `bottom-up`, the PowerVR needs the
> `window_ds_top_down = false` flag.

**Issue 2: Pi 4 V3D: single-case check of the window's depth row order**

> The same question for the V3D. Read from the code, not measured: top-down
> (the viewport carries the flip, `src/v3d/state.rs:242-247`).
> `v3d_depth_resolve` reads the window's `ZsBuf` (`owners[0] == id`) only when it
> is `Stored` (a submitted job stored its depth, single-sampled) and answers 1
> otherwise; a swap drops it to `Untouched`.
>
> Case: the depth-ramp case on `target:pi4`, read before the swap (the GL core
> submits and waits first), `WINDOWDEPTH v3d fbo_row0=<f> win_row0=<f>
> order=<...>`. Expected: `order=top-down`. A `none` on a window that was drawn
> means the job did not store the window's depth: say so in the issue and name
> what made it store (the stored plane is the input of the blit design). The
> FBO half equals the ramp gl_test 169 expects on this board (the V3D
> depth readback is unmeasured until 169 runs there).

**Issue 3: i.MX8MP GC7000UL: single-case check of the window's depth row order**

> The same question for the GC7000UL. Read from the code, not measured:
> top-down (`mirrors(flip_y)` mirrors in the viewport, the depth surface is not
> reversed: `src/vivante/backend.rs:1171-1180`); the window's depth surface is
> `Linear` (never PE-private, `ds_cache::pe_private`), so `gc_depth_resolve`
> reads it row-linear like the FBO's (measured 2026-09-30 for the FBO).
>
> Case: the depth-ramp case on `target:imx8mp`, default configuration (no
> flip study), `WINDOWDEPTH vivante fbo_row0=<f> win_row0=<f> order=<...>`.
> Expected: `order=top-down`. Run it once more with the tiled-target knob on
> (`gc_debug_set_tiled_rt(1)`): the window must stay `Linear` and the line the
> same.

**Issue 4: NanoPC-T6 Mali: single-case check of the window's depth row order**

> The same question for the Mali (Valhall). Read from the code, not measured:
> top-down (the frame builder's flipped viewport for a window,
> `src/mali/backend.rs:1449-1450`); `mali_depth_resolve` reads the window's
> linear D24S8 `ZsBuf` by `ds_id` at `4 * w` bytes a row (`PlaneEntry`).
>
> Case: the depth-ramp case on `target:nanopc-t6`, `WINDOWDEPTH mali
> fbo_row0=<f> win_row0=<f> order=<...>`. Expected: `order=top-down`, with
> `fbo_row0` about 0.5/H (Mesa's case-169 reference has "row 0 holding GL's
> bottom row" for the FBO). A window whose `ZsBuf` is smaller than the target
> (`ds_w < target_w`, `:1467`) is refused before this: say so if the case
> hits it.

## Why this is not finished here

The window row order above is read from the code and no board has run the
four checks (the work had no board and the plan owns none). What remains: the
four checks, `DepthSource` from a cube face or an array layer and stencil
to or from a texture (gl-ds-layers, behind gl-ds-layers-core), the
`ds_write` and `stencil_resolve` entries of the GC7000UL and the Mali, and a
`YakoglBackend` flag only if a check disagrees with the design. The window
path (`DepthSource::Window`, `PlaneDst::Window`) and the V3D's `ds_write` and
`stencil_resolve` are in the tree. How many dEQP-GLES3 cases reach each
refusal is measured in the next section.

## Which mustpass cases reach each refusal (HOST-ONLY, measured at `984a35e3`)

HOST-ONLY: the library runs on the Linux OSAL, which keeps the command stream
and never executes it, so this counts only the cases whose log carries a
refusal line (an API verdict). It says nothing about a picture, a case's
Pass or Fail, or whether a lifted blit would then match; a render verdict from
this run must never be quoted (`tools/cts/README.md` 2b).

**How.** `tools/cts/host-deqp-run.sh` (not edited), `MODULE=gles3 CHUNK=1`, so
every chunk is one case and the refusal line in its own `c_NNN.out` belongs to
that case, once per GPU (`GPU=powervr`, `GPU=v3d`, `GPU=vivante`). The CTS
tree is VK-GL-CTS `93bca01` with `cts-yakogl-target.patch`'s three files and
`external/fetch_sources.py`. Commit measured: `984a35e3`. The causes are the
four log lines of `refuse_ds` (`src/gl/blit.rs:911-938`, said once per
process), matched by their text: `REFUSED_STENCIL`, `REFUSED_DEPTH_DST`,
`REFUSED_DEPTH_SRC`, `REFUSED_DS_UNHELD` (`:896-899`); the file defines no
other `REFUSED_*`.

**Case set run: 2,366 of the 42,810 mustpass cases**, every one began on every
GPU. They are the groups the mustpass names reach `glBlitFramebuffer` in
(`grep -i blit tools/cts/gles3-mustpass.txt`: `fbo.blit` 610, `fbo.color` 72,
`fbo.invalidate` 16, `fragment_ops.scissor` 3, `negative_api.buffer` 2), taken
whole: `dEQP-GLES3.functional.fbo.*` (2,028: api 20, blit 610, color 302,
completeness 560, depth 15, invalidate 131, msaa 70, multiview 3, render 308,
stencil 9), `negative_api.*` (312) and `fragment_ops.scissor*` (26). The
other 40,444 cases were not run; none of their names mentions a blit. (The
script's `split -a 3` allows 1,000 chunks, so `fbo.*` ran as ten sub-groups of
at most 610 cases.)

| Cause | PowerVR | V3D | GC7000UL (vivante) |
| --- | ---: | ---: | ---: |
| `REFUSED_DEPTH_SRC` (depth from a cube face, array layer, multisampled window, or a back end without `depth_resolve`) | 0 | 0 | **0** |
| `REFUSED_DEPTH_DST` | 0 | 0 | 16 |
| `REFUSED_STENCIL` | 0 | 0 | 12 |
| `REFUSED_DS_UNHELD` | 0 | 0 | 0 |
| distinct cases refused | 0 | 0 | 22 |

Before (earlier agents' reports, not in the tree): GPU=vivante at `a1257a1d`:
`REFUSED_DEPTH_DST` 16, `REFUSED_STENCIL` 12, `REFUSED_DEPTH_SRC` 0,
`REFUSED_DS_UNHELD` 0, 22 distinct cases. GPU=powervr: only the six
`fbo.invalidate.*.unbind_blit_*` cases, refused as window destinations. GPU=v3d
after `v3d-ds-write`: 0 in `fbo.blit.*`, 6 in the invalidate window cases.

**The difference, explained.** The GC7000UL is unchanged: 16, 12, 0, 0, the
same 22 cases. The PowerVR and V3D went from 6 to 0: `gl-ds-window` lifted
the window destination (`PlaneDst::Window`), and those six cases were exactly
the window-destination ones, so no mustpass case reaches a refusal there now.
Whether they pass the image compare is not known here (a host read-back is
black).

**GC7000UL, by name (22 distinct cases).**

| Group | Cases | Causes |
| --- | --- | --- |
| `fbo.blit.depth_stencil`, renderbuffer to renderbuffer: `depth24_stencil8_{basic,scale}`, `depth32f_stencil8_{basic,scale}` | 4 | DEPTH_DST and STENCIL |
| the same, `depth24_stencil8_depth_only`, `depth32f_stencil8_depth_only`, `depth_component{16,24,32f}_{basic,scale}` | 8 | DEPTH_DST |
| the same, `depth24_stencil8_stencil_only`, `depth32f_stencil8_stencil_only`, `stencil_index8_{basic,scale}` | 4 | STENCIL |
| `fbo.invalidate.{sub,whole}.unbind_blit_{color,depth,stencil}`, into the window | 6 | `_color`: DEPTH_DST and STENCIL; `_depth`: STENCIL; `_stencil`: DEPTH_DST |

The first three rows are 16 renderbuffer-to-renderbuffer cases that lift with
the GC7000UL's `ds_write` and `stencil_resolve` entries; the six window cases need them and
the window path, which is in the tree. Of the 16 `REFUSED_DEPTH_DST` cases 4
are window blits and 12 renderbuffer blits; of the 12 `REFUSED_STENCIL` cases
4 are window blits and 8 renderbuffer blits.

**What the counts say about the three source cases.** `REFUSED_DEPTH_SRC` is
0 on all three back ends, and no case that ran blits depth out of a cube face,
an array layer or a stencil into or out of a texture (no `REFUSED_*` line names
them; the six `fbo.api` cube-face depth attachments only attach). The
gl-ds-layers lift (depth from a cube face or array layer, stencil through a
texture) is GL-completeness work the mustpass does not reach. The Mali has no
host dEQP back end (`GPU=` takes vivante, powervr and v3d), so it has no
count; its table has `depth_resolve` only, like the GC7000UL's, so the same
blits would be refused there.
