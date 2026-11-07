<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: the Pi 4 draw_buffers_indexed.* rows

Research, written before any code item. `docs/es3/pi4-gles3-fails.tsv` lists
76 `dEQP-GLES3.functional.draw_buffers_indexed.*` rows, all `NotSupported`,
all from image D1 (`build/f-D1.txt`, the 925dbca9 run). This note sorts them
by cause, says what the V3D 4.2 hardware has per render target (Mesa 26.2.1
`v3d`), what `src/v3d` and `driver/src/v3d` carry today, and what a fix needs
in `src/v3d`, `driver/` and `src/gl`.

Function names are the address; line numbers drift (yakogl is main at
`05888810`). Mesa paths are relative to `src/gallium/drivers/v3d/` of the full
26.2.1 tree in the deps cache (`mesa-pan`), not the vendored `third_party`
subset; Mesa is described in words here, not copied.

## Result in one paragraph

None of the 76 rows is a V3D defect today. They are `NotSupported` because
the library does not advertise `GL_EXT_draw_buffers_indexed` (nor
`GL_OES_draw_buffers_indexed`), so dEQP skips the groups by its own
extension test. The 10 rows that also need `GL_KHR_blend_equation_advanced`
stay `NotSupported` however the indexed extension goes: the V3D has no
advanced blend equations. Turning the other 66 into runs is a GL-core job
(entry points and per-buffer state) first; the V3D then needs per-render-target
blend and write-mask words, which the hardware has and the job builder does
not yet emit.

## (a) Whether the build advertises it, and what dEQP does

- `src/gl/consts.rs` defines `GL_OES_draw_buffers_indexed` and
  `GL_EXT_draw_buffers_indexed` (both `0x1`), but those constants come from
  the registry header list; they are not the extension string. The string is
  `ALL_EXTENSION_NAMES` in `src/gl/state.rs` (`extension_listed`), and neither
  name is in it. `GL_KHR_blend_equation_advanced` is not in it either.
- No `glEnableiEXT`, `glDisableiEXT`, `glIsEnablediEXT`, `glBlendEquationiEXT`,
  `glBlendEquationSeparateiEXT`, `glBlendFunciEXT`, `glBlendFuncSeparateiEXT`
  or `glColorMaskiEXT` exists in `src/` (grep over the sources and
  `src/GL_CORE_NOTES.md` finds only the two constants).
- dEQP (`es3fDrawBuffersIndexedTests.cpp`) skips with `NotSupported`: for a
  case that uses advanced equations, `Extension GL_KHR_blend_equation_advanced
  not supported` (line 1245) checked first; else `Extension
  GL_EXT_draw_buffers_indexed not supported` (line 1248); the two `random`
  groups the same at line 1470. The reason strings come from the AM62P run
  of the same dEQP (`docs/es3/am62p-gles3-notpass.tsv`, the same 76 case
  names); the Pi 4 tsv keeps the status only, so the split below is by case
  name plus that run. The correct status for a build that does not claim the
  extension *is* `NotSupported`: these rows do not count as a Fail and need
  no fix to stay correct. They are not-pass only because the goal is to run
  them.

## (b) The rows, by cause

| # | Cause | Rows | Names |
|---|---|---|---|
| C1 | `GL_KHR_blend_equation_advanced` absent (checked first) | 10 | `overwrite_common.*advanced_blend_eq*` 5, `overwrite_indexed.*advanced_blend_eq*` 5: `common_advanced_blend_eq_buffer_{advanced,blend,separate}_blend_eq`, `common_blend_eq_buffer_advanced_blend_eq`, `common_separate_blend_eq_buffer_advanced_blend_eq`, in each group |
| C2 | `GL_EXT_draw_buffers_indexed` absent, `overwrite_*` | 26 | `overwrite_common` 13, `overwrite_indexed` 13: the `blend_eq`/`separate_blend_eq`/`blend_func`/`separate_blend_func`/`color_mask`/`enable`/`disable` pairs without advanced |
| C3 | `GL_EXT_draw_buffers_indexed` absent, `random.*` | 40 | `random.max_required_draw_buffers.0-19` 20, `random.max_implementation_draw_buffers.0-19` 20 |
| | total | 76 | = 10 + 26 + 40; checked by grep over the tsv: 18 + 18 + 40 rows in the three groups |

Cause per row is therefore: C1 10, C2 26, C3 40. Only C2 and C3 (66 rows)
can ever run on this V3D; C1 cannot (`v3d_needs_software_blend` in Mesa's
`v3dx_state.c` knows only dual-source blend; I found no advanced-blend path
in that driver). If the product
decision is to never advertise the extension, 66 rows would be the most to win
and the 10 stay `NotSupported` by design (to be recorded as expected in
`docs/es3-cts-failures.md`; that file belongs to another item).

What a C3 case needs beyond the extension: `max_required_draw_buffers` uses 4
buffers and `max_implementation_draw_buffers` uses `GL_MAX_DRAW_BUFFERS`;
`max_draw_buffers()` (`src/gl/context.rs`) is 3 on every GPU today
(`DRAW_TARGETS`, `src/backend.rs`), below the ES 3.0 minimum of 4. A
`max_required` case would fail on the fourth buffer even with the extension.
That is the RC14 limit (`docs/es3-cts-failures.md`) and is shared with
`implementation_limits` and part of `fragment_out`; see
`docs/notes/v3d-mrt-sizes.md` and the `mrt4` item. Note C2's overwrite cases
use few buffers (the test draws to as many as its fixture sets, read from the
name-only listing: not known without the dEQP source, which is not in the
tree).

## (c) What the V3D 4.2 has per render target

Mesa 26.2.1 `v3dx_emit.c` (`v3dX(emit_state)`) and `v3dx_state.c`:

- `BLEND_ENABLES`: one mask, bit `i` = RT `i` blends. `v3d_create_blend_state`
  builds it from `rt[i].blend_enable` when `independent_blend_enable`, else
  all bits (`(1 << max_rts) - 1`, 4 on 4.2) when `rt[0]` blends.
- `BLEND_CFG`: one packet *per RT configuration* with a `render_target_mask`
  field (colour and alpha mode, src and dst factors). Independent blend emits
  one packet per RT with the single bit `1 << i` (`emit_rt_blend`, skipped for
  an RT whose blend is off). Without independent blend, one packet covers the
  whole mask; if some RTs lack alpha (`blend_dst_alpha_one`, set per
  colour buffer in `v3d_set_framebuffer_state`, swizzle alpha = 1) it emits
  two packets, with and without the dst-alpha-one factor rewrite
  (`v3d_factor`). So the hardware blends per RT, and the *factor rewrite for
  an alpha-less target is per RT*.
- `COLOR_WRITE_MASKS`: a single packet with 4 bits per RT (a set bit
  disables a channel, `translate_colormask`, which also swaps R and B for an
  RT in `swap_color_rb`). Loop over `max_rts`; `rt = independent ? i : 0`.
- `BLEND_CONSTANT_COLOR`: one for the job, not per RT (matches GL).
- The sample mask, the alpha-to-coverage and the logic op are not per RT.

So per-RT enable, per-RT equation and factors and per-RT write masks are all
hardware features; GL's `EXT_draw_buffers_indexed` maps onto
`independent_blend_enable` one to one.

## (d) What the V3D back end carries today

- `GlesDraw` (`src/backend.rs`) has one `blend`, one set of
  `blend_{src,dst}_{rgb,alpha}`, `blend_eq_{rgb,alpha}`, `blend_color`, one
  `color_mask` and one `color_write`. `GlesDrawTarget` has `gpu`, `w`, `h`,
  `format`, `out_shift` and no blend or mask.
- `src/v3d/state.rs` `job_state` fills `V3dJobState` from those: one
  `blend_enabled`, one factor set, one `colormask` (`d.color_mask & 0xf`,
  `& 0x7` if RT 0 has no alpha). `dst_alpha_one` is `target_alpha_bits == 0`
  of the frame target only.
- `driver/src/v3d/v3d_job.h`, `struct v3d_job_state`: "pipe_blend_state,
  render target 0": `blend_enabled`, `rgb_func/src/dst`, `alpha_*`,
  `colormask`, `blend_color_f16`.
- `driver/src/v3d/v3d_job.c`: `emit_blend` writes `BLEND_ENABLES.mask = 1`
  (RT 0) and one `BLEND_CFG` with `render_target_mask = 0xf`; the
  `emit_state` comment says "one render target, no independent blend".
  `COLOR_WRITE_MASKS` is `off | off << 4 | off << 8 | off << 12`, RT 0's mask
  for all four. The packet-skip key (`PACK(V3D_PACK_BLEND ...)`,
  `V3D_PACK_COLOR_MASK`) lists exactly those inputs.
- Consequence today (read from the code, not run): `BLEND_ENABLES.mask` is
  1, so with several attachments only RT 0 blends; GL blends every enabled
  draw buffer. That deviation exists without this extension; whether a host
  test pins it is for `t-mrt-blend`.

## (e) The V3D and driver changes a fix needs

All in `src/v3d` and `driver/src/v3d`, after the GL core hands over per-buffer
state (section f).

1. `GlesDrawTarget` (or a sibling array in `GlesDraw`) gains per-target
   `blend`, factors, equations and `color_mask` (`src/backend.rs`; a field on
   a table struct: grep `tests/` and `src` for every literal, CLAUDE.md "Two
   green pull requests"). This is `src/backend.rs`, so it belongs to the
   GL-core side of the split (section f), not to `src/v3d`.
2. `driver/src/v3d/v3d_job.h`: `struct v3d_job_state` gains
   `rt[4]` of `{blend_enabled, rgb_func, rgb_src, rgb_dst, alpha_func,
   alpha_src, alpha_dst, colormask}` and an `independent` flag; `blend_color_f16`
   stays single. `ffi_layout_check.c` pins the layout and must move in the
   same commit.
3. `v3d_job.c`: `emit_blend` follows Mesa: `BLEND_ENABLES.mask` = OR of per-RT
   enables; when independent, one `BLEND_CFG` per enabled RT with
   `render_target_mask = 1 << i`; when not, today's single packet but with
   the enables mask `(1 << rts) - 1` for the non-indexed several-attachment
   draw (this fixes (d)'s last bullet). `COLOR_WRITE_MASKS` packs
   `~mask_i & 0xf` at `4 * i`, with R/B swap for an XRGB RT. The `PACK`
   keys list every per-RT input (inputs of a disabled RT listed as 0, as the
   existing packing does).
4. `src/v3d/state.rs`: `job_state` fills the rt array; `dst_alpha_one` becomes
   per RT (`target alpha bits` of each `GlesDrawTarget`; this needs
   `alpha_bits` per target, which `GlesDrawTarget` has no field for, so the
   format byte decides it: formats with no alpha are RGB565/XRGB-class).
   `blend_factor(f, dst_alpha_one)` is already a function of that flag.
5. Interaction with the integer render targets (`docs/notes/v3d-int-clear.md`,
   `state::rt_format`): GL ignores blend for integer attachments, so an
   integer RT must have its `BLEND_ENABLES` bit off whatever the enable says;
   the existing single-RT code never meets the case, the per-RT code must
   handle it. Also the clear quad (`clear_mrt`) uses the colour mask per RT:
   the per-RT mask words are the same ones, so the
   scissored/masked several-attachment clear item and this one share
   `COLOR_WRITE_MASKS`.
6. `v3d_frame.c` (the clear-only frame) keeps its all-RT zero mask packets.

Host tests (none exist yet for this): extend the job-state tests in
`src/v3d/backend_tests.rs` / `tests/es3_mrt_blend_v3d.rs` (running item
`t-mrt-blend`) with a draw whose RTs have different blend and masks, reading
the recorded `BLEND_ENABLES`, `BLEND_CFG` and `COLOR_WRITE_MASKS` words.

## (f) What needs src/gl (issue drafts)

The V3D cannot be exercised by dEQP until the GL core can say it: entry
points, state and queries. Not changed here; drafts for the report.

### Draft 1: GL core: GL_EXT_draw_buffers_indexed and GL_OES_draw_buffers_indexed

Title: `GL core: per-draw-buffer blend enable, equation, factors and colour mask (GL_EXT/OES_draw_buffers_indexed)`

Text: dEQP-GLES3 `functional.draw_buffers_indexed.*` (76 cases) is
`NotSupported` on all three boards because the library neither lists
`GL_EXT_draw_buffers_indexed` / `GL_OES_draw_buffers_indexed` nor has the
entry points (Pi 4 image D1: 76 NotSupported). The GL core has one blend
state and one colour mask per context (`GlesDraw::blend*`, `color_mask`).
Needed: (1) state per draw buffer `0..GL_MAX_DRAW_BUFFERS`: blend enable,
RGB/alpha equation, src/dst RGB/alpha factors, colour mask; the
non-indexed calls set all buffers; (2) `glEnableiEXT`/`glDisableiEXT`
(`GL_BLEND` only; others `GL_INVALID_ENUM`), `glIsEnablediEXT`,
`glBlendEquationiEXT`, `glBlendEquationSeparateiEXT`, `glBlendFunciEXT`,
`glBlendFuncSeparateiEXT`, `glColorMaskiEXT`, with `GL_INVALID_VALUE` for an
index >= `GL_MAX_DRAW_BUFFERS`; (3) the indexed queries: `glGetIntegeri_v`
for `GL_BLEND_EQUATION_RGB/ALPHA`, `GL_BLEND_SRC/DST_RGB/ALPHA`,
`glGetBooleani_v` for `GL_COLOR_WRITEMASK` and `GL_BLEND`; the non-indexed
queries return the buffer-0 value when they differ (per the extension);
(4) carry the per-buffer state into the draw (`GlesDraw`: a per-target array
beside `targets[]`); every back end keeps reading buffer 0 until it takes the
array, so this is behaviour-neutral on the other GPUs; (5) list the two
names in `ALL_EXTENSION_NAMES` only when the V3D (the first user) takes the
array; the PowerVR and Vivante back ends must either honour it or refuse a
draw with unequal per-buffer state by name (`-ENOTSUP`). Host tests in
`tests/es3_*`: unequal blend and mask per buffer read back per attachment on
the host back end that implements it. Acceptance: Pi 4 image D1 runs the 66
non-advanced cases. `GL_KHR_blend_equation_advanced` is out of scope (10
cases stay `NotSupported`). Needs: the V3D item below; `GL_MAX_DRAW_BUFFERS`
4 for the `max_required` cases (RC14, the `mrt4` item).

### Draft 2: V3D: per-render-target blend and colour mask

Title: `V3D: BLEND_ENABLES/BLEND_CFG/COLOR_WRITE_MASKS per render target`

Text: `driver/src/v3d/v3d_job.c` `emit_blend` and the `COLOR_WRITE_MASKS`
emit take render target 0's state for all targets (`BLEND_ENABLES.mask = 1`,
`render_target_mask 0xf`, RT 0's mask in every nibble). Mesa v3d emits a
per-RT enables mask, one `BLEND_CFG` per RT under independent blend and per-RT
`COLOR_WRITE_MASKS` nibbles (`v3dx_emit.c`). Implement the same: per-RT array
in `struct v3d_job_state` (with `ffi_layout_check.c`), per-RT `dst_alpha_one`,
integer RTs never blend; this also fixes a several-attachment draw with blending
on, which today blends RT 0 only. Needs the GL-core draft only for the
indexed calls; the non-indexed several-attachment case can be done first
(host test with `t-mrt-blend`). Does not move any Pi 4 draw_buffers_indexed
row on its own.

### Draft 3 (docs, for the `v3d-es3-docs` item)

`docs/es3-cts-failures.md` RC14 says the 76 `draw_buffers_indexed` rows are
explained by `max_draw_buffers` 3. By the dEQP source lines above they are
`NotSupported` because the extensions are not advertised (66) or advanced
blend is absent (10); RC14 explains only the `max_required` half of the
`random` cases once the extension exists.

## What I did not check

- The dEQP source is not in the tree; the cause split rests on the case
  names and the AM62P run's reason strings (same dEQP build, the same 76
  names). A Pi 4 run reads the same text from `build/f-D1.txt` if the full
  log is kept.
- Whether the `BLEND_ENABLES` on the current single-RT code leaves RTs 1..3
  unblended on silicon was read from the code, not measured.
- Which Mesa packets the 4.2 hardware ignores for an integer RT was not
  verified against Mesa's state tracker (it clears the blend enable for
  integer formats in `st_atom_blend`/`util_blend`, not in v3d); the design
  assumes the safe side (the enable bit off).
