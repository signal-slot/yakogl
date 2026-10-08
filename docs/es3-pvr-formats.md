<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 internal formats on the PowerVR backend

Design for plan #3 items `int-rt-pvr` (integer render targets) and
`formats-pvr` (sized texture formats). It gives one row per ES 3.0 sized
internal format (ES 3.0.6 tables 3.13 and 3.14, the compressed ETC2
formats included): the PBE pack format and the USC-side conversion for
rendering, the TPU texture format and swizzle for sampling, filterability,
and the sample return type. Every mapping cites Mesa 26.2.1. It then names the yakogl functions and tables that hold today's
formats, what each must gain, how integer fragment outputs and raw
`glClearBuffer{i,ui}v` clears go through the end-of-tile and clear programs,
whether `src/pvr/internal.rs` must change, the ES 2.0 boundary, and the host
tests that check the PBE and texture-state words per format without a board.

Nothing here is implemented yet; this file is the only change.

## Sources and citation shorthand

Mesa 26.2.1, two copies:

- the vendored subset `third_party/mesa-26.2.1/` (the csbgen packet XMLs);
- the full release tree `$CCFARM_DEPS_DIR/mesa-26.2.1-src` (the C sources;
  tarball sha256 `c47e81bd…` as recorded in `third_party/mesa-26.2.1/UPSTREAM`).

Shorthands used below (all Mesa paths are under `src/imagination/` unless
said otherwise):

| short | file |
|---|---|
| `arch-formats.c` | `vulkan/pvr_arch_formats.c` |
| `formats.c` | `vulkan/pvr_formats.c` |
| `formats.h` | `vulkan/pvr_formats.h` |
| `tex-state.c` | `vulkan/pvr_arch_tex_state.c` |
| `job-common.c` | `vulkan/pvr_arch_job_common.c` |
| `cmd-buffer.c` | `vulkan/pvr_arch_cmd_buffer.c` |
| `image.c` | `vulkan/pvr_arch_image.c` |
| `usc.c` | `vulkan/pvr_usc.c` |
| `pvfio.c` | `pco/pco_nir_pvfio.c` |
| `nir-tex.c` | `pco/pco_nir_tex.c` |
| `pco.c` | `pco/pco.c` |
| `pco_isa.py` | `pco/pco_isa.py` |
| `iface.h` | `common/pvr_iface.h` |
| `bxs.h` | `common/device_info/bxs-4-64.h` |
| `pbestate.xml` | `csbgen/rogue/pbestate.xml` (vendored) |
| `texstate.xml` | `csbgen/rogue/texstate.xml` (vendored) |
| `u_format.yaml` | `src/util/format/u_format.yaml` (format channel swizzles) |
| `st_format.c` | `src/mesa/state_tracker/st_format.c` (GL sized format → pipe format) |
| `teximage.c` | `src/mesa/main/teximage.c` (GL texture format swizzles) |

In the format tables, a bare `:NNN` is a line of `arch-formats.c`.

yakogl paths are repo-relative (`src/pvr/backend.rs:792` etc.).

## 1. How the hardware encodes a format

### 1.1 Mesa's two format tables

Mesa's PowerVR driver maps every pipe format twice:

- `pvr_format_table` (`arch-formats.c:87-170`): pipe format →
  `ROGUE_TEXSTATE_FORMAT_*` (the TPU texel format, `texstate.xml:95-193`)
  plus bind flags. The bind flags (`arch-formats.c:39-52`) say what the
  format may be used as: `_V` vertex attribute, `_T` sampler view, `_R`
  render target, `_I` storage image, `_Z` depth-stencil. The `_R` flag is
  renderability: RGB9_E5 has `VT__` (`:151`) — texture and vertex only, not
  renderable, exactly as ES 3.0 table 3.13 says.
- `pvr_pbe_format_table` (`arch-formats.c:195-266`): pipe format →
  `ROGUE_PBESTATE_PACKMODE_*` (the PBE pack mode, `pbestate.xml:60-123`)
  plus `PVR_PBE_ACCUM_FORMAT_*` (the per-pixel accumulator layout the USC
  writes and the clear colour is packed into).

Accessors: `pvr_arch_get_tex_format` / `_aspect` (`arch-formats.c:302-324`),
`pvr_arch_get_pbe_packmode` / `_get_pbe_accum_format`
(`arch-formats.c:326-340`). A combined depth-stencil image sampled as a
texture defaults to the depth aspect (`tex-state.c:190-191`); the stencil
aspect comes from the third `FORMAT_DEPTH_STENCIL` argument
(`arch-formats.c:78-84`, `:152-157`).

### 1.2 The PBE words

`pvr_setup_pbe_state` (`cmd-buffer.c:1100-1229`) fills
`ROGUE_PBESTATE_STATE_WORD1` and `REG_WORD0` through
`pvr_arch_pbe_pack_state` (`job-common.c:82-297`):

- `packmode` [7:12] (`pbestate.xml:240`) from the PBE table
  (`cmd-buffer.c:1148`).
- `norm` [13] (`pbestate.xml:239`) = "every channel is normalized",
  `pvr_vk_format_is_fully_normalized` (`formats.h:264-273`, used at
  `cmd-buffer.c:1146-1147`). UNORM/SNORM → 1; float and integer → 0.
- `source_format` [19] (`pbestate.xml:155-158`, `:236`): how the PBE reads
  the USC's tile-buffer registers. The rule
  (`pvr_arch_pbe_get_src_format_and_gamma`, `job-common.c:39-79`):
  a 32-bit component or any integer format → `8_PER_CHANNEL` (`:51-53`);
  float → `F16_PER_CHANNEL` (`:54-55`); sRGB → gamma on + `F16_PER_CHANNEL`
  (`:56-60`); depth over 16 bits or any stencil → `8_PER_CHANNEL`
  (`:61-69`); a channel-0 width over 16 → `8_PER_CHANNEL`, over 8 →
  `F16_PER_CHANNEL` (`:70-73`); otherwise `F16_PER_CHANNEL` unless the USC
  packs to 8 bits (`with_packed_usc_channel`, `cmd-buffer.c:1130-1135`:
  `has_usc_f16sop_u8` and a UNORM/SNORM format) → `8_PER_CHANNEL`
  (`:74-77`).
- `source_pos` [14:15] (`pbestate.xml:160-165`, `:238`): which USC output
  registers this attachment reads — position 0 or 4 → `STARTPOS_BIT0`, 1 or
  5 → `STARTPOS_BIT32`, 2 or 6 → `BIT64`, 3 or 7 → `BIT96`
  (`cmd-buffer.c:1186-1206`); positions 4..7 need `eight_output_registers`
  (`cmd-buffer.c:1184`), and there `source_pos_offset_128` [28]
  (`pbestate.xml:232`) picks the upper 128 bits (`job-common.c:124-129`).
- `mrt_index` [16:18] (`job-common.c:133`): the attachment number.
- swizzle `swiz_chan0..3` [20:31] of `REG_WORD0` (`pbestate.xml:262-265`):
  the format's channel swizzle (`pvr_get_format_swizzle`,
  `formats.c:339-343` — the util_format `desc->swizzle`), applied at
  `cmd-buffer.c:1137-1138` and written at `job-common.c:181-272`.
- gamma / `twocomp_gamma` [4]/[5] (`pbestate.xml:267-268`): sRGB render
  targets get `gamma` (`job-common.c:166-172`).

### 1.3 What the USC writes into the tile buffer

The fragment shader's output is packed by PCO's NIR lowering, not by the
PBE. `lower_pfo_store` (`pvfio.c:319-377`) takes the render target's pipe
format from `fs->output_formats[location]` (`:344`), maps it through
`to_pbe_format` (`:350`, body `:86-123` — packed formats widen to their
16-bit form: RGB565→R8G8B8, RGB10_A2→R16G16B16A16_FLOAT, R11G11B10→F16×3,
sRGB gets `fsat` at `:100`), then `pack_to_format` (`:353`, body
`:149-218`):

- a format with a 32-bit channel needs no packing: the components are
  stored as they are (`pvfio.c:180-182`);
- a pure integer format is bit-packed with `nir_bitfield_insert_imm` per
  channel (`pvfio.c:184-198`, the insert at `:191`) — no rounding, no
  clamping beyond the field;
- everything else is packed with `nir_pack_pco` (`pvfio.c:200-217`, the
  call at `:211`), the PCK instruction.

The packed value (a `uvec` variable, `pvfio.c:373`) is then moved to the
PIXOUT register with a plain MOV (`trans_store_output_fs`,
`pco_trans_nir.c:629-660`): the USC does no further type conversion. The
load path is the mirror: `lower_pfo_load` unpacks with
`unpack_from_format` (`pvfio.c:379-430+`, body from `:220`).

So for integer render targets the USC's job is only the bit-packing; the
PBE's `packmode`/`norm`/`source_format` words describe the same layout from
the other side.

### 1.4 Clear colour

The clear colour is not a uniform the program computes with. It is packed
on the CPU into the clear-constants buffer and read back as raw dwords:

- `pvr_get_hw_clear_color` (`formats.c:145-218`) packs the RGBA floats or
  ints per accum format: `U8`/`S8`/`UINT8`/`SINT8` (byte per channel),
  `U16`/`S16`/`F16`/`UINT16`/`SINT16`, `U1010102` (one dword,
  `pvr_pack_a2x10y10z10_uint`, `formats.c:103-121`; the PBE cannot take a
  swizzled 1010102 UINT, `formats.c:200-201`), `F32`/`UINT32`/`SINT32`
  (dword per channel). The buffer size is
  `pvr_get_pbe_accum_format_size_in_bytes` (`formats.c:57-94`: 8-bit
  accum → channels×1 byte, 16-bit → ×2, 32-bit → ×4, U1010102 → 4).
- The buffer is a preamble constant block (`cmd-buffer.c:753-800`, size
  `:789-790`, packing `:797-799`).
- The clear/loadop program (`pvr_uscgen_loadop`, `usc.c:1023-1281`) reads
  each dword with `nir_load_preamble` and stores it raw
  (`:1122-1135`, `src_type` 32-bit, no conversion). Its output variable is
  `ivec`/`uvec` for integer formats (`:1050-1056`) and its
  `output_formats` are `R32_UINT`/`R32G32_UINT`/`R32G32B32_UINT`/
  `R32G32B32A32_UINT` chosen by the accum dword count (`:1058-1077`), so
  `pack_to_format` stores them without touching the bits. A load (not a
  clear) outputs the destination format itself, `ivec4`/`uvec4` for pure
  integer (`:1082-1095`). Depth clears output `R32_FLOAT`
  (`:1152-1153`, format from `pvr_uscgen_format_for_accum`, `:1001-1021`).

The end-of-tile program (`pvr_usc_eot`, `usc.c:86-181`) only carries the
PBE state words (STATE_WORD0/1 from the preamble or immediates,
`:159-161`) and runs `nir_emitpix_pco` per attachment; it converts nothing
— the PBE packs.

### 1.5 Texture state and PCK_INFO

`pvr_arch_pack_tex_state` (`tex-state.c:114-324`) fills
`TEXSTATE_IMAGE_WORD0` (`texstate.xml`): `texformat` from the format table
(`:190-191`), `swiz0..3` from the composed swizzle (`:199-205`, values from
`pvr_get_hw_swizzle`, `:40-64`; `PIPE_SWIZZLE_NONE` → `SRC_ONE` for alpha,
`SRC_ZERO` otherwise), sRGB gamma bits (`:208-228`), width/height−1
(`:230-233`). Word 1 is the STRIDE variant for linear images
(`TEXSTATE_STRIDE_IMAGE_WORD1`, `:245-270`: stride−1, level count,
`alpha_msb` `:253-254`) or the twiddled variant (`:272-315`).

The sampling swizzle for a view composes the image swizzle with the view
swizzle (`image.c:166-191`); depth/stencil images remap to a colour format
when sampled (S8_UINT→R8_UINT `:173-175`, Z16→R16_UINT `:177-179`,
Z32F→R32_FLOAT `:181-183`) and their TPU swizzle puts the value in X
(`pvr_get_format_swizzle_for_tpu`, `formats.c:352-368`).

`PCO_IMAGE_META_PCK_INFO` is not a csbgen word: it is PCO's own packing
hint (`iface.h:132-142`: format bits [0:4], `SPLIT` bit 5, `SCALE` bit 6,
`ROUNDZERO` bit 7, `UNORM` bit 8; `PVR_PCK_FORMAT_INVALID` = `0b11111`).
`setup_pck_info` (`tex-state.c:84-112`) builds it from
`pco_pipe_to_pck_format` (`pco.c:387-463`) and stores it at
`tex-state.c:321`. The format numbers are `pco_isa.py:980-1003`
(`u8888`=0, `s8888`=1, `u1616`=3, `s1616`=4, `u32`=6, `s32`=7,
`u1010102`=8, `f111110`=12, `f16f16`=14, `f32`=15, …). Note the default in
`pco.c:387-463`: integer, 32-bit, packed-float and depth/stencil formats
get **no** PCK (INVALID) — only 8/16-bit UNORM/SNORM and 16F do. PCK_INFO
is consumed by the USC's tex path for shadow-compare clamping
(`nir-tex.c:772-777`, bit 8) and image-store packing
(`nir-tex.c:936-1060`), not by the TPU itself.

### 1.6 Filterability

Mesa's rule (`pvr_get_image_format_features2`, `formats.c:246-281`, the
test at `:275-279`): a format is
`SAMPLED_IMAGE_FILTER_LINEAR` when it is not integer, not depth/stencil,
and its first component is under 32 bits (or it is block-compressed). So
every integer format and every 32-bit float format (R32F, RG32F, RGB32F,
RGBA32F) is **not** filterable; 8/16-bit, SNORM, 16F and the packed floats
are. The PBE's own filter-downscale has a parallel rule
(`pvr_arch_format_is_pbe_downscalable`, `arch-formats.c:344-392`): integer
→ false; F16 → the `has_pbe_filterable_f16` feature (`bxs.h:54`, set);
16- and 32-bit packmodes → false; everything else true.

### 1.7 Sample return type

The TPU returns the texel in its own format; the USC converts it only when
the destination is float. `pco_emit_nir_smp` (`nir-tex.c:230-240`) sets the
SMP instruction's `fcnorm` modifier from the destination type
(`.fcnorm = base_type(dest_type) == nir_type_float`, `:236`) and the
`integer` modifier from `int_mode` (`:239`). An `isampler`/`usampler` fetch
keeps the texel's integer bits (no `fcnorm`); a float sampler of a
normalized or float format gets `fcnorm`. Hardware integer lookups beyond
the basic ones are gated by `has_tpu_extended_integer_lookup`
(`bxs.h:68`; `nir-tex.c:470-472`, `:937-939`; LOD disables them,
`:607-610`).

In GL terms: integer formats sample through `isampler*`/`usampler*` only
(ivec4/uvec4), float and normalized formats through `sampler*` (vec4).

### 1.8 BXS-4-64 limits

The AM62P core (`bxs-4-64.h`): `has_pbe_filterable_f16` (`:54`),
`has_tpu_extended_integer_lookup` (`:68`), `has_tpu_image_state_v2`
(`:69`), `has_usc_f16sop_u8` (`:73`), `usc_min_output_registers_per_pix`
(`:77`, value 2 at `:104`), `max_multisample` = 4 (`:47`, `:92`).
`has_eight_output_registers` is not set, so the USC has 4 PIXOUT registers
per pixel — enough for RGBA32UI (4 dwords) and matching yakogl's
`GLES_MAX_TARGETS = 4` (`src/backend.rs:288`).

## 2. ES 3.0 table 3.13, format by format

Columns:

- **pipe** — the Mesa pipe_format the GL token maps to (Vulkan has no 24-bit
  RGB or 3×16F formats, so RGB8/RGB16F widen to their 4-channel form;
  GL_RGB565 is Mesa's `B5G6R5_UNORM` — the name lists bits LSB-first, so
  `B5G6R5` is R in the high 5 bits, which is GL's RGB565).
- **packmode / accum** — `pvr_pbe_format_table` rows.
- **USC→PBE** — `source_format`·`norm` from the rules of §1.2
  (`job-common.c:51-77`, `formats.h:264`): `8P` = `8_PER_CHANNEL`,
  `F16` = `F16_PER_CHANNEL`, `γ` = gamma on.
- **texformat** — `pvr_format_table` row (`ROGUE_TEXSTATE_FORMAT_*`,
  `texstate.xml:95-193`; compressed formats use the `FORMAT_COMPRESSED`
  enum, `texstate.xml:195-232`).
- **swizzle** — the util_format swizzle (`u_format.yaml`; delivered by
  `formats.c:339-343` into both the PBE `REG_WORD0` swizzle and the TPU
  `swiz0..3`). `X..W` are texel channels 0..3; `0`/`1` are constants.
- **filter** — §1.6 (`formats.c:275-279`).
- **sampler** — §1.7.

### 2.1 8/16-bit, packed and sRGB

| GL | pipe | packmode | accum | USC→PBE | texformat | swizzle | filter | sampler |
|---|---|---|---|---|---|---|---|---|
| R8 | R8_UNORM | U8 :199 | U8 :199 | 8P·n1 | U8 :91 | X,0,0,1 | yes | float |
| R8_SNORM | R8_SNORM | S8 :200 | S8 :200 | 8P·n1 | S8 :92 | X,0,0,1 | yes | float |
| R16 | R16_UNORM | U16 :228 | U16 :228 | F16·n1 | U16 :120 | X,0,0,1 | yes | float |
| R16_SNORM | R16_SNORM | S16 :229 | S16 :229 | F16·n1 | S16 :121 | X,0,0,1 | yes | float |
| RG8 | R8G8_UNORM | U8U8 :203 | U8 :203 | 8P·n1 | U8U8 :95 | X,Y,0,1 | yes | float |
| RG8_SNORM | R8G8_SNORM | S8S8 :204 | S8 :204 | 8P·n1 | S8S8 :96 | X,Y,0,1 | yes | float |
| RG16 | R16G16_UNORM | U16U16 :233 | U16 :233 | F16·n1 | U16U16 :125 | X,Y,0,1 | yes | float |
| RG16_SNORM | R16G16_SNORM | S16S16 :234 | S16 :234 | F16·n1 | S16S16 :126 | X,Y,0,1 | yes | float |
| RGB8 | R8G8B8A8_UNORM | U8U8U8U8 :209 | U8 :209 | 8P·n1 | U8U8U8U8 :101 | X,Y,Z,1 | yes | float |
| RGB8_SNORM | R8G8B8A8_SNORM | S8S8S8S8 :210 | S8 :210 | 8P·n1 | S8S8S8S8 :102 | X,Y,Z,1 | yes | float |
| RGBA8 | R8G8B8A8_UNORM | U8U8U8U8 :209 | U8 :209 | 8P·n1 | U8U8U8U8 :101 | X,Y,Z,W | yes | float |
| RGBA8_SNORM | R8G8B8A8_SNORM | S8S8S8S8 :210 | S8 :210 | 8P·n1 | S8S8S8S8 :102 | X,Y,Z,W | yes | float |
| SRGB8 | R8G8B8A8_SRGB | U8U8U8U8 :213 | F16 :213 | F16·γ·n1 | U8U8U8U8 :105 | X,Y,Z,1 | yes | float |
| SRGB8_ALPHA8 | R8G8B8A8_SRGB | U8U8U8U8 :213 | F16 :213 | F16·γ·n1 | U8U8U8U8 :105 | X,Y,Z,W | yes | float |
| RGBA4 | A4R4G4B4_UNORM | A4R4G4B4 :196 | U8 :196 | 8P·n1 | A4R4G4B4 :88 | Y,Z,W,X | yes | float |
| RGB5_A1 | B5G5R5A1_UNORM | A1R5G5B5 :198 | U8 :198 | 8P·n1 | A1R5G5B5 :90 | Z,Y,X,W | yes | float |
| RGB5 | B5G5R5A1_UNORM | A1R5G5B5 :198 | U8 :198 | 8P·n1 | A1R5G5B5 :90 | Z,Y,X,1 | yes | float |
| RGB565 | B5G6R5_UNORM | R5G6B5 :197 | U8 :197 | 8P·n1 | R5G6B5 :89 | Z,Y,X,1 | yes | float |
| RGB10_A2 | R10G10B10A2_UNORM | A2R10B10G10 :223 | F16 :223 | F16·n1 | A2R10B10G10 :115 | X,Y,Z,W | yes | float |

(sRGB renders with gamma because the accum is F16 and the PBE encodes to
8-bit on the way out; the USC-side `fsat` is `pvfio.c:99-101`.)

R16/RG16 read their 16-bit channels as `F16_PER_CHANNEL` (channel width
over 8, `job-common.c:70-73`). RGB5: Mesa prefers `B5G5R5X1_UNORM`
(`st_format.c:307-311`) but the PowerVR tables have no row for it, so the
`B5G5R5A1_UNORM` row applies (render swizzle `Z,Y,X,W` like RGB5_A1); the
table shows the sampling swizzle — Mesa composes its `.xyz1` texture
swizzle (`teximage.c:759`) with the image swizzle (`image.c:166-191`) so
the TPU reads A as 1.

### 2.2 Float

| GL | pipe | packmode | accum | USC→PBE | texformat | swizzle | filter | sampler |
|---|---|---|---|---|---|---|---|---|
| R16F | R16_FLOAT | F16 :232 | F16 :232 | F16·n0 | F16 :124 | X,0,0,1 | yes | float |
| RG16F | R16G16_FLOAT | F16F16 :237 | F16 :237 | F16·n0 | F16F16 :129 | X,Y,0,1 | yes | float |
| RGB16F | R16G16B16A16_FLOAT | F16F16F16F16 :245 | F16 :245 | F16·n0 | F16F16F16F16 :137 | X,Y,Z,1 | yes | float |
| RGBA16F | R16G16B16A16_FLOAT | F16F16F16F16 :245 | F16 :245 | F16·n0 | F16F16F16F16 :137 | X,Y,Z,W | yes | float |
| R32F | R32_FLOAT | F32 :248 | F32 :248 | 8P·n0 | F32 :140 | X,0,0,1 | **no** | float |
| RG32F | R32G32_FLOAT | F32F32 :251 | F32 :251 | 8P·n0 | F32F32 :143 | X,Y,0,1 | **no** | float |
| RGB32F | R32G32B32_FLOAT | F32F32F32 :254 | F32 :254 | 8P·n0 | F32F32F32 :146 | X,Y,Z,1 | **no** | float |
| RGBA32F | R32G32B32A32_FLOAT | F32F32F32F32 :257 | F32 :257 | 8P·n0 | F32F32F32F32 :149 | X,Y,Z,W | **no** | float |
| R11F_G11F_B10F | R11G11B10_FLOAT | F10F11F11 :258 | F16 :258 | F16·n0 | F10F11F11 :150 | X,Y,Z,1 | yes | float |
| RGB9_E5 | R9G9B9E5_FLOAT | SE9995 :259 | **INVALID** :259 | — (not renderable) | SE9995 :151 | X,Y,Z,1 | yes | float |

RGB9_E5 has no accum format (`arch-formats.c:259`, bind `VT__` `:151`):
texture only, never a render target — as ES 3.0 says.

### 2.3 Integer

All integer formats: `norm` 0 (`formats.h:264`), `source_format`
`8_PER_CHANNEL` (`job-common.c:51-53`), not filterable
(`formats.c:275-279`), sampled only through `isampler`/`usampler`
(§1.7). The USC packs them with bitfield inserts, never PCK
(`pvfio.c:184-198`); 32-bit ones need no packing at all
(`pvfio.c:180-182`).

| GL | pipe | packmode | accum | texformat | swizzle |
|---|---|---|---|---|---|
| R8I | R8_SINT | S8 :202 | SINT8 :202 | S8 :94 | X,0,0,1 |
| R8UI | R8_UINT | U8 :201 | UINT8 :201 | U8 :93 | X,0,0,1 |
| R16I | R16_SINT | S16 :231 | SINT16 :231 | S16 :123 | X,0,0,1 |
| R16UI | R16_UINT | U16 :230 | UINT16 :230 | U16 :122 | X,0,0,1 |
| R32I | R32_SINT | S32 :247 | SINT32 :247 | S32 :139 | X,0,0,1 |
| R32UI | R32_UINT | U32 :246 | UINT32 :246 | U32 :138 | X,0,0,1 |
| RG8I | R8G8_SINT | S8S8 :207 | SINT8 :207 | S8S8 :99 | X,Y,0,1 |
| RG8UI | R8G8_UINT | U8U8 :206 | UINT8 :206 | U8U8 :98 | X,Y,0,1 |
| RG16I | R16G16_SINT | S16S16 :236 | SINT16 :236 | S16S16 :128 | X,Y,0,1 |
| RG16UI | R16G16_UINT | U16U16 :235 | UINT16 :235 | U16U16 :127 | X,Y,0,1 |
| RG32I | R32G32_SINT | S32S32 :250 | SINT32 :250 | S32S32 :142 | X,Y,0,1 |
| RG32UI | R32G32_UINT | U32U32 :249 | UINT32 :249 | U32U32 :141 | X,Y,0,1 |
| RGB8I | R8G8B8A8_SINT | S8S8S8S8 :212 | SINT8 :212 | S8S8S8S8 :104 | X,Y,Z,1 |
| RGB8UI | R8G8B8_UINT | U8U8U8 :208 | UINT8 :208 | U8U8U8 :100 | X,Y,Z,1 |
| RGB16I | R16G16B16_SINT | S16S16S16 :240 | SINT16 :240 | S16S16S16 :132 | X,Y,Z,1 |
| RGB16UI | R16G16B16_UINT | U16U16U16 :239 | UINT16 :239 | U16U16U16 :131 | X,Y,Z,1 |
| RGB32I | R32G32B32_SINT | S32S32S32 :253 | SINT32 :253 | S32S32S32 :145 | X,Y,Z,1 |
| RGB32UI | R32G32B32_UINT | U32U32U32 :252 | UINT32 :252 | U32U32U32 :144 | X,Y,Z,1 |
| RGBA8I | R8G8B8A8_SINT | S8S8S8S8 :212 | SINT8 :212 | S8S8S8S8 :104 | X,Y,Z,W |
| RGBA8UI | R8G8B8A8_UINT | U8U8U8U8 :211 | UINT8 :211 | U8U8U8U8 :103 | X,Y,Z,W |
| RGBA16I | R16G16B16A16_SINT | S16S16S16S16 :244 | SINT16 :244 | S16S16S16S16 :136 | X,Y,Z,W |
| RGBA16UI | R16G16B16A16_UINT | U16U16U16U16 :243 | UINT16 :243 | U16U16U16U16 :135 | X,Y,Z,W |
| RGBA32I | R32G32B32A32_SINT | S32S32S32S32 :256 | SINT32 :256 | S32S32S32S32 :148 | X,Y,Z,W |
| RGBA32UI | R32G32B32A32_UINT | U32U32U32U32 :255 | UINT32 :255 | U32U32U32U32 :147 | X,Y,Z,W |
| RGB10_A2UI | R10G10B10A2_UINT | U32 :227 | U1010102 :227 | A2R10B10G10 :119 | X,Y,Z,W |

RGB8I has no 24-bit pipe format (Vulkan has no `R8G8B8_SINT`), so it takes
the RGBA8_SINT row; RGB8UI does have its own (`R8G8B8_UINT`).
RGB10_A2UI renders as one packed dword (`U32` packmode, `U1010102` accum;
clear colour via `pvr_pack_a2x10y10z10_uint`, `formats.c:103-121`) but
samples as the TPU's `A2R10B10G10` integer format.

### 2.4 Compressed (ETC2)

Compressed formats are texture-only: the `FORMAT_COMPRESSED` macro
(`arch-formats.c:70-77`) gives bind `__T__` and no PBE row — they are
never render targets, so there is no packmode, accum or USC conversion.
Their texformat numbers come from the `FORMAT_COMPRESSED` enum
(`texstate.xml:195-232`, ETC2 values `:224-230`). All are filterable
(block-compressed exception, `formats.c:278`) and sample as float.

| GL | pipe | texformat | filter | sampler |
|---|---|---|---|---|
| COMPRESSED_RGB8_ETC2 | ETC2_RGB8 | ETC2_RGB :158 | yes | float |
| COMPRESSED_SRGB8_ETC2 | ETC2_SRGB8 | ETC2_RGB :159 | yes | float |
| COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2 | ETC2_RGB8A1 | ETC2_PUNCHTHROUGHA :160 | yes | float |
| COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2 | ETC2_SRGB8A1 | ETC2_PUNCHTHROUGHA :161 | yes | float |
| COMPRESSED_RGBA8_ETC2_EAC | ETC2_RGBA8 | ETC2A_RGBA :162 | yes | float |
| COMPRESSED_SRGB8_ALPHA8_ETC2_EAC | ETC2_SRGBA8 | ETC2A_RGBA :163 | yes | float |
| COMPRESSED_R11_EAC | ETC2_R11_UNORM | EAC_R11_UNSIGNED :164 | yes | float |
| COMPRESSED_SIGNED_R11_EAC | ETC2_R11_SNORM | EAC_R11_SIGNED :165 | yes | float |
| COMPRESSED_RG11_EAC | ETC2_RG11_UNORM | EAC_RG11_UNSIGNED :166 | yes | float |
| COMPRESSED_SIGNED_RG11_EAC | ETC2_RG11_SNORM | EAC_RG11_SIGNED :167 | yes | float |

The sRGB variants get the TEXSTATE gamma bits like the other sRGB formats
(`tex-state.c:208-228`). Their PCK_INFO is plain INVALID: compressed
formats have no typed channels (`u_format.yaml:997-1003`, `channels:
[X64]`), so `pco_pipe_to_pck_format` returns INVALID and the UNORM bit
(`setup_pck_info`, `tex-state.c:108-109`) stays clear — yakogl's
`TEX_PCK_INFO_COMPRESSED_UNORM` = 0x11f (`src/pvr/backend.rs:900`) sets
that bit on top of Mesa's value. yakogl already stores the RGB8/A1/RGBA8
families as ETC2 texfmts (§4.1); the EAC R11/RG11 rows are recorded here
for completeness.

### 2.5 Render-target summary (PBE side), every ES 3.0 colour-renderable format

One row per format `int-rt-pvr` has to be able to render to, gathering what
§1.2–§1.4 and the tables above spread over several places: packmode, `norm`,
`source_format`, swizzle, how the USC packs the fragment output, and how the
clear colour is packed. Mesa citations: packmode/accum `arch-formats.c:NNN`
(`:195-266`); `norm` = `pvr_vk_format_is_fully_normalized` (`formats.h:264-273`,
every channel normalized; sRGB channels are `UN8`, hence 1); `source_format`
`job-common.c:39-79`; swizzle = util_format swizzle (`formats.c:339-343`);
USC `to_pbe_format` (`pvfio.c:86-123`) + `pack_to_format`
(`pvfio.c:149-218`); clear packing `pvr_get_hw_clear_color` by accum
(`formats.c:171-215`).

Clear-colour packing by accum (`formats.c`): U8 `:171` (f32→unorm8 ×4),
S8 `:174` (snorm8), UINT8 `:177` / SINT8 `:180` (low byte of each
component), U16 `:184` / S16 `:187` (unorm16/snorm16), F16 `:190`
(f32→f16 ×4), UINT16 `:193` / SINT16 `:196`, U1010102 `:200` (one dword,
`pvr_pack_a2x10y10z10_uint`, `:103-121`), F32 `:207`, UINT32 `:210`,
SINT32 `:213` (one dword per component). Every row packs all four RGBA
components regardless of the format's channel count (`formats.c:166-168`
TODO), so unused components are simply ignored by the PBE swizzle.

USC packing legend: `PCK` = `nir_pack_pco` (`pvfio.c:211`), `ins` =
`nir_bitfield_insert_imm` (`pvfio.c:191`), `raw` = no packing
(`pvfio.c:181-182`); "N dw" = the number of PIXOUT dwords
(`DIV_ROUND_UP(blocksizebits, 32)`, `pvfio.c:177`). Mesa packs `PCK`
channels `format_chans_per_dword` at a time (`pvfio.c:125-147`).

Core ES 3.0 (table 3.13 "color-renderable"):

| GL | packmode (`arch-formats.c`) | norm | source_format | swizzle | USC output (to_pbe_format → pack) | clear (accum) |
|---|---|---|---|---|---|---|
| R8 | U8 :199 | 1 | 8P | X,0,0,1 | R8_UNORM, PCK, 1 dw | U8 `formats.c:171` |
| RG8 | U8U8 :203 | 1 | 8P | X,Y,0,1 | R8G8_UNORM, PCK, 1 dw | U8 `:171` |
| RGB8 | U8U8U8U8 :209 | 1 | 8P | X,Y,Z,1 | R8G8B8A8_UNORM, PCK, 1 dw | U8 `:171` |
| RGBA8 | U8U8U8U8 :209 | 1 | 8P | X,Y,Z,W | R8G8B8A8_UNORM, PCK, 1 dw | U8 `:171` |
| SRGB8_ALPHA8 | U8U8U8U8 :213 + gamma | 1 | F16·γ | X,Y,Z,W | `fsat` then R16G16B16A16_FLOAT (`pvfio.c:97-104`), PCK, 2 dw | F16 `:190` (linear→f16; the PBE gamma-encodes on store) |
| RGB565 | R5G6B5 :197 | 1 | 8P | Z,Y,X,1 | R8G8B8_UNORM (`pvfio.c:89-90`), PCK, 1 dw | U8 `:171` |
| RGBA4 | A4R4G4B4 :196 | 1 | 8P | Y,Z,W,X | R8G8B8A8_UNORM (`pvfio.c:92-95`), PCK, 1 dw | U8 `:171` |
| RGB5_A1 | A1R5G5B5 :198 | 1 | 8P | Z,Y,X,W | R8G8B8A8_UNORM (`pvfio.c:92-95`), PCK, 1 dw | U8 `:171` |
| RGB10_A2 | A2R10B10G10 :223 | 1 | F16 | X,Y,Z,W | R16G16B16A16_FLOAT (`pvfio.c:103-104`), PCK, 2 dw | F16 `:190` |
| RGB10_A2UI | U32 :227 | 0 | 8P (int) | X,Y,Z,W | R10G10B10A2_UINT, `ins` shifts 0/10/20/30 sizes 10/10/10/2, 1 dw | U1010102 `:200` |
| R8I | S8 :202 | 0 | 8P | X,0,0,1 | `ins`, 1 dw | SINT8 `:180` |
| R8UI | U8 :201 | 0 | 8P | X,0,0,1 | `ins`, 1 dw | UINT8 `:177` |
| RG8I | S8S8 :207 | 0 | 8P | X,Y,0,1 | `ins`, 1 dw | SINT8 `:180` |
| RG8UI | U8U8 :206 | 0 | 8P | X,Y,0,1 | `ins`, 1 dw | UINT8 `:177` |
| RGBA8I | S8S8S8S8 :212 | 0 | 8P | X,Y,Z,W | `ins`, 1 dw | SINT8 `:180` |
| RGBA8UI | U8U8U8U8 :211 | 0 | 8P | X,Y,Z,W | `ins`, 1 dw | UINT8 `:177` |
| R16I | S16 :231 | 0 | 8P | X,0,0,1 | `ins`, 1 dw | SINT16 `:196` |
| R16UI | U16 :230 | 0 | 8P | X,0,0,1 | `ins`, 1 dw | UINT16 `:193` |
| RG16I | S16S16 :236 | 0 | 8P | X,Y,0,1 | `ins`, 1 dw | SINT16 `:196` |
| RG16UI | U16U16 :235 | 0 | 8P | X,Y,0,1 | `ins`, 1 dw | UINT16 `:193` |
| RGBA16I | S16S16S16S16 :244 | 0 | 8P | X,Y,Z,W | `ins`, 2 dw | SINT16 `:196` |
| RGBA16UI | U16U16U16U16 :243 | 0 | 8P | X,Y,Z,W | `ins`, 2 dw | UINT16 `:193` |
| R32I | S32 :247 | 0 | 8P | X,0,0,1 | raw, 1 dw | SINT32 `:213` |
| R32UI | U32 :246 | 0 | 8P | X,0,0,1 | raw, 1 dw | UINT32 `:210` |
| RG32I | S32S32 :250 | 0 | 8P | X,Y,0,1 | raw, 2 dw | SINT32 `:213` |
| RG32UI | U32U32 :249 | 0 | 8P | X,Y,0,1 | raw, 2 dw | UINT32 `:210` |
| RGBA32I | S32S32S32S32 :256 | 0 | 8P | X,Y,Z,W | raw, 4 dw | SINT32 `:213` |
| RGBA32UI | U32U32U32U32 :255 | 0 | 8P | X,Y,Z,W | raw, 4 dw | UINT32 `:210` |

All `8P` rows are `8_PER_CHANNEL` (`job-common.c:51-53` for integers,
`:75-78` for the packed/8-bit unorm rows, with `with_packed_usc_channel`
true because BXS-4-64 has `usc_f16sop_u8`, `cmd-buffer.c:1130-1135`). The
RGB565/RGBA4/RGB5_A1 "swizzle" is the pipe format's own (the names list
channels LSB first, §2); the GL-visible RGBA order is restored by it.
Not in table 3.13 as renderable and therefore not needed for ES 3.0 core,
although Mesa has rows: RGB8I/UI, RGB16I/UI, RGB32I/UI, SRGB8, R8_SNORM
and the other SNORM formats (`EXT_render_snorm`), R16/RG16 UNORM
(`EXT_texture_norm16`), RGB9_E5 (never: accum INVALID, `:259`).

Renderable only with extensions (an ES 3.0 context without the extension
must reject them as colour-renderable):

| GL | extension | packmode | norm | source_format | swizzle | USC output | clear (accum) |
|---|---|---|---|---|---|---|---|
| R16F | `EXT_color_buffer_half_float` / `EXT_color_buffer_float` | F16 :232 | 0 | F16 | X,0,0,1 | R16_FLOAT, PCK, 1 dw | F16 `:190` |
| RG16F | same | F16F16 :237 | 0 | F16 | X,Y,0,1 | R16G16_FLOAT, PCK, 1 dw | F16 `:190` |
| RGBA16F | same | F16F16F16F16 :245 | 0 | F16 | X,Y,Z,W | R16G16B16A16_FLOAT, PCK, 2 dw | F16 `:190` |
| R32F | `EXT_color_buffer_float` | F32 :248 | 0 | 8P (32-bit comp, `job-common.c:51-53`) | X,0,0,1 | raw, 1 dw | F32 `:207` |
| RG32F | `EXT_color_buffer_float` | F32F32 :251 | 0 | 8P | X,Y,0,1 | raw, 2 dw | F32 `:207` |
| RGBA32F | `EXT_color_buffer_float` | F32F32F32F32 :257 | 0 | 8P | X,Y,Z,W | raw, 4 dw | F32 `:207` |
| R11F_G11F_B10F | `EXT_color_buffer_float` | F10F11F11 :258 | 0 | F16 | X,Y,Z,1 | R16G16B16_FLOAT (`pvfio.c:106-107`), PCK, 2 dw | F16 `:190` |

RGB32F is never colour-renderable, and RGB16F only through
`EXT_color_buffer_half_float` (`EXT_color_buffer_float` lists only the
one/two/four-channel formats; RGB16F would take the RGBA16F row, `:245`).
Mesa's RGB32F row (`:254`) exists only because Vulkan renders R32G32B32. RGB9_E5 is never renderable. The sRGB clear row is the only
non-obvious one: Mesa clears gamma formats with a linear colour packed as
F16 and lets the PBE gamma bit encode it (`formats.c:190`,
`job-common.c:166-172`), so `glClearBufferfv` on SRGB8_ALPHA8 passes the
linear value through (the GL clear is specified in linear space).

Checked against Mesa 26.2.1 on 2026-10-08: the PBE packmode/accum rows of
§2.1–§2.3 and §2.5 match `pvr_pbe_format_table` (`arch-formats.c:195-266`)
line for line; §2.5 and the sRGB `norm` correction were added by item
`pvr-pbe-map`.

## 3. ES 3.0 table 3.14, depth and stencil

`FORMAT_DEPTH_STENCIL` rows: the TPU table gives combined / depth-aspect /
stencil-aspect formats (`arch-formats.c:78-84`); the PBE table gives the
combined packmode with accum INVALID (`arch-formats.c:184-188`) — depth and
stencil never go through the colour accum or the PBE pack path; the ISP
writes them and the USC clear program outputs `R32_FLOAT` for depth
(`usc.c:1152-1153`).

| GL | pipe | PBE packmode | TPU combined | depth aspect | stencil aspect |
|---|---|---|---|---|---|
| DEPTH_COMPONENT16 | Z16_UNORM | U16 :260 | U16 :152 | U16 | INVALID |
| DEPTH_COMPONENT24 | Z24X8_UNORM | X8U24 :261 | X8U24 :153 | X8U24 | INVALID |
| DEPTH_COMPONENT32F | Z32_FLOAT | F32 :262 | F32 :154 | F32 | INVALID |
| STENCIL_INDEX8 | S8_UINT | U8 :263 | U8 :155 | INVALID | U8 |
| DEPTH24_STENCIL8 | Z24_UNORM_S8_UINT | ST8U24 :264 | ST8U24 :156 | X8U24 | U8X24 |
| DEPTH32F_STENCIL8 | Z32_FLOAT_S8X24_UINT | X24U8F32 :265 | X24U8F32 :157 | X24X8F32 | X24G8X32 |

Sampled as textures they remap to colour formats with the value in X
(`image.c:173-183`, `formats.c:352-368`): S8→R8_UINT, Z16→R16_UINT,
Z32F→R32_FLOAT. yakogl today stores every depth texture as R32F
(`src/gl/texture.rs:4368-4372`), which is the same look-up.

## 4. yakogl today, and what each part must gain

### 4.1 `src/backend.rs` — the format layer

- `GLES_TEXFMT_*` (`:84-101`): RGBA8, ETC2×3, RGB565, RGBA16F, R16F, R32F,
  L8, A8, LA8. `gles_texfmt_bytes` (`:104-114`) sizes them.
- `GLES_SURF_*` (`:120-125`): RGBA8888, XRGB8888, RGB565, RGBA16F, R16F,
  R32F. `gles_surf_bytes_per_pixel` (`:127-135`), `gles_surf_rt_class`
  (`:138-145`: 0 = RGBA8 class, 1 = RGBA16F, 2 = R16F, 3 = R32F),
  `gles_surf_texfmt` (`:148-156`).

Must gain: integer texfmts (at least the RGBA8I/UI, RGBA16I/UI,
RGBA32I/UI, R8/RG8 integer and RGB10_A2UI families — the narrow R/RGB
families can reuse the RGBA integer store with the swizzle doing the
component fill, the way `TEXF_R32F` reuses F32 today) and integer surf
formats with a new `gles_surf_rt_class` value (the integer class), so the
shader compiler, the PBE words and the clear path can key off it.
`gles_surf_bytes_per_pixel` and `gles_texfmt_bytes` gain the 1/2/4-byte
integer sizes. The §2.1 rows also need R16/R16_SNORM/RG16/RG16_SNORM
texfmts (the TPU has U16/S16/U16U16/S16S16 formats,
`arch-formats.c:120-126`) and an RGB5 texfmt (the `A1R5G5B5` row, alpha
forced to 1 in the swizzle).

### 4.2 `src/pvr/backend.rs` — texture state

- `TexFormat` (`:843-849`: texformat, compressed, swiz, textype) and the
  `TEXF_*` constants (`:865-905`), e.g. `TEXF_R32F` = F32 with swizzle
  `R,ZERO,ZERO,ONE` (`:875`) — the same shape Mesa gives R32F
  (`arch-formats.c:140` + swizzle X,0,0,1).
- `tex_image_word0` (`:909-927`), `tex_stride_image_word1` (`:932-945`),
  `tex_twiddled_image_word1` (`:952-976`) pack the TEXSTATE words.
- The sampler descriptor's word 7 is the PCK_INFO meta (`:3625-3675`):
  `TEX_PCK_INFO_RGBA8_UNORM` = 0x140 (`:1007`), `TEX_PCK_INFO_F16` =
  14|SPLIT (`:881`), `TEX_PCK_INFO_F32` = 15 (`:882`),
  `TEX_PCK_INFO_COMPRESSED_UNORM` = 0x11f (`:900`).

Must gain: a `TEXF_*` entry per integer texfmt with the texformat numbers
of §2.3 (`texstate.xml:95-193`) and Mesa's swizzles; PCK_INFO for them is
INVALID (`pco.c:387-463` gives integer formats no PCK) — word 7 takes
`PVR_PCK_FORMAT_INVALID` (0b11111, `iface.h:132`), which the USC already
tolerates (it only reads PCK bits for shadow clamps and image stores,
`nir-tex.c:772-777`, `:936-1060`).

### 4.3 `src/pvr/backend.rs` — PBE words

- `pbe_swizzle` (`:706-726`): XRGB8888/RGB565 → chan2,1,0,3; `alpha_one` →
  swiz[3]=ONE.
- `pbe_reg_words` (`:737-770`): tilerelative, downscale,
  linestride=(pitch−1)/2, memlayout, swizzle, maxclip, twiddle sizes.
- `pbe_emit_words` (`:792-821`): packmode RGBA16F→F16F16F16F16,
  R16F→F16, other float→F32, RGB565→R5G6B5, else U8U8U8U8;
  `source_format` = F16_PER_CHANNEL for the float classes else
  8_PER_CHANNEL; `norm = !float`; `source_pos`; `mrt_index & 7`.
- `pbe_emit_words_z_only` (`:783-790`): the emptytile/depth-only store.

Must gain (all per §1.2): the integer packmodes (U8/S8/U8U8/S8S8/
U8U8U8/U8U8U8U8/S8S8S8S8/U16…/U32U32U32U32/S32…/U32 for RGB10_A2UI —
`pbestate.xml:60-123`); `norm` must become "fully normalized" (Mesa's
`formats.h:264`), not `!float` — today's rule would wrongly set norm=1 for
integer targets; `source_format` 8_PER_CHANNEL for integers
(`job-common.c:51-53`); the RGB10_A2UI clear/accum dword packing
(`formats.c:103-121`). The swizzle for integer formats is Mesa's identity
or X,0,0,1 form — `pbe_swizzle` gains the per-format table.

### 4.4 `src/shaderc` — fragment output

- `SHC_RT_RGBA8/F16X4/F16/F32` (`src/shaderc/mod.rs:100-110`, blend-key
  bits at [30:32]) select the output tail in
  `src/shaderc/online/powervr/assemble.rs`: packs = F16X4→2, F32→0, else 1
  (`:1388-1392`); PCK u8888 for RGBA8, f16f16 rpt2/rpt1 for the half
  classes (`:1397-1400`); PIXOUT writes — F32 raw, the rest packed
  (`:1417-1422`, `build_pixout_write_to` `:1286-1291`, PIXOUT0=32
  `:1146`, `build_pck` `:1165-1168`).

Must gain: an integer RT class. Its tail follows Mesa's `pack_to_format`
(`pvfio.c:149-218`): 32-bit integer → no pack, 4 raw dwords to PIXOUT
(exactly today's F32 shape); 16-bit → two channels per dword; 8-bit → four
channels per dword — with **bitfield inserts**, not PCK (Mesa deliberately
uses `nir_bitfield_insert_imm` for pure integer, `pvfio.c:184-198`, so no
rounding or clamping happens); RGB10_A2UI → one dword, 10/10/10/2 inserts.
The ISA already has the PCK format numbers (`pco_isa.py:980-1003`) but the
integer path should not use them.

### 4.5 `src/shaderc` — sampling

- `build_smp` (`src/shaderc/online/powervr/fragment.rs:248-315`) hardcodes
  `("fcnorm", 1)` in the brief (`:285`) and exta (`:297`) forms. The ISA
  table has the `integer` field on `smp_extabc`
  (`online/powervr/isa_tables.rs:437`) and
  `fcnorm` on the itrsmp variants (`:535-537`).
- `select.rs` rejects every integer-result fetch today: the catch-all at
  `:1164-1166` ("…an integer result…is not selected yet").
- The front end already has ivec/uvec and isampler/usampler types
  (`src/shaderc/online/ast.rs:68-137`, keywords in `lexer.rs`).

Must gain: `fcnorm` from the destination type (Mesa's rule,
`nir-tex.c:236`), the `integer` modifier for integer fetches
(`nir-tex.c:239`), and selection of `Op::Tex` with an integer result.
BXS-4-64 has `has_tpu_extended_integer_lookup` (`bxs.h:68`), but Mesa
still disables the extended integer paths under LOD
(`nir-tex.c:607-609`) — nearest filtering is the safe first target.

### 4.6 `src/gl` — entry points

- `sized_format` (`src/gl/texture.rs:4326-4375`): no integer internal
  formats, no SNORM, no RGB10_A2 — they return None and glTexImage2D
  answers GL_INVALID_VALUE (`:1887-1888`). RGB565/RGB8/RGBA4/RGB5_A1 are
  stored as RGBA8 today; RGBA32F is stored as halves (`:4334`).
  `texfmt_of` (`:4378-4391`) maps the internal format to the texfmt.
- `renderbuffer_surf_format` (`src/gl/framebuffer.rs:513-518`):
  RGBA16F/RGB16F/RG16F→RGBA16F, R16F→R16F, R32F→R32F, else RGBA8888;
  `is_renderable_format` / `renderbuffer_storage_args_ok` refuse integer
  internal formats today — this is the 1,483-case integer-render-target
  bucket of `tools/cts/RESULTS.md` §1r.
- `glClearBufferiv` / `glClearBufferuiv` (`src/gl/es3.rs:1096-1126`,
  `:1129-1146`) cast the GLint/GLuint value to GLfloat and call
  `glClearBufferfv` (`:1114-1122`, `:1138-1145`) — the clear colour goes
  through the float path.

Must gain: the integer internal formats in `sized_format` (→ new texfmts)
and in `renderbuffer_surf_format` (→ the integer surf class); a raw-integer
clear path that keeps the app's ints as ints (Mesa packs them per accum
format, `formats.c:145-218`), reached from `glClearBufferiv/uiv` and from
`glClearColor`-style clears on integer targets. The same argument checks
must accept R16/R16_SNORM/RG16/RG16_SNORM/RGB5 (→ the new texfmts of
§4.1).

## 5. Integer fragment outputs and raw clears through the programs

The end-of-tile program needs no new shape: Mesa's EOT (`usc.c:86-181`)
carries only the PBE state words and emitpix; yakogl's EOT already moves
packed dwords from the tile buffer or the USC registers to PIXOUT
(`eot_doutu_src0_n[]` / `eot_doutu_src0_tb[]`, `src/pvr/backend.rs:5932-5997`,
EOT data `:6019`, kick `:6147`). For an integer target the fragment
program's tail (§4.4) leaves 1/2/4 packed dwords in the USC output
registers, and the PBE words (§4.3) tell the PBE how to read them — the
same contract the F32 class uses today, with `norm=0` and the integer
packmode instead.

The clear path is where the new programs come in. Mesa's clear program
(`usc.c:1023-1281`) does not compute the colour: it reads the CPU-packed
clear dwords from the preamble (`:1122-1135`) and stores them raw into an
`ivec`/`uvec` output typed by the accum dword count (`:1050-1077`).
yakogl's clear programs are GLSL (`tools/shaders/internal/es/clear.frag`:
`gl_FragColor = u_color`, mediump float) with the colour passed as
`clear_uniform` → `pack_rgba8` or `pack_clear_float`
(`src/pvr/backend.rs:2581-2620`), one dword (`clear_fs`), two for RGBA16F
(`clear_fs2`) or three attachments (`clear_fs3`) — objects at
`:1191-1223`, colour buffer `bg_color` (6 dwords, `CLEAR_ATTACH_DWORDS`
`:823`). For integer targets:

- the colour must be packed per accum format (one byte per channel for
  8-bit accum, one 16-bit int per channel for 16-bit, one dword per
  channel for 32-bit, one packed dword for RGB10_A2UI), mirroring
  `pvr_get_hw_clear_color` (`formats.c:145-218`);
- the program must output the value without float→byte conversion. A float
  program cannot carry integer bits faithfully (GLSL has no bitcast), so
  the clear programs need integer variants — see §6;
- masked clears (`clear_mask_fs`, `src/pvr/backend.rs:1223`) and the
  fbload quad (`FbLoad`, `:1408-1422`) follow the same split: a load of an
  integer target must fetch the texel as integer (no `fcnorm`, §4.5) and
  write it back packed as integer (Mesa's load outputs `ivec4`/`uvec4`,
  `usc.c:1082-1095`).

## 6. `src/pvr/internal.rs`

`src/pvr/internal.rs` is generated (`:3-4` header: `yakogl-precompile
--internal`, reproduced byte-for-byte by `tools/check.sh`) and holds the
fixed programs: `quad_vs`, `fbload` + `fbload_f16x4/f16/f32`,
`clear_m1..clear_me` + their `_f16x4` variants, `clear_attach1/2/3`
(`:14-50`).

It must change. The float clear programs and the three fbload variants
cover today's four RT classes; integer targets need:

- clear programs with integer outputs (the Mesa loadop shape: raw preamble
  dwords stored to ivec/uvec outputs), as new `replace:` variants of
  `clear.frag` (or a new source shader) per integer width — at minimum an
  8-bit and a 32-bit family, plus the `_f16x4`-style masked variants if
  masked integer clears are kept;
- fbload variants for integer targets (integer fetch + integer store).

The regeneration path is unchanged: add the variants to the precompile
source list, run `yakogl-precompile --internal`, and `tools/check.sh`
verifies the file. No hand-editing of `internal.rs`.

## 7. The ES 2.0 boundary

`REPORT_ES2` (`src/lib.rs:65-85`; CMake `-DYAKOGL_REPORT_ES2=ON`, Kconfig,
`make REPORT_ES2=1`) stays the default; ES 3.0 behaviour is behind
`!REPORT_ES2`, the pattern already used for the sRGB/float sized names
(`src/gl/texture.rs:1878-1883`) and the framebuffer checks
(`src/gl/framebuffer.rs:496`, `:1663`, `:1975`, …).

- Sized integer internal formats (table 3.13) are accepted only when
  `!REPORT_ES2`, in the same argument checks that gate the float names
  today. `es2_integer_format` (`src/gl/texture.rs:1738-1740`) keeps
  rejecting the `*_INTEGER` base formats in an ES 2.0 build.
- Integer renderbuffers stay refused in an ES 2.0 build
  (`renderbuffer_storage_args_ok` / `is_renderable_format` unchanged when
  `REPORT_ES2`).
- `glClearBufferiv/uiv` keep the float cast when `REPORT_ES2` (an ES 2.0
  build has no integer target to clear); the raw path is the `!REPORT_ES2`
  branch.
- The new RT class is never selected in an ES 2.0 build (no integer
  surfaces exist), so the blend-key space (`SHC_BLEND_KEY_RT_SHIFT`,
  `src/shaderc/mod.rs:103`) and every ES 2.0 program are untouched.
- The PBE/TEXSTATE words of every format ES 2.0 already uses (RGBA8,
  RGB565, the float classes, ETC2, L8/A8/LA8) are unchanged: the tables in
  §2 give the same rows Mesa gives them today.

## 8. Host tests

Existing tests that check PBE and texture-state words per format without a
board:

- `src/pvr/backend_csbgen_tests.rs`: hand-packed references
  (`old_build_tex_desc` `:16`, `old_tex_desc_sampler` `:182`, `old_emit`
  `:219`, `old_pbe_swizzle` `:245`, `old_pbe_reg` `:255`) compared against
  the csbgen packers (`csbgen_fragment_registers_equal_the_hand_packed_ones`
  `:306`, `csbgen_texture_descriptors_equal_the_hand_packed_ones` `:422`).
- `src/pvr/backend_tests.rs`: the C-backend Rig inspects
  `k.frag.pbe_word` (`:1524`), one PBE state + EOT per attachment
  (`three_colour_attachments_get_a_pbe_state_and_an_eot_program_each`
  `:2752-2768`), the twiddle probe (`:3884-3903`), the view window's
  linestride (`:3970-3994`), z-only/emptytile (`:4045-4065`).
- Inline tests in `src/pvr/backend.rs` check the sampler descriptor per
  format: `float_texture_descriptors_use_the_float_formats_and_pack_info`
  (`:7046-7075`, word 0 FORMAT field + word 7 PCK),
  `a_luminance_depth_texture_swizzles_its_red_into_green_and_blue`
  (`:7080-7100`), `narrow_luminance_alpha_descriptors_take_u8_formats_…`
  (`:7102-7128`).
- `tests/internal_programs.rs`: the internal programs against Mesa's
  reference programs (`tests/reference/` blobs).

Tests to add with `int-rt-pvr` / `formats-pvr` (same patterns, no board):

1. a per-format PBE-word table test: for each integer packmode (§2.3)
   and the new 16-bit UNORM/SNORM packmodes (§2.1), `pbe_emit_words`
   equals the csbgen-packed STATE_WORD1 with the table's packmode, norm
   and source_format (the `backend_csbgen_tests.rs` pattern);
2. a per-format sampler-descriptor test: integer `TEXF_*` word 0 (FORMAT
   field from `texstate.xml:95-193`), swizzle, and word 7 = PCK INVALID
   (the `float_texture_descriptors_…` pattern);
3. a clear-encoding test: `clear_uniform` (or its integer successor) for
   each accum width — 1/2/4 dwords, the RGB10_A2UI packed dword
   (`formats.c:103-121`);
4. an online-shaderc tail test: the integer RT class emits bitfield
   inserts (not PCK) and the right PIXOUT writes, mirroring the existing
   pack/pixout expectations (`assemble.rs:1385-1425`);
5. `tests/internal_programs.rs` entries for the new clear/fbload variants
   once §6 lands.

## 9. Decisions this doc makes

- Follow Mesa's `pack_to_format` exactly for integer outputs: bitfield
  inserts, never PCK (`pvfio.c:184-198`); 32-bit integer formats need no
  packing at all (`pvfio.c:180-182`).
- `norm` is "fully normalized" (`formats.h:264`), not `!float` — this also
  fixes nothing today (RGBA8/RGB565 are normalized, the float classes are
  not) but is the rule the integer formats need.
- Integer textures get their own texfmts/surf class rather than riding the
  RGBA8 store: the CTS cases check sampled values, and the swizzle can fix
  component fill but not the 8/16/32-bit widths.
- RGB8/RGB16F storage follows Mesa's widening (no 24-bit pipe formats):
  RGBA8 / RGBA16F rows of §2.
- RGB5 follows Mesa's fallback: the `B5G5R5A1_UNORM` row (the PowerVR
  tables have no row for Mesa's preferred `B5G5R5X1_UNORM`,
  `st_format.c:307-311`), with Mesa's `.xyz1` view swizzle giving A=1
  (`teximage.c:759`).
- RGB9_E5 stays texture-only (no accum format, `arch-formats.c:259`).
- Depth/stencil keep today's R32F texture storage and the ISP's own depth
  path; only the table rows are recorded here (§3).
- First integer sampling target: nearest filtering (Mesa disables the
  extended integer lookup under LOD, `nir-tex.c:607-609`); linear filtering
  of integer formats is out of the question per §1.6 anyway.
