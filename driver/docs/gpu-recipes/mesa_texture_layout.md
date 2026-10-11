<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR BXS-4-64 texture-sampling state layouts (from Mesa PVR + DDK)

> **Verified on silicon (2026-09-14):** mip chains work on the STRIDE (linear) layout
> with the levels consecutive in memory, each tightly packed at its own width (Mesa's
> `pvr_image_plane_setup_mip_levels`): `num_mip_levels` = levels, `mipmaps_present` = 1,
> sampler `maxlod` = CLAMP MAX, `minfilter`/`mipfilter` per the GL min filter. yakogl
> `gl_test` case 2 selects levels 0/1/2 by magnification and shows a glGenerateMipmap
> box-filtered level.

Target: BXS-4-64, BVNC **36.53.104.796**. Analysis of Mesa's open PVR Vulkan driver
(Mesa `src/imagination`) cross-checked against the TI DDK 25.2
(the `ti-img-rogue-driver` kernel tree). **Analysis only — nothing modified.**

## 0. CRITICAL: which arch/csbgen applies

Mesa classifies BXS-4-64 as the **ROGUE** arch, not "volcanic":

- `common/device_info/bxs-4-64.h:33` — `.public_name = "BXS-4-64", .arch = PVR_DEVICE_ARCH_ROGUE`
- `vulkan/meson.build:143` — the driver only builds `foreach arch : ['rogue']`, with
  `--device-prefix pvr_rogue` (meson.build:13).

Therefore the authoritative TPU state layout is **`csbgen/rogue/texstate.xml`**
(`<csbgen name="ROGUE" prefix="TEXSTATE">`), and the emit code
`vulkan/pvr_arch_tex_state.c` / `pvr_arch_sampler.c` is exactly the code that runs for
our part. **Do NOT use `csbgen/volcanic/texstate.xml`** — it is a different, newer part
family and its IMAGE_WORD0/SAMPLER_WORD0 bit assignments differ (no `smpcnt`, no `gamma`,
different `depth`/`stride` positions). The rest of this document uses the rogue layout.

Device features that gate the layout (from `bxs-4-64.h`):
- `has_tpu_image_state_v2 = true`  → csbgen condition **TPU_IMAGE_STATE_V2 = TRUE**
- no TFBC feature listed           → csbgen condition **TFBC = FALSE**
- `has_tpu_extended_integer_lookup = true` → the `index_lookup` bit is NOT used
- descriptor sizes: `ROGUE_NUM_TEXSTATE_IMAGE_WORDS = 2`,
  `ROGUE_NUM_TEXSTATE_SAMPLER_WORDS = 2`, `ROGUE_NUM_TEXSTATE_DWORDS = 4`
  (`include/hwdef/rogue_hw_defs.h:107-112`); `PVR_IMAGE_DESCRIPTOR_SIZE = 4` dwords,
  `PVR_SAMPLER_DESCRIPTOR_SIZE = 4` dwords (`vulkan/pvr_common.h:55-56`).

So: **image state = 2×u64 (16 B), sampler state = 2×u64 (16 B).**

csbgen bit convention: fields are `[start:end]` inside a little-endian 64-bit struct
(`length="2"` = 2 dwords). An `address` field with `shift=N` stores `addr >> N` placed at
`start` (alignment = 2^N); i.e. packed contribution = `((addr >> N) << start)`, masked to
the field. (`csbgen/gen_pack_header.py:933`, `Define ALIGNMENT = 2**shift`.)

---

## 1. Image / texture-state words (what the TPU SMP consumes)

Source of truth: `csbgen/rogue/texstate.xml`; emit logic `vulkan/pvr_arch_tex_state.c`.
For a **linear** 2D image the driver uses TEXTYPE **STRIDE** and packs
`IMAGE_WORD0` + `STRIDE_IMAGE_WORD1` (`pvr_arch_tex_state.c:139-146, 246-283`).

### IMAGE_WORD0 (u64, `texstate.xml:272-291`)
| field | bits | meaning | value for 64×64 linear RGBA8888 |
|---|---|---|---|
| textype | 0:2 | TEXTYPE | **STRIDE = 4** (linear 2D → STRIDE; `pvr_arch_tex_state.c:143`) |
| gamma | 3:3 | sRGB gamma (overlaps twocomp_gamma) | 0 |
| twocomp_gamma | 3:4 | 2-comp gamma | 0 |
| swiz3 | 5:7 | channel A source | **SRCCHAN_3 = 3** |
| swiz2 | 8:10 | channel B source | **SRCCHAN_2 = 2** |
| swiz1 | 11:13 | channel G source | **SRCCHAN_1 = 1** |
| swiz0 | 14:16 | channel R source | **SRCCHAN_0 = 0** |
| minlod | 17:26 | min LOD (u4.6) | 0 |
| texformat | 27:33 | FORMAT enum | **U8U8U8U8 = 12** |
| width | 34:47 | width − 1 | **63** |
| height | 48:61 | height − 1 | **63** |
| smpcnt | 62:63 | log2(sample_count) | 0 |

- `R8G8B8A8_UNORM → texformat U8U8U8U8 (12)` : `pvr_arch_formats.c:101`.
- swizzle: identity view → SRCCHAN_0..3 (`pvr_get_hw_swizzle`, `pvr_arch_tex_state.c:41-64`).
  U8U8U8U8 reads byte0→chan0, byte1→chan1, ...; a memory buffer laid out R,G,B,A ascending
  (a little-endian `0xAABBGGRR` u32) with identity swizzle yields result.rgba = R,G,B,A.
- `width/height = extent − 1` (`pvr_arch_tex_state.c:277-281`).

**IMAGE_WORD0 = `0x003F00FC_60000A64`** → dwords `[0x60000A64, 0x003F00FC]`.
Scaling: width bumps bits 34:47 (=W−1), height bits 48:61 (=H−1); nothing else changes.

### STRIDE_IMAGE_WORD1 (u64, linear path; `texstate.xml:315-329`) — with V2=on, TFBC=off
| field | bits | meaning | value |
|---|---|---|---|
| stride | 0:14 | row stride in **texels** − 1 | **63** (= row_pitch_texels − 1) |
| mipmaps_present | 15:15 | bool | 0 |
| texaddr | 16:53 (shift 2) | base dev addr, 4-byte aligned; stores addr>>2 | **BASE>>2** |
| tpu_image_state_v2_compression_mode | 54:56 | COMPRESSION_MODE | **NONE = 0** (uncompressed) |
| alpha_msb | 59:59 | alpha is MSB of texel | **1** for R8G8B8A8 |
| num_mip_levels | 60:63 | levels HW may access | **1** |

- `stride = info->stride − 1`, `info->stride = physical row width in texels`
  (`pvr_arch_tex_state.c:248-249`, `pvr_arch_image.c:117`). For a tightly packed 64-wide
  RGBA8 buffer, row pitch = 64 texels = 256 bytes → stride field = 63.
- Note the V2/TFBC=off branch means: `lossy_compression_mode` and the else-branch bits
  (`tile_size`, `index_lookup`, `frame_buffer_compression*`) are **absent** — leave 0.
- `alpha_msb`: `vk_format_is_alpha_on_msb(R8G8B8A8_UNORM)` = true (`pvr_arch_tex_state.c:256-257`).
- `num_mip_levels = info->mip_levels` = 1 for single level (`pvr_arch_tex_state.c:250`).
- address encoding: `texaddr = PVR_DEV_ADDR_OFFSET(addr, offset)`; packed as
  `((BASE>>2) << 16)`. BASE must be ≥4-byte aligned (recommend ≥64-byte / SLC-line).

**Example** BASE = `0x9000_0000`:
STRIDE_IMAGE_WORD1 = `0x1800_0000_0000_003F | ((0x9000_0000>>2)<<16)`
= `0x1800_2400_0000_003F` → dwords `[0x0000003F, 0x18002400]`.
(Constant part `0x1800_0000_0000_003F` = num_mip_levels=1 [bit60] + alpha_msb [bit59] + stride 63.)

**Image descriptor (4 dwords, the DMA payload for the image):**
`[0x60000A64, 0x003F00FC, 0x0000003F, 0x18002400]` (last two vary with BASE/stride).

> Twiddled (non-linear) would instead use `IMAGE_WORD1` (`texstate.xml:292-314`) with
> `depth`, `baselevel`, `texaddr` at 16:53 — not needed for our linear source.

---

## 2. Sampler-state words

Source: `csbgen/rogue/texstate.xml:342-366`; emit `vulkan/pvr_arch_sampler.c`.
Sampler = `SAMPLER_WORD0` + `SAMPLER_WORD1` (2×u64 = 4 dwords).

### SAMPLER_WORD0 (u64)
| field | bits | meaning | nearest+clamp value |
|---|---|---|---|
| dadjust | 0:12 | LOD bias, DADJUST fixed | **ZERO_UINT = 4095 (0xFFF)** (bias 0) |
| minlod | 13:22 | CLAMP u4.6 | 0 |
| maxlod | 23:32 | CLAMP u4.6 | 0 |
| anisoctl | 33:35 | ANISOCTL | **DISABLED = 0** |
| magfilter | 36:37 | FILTER | **POINT = 0** |
| minfilter | 38:39 | FILTER | **POINT = 0** |
| mipfilter | 40:40 | bool (mip LINEAR) | 0 |
| addrmode_u | 41:43 | ADDRMODE | **CLAMP_TO_EDGE = 2** |
| addrmode_v | 44:46 | ADDRMODE | **CLAMP_TO_EDGE = 2** |
| lumakey | 47:47 | bool | 0 |
| lumakey_alphamult | 48:48 | bool | 0 |
| non_normalized_coords | 49:49 | bool | **0** (normalized 0..1 UV) |
| bordercolor_index | 50:55 | u6 (overlaps texaddr_plane2_lo) | 0 |
| addrmode_w | 56:58 | ADDRMODE | **CLAMP_TO_EDGE = 2** |
| cmp_mode | 59:61 | CMP_MODE | **NEVER = 0** |

Enums (`texstate.xml`): FILTER POINT=0/LINEAR=1; ADDRMODE REPEAT=0/FLIP=1/CLAMP_TO_EDGE=2/
CLAMP_TO_BORDER=4; ANISOCTL DISABLED=0; DADJUST ZERO_UINT=4095, FRACTIONAL_BITS=8;
CLAMP INTEGER_BITS=4, FRACTIONAL_BITS=6 (so LOD units are u4.6).
Emit: filters `pvr_arch_sampler.c:24-35,131-133`; addr modes `:37-53,140-147`;
`dadjust = ZERO_UINT + fixed(bias)` `:160-166`; aniso `:171-181`; lod clamps `:196-205`.

For **LINEAR** filtering instead: set magfilter=minfilter=**1**.

**SAMPLER_WORD0 = `0x0200_2400_0000_0FFF`** → dwords `[0x00000FFF, 0x02002400]`.

### SAMPLER_WORD1 (u64, `texstate.xml` SAMPLER_WORD1)
| field | bits | meaning | value (non-YUV, non-cube) |
|---|---|---|---|
| cemedge_dontfilter | 63:63 | non-seamless cube | 0 |
| texaddr_plane3 | 24:61 (shift 2) | YUV plane 3 addr | 0 |
| texaddr_plane2_hi | 0:23 (shift 16) | YUV plane 2 addr hi | 0 |

`pvr_arch_sampler.c:207-210` sets only `cemedge_dontfilter`. For plain RGBA → all 0.

**Sampler descriptor (4 dwords):** `[0x00000FFF, 0x02002400, 0x00000000, 0x00000000]`.

---

## 3. How the shader references image + sampler state (binding)

Mechanism (graphics fragment path; concrete analog is the transfer/blit path
`vulkan/pvr_arch_job_transfer.c:2944-2995`):

1. Image (4 dw) + sampler (4 dw) [+ optional USC const floats] are written contiguously
   into a small BO in memory (`pvr_sampler_image_state()` → `dma_space`).
2. A **PDS "texture-state" (secondary-attribute / uniform_texture) program** DMAs that block
   into the USC **Common Store** (shared registers) via one `DOUTD` (see §4). The block lands
   at common-store dword offset = `state->common_ptr` (the DMA `dest_offset`).
3. The USC `SMP` instruction reads image state from the shared reg where the image words were
   placed, and sampler state from where the sampler words were placed (see §5). The SMP's
   `tex_state`/`smp_state` operands are shared-register locations, NOT a table index — you
   choose the common-store offset and make the SMP operands match.

**PPP TA_STATE fields (`csbgen/rogue/ppp.xml:283-325`; set exactly as
`pvr_arch_cmd_buffer.c:3654-3691`):**

| PPP word | field | bits | set to |
|---|---|---|---|
| STATE_PDS_TEXUNICODEBASE | addr | 4:31 (shift 4) | **CODE** seg addr of the PDS texture-state program (16-B aligned) |
| STATE_PDS_TEXTUREDATABASE | addr | 4:31 (shift 4) | **DATA** seg addr of the PDS texture-state program (16-B aligned) |
| STATE_PDS_SIZEINFO1 | pds_texturestatesize | 16:22 | `DIV_ROUND_UP(pds_texstate_prog.data_size_dw, 4)` (UNIT_SIZE=4) |
| STATE_PDS_SIZEINFO1 | pds_tempsize | 0:3 | `DIV_ROUND_UP(temps_used, 4)` (min 1) |
| STATE_PDS_SIZEINFO2 | usc_sharedsize | 23:31 | `DIV_ROUND_UP(total_dw_DMAed_to_common_store, 16)` (UNIT_SIZE=16) |

UNIT_SIZEs are in `ppp.xml` (pds_texturestatesize UNIT_SIZE=4, usc_sharedsize UNIT_SIZE=16,
pds_tempsize UNIT_SIZE=4). `usc_sharedsize` = number of common-store dwords the DMA fills
(= 8 for 1 image + 1 sampler with no extra floats) → value `DIV_ROUND_UP(8,16) = 1`.
`pds_texturestatesize` = the texture-state PDS program's DATA size (3 dw for 1 kick) →
`DIV_ROUND_UP(3,4) = 1`.

`STATE_PDS_SHADERBASE` (your existing frag PDS-kick program) is separate and unchanged; the
DOUTU that kicks the fragment USC lives there, not in the texture-state program.

---

## 4. The PDS texture-state program (DOUTD DMA)

Generator: `pvr_pds_pixel_shader_uniform_texture_{code,data}` (`vulkan/pds/pvr_pds.c:2814,3184`;
public wrappers `pvr_pds.h:697-717`). DMA control built by
`pvr_pds_encode_dma_burst` (`pvr_pds.c:446-471`).

**DMA control word** (`texture_dma_control[0]`), fields in `pvr_rogue_pds_defs.h`:
```
control = (dma_size_dw << 0)                 // BSIZE  [0:11], BSIZE_SHIFT=0
        | (dest_offset  << 13)               // AO     [13:25], AO_SHIFT=13  (common-store dword offset)
        | 0x00000000                         // CMODE_CACHED
        | 0x10000000                         // DEST_COMMON_STORE (bit 28)
        | 0x80000000;                        // LAST_EN (bit 31)  (last kick)
```
For `dma_size_dw = 8`, `dest_offset = 0`, last: **control = `0x90000008`**.
`dma_address[0] = src_BO_dev_addr` (optionally `| DOUTD_SRC0_SLCMODE_CACHED` if the part has
`slc_mcu_cache_controls`).

**DOUT instruction encoding** (`pvr_rogue_pds_encode.h:514-536`, defs `pvr_rogue_pds_defs.h`):
```
inst = (0xF << 28)          // OPCODEC_DOUT
     | (cc  << 27)
     | (end << 26)
     | ((src1 & 0xFF) << 16)   // SRC1 = 32-bit constant index (control)
     | ((src0 & 0x7F) << 8)    // SRC0 = 64-bit constant index >> 1 (address)
     | (dst  << 0);            // DSTDOUT: DOUTD=0, DOUTW=1, DOUTU=2
HALT = (0xD << 28) | (6 << 23);   // OPCODEC_SP | OPCODESP_HALT  = 0xD3000000
```

**Data-segment layout for 1 texture kick** (`pvr_pds.c:3499-3539`,
`PVR_PDS_CONSTANTS_BLOCK_BASE = 0`): 64-bit address first, then 32-bit control:
```
constant[0..1] = src_BO_dev_addr   (64-bit)
constant[2]    = dma_control       (0x90000008)
```
DOUTD operands: `src0 = addr_const64 >> 1 = 0`, `src1 = control_const32 = 2`.

**Minimal hand-emitted PDS texture-state program (1 texture + 1 sampler):**
```
CODE  (2 dwords, 16-B aligned):
  [0] DOUTD cc=0 end=1 src1=2 src0=0  = 0xF4020000
  [1] HALT                            = 0xD3000000
DATA  (3 dwords, 16-B aligned):
  [0] src_BO_dev_addr & 0xFFFFFFFF
  [1] src_BO_dev_addr >> 32
  [2] 0x90000008                       // size=8 dw, dest=common[0], DEST_COMMON_STORE|LAST
reported: data_size = 3 dw, code_size = 2 dw, temps_used = 1
```
Point `TEXUNICODEBASE` → CODE addr, `TEXTUREDATABASE` → DATA addr,
`SIZEINFO1.pds_texturestatesize = 1`, `SIZEINFO2.usc_sharedsize = 1`,
`SIZEINFO1.pds_tempsize = 1`. `kick_usc=false` (USC kicked by the main frag PDS program).

---

## 5. The SMP fragment shader (USC)

Mesa builds the sampling FS with the PCO compiler; the SMP is emitted by
`pco_emit_nir_smp()` (`pco/pco_nir_tex.c`) as a NIR intrinsic, then encoded to the Rogue USC
ISA by `pco/pco_isa.py` (`I_SMP_BRIEF/EXTA/EXTB/EXTAB/EXTABC`, backend/DMA class,
`dma_op = smp`, `pco_isa.py:2499-2633`). **There is no static blit-shader dword array in the
tree** (the transfer path compiles USC at runtime), so hand-assembly uses the ISA + flags.

SMP operand model (`pco_smp_params`, `pco.h:67-99`; emit `pco_nir_tex.c:pco_emit_nir_smp`):
- `tex_state`  = common-store location of the 2 image words (§1) — place at common[0].
- `smp_state`  = common-store location of the 2 sampler words (§2) — place at common[2]
  (image occupies common dwords 0..3, sampler 4..7; SMP addresses image/sampler state in u64
  units, so tex_state = shared-reg pair 0, smp_state = shared-reg pair 2).
- `coords`     = the 2 UV f32 (feed the existing vec4 varying `.xy`; no VS change needed).
- `dest_type` = float (UNORM), 4 components RGBA → result written to the pixel-output regs
  (same path your interpolation FS uses for its final `pck`/output).

SMP flags to set (`pco_smp_flags`, `pco_nir_tex.c` + `pco_isa.py`):
```
dim      = PCO_DIM_2D          // 2D
fcnorm   = 1                   // float, normalized result (UNORM→float)
nncoords = 0                   // normalized (0..1) coords  (match sampler bit 49 = 0)
lod_mode = PCO_LOD_MODE_REPLACE, lod_replace = 0   // 1 extra data word, avoids ddx/ddy
proj/array/tao/soo/sno/wrt = 0
drc      = a dependent-read-counter slot
```
Coefficients iterated: just the 2 UV f32 varyings (num_components = 2). If you reuse the
existing vec4 varying, iterate `.xy`. `lod_mode=REPLACE lod=0` is the cheapest (no gradients,
no derivatives); alternatively `lod_mode=NORMAL` needs ddx/ddy of the coords.

Result: one `SMP` issues a TPU sample; the 4 returned channels are the RGBA texels which you
then pack/emit to the output register exactly like the interpolated color in your working FS.

Practical recommendation: rather than hand-encoding the multi-word USC SMP, the lowest-risk
path is to append a single SMP backend instruction to your existing interpolation FS with the
flags above, or compile a 3-line `texture(sampler2D, uv)` shader through PCO and lift its
`SMP` dword(s). The exact bit positions of every SMP field are in `pco/pco_isa.py` lines
2499-2633 (base `I_SMP_BRIEF` + extension words `EXTA/EXTB/EXTAB/EXTABC`) and the shared
`I_BACKEND` bit-set for source-register encoding.

---

## Concrete build recipe (1 linear RGBA8888 texture on a quad)

Assume source buffer at `BASE` (≥4-byte, ideally ≥64-byte aligned), 64×64, row pitch 64
texels (256 B), memory order R,G,B,A ascending (little-endian `0xAABBGGRR`).

**A. Image state descriptor — 4 dwords (place at common-store dwords 0..3):**
```
IMAGE_WORD0        = 0x003F00FC_60000A64   → dw: 0x60000A64, 0x003F00FC
STRIDE_IMAGE_WORD1 = 0x18000000_0000003F | ((BASE>>2)<<16)
   e.g. BASE=0x90000000 → 0x18002400_0000003F → dw: 0x0000003F, 0x18002400
```
Scaling: WIDTH→bits34:47 = W−1; HEIGHT→bits48:61 = H−1; stride(texels)−1→bits0:14 of WORD1.
For LINEAR filtering the image words are unchanged (filtering is in the sampler).

**B. Sampler state descriptor — 4 dwords (place at common-store dwords 4..7):**
```
SAMPLER_WORD0 = 0x02002400_00000FFF   → dw: 0x00000FFF, 0x02002400   (nearest, clamp-to-edge, bias 0)
SAMPLER_WORD1 = 0x00000000_00000000   → dw: 0x00000000, 0x00000000
```
For LINEAR filtering: set magfilter (bit36)=1 and minfilter (bit38)=1, i.e. OR in
`(1<<36)|(1<<38) = 0x00000050_00000000` → **SAMPLER_WORD0 = `0x02002450_00000FFF`**
(dw: `0x00000FFF, 0x02002450`).

**C. DMA source BO — 8 dwords, contiguous:**
```
[0..3] = image descriptor (A)
[4..7] = sampler descriptor (B)
```
(No extra USC-const floats needed for normalized UV nearest/linear.)

**D. PDS texture-state program (§4):**
```
CODE (2 dw): 0xF4020000, 0xD3000000
DATA (3 dw): (BASE_of_BO & 0xFFFFFFFF), (BASE_of_BO >> 32), 0x90000008
```

**E. Edits to your existing PPP TA_STATE (fragment):**
```
STATE_PDS_TEXUNICODEBASE.addr   = <dev addr of PDS CODE seg (D)>          // 16-B aligned
STATE_PDS_TEXTUREDATABASE.addr  = <dev addr of PDS DATA seg (D)>          // 16-B aligned
STATE_PDS_SIZEINFO1.pds_texturestatesize = 1   // DIV_ROUND_UP(3,4)
STATE_PDS_SIZEINFO1.pds_tempsize         = 1   // DIV_ROUND_UP(1,4)
STATE_PDS_SIZEINFO2.usc_sharedsize       = 1   // DIV_ROUND_UP(8,16)   (8 dwords in common store)
```
(SHADERBASE / your frag-kick PDS program and coeff/DOUTI program stay as-is; the DOUTU that
kicks the FS remains in the SHADERBASE program.)

**F. Fragment USC (§5):** append one `SMP` (dim=2D, fcnorm=1, nncoords=0,
lod_mode=REPLACE lod=0) with `tex_state`=common pair 0, `smp_state`=common pair 2, coords =
UV from the existing vec4 varying `.xy`; write the 4 RGBA results to the pixel output register
via the same pack/emit path as your current interpolation FS.

### File citations
- Arch decision: `common/device_info/bxs-4-64.h:33`, `vulkan/meson.build:13,143`
- Image/sampler bit layout: `csbgen/rogue/texstate.xml:272-366`
- Image emit + linear→STRIDE + format/swizzle/alpha_msb: `vulkan/pvr_arch_tex_state.c:126-283`
- RGBA8→U8U8U8U8: `vulkan/pvr_arch_formats.c:101`
- Sampler emit: `vulkan/pvr_arch_sampler.c:24-210`
- Descriptor sizes: `vulkan/pvr_common.h:55-56`, `include/hwdef/rogue_hw_defs.h:107-112`
- stride/mip population: `vulkan/pvr_arch_image.c:109-119`
- DMA + PDS texture-state program: `vulkan/pds/pvr_pds.c:446-471, 2814-3172, 3184-3540`;
  DOUT/HALT encode `vulkan/pds/pvr_rogue_pds_encode.h:445-536`; field defs
  `vulkan/pds/pvr_rogue_pds_defs.h:1145-1648`
- PPP TA_STATE fields + example: `csbgen/rogue/ppp.xml:283-329`,
  `vulkan/pvr_arch_cmd_buffer.c:3654-3691`
- Blit/transfer analog (image+sampler+float DMA into common store): `vulkan/pvr_arch_job_transfer.c:2944-2995`
- SMP shader emit + ISA: `pco/pco_nir_tex.c` (`pco_emit_nir_smp`), `pco/pco.h:67-100`,
  `pco/pco_isa.py:2499-2633`


## Compressed textures (ETC1 / ETC2), verified on silicon 2026-09-14

`gl_test.c` case 20: a 16x8 ETC1 texture of 4x2 flat blocks and an 8x8 ETC2 RGBA8
(EAC alpha) texture sample back exactly, every block in place. The TPU decodes them;
the state differs from the linear RGBA8 case in three ways (Mesa samples block-compressed
formats only with optimal = twiddled tiling, `pvr_formats.c` / `pvr_arch_tex_state.c`):

- IMAGE_WORD0: `textype = 2D` (1, not STRIDE), `texformat` = ETC2_RGB 68 / ETC2A_RGBA 69
  / ETC2_PUNCHTHROUGHA 70; RGB formats read alpha through `swiz3 = SRC_ONE` (4).
  ETC1 blocks are valid ETC2_RGB blocks (`GL_ETC1_RGB8_OES` uses format 68).
- IMAGE_WORD1 is the twiddled variant (no stride field): `texaddr` [16:53],
  `num_mip_levels` [60:63], `tpu_image_state_v2_compression_mode = TPU` (1 at [54:56]);
  no `alpha_msb`.
- Image meta PCK_INFO = `PVR_PCK_FORMAT_INVALID (0x1f) | UNORM (bit 8)` = 0x11f
  (`setup_pck_info`: pco has no pack format for compressed inputs).

Memory layout: the 4x4 blocks (8 bytes ETC2_RGB, 16 bytes ETC2A) of a power-of-two block
grid in the PowerVR twiddled order — Morton with the y bit in the lower position of each
pair and the longer dimension's spare bits above (`gl_texture.c` `twiddle_index`, the SDK's
TwiddleUV): for 4x2 blocks, index = (y & 1) | ((x & 1) << 1) | ((x >> 1) << 2). One level per
texture (`glCompressedTexImage2D` level > 0 is `GL_INVALID_OPERATION`).
