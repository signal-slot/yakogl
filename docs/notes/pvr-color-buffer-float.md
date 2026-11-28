<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR BXS-4-64: what it takes to claim GL_EXT_color_buffer_float (NS-FLOAT-RT, NS-PBO-FLOAT)

This is item `design-cbfloat` of plan #400. It is a design note: nothing in it
is implemented, and nothing was run on the board. Line numbers are at
`5f781c49`. Grep the quoted name if they drift. Mesa is 26.2.1
(`mesa-pan/mesa-26.2.1` in the deps cache, `src/imagination/`). The
VK-GL-CTS source was not checked out for this note, so the dEQP side comes
only from the message lines in the TSV.

Earlier notes this one builds on, and does not repeat:
`docs/notes/es3-float-targets.md` (the plan #360 decision, the V3D arm, the
host before/after counts), `docs/es3/pvr-formats.md` section 2.5 and the
"GL_EXT_color_buffer_float" table near its end (the Mesa PBE rows),
`docs/es3/backend-caps.md` section 4, `docs/notes/pvr-mrt4-budget.md` (the
four output registers) and `docs/notes/pvr-fragout-array-float.md` (the
tile-buffer spill defect).

## Summary

1. **The GL core is ready, and the V3D already claims the extension.**
   Plan #360 built the rules, the surfaces (`GLES_SURF_RG32F`, `_RGBA32F`,
   `_R11F_G11F_B10F`), the read-back, the 32F blend refusal and the
   `color-buffer-float` feature. On the PowerVR the claim is off for one
   reason: `FLOAT_RT_FORMATS` is 0 for every GPU but the V3D
   (`src/backend.rs:603-612`).
2. **The PowerVR draws one of the four extension-only formats, and that one
   has a known doubt.** R32F draws, but with the PBE source format
   `F16_PER_CHANNEL`. Mesa uses `8_PER_CHANNEL` (raw) for any 32-bit
   component. RG32F, RGBA32F and R11F_G11F_B10F have no PowerVR path: their
   pack mode falls through to `U8U8U8U8`, normalised.
3. **The hardware has all of it.** Every pack mode exists in our csbgen
   (`F32F32` 42, `F32F32F32F32` 13, `F10F11F11` 29, `src/pvr/csbgen.rs`). The
   raw 2- and 4-dword output, frame load and clear already exist for
   RG32UI and RGBA32UI. R11F_G11F_B10F is three halves in two dwords, which
   is the RGBA16F shader class.
4. **The claim needs work outside `src/pvr/**`.** In `src/backend.rs`: the
   capability bits and the R11F_G11F_B10F shader class. In `src/target_key.rs`:
   the per-target key. In `src/gl/draw.rs`: one branch of the variant key. In
   `src/gl/blit.rs`: the CPU blit has no decode or encode for the three
   surfaces, and 134 of the 334 cases are `fbo.blit`. In `zephyr/`: a Kconfig
   switch, because the AM62P image cannot turn the feature on today.
   Section 5 names each change exactly.
5. **Expected effect** on the 334 cases, after the whole design: about 236
   Pass, 12 still NotSupported (multisample), and up to 86 Fail on causes
   other plans own. Section 7 gives the breakdown.

## 1. The cases

Source: `docs/es3/am62p-gles3-notpass.tsv` at `5f781c49`, sorted by
`python3 -I tools/cts/am62p-notpass-causes.py --list docs/es3/am62p-gles3-notpass.tsv`:

- `NS-FLOAT-RT`: **328** NotSupported.
- `NS-PBO-FLOAT`: **6** NotSupported.
- Total **334**.

The TSV header says which build produced each row range: rows 0-21 are
`b25997e4`, rows 22-45 are `ad2b5ed`. Neither build claimed the extension,
so every one of these cases was excused, not run. There are three messages:
183 "Format not supported", 30 "Format not supported, requires
GL_EXT_color_buffer_float" (all in `fbo.render`) and 115 "Unsupported
attachment format" (all in `fragment_out`). `NS-PBO-FLOAT` is
`pbo.renderbuffer.*`, "Renderbuffer format not supported".

The table counts cases by group and by the extension-only formats the case
name carries. A blit conversion case can carry two. `fragment_out.random`
names carry no format.

| group | R32F | RG32F | RGBA32F | R11F_G11F_B10F | two of them | none named | total |
|---|---|---|---|---|---|---|---|
| `fbo.blit` (112 `conversion`, 22 `default_framebuffer`) | 30 | 30 | 30 | 32 | 12 | 0 | 134 |
| `fbo.color` (clear 4, tex2d 4, tex2darray 4, tex3d 4, texcube 4, repeated_clear 12, blend 1) | 8 | 8 | 8 | 9 | 0 | 0 | 33 |
| `fbo.render` (recreate_color 8, resize 8, shared_colorbuffer 6, shared_colorbuffer_clear 8) | 6 | 6 | 6 | 12 | 0 | 0 | 30 |
| `fbo.invalidate.format` | 1 | 1 | 1 | 1 | 0 | 0 | 4 |
| `fbo.msaa.{2,4,8}_samples` | 3 | 3 | 3 | 3 | 0 | 0 | 12 |
| `fragment_out.basic.float` | 12 | 12 | 12 | 12 | 0 | 0 | 48 |
| `fragment_out.array.float` | 12 | 12 | 12 | 12 | 0 | 0 | 48 |
| `fragment_out.random` | - | - | - | - | - | 19 | 19 |
| **NS-FLOAT-RT** | 72 | 72 | 72 | 81 | 12 | 19 | **328** |
| `pbo.renderbuffer` (`_clears`, `_triangles`), **NS-PBO-FLOAT** | 0 | 2 | 2 | 2 | 0 | 0 | **6** |

The `fragment_out.array` and `fragment_out.basic` rows are
`{format} x {lowp, mediump, highp} x {float, vec2, vec3, vec4}`. An array
case has three attachments of one format (`docs/notes/pvr-fragout-array-float.md`
section 1). The 19 `fragment_out.random` cases are seeded mixes of formats
and output counts, at least one of them an extension-only format. Which mix
each seed draws is not known without the dEQP source.

## 2. Per-format state at `5f781c49`

"Renders" means the PowerVR back end emits a correct pack mode, output
registers, frame load and clear for the surface. "Blends" means what ES 3.0
plus EXT_color_buffer_float asks for: the 16F formats and R11F_G11F_B10F
blend, and the 32F formats must refuse a blended draw with
`GL_INVALID_OPERATION` (`src/gl/draw.rs:5414-5428`, `blends_a_32f_draw_buffer`).
"Reads back" means `glReadPixels` and the CPU blit.

| format | GL surface | renders | blends | reads back | missing |
|---|---|---|---|---|---|
| R16F | `GLES_SURF_R16F` | yes: PBE `F16`, `F16_PER_CHANNEL`, 1 dword (`src/pvr/backend.rs:1203`, `:1215`) | yes, shader blend in class `SHC_RT_F16` | yes (`read_float_row`, `src/gl/framebuffer.rs:3634`; `blit.rs` `decode_float` `:620`) | nothing |
| RG16F | the RGBA16F surface (`renderbuffer_surf_format`; RG channels masked) | yes, as RGBA16F | yes | yes | nothing for the claim. `fbo.render.shared_colorbuffer.tex2d_rg16f*` fail on content, a separate defect |
| RGBA16F | `GLES_SURF_RGBA16F` | yes: `F16F16F16F16`, 2 dwords (`:1202`, `surf_out_dwords` `:1124`) | yes, `SHC_RT_F16X4` | yes | nothing (FO-ARRAY-FLOAT: three of them spill, `pvr-fragout-array-float.md`) |
| R32F | `GLES_SURF_R32F` | **doubtful**: pack `F32` (`:1204`) but source `F16_PER_CHANNEL` (`:1215-1216`, every float class). Mesa: `8_PER_CHANNEL` for any 32-bit component (`vulkan/pvr_arch_job_common.c:51-53`) over accum `F32` (`vulkan/pvr_arch_formats.c` `R32_FLOAT` row). The shader writes the raw float to output 0 (`src/shaderc/online/powervr/assemble.rs:2035`). No board case has rendered it: dEQP excused every R32F target case | the 32F refusal is in the GL core | yes, raw floats (`framebuffer.rs` `read_float_row` `_` arm; `blit.rs:629`, `:691`) | the source format `8_PER_CHANNEL`, then a board check |
| RG32F | `GLES_SURF_RG32F`, 8 bytes, for a renderbuffer (`framebuffer.rs:737`, `:870-872`). A texture is stored in the RGBA32F class and drawn as `GLES_SURF_RGBA32F` with B and A masked (`ext_float_texture` `:764-771`) | **no**: `gles_surf_is_float` is false for it (`src/backend.rs:716-718`), so `pbe_emit_words` gives `U8U8U8U8`, normalised, `8_PER_CHANNEL`. `surf_out_dwords` gives 1. `gles_surf_rt_class` gives `SHC_RT_RGBA8`, so the shader packs bytes. Frame load: `gles_surf_texfmt` gives RGBA8 | must refuse (GL core does) | `read_float_row`: yes. CPU blit: **no**, `decode_float`/`encode_float` have no arm and fall to RGBA8 bytes (`blit.rs:631-648`, `:697`) | pack mode `F32F32`, source `8_PER_CHANNEL`, 2 raw output dwords, raw frame load (`PVR_TEXFMT_RAW64`), raw clear, the variant key, the blit arms |
| RGBA32F | `GLES_SURF_RGBA32F`, 16 bytes. The texture is in its own class on the PowerVR (`texture.rs` `rgba32f_own_class` `:6618`, `GLES_TEXFMT_RGBA32F` in `PVR_SAMPLED_TEXFMTS` `src/backend.rs:215`). Full floats are sampled (`tests/` `rgba32f-cube3d-test`) | **no**: same fall-through as RG32F | must refuse (GL core does) | `read_float_row`: yes. CPU blit: **no**, and a texture attachment has no `attachment_image` surface at all (`blit.rs:179-197`: `GLES_TEXFMT_RGBA32F` goes to `integer_store_surface`, which returns `None`) | pack mode `F32F32F32F32`, `8_PER_CHANNEL`, 4 raw dwords (the RGBA32UI shape), `PVR_TEXFMT_RAW128` frame load, raw 4-dword clear, the key, the blit arms, the absent-lane swizzle for an RG32F texture (`pbe_four_lanes` `:971`) |
| R11F_G11F_B10F | `GLES_SURF_R11F_G11F_B10F`, a 4-byte word (`framebuffer.rs:739`, `:768`). The texture class `GLES_TEXFMT_R11G11B10F` is sampled (`TEXF_R11G11B10F`, `src/pvr/backend.rs:1406`) | **no**: same fall-through, `U8U8U8U8` | **no** (it must blend: `fbo.color.blend.r11f_g11f_b10f_src_over`) | `read_float_row`: yes (`float_pack::r11g11b10f_row_to_rgba_f32`, `framebuffer.rs:3676`). CPU blit: **no**, as RG32F | pack mode `F10F11F11`, source `F16_PER_CHANNEL`, accum F16 (Mesa `R11G11B10_FLOAT` row). The USC writes it as `R16G16B16_FLOAT` (`pco/pco_nir_pvfio.c:106-107`): 2 dwords of halves, which is the `SHC_RT_F16X4` class. Frame load: the F16X4 load over the R11G11B10F texel. Clear: halves. The key, the blit arms |

Two facts the table rests on:

- **Raw 32-bit lanes already work for integers.** RG32UI and RGBA32UI take
  `8_PER_CHANNEL`, `U32U32` and `U32U32U32U32`, 2 and 4 output dwords at an
  even register (`docs/notes/pvr-mrt4-budget.md` rule 2), the raw frame
  loads `PVR_INTERNAL_FBLOAD_U32X2` and `_U32X4` with `PVR_TEXFMT_RAW64` and
  `_RAW128` (`src/pvr/backend.rs:2451`, `int_load_texfmt` `:2744`), and a
  32-bit clear "as it is" (`pack_clear_int`, `:4582-`). A 32F target is the
  same register and memory shape. Only the PBE pack mode differs, and the
  input to the clear (`f32::to_bits`).
- **The shader's integer pack is a raw move for 32-bit lanes.**
  `TargetPack::Int` "bit-inserted into its layout's dwords with no
  conversion" (`src/shaderc/online/powervr/chain.rs:1617-1620`). For a
  32-bit field at offset 0 that is the float's bits, which is what Mesa does
  for any format whose channel size is 32 ("no packing required",
  `pco_nir_pvfio.c`, the `get_max_channel_size == 32` case). Whether
  `pack_int_output` (`chain.rs:1441`) accepts a float-typed output is not
  verified. It is the first host test of section 6.

## 3. Where the claim is decided

The extension name is listed when **all** of these hold:

1. `src/backend.rs:603-612` `FLOAT_RT_FORMATS`: the bits for RG32F,
   RGBA32F and R11F_G11F_B10F. Today they are
   `#[cfg(all(feature = "gpu-v3d", feature = "color-buffer-float"))]` → 7
   and otherwise 0. **This is the PowerVR claim point.**
2. `src/backend.rs:619-622` `color_buffer_float_capable()`: all three bits
   are set.
3. `src/gl/state.rs:192-193` `COLOR_BUFFER_FLOAT_EXTENSION =
   cfg!(feature = "color-buffer-float") && !crate::REPORT_ES2 &&
   backend::color_buffer_float_capable()`. That puts the name
   (`state.rs:133`) in `EXTENSION_NAMES` and in the string (`state.rs:291`).
4. The GL core then makes R32F colour-renderable (`framebuffer.rs:716-722`,
   `r32f_color_renderable` reads the listed name). RG32F, RGBA32F and
   R11F_G11F_B10F become colour-renderable where `rt_format_drawable` is
   also true (`framebuffer.rs:749-752`; `src/backend.rs:661-668` reads
   `FLOAT_RT_FORMATS`).

The feature reaches a build through `Cargo.toml:171` (`color-buffer-float`),
`capi/Cargo.toml:85`, `CMakeLists.txt:113` (`YAKOGL_COLOR_BUFFER_FLOAT`) and
`capi/yakogl_capi.cmake:216`. **The Zephyr image has no switch for it.**
`zephyr/Kconfig:756` and `zephyr/CMakeLists.txt:181-185` forward only
`YAKOGL_MSAA_FBO`. So the AM62P dEQP image cannot claim the extension until
a `CONFIG_YAKOGL_COLOR_BUFFER_FLOAT` exists and `am62p-deqp-images.sh`
(`:153`, the gles3 `MODARGS`) passes it.

## 4. Fix design (PowerVR arm)

Follow the V3D arm `e59119a1` (one commit: back end, capability, tests).
Keep the claim behind `color-buffer-float`, so a default build changes in
one place only: R32F's source format, step P1.

**P1. R32F source format** (`src/pvr/backend.rs` `pbe_emit_words`, `:1188-1231`).
Choose the source per format, as Mesa's `pvr_arch_pbe_get_src_format_and_gamma`
does: `8_PER_CHANNEL` when any component is 32 bits (R32F, RG32F, RGBA32F,
and the integer formats already), otherwise `F16_PER_CHANNEL` for a float
or halves format (R16F, RGBA16F, R11F_G11F_B10F, sRGB, RGB10_A2). `norm` is
0 for all the float formats. It is not gated on the feature: Mesa's value is
the right one in any build. Without the extension, R32F is not a colour
target in an ES 3.0 build anyway (`c558647b`).

**P2. Pack modes and dwords** (`src/pvr/backend.rs`).

- `pbe_emit_words`: `GLES_SURF_RG32F` → `ROGUE_PBESTATE_PACKMODE_F32F32`,
  `GLES_SURF_RGBA32F` → `_F32F32F32F32`, `GLES_SURF_R11F_G11F_B10F` →
  `_F10F11F11`. Match the three formats before the `_ if float` arm, as
  `gles_surf_is_float` stays false for them.
- `surf_out_dwords` (`:1124`): 2, 4 and 2.
- `pbe_four_lanes` (`:971`): add `GLES_SURF_RGBA32F`, so an RG32F texture
  drawn as RGBA32F stores the constants 0 and 1 for the absent B and A, as
  RG16F does on RGBA16F. R11F_G11F_B10F has no alpha lane, and its
  `F10F11F11` store ignores source channel 3.
- `pbe_reg_words`: no change expected. The line stride is in pixel pairs
  whatever the size (`:976-990`). Pin it for a 16-byte pixel all the same.

**P3. The shader variant per target.** The GL core picks the variant key
(`src/gl/draw.rs:1485` `current_blend_key`). The PowerVR back end only
checks it (`src/pvr/backend.rs:7451` `int_targets_drawn`, `:7500-7518`).

- RG32F and RGBA32F take the integer key of the unsigned 32-bit layout of
  their channel count: `shc_blend_key_int_rt(ShcIntRt { channels: 2 | 4,
  bits: 32, signed: false })`. That is the raw pack of section 2. It needs
  no new class code, and the 2-bit class field
  (`src/shaderc/mod.rs:100-110`) has none free. Blending never applies (the
  GL core refuses it). A colour mask uses the integer key's mask bits
  (`shc_blend_key_mask_off`), as for RG32UI.
  - New in `src/backend.rs`: `pub const fn gles_surf_raw_f32_layout(format: u8) -> Option<GlesIntLayout>`.
    It gives `Some((2, 32, false))` for `GLES_SURF_RG32F` and
    `Some((4, 32, false))` for `GLES_SURF_RGBA32F` under
    `cfg(feature = "gpu-powervr")`, and `None` otherwise. The V3D, GC7000
    and Mali blend in fixed function, so their key is 0
    (`draw.rs:4469-4479`). Gating it keeps their keys unchanged anyway.
  - `src/gl/draw.rs:1508`: `gles_surf_int_layout(fmt)` becomes
    `gles_surf_int_layout(fmt).or(backend::gles_surf_raw_f32_layout(fmt))`
    (section 5, G1).
  - `src/target_key.rs:64-72` `gles_surf_target_key`: the same `.or(...)`
    before the `fmt > GLES_SURF_RGB10_A2` return, and R11F_G11F_B10F to the
    F16X4 class key. Without this, every mixed-target draw that has a 32F or
    R11F_G11F_B10F target is refused "not a mix" (several
    `fragment_out.random` cases).
  - The PowerVR checks: `int_targets_drawn` (`src/pvr/backend.rs:7451-`)
    must accept a raw-32F target whose key is the U32 layout. The
    `num_outputs == dwords` test (`:7470`) holds as for RG32UI and RGBA32UI.
- R11F_G11F_B10F takes the `SHC_RT_F16X4` class. That means
  `src/backend.rs:704-712` `gles_surf_rt_class`:
  `GLES_SURF_R11F_G11F_B10F => SHC_RT_F16X4` (gated on `gpu-powervr`, or
  ungated: only the PowerVR and the GC7000 read it, and the GC7000 never
  gets the surface). Blending then is the F16X4 shader blend over the
  frame-loaded halves. The PBE's `F10F11F11` store clamps and rounds.

**P4. Frame load and clear** (`src/pvr/backend.rs`).

- `fbload_row` (`:2476`): RG32F and RGBA32F go to the raw rows
  `4 + fbload_int_index(2 | 4)` (`FBLOAD_U32X2`, `_U32X4`), bound through
  `int_load_texfmt` (by bytes: RAW64, RAW128). R11F_G11F_B10F goes to the
  F16X4 row, bound as `TEXF_R11G11B10F`. The texel there is decoded to
  floats and packed as halves by the F16X4 program. `gles_surf_texfmt`
  already maps it (`src/backend.rs:732`).
- `clear_uniform` (`:4571`): RG32F and RGBA32F give the raw
  `c.map(f32::to_bits)`, 2 or 4 dwords, as `pack_clear_int` does for
  32-bit lanes. R11F_G11F_B10F gives the four halves of `pack_clear_float`
  for RGBA16F.
- `clears_by_quad` (`:4796`): true for all three formats.

**P5. Capability** (`src/backend.rs:603-612`). Add
`#[cfg(all(feature = "gpu-powervr", feature = "color-buffer-float"))]` → all
three bits. Update the doc comments of `FLOAT_RT_FORMATS`,
`color_buffer_float_capable` and `GLES_SURF_RG32F`, which say "the V3D"
alone, the `Cargo.toml:160-171` comment, and `src/backend.h:100` ("no back
end draws them yet").

**P6. Multiple targets.** The pixel has four output registers. 3 x R32F (3
dwords) fits. 3 x RG32F (6), 3 x RGBA32F (12) and 3 x R11F_G11F_B10F (6) do
not. For R11F_G11F_B10F, `mrt_layout` places the third target in tile
buffer 0, which is the FO-ARRAY-FLOAT spill that draws wrong on the board
(`docs/notes/pvr-fragout-array-float.md` section 2). For raw-32F, the
integer pack refuses outright ("integer colour outputs past the pixel's
four output registers", `chain.rs:1470-1471`). That is a refusal by name,
not a wrong picture, so it is acceptable as the first step. Spilling raw
lanes to tile buffers comes after the FO-ARRAY-FLOAT fix, in its own item.

**P7. Image build** (`zephyr/Kconfig`, `zephyr/CMakeLists.txt`,
`tools/cts/am62p-deqp-images.sh`). Add `config YAKOGL_COLOR_BUFFER_FLOAT`,
forwarded as `YAKOGL_COLOR_BUFFER_FLOAT`, modelled on `YAKOGL_MSAA_FBO`
(`zephyr/Kconfig:756-768`, `zephyr/CMakeLists.txt:181-185`). Then add
`-DCONFIG_YAKOGL_COLOR_BUFFER_FLOAT=y` to the gles3 `MODARGS` of
`am62p-deqp-images.sh:153`. `tools/cts/**` is held by board-a and board-b
of this plan, so this step waits for them.

## 5. Changes outside `src/pvr/**`, named exactly

| id | file | change | why it cannot live in `src/pvr` |
|---|---|---|---|
| G1 | `src/gl/draw.rs:1508` (`current_blend_key`) | `if let Some(l) = gles_surf_int_layout(fmt).or(backend::gles_surf_raw_f32_layout(fmt))`, with the rest of the branch unchanged. Optional, same file `:4495-4499`: in the stencil-only-discard key, a raw-32F target needs the raw key, not `gles_surf_rt_class` (an integer target has the same pre-existing gap there) | the GL core picks the shader variant. The back end receives the binary already compiled (`d.fs`) |
| G2 | `src/gl/blit.rs:179-197` (`attachment_image`) | texture arms `backend::GLES_TEXFMT_RGBA32F => GLES_SURF_RGBA32F` (it holds RG32F too) and `GLES_TEXFMT_R11G11B10F => GLES_SURF_R11F_G11F_B10F`, taken only where `framebuffer::ext_float_texture` gives a surface | the CPU blit is the GL core's |
| G3 | `src/gl/blit.rs:620-660` (`decode_float`) and `:670-710` (`encode_float`) | arms for `GLES_SURF_RG32F` (2 f32 LE), `GLES_SURF_RGBA32F` (4 f32 LE) and `GLES_SURF_R11F_G11F_B10F` (`float_pack::unpack_r11g11b10f` / `pack_r11g11b10f`, `src/gl/float_pack.rs:94`, `:100`) | as G2. This is a defect of the V3D build with the feature on, too: its `fbo.blit` 32F rows go through the same CPU copy |
| B1 | `src/backend.rs` | `gles_surf_raw_f32_layout` (P3), the `gles_surf_rt_class` arm for R11F_G11F_B10F (P3), the `FLOAT_RT_FORMATS` arm (P5) | shared tables, not GL core |
| B2 | `src/target_key.rs:64-72` | `gles_surf_target_key` for the three surfaces (P3) | crate-level key table, shared by `src/gl/target_keys.rs` and `src/pvr/target_keys.rs` |
| Z1 | `zephyr/Kconfig`, `zephyr/CMakeLists.txt` | `YAKOGL_COLOR_BUFFER_FLOAT` (P7) | the image build |

No `YakoglBackend` field is added, so no struct literal in `tests/` moves.

## 6. Host tests to add

All of them run with
`cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,color-buffer-float`
unless the item says otherwise.

1. **Raw-32F pack compiles** (`src/shaderc/online/powervr` unit test, or a
   new `tests/pvr_color_buffer_float.rs`). An `out highp vec4` and an
   `out highp vec2`, compiled under the U32x4 and U32x2 integer keys, give
   exactly 4 and 2 pixel-output moves of the colour registers, with no
   conversion instruction. This is the open question of section 2.
2. **PBE words per format** (`src/pvr/backend.rs` tests, beside
   `clear_colour_of_a_float_target_is_packed_for_its_class` `:11861`):
   - `pbe_emit_words` source format and pack mode for R16F (F16P, `F16`),
     RGBA16F (F16P, `F16F16F16F16`), R32F (**8P**, `F32`), RG32F (8P,
     `F32F32`), RGBA32F (8P, `F32F32F32F32`) and R11F_G11F_B10F (F16P,
     `F10F11F11`), all with `norm` 0;
   - `surf_out_dwords` 1, 2, 1, 2, 4, 2;
   - `clear_uniform` raw bits for the 32F formats and halves for
     R11F_G11F_B10F;
   - `clears_by_quad` true;
   - `fbload_row` raw rows for RG32F and RGBA32F and the F16X4 row for
     R11F_G11F_B10F;
   - `pbe_swizzle` gives ZERO and ONE for absent B and A on RGBA32F.
   `src/pvr/backend_csbgen_tests.rs:225-241` (`PBE_PACKMODE_F32` as the
   default float arm) must learn the new arms.
3. **GL draw through the mock back end** (`tests/pvr_color_buffer_float.rs`,
   new, modelled on `tests/es3_color_buffer_float_v3d.rs`):
   - the extension is listed;
   - for each of R32F, RG32F (renderbuffer, and texture in the RGBA32F
     class), RGBA32F and R11F_G11F_B10F, a complete FBO and a draw whose
     captured `GlesDraw` has the expected target format, PBE words, key and
     `num_outputs`;
   - `glClear` and `glClearBufferfv` words;
   - blending on a 32F target is `GL_INVALID_OPERATION`, and on
     R11F_G11F_B10F it gives the F16X4 blend variant;
   - 3 x R32F is laid out at r0, r1, r2, and 3 x RGBA32F is refused by name.
4. **Read-back and blit** (`src/gl/blit.rs` unit tests for G2 and G3; no GPU
   gate): decode and encode round trips for the three surfaces,
   RGBA32F → RGBA8 and R11F_G11F_B10F → R32F conversions, and an RGBA32F
   texture attachment is an `attachment_image`. `glReadPixels` of each
   format with `GL_RGBA`/`GL_FLOAT` over mock memory filled with known
   bits.
5. **Pins that flip.** `tests/es3_color_buffer_float.rs`
   (`feature_on_not_claimed_while_incapable` `:309-325` and its `GPU`
   arms) must expect the claim on `powervr` with the feature on.
   `tests/es3_mrt4_formats_pvr.rs:790-792` pins R11F_G11F_B10F as
   `NotRenderable` on `GLES_SURF_RGBA8888`, and probably RG32F and RGBA32F
   in the same table. Under the feature those rows must become drawable
   rows with their own surfaces. Grep `tests/` for `R32F`, `RG32F`,
   `RGBA32F` and `R11F` under `gpu-powervr`. A new test file needs a
   `tools/check.sh` row (plan #387 holds that file).
6. **Host dEQP, HOST-ONLY**: for verdicts, not pictures. `GPU=powervr`,
   `EXTRA_FEATURES=color-buffer-float`, the globs of
   `docs/notes/es3-float-targets.md` section 8 plus `fbo.blit.*`,
   `fragment_out.*` and `fbo.render.*`. Expected: the 334 cases leave
   NotSupported, except the 12 `fbo.msaa` cases (section 7), and no new
   Fail on a GL error.

## 7. Expected effect on the 334 cases

These are expectations, not measurements.

| cases | expected after P1-P7 and G1-G3 | why |
|---|---|---|
| `pbo.renderbuffer` 6 | Pass | API and read-back. The V3D's host run moved its four 32F cases NotSupported → Pass (`es3-float-targets.md` section 8) |
| `fbo.invalidate.format` 4 | Pass | single target, clear and read back |
| `fragment_out.basic.float` 48 | Pass | one target, fits the four registers |
| `fragment_out.array.float` r32f 12 | Pass | 3 dwords, no spill |
| `fragment_out.array.float` rg32f, rgba32f 24 | Fail (refused by name, P6) | past four registers: needs raw-lane tile-buffer spill after FO-ARRAY-FLOAT |
| `fragment_out.array.float` r11f_g11f_b10f 12 | Fail | the FO-ARRAY-FLOAT spill, the same 6-dword layout as 3 x RG16F |
| `fragment_out.random` 19 | some Pass, rest Fail | depends on each seed's mix. Mixes past four registers and spills fail as above |
| `fbo.color` clear, tex2d, tex2darray, repeated_clear, blend 25 | Pass | their RGBA16F/R16F peers are not in the not-pass list (they pass) |
| `fbo.color` tex3d, texcube 8 | Fail until `fbo-tex3d-slices` and `fbo-cube-faces` | every format of these two sub-groups fails today, RGBA8 included |
| `fbo.render` 30 | Pass, with a risk on `tex2d_rg32f` (3) | the RG16F peers `shared_colorbuffer*.tex2d_rg16f*` fail today. RG32F is the same channel-masked shape. Find that cause first |
| `fbo.blit` 134 | Pass with G2 and G3. Without them, Fail | the CPU blit |
| `fbo.msaa` 12 | NotSupported | `8_samples` is past MAX_SAMPLES 4. For 2 and 4 samples, `msaa::multisample_renderable` (`src/gl/msaa.rs:82-93`) lists no 32F or R11F_G11F_B10F format, so GL_SAMPLES is empty ("Sample count not supported", as `fbo.msaa.2_samples.rgba16f` today). EXT_color_buffer_float does not require multisampled float |

Totals: about 236 Pass (6 + 4 + 48 + 12 + 25 + 30 + 134 − 3 at risk, plus
some of the 19 random), 12 NotSupported, and 36 + 8 + up to 3 + up to 19 Fail
on causes other items own. After the claim, `tools/cts/am62p-notpass-causes.py`
needs its NS-FLOAT-RT and NS-PBO-FLOAT rules retired or narrowed, and a rule
for the 8 multisample "Sample count not supported" lines (`NS-MSAA-RGBA16F`
covers only rgba16f).

## 8. Order and risks

- **Order**: P1 with its test first, because it changes a default build. Then
  P2-P4 with B1, B2 and G1 behind the feature, with tests 1-3. G2 and G3 can
  land on their own: they are GL core and help the V3D too. P5 is the claim,
  then P7, then a board run of the `fbo.*`, `fragment_out.*` and
  `pbo.renderbuffer` rows on the AM62P.
- **Risk, R32F source format**: `F16_PER_CHANNEL` with `F32` may already store
  the raw float correctly, or it may truncate it to a half. Neither is
  measured. P1 follows Mesa, so either way it is safe.
- **Risk, PBE 128-bit store**: RGBA32UI already stores 4 dwords on this core
  (`pbe2_in_xe`, `docs/notes/pvr-int-mixed-targets.md` 64-67). RGBA32F uses
  the same path, with only the pack mode changed.
- **Not in this design**: EXT_float_blend (32F blending), float
  multisample, and spilling raw 32F lanes to tile buffers.
