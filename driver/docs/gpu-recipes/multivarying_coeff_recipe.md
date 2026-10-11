<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Multi-varying PDS coefficient-loading (DOUTI) recipe — V1 + F4 (two smooth varyings)

Read-only source analysis. Target: BXS-4-64, BVNC **36.53.104.796**, Mesa arch ROGUE.
Goal: run the pco_shc-compiled **V1_passthrough** (VS) + **F4_multivary** (FS) pair on the hand-written
driver so the FS reads two **perspective-correct (smooth)** varyings and interpolates across all 3 vertices.

## Authoritative sources (file:line)
- Coeff/DOUTI descriptor build: Mesa 26.2.1 `src/imagination/vulkan/pvr_arch_pipeline.c:1486-1616`
  `pvr_graphics_pipeline_setup_fragment_coeff_program()`.
- PDS program (DATA+CODE) layout: Mesa 26.2.1 `src/imagination/vulkan/pds/pvr_pds.c:4136-4230`
  `pvr_pds_coefficient_loading()`.
- DOUT encoders: `pvr_rogue_pds_encode.h:514` (`pvr_pds_inst_encode_dout`), `pvr_pds.c:225`
  (`pvr_pds_encode_douti`); shifts in `pvr_rogue_pds_defs.h` (OPCODE 28, DST 0, SRC0 8, END 26,
  DOUTI_SRC0 ISSUE0 0 / DEST 54 / ISSUE(last) 63; DSTDOUT_DOUTI=4).
- PPP fields: `csbgen/rogue/ppp.xml` OUTPUT_SEL:194 (vtxsize[24:31], rhw_pres[16]),
  STATE_VARYING0:220 (f32_linear[0:7], f32_flat[8:15], f32_npc[16:23]),
  STATE_PDS_SIZEINFO1:301 (usc_varyingsize[4:9] UNIT=16, pds_varyingsize[10:15] UNIT=4).
- DOUTI descriptor bits: `csbgen/rogue/pds.xml:81-132` (shademodel[24:25] GOURUAD=3, size[18:19]
  1D=0/2D=1/3D=2/4D=3, perspective[16], f32_offset[8:15], f16_offset[0:7]).
- Coeff set size: `include/hwdef/rogue_hw_defs.h:171-177` `ROGUE_USC_COEFFICIENT_SET_SIZE = 4`
  (A,B,C,PAD).
- Output-select emit: `pvr_arch_cmd_buffer.c:6562-6639` (`pvr_setup_output_select`), SIZEINFO1 emit
  `:7359-7411`.
- Driver current single-varying path: `powervr-bxs-am62p-geom/src/pvr_fw_boot.c:2748-2761`
  (`coeff_prog[5]`), `src/pvr_vdm_ppp_gen.c:291-318` (OUTPUT_SEL / VARYING0), `:219-233` (SIZEINFO1),
  vertex fetch `pvr_pds_render_gen.c:266`.

## Shader layout being wired (from meta)
- **V1** (`V1_passthrough.meta.txt`): temps=0, vtxins=10, **vtxouts=10**, **f32_smooth=6** / flat=0 / npc=0.
  vtxin a_pos@vi0..3, a_color@vi4..7, a_uv@vi8..9. vs_out POS@0(4), VAR0@4(4), VAR1@8(2).
- **F4** (`F4_multivary.meta.txt`): temps=10, **coeffs=28**, uses z=0 **w=1**.
  fs_in POS(z/w)@cf0..3 (1 set), VAR0@cf4..19 (4 sets), VAR1@cf20..27 (2 sets).
  Disasm reads `ditrp.pixel cf4..19, cf0..3` and `cf20..27, cf0..3` — the `cf0..3` operand is the
  RHW/W plane used for the perspective divide, so the W iterator is mandatory for this binary.

---

## 1. PDS coefficient-loading program (DOUTI) — bind at TA_STATE_PDS_VARYINGBASE

Iterator order (from the oracle): z(none) → **W** → pntc(none) → **VAR0** → **VAR1**.
`num_fpu_iterators = 3` → `data_size = ALIGN(2*3,4) = 8` dwords, `code_size = 3` dwords, **11 dwords total**.
Bind VARYINGBASE at the program start (DATA first; CODE auto-follows at +data_size = +0x20), exactly as
the working single-varying path does.

### Per-iterator DOUTI descriptor (the FPU_iterators word = low 32 bits of each 64-bit DATA word)

| iter | slot | count | shademodel | perspective | size | f32_offset (=f16_offset) | **descriptor** |
|---|---|---|---|---|---|---|---|
| 0 | W (RHW) | 1 | GOURUAD(3) | 0 | 1D(0) | 0 | **0x03000000** |
| 1 | VAR0 v_color vec4 | 4 | GOURUAD(3) | **1** | 4D(3) | **2** (0x02) | **0x030D0202** |
| 2 | VAR1 v_uv vec2 | 2 | GOURUAD(3) | **1** | 2D(1) | **10** (0x0A) | **0x03050A0A** |

f32_offset derivation (`pvr_arch_pipeline.c:1568-1585`, doc rule `f32_offset=(vtxout-2-!z-!w)*2`):
- VAR0: vtxout_start=4 → 4−2(pos.xy)−1(!z, z unused)−0(w USED, not subtracted) = 1 → ×2 = **2**.
- VAR1: vtxout_start=8 → 8−2−1−0 = 5 → ×2 = **10 (0xA)**.
- W: `f32_offset=0`, size=1D, shademodel=GOURUAD, no perspective bit (`:1520-1531`).

Field packing check (pds.xml): VAR0 = shademodel3<<24 | size3<<18 | persp1<<16 | 2<<8 | 2
= 0x03000000|0x000C0000|0x00010000|0x0200|0x02 = **0x030D0202**.
VAR1 = 0x03000000|size1<<18(0x00040000)|0x00010000|0x0A00|0x0A = **0x03050A0A**.
(Note VAR1 size is **2D=1**, not 3D — `size = SIZE_1D + count-1 = 0+2-1 = 1`.)

### 64-bit DATA words (little-endian: dword[2i]=low, dword[2i+1]=high)
`iter_word = descriptor | (dest<<54) | (last?1<<63:0)`, `dest = cf_start/4`
(W dest=0→cf0..3, VAR0 dest=1→cf4.., VAR1 dest=5→cf20..). dest<<54 → high32 = dest<<22; last → high bit31.

### Full program (11 dwords)
```c
static const uint32_t coeff_prog[11] = {
    /* DATA (8 dw) */
    0x03000000u, 0x00000000u,   /* iter0 W    -> cf0..3   (dest set0, GOURUAD 1D, persp=0, off=0)   */
    0x030D0202u, 0x00400000u,   /* iter1 VAR0 -> cf4..19  (dest set1, GOURUAD 4D, persp=1, off=2)   */
    0x03050A0Au, 0x81400000u,   /* iter2 VAR1 -> cf20..27 (dest set5, GOURUAD 2D, persp=1, off=10, LAST) */
    0x00000000u, 0x00000000u,   /* pad to 128-bit boundary (ALIGN_POT(6,4)=8) */
    /* CODE (3 dw): DOUTI, SRC0 = constant_index>>1 (0,1,2), END on last */
    0xF0000004u,                /* DOUTI src0=0  (iter0) */
    0xF0000104u,                /* DOUTI src0=1  (iter1) */
    0xF4000204u,                /* DOUTI src0=2  (iter2) + END (bit26) */
};
```
CODE derivation (`pvr_pds.c:4212-4219`): `encode_douti(cc=0,end=0,src0=const>>1)` =
`0xF<<28 | DOUTI(4)<<0 | src0<<8` = 0xF0000004 / 0xF0000104 / 0xF0000204; last |= 1<<26 →
**0xF4000204**. (Cross-check: single-iterator path is `0xf4000004` = this with src0=0 and END — identical
mechanism, `pvr_fw_boot.c:2759`.)

DATA cross-check vs the working single npc word (`coeff_prog[5]={0x030c0000,0x80000000,...}`): that is
one iterator with descriptor 0x030c0000 (GOURUAD 4D npc off0), dest0, LAST(0x80000000 high). Same
encoding rules, so this 3-iterator extension is mechanically identical, only more iterators + the W plane.

---

## 2. PPP TA_STATE_VARYING0 / 1 / 2

`pvr_setup_output_select` (`:6618-6633`): `f32_linear = vs.f32_smooth (=6)`, `f32_flat = 0`,
`f32_npc = 0`; VARYING1 = f16_* = 0. All 6 smooth f32 components go in **f32_linear** (they are
perspective/smooth), and the 6 vertex-output varying dwords (color@4..7, uv@8..9) are already contiguous
right after position, satisfying the linear-partition-first rule.

- **TA_STATE_VARYING0 = 0x00000006**  (f32_linear=6, f32_flat=0, f32_npc=0)
- **TA_STATE_VARYING1 = 0x00000000**
- **TA_STATE_VARYING2 = 0x00000000**

The RHW is **not** signalled in VARYING0 — it is signalled by `OUTPUT_SEL.rhw_pres=1` (§3) plus the W
DOUTI iterator (§1). VARYING0 only classifies the *user* varyings into linear/flat/npc partitions.

---

## 3. TA_OUTPUT_SEL

`pvr_setup_output_select` (`:6585-6589`): `rhw_pres = fs.uses.w (=1)`, `tsp_unclamped_z_pres =
fs.uses.z (=0)`, `vtxsize = vs.vtxouts (=10)`, all plane/psprite/viewport = 0.

- **TA_OUTPUT_SEL = 0x0A010000**  (vtxsize=10<<24 | rhw_pres=1<<16)

(vs. the working single-varying word 0x08000000 = vtxsize=8, rhw_pres=0.)

### SIZEINFO1 (load-bearing side-effect of the coeff program — not in the deliverable list but required)
`pvr_arch_cmd_buffer.c:7359-7409`:
- `pds_varyingsize = ceil(coeff_prog.data_size / 4) = ceil(8/4) = 2` → bits[10:15]=2 → 0x00000800.
- `usc_varyingsize = ceil(fs.coeffs / 16) = ceil(28/16) = 2` → bits[4:9]=2 → 0x00000020.
- **SIZEINFO1 varying bits = 0x00000820** (with texturestate=temp=uniform=0, matching the GRADIENT no-color-upload path). The current driver hardcodes 0x410 (both =1) for one varying; change to
  `pvr_u(2u,4) | pvr_u(2u,10)` = **0x820** for this pair. SIZEINFO2 usc_sharedsize=0.

---

## 4. VDM vs_output_size

- `vs_output_size_bytes = vtxouts * 4 = 10 * 4 = **40**`.
- VDM_STATE4 `vs_output_size` field = `DRU(40,4) = **10**` (unit = 4 bytes, `pvr_vdm_ppp_gen.c:90`).
- CAM size / max-instances recompute automatically from 40 via
  `pvr_vdm_calc_cam_size_bxs4_64(40, raster=1, …)`.
- UVSW implication: the VS emits **10** vertex-output dwords (pos@0..3, color@4..7, uv@8..9); OUTPUT_SEL.vtxsize
  (=10) and VDM vs_output_size (=10) MUST both be 10 so the tiler strides to v1/v2 correctly. V1 already
  emits exactly this (disasm `uvsw.write ...,0x4 / 0x8 / 0x0`, then `uvsw.emit.endtask`).

---

## 5. Vertex buffer + vertex-fetch

`pvr_pds_render_clear_vertex_program(vs_usc_exec_offset, vs_temps, vertex_buf_addr, vertex_stride,
vertex_dma_size, vertex_dst_reg, has_slc_mcu_cache_controls, data, code, info)`
(`pvr_pds_render_gen.c:266`) issues **one contiguous DMA** of `vertex_dma_size` bytes (stride
`vertex_stride`) into vertex-input regs starting at `vertex_dst_reg`.

Pack **10 floats/vertex** = `[pos.x,pos.y,pos.z,pos.w, color.r,g,b,a, uv.x,uv.y]` (40 bytes). A single
40-byte DMA at dst_reg 0 lands a_pos@vi0..3, a_color@vi4..7, a_uv@vi8..9 — exactly V1's vtxin layout
(fmt=24 = float32 vec, `V1_passthrough.meta.txt`).

Exact call (V1 temps=0):
```c
pvr_pds_render_clear_vertex_program(
    /* vs_usc_exec_offset      */ <V1_passthrough USC heap offset>,
    /* vs_temps                */ 0,          /* V1 temps=0 */
    /* vertex_buf_addr         */ vtx.gpu,    /* GPU VA of the packed vertex buffer */
    /* vertex_stride           */ 40,
    /* vertex_dma_size         */ 40,
    /* vertex_dst_reg          */ 0,          /* -> vi0.. */
    /* has_slc_mcu_cache_ctrls */ 0,
    vdata, vcode, &vinfo);
```
The DOUTU in this program must point at the V1 USC with **vs_temps=0** (not the passthrough clear VS's
temp count). Vertex count = 3 for one triangle.

---

## 6. Perspective (smooth) vs npc — recommendation

Because every vertex has **w = 1.0**, RHW = 1/w = 1 everywhere, so the perspective divide is the identity:
smooth-perspective interpolation is **numerically identical to npc** for this data. So both paths give the
same pixels here.

Two ways to get there:

**(A) Smooth path (this recipe) — RECOMMENDED.** Matches the already-compiled binaries exactly: F4
reads the W plane (`uses.w=1`, `ditrp … cf0..3`) and V1 reports `f32_smooth=6`. No recompile, no
VS/FS/driver mismatch. Cost: it exercises the W iterator + `rhw_pres=1`, which is new relative to the
proven single-npc path. Risk is contained: the DATA/CODE mechanics are the same generator the working
path already validated; only the W plane and the extra iterators are new.

**(B) noperspective/npc path — fallback only.** Would drop the W plane (VARYING0.f32_npc=6,
OUTPUT_SEL.rhw_pres=0, no W iterator, 2 iterators) and reuse the *proven* npc machinery. BUT it requires
**recompiling both V1 and F4 with `noperspective` qualifiers** — F4 must be rebuilt so `uses.w=0` and it
stops emitting the `cf0..3` divide; the coeff allocation then shifts (VAR0 to cf0.., no POS set), so the
exact descriptors/offsets/dests must be re-derived from the *recompiled* metadata (not the numbers above).
Feeding the current smooth F4 binary an npc PPP config (no W plane) would leave its `ditrp cf0..3` divide
reading an un-iterated set → collapse toward the provoking vertex.

**Recommendation:** use **(A)** as primary — it is the lower-risk match to the shipped binaries and also
the only path that generalizes to real perspective (w≠1) later. Keep **(B)** as a fallback *only if* the
W plane misbehaves on silicon, and in that case recompile the pair noperspective and regenerate the
descriptors from the new meta.

---

## Confidence & on-silicon checks
- **High confidence** on every dword: each is byte-derived from the production oracle
  (`pvr_pds_coefficient_loading` + `setup_fragment_coeff_program` + `pvr_setup_output_select`) and
  cross-checked to csbgen ppp.xml/pds.xml and to the working single-npc program's encoding.
- **Must verify on silicon (unproven on this driver):**
  1. The **W/RHW plane** end-to-end (`rhw_pres=1` + W DOUTI + the FS `cf0..3` divide) — the working path
     never used it. This is the single biggest new element.
  2. **Multi-iterator DATA constant packing** (dest at bits 54.., LAST at 63, SRC0=const>>1 in CODE) with
     3 iterators — validated by source but only ever run with 1 iterator here.
  3. **SIZEINFO1 = 0x820** (usc_varyingsize=2, pds_varyingsize=2) actually reserving cf0..27; if the FS
     reads cf20..27 as zero, bump/verify these.
  4. V1 fed with **vs_temps=0** in the vertex-fetch DOUTU and a **40-byte** contiguous vertex.
- Suggested bring-up: draw one triangle with distinct per-vertex v_color and v_uv=(0,0)/(1,0)/(0,1),
  expect a smooth RGB gradient modulated by uv; capture via the panel document camera.
