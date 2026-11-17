<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: R32I/UI and RG32I/UI colour render targets (design)

What the pixel engine (PE) and resolve engine (RS) have for 32-bit-integer
one- and two-channel colour targets, what Mesa's etnaviv does with them, what
the back end already draws, and what is left. Design only: nothing here was
run and no code changed.

Sources. Mesa 26.2.1 (the full tree, `mesa-26.2.1`; only
`third_party/mesa-26.2.1/UPSTREAM` and a few headers are vendored, the etnaviv
driver is read from the checkout named in
`docs/notes/gc-128bit-render-targets.md`, read only). Paths are
`src/gallium/drivers/etnaviv/` unless they start with `driver/` or `src/`
(ours). The core is the GC7000UL, HALTI5, `HalfFloatPipe` 1
(`src/etnaviv/hwdb/nxp/gc_feature_database.h:19026`, see
`gc-128bit-render-targets.md`). Our lines are of `5454be1b`; grep the name
when they drift.

## What this note is, and is not

`gc-128bit-render-targets.md` covers RGBA32I/UI/F, and names RG32I/UI only as
the format each plane reuses. `gc-int64-mixed-targets.md` covers RG32I/UI (and
RGBA16I/UI) beside another target. Neither covers the one- and two-channel
classes themselves: their format column in Mesa, their clear and blit paths,
their blend and mask, and what is left after the single-target draw. That is
this note. R32I/UI and RG32I/UI **already draw alone on the host back end**
(`docs/es3/int-rt-gc.md` section 1, `tests/es3_int32_rt_gc.rs`); the roadmap
item "R32I/UI and RG32I/UI render-target classes" is the rest (section 6),
none of it board-run.

## 1. The PE and RS formats

**PE.** Two extended formats cover all four GL formats. The enum has no
32-bit integer entry (`driver/src/vivante/hw/state_3d.xml.h:101-102` here;
the same values in Mesa's `src/etnaviv/hw/state_3d.xml.h`):

| GL | PE format | value | bytes a pixel |
|---|---|---|---|
| R32I, R32UI | `PE_FORMAT_R32F` | 0x14 | 4 |
| RG32I, RG32UI | `PE_FORMAT_G32R32F` | 0x15 | 8 |

Mesa's table has one PE column for both signs: `R32_SINT`, `R32_UINT`,
`R32_FLOAT` are `R32F` (`etnaviv_format.c:136-137,140`) and `R32G32_UINT`,
`R32G32_SINT`, `R32G32_FLOAT` are `G32R32F` (`:209-210,213`). The sign is not
in the PE format; it is in the output mode.

**Output mode.** `translate_output_mode` returns `COLOR_OUTPUT_MODE_UIF32`
first for any format whose first component has 32 bits
(`etnaviv_translate.h:417-423`), before it looks at integer-ness. It is
written to `PS_CONTROL_EXT.OUTPUT_MODEn` (`etnaviv_state.c:376-377`). UIF32
(0x2, `state_3d.xml.h:132` here) is "UI or F32": the shader's 32-bit register
goes to the target as its bits, no int-to-float conversion. Saturation is off
because only `util_format_is_unorm` formats saturate (`etnaviv_state.c:361-366`).
We write the same: `pe_color_format` (`src/vivante/backend.rs:333-352`:
R32I/UI to `R32F`, RG32I/UI to `G32R32F`, extended) and `shader.rs`
`ps_color_output_mode`.

**Support gate.** `gpu_supports_render_format` allows any pure-integer or
float format on HALTI2 or later (`etnaviv_screen.c:504-505`), after
`translate_pe_format` matches (`:474-477`). The 8-bit integers have a
separate HALTI5 clause (`:497-498`: "figure out 8bpp RS clear"); the 32-bit
ones have none. So Mesa offers all four on this core with no refusal.

**RS and BLT.** Neither engine has a 32-bit-integer format.
`translate_rs_format` lists only depth, 4/5/6/8-bit-channel unorm colour
(`etnaviv_translate.h:278-312`): no `R32_*`, no `R32G32_*`. `translate_blt_format`
lists the same families plus `R10G10B10A2` and a few 8-bit ones
(`:315-` the switch; no `R32`). So Mesa never blits or resolves these formats
under their own name; it falls back by size (section 4).

**Texture side, for contrast** (`docs/notes/gc-r32-int-sampling.md`): on
HALTI5 the TE has real integer formats, `EXT_R32I` and `EXT_G32R32I`
(`etnaviv_format.c:286-291`). Only the render side is the float format
reinterpreted.

**Layout.** Mesa renders only TILED (or super-tiled) targets, with tile
status for those the RS can address (`etna_resource_can_use_ts`,
`etnaviv_resource.c:148-170`; no TS for 128-bit only). `PE_LOGIC_OP.SINGLE_BUFFER`
is 2 for any tiled target above 16 bits a pixel, 3 at 16 bits, 1 if a target
is linear (`etnaviv_state.c:533-539`). We render LINEAR for 4-byte targets (R32I/UI;
the measured path of every RGBA8 draw) and a 4x4-tiled shadow for 8-byte ones
(RG32I/UI), because the PE misplaces 64-bit pixels in a linear target (gl_test
case 44; `src/vivante/wide_target.rs:8-22`, `docs/es3/int-rt-gc.md` section 1).
So the two classes differ in layout, and only the second needs the shadow.

## 2. What the shader output and blend state need

- **Shader output.** The register's 32 bits, unconverted. `ivec4`/`uvec4`
  outputs stay in their integer register in our compiler (`tests/es3_int_rt_gc.rs`;
  `docs/es3/int-rt-gc.md` "Shader outputs, unconverted"); RG32 stores two words
  (R,G) to one output, R32 one. Nothing to add.
- **Blend.** None. ES 3.0 4.1.8 disables blending for an integer attachment, and
  Mesa's blend state has no integer rule of its own (it never sees `blend` on an
  integer buffer, the GL state tracker clears it). Our GL core clears
  `GlesDraw::blend` when attachment 0 is integer (`src/gl/draw.rs:3754`,
  `:1432`). Draw buffers 1 to 3 get an alpha-config word of 0 for an integer
  format (`rt_blend`, `src/vivante/backend.rs`). **Gap:** slot 0 takes
  `PE_ALPHA_CONFIG` from the draw's `blend` as a whole, so RT0 integer with a
  blended float RT1 leaves RT1 unblended; Mesa has a per-RT alpha config
  (`etnaviv_blend.c:176-`). `gc-int64-mixed-targets.md` step 5 is the fix; it
  helps R32 beside a float target as well.
- **Colour mask.** The mask words are per target
  (`etnaviv_blend.c:150-172`: slot 0 `PE_COLOR_FORMAT.COMPONENTS`, others
  `PE_HALTI5_RT_COLORMASK`). R32 has one component, bit 0; the PE keeps the
  other three bits as given (Mesa passes the user's four bits through for a
  format with fewer channels, `util_format_colormask_full` handling the full
  case, `:150-151`). We write `COMPONENTS` from the draw's mask. An alpha-less
  target "loses OVERWRITE": a full mask is only `0b0111` for R32 and RG32 in our
  word (`tests/es3_int32_rt_gc.rs` header: draw `0x14000710`, `0x15000710`, no
  OVERWRITE). For RG32 the meaning of the four bits in a `G32R32F` slot is the
  PE's, not ours (`gc-int64-mixed-targets.md` section 2, "Partial colour masks");
  for R32F it is the same question one channel narrower. Board.
- **Bits kept.** That `R32F`/`G32R32F` with UIF32 keeps all 32 bits of a
  negative or above-2^24 value is Mesa's assumption (the integer rows above
  rely on it); no measurement of it on this core exists in the repo. Board.

## 3. Clears

- **Mesa.** `glClearBuffer{i,ui}v` is an RS clear: `etna_blit_clear_color_rs`
  packs the colour with `etna_clear_blit_pack_rgba` (`etnaviv_clear_blit.c:90-111`):
  `util_pack_color_union` into the PE-internal format, then a 4-byte value is
  replicated into both halves of the 64-bit RS clear value, an 8-byte value
  (RG32) fills it natively as `g << 32 | r`. The clear runs without tile status
  as `etna_rs_gen_clear_cmd` with mask `0xffff` (`etnaviv_rs.c:399-423`), or
  through the TS clear value with tile status (the branch above it, from `:369`).
  The RS fill writes whole pixels: it has no per-channel mask (`:416-420` passes
  `0xffff`), so a clear that needs one is not an RS clear; whether etnaviv's
  clear entry sends such a clear to the blitter draw (`etnaviv_clear_blit.c:226-236`,
  `util_blitter_clear_render_target`) was not traced here.
- **Ours.** A draw of the clear quad: one `mov` of the four raw words to the
  colour output; the target's output mode is programmed from the target's
  format, so the integers are stored (`docs/es3/int-rt-gc.md` "Clears";
  `emit_clear`, `src/vivante/backend.rs:2651`, pinned by
  `tests/es3_int32_rt_gc.rs::a_clear_of_each_emits_its_words_and_reads_back_bit_exact`).
  R32I/UI works alone; for RG32I/UI it is the single-target shadow draw. A clear
  of an R32 or RG32 attachment **beside another attachment** is a mixed draw:
  R32 beside a 32-bit target is fine (linear, `clear_takes` zeroes the others'
  mask, `backend.rs:547-550`); RG32 beside anything is refused
  (`gc-int64-mixed-targets.md` section 1, two `#[ignore]`d tests in
  `tests/es3_int_rt_mrt_gc.rs`).
- **RS clear instead of the quad** is possible for R32 (the RS clears raw
  32-bit words, the same value the quad would store) but not needed: the quad
  already stores them and handles the mask, scissor and sub-rectangle. Not an
  item. It would matter only if a board run showed the quad's UIF32 path
  losing bits where a raw RS fill does not, which makes it a diagnostic, not a
  design.

## 4. glReadPixels, blit and copy

- **Read-back.** Integer `glReadPixels` reads the storage on the CPU: linear
  for R32, through `wide_target::retire` (untile) for RG32. R is the low word,
  the order the GL core keeps (`backend::gles_surf_int_layout`,
  `src/backend.rs:462-480`); no swap. Bit-exact on the host through the
  stand-in GPU (`tests/es3_int32_rt_gc.rs`), not a measurement of the PE.
- **Blit and copy in Mesa.** A same-format blit of the integer class cannot
  use a named RS or BLT format; `etna_try_rs_blit` first tries the exact
  format, then, when only converting layout (no MSAA downsample), a
  size-compatible one: 4 bytes `RS_FORMAT_A8R8G8B8`, 2 bytes `A4R4G4B4`,
  anything else none (`etnaviv_rs.c:42-52,741-750`). So R32 copies raw
  (4 bytes) under RS, RG32 (8 bytes) has no RS format and falls to the BLT
  engine's `A16R16G16B16` (`etnaviv_blt.c:128-139`) on a BLT core, or to the
  blitter (a shader draw) / manual tile copy otherwise (`etna_manual_blit`, `etnaviv_rs.c:584`, called at
  `:953`; tile size `blocksize * 16`). The RS only accepts the same
  source and destination format (`etnaviv_rs.c:730-737`), so no conversion
  between R32I and R32F happens there.
- **Ours.** The GL core blits and copies on the CPU, in
  `src/gl/blit.rs` (`blit_framebuffer`, `:1092`; integer to integer saturates
  to the destination's range, `:1857`). Nothing goes through the RS, so a
  tiled RG32 storage must be retired (untiled) before a CPU blit reads it:
  `kick_pending` and `retire_wide_targets`, as for RGBA16F today. No new
  hardware blit is needed or planned.
- **Tile status.** We use none. Mesa's TS clear would apply to the 32-bit
  class on a core that has FAST_CLEAR; not planned here.

## 5. How this lines up with the other two notes

- RGBA32I/UI planes are two `G32R32F` targets with UIF32
  (`gc-128bit-render-targets.md`): the same word as RG32I/UI. Whatever the
  board says about RG32I/UI (bits kept, tiling, mask) says the same about
  each RGBA32 plane, so one board probe serves both: run the RG32 questions of
  section 7 first.
- RG32I/UI mixed with other targets needs the multi-target shadow of
  `gc-int64-mixed-targets.md` steps 1 to 7; RGBA32 needs the same shadow, with
  a plane split on top (`gc-128bit-render-targets.md` "Implementation
  sequence", step 2, already done inert in `wide_target.rs`). The shadow work
  is one item, not two; this note does not duplicate it.
- R32I/UI is the only class of the four with no gap in a mixed draw once the
  slot-0 blend rule lands, because it stays linear. **But** SINGLE_BUFFER is
  one switch: an R32 target beside an RG32/RGBA16 one is in a tiled draw, so
  R32 must then be tiled in the shadow too (`gc-int64-mixed-targets.md` section 2).
- Texture attachment: an R32/RG32 texture is an FBO colour attachment since
  `6c5b004f` (`integer_texture_surface`); `glGenerateMipmap` on them is not
  defined for integer formats, so no render path is needed.

## 6. What is already done

Single-target draw, clear and read-back, all four formats, on the host: the PE
words `0x14000710` / `0x15000710` (draw), `0x14010f10` / `0x15010f10` (clear),
UIF32, no saturate, no blend, linear storage for R32, the shadow for RG32,
renderbuffer and texture attachments complete (`tests/es3_int32_rt_gc.rs`,
`docs/es3/int-rt-gc.md`). The GL core's name lists contain them
(`GC_INTEGER_RT_FORMATS`, `src/vivante/backend.rs:366-369`; `rt_format_drawable`,
`src/backend.rs:577-`). **Not** done, and why:

| Gap | Cause | Owner note |
|---|---|---|
| RG32I/UI beside another target | SINGLE_BUFFER tiles every target; shadow serves one target (`draw_one` refusal, `backend.rs:3197-3213`) | `gc-int64-mixed-targets.md` |
| R32I/UI beside a 64-bit target | same switch: the R32 target needs a shadow too | same |
| Slot-0 blend with RT0 integer | `PE_ALPHA_CONFIG` from the whole-draw `blend` | `gc-int64-mixed-targets.md` step 5 |
| Mask words of a `G32R32F` slot | meaning of the four bits unknown | board |
| UIF32 keeps 32 bits on this core | Mesa's assumption, no record | board |
| Whether `R32F`/`G32R32F` targets alone behave as hoped | nothing was run | board |

## 7. What only the board can answer

1. **Bit-exact R32I/UI and RG32I/UI.** One gl_test case per class: a
   constant with `0x80000000`, `0xFFFFFFFF`, `0x01000001`, `0xDEADBEEF`
   (above 2^24 and negative), drawn and cleared, read back raw. A `0x01000000`
   for `0x01000001` is a 24-bit float path (UIF32 not raw). The RG32 case is
   the probe `gc-128bit-render-targets.md` already specifies, with one plane.
2. **RG32 tiling and the 64-bit pixel placement** in the shadow
   (gl_test 44's fault again if wrong). The measured single-target path is
   `wide_target.rs`'s header; the board record is gl_test 44 only for RGBA16F.
3. **Colour-mask component bits** of both formats: masks `0x1`, `0x2`, `0x3`,
   `0xf`, each with the masked channels preserved.
4. **Blend off with `GL_BLEND` on** beside a float target in RT1, once the
   slot-0 rule lands.
5. **dEQP** (`fragment_out`, `fbo.completeness.renderable.*32*`,
   `fbo.color.clear`): the board rerun after items 1 to 4; host image
   comparison is zeros on `GPU=vivante` (the Linux OSAL never runs the stream).

## Implementation items

Each is one host-tested item unless it says board. The shadow items are the
two other notes' work; listed for ordering, not to be done twice.

1. **Board probe, one gl_test case per class** (R32I, R32UI, RG32I, RG32UI),
   draw and clear, bit-exact against the constants of section 7.1. Files:
   `driver/samples/zephyr/gl_test/src/` (a new case), `tools/board.sh` stays
   untouched. Needs the i.MX8MP; its result settles sections 2 and 7.
2. **Slot-0 per-RT blend rule**: `blend && !gles_surf_is_integer(target_format)`
   for `PE_ALPHA_CONFIG`, as slots 1 to 3 do. Files: `src/vivante/backend.rs`
   (`emit_pe_state`, the slot-0 alpha-config block), test in
   `src/vivante/backend_tests.rs` or `tests/es3_int_rt_mrt_gc.rs` (RT0 RGBA8UI or
   R32UI, RT1 RGBA8, `GL_BLEND` on: RT0 no blend enable, RT1 blended). Same step
   as `gc-int64-mixed-targets.md` step 5; do it once.
3. **Multi-target shadow** (RG32I/UI beside anything, R32 beside a 64-bit
   target): `gc-int64-mixed-targets.md` steps 1 to 4 and 7, the work of
   `gc-rt128-shadow` (inert, `src/vivante/wide_target.rs`) and
   `gc-rt128-hwtargets` (`src/vivante/backend.rs`, `hw_targets`, `mrt_targets`).
   Files: `src/vivante/wide_target.rs`, `src/vivante/backend.rs` (`wide_target`,
   `draw_one` refusal at `:3197-3213`, `emit_pe_state`), `src/backend.rs` (a CPU
   view and pitch for every target, `GlesDraw::new`), `src/gl/draw.rs` (fill
   `targets[]`; plan #361's file, put this edit in its own item),
   `tests/es3_int_rt_mrt_gc.rs` (the refusal tests turn into draw tests, the two
   `#[ignore]`d clear tests run), `docs/es3/int-rt-gc.md`. Opt-in until item 1
   and the mixed-target probe say yes.
4. **Mask words for `R32F`/`G32R32F` slots**: pin what the code writes for masks
   `0x1`, `0x3`, `0x5`, `0xf` per format with the hypothesis stated; no default
   change. Files: `src/vivante/backend.rs` (`emit_pe_state`, mask block), a test
   in `src/vivante/backend_tests.rs`. Board confirms through item 1's masked
   variant.
5. **Tests for the clear of R32/RG32 beside another attachment** (R32 beside
   RGBA8UI now; RG32 beside any after item 3): `glClearBuffer`, `glClear` and a
   masked clear, the other attachment's pixels surviving. Files:
   `tests/es3_int_rt_mrt_gc.rs` (new cases, not ignored for R32), or a new
   test file modelled on `tests/es3_int32_rt_gc.rs`.
6. **CPU blit/copy of a tiled RG32 storage**: pin that `glBlitFramebuffer`,
   `glCopyTexSubImage2D` and `glReadPixels` of an RG32I/UI attachment see
   the shadow's untiled bytes after a kick (`kick_pending` then `retire`), by a
   host test with the stand-in GPU of `tests/es3_int32_rt_gc.rs`. Files: a new
   test file (never committed); no code unless it fails
   (`src/vivante/wide_target.rs`, `src/gl/blit.rs`).
7. **Docs after the board**: `docs/es3/int-rt-gc.md` (the 32-bit-channel
   paragraph "for the board to confirm" turns into the record), RC1 and RC2 rows
   of `docs/es3-cts-failures.md`, `docs/es3/backend-caps.md` only if the class
   list changes. Not before item 1 has a record.
8. **Optional diagnostic, no default**: an RS raw-fill clear of an R32 target
   (`etnaviv_rs.c:399-423` shape) behind a debug knob, only if item 1 shows the
   quad losing bits where a raw fill keeps them. Files: `src/vivante/backend.rs`
   (`emit_clear`). Skip unless item 1 asks for it.
