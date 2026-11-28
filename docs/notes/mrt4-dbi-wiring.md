<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Wiring per-draw-buffer blend state (DrawBuffersIndexed) into V3D and PowerVR

Plan #368, item `dbi-wire-doc`. Design only: nothing here was built or run,
and nothing was measured on a board. It states how the merged state model
`src/gl/draw_buffers_indexed.rs` (`DrawBufferState`, `DrawBuffersIndexed`)
reaches the two back ends, who owns each file, in which order items would
implement it, and when `GL_EXT_draw_buffers_indexed` /
`GL_OES_draw_buffers_indexed` may be listed. The entry points
(`glEnablei`, `glBlendFunci`, `glColorMaski`, item `dbi-entry`) are in review
and not advertised. Related: `v3d-draw-buffers-indexed.md` (the 76 dEQP rows
and the V3D hardware), `pvr-blend-multi.md` (the PowerVR key and builder),
`mrt4-deqp-cases.md` (the cases the flip should change).

Function names are the address; line numbers drift. Mesa citations are
`file:line` in the vendored `third_party/mesa-26.2.1` (read only). That
subset has the packet definitions but not the Gallium `v3dx_emit.c` /
`v3dx_state.c` or the pco compiler; those two are named by function only,
as `v3d-draw-buffers-indexed.md` and `pvr-blend-multi.md` read them from the
full 26.2.1 tree.

## 1. Where things are today

- `DrawBuffersIndexed` holds `GLES_MAX_TARGETS` (4) `DrawBufferState`s
  (enable, RGB/alpha equation, four factors, colour mask). Non-indexed setters
  write every buffer; indices are valid below `backend::draw_targets()`. It is
  not a member of the GL context's draw path: `GlContext` still has the single
  `c.blend*` / colour-mask fields, `GlesDraw` one `blend`, one factor set and
  one `color_mask` (`src/backend.rs`, `GlesDraw::blend` and following), plus
  `per_target_mask` / `target_color_mask[4]` (the masks of targets that differ
  by *format*, not by state).
- V3D already emits per-RT `BLEND_ENABLES` (`rt_blend_mask`) and per-RT
  `COLOR_WRITE_MASKS` nibbles (`rt_colormask`), but one `BLEND_CFG` with
  `render_target_mask = 0xf` (`driver/src/v3d/v3d_job.c`, `emit_blend`).
- PowerVR has a 64-bit multi blend key with one blend word and a (class, mask)
  pair per slot (`src/target_key.rs`, `shc_blend_key_multi`) and a per-slot
  blend tail in the online compiler
  (`src/shaderc/online/powervr/blend.rs`); its key still carries ONE equation
  for all slots, bit 62 is reserved for "more words follow".

## 2. What the draw record must carry

`GlesDraw` (`src/backend.rs`) gains one array beside `targets[]`:

```text
pub blend_rt: [GlesRtBlend; GLES_MAX_TARGETS]   // slot k = draw buffer k
struct GlesRtBlend { blend: bool, eq_rgb: u8, eq_alpha: u8,
                     src_rgb: u8, dst_rgb: u8, src_alpha: u8, dst_alpha: u8,
                     color_mask: u8 }          // shc_blend_factor / shc_blend_func codes
pub independent_blend: bool                    // any slot differs from slot 0
```

Decisions:

1. **Slot k is draw buffer k**, the numbering `targets[]`, `out_shift` and
   `attachment_out_reg` already use, not a dense index. A `GL_NONE` slot
   has `targets[k].gpu == 0`; its `GlesRtBlend` is carried but ignored (V3D
   masks all four channels, PowerVR slot code 0).
2. **The scalar fields stay and mean slot 0.** `blend`, `blend_*`, `color_mask`
   and `per_target_mask` keep their values (`DrawBuffersIndexed::color_mask0`
   and `get_integer` already define "buffer 0's value"), so every back end
   that does not read the array (Mali, Vivante, host stubs) is unchanged. A
   back end that cannot honour `independent_blend` refuses a draw by name
   (`-ENOTSUP` / the existing `DrawError` refusals) instead of using slot 0:
   a silent wrong blend is the worse answer.
3. **`independent_blend` is false whenever the four slots (up to
   `num_targets`, present ones only) are equal.** The non-indexed case and
   every ES 3.0 program therefore take today's code paths unchanged; the
   tests pin that (bytes of the recorded packets and the key equal today's).
4. **Integer slots**: the GL core stores what the application set (GL keeps
   the state, ignores blend for integer attachments). The back end drops the
   blend of an integer or, on V3D, 32F target (`rt_blendable`); the record
   does not pre-filter, so the queries stay exact.
5. **`per_target_mask`/`target_color_mask`** are the *format* mask
   (channels a target has). The effective mask of slot k is
   `blend_rt[k].color_mask & target_color_mask[k]` when `per_target_mask`.
   One place computes it (`src/gl/draw.rs::per_target_masks`, :2782) so the
   back ends read a single field.
6. Adding fields to `GlesDraw` and a struct to the table: grep `tests/` and
   `src/` for every `GlesDraw {` literal and `GlesDrawTarget` use before the
   commit (CLAUDE.md, "Two green pull requests").

## 3. V3D: per-RT BLEND_CFG and COLOR_WRITE_MASKS

What the hardware has (Mesa 26.2.1):

- `BLEND_ENABLES` is an 8-bit mask, bit i = RT i
  (`third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml:773-775`).
- `BLEND_CFG` has a `Render Target Mask` (4 bits on 4.2, 8 on 7.1) and one
  mode/factor set (`v3d_packet.xml:777-785`, 4.2; `:787-795`, 7.1). The mask
  names the RTs the packet applies to, so independent blend is **one
  packet per RT** with `1 << i`. Gallium's `v3dX(emit_state)` does exactly that
  for an independent blend state (`emit_rt_blend`, skipped for an RT that
  does not blend), and `v3d_create_blend_state` builds the enables from the
  per-RT enable when `independent_blend_enable`.
- `COLOR_WRITE_MASKS` is one 32-bit word, 4 bits per RT, a set bit disables
  a channel (`v3d_packet.xml:804-806`). `BLEND_CONSTANT_COLOR` is one for the
  job (`v3d_packet.xml:797-802`), as GL's.
- A target without alpha (`dst_alpha_one`) rewrites the DST_ALPHA factors; in
  Mesa that is per colour buffer, so with independent blend it is per
  packet.

What changes here (all after the GL core carries `blend_rt`):

1. `driver/src/v3d/v3d_job.h`, `struct v3d_job_state`: `rt[4]` of
   `{blend_enabled, rgb_func, rgb_src, rgb_dst, alpha_func, alpha_src,
   alpha_dst, colormask}` and an `independent` flag. The present scalar fields
   and `rt_blend_*` / `rt_colormask*` stay for `independent == false` (the
   packet bytes of every existing draw must not move, pinned by
   `tests/es3_v3d_int_rg_mrt_mask.rs` and `src/v3d/state.rs` tests).
   `ffi_layout_check.c` and `src/v3d/ffi.rs` move in the same commit.
2. `v3d_job.c::emit_blend`: `BLEND_ENABLES.mask` = the OR of the blendable RTs
   that enable; then, when `independent`, one `BLEND_CFG` per enabled RT with
   `render_target_mask = 1 << i` and that RT's factors, else today's single
   packet with mask 0xf. The packet-skip key (`PACK(V3D_PACK_BLEND, ...)`)
   lists every per-RT word; inputs of a disabled RT are packed as 0, as now.
3. `COLOR_WRITE_MASKS`: nibble i is `~mask_i & 0xf` from `rt[i]` when
   `independent`; the GL_NONE rule of `rt_masks` (absent slot = every channel
   disabled) stays.
4. `src/v3d/state.rs::job_state`: fills `rt[]`; `rt_blendable` stays the
   filter (integer and 32F targets never blend); `dst_alpha_one` becomes
   per RT from the slot's format (no alpha bits: RGB565/XRGB class), which
   `blend_factor(f, dst_alpha_one)` already takes as an argument.
5. Both 4.2 and 7.1: `fill71`/`table71` read the same `GlesDraw`; the 7.1
   `Render Target Mask` is 8 bits, the same one packet per RT. Until the 7.1
   path takes it, it refuses `independent_blend` by name.

Host proof: per RT, four draws on the host back end with four different
states (enable, equation, factors, mask) read back per attachment against the
one-target formula, plus a byte test of the recorded `BLEND_ENABLES`,
`BLEND_CFG` (one per RT) and `COLOR_WRITE_MASKS` words. The vendored
simulator reference (item `v3d-sim4`) is the check that silicon-order
semantics of several `BLEND_CFG` packets hold.

## 4. PowerVR: from the blend key to per-slot state

Today the key (`pvr-blend-multi.md` section 3) is
`[blend word 26 bits][slot0..3: 9 bits each (class 5, mask 4)][bit 62
reserved][bit 63 tag]`: one equation for all slots. `GL_EXT_draw_buffers_indexed`
makes the equation per slot, so the key and the builder grow:

1. **Key.** Bit 62 says "a second word follows". The second word is
   `[slot0..3 blend word 3 x ... ]`: each slot gets its own 26-bit blend word
   in a 128-bit key (`u64 hi, u64 lo`): `lo` is today's key with bit 62 set,
   `ext` holds the four 26-bit words (4 x 26 = 104 bits; the unused bits
   zero). Rule: a draw whose slots share one blend word keeps today's 64-bit
   key (bit 62 clear), so every ES 3.0 draw and every offline row keeps its
   value. Constructor and decoder `shc_blend_key_indexed` /
   `shc_blend_key_indexed_slots` in `src/target_key.rs`, canonical (decode,
   rebuild, equal) like `shc_blend_key_multi`. The key already crosses the C
   ABI as `blend_key` + `blend_key_hi`; the ext word is a third field
   (`blend_key_ext: [u32; 2]` in `ShcShader`, `blend_variant`, the program's
   variant cache `ProgFsVariant`, `src/abi_layout.rs`, `abi_layout_check.c`,
   `include/gles_shaderc.h`). Items that only widen a key are separate from
   the builder.
2. **Builder.** `src/shaderc/online/powervr/blend.rs::Blend::from_key` takes
   the slot's own word instead of the shared one; the per-slot loop of the
   builder (section 4 of `pvr-blend-multi.md`) already runs
   once per slot, so the change is the source of the word, not the control
   flow. Mesa's model is per target: `nir_lower_blend` takes an `rt[]` of
   `{format, rgb, alpha, colormask}` and lowers each store with its own
   channel (`compiler/nir/nir_lower_blend.h`, the `nir_lower_blend_rt`
   struct, in the full tree; the pvr driver fills one per attachment in
   `pvr_init_fs_blend`).
3. **GL core.** `src/gl/draw.rs::current_blend_key` (:1485) builds the key
   from the slot states: the shared-word case as today, the per-slot case
   from `blend_rt`. `GlesDraw::blend` becomes "any present non-integer slot
   blends" so `fs.uses_fbfetch` / early depth (`src/pvr/backend.rs`) see
   it.
4. **Back end.** `REFUSE_MIX_BLEND` / `REFUSE_MIX_MASK`
   (`src/pvr/backend.rs`, mixed-list check) are lifted by item E of
   `pvr-blend-multi.md` (a pre-condition); it then compares the program's
   key, both words, to the draw's. Until `blend_rt` exists the back end keeps
   reading the scalar state.
5. **What the host cannot prove** is the same list as `pvr-blend-multi.md`
   section 6 (the per-slot pixel read, pixout 1..3, rounding); the indexed
   board case adds nothing but a different equation per target.

PowerVR output registers are four 32-bit words, so the number of slots is
bounded by the mrt layout, not by the key.

## 5. Which owner plan's file each step touches

| Step | Files | Owner |
|---|---|---|
| W1 draw record: `GlesRtBlend`, `GlesDraw::blend_rt`, `independent_blend` | `src/backend.rs` | #354 |
| W2 GL core fills it from `DrawBuffersIndexed` (and holds the state: the context field, the non-indexed setters writing all slots, the queries) | `src/gl/draw.rs`, `src/gl/state.rs`, `src/gl/context.rs` | #361 (`draw.rs`), #364 (`state.rs`, `framebuffer.rs`, `mod.rs`: wait for it to merge) |
| W3 V3D job state and packets | `driver/src/v3d/v3d_job.h`, `v3d_job.c`, `ffi_layout_check.c`, `src/v3d/state.rs`, `src/v3d/ffi.rs` | #359 (`src/v3d/backend.rs`, `backend_tests.rs`); `driver/src/v3d/*` and `state.rs` are not named by a plan: this plan's items |
| W4 PowerVR key (ext word) and ABI | `src/target_key.rs`, `src/shaderc/mod.rs`, `src/abi_layout*.{rs,c}`, `include/gles_shaderc.h` | #368 |
| W5 PowerVR builder takes the slot's word | `src/shaderc/online/powervr/blend.rs`, `chain.rs` | #368 |
| W6 PowerVR back end accepts the draw, compares both words | `src/pvr/backend.rs`, `src/pvr/target_keys.rs` | #350 / #325 |
| W7 entry points advertised, `ALL_EXTENSION_NAMES` | `src/gl/state.rs`, `src/gl/es3.rs` | #364 (it changes `es3.rs`, `state.rs`) |
| W8 board check, issue texts | `driver/tools/pvrlab/gl_test_es3_mrt4.inc`, issues | board sessions #313 (Pi 4), #316 / #320 (AM62P) |

A step that shares a file with a running plan rebases onto it or waits; this
note changes none of them. `src/backend.rs` and `src/gl/draw.rs` carry the
single new array and the single place the effective mask is computed (decision
5), so W1 and W2 are the narrow, early steps.

## 6. Order of the items that would implement it

1. **W1 + W2 (record).** `GlesDraw::blend_rt` and `independent_blend`
   filled from `DrawBuffersIndexed` (state held in the context), with
   `independent_blend == false` for every non-indexed draw. Host tests: the
   recorded draw's array for a few call sequences; unchanged recorded draws
   for the existing suites. Neutral for every back end.
2. **W3 (V3D)**: needs W1. Can run in parallel with W4/W5 below.
3. **W4 (PowerVR key)**, then **W5 (builder)**, then **W6 (back end)**; W4
   and W5 need only the key shape, W6 needs `pvr-blend-multi.md` item E
   first (the multi-target blend must run with a shared equation before it
   runs with per-slot ones).
4. **Back-end guard (each GPU)**: a back end that does not take the array
   refuses `independent_blend` by name. Land with W1, so the GL core can
   never hand a draw to a back end that would ignore it.
5. **W7 (advertise)**, last, when the condition below holds.
6. **W8** board check after W7 builds into the images, a flag-gated first
   (the `CONFIG_*_GL_TEST_MRT4` shape) so default images do not change.

## 7. When to list the extensions

List `GL_EXT_draw_buffers_indexed` and `GL_OES_draw_buffers_indexed` in
`ALL_EXTENSION_NAMES` for a build only when all hold for the back end that
build uses:

1. `backend::draw_targets()` is 4: the `random.max_required_draw_buffers.*`
   cases (20) use four buffers; below that they would fail on buffer 4 even
   with the extension (`v3d-draw-buffers-indexed.md` (b); RC14, the knob).
2. The back end honours per-buffer state or refuses it by name (steps W3,
   W6 or the guard of item 4). A back end that ignores it must not advertise.
3. The host dEQP `draw_buffers_indexed` run has no Fail among the 66
   non-advanced cases: `overwrite_common`/`overwrite_indexed` (26) and
   `random.*` (40). The 10 `*advanced_blend_eq*` rows stay `NotSupported`
   (`GL_KHR_blend_equation_advanced` is never listed): that is the correct
   status, not a defect (`mrt4-deqp-cases.md`).
4. The queries are exact: `glGetIntegeri_v` of the six blend pnames,
   `glGetBooleani_v` of `GL_COLOR_WRITEMASK`, `glIsEnablediEXT`, the errors
   (`GL_INVALID_ENUM`, `GL_INVALID_VALUE` at or past `GL_MAX_DRAW_BUFFERS`),
   already modelled in `DrawBuffersIndexed` and to be pinned at GL level when
   `dbi-entry` merges.
5. Per GPU, so the string differs by build: Pi 4 and AM62P independently;
   the GC7000UL and Mali builds list it only if their own back ends take the
   array (not planned here).

The board proof (#313, #316/#320) is a run of the same 66 cases on the image
with the knob on; the result goes to `tools/cts/RESULTS.md` and the issue,
not here.

## 8. Not decided here

- Whether the ext key is a third `u64` or a wider key struct on the C side.
- 7.1 (Pi 5) and Mali/Vivante support, beyond refusal by name.
- `GL_KHR_blend_equation_advanced`, dual-source blend and logic ops.
