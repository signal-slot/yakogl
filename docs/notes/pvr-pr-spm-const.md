<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The PR background load: Mesa's SPM-load constants for 800x600 RGBA8

Host-only reading and arithmetic, 2026-10-30, nothing run on a board.
For `bgload-pds`: the exact dwords of the per-render constants buffer (sh0-7),
the PDS DOUTD data that copies it into the common store, and the DOUTU word
that kicks the load shader.

**Source:** Mesa 26.2.1 (tag `mesa-26.2.1`, commit `da14d65e4499`, tarball
sha256 `c47e81bd...a4442`, see `third_party/mesa-26.2.1/UPSTREAM`). The
vendored tree lacks the `vulkan/` sources this note cites, so paths are
relative to the tarball's `mesa-26.2.1/src/imagination/` and read from the
tarball itself. A reader with the tarball can check every line. Files:
- `SPM` = `vulkan/pvr_arch_spm.c`
- `IFACE` = `common/pvr_iface.h`
- `USCH` = `vulkan/pvr_usc.h`
- `TEXC` = `vulkan/pvr_arch_tex_state.c`
- `PDSC` = `vulkan/pds/pvr_pds.c`
- `PDSD` = `vulkan/pds/pvr_rogue_pds_defs.h`
- `TEXX` = `csbgen/rogue/texstate.xml`, `PDSX` = `csbgen/rogue/pds.xml`,
  `CRX` = `csbgen/rogue/cr.xml` (the three XML files are also vendored in
  `third_party/mesa-26.2.1`)

Marks: **read** = a source says it; **computed** = arithmetic on read fields
(the packing below was done by script, not on hardware).

## Our configuration

| Input | Value | Source |
|---|---|---|
| framebuffer | 800 x 600 | task |
| attachment | RGBA8, 1 output dword | `SPM:780-786` (`dword_count = MIN2(output_regs_count, 4)` when no tile buffers) |
| tile buffers | 0 | `SPM:783`, `SPM:790` |
| sample count | 1 (`is_multisampled` false) | `SPM:791`, `SPM:839` |
| load shader | `pvr_spm_load_props` {1, 0, false} | `SPM:788-794` |
| GPU | BXS-4-64 | `third_party/mesa-26.2.1/src/imagination/common/device_info/bxs-4-64.h` |

BXS-4-64 sets `has_tpu_image_state_v2` and `has_tpu_extended_integer_lookup`
(`bxs-4-64.h:68-69`) and does **not** set `slc_mcu_cache_controls`. The
`tpu_image_state_v2` flag does not change any word below for R32_UINT:
`TEXC:256-270` only sets `index_lookup` when both v2 features are absent, and
`TEXC:266-269` only touches compressed formats.

## 1. Constants buffer layout and size

| Item | Value | Source |
|---|---|---|
| sh0-3 `PVR_SPM_LOAD_DATA_SMP` | offset 0, 4 dwords | `IFACE:76`, `SPM:818-819` |
| sh4-7 `PVR_SPM_LOAD_DATA_REG_TEX` | offset 4, 4 dwords | `IFACE:78`, `SPM:841` |
| buffer size, 0 tile buffers | `BUF_TEX_0` = **8 dwords** (32 bytes) | `USCH:248-254`, `IFACE:80` |
| tile-buffer words | none (loop runs 0 times) | `SPM:809`, `SPM:850` |
| `usc_sharedsize` (our value) | ceil(8 / 16) = **1** | `CRX:333-335` (unit 16), computed |
| DOUTD `bsize` (our value) | **8** | `SPM:717`, computed |

**Mesa oddity (read):** Mesa passes the size in bytes: `consts_buffer_size =
PVR_DW_TO_BYTES(pvr_uscgen_spm_load_data_size(...))` = 32 (`SPM:796`) and hands
that to both `bsize` (`SPM:872-877` -> `:717`) and `usc_sharedsize`
(`SPM:908-910`, giving ceil(32/16) = 2). A transcription uses 8 dwords, so
bsize 8 and usc_sharedsize 1, as `docs/notes/pvr-render-pool.md` (part 4)
decides. Mesa's literal control word is shown below for comparison.

## 2. Sampler words, sh0-3

Two 64-bit words, written as `descriptor->words[0]` and `words[1]`
(`SPM:818-831`). A 64-bit word `W` occupies dwords {lo = `W & 0xffffffff`,
hi = `W >> 32`}.

`SAMPLER_WORD0` fields (bit ranges `TEXX:233-248`; enum values `TEXX:30-80`):

| Field | Set at | Value | Bits |
|---|---|---|---|
| `non_normalized_coords` | `SPM:821` | 1 | 49 |
| `addrmode_v` | `SPM:822` | `CLAMP_TO_EDGE` = 2 | 44-46 |
| `addrmode_u` | `SPM:823` | `CLAMP_TO_EDGE` = 2 | 41-43 |
| `minfilter` | `SPM:824` | `POINT` = 0 | 38-39 |
| `magfilter` | `SPM:825` | `POINT` = 0 | 36-37 |
| `maxlod` | `SPM:826` | `CLAMP_MIN` = 0 | 23-32 |
| `minlod` | `SPM:827` | `CLAMP_MIN` = 0 | 13-22 |
| `dadjust` | `SPM:828` | `ZERO_UINT` = 4095 (0xfff) | 0-12 |
| all others | not set | 0 | |

| Dword | Name | Value | Source |
|---|---|---|---|
| sh0 | `SAMPLER_WORD0` low | `0x00000fff` | `SPM:820-829`, computed |
| sh1 | `SAMPLER_WORD0` high | `0x00022400` | same; (1<<17)\|(2<<12)\|(2<<9) |
| sh2 | `SAMPLER_WORD1` low | `0x00000000` | `SPM:831` (no fields set) |
| sh3 | `SAMPLER_WORD1` high | `0x00000000` | `SPM:831` |

Check: `WORD0` = `0x0002240000000fff`.

## 3. Texture-state words, sh4-7

`pvr_spm_setup_texture_state_words` (`SPM:638-691`) fills a
`pvr_texture_state_info` and calls `pvr_arch_pack_tex_state`; the first
`sizeof(image_descriptor.words)` = 2 x 8 bytes are copied to sh4-7
(`SPM:684-686`; `ROGUE_NUM_TEXSTATE_IMAGE_WORDS` = 2, `include/hwdef/rogue_hw_defs.h:107`;
`pvr_common.h:208-211`). The `meta[]` array is not copied.

Inputs (all read):

| Input | Value | Source |
|---|---|---|
| format | `pvr_get_format_from_dword_count(1)` = `VK_FORMAT_R32_UINT` | `SPM:655`, `SPM:630` |
| layout | `PVR_MEMLAYOUT_LINEAR` | `SPM:656` |
| view type | `VK_IMAGE_VIEW_TYPE_2D` | `SPM:658` |
| state type | `PVR_TEXTURE_STATE_STORAGE` | `SPM:659` |
| extent | 800 x 600 | `SPM:660-663` |
| mip levels | 1 | `SPM:665` |
| sample count | 1 | `SPM:667` |
| stride | `ALIGN_POT(800, ROGUE_CR_PBE_WORD0_MRT0_LINESTRIDE_ALIGNMENT)` = `ALIGN_POT(800, 2)` = 800 | `SPM:647-649`, `SPM:668`; alignment 2 at `CRX:612-614` |
| base address | `S` = the SPM colour scratch buffer (placeholder) | `SPM:670`, `SPM:770`, `SPM:840` |
| swizzle | `{X, ZERO, ZERO, ONE}` | `SPM:677-678`; `src/util/format/u_format.yaml:2675-2680` (`swizzles: [X, 0, 0, 1]`); mapped by `TEXC:40-64` |
| texformat | `TEXSTATE_FORMAT_U32` = 24 (`R32_UINT` row) | `vulkan/pvr_arch_formats.c:138`, `:246`, `:312-327`; `TEXX:108` |

`IMAGE_WORD0` (`TEXC:139-234`; bit ranges `TEXX:210-226`):

| Field | Set at | Value | Bits |
|---|---|---|---|
| `textype` | `TEXC:140-145` (linear + 2D) | `STRIDE` = 4 | 0-2 |
| `swiz3` (A) | `TEXC:204-205` | `SRC_ONE` = 4 | 5-7 |
| `swiz2` (B) | `TEXC:202-203` | `SRC_ZERO` = 5 | 8-10 |
| `swiz1` (G) | `TEXC:200-201` | `SRC_ZERO` = 5 | 11-13 |
| `swiz0` (R) | `TEXC:198-199` | `SRCCHAN_0` = 0 | 14-16 |
| `texformat` | `TEXC:190-191` | `U32` = 24 | 27-33 |
| `gamma` | `TEXC:208` (not sRGB) | 0 | 3 |
| `width` | `TEXC:230` | 800 - 1 = 799 | 34-47 |
| `height` | `TEXC:231-233` | 600 - 1 = 599 | 48-61 |
| `smpcnt` | `TEXC:197` | log2(1) = 0 | 62-63 |
| `minlod` | not set | 0 | 17-26 |

`STRIDE_IMAGE_WORD1` (linear branch, `TEXC:244-270`; bit ranges `TEXX:230-246`):

| Field | Set at | Value | Bits |
|---|---|---|---|
| `stride` | `TEXC:247` | 800 - 1 = 799 | 0-14 |
| `mipmaps_present` | `TEXC:249` | 0 (`info->mipmaps_present` not set, `SPM:654-671`) | 15 |
| `texaddr` | `TEXC:251` | `S >> 2` (`shift="2"`, `TEXX:241`) | 16-53 |
| `alpha_msb` | `TEXC:253-254` | 0 (R32_UINT is not alpha-on-MSB) | 59 |
| `num_mip_levels` | `TEXC:248` | 1 | 60-63 |

| Dword | Name | Value | Source |
|---|---|---|---|
| sh4 | `IMAGE_WORD0` low | `0xc0002d84` | `TEXC:139-205`, computed |
| sh5 | `IMAGE_WORD0` high | `0x02570c7c` | `TEXC:197-233`, computed |
| sh6 | `STRIDE_IMAGE_WORD1` low | `0x0000031f \| ((S >> 2) << 16)` low 32 bits | `TEXC:245-251`, computed |
| sh7 | `STRIDE_IMAGE_WORD1` high | `0x10000000 \| ((S >> 2) >> 16)` | same |

Written out for sh6/sh7, with `A = S >> 2` (38 bits at bit 16 of the 64-bit
word, so `S` must be 4-byte aligned and below 2^40):
- sh6 = `0x0000031f | ((A & 0xffff) << 16)`
- sh7 = `0x10000000 | (A >> 16)` (the constant `0x10000000` is `num_mip_levels`
  = 1 at bit 60; `A >> 16` is at most 22 bits, clear of bit 28)

Checks: `IMAGE_WORD0` = `0x02570c7cc0002d84`; the `WORD1` constant part is
`0x100000000000031f`.

## 4. The PDS DOUTD data (texture program)

Per render, Mesa builds a one-kick texture-DMA PDS data segment
(`SPM:694-758`):

| Step | Source |
|---|---|
| `texture_dma_address[0]` = DOUTD_SRC0 with `sbase = consts_buffer->dev_addr` (`C`, 40-bit; `doffset` 0) | `SPM:707-711`; `PDSX:100-107` |
| `texture_dma_control[0]` = DOUTD_SRC1 with `dest = COMMON_STORE`, `bsize = const_shared_regs` | `SPM:713-718`; `PDSX:108-117` |
| `num_texture_dma_kicks = 1` | `SPM:720` |
| data generated by `pvr_pds_generate_pixel_shader_sa_texture_state_data` | `SPM:736-738` |

The generator (`PDSC:3184`, texture branch `PDSC:3499-3541`) allocates
3 constants per kick: the 64-bit address first, then the 32-bit control
(`PDSC:3499-3510`), starting at constant 0 (odd count, `PDSC:253-276`).
`data_size` = 3 dwords (computed; Mesa records it as
`pds_texture_program_data_size`, `SPM:182-185`). The address word gets no
SLC bit, because BXS-4-64 lacks `slc_mcu_cache_controls`
(`PDSC:392-393`; the bit would be `PDSD:1212`). The last kick's control gets
`LAST` (`PDSC:3535-3538`).

| Data dword | Value | Source |
|---|---|---|
| 0 | `C & 0xffffffff` | `PDSC:399`, `PDSC:3512-3528` |
| 1 | `C >> 32` | `PDSC:400` |
| 2 (control) | **`0x90000008`** = `LAST` `0x80000000` \| `COMMON_STORE` `0x10000000` \| `bsize` 8 | `PDSC:3535-3538`; `PDSD:1272`, `:1283`; `PDSX:108-117`; computed |

Mesa's literal control word is `0x90000020` (`bsize` 32, from the bytes
oddity in section 1). `wordsize` ONE (0), `cmode` (0), `a0` (0) and `repeat`
(0) are not set.

`CR_PDS_BGRND3_SIZEINFO` fields (`SPM:903-917`; `CRX:332-352`):

| Field | Our value | Source |
|---|---|---|
| `usc_sharedsize` | 1 (Mesa literal: 2) | section 1 |
| `pds_texturestatesize` | ceil(3 / 4) = 1 | `SPM:911-913`, unit 4 at `CRX:342`, computed |
| `pds_tempsize` | ceil(`pds_texture_program_temps_count` / 4) | `SPM:914-916`; the temps count comes from the code generator (`PDSC` `temps_used`, minimum 1 at `PDSC:3546-3547`; stored at `SPM:239-240`), so at least 1, giving 1 |

`CR_PDS_BGRND0_BASE` and `BGRND1_BASE` are address words and are not
constants (`SPM:886-901`, `CRX:313-325`): `shader_addr` = the pixel program,
`texunicode_addr` = the texture program code, `texturedata_addr` = the data
above.

## 5. The DOUTU word (pixel program kick)

Built once per load shader (`SPM:217-228`):
`pvr_pds_setup_doutu(&usc_task_control, usc_program_dev_addr.addr,
shader_data->common.temps, sample_rate, false)` with `sample_rate` =
`ROGUE_PDSINST_DOUTU_SAMPLE_RATE_INSTANCE` (0), because the shader does not use
sample shading (`SPM:220-222`). Packing (`PDSC:485-523`, constants
`PDSD:1172-1190`; field layout `PDSX:91-99`):

| Field | Value | Bits | Source |
|---|---|---|---|
| `exe_off` | `U & ~3` (low 32 bits) where `U` is the load shader's USC address | 2-31 | `PDSC:494-495`; `pvr_pds.h:1248-1254` |
| `temps` | `ceil(T / 4)` if `T > 0` (`T` = pco's `common.temps`) | 35-40 | `PDSC:497-515`, `PDSD:1172-1176` |
| `sample_rate` | `INSTANCE` = 0, nothing added | 33-34 | `PDSC:512-516`, `PDSX:58-62` |
| `dual_phase` | 0 (`phase_rate_change` false, `SPM:223`) | 41 | `PDSC:518-521` |

`DOUTU_SRC0` = `(U & 0xfffffffc) | (ceil(T / 4) << 35)`, as two dwords:
lo = `U & 0xfffffffc`, hi = `ceil(T / 4) << 3`. `T` is **not** known here: it
is the register count pco reports for the generated load shader, which the
host shader item (`tools/pco_shc`) produces. The note does not guess it.

## 6. Summary: the dwords to pack

`S` = scratch buffer device address, `C` = constants buffer device address,
`U` = load shader USC address, `T` = pco temps. Everything else is fixed for
800x600 RGBA8, 1 sample, 0 tile buffers on BXS-4-64.

| Where | Dword | Value |
|---|---|---|
| constants buffer | sh0 | `0x00000fff` |
| | sh1 | `0x00022400` |
| | sh2 | `0x00000000` |
| | sh3 | `0x00000000` |
| | sh4 | `0xc0002d84` |
| | sh5 | `0x02570c7c` |
| | sh6 | `0x0000031f \| ((S >> 2) & 0xffff) << 16` |
| | sh7 | `0x10000000 \| ((S >> 2) >> 16)` |
| texture-DMA data | 0 | `C & 0xffffffff` |
| | 1 | `C >> 32` |
| | 2 | `0x90000008` |
| DOUTU src0 | lo / hi | `U & 0xfffffffc` / `ceil(T / 4) << 3` |
| `SIZEINFO` | `usc_sharedsize` / `pds_texturestatesize` | 1 / 1 |

## Open points

- `S` must be the scratch buffer's address of the same render: Mesa takes the
  render state's scratch buffer start (`SPM:770`, `SPM:840`); its size
  (800 x 600 x 4 B, stride 800) is `SPM:672`, `:688`.
- `T` (the pco temps of the generated load shader) and the shader address `U`
  come from the load-shader item.
- Not measured: whether the firmware accepts `bsize` 8 / `usc_sharedsize` 1
  (our choice) as well as Mesa's 32 / 2. The board must check it.
