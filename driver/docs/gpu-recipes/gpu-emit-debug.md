# PowerVR BXS-4-64 TQ_3D FastScale FILL — "completes but no pixels" root-cause analysis

BVNC 36.53.104.796, TI AM62P, bare-metal Zephyr UMD.
Symptom: TQ_3D FILL is SUBMITTED, scheduled on FRAG DM, CCCB UPDATE signals timeline UFO=1,
but the destination surface still holds the CPU prefill 0xA5A5A5A5 (expected 0xFF0000FF).

All line refs are into the local Mesa 26.2.1 checkout
(`mesa-build/mesa-26.2.1/src/imagination/...`) and `pvr-src/` (ported kernel driver).

---

## ROOT CAUSE (rank 1, near-certain): PBE emit STATE_WORD0 encodes the WRONG destination address

The destination base address is carried **only** in the PBE emit STATE words
(`emit_words[0]`/`emit_words[1]` = "pbe_word0"/"pbe_word1"), which live in the PDS
pixel-event **data segment** at dword[2]/dword[3]. It is NOT in `pbe_wordx_mrty`
(those REG words carry only memlayout/stride/swizzle/clip — verified below).

Generated packer (`build/.../rogue/pbestate.h:543`, helper `pvr_packet_helpers.h:162`):

    STATE_WORD0.address_low = (addr >> 2) & 0xFFFFFFFF      (start=0, shift=2)
    STATE_WORD1.address_high = (addr >> 34) & 0x3F          (start=0, shift=34)

For the **actual** destination `0x8000000000` (GENERAL heap, offset 0):

    pbe_word0 = (0x8000000000 >> 2) & 0xFFFFFFFF = 0x00000000   <-- MUST be 0
    pbe_word1  address_high = (0x8000000000 >> 34) & 0x3F = 0x20

The user is submitting **pbe_word0 = 0x04000000**. Decoding that:

    0x04000000 << 2 = 0x10000000, OR'd with (0x20 << 34) = 0x8000000000
      => effective PBE destination = 0x8010000000

That is the address from the **spec/capture example**, which was written for dst
`0x8010000000` (see `gpu-cmd-final.md:5,241-242` and `gpu-usc-pds-spec.md:310,314`).
pbe_word1 is identical for both addresses (0x10000000 = bit28, below the bit34 address_high
field), which is why word1 "looks right" and masks the bug.

Net effect: the render runs to completion, the PBE happily writes red — 256 MiB away
at `0x8010000000`, which is not the surface the CPU prefilled/reads back. Job completes,
UFO signals, dst at 0x8000000000 stays 0xA5A5A5A5. This matches the symptom exactly.

### FIX
Set the PBE emit STATE_WORD0 for the real destination:

    emit_words[0] = pbe_word0 = (dst_vaddr >> 2) & 0xFFFFFFFF          // = 0x00000000 for 0x8000000000
    emit_words[1] = pbe_word1 = keep 0x00082020 (address_high 0x20 unchanged for 0x8000000000)

i.e. PDS pixel-event **data-segment dword[2] must become 0x00000000** (dword[3] stays 0x00082020).
General formula (always recompute from the live dst, never hardcode from the spec):

    pbe_word0 = (dst >> 2) & 0xFFFFFFFF
    pbe_word1 = (existing non-address bits) | ((dst >> 34) & 0x3F)

NB: `pbe_wordx_mrty` (the transfer_regs REG words) do NOT need the address and are fine
as-is — see below. Only the EOT emit words carry the base.

---

## Answers to the specific questions

### Q1 — Does FastScale fill emit via process_empty_tiles + clear registers? Is isp_bgobjvals=0 correct?
YES and YES. In `pvr_3d_copy_blit_core` the FILL branch (`pvr_arch_job_transfer.c:2828-2875`):
- explicitly "No background enabled": `pvr_csb_pack(&regs->isp_bgobjvals, CR_ISP_BGOBJVALS, reg)` with
  no fields => **isp_bgobjvals = 0 is correct**. Do NOT set enablebgtag for a fill
  (that is only for the source-copy branch, line 2999-3001).
- No shader / no background object => `pds_bgnd0/1/3 = 0` is correct
  (`pvr_setup_hwbg_object`, line 651-698, packs all zeros when
  pds_shader_task_offset/uni_tex_code_offset/tex_state_data_offset/common_ptr/... are 0).
- The clear color goes into `usc_clear_register0..3` (line 2856-2869). 0xFF0000FF is the
  correct little-endian pack for R8G8B8A8_UNORM red (R=FF,G=00,B=00,A=FF).
- Emit path: `isp_ctl.process_empty_tiles=true` (line 1451) makes the ISP walk every tile;
  the HW loads the CLEAR registers as the tile's output; the **EOT pixel-event PDS program**
  (DOUTW the PBE state pair into the Common Store, then DOUTU END to kick the EOT USC's
  `emitpix`) writes to memory using the PBE state = STATE_WORD0/1 (the address) + the
  `pbe_wordx_mrty` REG words (layout/stride/swizzle). `isp_render=FAST_SCALE, disable_eomt=0`
  (0x3) is correct — EOMT emit is enabled.
usc_clear_register0 is consumed by the HW background/empty-tile path as the pixel value; the
destination address it is written to comes from the emit words. So the color source is fine;
only the address was wrong.

### Q2 — event_pixel_pds_data/code semantics + alignment; is offset 0 valid?
- cr.xml (`csbgen/rogue/cr.xml:292-298`): both CODE and DATA `addr` are `start=4 end=31 shift=4`.
  With the packer `((addr>>4)<<4) & mask` the shift and start cancel: the register stores the
  **raw byte offset with the low 4 bits forced to 0**. So your raw byte offset is correct —
  there is NO extra >>4 to apply. Requirement: **16-byte aligned** (ROGUE_CR_EVENT_PIXEL_PDS_*_ADDR_ALIGNMENT).
- **offset 0 is valid** (it means "PDS-heap base"); it is NOT treated as "no program". The FW
  knows there is a data segment because `event_pixel_pds_info.const_size != 0`.
- `event_pixel_pds_info = 0x00000202` decodes correctly (cr.xml:301-311):
  const_size=2 (=DIV_ROUND_UP(5 data dwords,4)), temp_stride=0, usc_sr_size=1
  (=DIV_ROUND_UP(1*PVR_STATE_PBE_DWORDS(2),16)). Consistent with 1 RT / 5-dword data seg.
- Make sure DATA at 0 and CODE at its offset do not overlap: data seg is 5 dwords = 20 bytes,
  so CODE must be at >= 32 (next 16-aligned). Confirm your CODE offset field is the byte offset.
Verdict: not the bug, but keep both 16-byte aligned.

### Q3 — Is `struct rogue_fwif_cmd_transfer` the right CCCB payload? Does the kernel add anything?
YES, verbatim. The FW consumes `struct rogue_fwif_cmd_transfer` = `cmd_common` (8) +
`struct rogue_fwif_transfer_regs regs` (176) + flags(4) + padding(4) = **192 bytes**
(`pvr_rogue_fwif_client.h:317-369`). This is the *deserialized* struct, NOT the Mesa
"KMD_STREAM" byte-stream (`pvr_submit_info_stream_init`, transfer.c:5809-5899) — that stream
is what Mesa hands the *kernel*, which pvr_stream.c re-packs into this struct. Since you build
the struct directly, that is correct.
CCCB packing for a transfer job (`pvr_queue.c:678-737`) is exactly:
  [optional FENCE_PR for unsignaled deps] -> [TQ_3D cmd] -> [UPDATE ufo].
No sub-command count, no terminate marker, no geom/frag split for a pure transfer. Your CCCB
layout `[TQ_3D hdr][cmd_transfer 192][UPDATE hdr][ufo{addr,1}]` is correct.
CAUTION on struct layout (verify your hand-built struct matches C exactly):
`struct rogue_fwif_transfer_regs` field order is 32-bit block first
(isp_bgobjvals, usc_pixel_output_ctrl, usc_clear_register0..3, isp_mtile_size,
isp_render_origin, isp_ctl, **isp_xtp_pipe_enable**, isp_aa, event_pixel_pds_info,
event_pixel_pds_code, event_pixel_pds_data, isp_render, isp_rgn, frag_screen) = 17 u32 = 68B,
padded to 72, then aligned_u64 block (pds_bgnd0_base, pds_bgnd1_base, pds_bgnd3_sizeinfo,
isp_mtile_base, pbe_wordx_mrty[9]) = 176B total. The field `isp_xtp_pipe_enable` is ALWAYS
present in the struct (no #ifdef) even though only used with S7_TOP_INFRASTRUCTURE — if you
omitted it you would shift the entire u64 section by 4 bytes and corrupt pds_bgnd*/pbe words.
Given the job completes cleanly this is probably right, but confirm it.

### Q4 — Could offset 0 be treated as null (dst/pds/usc)?
No null-gating problem here:
- dst at 0x8000000000 is non-null (high bits set); the *fix* makes STATE_WORD0=0 but the full
  40-bit address (with STATE_WORD1 high=0x20) is 0x8000000000, valid.
- PDS data at offset 0 is valid (see Q2).
- USC EOT at offset 0 is valid: DOUTU exe offset is USC-heap-relative; 0 => usc_exec_base
  (0xE000000000). data[0]=eot_off&~3=0, data[1]=0 (temps=0) are correct.
The real issue is the OPPOSITE of a null: STATE_WORD0 is non-zero when it should be zero.

### Q5 — Required KCCB command / nonzero context field before the kick?
The job reaches the FRAG DM and signals the UFO, which proves the transfer context is
registered and the DM executes (not no-op). No pre-kick cache-flush/healthcheck is needed for
the render to run. So Q5 is not the blocker. (If it had not scheduled at all, that would be a
different failure mode than the one observed.)

---

## Other registers — all decode as correct (not the bug)
- isp_render 0x00000003 => mode_type=FAST_SCALE(3), dir_type=0, resume=0, disable_eomt=0. OK.
- isp_ctl 0x00025000 => process_empty_tiles + tiles-in-flight. OK.
- frag_screen 0x003F003F => xmax=63, ymax=63 (64x64). OK.
- pbe_wordx_mrty[0]=0x0200007C68800000 => REG_WORD0: memlayout=LINEAR, tilerelative=1,
  identity swizzle, linestride field set. Carries NO base address. OK.
- pbe_wordx_mrty[1]=0x0000003F0000003F => REG_WORD1: maxclip_x=63, maxclip_y=63 (64x64). OK.
- pbe_wordx_mrty[2]=0 (enh42064 word). OK.
- PDS data seg [eot&~3,0,pbe_word0,pbe_word1,0x90000002] matches
  pvr_pds_generate_pixel_event_tq layout exactly; ctrl word 0x90000002 =
  LAST_EN|DEST_COMMON_STORE|BSIZE_ALL64 (no SLC bypass bit -> slc_mcu_cache_controls absent). OK.
- USC EOT (16B) is address-agnostic (address comes only from the emit words). OK.

---

## On-hardware diagnostics to disambiguate (CR offsets, pvr-src/pvr_rogue_cr_defs.h)
1. Confirm the write went to the wrong address:
   - `ROGUE_CR_BIF_FAULT_BANK0_MMU_STATUS` = **0x12B0** — FAULT bit0; if 0x8010000000 is
     unmapped you'll see a page fault here after the kick.
   - `ROGUE_CR_BIF_FAULT_BANK0_REQ_STATUS` = **0x12B8** — the faulting ADDRESS field; expect
     it to read back ~0x8010000000 (the mis-encoded PBE target), which directly proves rank 1.
   - `ROGUE_CR_EVENT_STATUS` MMU_PAGE_FAULT bit9 (0x200) also latches a page fault.
   If 0x8010000000 happens to be *mapped* (large GENERAL heap) there will be NO fault — the red
   pixels just land there. Read back memory at 0x8010000000: it should now contain 0xFF0000FF.
2. Confirm the TQ_3D was actually consumed (it was, since UFO=1): read the CCCB
   read_offset (client CCB ctl) and compare to write_offset=280.
3. Read the FW trace buffer / FWCCB for any TDM/FRAG error (none expected — clean completion).

## Verification after fix
Set PDS data-seg dword[2] (pbe_word0) = 0x00000000, resubmit, read back 0x8000000000:
it should become 0xFF0000FF across the 64x64 surface.
