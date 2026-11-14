<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: an integer colour attachment beside attachments of another class (design)

Plan #361, item `design-note`. How the PowerVR (AM62P, BXS-4-64) draws an
integer colour attachment beside float or unorm attachments, or beside an
integer attachment of another layout, instead of refusing the draw. This is
the "Left on the PowerVR" row of RC1 in `docs/es3-cts-failures.md`. Design
only: nothing here was built or run. The later items of the plan
(`design-key`, `chain-core`, `chain-pack`, `gl-key`, `pvr-accept`,
`docs-rc1`) take it as their contract: the names, bit positions, register
rule and refusal strings below are the ones they implement. Line numbers
are of commit `f2748a1f`; grep the quoted name when they drift.

Related notes: `docs/notes/pvr-int-output-alu.md` (integer outputs from an
ALU result, the invariant in its section 3 holds here too),
`docs/notes/gc-int64-mixed-targets.md` (the GC7000UL's design for the same
lift, a different problem: there the obstacle is the pixel engine's tiling,
here it is the program's output packing).

## 1. What is refused today, and why

`pvr_backend_draw` refuses any draw with more than one colour attachment of
which one is integer (`src/pvr/backend.rs:5848-5860`, log line
`draw refused: an integer colour attachment beside another colour
attachment`) unless `several_int_targets_drawn` holds
(`backend.rs:5803-5814`): every attachment present and of attachment 0's
integer layout, no colour clear, and a program whose key is that layout's
integer variant writing `n * d` dwords. The refusal is right for everything
else, because no program writes the registers the end-of-tile emit reads
for a mix:

- **The emit is already per attachment.** The end-of-tile emit
  (`backend.rs:8647-8718`) walks the attachments in slot order and gives
  attachment `i` the next free pixel output register, `surf_out_dwords(fmt)`
  of them (`backend.rs:994-1000`: an integer layout's packed dwords, 2 for
  RGBA16F and sRGB, 1 for every other format), moving a source of 2 or more
  registers to an even register (pbe2_in_xe) and not emitting an attachment
  that would pass register 4 (`PVR_BACKEND_MAX_OUT_REGS`, `backend.rs:492`).
  The PBE state of each attachment (`pbe_emit_words`, `backend.rs:1014-1050`)
  is its own format's: pack mode, source format, `norm`, `source_pos` (the
  first register), `mrt_index`. Nothing in the emit needs to change for a
  mix.
- **The programs are per class, not per target.** A blend-variant key
  (`src/shaderc/mod.rs:83-257`) describes one target: a blend with one
  float class (`shc_blend_key_target`), one integer layout
  (`shc_blend_key_int_rt`, `ShcIntRt`) or several sRGB attachments
  encoded as RGBA8 (`SHC_BLEND_KEY_SRGB_ENCODED`). The compiler packs every
  colour output for that one target: `pack_int_output`
  (`src/shaderc/online/powervr/chain.rs:895-1010`) packs output `n` into
  dwords `n * d ..` of one layout; the several-output path
  (`chain.rs:791-821`, `assemble::build_fragment_shader_outputs`,
  `src/shaderc/online/powervr/assemble.rs:1577-1678`) packs every output as
  RGBA8 into register `location`. So an RGBA8 output beside an R32UI one
  is packed as two RGBA8 dwords: the integer is clamped to [0, 1] and
  reads 0 (`tests/es3_int_mrt_alu_pvr.rs`, `int_alu_next_to_float_lands`,
  `#[ignore]`d for RC1).
- **Mesa packs per location.** pco packs each colour output for the format
  of its own render target: `lower_pfo_store` reads
  `fs->output_formats[location]` and calls `pack_to_format` with it
  (Mesa 26.2.1 `src/imagination/pco/pco_nir_pvfio.c:344-353`, `:149-218`),
  and the Vulkan driver gives each render target its own output registers
  (`pvr_alloc_mrt`, `src/imagination/vulkan/pvr_arch_mrt.c:116-137`, first
  fit in `pvr_mrt_alloc_from_buffer`, `:57-80`, with an even start for a
  2- or 4-register pixel when the core has `pbe2_in_xe`). This design does
  the same, keyed per draw, because yakogl compiles a variant per key
  rather than per render pass.

## 2. The key

### 2.1 The per-target key

Each draw buffer of a mix is described by a **per-target key**: the
one-target key the GL core would give that attachment alone with no blend
and no colour mask. It is a plain `u32` built only from what
`src/shaderc/mod.rs` already has, so the compiler's per-target code
(`chain-core`) can be written before the mixed key lands:

| target | per-target key | output registers |
|---|---|---|
| integer layout `rt` (any of the 19 integer formats) | `shc_blend_key_int_rt(rt)` (no mask bits) | `rt.dwords()`: 1, 2 or 4 |
| RGBA8 class (every colour format that is neither integer nor sRGB and that `gles_surf_rt_class`, `src/backend.rs:608-616`, maps to `SHC_RT_RGBA8`: the 8-bit and packed unorm formats) | `shc_blend_key_rt(SHC_RT_RGBA8)` = `0x0000_0000` | 1 |
| RGBA16F | `shc_blend_key_rt(SHC_RT_F16X4)` = `0x4000_0000` | 2 |
| R16F | `shc_blend_key_rt(SHC_RT_F16)` = `0x8000_0000` | 1 |
| R32F | `shc_blend_key_rt(SHC_RT_F32)` = `0xC000_0000` | 1 |

The register counts are `surf_out_dwords` of the same formats, which is
what the emit counts. **sRGB is refused** in a mix (section 5): its
one-target form is two registers of linear halves that the PBE encodes
with `gamma`, its several-target form is RGBA8 bytes encoded by the
program; neither is a class of this key. A later item can add it as code
`0x04` (section 2.2), which is reserved for it.

A per-target key `t` decodes with the decoders that exist:
`ShcIntRt::of_key(t)` is `Some(rt)` for an integer target, else
`shc_blend_key_class(t)` is the class. A per-target key is **valid** when
it is exactly one of the values above (an integer one with its mask bits
[26:29] clear, a class one with [0:29] clear).

### 2.2 The mixed key: bit layout

| bits | [26:31] | [25] | [20:24] | [15:19] | [10:14] | [0:9] |
|---|---|---|---|---|---|---|
| holds | 0 | 1 (tag) | slot 2 | slot 1 | slot 0 | 0 |

- [0:9]: zero.
- [10:14], [15:19], [20:24]: draw buffer 0, 1, 2, a 5-bit **slot code**
  each (slot `i` at `10 + 5 * i`).
- [25]: the tag, set (`SHC_BLEND_KEY_MIXED_TAG`).
- [26:31]: zero.

Slot code, from the per-target key `t`:

| code | meaning |
|---|---|
| `0x00` | no target: a slot past the draw's last target |
| `ch \| w << 2 \| s << 4`, `ch` in 1..=3 | an integer layout: exactly bits [5:9] of `shc_blend_key_int_rt(rt)`, i.e. `(t >> 5) & 0x1f` (`ch` 1: one channel, 2: two, 3: four; `w` 0: 8 bits, 1: 16, 2: 32, 3: RGB10_A2UI; `s` signed). Decoded by `ShcIntRt::of_key(code << 5)`, which also rejects `w` 3 with fewer than four channels |
| `0x10` | RGBA8 class |
| `0x14` | RGBA16F (`0x10 \| SHC_RT_F16X4 << 2`) |
| `0x18` | R16F (`0x10 \| SHC_RT_F16 << 2`) |
| `0x1C` | R32F (`0x10 \| SHC_RT_F32 << 2`) |
| `0x04` | reserved (an sRGB class, not built) |
| `0x08`, `0x0C` | invalid |

So a code with its low two bits non-zero is an integer layout, a code with
them zero and bit 4 set is a float or unorm class: the two never collide.

A mixed key is **canonical**: the decoder accepts a key only when the
constructor would build it from the list it decodes to. The conditions
(all of them):

1. [0:9] and [26:31] are zero and [25] is set;
2. every slot decodes (no invalid or reserved code), and the slots that are
   present are dense from slot 0 (no `0x00` before a non-zero code);
3. two or three slots are present;
4. at least one is an integer layout, and they are not all the **same**
   integer layout (one layout everywhere keeps today's
   `shc_blend_key_int_rt` key, section 2.4);
5. the register rule of section 3 fits them in four registers.

Worked values (the tests of section 6 pin these):

| draw buffers | slot codes | key |
|---|---|---|
| RGBA8, R32UI | `0x10`, `0x09` | `0x0204_C000` |
| R32UI, R32I | `0x09`, `0x19` | `0x020C_A400` |
| RGBA8, R32I, RGBA16F | `0x10`, `0x19`, `0x14` | `0x034C_C000` |
| RGBA8, RG32UI | `0x10`, `0x0A` | `0x0205_4000` |

### 2.3 Why no other key can have this pattern

Every blend-variant key that exists has one of these shapes, and each
differs from a mixed key in a bit the mixed key fixes:

- **A blend key** (`shc_blend_key`, with or without the target bits of
  `shc_blend_key_target`, including `SHC_BLEND_KEY_REPLACE`,
  `SHC_BLEND_KEY_KEEP` and the stencil-only `discard` key,
  `src/gl/draw.rs:3792-3806`): [0:4] is `src_rgb`, a `shc_blend_factor`,
  which is 1..=8 or `0x11..=0x18` and never 0
  (`mod.rs:57-74`, `map_blend_factor` `draw.rs:1346-1365`). A mixed key has
  [0:4] zero.
- **`SHC_VIV_KEY_FRONT_FLIPPED`** (1, the GC7000's only key,
  `mod.rs:89-93`): [0:4] is 1.
- **An integer target's key** (`shc_blend_key_int_rt`, `mod.rs:198-211`,
  with mask bits [26:29]): [5:6] is the channel count, 1..=3, never 0. A
  mixed key has [5:9] zero. Its [10:25] are zero too, so its bit 25 is
  clear.
- **`SHC_BLEND_KEY_SRGB_ENCODED`** (`0xA000_0000`, `mod.rs:164`): bit 25
  clear, [29:31] set. A mixed key has bit 25 set and [26:31] zero.
- **0** (no variant): bit 25 clear.

The V3D, GC7000 and Mali build no blend key at all (`fixed_function_blend`,
`draw.rs:3773-3783`): they never see a mixed key.

The decoders that exist read a mixed key harmlessly, so code that does not
know it treats it as "not mine": `ShcIntRt::of_key` gives `None` ([5:6]
is 0), `shc_blend_key_class` gives `SHC_RT_RGBA8` ([30:31] and [29] are 0),
`shc_blend_key_masked` gives 0, `int_rt_reads_pixel` gives `false`.
`blend::Blend::from_key` gives `Err("a blend factor outside
shc_blend_factor")` (src_rgb 0), which is why the compiler tests for a
mixed key **before** it decodes a blend (section 4). Whether any other
consumer (record keys, the precompiled table's key list, fixtures) reads
a key's bits is the subject of the read-only item `survey-keys`; this
design adds no precompiled rows.

The layout leaves room for RC14's fourth draw buffer: a slot 3 fits at
[26:30], and the argument above does not use [26:31].

### 2.4 Which draws keep their keys

A draw keys exactly as before unless it is a mix: two or three draw
buffers present, at least one integer, not all of one integer layout. In
particular one attachment, several float or unorm attachments, several
sRGB attachments, and several integer attachments of one layout (dEQP's
`common.modf` executor, `several_int_targets_drawn`, `tests/es3_pvr_modf.rs`)
keep their keys, their programs and their back-end paths. A draw with a
`GL_NONE` slot between targets is not a mix the GL core can key (the
constructor takes a dense list) and keeps its old key; the back end
refuses it by name (section 5).

### 2.5 The API in `src/target_key.rs` (item `design-key`)

The API is a module of its own, so that the key does not wait on `src/shaderc/mod.rs`.

```rust
/// The most draw buffers a mixed key carries (the PowerVR's
/// `backend::DRAW_TARGETS`, RC14).
pub const SHC_MIXED_MAX_TARGETS: usize = 3;

/// Bit 25: the tag of a mixed key ([0:9] and [26:31] clear).
pub const SHC_BLEND_KEY_MIXED_TAG: u32 = 1 << 25;

/// The pixel output registers one per-target key's colour takes (section
/// 2.1), or `None` when `t` is not a valid per-target key.
pub const fn shc_target_key_dwords(t: u32) -> Option<u32>;

/// The first pixel output register of each target and the registers the
/// outputs span (section 3), or `None` when they pass the pixel's four
/// registers, `targets` is empty or longer than SHC_MIXED_MAX_TARGETS, or a
/// per-target key is not valid. `start[i]` for i >= targets.len() is 0.
pub const fn shc_mixed_out_layout(targets: &[u32]) -> Option<([u8; SHC_MIXED_MAX_TARGETS], u8)>;

/// The mixed key of draw buffers 0.. (one per-target key each, dense), or
/// `None` when the list is not a mix (section 2.2, conditions 2-5).
pub const fn shc_blend_key_mixed(targets: &[u32]) -> Option<u32>;

/// The per-target keys a mixed key carries and how many; `None` for any
/// key that is not a canonical mixed key (every other key, section 2.3).
pub const fn shc_blend_key_mixed_targets(key: u32) -> Option<([u32; SHC_MIXED_MAX_TARGETS], usize)>;

/// `shc_blend_key_mixed_targets(key).is_some()`.
pub const fn shc_blend_key_is_mixed(key: u32) -> bool;
```

Round trip: for every list `l` with `shc_blend_key_mixed(l) == Some(k)`,
`shc_blend_key_mixed_targets(k)` gives `l` back (unused entries 0). The
slot codes are private (`const fn` helpers in the same file); every user
goes through the five items above. `const fn` because the GL core's tests
and the back end compare constants; the bodies need only loops and
matches.

## 3. The output registers

One rule, used by the compiler, the back end's acceptance and the emit:

```
next = 0
for each target i in draw-buffer order:
    d = shc_target_key_dwords(t[i])        // = surf_out_dwords(format of i)
    s = next
    if d >= 2 and s is odd: s += 1         // a 64- or 128-bit source on an even register
    if s + d > 4: no layout (refused)
    start[i] = s
    next = s + d
span = next
```

This is the emit's walk (`backend.rs:8672-8700`: `spos = out_regs`, `regs
>= 2 && spos & 1 != 0` moves it up, `spos + regs > PVR_BACKEND_MAX_OUT_REGS`
does not emit) with the tile-buffer branch left out: a mixed program never
spills a target to a tile buffer, so a mix that needs one is refused. It
is not Mesa's first fit (`pvr_mrt_alloc_from_buffer` would put RGBA8,
RG32UI, RGBA8 at 0, 2, 1); yakogl's emit is sequential, so the program
follows the emit, and a later target never fills an alignment hole.

The program writes target `i`'s packed dwords to pixel output registers
`start[i] .. start[i] + d - 1` (special registers `PIXOUT0 + start[i]` ..,
`assemble::PIXOUT0` = 32, `assemble.rs:1176`). A hole left by the alignment
is not written; the emit never reads it. The record's `num_outputs` is
`span`. The emit gives attachment `i` `source_pos = start[i]` and
`mrt_index` = its emit index (`i` when no slot is empty, which a mix
guarantees).

Examples:

| draw buffers | start | span | registers |
|---|---|---|---|
| RGBA8, R32UI | 0, 1 | 2 | r0 RGBA8, r1 R32UI |
| R32UI, R32I | 0, 1 | 2 | r0, r1 |
| RGBA8, RG32UI | 0, 2 | 4 | r0 RGBA8, r1 hole, r2-r3 RG32UI |
| RG32UI, RGBA8 | 0, 2 | 3 | r0-r1, r2 |
| R32UI, RGBA16F | 0, 2 | 4 | r0, r1 hole, r2-r3 halves |
| RGBA8, R32I, RGBA16F | 0, 1, 2 | 4 | r0, r1, r2-r3 |
| RGBA16F, R32UI, RGBA8 | 0, 2, 3 | 4 | r0-r1, r2, r3 |
| RGBA8, RGBA32UI | - | - | refused: RGBA32UI would take r2-r5 |
| RGBA32UI, R8UI | - | - | refused: r4 does not exist |

Limit: four registers in all (CR_USC_PIXEL_OUTPUT_CTRL is programmed for
four, `backend.rs:8804-8808`). So an RGBA32I/UI target is never drawn in
a mix: it takes all four registers alone, and a mix has two targets at
least. dEQP's `common.modf` `vec3` / `vec4` cases (two
RGBA32UI) stay refused for the same reason in the one-layout path; this
design does not change that (tile buffers would).

### 3.1 What each target's dwords hold

The compiler packs the colour output at location `i` for target `i`
(Mesa's per-location `pack_to_format`, section 1), exactly as the
one-target variant of that target packs its one output:

- **Integer layout**: `pack_int_output`'s fields with no mask
  (`chain.rs:939-982`): 32-bit channels copied, narrower ones masked,
  shifted and or-ed, RGB10_A2UI as one 10/10/10/2 dword. No conversion.
- **RGBA8**: the four components through `pck` u8888, scaled (clamp to
  [0, 1], to 0..255), one dword, as `build_fragment_shader_outputs` does
  (`assemble.rs:1622-1625`, `build_pck_u8888`).
- **RGBA16F**: `pck` f16f16, repeat 2, of R, G into the first dword and B, A
  into the second, unscaled (`assemble.rs:1444-1445`).
- **R16F**: `pck` f16f16, repeat 1, of R (`assemble.rs:1446`).
- **R32F**: R as it is, no pack (`assemble.rs:1437`, `SHC_RT_F32 => 0`).

A location with no written output leaves its target's registers unwritten
(ES 3.0 leaves that attachment undefined). An output at a location past
the last target is dropped (no draw buffer takes it). Each register is
written once, in ascending register order; the last write ends the program;
a shader that discards does its kill test and feedback before the writes,
as `build_fragment_shader_packed` orders them (`assemble.rs:1689-1757`).
`docs/notes/pvr-int-output-alu.md` section 3 holds: the packs read only the
lanes a value has (lanes past the declared width repeat lane 0).

### 3.2 The compiler's API (items `chain-core` and `chain-pack`)

`chain-core`, written against the per-target keys of section 2.1 only:

```rust
// src/shaderc/online/powervr/assemble.rs
/// One target of a mixed program: its per-target key, its first pixel
/// output register, and its colour -- the packed dwords (the first
/// `dwords` entries) for an integer target, the four components for a
/// float or unorm one.
pub struct TargetOut {
    pub key: u32,
    pub start: u16,
    pub src: [(super::sel::Bank, u16); 4],
}

/// The fragment program of a mixed key: each target's colour packed for
/// its class (section 3.1) and written to its own registers, the kill test
/// and feedback of a shader that discards before the writes, the last write
/// ending the program. Returns the bytes written and the temporaries.
pub fn build_fragment_shader_targets(
    iterated: &[super::fragment::Iterated],
    arithmetic: &[super::sel::SelGroup],
    outs: &[TargetOut],
    discard: Option<u16>,
    free: u16,
    buf: &mut [u8],
) -> Result<(usize, u16), BuildError>;

// src/shaderc/online/powervr/chain.rs
/// `build_fragment_for` for draw buffers `targets`: (per-target key, first
/// pixel output register) each, in draw-buffer order. The same body as
/// `build_fragment_for` (one inner function taking `Option<&[(u32, u8)]>`),
/// not a copy.
pub fn build_fragment_for_targets<'a>(
    m: &'a ir::Module<'a>,
    limits: Limits,
    a: &'a Arena<'a>,
    buf: &mut [u8],
    targets: &[(u32, u8)],
) -> Result<BuiltFragment<'a>, ChainError>;
```

`BuiltFragment` gains `pub target_regs: Option<u8>`: the span, set only by
the mixed path. The integer packing is `pack_int_output`'s per-output body
moved into a helper both use, so the one-layout variant's bytes do not
change (its tests in `tests/es3_pvr_modf.rs` and `tests/es3_int_rt.rs` hold
them).

`chain-pack` wires the key: in `build_fragment_for`, **before**
`ShcIntRt::of_key` and before `Blend::from_key`,
`shc_blend_key_mixed_targets(blend_key)` -> `shc_mixed_out_layout` -> the
`(key, start)` list -> the mixed path. `compile_fragment_for`
(`chain.rs:1391-1497`), when `target_regs` is `Some(r)`: `sh.blend_key =
blend_key`, `sh.num_outputs = r`, `sh.uses_fbfetch = 0`.

Refused by the compiler, `ChainError::Build(BuildError::Unsupported(..))`
with these strings:

- `"gl_FragDepth with an integer render target"` (the string that exists,
  `chain.rs:393`, reused for a mixed key);
- `"alpha-to-one with an integer colour attachment beside another"`;
- `"an output other than a colour at the draw's locations with mixed targets"`
  for any output slot that is not `FragColor` (the sample mask and other
  special outputs are not packed per target).

## 4. The GL core (item `gl-key`)

`current_blend_key` (`src/gl/draw.rs:1409-1462`) tries the mixed key
**first**, before the integer branch: for a draw with several targets
(`several_targets`, `draw.rs:1398-1403`), the per-target key of each draw
buffer slot `resolve_targets` builds (`draw.rs:2499-2515`: slot `i` is
attachment `frame_attachment + i`), from its format:
`gles_surf_int_layout` -> `shc_blend_key_int_rt`, else
`gles_surf_rt_class` -> `shc_blend_key_rt` for RGBA8, RGBA16F, R16F, R32F.
An sRGB slot, a `GL_NONE` slot before the last target, or a list that is
not a mix: no mixed key, the function goes on as today. The mixed key
carries neither the blend state nor the colour mask: those are the back
end's to refuse (section 5), so a blended or masked mix compiles the plain
mixed program and is refused at the draw, by name, rather than drawn with
a variant of attachment 0. The stencil-only `discard` override
(`draw.rs:3792-3806`) replaces the key with attachment 0's KEEP key as
today; such a draw is refused by the back end's fallback string.

## 5. The back end (item `pvr-accept`)

The block at `backend.rs:5848-5860` becomes, for a draw with more than one
target and an integer one among them:

1. `several_int_targets_drawn(d, fs)`: drawn, unchanged.
2. Else the checks below in this order; the first that fails logs its
   line (`glog_once!`, each its own literal) and returns `-ENOTSUP`.
3. All pass: drawn. The emit is not changed (section 1).

`several_int_targets_drawn` stays as it is; the new test sits in a new
`pub fn mixed_targets_drawn(d: &GlesDraw, fs: Option<&ShcShader>) ->
Result<(), &'static str>` (the `Err` is the reason; the caller logs the
matching literal) so the tests can call it as `tests/es3_pvr_modf.rs`
calls the one-layout helper.

The refused cases and their exact log lines, in check order:

| case | log line |
|---|---|
| a slot with no target (`gpu` 0) before the last target | `yakogl/pvr: draw refused: a GL_NONE draw buffer between an integer colour attachment and another (not built)` |
| an SRGB8_ALPHA8 attachment in the mix | `yakogl/pvr: draw refused: an SRGB8_ALPHA8 colour attachment beside an integer one (not built)` |
| the layout passes four registers (section 3; any RGBA32I/UI in a mix) | `yakogl/pvr: draw refused: integer and other colour attachments past the pixel's four output registers` |
| a colour clear (`do_clear` or `clear_quad`), as `several_int_targets_drawn` already excludes | `yakogl/pvr: clear refused: a colour clear of an integer colour attachment beside another (not built)` |
| blending enabled (`d.blend`) | `yakogl/pvr: draw refused: blending with an integer colour attachment beside another (not built)` |
| a colour mask that keeps some channels (`d.color_mask & 0xf` neither 0 nor `0xf`), which covers every mask that keeps only some of an integer target's channels | `yakogl/pvr: draw refused: a colour mask that keeps some channels, with an integer colour attachment beside another (not built)` |
| the program is not the mix's: `fs.blend_key != shc_blend_key_mixed(&per_target_keys(d))` or `fs.num_outputs != span` (a missing variant, the stencil-only KEEP key, a program compiled for another list) | `yakogl/pvr: draw refused: an integer colour attachment beside another colour attachment` (the line that exists) |

Why refuse rather than build, for now:

- **Masks.** A mask that keeps some channels needs the program to read the
  pixel and keep the masked bits per target (`pack_int_output` does it for
  one layout with `olchk` reads); for a mix it is that per target, plus the
  float classes' masked writes the blend tail makes. GL's mask is global
  in ES 3.0, so it reaches every target. Not built; refused. A mask of no
  channels is the ISP's (`color_write` false) and is drawn.
- **Blending.** ES 3.0 4.1.7 blends the float and unorm targets and skips
  the integer ones; the blend tail (`blend.rs`) blends one colour of one
  class. Not built; refused.
- **Clears.** A clear of a mixed framebuffer needs the packed clear colour
  per target in its own registers (`clear_uniform`, `backend.rs:3676-3685`,
  packs for one format; `mrt_clear_dwords`, `backend.rs:2036-2046`, only
  for 8-bit attachments). A separate item if dEQP needs it (section 7).

The per-target keys of a draw come from the formats with the same mapping
as the GL core (section 4); a helper next to `surf_out_dwords` gives them,
and a host test checks `shc_target_key_dwords(key_of(fmt)) ==
surf_out_dwords(fmt)` for every non-sRGB colour format, so the program's
layout and the emit's cannot drift apart.

## 6. Tests the later items add

Run each with the feature set in its header; for all of these it is
`--no-default-features --features host-test,gpu-powervr,shaderc-online`.

- **`src/target_key.rs` `tests`** (`design-key`): the four worked keys
  of section 2.2; the round trip; `None` for one target, for no integer
  target, for one layout everywhere, for a gap, for an invalid per-target
  key (a masked integer key, a blend key, the sRGB class), for a list past
  four registers (RGBA8 + RGBA32UI); `shc_blend_key_is_mixed` false for
  every key the existing sweeps build (blend x mask x class, the sRGB keys,
  the integer keys with every mask, `SHC_BLEND_KEY_SRGB_ENCODED`,
  `SHC_BLEND_KEY_KEEP`, `SHC_VIV_KEY_FRONT_FLIPPED`, 0); the layouts of
  the table in section 3.
- **`tests/es3_int_mixed_out_pvr.rs`** (new; `chain-core` and
  `chain-pack`): the program compiled under the mixed key through
  `compile_fragment_for`, its record checked (`blend_key`, `num_outputs` =
  span, `uses_fbfetch` 0), its bytes run through `usc_exec::run_binary`
  (`tests/usc_exec/mod.rs`, which models `pck` u8888 and f16f16) as
  `tests/es3_int_out_alu_pvr.rs` does:
  - RGBA8 + R32UI (`0x0204_C000`): `out vec4` at 0, `out uint` from
    arithmetic at 1; pixel output 32 the u8888 dword, 33 the uint.
  - two integer layouts, R32UI + R32I (`0x020C_A400`): a highp float
    written as `floatBitsToUint` at 0 and an `int` at 1 (a negative value,
    so the sign bits are checked), the executor's shape for a float and an
    int result; 32 the float's bits, 33 the int.
  - three targets, RGBA8 + R32I + RGBA16F (`0x034C_C000`): 32 the u8888
    dword, 33 the int, 34 and 35 the halves.
  - the hole, RGBA8 + RG32UI (`0x0205_4000`): 32, 34, 35 written once
    each and 33 never.
  - the compiler's refusals of section 3.2 by their strings.
- **`src/pvr/backend_tests.rs`** (`pvr-accept`; the module is `rig_tests`,
  `backend.rs:9658-9660`):
  - `integer_targets_refuse_partial_masks_and_company` (`:5565`) keeps its
    clear-mask half; its RGBA8888 + R32UI draw is drawn when its `fs` is a
    record with the mixed key and `num_outputs` 2, and still refused with
    the fallback line when the `fs` has another key.
  - a new test per refusal of section 5, each asserting its log line.
  - a new test of the end-of-tile words for RGBA8 + R32UI, RGBA8 + RG32UI
    and RGBA16F + R32UI + RGBA8: per emitted attachment, STATE_WORD1's
    `source_pos` [14:15] equal to `start[i]` of `shc_mixed_out_layout`,
    `mrt_index` [16:18] equal to `i`, and the pack mode [7:12] the
    attachment's own (`U8U8U8U8`, `U32`, `U32U32`, `F16F16F16F16`), read
    from `rig.eot_emits()` as the sRGB test does (`:5620-5624`).
- **`tests/es3_int_mrt_alu_pvr.rs`**: `int_alu_next_to_float_lands`
  un-`#[ignore]`d, compiled under the mixed key of (RGBA8 at the float's
  location, R32I or R32UI at the integer's), and its header and the
  `[32, 33]` / `PACKED_F` expectations updated to the per-target packing.
- **`src/gl/draw.rs` tests** (`gl-key`): `current_blend_key` gives the
  mixed key for RGBA8 + R32UI and for R32UI + R32I, the integer key for two
  R32UI (unchanged), the old keys for one target and for several RGBA8 or
  sRGB targets, and no mixed key with a `GL_NONE` gap.

`tests/es3_pvr_modf.rs` does not change: its draws are one layout.

## 7. Open, not in this design

- **dEQP's shapes.** RC1's row says `common.modf` fails because the
  executor binds two integer attachments; `tests/es3_pvr_modf.rs` says
  they are of one layout, which `several_int_targets_drawn` already draws,
  except `vec3` / `vec4` (two RGBA32UI, past four registers, still refused
  in both paths). Which target mixes `fragment_out` and `fbo.color` bind
  is the read-only item `survey-deqp`; the AM62P run of those rows is
  pending after the code lands.
- **Clears and loads of a mixed framebuffer.** Refused (clears) or as for
  every several-attachment frame today: `record_frame_quad`
  (`backend.rs:3868-3931`) reloads attachment 0 alone, so a frame that
  does not start with a clear stores undefined data into attachments 1..
  outside what the draws cover. `survey-emit` reports on it; if dEQP's
  cases depend on either, that is a new item.
- **A mix of float classes without an integer target** (RGBA8 + RGBA16F):
  the key could carry it (condition 4 is what excludes it) but it is not
  RC1 and keeps its path.
- **Tile buffers**, which would lift the four-register limit (RGBA32UI in
  a mix, `modf` `vec3` / `vec4`).
