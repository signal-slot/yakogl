<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Gouraud varying interpolation on BXS-4-64 (Volcanic): root cause and fixes

Bring-up note, cut to its conclusions. The first version decoded words of a
captured DDK stream; those words are not reproduced. Every field layout below
is from Mesa's csbgen XML and its Vulkan driver, and the values are computed
from those fields.

Sources: Mesa `src/imagination/csbgen/rogue/{ppp,vdm}.xml`,
`csbgen/{rogue,volcanic}/pds.xml`, `vulkan/pds/pvr_pds.c`,
`vulkan/pvr_arch_cmd_buffer.c`. BXS-4-64 is Volcanic. In Mesa the PPP/VDM
control-stream layout is the shared `csbgen/rogue/{ppp,vdm}.xml` (there is no
volcanic ppp.xml); PDS has a Volcanic-specific `csbgen/volcanic/pds.xml`, but
the DOUTI/DOUTU field layouts are identical.

## Why the panel was black, and the fix ladder

A triangle drawn with a coefficient program but with
`TA_STATE_PDS_SIZEINFO1.usc_varyingsize = 0` and `pds_varyingsize = 0` shows
black: the USC is told it has no coefficient store, so the iterated varying
has nowhere to land and the fragment shader reads 0. Our generator packed only
`pds_texturestatesize | pds_tempsize`.

1. SIZEINFO1: set `usc_varyingsize` and `pds_varyingsize` (one vec4 needs 1
   and 1). The triangle then takes the flat vertex-0 colour (coefficients flow).
2. DOUTI `shademodel` FLAT_VERTEX0 to GOURUAD, `perspective` 1, and
   `TA_STATE_VARYING0` `f32_flat` to `f32_linear`. The triangle then shows the
   RGB gradient.
3. Optional: `usc_sharedsize` in SIZEINFO2 should match what the shader reads
   (0 for an interpolating shader that reads no shared registers).

## 1. How a fragment shader receives an interpolated varying

```
VS output (position + varyings)
   | TA/geometry phase: the TE stores per-vertex varyings in the parameter
   | buffer, guided by PPP TA_STATE_VARYING0/1/2 (count and class of f32/f16
   | varyings: linear = smooth/perspective, flat, npc).
   v
ISP / coefficient engine, 3D phase, per covered primitive:
   | reads the three vertices' varyings, computes plane-equation coefficients
   | per component, writes the on-chip coefficient store.
   v
PDS coefficient-loading program at TA_STATE_PDS_VARYINGBASE:
   | one DOUTI per varying issues the iteration into the USC coefficient
   | registers; the DOUTI source word carries shademodel, size, perspective,
   | f16 and the f32/f16 coefficient offset.
   v
USC pixel shader reads the value from its coefficient registers. The space
given to it is TA_STATE_PDS_SIZEINFO1.usc_varyingsize; at 0 it reads 0.
```

The plane equations are computed by hardware, driven by `TA_STATE_VARYING0/1/2`.
The fragment shader does not read a static memory buffer. The coefficient base
is implicit and hardware-computed; the driver programs only how much
(`usc_varyingsize`) and what layout (`VARYING0/1/2` plus the DOUTI offsets).

## 2. The coefficient PDS program

`dst=4` in the DOUT instruction is DOUTI, not DOUTC: `pvr_pds_encode_doutu`
(the SHADERBASE kick) uses `DSTDOUT_DOUTU`, the varying program uses
`pvr_pds_encode_douti` / `DSTDOUT_DOUTI`, and `DSTDOUT_DOUTC` takes no source
words. The destination nibbles are 2 = DOUTU, 4 = DOUTI.

`DOUT_FIELDS_DOUTI_SRC` (pds.xml):

| field       | bits    | meaning                          |
|-------------|---------|----------------------------------|
| f16_offset  | [0:7]   | coefficient offset, f16 varyings |
| f32_offset  | [8:15]  | coefficient offset, f32 varyings |
| perspective | [16]    | perspective-correct              |
| f16         | [17]    | varying is f16                   |
| size        | [18:19] | 1D=0, 2D=1, 3D=2, 4D=3           |
| wrapu/v/s   | [20:22] | texture wrap                     |
| pointsprite | [23]    |                                  |
| shademodel  | [24:25] | FLAT_VERTEX0=0, FLAT_VERTEX1=1, FLAT_VERTEX2=2, GOURUAD=3 |
| primitiveid | [26]    |                                  |
| depthbias   | [27]    |                                  |

A Gouraud, perspective-correct f32 vec4 at offset 0 is
`(3<<24)|(1<<16)|(3<<18) = 0x030d0000`; without perspective
`(3<<24)|(3<<18) = 0x030c0000`. The two are numerically identical while every
vertex has w = 1.0.

Mesa's compiler emits the DOUTI with the GOURUAD default and the driver only
patches `shademodel` to `FLAT_VERTEXn` for varyings in `flat_iter_mask` at draw
time (`pvr_arch_cmd_buffer.c`, around line 7290). A flat varying is therefore
the patched form of the same program.

## 3. PPP fields that govern varying iteration (ppp.xml)

`TA_STATE_VARYING0`: `f32_npc` [16:23], `f32_flat` [8:15], `f32_linear` [0:7].
Mesa packs `f32_linear = vs.f32_smooth`, `f32_flat = vs.f32_flat`,
`f32_npc = vs.f32_npc`. One smooth vec4 is `0x00000004`; one flat vec4
`0x00000400`; one non-perspective vec4 `0x00040000` (pair with DOUTI
`perspective=0`). The class here and the DOUTI perspective bit must agree.

`TA_STATE_VARYING1` (f16 counts: `f16_npc` [24:31], `f16_flat` [16:23],
`f16_linear` [8:15]) and `TA_STATE_VARYING2` (`output_clip_planes` [16:19])
stay 0 here.

`TA_STATE_PDS_SIZEINFO1`:

```
pds_uniformsize      [23:31]  unit 4
pds_texturestatesize [16:22]  unit 4
pds_varyingsize      [10:15]  unit 4    size of the coefficient (DOUTI) program's data
usc_varyingsize      [4:9]    unit 16, max 63   coefficient space given to the USC
pds_tempsize         [0:3]    unit 4
```

Mesa: `pds_varyingsize = DIV_ROUND_UP(pds_coeff_program.data_size, 4)` and
`usc_varyingsize = DIV_ROUND_UP(fs_data->common.coeffs, 16)`
(`pvr_arch_cmd_buffer.c`, around line 7414). One vec4 gives 1 and 1, that is
bits 4 and 10 set.

`TA_STATE_PDS_SIZEINFO2`: `usc_sharedsize` [23:31] unit 16,
`pds_tri_merge_disable` [14], `pds_batchnum` [0:13].
`TA_STATE_PDS_VARYINGBASE`: `addr` [4:31] shift 4 (points at the coefficient
program). `TA_OUTPUT_SEL`: `vtxsize` [24:31], `rhw_pres` [16]. `rhw_pres=0`
stores no reciprocal-w, so true perspective division has nothing to use; it is
harmless while every vertex has w = 1.0, otherwise store rhw or use the npc
class.

## 4. What the interpolating fragment shader needs that a flat one did not

A flat triangle read a shared register loaded by the texture/uniform DOUTD and
never touched the coefficient store, so `usc_varyingsize = 0` was enough. The
interpolating path needs:

1. `usc_varyingsize >= 1` and `pds_varyingsize >= 1` in SIZEINFO1.
2. `VARYING0` declaring the varying linear, so the engine computes a plane
   equation rather than a constant.
3. DOUTI `shademodel = GOURUAD` (and perspective) so the iterator loads the
   interpolated value.
4. No new buffer or base, no FWIF field, no geometry or 3D command change.

The fixes belong in the SIZEINFO1 packing of `driver/src/pvr_vdm_ppp_gen.c`
(`struct pvr_ppp_frag_inputs` carries the two sizes), the DOUTI descriptor
word of the coefficient program, and the VARYING0 word.

## Verification

- With only the size fields set: a solid triangle in vertex 0's colour. If it
  is still black, dump the emitted SIZEINFO1 word and confirm bits 4 and 10.
- With the DOUTI and VARYING0 changes as well: the red/green/blue gradient.
  If it comes out flat, one of the two did not take effect.
- Do not chase perspective as a cause while every vertex has w = 1.0.
