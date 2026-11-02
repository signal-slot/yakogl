<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the vertex-shader ops behind the `arithmetic` refusals

The vertex half of `notes/mali-alu-ops.md`, for the eight vertex sources that
`tools/mali/corpus-verdicts.txt` pins as `arithmetic other than +, -, * and a
dot or matrix product` (#33): gl_test cases 86 anyall, 181 fusemad (its control
half), 182 fsat and 188 predif, and the corpus sources
`es100/{anyall,partial_varying,psiz_if,vrcp}.vert`,
`gltest/{fma_vs_ctrl,fsat_vs_ctrl,fsat_vs_folded,predif_vs_uni}.vert`. Mesa's
side is 26.2.1 on drm-shim (GPU_ID 0xa867): every claim below is read from a
reference `tools/mali/testdata/vsarith_<case>_<name>.{vert,frag,expect.json}`
(per region the words, the disassembly `text`, `work_reg_count`, FAU).

## How the ops were found and the references made

The refusal is the `_ => return Err(MaliError::Arithmetic)` arm of the vertex
`IrOp::Alu` match in `src/shaderc/online/mali/nir.rs`. A temporary `eprintln!`
of the op at that arm (and at `Ffma => FusedMultiplyAdd`), made non-fatal, and a
second one over every `IrOp::Alu` of the module before the walk, listed the ops
of each source below; reverted, nothing under `src/` is in the commit. The
second list is the complete one (a non-fatal walk stops at the first later
error: `Ball` on a bool made the first run lose `Flt`/`Bany`).

Per pair (`<shim>` a `drm_shim_ref.sh build` of Mesa 26.2.1; gl_ref.py draws
once, 64x64):

    tools/mali/drm_shim_ref.sh run <shim> V F target/vsarith/<name>
    python3 -I tools/mali/pan_ref_diff.py extract target/vsarith/<name>.trace --draw \
        > tools/mali/testdata/vsarith_<name>.expect.json

| Reference | Vertex shader | Fragment partner |
|---|---|---|
| `vsarith_86_anyall` | `es100/anyall.vert` | `gl_test_es100/interp_color.frag` |
| `vsarith_181_fmactrl` | `gltest/fma_vs_ctrl.vert` | `gltest/fma_fs_varying.frag` |
| `vsarith_182_fsat` | `gltest/fsat_vs_ctrl.vert` | `gltest/fma_fs_varying.frag` |
| `vsarith_182_fsatfolded` | `gltest/fsat_vs_folded.vert` | `gltest/fma_fs_varying.frag` |
| `vsarith_188_predif` | `gltest/predif_vs_uni.vert` | `gltest/fma_fs_varying.frag` |
| `vsarith_vrcp` | `es100/vrcp.vert` | `vsarith_vrcp.frag` (draws `v_t`) |
| `vsarith_psiz_if` | `es100/psiz_if.vert` | `es100/va.frag` |
| `vsarith_partial_varying` | `es100/partial_varying.vert` | `es100/partial_varying.frag` |

Fresh runs equal the older references (compared as JSON without the `trace`
file name): `vsarith_86_anyall` = `alu_anyall`, `vsarith_182_fsat` =
`alu_fsat`, `vsarith_182_fsatfolded` = `alu_fsat_folded`,
`vsarith_188_predif` = `alu_predif`. The other four have no older twin
(`varyconst_181_fusemad` is the *fused* half, `fma_vs_fused.vert`). The
`vs.position` program is the same words in all of them but `psiz_if`.

## The ops each source meets (complete lists, in IR order)

| Source (case) | Ops of our IR | Refused by the vertex walk |
|---|---|---|
| `es100/anyall.vert` (86) | `Feq`(w4) `Ball` `Bcsel`(w4) `Flt` `Bany` `Bcsel` | `Feq`, `Ball`, `Flt`, `Bany` (`Bcsel` is taken) |
| `gltest/fma_vs_ctrl.vert` (181) | `Fmul Fneg Fadd Fmul Fmul Fabs Fneg Fadd Fmul Fmul Ffloor Fadd` | `Fabs`(w4), `Ffloor` |
| `gltest/fsat_vs_ctrl.vert` (182) | `Fmax Fmin Fmul Fadd` x3 | `Fmax`, `Fmin` |
| `gltest/fsat_vs_folded.vert` (182) | `Fmax Fmin` x6 around `Fneg Fadd Fmul Fadd` | `Fmax`, `Fmin` |
| `gltest/predif_vs_uni.vert` (188) | `Flt Fmul Fadd Fmul` (the arms) | `Flt`; then the `if` (`ControlFlow`) |
| `es100/vrcp.vert` | `Frcp Fmul Frsq` x2 | `Frsq` (`Frcp` is taken) |
| `es100/psiz_if.vert` | `Flt Fadd Fmul` | `Flt`; then the `if` (`ControlFlow`) |
| `es100/partial_varying.vert` | `Flt` | `Flt`; then the `if` (`ControlFlow`) |

## Mesa's instruction sequences, from the references

All f32 (`.f32`); the vertex stage is highp. Every arithmetic varying lane ends
in `FMA.f32 x, x, 0x0, x` (the `+0` quirk, in `alu_fsat` too).

- **FMA fusion is not done on a shared product** (`vsarith_181_fmactrl`).
  `p = u_a*u_b` has four uses, so `p` is a single `FMA.f32 r0, u5.w1, r0,
  0x0.neg` (a product is an FMA with `-0.0`; `r0` a `MOV` of the other
  uniform), and its users are separate: `u_x - p` is `FADD.f32 r1, u6.w1,
  r0.neg`, `p*2.0` is `FADD.f32 r1, r0, r0`. The fused half
  (`varyconst_181_fusemad`) is the contrast: `FMA.f32 r1, 0x40000000.neg,
  u6.w0, r0` for `u_x - 2.0*u_q`, one FMA each.
- **`abs` is a source modifier** (`vsarith_181_fmactrl`): `u_x - abs(p)` is
  `FADD.f32 r1, u6.w1, r0.neg.abs` (`-abs(p)` as `.neg.abs`). `abs` alone
  would be `.abs` on its consumer; it is never an instruction here.
- **`floor`** (`vsarith_181_fmactrl`): `floor(p*4.0)+0.25` is `FMA.f32 r0, r0,
  0x44000000.h1, 0x0.neg` (the `* 4.0`; `0x44000000.h1` is the half 4.0),
  `FROUND.f32.rtn r0, r0`, `FADD_IMM.f32 r0, r0, #0x3E800000`.
- **fsat / clamp / min / max with a bound of 0 or 1 is a modifier**
  (`vsarith_182_fsat`, `vsarith_182_fsatfolded`): `max(q,0)` is `FADD.f32
  .clamp_0_inf x, 0x0.neg`; `min(max(q,0),1)` is `FADD.f32.clamp_0_1 x,
  0x0.neg` (the `+ -0` is the identity the modifier rides on); `min(1,
  max(0, u_a - 1.1))` is one `FADD.f32.clamp_0_1 r2, u6.w1, r2` with `r2` a
  `MOV` of the FAU word `bf8ccccd`. A bound other than 0 or 1 stays an
  instruction, and the 0/1 side is still a modifier: `clamp(u_a,0,0.8)` is
  `FMIN.f32.clamp_0_1 r2, u6.w1, r2` (`r2` the FAU word `3f4ccccd`) and
  `min(max(u_x,0.2),1)` is `FMAX.f32.clamp_0_1 r2, u5.w1, r2` (`3e4ccccd`).
  A constant bound goes through a `MOV` of its FAU word, not an operand.
- **compare and select** (`vsarith_188_predif`): `u_x > 0.0` is `FCMP_OR.f32.lt.m1
  r3, 0x0, u5.w1, 0x0` (`0 < x`: operands swapped, `.m1` writes all ones);
  both arms are computed first (`FADD.f32 r0, u6.w1, u6.w1` is `a*2.0`,
  `FADD_IMM.f32 r1, u6.w0, #0x3F000000` is `q+0.5`, a `FMA.f32` with
  `0x38000000.h1` is `a*0.5`) and one `CSEL.u32.eq r, r3, 0x0, then, else`
  per differing lane picks; a lane equal in both arms is stored once (`1.0`
  as `IADD_IMM.i32 r2, 0x0, #0x3F800000`), a lane constant per arm (0.25 / 0.75)
  is a `CSEL` of two FAU words. No branch.
- **a compare feeding `? 1.0 : 0.0` is the `.f1` form** (`vsarith_86_anyall`):
  the select is gone; `FCMP_OR.f32.eq.f1` / `FCMP_OR.f32.lt.f1` write 1.0 or
  0.0 directly.
- **`all(equal(uv, vec2(0)))`** (`vsarith_86_anyall`): `FADD.f32 r0, r1.abs,
  r0.abs` then `FCMP_OR.f32.eq.f1 r0, r0, 0x0, 0x0`.
  **`any(greaterThan(n, vec3(0.5)))`**: `FMAX.f32 r1, r4, r2`, `FMAX.f32 r1,
  r1, r3` then `FCMP_OR.f32.lt.f1 r1, 0x38000000.h1, r1, 0x0` (`0.5 < max`).
  Neither is a bool vector or a reduction instruction; the vector `Feq`/`Flt`
  and `Ball`/`Bany` of our IR do not exist in Mesa's program. The varying
  stores only the two lanes the shader computes (`estream`, two stores); `z`
  and `w` are constants the link moves.
- **`frcp` / `frsq`** (`vsarith_vrcp`): `1.0 / a_w.x` is `FRCP.f32 r0, r0` and
  `inversesqrt(a_w.y)` is `FRSQ.f32 r1, r1`; no fixup instruction, no `+0`
  quirk (the lane is an attribute transform that ends in the store). `Frcp` is
  already taken by our vertex walk (the perspective divide uses it).
- **a point-size select** (`vsarith_psiz_if`, in `vs.position`): the `if` over
  `gl_PointSize` is `FCMP_OR.f32.lt.m1 r6, 0x0, r2, 0x0` and `CSEL.u32.eq r0,
  r6, 0x0, r1, r0` (`r0 = 2.0 + a_color.x` as `FADD_IMM.f32 #0x40000000`,
  `r1 = 5.0 * a_color.y` as `FMA.f32 r1, u3.w0, r1, 0x0.neg`), then `FADD.f32
  r0.h0, r0, 0x0.neg` (narrow to f16) and `FMAX.v2f16 r0, r0.h00,
  0x3C000000.h11`. The varying program is the plain copy of `a_color`.
- **a conditionally written varying lane** (`vsarith_partial_varying`): the
  `if (a_coords.x > 0.0) tt.x = ...` leaves **no instruction**: the varying
  program is the four stores of `a_coords` (`LD_ATTR_IMM.v4`, `index:0x1`),
  no compare, no `CSEL`, nothing for `tt`; the fragment loads one `v4`
  varying. The `Flt` of our IR is dead in Mesa's.

## What each op unblocks

| Op (Mesa form) | Sources | Cases |
|---|---|---|
| `Feq`/`Flt` + `Ball`/`Bany`, bcsel(1,0) (`FADD abs` / `FMAX` chain, `FCMP_OR.f1`) | `anyall.vert` | 86 |
| `Fmax`/`Fmin` (`.clamp_0_1`, `.clamp_0_inf` on FADD/FMIN/FMAX, bound via `MOV` of a FAU word) | `fsat_vs_ctrl`, `fsat_vs_folded` | 182 |
| `Fabs` (`.abs`, `.neg.abs` modifiers), `Ffloor` (`FROUND.f32.rtn`) | `fma_vs_ctrl` | 181 |
| `Flt` + vertex `if` as `CSEL` | `predif_vs_uni` | 188 |
| `Flt` + `if` over `gl_PointSize` + the f16 narrow and `FMAX.v2f16` | `psiz_if` | none |
| a dead conditional write of a varying lane (no `Flt` at all) | `partial_varying` | none |
| `Frsq` (`FRSQ.f32`) | `vrcp` | none |

Cases 181, 182 and 188 write varying lanes computed from uniforms only; the
op work alone does not finish them: the next refusal is
`VaryingNotFromAttribute` (`notes/mali-varying-constants.md`, rule 3: the
uniform-only lane is a flat varying). In `notes/mali-alu-ops.md` the `fmax`/`fmin` rules of 182 were written and
dropped for that reason: no byte-equal test could hold them until the flat
varying exists, so items 2, 3 and 5 below finish their cases together with
item 4.

## Lowering items (each one agent session), in order

1. **vs-cmp-reduce (case 86)**: `Feq`/`Flt` into `FCMP_OR.f32.<cmp>.f1` when the
   consumer is `bcsel(c, 1.0, 0.0)`, `Ball`/`Bany` of `Feq`/`Flt` lanes into
   `FADD abs` + compare and the `FMAX` chain, the lane subset (a vec2 and a
   vec3 over a vec4 IR value). Attributes only, no flat varying: the one
   item that finishes a case alone. Reference `vsarith_86_anyall`.
2. **vs-clamp (case 182)**: `Fmax`/`Fmin` into `.clamp_0_1` / `.clamp_0_inf`
   on `FADD`/`FMIN`/`FMAX`, constant bounds via `MOV` of the FAU word.
   References `vsarith_182_fsat`, `vsarith_182_fsatfolded`. Finishes the
   case with item 4 (flat varying).
3. **vs-abs-floor (case 181)**: `Fabs` as the `.abs` / `.neg.abs` source
   modifier, `Ffloor` as `FROUND.f32.rtn`, the shared-product rule (no FMA
   when the product has several users). Reference `vsarith_181_fmactrl`.
   Finishes the case with item 4.
4. **vs-flat-varying (cases 181, 182, 188)**: the uniform-only flat varying of
   `notes/mali-varying-constants.md` rule 3, already designed there; listed
   because items 2, 3 and 5 depend on it for their cases.
5. **vs-if-convert (case 188; `psiz_if`)**: the vertex `if` with the compare
   of `Flt` into `FCMP_OR.f32.lt.m1` and one `CSEL.u32.eq` per differing lane;
   `gl_PointSize` as a select with the f16 narrow and `FMAX.v2f16`. References
   `vsarith_188_predif`, `vsarith_psiz_if`.
6. **vs-frsq and dead varying write**: `Frsq` as `FRSQ.f32` (`vsarith_vrcp`),
   and the removal of a conditional write of a varying lane the vertex shader
   leaves partly undefined (`vsarith_partial_varying`). No sweep case.
