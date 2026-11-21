<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: blend and colour mask for several colour outputs (design)

Plan #368 (RC14, four draw targets on the BXS-4-64), item `pvr-blend-doc`.
Design only: nothing here was built or run. It settles the variant-key
decision that items B, C and D below need, and says what a host test can
prove and what only the board can. It is the contract of the four
`#[ignore]`d tests of `tests/es3_mrt4_blend_pvr.rs`
(`blend_reaches_four_float_targets`, `partial_mask_reaches_four_float_targets`,
`float_blend_variant_beside_integer_target_3`,
`float_targets_blend_beside_integer_target_0`).

Functions are named as `file::function`; line numbers are those of the
repository tip this note was written at (7fbc55e6) and drift. Mesa is
Mesa 26.2.1, a full tree (the trimmed `third_party/mesa-26.2.1` has no pco and
no `nir_lower_blend`; the full tree is in the dependency cache, read only).
Mesa is described in words here, never copied. Paths below that start
`compiler/` or `imagination/` are under its `src/`.

## 1. What the code does today (checked at the tip)

* **GL core.** `src/gl/draw.rs::current_blend_key` (:1450) builds a 32-bit key
  from ONE render target: the frame's own attachment
  (`framebuffer::frame_attachment`, :1460 of `src/gl/framebuffer.rs`, the
  first drawn attachment). The blend state it reads is the context's single
  one (`c.blend`, `c.blend_src`, ... :1483): ES 3.0 has one blend equation, one
  pair of factors per channel group and one `glColorMask` for every draw
  buffer. Only the render-target class and the channels a target has differ
  from buffer to buffer. For several targets it first tries
  `src/gl/target_keys.rs::draw_mixed_key` (:58), the blend-free, mask-free key
  of an integer target beside others (3 slots, `SHC_MIXED_MAX_TARGETS`,
  `src/target_key.rs:18`).
* **`GlesDraw::blend`** is `c.blend && !gles_surf_is_integer(d.target_format)`
  (`src/gl/draw.rs` :4293): target 0's (the frame's) format only. Confirmed.
  Other readers of the flag: `src/pvr/backend.rs` (:8616 sample rate, :9036 the
  early-depth choice), which is why D must change it, not only the key.
* **The builder takes one output.** `src/shaderc/online/powervr/blend.rs`:
  `Blend::from_key` (:69) decodes a 32-bit key into one `Blend` (factors,
  equations, one mask, one class); `Regs` (:148) gives one colour (4
  temporaries) plus 18 scratch; `step` (:437) emits the tail that reads
  pixout0 (or pixout0/1 for halves), blends and writes it. The only caller is
  `src/shaderc/online/powervr/chain.rs::build_fragment_with` (:986), and its
  several-outputs branch refuses a blend variant by name
  (`"a blend variant of a shader with more than one colour output"`, :1135)
  and alpha-to-one (:1138). Confirmed. The refusal is the whole defect of the
  first two ignored tests.
* **The back end also refuses.** `src/pvr/backend.rs` (mixed-list check around
  :6700-6750): `REFUSE_MIX_BLEND` (:6738) when `d.blend` or an integer
  attachment 0 with a non-replace state, `REFUSE_MIX_MASK` (:6741) for a
  partial `color_mask`, and it checks `fs.blend_key == key` against the
  program (:6713, :6743). The third and fourth ignored tests need these lifted
  as well as the key; the brief did not say so.
* **The key is 32 bits in many places, not two.** `ShcShader::blend_key`
  (`src/shaderc/mod.rs:1423`, offset 1236, pinned in `src/abi_layout.rs:209`
  and `src/abi_layout_check.c:226`; the PowerVR `ShcShader` is 1272 bytes,
  `src/abi_layout.rs:182`), the `blend_variant` entry (`src/shaderc/mod.rs`
  :1577-1584, `blend_key: u32`), `ShcPrecompiledLink::fs_blend_key` (:1839),
  the offline row key (:1910, `src/shaderc/precompiled.rs::find` :278), the
  program's variant cache (`src/gl/draw.rs::ProgFsVariant::blend_key` :173,
  looked up at :1694) and the generator tools under `tools/yakogl-tools`.
  A wider key is an interface change, not a local one. The offline tables
  hold single-output variants only, so a multi key never needs a row.
* **Frame load.** `src/pvr/backend.rs::attachment_out_reg` is at :2335 and its
  frame-load use `record_attachment_loads` at ~:4520 (the brief's ~:4496 is
  stale). It places attachment `i` at the register the end-of-tile emit
  stores it from (four registers, 64-bit formats on even pairs). That a
  blended read of target `i` finds attachment `i`'s old colour in that
  register is the one claim here a host test cannot prove (section 6).
* **Masked clears** (`CLEAR_MASKED`, `src/pvr/backend.rs` ~:2217) are separate
  internal one-output programs (`clear_m1..mE`, plus `_F16X4`); they are not
  part of this design (a masked multi-target clear is `mix_clear_refused`,
  :4678, a different item).
* **Correction of the brief:** the key's constructors are not in `src/pvr/**`:
  they are `src/shaderc/mod.rs::shc_blend_key*` and `src/target_key.rs`;
  `src/pvr/target_keys.rs` only decodes. The recommendation of a 64-bit key is
  right, but it touches the C ABI (section 3.4).

## 2. What Mesa does (the model for the builder)

pco (the PowerVR compiler of Mesa's pvr driver) blends per render target, with
`nir_lower_blend` run over the fragment shader before the output lowering:

1. **Pass order.** `imagination/pco/pco_nir.c:970` runs `nir_lower_blend`
   with `data->fs.blend_opts`, and `:978` runs `pco_nir_pfo` (output packing)
   after it. Blending is in float, ahead of the PBE-format pack. The `pfo`
   pass runs alpha-to-one first and then the colour-write-enable and output
   passes (`imagination/pco/pco_nir_pvfio.c:845-866`).
2. **The options are per target.** `compiler/nir/nir_lower_blend.h:36-40` has
   a blend channel (function, source factor, destination factor); `:42-56` a
   render target: its `format`, an `rgb` and an `alpha` channel and a 4-bit
   `colormask` (`:48-49`, "0x0 for none, 0xF for RGBA, 0x1 for R"); `:58-67`
   the options with `rt[8]` (`:59`), the logic-op pair (`:61-62`) and
   `scalar_blend_const` (`:66`). The pvr driver fills one entry per colour
   attachment, format from `output_formats[location]` and `write_mask` into
   `colormask` (`imagination/vulkan/pvr_arch_pipeline.c::pvr_init_fs_blend`
   :2229-2278, format :2243, mask :2270; the per-location format array is
   `imagination/pco/pco_data.h:81`, the options member `:99`). Vulkan makes
   equation and mask per attachment; ES 3.0 makes them global, so for us the
   per-target data is only format and the channels the format has.
3. **Each store is blended on its own.** `nir_lower_blend_instr`
   (`compiler/nir/nir_lower_blend.c:1004`) maps the store's location to its
   target index (`color_index_for_location` :994-1002), leaves it alone when
   the target's format is none (:1013-1016), sinks it to the end of the block
   so dual-source stores are already seen (:1042-1046), and removes it when
   that target's mask is 0 (:1052-1058). It reads the pixel (framebuffer
   fetch of that same output location) only when the equation or the mask
   needs it: a min or max, a non-zero destination factor, a source factor of
   destination colour, alpha or alpha-saturate (`channel_uses_dest`
   :157-176), or a mask other than 0xF (:1070-1085). Then it blends
   (:1098-1109), applies the mask by taking masked channels from the pixel
   (`nir_color_mask` :198-213, :1111-1112) and trims the value to the
   format's components (:1114-1117).
4. **The equation is the one the builder already follows.**
   `nir_color_blend` (:911-991): a pure-integer format or a replace state
   returns the source untouched (:914-915); the source and the blend
   constant are saturated to the format (`nir_fsat_to_format` :146-154,
   applied :940-948); destination alpha reads 1.0 where the format has no
   alpha, other absent channels read 0 (:950-961); each channel is blended
   with its group's function and factors (:963-985; factor values
   :105-136, an inverted factor is 1-f :179-195). Same for every target: only
   the format changes what is saturated and which channels exist.
5. **Integer targets are not blended** (:914, :1104-1106) but still take the
   mask. pco then packs each output for its format and reads it back through
   the same per-format unpack: `to_pbe_format` (`pco_nir_pvfio.c:86-122`;
   sRGB is saturated, :97-101, then held as four halves), `lower_pfo_store`
   (:319-377; a format of none removes the store, :345-348) and
   `lower_pfo_load` (:379-449; the pixel read is four packed dwords of that
   location, :413-426, then unpacked by format). So a pixel read is per output
   location, in that output's own format.
6. **Alpha-to-one** applies to every four-component colour store
   (`lower_alpha_to_one`, :699-737), before blending is lowered into the
   packing. A per-slot version is therefore the Mesa behaviour; the first cut
   may keep refusing it (item C).
7. **Colour-write-enable** (`lower_color_write_enable`, :769-815) is another
   path that selects between the new colour and the pixel, per location, from a
   metadata bit. We do not use it: GL's mask is channel-wise.

Consequence for us: the shader's blend tail is repeated once per colour output
slot, each time with the shared equation, that slot's class and that slot's
mask. This is what the key must carry.

## 3. Decision: the variant key

### 3.1 The choice

**A 64-bit multi key, a pure function of the draw state, with the shared
blend equation once and a (class, mask) pair per target.** Not an interned
side table. Reasons:

* A multi key is *canonical*: the same draw state is always the same number, on
  the host, in the cache of `ProgFsVariant`, in a test's assertion and on the
  board. An interned id depends on the order draws happened in; a host test
  could not pin it, and a key logged on the board would not name a variant.
* It is a pure function: `current_blend_key` stays free of any global table,
  and the key can be decoded (as `shc_blend_key_mixed_targets` already is) to
  check the program against the draw (`pvr/backend.rs` :6743).
* It reuses `src/target_key.rs`'s per-target code, so the one-class mixed key
  (blend-free) is the multi key with a zero blend word and zero masks.
* Cost: the key crosses the C ABI (3.4). The side table would avoid that but
  would make the program cache, the logs and the offline lookup order-dependent;
  that is worse than a one-time ABI widening, which the offline tables (single
  output only) need no row for.
* 32 bits cannot hold it: 26 bits of blend plus 4 x 9 bits.

### 3.2 Bit layout (key `K`, 64 bits; `lo` = bits 0-31 is today's key)

```text
 bits    field
 0..4    src_rgb       } the blend word, the same positions as shc_blend_key
 5..9    dst_rgb       } (src/shaderc/mod.rs:85): one equation for every
 10..14  src_alpha     } target, as ES 3.0 has. All zero = no blend (replace
 15..19  dst_alpha     } writes the same bits as "no blend"; see 3.3)
 20..22  eq_rgb        }
 23..25  eq_alpha      }
 26..34  slot 0   } four slots of 9 bits each, slot i at 26 + 9 i:
 35..43  slot 1   }   [0..4] class code, the 5-bit slot code of
 44..52  slot 2   }        src/target_key.rs (0 = no target; 0x10|class<<2 for
 53..61  slot 3   }        RGBA8 / F16X4 / F16 / F32; integer layouts as
                          (shc_blend_key_int_rt >> 5) & 0x1f, never 0 mod 4)
                          [5..8] mask: channels glColorMask disables, bit 0 R
                          .. 3 A, set = disabled, only channels the class has
 62      reserved, must be 0 (per-target blend words, see 3.5)
 63      multi tag, 1 for a multi key
```

* Slot code values 0x04, 0x08 and 0x0C are unused by `slot_code`; they are
  reserved for the two sRGB forms (halves and shader-encoded bytes) that item
  `pvr-srgb-mix` designs. Until it lands, an sRGB target has no slot code and a
  multi draw that holds one keeps today's refusal.
* Slot `i` is the draw buffer `i`'s *attachment-numbered* target, not a dense
  index: a gap (`GL_NONE`) is a zero code. That is the numbering the mrt layout
  and `attachment_out_reg` use. A zero slot in the middle is allowed; the
  `SHC_MIXED_MAX_TARGETS = 3` dense limit of the mixed key is lifted here
  (item `pvr-mixed4-note` shares this decision and must use this layout).
* A class code of an integer layout carries its width and signedness in the
  code, so the layout for `pack_int_output` is read from the slot.
* The mask is canonical: bits of channels a class does not have are 0
  (F16 and F32 have bit 0 only; an integer slot has its channel count's bits),
  exactly as `shc_blend_key_target` drops them today. A slot whose every
  channel is disabled keeps the bits (Mesa removes that store, `nir_lower_blend.c`
  :1052-1058; the builder then writes nothing for it).

### 3.3 When a multi key is used (the canonical rule)

Every key a legacy path can express keeps its value. A multi key (bit 63) is
used exactly when more than one target is present and no legacy key
expresses the draw:

* same-class draws without blend or mask, and the one-class replace key
  with several outputs, keep today's keys (`chain.rs` `replace_class`);
* the 3-slot blend-free mixed key (bit 25 tag) keeps its value until
  `pvr-mixed4` moves it into the multi key; a decoder maps it (zero blend
  word, zero masks) to the same slots;
* any blend that is not the replace state on at least one non-integer
  target, any partial colour mask beside several targets, and any draw of
  four targets of non-uniform class, are multi keys;
* a single target never uses a multi key: it keeps `current_blend_key`'s
  one-target value (`lo` only, `hi == 0`).

"Replace" is judged as `nir_blend_replace_rt` does (add, one, zero in both
groups; `nir_lower_blend.c:342-354`): a blend word equal to the replace state
is written as 0, so enabling blending with `glBlendFunc(ONE, ZERO)` does not
make a second variant.

### 3.4 Interface changes B must make together

* `ShcShader` gets `blend_key_hi: u32` (the key's bits 32-63) in existing
  padding or appended; `blend_key` stays the `lo` half, so every reader of the
  one-target key is untouched. `src/abi_layout.rs`, `src/abi_layout_check.c`,
  `include/gles_shaderc.h` and the size pins follow in the same commit.
* `blend_variant`'s `blend_key` parameter becomes `u64` (aarch64 passes it in
  one register); the program cache key and `ProgFsVariant::blend_key` become
  `u64`. Backends other than the PowerVR online one ignore the high word (a key
  with the multi tag reaches only the PowerVR online compiler; offline lookup
  answers -ENOENT for it).
* `shc_blend_key_multi(blend, [slot; 4]) -> Option<u64>` and its decode
  `shc_blend_key_multi_slots(key)`, const, canonical (decode then rebuild must
  equal the key, as `shc_blend_key_mixed_targets` does), live in
  `src/target_key.rs`.

### 3.5 What the key does not carry (and why that is enough)

* **Per-target blend equations.** ES 3.0 has one blend state; the key has one
  blend word. `GL_EXT_draw_buffers_indexed` (plan item `indexed-plan`) would need
  a blend word per slot. Bit 62 is reserved so it can say "a second key word
  follows" without moving anything; that item decides whether it is a second
  `u64` or a wider key. Not decided here, not needed for ES 3.0.
* **Logic op, advanced blend, dual-source.** Not in ES 3.0.
* **Blend constants.** As today: four dwords the GL core appends to the
  uniform block, one set for all targets (`blend_consts_off`).

## 4. The builder (item C)

`blend.rs` keeps its per-channel equation code and gains a loop over slots:

* `Blend::from_key` splits into a shared `BlendWord` and a slot's class and
  mask; `Blend` for slot `i` is the pair (the existing struct stays the unit of
  `channel`/`step`).
* For each present slot, in slot order: read that slot's pixel from its pixel
  output register (`build_pixout_read_from(reg_i, ...)`, two dwords for
  halves), unpack by class, blend per channel with the shared word if the slot
  is not integer (an integer slot is only masked: the masked channels keep
  the pixel's raw dwords), apply the slot's mask, pack, and write
  `build_pixout_write_to(reg_i, ..)`. `reg_i` is the slot's first register by
  the mrt layout (`src/target_key.rs::shc_mixed_out_layout`, extended to four
  slots by `pvr-mixed4`/`pvr-pack`), the same registers the end-of-tile emit
  and `attachment_out_reg` use.
* A slot that reads nothing (replace, full mask) emits no read, as
  `Blend::reads_pixel` does today; `uses_fbfetch` is set when any slot
  reads (Mesa sets one flag too, `nir_lower_blend.c:1070-1080`).
* Registers: the colour of slot `i` is 4 temporaries, the scratch (18,
  `Regs::SCRATCH`) is reused slot after slot because the tail is serial;
  four colours plus scratch is 34 temporaries, which the allocator limit
  (`free`, `chain.rs` ~:975) must be checked against. The constants are loaded
  once.
* The refusal at `chain.rs:1135` goes; the one at :1138 (alpha-to-one) may stay
  for the first cut (Mesa applies it per four-component store,
  `pco_nir_pvfio.c:699-737`), as may "blend constants in a shader that
  discards or reads gl_FragCoord" (:988-1000).
* `fused`, `channel` and `computed_earlier` are per-slot unchanged.

## 5. Item contracts

| item | what | files | owner | tests |
| --- | --- | --- | --- | --- |
| A | Check against Mesa (this note); re-run if pco's pass order changes | `docs/notes/pvr-blend-multi.md` | pvr-blend-doc | none |
| B | The multi key: constructor, decoder, canonical rule, `u64` through the cache and the `blend_variant` entry, `blend_key_hi` in `ShcShader`, ABI pins | `src/target_key.rs`, `src/shaderc/mod.rs`, `src/abi_layout.rs`, `src/abi_layout_check.c`, `include/gles_shaderc.h`, `src/gl/draw.rs::ProgFsVariant` and its lookup (D's file), `src/shaderc/online/backend.rs` | `src/shaderc/**` and `src/target_key.rs`: #368 with `pvr-mixed4`; `src/gl/draw.rs`: #361 | unit tests in `target_key.rs`: round trip over the class/mask space, every legacy key unchanged, the reserved bits rejected |
| C | The builder: per-slot tail, refusal at `chain.rs:1135` lifted | `src/shaderc/online/powervr/blend.rs`, `chain.rs`, `assemble.rs` as needed | `src/shaderc/online/powervr/**` | the online-compiler tests of `blend.rs`/`chain.rs`: four-slot RGBA8 blend, F16X4 + RGBA8, integer slot masked, slot mask 0xF writes nothing; compare each slot's tail to the one-target tail |
| D | The GL core builds the key per target; `GlesDraw::blend` true when any present target is non-integer | `src/gl/draw.rs::current_blend_key` (:1450) and :4293, `src/gl/target_keys.rs` | #361 | `draw.rs` unit tests of `current_blend_key` (the existing ones in the `blend_key_*` group stay green) |
| E | Back end accepts the draw: `REFUSE_MIX_BLEND` / `REFUSE_MIX_MASK` lifted when the program's key (both halves) equals the draw's | `src/pvr/backend.rs` (:6713-6743), `src/pvr/target_keys.rs` | `src/pvr/**`: #350/#325 | un-ignore the four tests of `tests/es3_mrt4_blend_pvr.rs` and flip their `*_is_refused_today` twins (`blend_with_four_float_targets_is_refused_today`, `partial_mask_with_four_float_targets_is_refused_today`, `uint_target_3_with_blend_is_refused_today`, `partial_mask_with_uint_target_3_is_refused_today`, `integer_target_0_turns_blend_off_today`) to the passing form; the file belongs to `indexed-plan` while it runs |
| F | Board check (section 6) | `driver/tools/pvrlab/gl_test_es3_mrt4.inc` (a blend case, gltest's file), issue text | board sessions (#316 / #320) | gl_test verdict lines |

Order: B first. C and D are independent after B and can run in parallel
(C needs only the key and the slot layout; D only the key constructor). E
needs B, C and D. Un-ignore tests at the commit that makes them pass, never
before. `pvr-r09` and `pvr-srgb-mix` also change the slot codes and the
registers, so they rebase on B, not the other way round. Name each ignored
test's gate: `blend_reaches_four_float_targets` and
`partial_mask_reaches_four_float_targets` need B+C+E (one class, shared blend);
`float_blend_variant_beside_integer_target_3` needs the integer slot in the
multi key (B+C+E); `float_targets_blend_beside_integer_target_0` needs D
(`d.blend`) and E.

## 6. What the host cannot prove, and the board check

Host-provable: the key (round trip, canonical rule, legacy keys unchanged),
the shape of the compiled tail per slot (which registers are read, blended and
written, which temporaries), the program/draw key agreement and the
refusals, and that `uses_fbfetch`/`d.blend` reach the back end. All of that is
in the item tests of section 5.

Not host-provable (the host back end records the draw; there is no USC):

1. **Per-target pixel read.** The blend reads attachment `i`'s old colour from
   pixel output register `attachment_out_reg(i)`; that is true only if the
   frame load (`record_attachment_loads`) restored attachment `i` into that
   register and the registers' meaning agrees between the load program, the
   blend tail and the end-of-tile emit. A host test can compare register
   numbers, not the hardware's reading.
2. **That the USC reads of pixout 1..3 behave.** Reading a pixel output
   register other than 0 in a pass that also writes it (the `olchk` form the
   one-target tail uses) is known only for pixout 0 and 1 (RGBA16F's pair).
3. **Rounding of the halves/bytes pack on four targets** and sRGB encode
   beside the blend (the latter is `pvr-srgb-mix`).

Board check, on the AM62P (#316 / #320), `gl_test` with the four-target knob on
(`CONFIG_PVR_GL_TEST_MRT4`) and the blend case added to
`driver/tools/pvrlab/gl_test_es3_mrt4.inc`:

* Clear targets 0-3 to four distinct colours, draw one quad with
  `glBlendFunc(SRC_ALPHA, ONE_MINUS_SRC_ALPHA)` and an output `oN = vec4(...)`
  that differs per `N`, read each attachment back: each must equal the
  one-target formula applied to its own old colour (not target 0's). Verdict
  lines: `mrt4 blend t<N> ok` per target, then `mrt4 blend PASS`; any mismatch
  prints `mrt4 blend t<N> got <rgba> want <rgba>` and `mrt4 blend FAIL`.
* The same with `glColorMask(1,0,1,0)`: masked channels keep each target's old
  value. Verdict `mrt4 mask PASS` / `FAIL`.
* An RGBA8UI target 3 beside three RGBA8: blend on, targets 0-2 blended, 3
  stored raw (`mrt4 intblend PASS`).
* If target `i` (i >= 1) reads back target 0's old colour or garbage, the
  frame load is the defect (section 6.1): record which `i`, and file an issue
  against `record_attachment_loads` rather than the key or builder.

The result, once measured, goes into the `tools/cts/RESULTS.md` row, the
commit message and the issue, not into this note.

## 7. Not decided here

* Per-target blend equations (`GL_EXT_draw_buffers_indexed`): bit 62 only
  reserves the place (3.5).
* The sRGB slot codes (`pvr-srgb-mix`) and the four-slot mrt layout of
  mixed classes (`pvr-mixed4`); this note fixes the key's shape, they fix the
  codes and registers.
* Alpha-to-one and constant blend colour in a shader that discards: stay
  refused in the first cut.
