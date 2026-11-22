<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Row pitch of a 3D slice on colour attachments 1..3 (V3D and PowerVR): a `pitch` in `GlesDrawTarget`

Design note of plan #368, item `slice-pitch-doc` (the design of the read-only
`slice-pitch-note`). Line numbers are at the integration tip `49145a0a`.

## The defect

A slice of a 3D texture is kept as a grid of slices (`grid_cols > 1`), so its
rows are `grid_level_pitch` pixels apart (`src/gl/texture.rs:1776`), not
`tex_pitch(w)`. Attachment 0 carries that pitch: `fb_target_pitched_n`
(`src/gl/framebuffer.rs:1379`) returns it, `resolve_target` stores it in
`GlesDraw::target_pitch` (`src/gl/draw.rs:2599`), and the back ends read it
through `draw_pitch` (`src/backend.rs:1433`). `GlesDrawTarget`
(`src/backend.rs:949`: `gpu`, `w`, `h`, `format`, `out_shift`) has no pitch, so
attachments 1..3 would be written at `tex_pitch(w)` and a grid slice would
overwrite its neighbour. The GL core therefore refuses it:

- `slot_pitch_ok` (`src/gl/framebuffer.rs:1483`) is false for a texture with
  `grid_cols > 1`.
- The framebuffer completeness check answers GL_FRAMEBUFFER_UNSUPPORTED
  through it (`:915`), and `fb_target_n` (`:1398`, the check at `:1417`)
  returns `None`.

ES 3.0 4.4.4 allows the slice on any attachment, so this is a defect, not a
limit. The back ends' side:

- V3D: `extra_targets` (`src/v3d/backend.rs:6737`) keeps only `rt_w`
  (`:6760`); the job's RT stride is `target_pitch(target.rt_w[i]) * bytes`
  (`:2745`), `target_pitch` being `tex_pitch` (`:2945`, pinned by
  `src/v3d/backend_tests.rs:1525`). Slot 0 uses `pitch * target.bpp()` (`:2735`).
- PowerVR: `pitch = tw.wrapping_add(1) & !1` for `slot != 0`, `b.target_pitch`
  for slot 0 (`src/pvr/backend.rs:10009`), fed to `pbe_reg_words`
  (`:942`, `linestride = (pitch - 1) / 2`, `:950`; callers `:9976`, `:10022`).

## Design

1. **`src/backend.rs`** (owner #354): add `pub pitch: u32` to `GlesDrawTarget`
   (`:949`), after `out_shift`, documented as "row pitch in pixels; 0 = the
   packed `tex_pitch(w)`", the meaning `target_pitch` has (`:1389-1394`). Add
   `pitch: 0` to `ZERO` (`:964`). The struct is `repr(C)` and `Default`; no C
   file declares it, so the layout change stays inside Rust. Every struct
   literal needs the field (CLAUDE.md, "Two green pull requests"):
   `grep -rl 'GlesDrawTarget {' src tests` lists `src/backend.rs`,
   `src/gl/draw.rs`, `src/v3d/state.rs`, `src/pvr/backend.rs`,
   `src/pvr/target_keys.rs`, the `backend_tests.rs` of v3d, pvr, mali and
   vivante, `src/vivante/mipmap.rs`, and the tests `pvr_target_keys_refusals`,
   `gc_rs_clear_stream`, `es3_pvr_modf`, `gc_rt128_targets`,
   `gc_resolve_wait_kick`, `mixed_target_keys_parity`, `es3_mrt4_msaa_v3d`
   (re-grep at the time; use `..GlesDrawTarget::ZERO` where a literal can).
2. **`src/gl/framebuffer.rs`**: make `fb_target_n` return the pitch the way
   `fb_target_pitched_n` does (`:1382-1387`: `loc_pitch(c, loc, w)` when it
   differs from `tex_pitch(w)`, else 0): a fifth tuple element, or a sibling
   function. Delete the `slot_pitch_ok` calls at `:915` and `:1417` and the
   function (`:1483`). Attachment 3 of a grid slice is then complete. Note
   that `loc_pitch` (`:1335-1347`) also returns a non-`tex_pitch` pitch for a
   mip level under `mip_level_align` / `linear_row_align` and for a
   renderbuffer; those slots are wrong today too (never refused) and the same
   field fixes them.
3. **`src/gl/draw.rs`** (owner #361): `resolve_targets` (`:2818`) fills `pitch`
   in `GlesDrawTarget { gpu, w, h, format, out_shift: 0 }` (`:2830`). Slot 0
   (`:2823`) keeps `d.target_pitch`; consumers keep reading `draw_pitch` for it.
4. **`src/v3d/backend.rs`** (owner #359): `ExtraTargets` (`:6716`) and
   `V3dJobTarget` gain `rt_pitch: [u32; 3]` next to `rt_w` (`:6722`, `:779`);
   `extra_targets` fills it (`:6760`) with `t.pitch`, or `target_pitch(t.w)`
   when 0; the `V3dJobRt` stride (`:2745`) becomes `rt_pitch[i] * bytes`. The
   pitch is part of the job's identity (as `rt_w`, `:774`). `v3d_job.c` takes a
   stride per RT already; no C change expected. Read-back and blit find a
   binding by `stride_texels` (`rt_find`, `:2013`, `:2235`, `:2453`) and get
   the grid pitch from the texture binding; the layers test below covers them.
5. **`src/pvr/backend.rs`** (owners #350, #325): `b.targets[i] = d.targets[i]`
   (`:6064`) already copies the field. The slot `!= 0` pitch (`:10009`) becomes
   `if t.pitch != 0 { t.pitch } else { tw + 1 & !1 }`; the `twiddle` test just
   below it compares `pitch` with the padded width and stays false for a grid.
   The frame-identity comparison (`:6646`, `(gpu, w, h, format)`) must add
   `pitch`, or a draw into a slice of another pitch reuses the frame.
   `record_attachment_loads` (`:4515`) and `record_slot_clear_quad` (`:4745`)
   work from `targets[i]`; verify with the clear test.

Order: step 1 (#354) first, then 2 and 3 (GL core, #361), then 4 and 5 in
parallel. Drop `slot_pitch_ok` only in the pull request in which both back
ends read the field: a back end that ignores it is correct only while the GL
core still refuses the slice.

## Tests to un-ignore and pins to drop

`tests/es3_mrt4_layers_v3d.rs`:
- drop `attachment3_a_slice_of_a_3d_texture_is_unsupported_today` (`:481-489`);
- remove `#[ignore]` from `attachment3_a_slice_of_a_3d_texture` (`:495`);
- reword the header lines that name `slot_pitch_ok`.

`tests/es3_mrt4_layers_pvr.rs`:
- drop `a_3d_slice_on_attachment_3_is_unsupported_today` (`:496-509`) and
  `a_3d_slice_on_attachment_3_draws_nothing_today` (`:684-709`);
- remove `#[ignore]` from `a_3d_slice_on_attachment_3_completes` (`:526`) and
  `a_3d_slice_on_attachment_3_draws` (`:712`); the latter should also assert
  the slot's `pitch` (the grid pitch, not `tex_pitch(8)`);
- reword the header (`:18-24`) and the comments at `:436` and `:496-500` that
  name `slot_pitch_ok`;
- keep `a_3d_slice_on_attachment_0_is_complete` (`:513`) as the contrast.

New tests with the change: a unit test in `src/gl/draw.rs` (a 3D slice on
attachment 1 gives the grid pitch, a plain texture 0), a V3D job test that
RT 3's stride is the grid pitch times the bytes, and a PowerVR test that the
slot's PBE `linestride` is `(pitch - 1) / 2`.

## Other back ends

Mali and Vivante carry the new field as 0 and ignore it. Removing
`slot_pitch_ok` lets the GL core hand them a grid slice on attachment 1..:
a back end that draws extra slots must read `pitch` before it reports more
than one target (Vivante: `src/vivante/**`, plans #335 and #355).
