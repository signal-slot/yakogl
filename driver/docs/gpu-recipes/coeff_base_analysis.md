<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Fragment coefficient-store fault (0x4c4000000) and the CMD3D layout

Conclusions of an early bring-up analysis, cut down to what is still true.
It originally decoded words of a captured DDK 3D command; those words are not
reproduced here (the capture is no longer in the tree). The layouts now come
from the vendored DDK headers (`third_party/ti-img-rogue-ddk-25.2`), which is
the authority; this note records only where the layout came from and what the
fault meant.

## 0. Target BVNC and what it gates

The target firmware is ROGUE, BVNC 36.53.104.796 (DDK 25.2
`config_kernel.h`: `rgxcore_km_36.53.104.796.h`).

- present: `FBCDC_ALGORITHM=50`, `FBCDC_ARCHITECTURE=7`, `GPU_MULTICORE_SUPPORT`,
  `NUM_CLUSTERS=1`, `NUM_RASTER_PIPES=1`, 16x16 tiles
- absent: `S7_TOP_INFRASTRUCTURE`, `CLUSTER_GROUPING`, `ZLS_SUBTILE`,
  `ISP_ZLS_D24_S8_PACKING_OGL_MODE`, `FB_CDC_V4`
- errata: only `FIX_HW_BRN_71317` (ERNs 42290/42606/47025/57596); BRN 49927,
  65101, 62850/62865, 66193 and 67182 are all absent.

These flags gate the conditional fields of the firmware's fragment and
geometry register blocks. The mainline-pvr struct that was first copied
declares every field unconditionally, so its command block is larger than, and
laid out differently from, what this firmware build reads. The firmware reads
its block by its own offsets, so a mis-sized struct delivers `isp_ctl`,
`isp_bgobjvals`, the PBE render-target address and the background PDS
programs to the wrong registers. The fix is to take the layout from the
vendored headers for this BVNC (`pvr_fwif.h`, now the DDK headers).

## 1. CMD3D dword field map (fragment register block, in dwords)

The order and gating that the capture pinned. Four independent anchors
(background depth, `isp_ctl`, the first PBE word with its tile-relative bit,
and the render-target size in the second PBE word) fixed the layout
byte-exactly up to the first PBE word. Offsets are dwords from the start of
`RGXFWIF_CMD3D`, after the 4-dword shared header.

```
dw0..3   cmd_shared: hwrt_data_fw_addr and the common header
dw4      usc_pixel_output_ctrl
dw5..8   usc_clear_register[4]        (4 here, not ROGUE_MAXIMUM_OUTPUT_REGISTERS_PER_PIXEL = 8)
dw9      isp_bgobjdepth
dw10     isp_bgobjvals
dw11     isp_aa
         (isp_xtp_pipe_enable absent: no S7_TOP_INFRASTRUCTURE)
dw12     isp_ctl
         (tpu absent: no BRN 49927)
dw13     event_pixel_pds_info
         (pixel_phantom absent: no CLUSTER_GROUPING)
dw14     view_idx
dw15     event_pixel_pds_data         (EOT PDS data address)
dw16     isp_oclqry_stride            (GPU_MULTICORE_SUPPORT present)
         (isp_zls_pixels, rgx_cr_blackpearl_fix absent)
         pad to 8 bytes
dw18     isp_scissor_base      u64
dw20     isp_dbias_base        u64
dw22     isp_oclqry_base       u64
dw24     isp_zlsctl            u64
dw26     isp_zload_store_base  u64
dw28     isp_stencil_load_store_base u64
         (fb_cdc_zls absent: FBCDC algorithm 50, no FB_CDC_V4)
dw30     pbe_word[0][0..]      u64 pairs, one per render target
...      tpu_border_colour_table u64
...      pds_bgnd[3], pds_pr_bgnd[3]   u64 each (pds_bgnd_brn65101 absent)
         (isp_dummy_stencil/depth_store_base absent: BRN 62850/66193;
          rgnhdr_single_rt_size, rgnhdr_scratch_offset absent: BRN 67182)
...      flags (DEPTHBUFFER / STENCILBUFFER), later in the block
```

The capture was 80 dwords long and stopped inside the PBE region, so it could
not pin the size of the `pbe_word` array; the vendored header does. The
capture also showed the geometry command's front consistent with
`rogue_fwif_geom_regs` up to `vdm_ctrl_stream_base`, and the same feature and
BRN gating must be applied to the geometry block.

## 2. Where the fragment coefficient base comes from

The fragment coefficient (plane-equation) store is not a field of the 3D
command. It is a parameter-buffer allocation made by the PM for the
coefficient engine and read back during the 3D phase through the PM MMU.

- `RGX_PM_MAX_PB_VIRT_ADDR_SPACE = 0x400000000` (16 GiB) in `rgxdefs_km.h`.
- Our measured fault VA, 0x4c4000000 = 0x400000000 + 0xC4000000, lies 0xC4000000
  past the top of that space: an out-of-range PB virtual address, not a
  low sequential allocation. It matches none of the device heaps (general
  0x80..., PDS 0xDA..., USC 0xE0..., PMMETA 0xFA...).
- `req_status` tag 0x14 is MCU_PDS_USCA, the pixel-shader PDS/coefficient
  fetch; `cat_base=1` is the normal context. The PM catalog contexts are
  `RGX_CR_BIF_PM_CAT_BASE_VCE0 (0x1248)`, `TE0 (0x1250)`, `ALIST0 (0x1260)`.
- Moving `TA_STATE_PDS_VARYINGBASE` (a device VA) changed nothing, so the
  faulting base is the PM/PB coefficient allocation, not our PDS program
  address. Mapping a scratch page at the fault VA turned the BIF fault into a
  deadline lockup (the data there is garbage), which confirmed the base is
  real but invalid.
- 0x4c4000000 = 0x131 << 26 is bit-stable: a deterministic garbage value from
  an uninitialised base plus offset, produced on the TA/geometry and
  HWRTData/freelist/PM path, not in the 3D command block.

## 3. The fix, and what to check

1. Take the command-block layout for BVNC 36.53.104.796 from the vendored
   headers (section 0), so every register reaches the field the firmware reads.
2. Audit the geometry/PM path: the geometry register gating, and the freelist
   page-buffer base (it must resolve to 0, not 0x400000000) and sizes. The
   PM catalog bases (0x1248/0x1250/0x1260) are read in `pm_snap()` right after
   the TA kick to see whether the VCE pointer is the source of a bad address.

Confidence at the time: high on the ABI mismatch, high on the mechanism
(out-of-range PM/PB coefficient allocation on the TA path), medium on the exact
numeric origin of the fault address.
