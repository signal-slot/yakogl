# PowerVR Rogue/Volcanic (BXS-4-64, BVNC 36.53.104.796): SMOOTH vs FLAT vec4 varying

Read-only source analysis. Authoritative sources (Mesa `sailfishos-mirror/mesa @ main`):

- `src/imagination/csbgen/rogue/ppp.xml`   — PPP TA_STATE bit layout
- `src/imagination/csbgen/rogue/pds.xml`    — DOUTI descriptor bit layout
- `src/imagination/csbgen/rogue/vdm.xml`    — VDM state
- `src/imagination/include/hwdef/rogue_hw_defs.h` — coefficient-set layout
- `src/imagination/vulkan/pvr_arch_pipeline.c` `pvr_graphics_pipeline_setup_fragment_coeff_program()` — **builds every DOUTI word**
- `src/imagination/vulkan/pvr_arch_cmd_buffer.c` `pvr_setup_output_select()` (VARYING0/OUTPUT_SEL), `setup_pds_coeff_program()` (flat provoking-vertex patch), VDM_STATE0 emit
- `src/imagination/pco/pco_trans_nir.c` `trans_load_input_fs()` — FS-side coeff read (smooth vs flat)
- `src/imagination/pco/pco_data.h`, `pco.h` — vs/fs data structs

The DDK-25.2 kernel tree contains no state setup (that lives in the closed UM driver); Mesa is the open oracle.

---

## The two-sided model (this is the whole answer)

Smoothness is expressed in **two independent places that must agree**, plus perspective needs a **third** (the W plane):

1. **Geometry / tiler side — TA_STATE_VARYING0 (per-primitive plane generation).**
   The tiler computes the plane-equation coefficients **A, B** (the screen-space gradients) from **all 3 vertices** *only* for varyings placed in the **f32_linear** (perspective) or **f32_npc** (non-perspective) partition. For varyings in the **f32_flat** partition it writes **only C**, taken from the provoking vertex (A=B=0, per-primitive). **This is the control that makes v1/v2 enter the plane equation.**

2. **Fragment side — the DOUTI iterator word (per-fragment iteration).**
   `shademodel = GOURUAD` + `perspective` tell the ISP/USC iterator to evaluate `A*x + B*y + C` and perspective-divide. `FLAT_VERTEXn` tells it to just read C.

3. **Perspective W plane.** Smooth-perspective requires a dedicated RHW (1/w) iterator/plane; Mesa hard-asserts `fs_data->uses.w` for `INTERP_MODE_SMOOTH`.

If (1) says flat but (2) says GOURUAD, the coefficient store is also mis-partitioned, so the GOURUAD iterator reads a set whose A,B were never filled from v1,v2 → exactly the reported "gradient but only the provoking vertex (v0) contributes, v1/v2 = 0."

---

## Exact bit fields

### PPP `TA_STATE_VARYING0` (ppp.xml `STATE_VARYING0`)
| field | bits | flat vec4 | **smooth vec4** |
|---|---|---|---|
| `f32_linear` | [0:7]  | 0 | **4** |
| `f32_flat`   | [8:15] | 4 | **0** |
| `f32_npc`    | [16:23]| 0 | 0 (would be 4 for `noperspective`) |

Driver: `pvr_setup_output_select()` — `varying0.f32_linear = vs.f32_smooth; .f32_flat = vs.f32_flat; .f32_npc = vs.f32_npc`. `f32_*` are dword counts; a vec4 = 4. The vertex-output dwords for the varying must sit in the matching contiguous partition (linear block first, then flat, then npc), in vertex-output order after position.

### PPP `TA_OUTPUT_SEL` (ppp.xml `OUTPUT_SEL`)
- `vtxsize` [24:31] = `vs.vtxouts` — total vertex-output dwords (your 8). Must cover pos+colour so the tiler strides to v1/v2 correctly.
- `rhw_pres` [16] = `fs.uses.w` — **must be 1 for smooth-perspective** (emits the RHW plane).
- `tsp_unclamped_z_pres` [17] = `fs.uses.z`.

### DOUTI descriptor `DOUT_FIELDS_DOUTI_SRC` (pds.xml)
| field | bits | **smooth vec4** | flat vec4 |
|---|---|---|---|
| `f16_offset` | [0:7]   | = f32_offset | = f32_offset |
| `f32_offset` | [8:15]  | `(vtxout-2-!z-!w) * 2` | same |
| `perspective`| [16]    | **1** | 0 |
| `f16`        | [17]    | 0 | 0 |
| `size`       | [18:19] | **4D = 3** (`SIZE_1D + count-1`, count=4) | (Mesa flat asserts scalar) |
| `wrapu/v/s`  | [20:22] | 0 | 0 |
| `pointsprite`| [23]    | 0 | 0 |
| `shademodel` | [24:25] | **GOURUAD = 3** | FLAT_VERTEX0/1/2 = 0/1/2 |
| `primitiveid`| [26]    | 0 | 0 |
| `depthbias`  | [27]    | 0 | 0 |

`DOUTI_SHADEMODEL`: FLAT_VERTEX0=0, FLAT_VERTEX1=1, FLAT_VERTEX2=2, **GOURUAD=3**.
`DOUTI_SIZE`: 1D=0, 2D=1, 3D=2, 4D=3.

Driver builds these in `pvr_graphics_pipeline_setup_fragment_coeff_program()`:
```c
douti_src.f32_offset = vtxout * sizeof(uint16_t);   // vtxout = vs varying start, minus unused pos comps
douti_src.f16 = false;
case INTERP_MODE_SMOOTH:  douti_src.perspective = true;  /* FALLTHROUGH */
case INTERP_MODE_NOPERSPECTIVE: douti_src.shademodel = GOURUAD; break;
case INTERP_MODE_FLAT: BITSET_SET(flat_iter_mask, fpu); break;   // shademodel patched at draw
douti_src.size = ROGUE_PDSINST_DOUTI_SIZE_1D + count - 1;        // vec4 -> 4D
```
`vtxout` offset math (offset into the coefficient store, uint16 units):
```
vtxout = vs_data->varyings[slot].start;   // >= 4, i.e. after pos
vtxout -= 2;                 // pos.x, pos.y never iterated
if (!fs.uses.z) vtxout -= 1;
if (!fs.uses.w) vtxout -= 1;
f32_offset = vtxout * 2;
```
The Z iterator (if `uses.z`) uses `f32_offset = uses.w ? 2 : 0`; the W iterator uses `f32_offset = 0`. So the coefficient store front is **[W][Z][varyings…]** and your varying offset must be computed the same way, or GOURUAD reads the wrong set.

### Coefficient set layout (`rogue_hw_defs.h`) — per varying **component**
`A=0, B=1, C=2, PAD=3, SET_SIZE=4`. So each scalar occupies 4 coeff regs; a vec4 = 16.
- SMOOTH FS read (`trans_load_input_fs`): `build_itr(dest, coeffs[A,B,C…], wcoeffs)` — full plane + perspective divide by the W set.
- FLAT FS read: `pco_mov(dest, coeffs_index + SET_C)` — reads **only C**.

### VDM `VDM_STATE0.flatshade_control` [1:2] (vdm.xml) and PPP `PPP_CTRL.flatshade_vtx` [7:8]
`FLATSHADE_CONTROL`: VERTEX_0=0, VERTEX_1=1, VERTEX_2=2. Set purely from provoking-vertex/topology (`cmd_buffer` lines 7525, 8171). **They select which vertex is the provoking one for FLAT varyings only. For a SMOOTH varying they are don't-care** — they never gate whether all 3 vertices are read.

---

## Answers to the specific questions

**Q1 — marking smooth vs flat, every differing field (vec4):**
- `TA_STATE_VARYING0`: smooth → `f32_linear=4, f32_flat=0`; flat → `f32_linear=0, f32_flat=4`. (npc = `f32_npc` for `noperspective`.) **This is the per-vertex-vs-per-primitive storage control** — nothing else. There is **no separate "iteration count" / "number of vertices" register**; triangle primitive assembly always supplies 3 verts, and linear-vs-flat classification alone decides whether the tiler fits A,B from all 3 or copies C from the provoking vertex.
- `DOUTI`: smooth → `shademodel=GOURUAD(3), perspective=1, f16=0, size=4D(3), f32_offset=(vtxout)*2`; flat → `shademodel=FLAT_VERTEXn, perspective=0`.
- Perspective also needs `OUTPUT_SEL.rhw_pres=1` + the W DOUTI iterator + VS emitting RHW.

**Q2 — VDM `flatshade_control` / provoking vertex:** affects **flat varyings only**. It does not affect smooth interpolation. For a smooth gradient its value is irrelevant (Mesa still writes it from provoking vertex; leave it at VERTEX_0, it is ignored by linear varyings).

**Q3 — VS per-vertex storage:** the VS writes each varying per-vertex with `uvsw.write(src, vtxout_addr = range.start + component)` (pco `trans_store_output_vs`). `OUTPUT_SEL.vtxsize` (=`vs.vtxouts`) and `VDM_STATE4.vs_output_size` must cover all outputs so the tiler strides to v1/v2. Your `vo[0..3]=pos, vo[4..7]=colour, vtxsize=8` is correct for a smooth colour varying — **provided the vertex output also carries W/RHW** for the perspective plane, and provided `VARYING0.f32_linear=4` so those 4 colour dwords are in the linear partition. The varying does **not** need a different vo region for smooth vs flat; only the VARYING0 partition/count changes.

**Q4 — does the compiler emit a different VS for smooth vs flat?** **No.** The VS output emission (`pco_uvsw_write`) is byte-identical regardless of interpolation; there is no interp flag on the write and **no VS-side DOUTI** (DOUTI is entirely fragment-side, in the coefficient-loading PDS program). The only VS-side difference is metadata: the dword lands in the `f32_smooth` vs `f32_flat` count (→ VARYING0). **A plain passthrough VS (`vo[k]=vtxin[k]`) is sufficient for smooth** — smoothness is fully determined by VARYING0 + the FS DOUTI (+ the W plane). The USC/VS does not encode per-vertex vs flat.

---

## Ranked fix list (flat-storage path → true 3-vertex smooth)

1. **`TA_STATE_VARYING0.f32_linear` must count the vec4 (=4); `f32_flat` must NOT (=0).**
   This is the tiler's per-vertex plane-generation switch. While the varying stays in the flat partition the tiler only writes C from the provoking vertex — the exact "only v0 contributes" signature. Also ensure the colour's 4 vertex-output dwords fall inside the contiguous linear block (order: linear, then flat, then npc, after position).

2. **Stand up the perspective W plane.** `OUTPUT_SEL.rhw_pres=1`, VS must output RHW/1/w in the position output, and a dedicated W DOUTI iterator must exist (`shademodel=GOURUAD, size=1D, f32_offset=0`). Smooth-perspective divides by this plane; if it is absent/zero the non-provoking contributions collapse. (Mesa asserts `uses.w` for smooth.)

3. **Fix `DOUTI.f32_offset` to match the coefficient-store partition.** It must equal `(vtxout_start - 2 - !uses.z - !uses.w) * 2`, with W at offset 0 and Z next. A GOURUAD iterator with a flat-era offset reads a set whose A,B were never populated from v1,v2 → gradient that tracks only v0.

4. **DOUTI field values for the vec4:** `shademodel=GOURUAD(3)`, `perspective=1`, `f16=0`, `size=4D(3)`. (You have GOURUAD+perspective; verify `size=4D` and the offset — flat paths often leave size at 1D/scalar.)

5. **Strides:** `OUTPUT_SEL.vtxsize=8` and `VDM_STATE4.vs_output_size=8` must agree and cover all outputs, so the tiler reads v1/v2 at the right offset. `VDM_STATE0.flatshade_control` / `PPP_CTRL.flatshade_vtx` are irrelevant to smooth — do not chase them.

**Most likely THE control:** #1 (VARYING0 linear vs flat partition), tightly coupled with #3 (offset must follow the re-partitioned store) and #2 (W plane) for perspective correctness.
