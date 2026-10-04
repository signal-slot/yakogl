# FINAL `rogue_fwif_transfer_regs` for a BXS-4-64 TQ_3D FILL (64x64 RGBA8888 linear)

Target: PowerVR **BXS-4-64, BVNC 36.53.104.796** (TI AM62P). Job = `DRM_PVR_JOB_TYPE_TRANSFER_FRAG`
→ `ROGUE_FWIF_CCB_CMD_TYPE_TQ_3D` (0x2ABC80CA), **FILL** mode.
Dest surface: dev-vaddr **0x8010000000** (GENERAL heap), **W=64 H=64**, VK_FORMAT_R8G8B8A8_UNORM,
LINEAR, **pixel stride = 64**. Single RT. Tile = 16x16 → **4x4 tiles**.

> **THE ONE BIG CORRECTION vs. the earlier specs.**
> A plain FILL is NOT the Fast2D / ISP-control-stream path. In Mesa a fill with no source and no
> FAST2D flag is routed `pvr_queue_transfer` → `pvr_3d_copy_blit` → **`pvr_3d_copy_blit_core`**
> (the **FastScale** HW path), because `PVR_TRANSFER_CMD_FLAGS_FAST2D` is set *only* by
> `pvr_reroute_to_clip` (`pvr_arch_job_transfer.c:5583`), and the top-level picks the path on that
> flag (`:5997`). Consequences, all verified in `pvr_3d_copy_blit_core` (`:2809-3059`):
> - `isp_render.mode_type = **FAST_SCALE (3)**`, NOT FAST_2D (`:3018-3025`).
> - **No ISP control stream is built** — `pvr_isp_ctrl_stream()` is called only from
>   `pvr_3d_clip_blit` (`:4656`), which the fill never enters. Therefore
>   **`isp_mtile_base = 0`** and **`isp_rgn = 0`** (both left at their zero-init; the `regs` live in
>   `prep_data` which is `memset(0)` at `:5985`). They are *not* an offset and *not* a size.
> - **No NOP-PDS primitive block exists** → there are no `TA_STATE_PDS_SHADERBASE/SIZEINFO1/
>   VARYINGBASE` inputs to produce. `pds_bgnd0/1/3 = 0`. See the NOP verdict at the bottom.
>
> The only hand-built blobs a FastScale fill needs are: (1) the fixed **USC EOT emit** program, and
> (2) the tiny **PDS pixel-event** program that kicks it (code = `{0xF0040101,0xF4000002}`), plus the
> handful of register values below. No ISP control stream, no NOP PDS.

Stream order the kernel expects (`pvr-src/pvr_stream_defs.c:234-259`): 64-bit block
(`pds_bgnd0_base, pds_bgnd1_base, pds_bgnd3_sizeinfo, isp_mtile_base, pbe_wordx_mrty[9]`) then 32-bit
(`isp_bgobjvals, usc_pixel_output_ctrl, usc_clear_register0..3, isp_mtile_size, isp_render_origin,
isp_ctl, isp_aa, event_pixel_pds_info, event_pixel_pds_code, event_pixel_pds_data, isp_render,
isp_rgn, frag_screen`). `isp_xtp_pipe_enable` is **ABSENT** (S7_TOP_INFRASTRUCTURE = FALSE);
`frag_screen` is **PRESENT** (GPU_MULTICORE_SUPPORT = TRUE). `common.frame_num`/`flags`/`padding` are
set by the kernel, not in the stream.

---

## FINAL VALUE TABLE

| field | type | FINAL value | fixed / runtime |
|---|---|---|---|
| `pds_bgnd0_base` | u64 | `0x0000000000000000` | fixed |
| `pds_bgnd1_base` | u64 | `0x0000000000000000` | fixed |
| `pds_bgnd3_sizeinfo` | u64 | `0x0000000000000000` | fixed |
| `isp_mtile_base` | u64 | `0x0000000000000000` | fixed (FastScale: no ctrl stream) |
| `pbe_wordx_mrty[0]` | u64 | `0x0200007C68800000` | addr/stride/fmt-dependent |
| `pbe_wordx_mrty[1]` | u64 | `0x0000003F0000003F` | W/H-dependent |
| `pbe_wordx_mrty[2]` | u64 | `0x0000000000000000` | fixed |
| `pbe_wordx_mrty[3..8]` | u64 | `0x0000000000000000` (all) | fixed (RT1/RT2 unused) |
| `isp_bgobjvals` | u32 | `0x00000000` | fixed |
| `usc_pixel_output_ctrl` | u32 | `0x0007FFF8` | fixed for this core |
| `usc_clear_register0` | u32 | `0xFF0000FF` (example opaque red) | = the fill color |
| `usc_clear_register1` | u32 | `0x00000000` | fixed |
| `usc_clear_register2` | u32 | `0x00000000` | fixed |
| `usc_clear_register3` | u32 | `0x00000000` | fixed |
| `isp_mtile_size` | u32 | `0x00040004` | W/H-in-tiles |
| `isp_render_origin` | u32 | `0x00000000` | fixed |
| `isp_ctl` | u32 | `0x00025000` | fixed for this core |
| `isp_aa` | u32 | `0x00000000` | fixed (1 sample) |
| `event_pixel_pds_info` | u32 | `0x00000202` | fixed for 1 RT |
| `event_pixel_pds_code` | u32 | `= PDS code offset` (runtime) | runtime (see formula) |
| `event_pixel_pds_data` | u32 | `= PDS data offset` (runtime) | runtime (see formula) |
| `isp_render` | u32 | `0x00000003` | fixed (FAST_SCALE, TL2BR) |
| `isp_rgn` | u32 | `0x00000000` | fixed (FastScale) |
| `frag_screen` | u32 | `0x003F003F` | W/H-dependent |
| `isp_xtp_pipe_enable` | — | **omit from stream** | S7_TOP_INFRASTRUCTURE=FALSE |

`common.frame_num = 0`, `flags = 0`, `padding = 0` (kernel: `pvr-src/pvr_job.c:271-272`).

---

## PER-FIELD DERIVATION + CR CITATIONS

All csbgen field cites are `mesa-build/mesa-26.2.1/src/imagination/csbgen/rogue/cr.xml` unless noted;
`pbestate.xml` = `.../csbgen/rogue/pbestate.xml`; kernel = `pvr-src/pvr_rogue_cr_defs.h` /
`pvr_rogue_cr_defs_client.h`. Mesa code = `.../vulkan/pvr_arch_job_transfer.c` and
`.../vulkan/pvr_arch_job_common.c`.

### pds_bgnd0_base / pds_bgnd1_base / pds_bgnd3_sizeinfo = 0
`pvr_setup_hwbg_object()` (`common… transfer.c:650-697`). For a fill:
`state->pds_shader_task_offset = 0` and `uni_tex_code_offset = 0` (`:2872-2874`) →
`PDS_BGRND0_BASE.shader_addr=0, texunicode_addr=0` → 0. `tex_state_data_offset=0` →
`PDS_BGRND1_BASE=0`. `PDS_BGRND3_SIZEINFO`: `usc_sharedsize=DIV_ROUND_UP(common_ptr,·)` with
`state->common_ptr=0` (`:2824`), and `pds_uniformsize/pds_texturestatesize/pds_tempsize` all 0
(`pds_temps=0`, `:2837`). ⇒ **0**. (cr.xml `PDS_BGRND0_BASE`@312, `PDS_BGRND1_BASE`@320,
`PDS_BGRND3_SIZEINFO`.)

### isp_mtile_base = 0
Set only by `pvr_isp_ctrl_stream()` `pvr_csb_pack(CR_ISP_MTILE_BASE){addr = cs_bo - transfer_frag_heap_base}`
(`:4557-4561`). The FastScale fill never calls that function, so the field stays at its
`memset(0)` value. **CR_ISP_MTILE_BASE** = `struct ISP_MTILE_BASE length=2`, field
`addr start=2 end=39 shift=2` (cr.xml:479-481). *Formula if you ever switch to the Fast2D path:*
`isp_mtile_base = ISP_control_stream_dev_vaddr − 0xE400000000` (TRANSFER_FRAG_HEAP base).
For FastScale keep **0**.

### pbe_wordx_mrty[0..2]  (PBE **reg** words, `pvr_arch_pbe_pack_state`, common.c:82-299)
Surface: addr=0x8010000000, LINEAR, pixel stride 64, identity swizzle X,Y,Z,W, min/max clip
x=[0,63] y=[0,63].
- **[0] = REG_WORD0 = 0x0200007C68800000** (pbestate.xml:245-273)
  - `tilerelative=1` (start 57) → `0x0200000000000000` (common.c:142)
  - `memlayout=LINEAR=0` (start 32-33) → 0 (pbestate.xml MEMLAYOUT.LINEAR=0)
  - `linestride=(stride-1)/UNIT_SIZE=(64-1)/2=31=0x1F` (start 34, `UNIT_SIZE=2`) →
    `31<<34 = 0x0000007C00000000` (common.c:174-175; pbestate.xml linestride)
  - swizzle identity: `swiz_chan0=SOURCE_CHAN0=0`(s20)=0, `swiz_chan1=SOURCE_CHAN1=1`(s23)=`0x800000`,
    `swiz_chan2=SOURCE_CHAN2=2`(s26)=`0x8000000`, `swiz_chan3=SOURCE_CHAN3=3`(s29)=`0x60000000`
    → `0x68800000` (common.c:179-278; SWIZ enum pbestate.xml; SOURCE_CHANn = n)
  - `minclip_x=0`, rotation/y_flip/gamma/downscale = 0.
- **[1] = REG_WORD1 = 0x0000003F0000003F** (pbestate.xml:275-282)
  - LINEAR ⇒ `size_x=size_y=0`; `minclip_y=0`; `zslice=0`
  - `maxclip_x=63` (start 32) → `0x0000003F00000000`; `maxclip_y=63` (start 0) → `0x3F`
    (common.c:293-296)
- **[2] = REG_WORD2 = 0** (pbestate.xml:284-289). `pbe_reg_words[2]=0` (common.c:99); ERN **42064 is
  ABSENT** on this BVNC (enhancements = 35421/38748/42307/45493) so neither the `pbe_regs[2]=0`
  re-zero nor the `sw_bytemask` OR runs (transfer.c:1282,1308-1316) — stays 0.
- **[3..8] = 0** (RT1/RT2 unused; `num_rts = plane_count(RGBA8888)=1`, transfer.c:1248).
Stride into the array = `ROGUE_PBE_WORDS_REQUIRED_FOR_TQS = 3`, array is `pbe_wordx_mrty[3*3]`
(`pvr-src/pvr_rogue_fwif_client.h:355-357`).
*Address formula for other dests:* `[0].address bits are NOT here` — the dest base lives in the PBE
**STATE** words (below); REG_WORD0/1 carry only stride + clip. `linestride=(pixel_stride−1)/2`,
`maxclip_x=W−1`, `maxclip_y=H−1`.

### isp_bgobjvals = 0
Fill branch `pvr_csb_pack(&regs->isp_bgobjvals, CR_ISP_BGOBJVALS, reg);` with an **empty** body
(`:2844`) ⇒ all fields 0 (enablebgtag=0, mask=0, stencil=0). The `mask=true` OR at `:4650` is inside
`pvr_3d_clip_blit`, which a fill never enters. **CR_ISP_BGOBJVALS**: `enablebgtag s9`, `mask s8`,
`stencil s0-7` (cr.xml:517-521).

### usc_pixel_output_ctrl = 0x0007FFF8
From `pvr_arch_setup_tiles_in_flight()` (common.c:304-503), `pvr_csb_pack(CR_USC_PIXEL_OUTPUT_CTRL)`
(common.c:463-495). Inputs for this core: `usc_min_output_registers_per_pix=2`, so
`pixel_width = next_pow2(MAX2(usc_pixel_width=1, 2)) = 2` ⇒ `width = PIXEL_WIDTH_2REGISTERS = 0`
(enum cr.xml:92-97). `usc_pixel_partition_mask` feature TRUE ⇒
`partition_mask = (1<<max_partitions)-1 = (1<<16)-1 = 0xFFFF` (common.c:482-485).
Pack: `partition_mask (start 3) = 0xFFFF<<3 = 0x7FFF8`, `enable_4th_partition (s2)=0`, `width (s0)=0`
⇒ **0x0007FFF8**. **CR_USC_PIXEL_OUTPUT_CTRL** fields (cr.xml:674-678):
`partition_mask start=3 end=20`, `enable_4th_partition start=2`, `width start=0 end=1`.
(`usc_pixel_width=1` because `pvr_pbe_get_pixel_size(RAW32/UU8888)=1`, transfer.c:963-976,1020.)

### usc_clear_register0 = fill color; 1..3 = 0
`pvr_pack_clear_color()` U8U8U8U8 branch (transfer.c): `pkd[0] = R | (G<<8) | (B<<16) | (A<<24)`,
each channel `pvr_float_to_ufixed(chan,8)` (0..255). Only `pkd[0]` is used for a 32-bpp format.
Written at transfer.c:2856-2871. **CR_USC_CLEAR_REGISTER.val start0 end31** (cr.xml, `USC_CLEAR_REGISTER`).
**Byte order is AABBGGRR.** Examples: opaque **red** (1,0,0,1) → **0xFF0000FF**; opaque **green**
(0,1,0,1) → **0xFF00FF00**; opaque **blue** → **0xFFFF0000**; opaque white → 0xFFFFFFFF.
`usc_clear_register1 = usc_clear_register2 = usc_clear_register3 = 0`.

### isp_mtile_size = 0x00040004
`pvr_isp_tiles()` `pvr_csb_pack(CR_ISP_MTILE_SIZE){ x = width_in_tiles; y = height_in_tiles; }`
(transfer.c:1431-1434). For 64x64, tile=16: `width_in_tiles = (max_x_clip + tile)/tile − origin =
(63+16)/16 = 4`; height likewise **4** (transfer.c:900-912). msaa_multiplier=0 (1 sample) so no
shift (transfer.c:1416-1423). **The field is the raw tile COUNT, not count−1, not a macrotile
count** — `reg.x = width` is assigned directly. Pack: `x (start16)=4<<16=0x40000`, `y (start0)=4`
⇒ **0x00040004**. **CR_ISP_MTILE_SIZE** (kernel `pvr_rogue_cr_defs_client.h:140-155`:
`X_SHIFT=16`, `Y_SHIFT=0`, `MASKFULL=0x03FF03FF`; cr.xml:483-486).

### isp_render_origin = 0
`pvr_csb_pack(CR_ISP_RENDER_ORIGIN){ x=origin_x=0; y=origin_y=0 }` (transfer.c:1436-1439);
origin_x/y_in_tiles=0 for a full-surface fill (transfer.c:900-901). **CR_ISP_RENDER_ORIGIN**:
`x start=16 end=25`, `y start=0 end=9` (cr.xml:474-477).

### isp_ctl = 0x00025000
Two contributions, OR'd (transfer.c:1450-1463):
1. `pvr_csb_pack(CR_ISP_CTL){ process_empty_tiles = true }` ⇒ bit17 = **0x00020000**
   (kernel `ROGUE_CR_ISP_CTL_PROCESS_EMPTY_TILES_SHIFT 17`, `_EN 0x00020000`, cr_defs.h:2304-2306).
   `paired_tiles` not set (fill uses `PVR_PAIRED_TILES_NONE`, transfer.c:1262).
2. `|= isp_tiles_in_flight` = the `CR_ISP_CTL` packed by `setup_tiles_in_flight` with
   `pipe_enable = total_tiles_in_flight − 1` (xt_top_infrastructure ABSENT, common.c:497-502).
   **total_tiles_in_flight = 6** (derivation below) ⇒ `pipe_enable = 5`, `PIPE_ENABLE_SHIFT 12`
   ⇒ `5<<12 = 0x00005000` (cr_defs.h:2310-2313, value 5 = `PIPE_ENABLE_PIPE_SIX 0x5000`).
⇒ **0x00020000 | 0x00005000 = 0x00025000**.

**tiles-in-flight derivation for BXS-4-64** (`pvr_arch_setup_tiles_in_flight`, common.c:304-503;
features from `mesa-src/bxs-4-64.h`), inputs `msaa_mode=AA_NONE`, `pixel_width=1`,
`paired_tiles=false`, `max_tiles_in_flight=0`:
- `usc_min_output_regs=2` → `pixel_width=next_pow2(max(1,2))=2`; `partition_size=2`.
- `isp_samples_per_pixel=4`, but AA_NONE so no multiply → `partition_size=2`.
- `tile_size_x=16` ⇒ `partition_size *= 16*16 = 256` → **512**.
- `max_partitions=16`; `total_reserved_partition_size` = kernel
  `common_store_partition_space_size = 16*16*16*2 = 8192` (pvr-src/pvr_drv.c:233-252) ⇒
  `usable = MIN(8192, 512*16=8192) = 8192`. (`common_store_size_in_dwords=21504 ≥ 16384`, so the
  16x16 low-CS clamp is skipped.)
- `partitions_available = MIN(16, 8192/512=16) = 16`.
- `roguexe` TRUE, `xt_top_infrastructure` absent ⇒ `max_phantoms = num_raster_pipes = 1`.
- loop (1 phantom): `usc_tiles_in_flight = 16` (16x16 tiles skip the quadrant scaling);
  `isp_tiles_in_flight = isp_max_tiles_in_flight = 6`; **SIPF v2 present ⇒ NOT divided by
  num_phantoms** (common.c:414-419); `isp_tiles_in_flight = MIN(16,6)=6`;
  `total_tiles_in_flight = 6`.
- SIPF v2 post-clamp: `total = MIN(6, partitions_available=16) = 6`. ⇒ **total_tiles_in_flight = 6**.

### isp_aa = 0
`pvr_csb_pack(CR_ISP_AA){ mode = aa_mode_type(sample_count=1) = AA_NONE = 0 }` (transfer.c:2846-2848).
**CR_ISP_AA.mode start=0 end=1, ISP_AA_MODE_TYPE.AA_NONE=0** (cr.xml:47-52, 523-525).

### event_pixel_pds_info = 0x00000202
`pvr_pbe_setup_emit()` `pvr_csb_pack(CR_EVENT_PIXEL_PDS_INFO)` (transfer.c:1223-1231):
- `temp_stride = 0`.
- `const_size = DIV_ROUND_UP(program.data_size, CONST_SIZE UNIT_SIZE=4)`. `data_size = 5` dwords
  (control@0[2] + emit@2[2] + ctrlword@4[1]; `pvr_pds_generate_pixel_event` + `pvr_pds_get_constants`,
  `pvr_pds.c:540-620`) ⇒ `DIV_ROUND_UP(5,4) = 2`.
- `usc_sr_size = DIV_ROUND_UP(rt_count(1) * PVR_STATE_PBE_DWORDS(2), USC_SR_SIZE UNIT_SIZE=16)
  = DIV_ROUND_UP(2,16) = 1`.
Pack: `usc_sr_size (start9)=1<<9=0x200 | temp_stride (start5)=0 | const_size (start0)=2` ⇒ **0x202**.
**CR_EVENT_PIXEL_PDS_INFO** (cr.xml:301-311): `usc_sr_size start=9 end=14 (UNIT_SIZE=16)`,
`temp_stride start=5 end=8 (UNIT_SIZE=4)`, `const_size start=0 end=4 (UNIT_SIZE=4)`.

### event_pixel_pds_code / event_pixel_pds_data = PDS-heap offsets (RUNTIME)
`pvr_csb_pack(CR_EVENT_PIXEL_PDS_CODE){ addr = pds_upload.code_offset }` and `…DATA{ addr =
pds_upload.data_offset }` (transfer.c:1233-1239). Both are **offsets from the PDSCODEDATA heap base**
(0xDA00000000; PDS `EXEC_BASE` is programmed globally), 16-byte aligned. The register value equals the
offset directly (field `addr start=4 end=31 shift=4` — the low 4 bits are 0 for a 16-aligned offset;
cr.xml:292-299). **Formula:** `event_pixel_pds_code = code_bo_vaddr − 0xDA00000000`;
`event_pixel_pds_data = data_bo_vaddr − 0xDA00000000`. Fill in at runtime after you upload the
5-dword data + 2-dword code (see `pds_pixel_event.h`).

### isp_render = 0x00000003  (FAST_SCALE, TL2BR)
`pvr_csb_pack(CR_ISP_RENDER){ mode_type = FAST_SCALE; dir_type = <scan dir> }` (transfer.c:3018-3025).
`mode_type = FAST_SCALE = 3` (start 0-1). `dir_type` from `pvr_isp_scan_direction`: with
**source_count=0** (a fill) the loop never runs → backwards_in_x=backwards_in_y=false →
`dir_type = TL2BR = 0` (transfer.c:2747-2807; DIR_TYPE.TL2BR=0, cr.xml:41-46). ⇒ **0x00000003**.
**CR_ISP_RENDER** (cr.xml:467-472): `mode_type start=0 end=1`, `dir_type start=2 end=3`,
`resume s4`, `disable_eomt s5`; ISP_RENDER_MODE_TYPE.FAST_SCALE=3 (cr.xml:55-59;
kernel `ROGUE_CR_ISP_RENDER_MODE_TYPE_FAST_2D=2 / FAST_SCALE=3`, cr_defs.h:2230-2232).
NOTE: this is **FAST_SCALE (3)**, not FAST_2D (2). FAST_2D would only be correct on the clip path.

### isp_rgn = 0
Set only by `pvr_isp_ctrl_stream` (SIPF branch `CR_ISP_RGN_SIPF{ cs_size_ipf_creq_pf = was_linked }`,
transfer.c:4569-4573), which the fill never calls ⇒ stays at zero-init. Even on the clip path a
single non-linked block gives `was_linked=false` ⇒ 0. **CR_ISP_RGN_SIPF** (cr.xml:497-514):
`cs_size_ipf_creq_pf start=24 end=28` (present because `ipf_creq_pf` TRUE), `size start=0 end=23
(UNIT_SIZE=1)`. Keep **0**.

### frag_screen = 0x003F003F
`pvr_csb_pack(CR_FRAG_SCREEN){ xmax = dst.width-1 = 63; ymax = dst.height-1 = 63 }` (transfer.c:3047-3050;
GPU_MULTICORE_SUPPORT present). Pack: `ymax (start16)=63<<16=0x3F0000 | xmax (start0)=63=0x3F` ⇒
**0x003F003F**. **CR_FRAG_SCREEN** (cr.xml:631-634): `ymax start=16 end=30`, `xmax start=0 end=14`.

---

## PBE STATE words (the EOT `emit_words` = "pbe_word0/word1") — needed by the PDS pixel-event data seg
These are NOT in `transfer_regs`; they are DMA'd into USC shared regs sh[0],sh[1] by the pixel-event
program and consumed by the EOT `emitpix`. Packed by the same `pvr_arch_pbe_pack_state` (STATE words,
common.c:115-140; pbestate.xml:226-243). For dest 0x8010000000, RGBA8888 UNORM:
- **pbe_word0 = STATE_WORD0 = 0x04000000** — `address_low start0 shift2 = (0x8010000000>>2)&0xFFFFFFFF`.
- **pbe_word1 = STATE_WORD1 = 0x00082020** — `address_high (start0 shift34)=(0x8010000000>>34)&0x3F=0x20`
  `| source_format=8_PER_CHANNEL=1 (start19) =0x80000 | norm=1 (start13) =0x2000 |
  packmode=U8U8U8U8=0 (start7) | source_pos=0 | mrt_index=0`.
  (source_format=8_PER_CHANNEL forced for a UNORM 8888 color_fill, transfer.c:1063-1070;
  is_normalized=true, common.c:825.)
*Address formula:* `pbe_word0 = (dest_vaddr>>2) & 0xFFFFFFFF`;
`pbe_word1 = ((dest_vaddr>>34)&0x3F) | 0x82020` (the 0x82020 = src_fmt|norm constant for 8888 UNORM).

---

## NOP PDS VERDICT (the "one remaining blob")

**A FastScale TQ_3D FILL needs NO NOP PDS program at all. Use `pds_bgnd0_base = pds_bgnd1_base =
pds_bgnd3_sizeinfo = 0`; there is no ISP primitive block, hence no `TA_STATE_PDS_SHADERBASE /
SIZEINFO1 / VARYINGBASE` to fill.** Evidence:

1. The fill path is `pvr_3d_copy_blit_core`, whose FILL branch sets `state->pds_shader_task_offset=0`,
   `uni_tex_code_offset=0`, `tex_state_data_offset=0`, `pds_temps=0`, `common_ptr=0`
   (transfer.c:2824,2837-2874). `pvr_setup_hwbg_object` turns those into `pds_bgnd0/1/3 = 0`
   (transfer.c:650-697). The background-object PDS (which is where `device->nop_program` would be
   referenced, via `pds_shader_task_offset`) is therefore **null**.
2. `device->nop_program` (`pvr_arch_device.c:517-577`, `pvr_device_init_nop_program`) is a
   *kick-USC* PDS program that launches a precompiled empty fragment USC (`pco_usclib_common[FS_NOP_COMMON]`).
   It is referenced **only** (a) as the background-object shader on the **source-blit** path when
   `state->empty_dst` (transfer.c:2925-2927), and (b) as the primitive-block `PDS_SHADERBASE` inside
   the **Fast2D** ISP control stream (transfer.c:4208-4209). Neither code path is reached by a
   FastScale fill.
3. There is **no ISP control stream / primitive block** in the FastScale fill (see the big note at
   top), so `TA_STATE_PDS_SHADERBASE/SIZEINFO1/VARYINGBASE` do not exist as inputs. The clear color
   is delivered purely by `usc_clear_register0..3`; the PBE write is delivered purely by the EOT emit
   program kicked by the pixel-event PDS program.

The ONLY PDS program a fill needs is the **pixel-event (EOT-kick)** program — that is not a "NOP", it
is essential, and it is fully specified/portable (see `pds_pixel_event.h`): code = constant
`{0xF0040101, 0xF4000002}`, data = `{eot_off&~3, (ceil(usc_temps/4)&0x3F)<<3, pbe_word0, pbe_word1,
0x90000002}`. The only compiled blob left is the **USC EOT emit** shader (color/size-independent;
capture once for this BVNC — see `gpu-usc-pds-spec.md §1.4` / `capture_eot_usc.c`).
