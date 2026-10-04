# PowerVR (Mesa pvr) ground-truth for a 64x64 RGBA8888 red FILL on BXS-4-64

Target: BXS-4-64, BVNC 36.53.104.796 (`pvr_device_info_36_53_104_796`).
Case: `PVR_TRANSFER_CMD_FLAGS_FILL`, `dst = 64x64 VK_FORMAT_R8G8B8A8_UNORM`, linear,
stride 64 px, GPU vaddr `0x8000000000`, clear color red (1,0,0,1), sample_count 1,
scissor = full 64x64.

Method: exhaustive static trace of `pvr_3d_copy_blit_core` and everything it calls,
decoded against the compiled `csbgen/rogue` register XML and the BXS-4-64 feature
table. Approach (A) (run a harness) is **not reliably possible offline**: the fill
builder needs a live `pvr_transfer_ctx` (device heaps, frag store, GPU-uploaded EOT
and PDS BOs). The existing `capture_eot_usc.c` only links because it calls the pure
compiler path `pvr_usc_eot()` with no device. So approach (B) was used and is
authoritative here.

Relevant BXS-4-64 features (device_info/bxs-4-64.h):
- tile_size = 16x16, max_partitions = 16, usc_pixel_partition_mask = true
- usc_min_output_registers_per_pix = **2**, isp_samples_per_pixel = 4
- isp_max_tiles_in_flight = 6, num_clusters = 1, num_raster_pipes = 1
- gpu_multicore_support = true, roguexe = true, xt_top_infrastructure = FALSE
- simple_internal_parameter_format v2 = true, enhancement 42064 = true
- eight_output_registers = false

## Diff table: Mesa ground-truth vs your hand-built value

| Field | Mesa ground-truth | Your value | Match |
|---|---|---|---|
| isp_bgobjvals | 0x00000000 (bg disabled; NO enablebgtag for fill) | 0 | ✅ |
| isp_aa | 0x0 (AA_NONE, sample_count 1) | (0) | ✅ |
| usc_clear_register0 | 0xFF0000FF (U8U8U8U8 norm: R=FF,G=00,B=00,A=FF) | 0xFF0000FF | ✅ |
| usc_clear_register1..3 | 0 | 0 | ✅ |
| isp_render | 0x3 (mode_type=FAST_SCALE(3), dir_type=TL2BR(0)) | FAST_SCALE(3) | ✅ |
| isp_ctl | 0x00025000 (process_empty_tiles bit17 + pipe_enable=5) | 0x00025000 | ✅ |
| isp_mtile_size | 0x00040004 (x=4,y=4 tiles; 64/16=4) | 0x00040004 | ✅ |
| isp_render_origin | 0x00000000 | (0) | ✅ |
| usc_pixel_output_ctrl | 0x0007FFF8 (width=2REGISTERS(0), partition_mask=0xFFFF) | 0x0007FFF8 | ✅ |
| frag_screen | 0x003F003F (xmax=63,ymax=63) | 0x003F003F | ✅ |
| event_pixel_pds_info | 0x00000202 (const_size=2, temp_stride=0, usc_sr_size=1) | 0x202 | ✅ |
| pbe_wordx_mrty[0] (REG_WORD0) | 0x0200007C68800000 | same | ✅ |
| pbe_wordx_mrty[1] (REG_WORD1) | 0x0000003F0000003F | same | ✅ |
| pbe_wordx_mrty[2] (REG_WORD2) | 0x0000000000000000 | 0 | ✅ |
| pbe_wordx_mrty[3..8] | 0 (unused RT1/RT2 slots, still transmitted) | ? | verify |
| PBE emit STATE word0 | 0x00000000 (address_low: addr[2..33]=0) | 0 | ✅ |
| PBE emit STATE word1 | 0x00082020 (addr_high 0x20=bit39, norm, src_fmt 8/chan) | 0x82020 | ✅ |
| pds_bgnd0_base | 0 (no shader for fill) | 0 | ✅ |
| pds_bgnd1_base | 0 | 0 | ✅ |
| pds_bgnd3_sizeinfo | 0 (usc_sharedsize/uniform/texstate/temps all 0) | 0 | ✅ |
| isp_mtile_base | 0 (FastScale has NO control stream) | 0 | ✅ |
| isp_rgn | 0 (FastScale has NO control stream) | 0 | ✅ |
| PDS pixel-event code | {DOUTW64, DOUTU.end} ≈ {0xF0040101,0xF4000002} | same | ✅ |
| PDS pixel-event data | {eot_src0_lo, 0, emit0=0, emit1=0x82020, doutw=0x90000002} | same | ✅ |
| USC EOT | emitpix(sh0,sh1).freep; ret | 16B emitpix.freep end | ✅ |

**CONCLUSION: every register/word you listed is byte-for-byte identical to what
Mesa 26.2.1 produces. The bug is NOT in any register value.** The color, the PBE
words (including the 0x8000000000 destination encoded in STATE word1 low 6 bits =
0x20 = bit39), the clear regs, the ISP tile setup, the pixel-event PDS/USC — all
correct.

### Key decodings (so you can re-derive)
- **STATE_WORD0.address_low** start0 end31 **shift=2** → holds addr bits [2..33].
  0x8000000000 has bits[2..33]=0, so STATE word0 = 0 is CORRECT, not a bug.
- **STATE_WORD1.address_high** start0 end5 **shift=34** → holds addr bits [34..39].
  bit39 → field bit5 → 0x20. Plus norm(bit13=0x2000) + source_format 8_PER_CHANNEL
  (bit19=0x80000) = 0x82020. CORRECT.
- **usc_pixel_output_ctrl width field**: PIXEL_WIDTH enum is 2REG=0,4REG=1,8REG=2,
  1REG=3. Because usc_min_output_registers_per_pix=2, Mesa computes pixel_width=2 →
  width=2REGISTERS=**0**. partition_mask=(1<<max_partitions)-1=(1<<16)-1=0xFFFF<<3.
  So 0x0007FFF8 is exactly right; width is NOT a per-RT count field, it is a
  partition granularity, and 0 (=2 regs) is correct on this core.
- **isp_ctl 0x00025000**: process_empty_tiles=bit17=0x20000; pipe_enable(bits12-15)=
  total_tiles_in_flight-1 = 6-1 = 5 → 0x5000. valid_id=0, upass_start=0 (correct
  for transfer).
- **event_pixel_pds_info 0x202**: const_size=DIV_ROUND_UP(data_size=5dw,4)=2;
  usc_sr_size=DIV_ROUND_UP(rt_count(1)*PVR_STATE_PBE_DWORDS(2), unit16)=1 → 1<<9=0x200.

## The exact submit stream (ground truth for the FW path)

`pvr_submit_info_stream_init()` writes `cmd->fw_stream` in THIS order (this is what
the firmware parses; NOT the struct-declaration order):

1. KMD_STREAM_HDR (u64, 8 bytes) — `value.length = fw_stream_len` filled in last
2. pds_bgnd0_base (u64)
3. pds_bgnd1_base (u64)
4. pds_bgnd3_sizeinfo (u64)
5. isp_mtile_base (u64)
6. **pbe_wordx_mrty[9] (9 × u64 = 72 bytes)** ← ALWAYS 9 entries (3 RT × 3 words),
   even for a single-RT fill. Only [0..2] are non-zero; [3..8] = 0.
7. isp_bgobjvals (u32)
8. usc_pixel_output_ctrl (u32)
9. usc_clear_register0 (u32)
10. usc_clear_register1 (u32)
11. usc_clear_register2 (u32)
12. usc_clear_register3 (u32)
13. isp_mtile_size (u32)
14. isp_render_origin (u32)
15. isp_ctl (u32)
16. isp_aa (u32)
17. event_pixel_pds_info (u32)
18. event_pixel_pds_code (u32)
19. event_pixel_pds_data (u32)
20. isp_render (u32)
21. isp_rgn (u32)
22. frag_screen (u32) — present because gpu_multicore_support
    (isp_xtp_pipe_enable is NOT in the stream: xt_top/S7 infra absent)

The FWIF wrapper is `struct rogue_fwif_cmd_transfer { cmn; regs; uint32_t flags;
uint32_t padding; }`. For gpu_multicore_support, Mesa sets
**`cmd->flags |= ROGUE_FWIF_CMDTRANSFER_SINGLE_CORE (0x2)`** (via use_single_core).

## Base-address semantics (the crucial "what base" answers)

- **event_pixel_pds_code / event_pixel_pds_data are offsets from the PDS HEAP base**
  (`device->heaps.pds_heap->base_addr`), computed in `pvr_gpu_upload_pds()` as
  `bo.dev_addr - pds_heap.base_addr`. The HW/FW resolves them against the PDS exec
  base. So they are relative to your pds_exec_base **only if 0xDA00000000 IS the PDS
  heap base**. There is NO separate CR_EVENT_PIXEL_PDS_*_BASE register — the addr
  fields (shift=4) are pure offsets added to the global PDS base.
- data at BO offset 0, code at `code_offset = ALIGN_POT(data_aligned_size, code_align)`
  in the SAME BO → event_pixel_pds_data < event_pixel_pds_code. Do not swap them.
- **DOUTU src0 in the PDS data (your data[0]=eot_off&~3) is the USC EOT address
  relative to the USC HEAP base** (`usc_eot_bos.dev_addr - usc_heap.base_addr`),
  temps and sample_rate OR'd into the 64-bit src0. SAMPLE_RATE_INSTANCE=0 and, for
  "emitpix;ret", temps is tiny/0 → data[1]=0 is expected.
- pds_bgnd0/1 addresses are PDS-heap-relative too (0 here, so moot).

## Fill emit mechanism (confirms nothing else is needed)

No fragment shader. `process_empty_tiles=1` makes every (empty) tile fire the
end-of-tile pixel event. The pixel-event PDS program: (1) DOUTW writes the 2 PBE
STATE words {0, 0x82020} into common-store regs [0..1]; (2) DOUTU.end kicks the USC
EOT. The EOT does `emitpix(sh0, sh1)` — state from common store, **pixel data from
the USC output registers, which the HW pre-loads from CR_USC_CLEAR_REGISTER0..3 for
empty tiles**. So the clear color reaches the PBE automatically; isp_bgobjvals=0 and
pds_bgnd=0 are correct. No extra "num tiles / render-target-count / VALID_ID / CR_ISP
MEM" register is required for FastScale — there is deliberately no control stream,
no PPP/TE/VDM, no parameter buffer.

## Ranked most-likely reasons the PBE emits nothing

Symptom: FRAG DM scheduled, CCCB advances, timeline signals, RDM_FC2_FINISHED set,
**PIXELBE_END_RENDER NOT set**, no MMU fault, dst unchanged. That pattern means the
render/rasterization completed but the end-of-tile pixel-event never produced a valid
PBE emit — and it read only mapped memory (no fault). Since every register value is
correct, the fault is in *program placement / base resolution / stream layout*, not
values:

1. **event_pixel_pds_code/data resolve to the wrong memory** (base mismatch). If
   0xDA00000000 is not exactly the PDS-heap base your offsets were computed against
   (or the pixel-event PDS bytes were not actually uploaded at
   pds_base + event_pixel_pds_code), the pixel event fetches mapped-but-wrong/zero
   PDS → runs as NOPs → no DOUTU → no USC emit → PIXELBE_END_RENDER never set, no
   fault, dst untouched. **Best fit for the exact symptom.** Verify: read back the
   bytes at (your PDS base + event_pixel_pds_code) and confirm they equal
   {0xF0040101,0xF4000002}, and at +event_pixel_pds_data the 5 data dwords.

2. **DOUTU EOT address base mismatch.** data[0]=eot_off must be the USC EOT offset
   from the USC-CODE heap base the HW uses (CR_USC_CODE_BASE / usc_heap). If your
   USC base differs, the DOUTU kicks a wrong/zero USC → no emitpix. Verify the USC
   base register equals the base your eot_off was subtracted from, and that the 16 EOT
   bytes actually live there.

3. **fw_stream layout / size mismatch (if you build the FW stream rather than poking
   CRs directly).** Most common concrete slip: emitting only 3 pbe_wordx_mrty u64s
   instead of the full **9** (72 bytes). That shifts every subsequent u32
   (isp_bgobjvals … event_pixel_pds_code/data … frag_screen) → the FW reads garbage
   event_pixel_pds_code/data → same no-emit symptom. Also check: 64-bit block first
   then 32-bit block (order above), frag_screen present, isp_xtp_pipe_enable absent,
   KMD_STREAM_HDR.length correct.

4. **Missing ROGUE_FWIF_CMDTRANSFER_SINGLE_CORE (0x2) in cmd->flags.** On this
   multicore part Mesa always sets it for transfer. Without it the fragment/PBE can
   be dispatched such that the emit path is mis-scheduled across cores.

5. **Cache/coherency: PDS program, USC EOT, or clear-reg-bearing state not flushed
   to memory (SLC) before the kick,** so the GPU fetches stale/zero. Fits "no fault,
   no emit". Flush/clean the PDS + USC BOs (and any CPU-written state) before submit.

6. (Lower) **common-store index agreement.** The DOUTW writes state to common-store
   [0..1]; the EOT `load_preamble` base must be 0/1 (state_regs[0]=0). If your EOT
   was compiled with shared_words=false (immediates) or a different base, sh0/sh1 are
   wrong. Mesa uses shared_words=true, state_regs[0]=0. Your "emitpix sh0,sh1" implies
   this is fine, but confirm the EOT truly sources sh0/sh1 (not immediates/o-regs).

### First diagnostic to run
Dump the actual bytes the GPU would fetch at:
`PDS_base + event_pixel_pds_code` (expect the 2 code dwords) and
`PDS_base + event_pixel_pds_data` (expect the 5 data dwords incl. 0x82020 and
0x90000002), and `USC_base + (data[0] & ~3)` (expect the 16-byte EOT). If any is
zero/wrong → cause #1/#2/#3. This single check discriminates the top three.
