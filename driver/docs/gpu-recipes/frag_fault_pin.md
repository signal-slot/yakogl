# 3D/fragment fault pinned (clean baseline 2026-09-12)

Bring-up note, cut to its measured facts and conclusion. Build:
`-DPVR_DDK_FW -DPVR_BIG_GLOBAL_FL -DPVR_DRAW_TRIANGLE`, TA tiling default.

## What was measured

The TA tiled correctly: `CAT_VCE0` showed the PM had allocated, and the TA
phase advanced. The 3D phase then faulted, and the fault survived the hardware
recovery (read from `hwrinfobuf`):

- HWR record for dm=3, type=3 (BIF fault); FWCCB `CONTEXT_RESET` for dm=3 with
  the same MMU status.
- Faulting VA 0x4c4000000 (about 19 GiB, outside the 16 GiB PB space: a garbage
  derived pointer).
- `req_status` tag 0x14 = MCU_PDS_USCA, the pixel-shader PDS/coefficient fetch.
- `mmu_status`: fault bit set, `cat_base=1` (a normal context, not the PM PB
  space).
- `pc` is a physical address inside our carveout (USC/PDS code), consistent
  with a pixel-PDS execution.

## Structural cause found

The drawn primitive's pixel shader and varying configuration come from the PPP
`TA_STATE` block the TA stores during tiling (in `pvr_ppp_build_clear_block`,
`driver/src/pvr_vdm_ppp_gen.c`), not from the fragment command's `pds_bgnd`
(which is only the background object for empty tiles). Our block declared one
four-dword varying per vertex in `VARYING0` (needed for the TA to tile) while
`VARYINGBASE` was 0, a dummy coefficient program. The pixel PDS expects to fetch
the iterated varying through the VARYINGBASE program, so it dereferenced a
garbage coefficient pointer. The shader's declared varyings and the primitive's
must agree.

Two ways out: build a real coefficient-load PDS plus a pixel USC that reads the
varying (the faithful one, and what Mesa does), or declare no varyings
consistently (which starved the TE varying store and broke tiling in an earlier
test).

## Confirmed: the coefficient base is hardware-computed

- Setting `VARYINGBASE` to a real coefficient program alone changed nothing; the
  fault was bit-identical. The coefficient source is therefore not the PDS
  VARYINGBASE program.
- Mapping a scratch 1 MiB page at VA 0x4c4000000 removed the type=3 BIF fault
  and replaced it with a type=0 deadline lockup (no status), with no pixels
  written. So the pixel/coefficient path does dereference a hardware-computed
  base at that address; the garbage data there makes the 3D hang instead of
  retire.
- 0x4c4000000 = 0x131 << 26 = 0x400000000 (the 16 GiB PB limit) + 0xC4000000,
  a base derived from an uninitialised field.

The fix is to give the coefficient path a valid base by correcting the
command-block and PM/freelist setup, not by the scratch mapping. See
`coeff_base_analysis.md` for the mechanism and `varying_interp_analysis.md` for
the PPP/PDS fields that govern varying iteration.
