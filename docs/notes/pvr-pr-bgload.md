<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR partial render: the PR background-object load (`pds_pr_bgnd`)

The recipe the resumed render's load needs (driver issue #2, part 3 of
`docs/notes/pvr-render-pool.md`, "The PR frame side, read (Mesa 26.2.1)"):
what the load shader reads, what the PDS program DMAs and kicks, which of
our generators builds it with what, and the three `pds_pr_bgnd` words. The
EOT side (the store the load reads back) is `docs/notes/pvr-pr-eot.md`
(pr-eot-read). The USC load shader is the bgload-shader item (tools/pco_shc);
neither is repeated here. Code is not changed by this note.

## How this was read (and what was not)

- **Mesa 26.2.1 was re-read** in `$CONNECTED_DEPS_DIR/mesa-26.2.1` (a
  shallow git checkout of tag `mesa-26.2.1`; paths below are under
  `src/imagination/`). Checked there: pvr_arch_spm.c:151-241 (programs),
  :638-691 (image words), :694-758 (DMA program), :760-917 (bgobj state),
  :796 and :908-916 (sizes), common/pvr_iface.h:75-102 (layout),
  vulkan/pvr_usc.h:247-253 (data size 8 dwords), pvr_device.c:905-950
  (`pvr_gpu_upload_pds`), csbgen/rogue/cr.xml:332-351 (units). The
  `pool:LINE` cites are the design note's, kept where not re-checked here
  (the pvr_usc.c:1373-1490 shader body, pvr_arch_job_render.c, the
  DDK header `pvr_rogue_fwif_client.h`, which the vendored tree lacks).
- **Our side was read now**, at the line numbers given (this tree,
  ea3424fc).
- The design note's own numbers for backend.rs (`:1136-:2249`, `:8420-8454`)
  have drifted; the lines below are current.

## 1. What the load shader reads

Mesa's shader (`pvr_uscgen_spm_load`, pool:3345-3353, from
pvr_usc.c:1373-1490) samples the SPM scratch once per pixel. Its inputs are
all in the shared registers, filled by the PDS DMA (section 2):

| shared regs | content | fields (pool:3277-3290) |
|---|---|---|
| sh0-3, `PVR_SPM_LOAD_DATA_SMP` | sampler, 4 dwords | `TEXSTATE_SAMPLER_WORD0`: non-normalised coordinates, clamp-to-edge u and v, point min and mag, minlod `CLAMP_MIN`, dadjust `ZERO_UINT`; WORD1 all zero (pvr_arch_spm.c:818-831) |
| sh4-7, `PVR_SPM_LOAD_DATA_REG_TEX` | image state, 4 dwords | format R32_UINT for 1 output dword, linear, 2D, `PVR_TEXTURE_STATE_STORAGE`, framebuffer width and height, **stride** align(width, 2), 1 mip level, the sample count, the scratch **base address** (pvr_arch_spm.c:638-691) |

- The constants buffer is 8 dwords for 0 tile buffers
  (`pvr_uscgen_spm_load_data_size`, pvr_usc.h:249-253, pool:3289-3290); the
  layout is common/pvr_iface.h:75-102 (pool:3277-3278).
- The shader's inputs: `VARYING_SLOT_POS` as texel x/y, one `smp` (uint32,
  2D, `lod_replace` 0), output `FRAG_RESULT_DATA0` R32_UINT in output
  register 0 (pool:3345-3353). Our configuration is shader index 0:
  `output_reg_count` 1, `tile_buffer_count` 0, not multisampled
  (pvr_usc.h:210-214, :264-267; pool:3339-3343).
- The scratch the image words name is the one the PR's PBE words write
  (packmode U32, stride align(width, 2), clip width-1 x height-1;
  pvr_arch_spm.c:258-326, pool:3354-3359). Its size and address are
  `src/pvr/spm.rs` (spm-calc), not this note's.
- **Our packers for the same words**, all in `src/pvr/backend.rs`:
  - image word0: `tex_image_word0` (:1270-1289), word1 of a linear image:
    `tex_stride_image_word1` (:1294-1306: `stride - 1`, mip levels, address);
  - sampler word0 packed the same way as :5205-5225
    (`ROGUE_TEXSTATE_SAMPLER_WORD0` with `dadjust` `ZERO_UINT`, `minlod`
    `CLAMP_MIN`, address modes, filters); word1 stays zero except the cube
    case (:5078-5084);
  - **gap:** there is no R32_UINT texel format constant. The existing ones
    are 8-bit, F16/F32, ETC, depth (`TEXF_*`, :1113-1200). `ROGUE_TEXSTATE_FORMAT_U32`
    is a csbgen name to confirm (the PBE side already uses U32 packmode,
    :963). The `TEX_DESC_DWORDS` combined descriptor (:1363, 20 dwords) is
    our draw's layout, not Mesa's 8-dword SPM block: do not reuse it as is.

## 2. What the PDS program DMAs and kicks

Three parts, as Mesa builds them once per load shader and per render
(pvr_arch_spm.c:151-241, :694-758, pool:3291-3309):

1. **Texture program** (`texunicode`): the common-store DMA,
   `pvr_pds_generate_pixel_shader_sa_code_segment` with **one** texture DMA
   kick. Data per render: DOUTD src0 = the constants buffer's device
   address, src1 = destination in the common store (0), `bsize` = the
   constants size.
2. **Pixel program** (`shader_addr`): DOUTU of the load shader with its
   temp count and sample rate INSTANCE (FULL only with sample shading;
   pvr_arch_spm.c:217-228).
3. **Per-render data** (`texturedata`): the DMA data above. Only this and
   the constants buffer change with the framebuffer; the two programs are
   static.

**Mesa unit oddity, decided in the design (pool:3310-3324):** Mesa passes the
constants size in bytes (pvr_arch_spm.c:796) where `bsize` and
`usc_sharedsize` count dwords, giving 32 instead of 8 for 0 tile buffers.
The transcription uses 8 dwords (`bsize` 8, `usc_sharedsize` 1) on purpose;
the board checks it.

## 3. Which of our generators builds it, with what

`driver/src/pvr_pds_render_gen.c`, Mesa's own generators behind a small C
face (`driver/src/pvr_pds_render_gen.h`), called from `src/pvr/backend.rs`:

| piece | generator | inputs | today's use |
|---|---|---|---|
| pixel program | `pvr_pds_render_kick_usc_program` (gen.c:200-225; wraps `pvr_pds_setup_doutu` + `pvr_pds_kick_usc`): data 8 dwords, code 1 | the load shader's USC heap offset, its temps, sample rate 0 (INSTANCE) | the clear kick, three variants, built once per frame set: `frame_set_static_init` (backend.rs:2608-2685); draw kicks at :7040-7075 |
| texture program | `pvr_pds_render_dma_program` (gen.c:289-298; one-kick form `pvr_pds_render_color_upload_program` :272-287, both over `dma_program` :235-270): data 3 dwords per kick, code 2, `temps_used` | `PvrPdsDmaKick { address, dwords, dest_offset }` (:7216, :7379): address = constants buffer, dwords = 8, dest_offset = 0 | the background colour DMA: 6 dwords from `bg_color`, dest 0 (backend.rs:2692-2723); draws use the multi-kick form (:7379) |

- A load shader is **not in the table yet**: its USC offset comes from the
  bgload-shader item (tools/pco_shc, in review) and the table entry that
  bgload-pds adds. `fs_usc_exec_offset` must be that entry's heap offset
  (`heap_off_of`, as :7042 does for draws).
- Both programs are static per device, so they belong in the static page
  built once like the clear's (the per-set static PDS page,
  `PDS_STATIC_BYTES`, :2608). The per-render part is the 8-dword constants
  buffer (sampler, image words from section 1), which holds the scratch
  address and so is written when the PR scratch is bound.
- The constants buffer is read by the DMA at kick time and written by the
  CPU: `cache_flush` it as `bg_color` is (:8087-8088). The generator output
  sizes are clamped to the arrays as the existing call sites do (:2641,
  :2711).

## 4. The three `pds_pr_bgnd` words

Packed exactly like our `pds_bgnd` (backend.rs:8421-8456, in `flush`, the
background-object block):

| word | register | fields | value for the PR load |
|---|---|---|---|
| `[0]` | `CR_PDS_BGRND0_BASE` | `shader_addr` = pixel program, `texunicode_addr` = texture program code | the load kick (`kick_usc` output) and the DMA code, both PDS heap offsets (`dev_addr`, as :8426-8433) |
| `[1]` | `CR_PDS_BGRND1_BASE` | `varying_addr` 0, `texturedata_addr` = per-render DMA data | the DMA data of section 2 (:8434-8441) |
| `[2]` | `CR_PDS_BGRND3_SIZEINFO` | `usc_sharedsize` = ceil(constants dwords / 16), `pds_texturestatesize` = ceil(data bytes / 4), `pds_tempsize` = ceil(temps / 4) | 8 dwords -> 1; **Mesa: ceil(aligned data dwords / 4) = 1** (`data_size` is in dwords, aligned to 16 bytes by `pvr_gpu_upload_pds`, pvr_device.c:938-949; pvr_arch_spm.c:911-913), so one kick (3 dwords, 4 aligned) gives 1, **not 3**; `temps_used` / 4 (unit register layout cr.xml:332-351, pool:3301-3309). Today's code writes `CLEAR_ATTACH_DWORDS.div_ceil(16)` (= 1; :1046, :8449), `bg_dma_data_dw * 4 / 4` = the dwords (3, :8450) and `bg_dma_temps` (:8451); the clear's 3 differs from Mesa's formula and is silicon-proven for the clear only - the PR word follows Mesa and is board-checked |

- Mesa fills `pds_pr_bgnd` from `pr_pds_bgnd_reg_values` and uses the same
  words for `pds_bgnd` of a barrier load
  (pvr_arch_job_render.c:1077-1083, pvr_arch_cmd_buffer.c:1696-1717;
  pool:2846-2850, :3308-3309).
- The struct is `pvr_rogue_fwif_client.h:232`, next to `pds_bgnd` `:227`
  (pool:2847). **Where it sits in our 80-dword 3D command is unknown.** Our
  command has `pds_bgnd` at dk[70..75] and the flags at dk[76]
  (driver/rust/src/gpu/pvr/render.rs:1434-1437); the self-test says the
  captured layout has no dwords for it (driver/src/pvr_selftest.c:1537).
  `RGXFWIF_CMD3D` is a DDK user-mode struct, so `driver/tools/offsetof`
  cannot pin it (pool:3325-3337). Needs a DDK capture of a 3D command with
  a PR background before bgload-pds writes it. The host layout check for
  the frag regs is `src/abi_layout.rs:80` (`pds_bgnd` at 104); the field
  goes into `PvrRenderFragRegs` (src/osal/render.rs:82) under kick-abi.
- For the ordinary case (every colour attachment STORE) Mesa's PR
  background is the render's own load-op program, not the scratch load
  (pool:3244-3252): only a scratch PR needs this recipe.

## 5. Host tests (for bgload-pds)

In `src/pvr/backend_tests.rs`, in the style of the `pds_bgnd` assertions
(:4651, :5539):

1. **Words pinned by literal:** `pds_pr_bgnd[0..3]` for 800x600 RGBA8 equal
   the three dwords computed from Mesa's field packing (hand-checked in the
   test comment: `usc_sharedsize` 1, `pds_texturestatesize` 1 (Mesa's 4 aligned dwords / 4),
   `pds_tempsize` 1 for the one-kick program).
2. **Only with the knob:** `pds_pr_bgnd` is all zero without it; `pds_bgnd`
   is byte-identical to today's (the existing background tests unchanged).
3. **Offsets point into the right programs:** `shader_addr` is the load
   kick's, `texunicode_addr`/`texturedata_addr` are the DMA's, and they
   differ from the clear background's (`bg_kick_off`, `bg_dma_*_off`).
4. **Kick program:** `pvr_pds_render_kick_usc_program` for the load
   shader's offset and temps gives data 8 / code 1 and the DOUTU words with
   that offset (the stub is `backend_tests.rs:301`).
5. **DMA program:** one kick of 8 dwords to dest 0 from the constants
   address: data 3 dwords, code 2, `temps_used` (stub :328); not 6 dwords
   (the clear's) and not 32 (Mesa's byte count).
6. **Constants buffer:** the sampler words equal the packed
   `SAMPLER_WORD0` (point, clamp, non-normalised, `ZERO_UINT`) and WORD1 is
   zero; the image words for a W x H scratch hold format R32_UINT, stride
   `align(W, 2) - 1`, width and height minus one, 1 level and the scratch
   address. Stride and address are the same values `src/pvr/spm.rs`
   (spm-calc) puts in the PR PBE words.
7. **Layout:** the new `PvrRenderFragRegs` field is placed by the capture
   (section 4) and covered by `src/abi_layout.rs`; recheck `grep -rl
   'PvrRenderFragRegs {' tests src` for literals (CLAUDE.md).

## Open, for the planner

- The unit oddity and the
  `pds_pr_bgnd` placement are the two decisions the board verifies.
- No R32_UINT texel format exists in `backend.rs`; bgload-pds adds one
  (swizzle R, 0, 0, 1) with its own host test.
- Whether the scratch is sampled with the frame's own twiddle/stride rules
  (it is linear, stride align(W, 2)) is settled by spm-calc's PBE words, which
  this load must match to the word.
