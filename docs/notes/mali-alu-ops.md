<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the ops behind the 16 `arithmetic` refusals

Study of the 16 cases of 0dce654a8:tools/mali/board-ref/2026-10-23-s6-discard/ours/
03-panthor-gltest--sweep-1.log that end in `Valhall: arithmetic other than
+, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)
is not lowered yet` (`MaliError::Arithmetic`). Mesa's side is 26.2.1 on
drm-shim (GPU_ID 0xa867): the references are
`tools/mali/testdata/alu_<name>.{vert,frag,expect.json}`, each a copy of the
shader pair and `pan_ref_diff.py extract --draw` of the first draw (per
region: words, disassembly, `work_reg_count`, preload, FAU).

## How the references were made

Per case (`<shim>` is a `drm_shim_ref.sh build` of Mesa 26.2.1, here
`$CONNECTED_DEPS_DIR/mesa-shim`; gl_ref.py draws the pair once, 64x64):

    tools/mali/drm_shim_ref.sh run <shim> V F target/alu/<name>
    python3 -I tools/mali/pan_ref_diff.py extract target/alu/<name>.trace --draw \
        > tools/mali/testdata/alu_<name>.expect.json

and V and F copied to `alu_<name>.vert` / `.frag`. The pairs (sources under
tools/shaders): V is `gl_test_es100/passthrough.vert` for armselect and
`es100/{loopcarry,deadloop,branchlane,ifoutput,ifoutput2,boolany,transc,
vtransc,branchquad}.frag`; `es100/anyall.vert` + `gl_test_es100/
interp_color.frag`; `es100/deriv.vert` + `es100/{deriv,unibranch,texbranch}
.frag`; `gltest/fsat_vs_{ctrl,folded}.vert` and `gltest/predif_vs_uni.vert`
with `gltest/fma_fs_varying.frag` (names `fsat`, `fsat_folded`, `predif`);
`es300/flatlast.vert` + `.frag`; `es300/bands.vert` + `es300/divloop.frag`
(gl_test.c's links). `ifoutput2` and `vtransc` are the second programs of
cases 74 and 77. For the two ES 3.00 pairs gl_ref.py reports "attributes
(none)" (their `layout(location=)` attributes are not bound by name); the
programs are compiled as the application would, so the words stand, but the
vertex fetch's attribute table of those two is not the case's.

## How the first op was found

A temporary `eprintln!` of the op at the four sites that return
`MaliError::Arithmetic` (`nir_fs.rs` `alu`, `alu32`, the generic
`ir::Op::Alu(..)` arm, `nir.rs`'s vertex `IrOp::Alu` arm) and a temporary test
that called `compile()` on each source, one stage at a time. Reverted; nothing
under `src/` is in the commit. Cases 86, 89, 182 and 188 are refused in
their **vertex** shader (the sweep log says "this vertex shader"); the
fragment partner of each already compiles (`fsat_fs_*`, `predif_fs_uni` are
`ok` in corpus-verdicts.txt; `predif_fs_coord` is refused for another reason).

## The table

Words are Mesa's instruction words (first, or the characteristic ones; every
word is in the named `alu_*.expect.json`). `mediump` fragment shaders are
fp16: Mesa uses `.v2f16` forms there.

| Case | First refused op | Mesa 26.2.1's Valhall instruction(s) | Words |
|---|---|---|---|
| 68 armselect | `flt` (`u_flag.x > 0.5`; if/else over two vec4 locals) | no branch: `MOV r0,0`; `FCMP_OR.v2f16.lt.m1`; two `MUX.v2i16` per pair of lanes; mixed with `* 0.5` constants | `00f5c004b04088da` FCMP_OR; `00b9c10128008584`, `40b9c00128408786` MUX (alu_armselect, 11 words) |
| 70 loopcarry | `fceil` (`ceil(u_k.x*3.0)`), then a loop with a `break` | `FMA.v2f16`; `FROUND.v2f16.rtp`; `FMIN.v2f16` (the 8 trip cap); the loop is **fully unrolled** into 8 x (`FCMP_OR.v2f16.ge/lt.m1` of the trip index + `MUX.v2i16` + `FADD.v2f16`); no branch | `0090c100400f0000` FROUND; `00f5c302a0c00186` FCMP_OR.ge; `00b9c2010c430042` MUX (alu_loopcarry, 29 words) |
| 71 deadloop | `flt` (`u_k.z > 0.0` in the dead `unused()`), then the same loop | the dead if and `waste` are deleted; the program is **word-for-word the program of 70** | as 70 (alu_deadloop equals alu_loopcarry) |
| 73 branchlane | `flt` (`u_c.y > 0.5` of `if (..) discard;`, then `u_c.w`) | one `DISCARD.f32.lt` per test, the uniform lane as the operand, **no compare instruction** (the shape of s6_discloop); then `MOV` colour constants | `6820c004280084da`, `4020c004280085da` (alu_branchlane, 8 words) |
| 74 ifoutput | `flt` (`u_c.x > 0.5`; `gl_FragColor` written in both arms) | `FCMP_OR.v2f16.lt.m1`; `MUX.v2i16` choosing the constants `0`/`1.0` per lane; `MKVEC.v2i16`; no branch. ifoutput2 (write, then overwrite in the `if`) is word-for-word the same | `00f5c004b04084da`, `00b9c1012c00dbc0`, `40b9c0013840c0db` (alu_ifoutput, alu_ifoutput2) |
| 76 boolany | `flt` (`greaterThan(u_v, vec4(0.5))`; then `any`/`all` and bool uniforms) | a bool uniform is an `i32` (`MOV r0,u5.w0`, `CSEL.u32.eq`); `any(bvec3)` / `all(bvec3)` of uniforms: `LSHIFT_OR.i32` / `LSHIFT_AND.i32` chains; the `greaterThan` lanes are `FCMP_OR.v2f16.lt.m1` with the lane masks AND/ORed as `.v2i16` (`LSHIFT_OR.v2i16`, `LSHIFT_AND.v2i16`, `IADD.v2u16`), then `MUX.v2i16` to 0/1.0 | `00b4c20111840185` LSHIFT_OR.i32; `00b4c30110840185` LSHIFT_AND.i32; `00f5c204b80189da` FCMP_OR; `00b5c3311102c002` LSHIFT_OR.v2i16 (alu_boolany, 29 words) |
| 77 transc | `fsin` (`sin(u_a.x)`; then `cos`, `log2`, `exp2`) | `sin`/`cos` (`bi_lower_fsincos_32`, the arg in radians, fp16 in, fp32 inside): `F16_TO_F32`, `FMA.f32` (x 2/pi + bias), `FADD_IMM.f32`, `FMA.f32`, `FSIN_TABLE.u6`, `FCOS_TABLE.u6`, `FMA_RSCALE.f32`, `FMA.f32`, `FMA.f32.clamp_m1_1`, `FADD.f32`; `log2`: `FREXPE.f32.log`, `S32_TO_F32`, `FADD_LSCALE.f32`, `FLOGD.f32`, `FMA.f32`; `exp2`: `FMA_RSCALE.f32`, `F32_TO_S32`, `FEXP.f32`. vtransc (the fragment-vertex-style variant) adds `FRSQ.f16` and `V2F32_TO_V2F16` | `009cc20000040001` FSIN_TABLE; `009cc10000050041` FCOS_TABLE; `0160c304c1c00000` FMA_RSCALE; `00b2c00200434140` FMA.clamp_m1_1; `0099c30002020002` FREXPE.log; `009cc20000080042` FLOGD; `00a4c30000084344` FEXP (alu_transc, 42 words; alu_vtransc, 38) |
| 81 branchquad | `flt` (`v_color.y > 0.5` of a discard; then `w`) | `LD_VAR_BUF_IMM.v4`, two `FADD.f32 r.h0, r, 0x0.neg` (fp32 to fp16 lane), two `DISCARD.f32.lt 0.5h` | `68a440100000c041`, `0020c004240040da`, `4020c004240041da` (alu_branchquad, 11 words) |
| 86 anyall (**vs**) | `feq` (`equal(a_uv, vec2(0))` inside `all`) | folded by NIR: `all(equal(uv,0))` is `FADD.f32 \|a\|,\|b\|` then `FCMP_OR.f32.eq.f1 ..,0`; `any(greaterThan(n,0.5))` is `FMAX.f32` chain then `FCMP_OR.f32.lt.f1 0.5h, max`; the `.f1` form writes 1.0 | `00a4c0a000004041` FADD abs; `00f4c00040c0c040` FCMP_OR.eq.f1; `00a4c10000034244` FMAX; `00f4c10460c041da` FCMP_OR.lt.f1 (alu_anyall vs.varying, 13 words) |
| 89 flatlast (**vs**, then fs) | `f2u` (`uint(col.a*4.0+0.5)`); the fs: `u2f` | **the vertex stage does the whole chain**: `FMA.f32`, `F32_TO_U32.rtz`, `U32_TO_F32`, `FMA.f32 (*0.25)` and stores the float; the fs loads one `LD_VAR_BUF_IMM.v4.src_flat32` (flat, provoking vertex) and converts `V2F32_TO_V2F16`; the uint is not a varying of its own | `0090c300c01c0043` F32_TO_U32; `0090c30000190043` U32_TO_F32; `085c8008300000c0` LD_VAR_BUF_IMM.src_flat32 (alu_flatlast) |
| 104 divloop | `ineg` (`int last = -1`; the folded `-1` is a constant) | a real loop, no unrolling: `FROUND.f32.rtn`, `FCMP_OR.f32.lt.m1` + `CSEL` (the 1e9 bound), `IADD_IMM #0xFFFFFFFF`, a counted body (`S32_TO_F32`, `FCMP_OR.f32.ge.m1`, `BRANCHZ.reconverge offset:4` forward exit, `FADD_IMM`, `IADD_IMM`, `BRANCHZ.eq.reconverge 0x0, offset:-8` back edge), then `FMA`/`FROUND` for the `mod`, `ICMP_OR.u32.eq.m1`, `CSEL`. `ineg` itself is no instruction here | `501fc02000000443` BRANCHZ (exit); `501fc017fffff8c0` BRANCHZ (back edge); `0110c2ffffffffc0` IADD_IMM -1; `00f0c10080c04142` ICMP_OR (alu_divloop, 44 words) |
| 138 deriv | `fddx` (`dFdx(v.x)`, `dFdy(v.y)`, `dFdx(gl_FragCoord.x)`) | fine derivatives, `bi_emit_derivative`: `CLPER.i32.subgroup4.xor` (axis 1 or 2) then `FADD.v2f16 right, left.neg`; a sign fix because the user cares about the sign: `LSHIFT_AND.i32 lane_id` (the Y axis only) and `LSHIFT_XOR.v2i16` of the lane id into the result; the `gl_FragCoord` operands via `U16_TO_F32 r59.h1/.h0` | `00a0c311180fc801` CLPER (x); `00a0c311180fc802`; `00a0c4111c0fc403` (y); `00a5c11028004143` FADD; `00b5c1011a41cb04` LSHIFT_XOR (alu_deriv, 37 words) |
| 142 unibranch | `flt` (`u.x > 0.5`, a uniform if with two large arms and `clamp`) | **if-converted, no branch** even though the arms exceed 24 ops: both arms in `FMA.v2f16`/`MKVEC.v2i16`/`.clamp_0_1` forms, one `FCMP_OR.v2f16.lt.m1`, then `MUX.v2i16` per lane pair | `00f5c004b04084da` FCMP_OR; `00b9c20128004243`, `40b9c00100404144` MUX (alu_unibranch, 61 words) |
| 148 texbranch | `flt` (`u.x > 0.5` guarding `texture2D`; then a 24-pass loop with a uniform `break`) | **real branches**: `BRANCHZ.eq.reconverge rX.h0, offset:N` round each look-up arm (`TEX_SINGLE` inside), the loop unrolled 24 passes each guarded by its own forward `BRANCHZ` (offsets 501, 482, 460, 439, ...); 542 words, 51 `TEX_SINGLE`, 30 `BRANCHZ`, no back edge | `501fc03000000800` BRANCHZ.reconverge; `0928028410412044` TEX_SINGLE (alu_texbranch) |
| 182 fsat (**vs**) | `fmax` (`max(u_q,0)`, `clamp(a,0,0.8)`, `max(min(..))`) | the clamp folds into the instruction's modifier: `FADD.f32.clamp_0_1 x, 0x0.neg`, `FADD.f32.clamp_0_inf` (the `max(q,0)`), `FMIN.f32.clamp_0_1`, `FMAX.f32.clamp_0_1`; the varying then `FMA.f32 *0.5+0.25` and a `FMA.f32 x,0,x` (the vertex stage's `+0` quirk). `fsat_folded` has only `FADD.clamp_0_1` | `00a4c0130000c08c` FADD.clamp_0_1; `00a4c4110000c08c` FADD.clamp_0_inf; `00a4c2030002428d` FMIN.clamp_0_1; `00a4c2030003028b` FMAX.clamp_0_1 (alu_fsat vs.varying, 30 words; alu_fsat_folded, 29) |
| 188 predif (**vs**) | `flt` (`u_x > 0.0` of an if/else over a vec4) | if-converted: `FCMP_OR.f32.lt.m1 0x0, u5.w1` (the compare with the constant on the left) and one `CSEL.u32.eq` per lane | `00f4c30480c08bc0` FCMP_OR; `0150c000408cc003`, `0150c1004142c003`, `0150c200428ec043` CSEL (alu_predif vs.varying, 26 words) |

## What the references say beyond the table

- **Mesa removes most of the control flow before the back end.** 68, 70, 71,
  74, 142 and 188 end with no branch at all (if-converted selects, the 8-pass
  loop of 70/71 fully unrolled). Only 104 (a real counted loop) and 148
  (look-ups inside conditionals, 24 unrolled guarded passes) keep
  `BRANCHZ`.
- **Whole-chain work moves across the varying link**: in 89 the vertex stage
  does `f2u`/`u2f`, in 86 NIR folds `all(equal(..))` to a `FADD abs` + one
  compare. Our front end must produce the same folded NIR to be byte-equal.
- **`ineg` of a constant is no instruction** (104): `IADD_IMM #0xFFFFFFFF`.

## The ops, grouped by shared work, and what each unlocks

1. **Compare outside a select** (`flt`/`feq`/`fle`): 68, 71, 73, 74, 76, 81,
   142, 148, 188, 86. References for the plain forms: s4_flt, s4_fle, s4_feq.
   The forms these cases need are `FCMP_OR.v2f16.<cmp>.m1` (mediump pair),
   `FCMP_OR.f32.<cmp>.m1/.f1` (highp, vertex), and the discard-fused
   `DISCARD.f32.<cmp>` (group 3).
2. **If-conversion into `MUX.v2i16` / `CSEL.u32.eq`**: 68, 74, 142, 188 (and
   the unrolled loop of 70/71). References: s4_bcsel (`CSEL`), the cases above.
3. **Discard behind a lane compare**: 73, 81. Fused `DISCARD.f32.lt` with the
   uniform or varying lane as operand (s6_discloop; docs/notes/mali-discard.md).
   In 81 the varying lane is first narrowed by `FADD.f32 r.h0, r, 0x0.neg`.
4. **Saturate / min / max folding into modifiers (vertex)**: 182 only, with
   `.clamp_0_1`, `.clamp_0_inf` on `FADD`/`FMIN`/`FMAX` (s4_fsat, s4_fmax,
   s4_fmin are the fragment forms).
5. **Rounding**: `fceil` (70) = `FROUND.<v2f16|f32>.rtp` (+ `FADD ..0x0.neg`
   in the highp form, s4_fceil); `ffloor` = `FROUND.rtn` (104, s4_ffloor).
6. **Transcendentals**: 77 only; the sin/cos, log2 and exp2 sequences of the
   table. No other case needs them, so they can be done after the cheap
   groups, but they are self-contained.
7. **Bool / integer values**: 76 (bool uniforms as i32, `any`/`all` as
   `LSHIFT_OR/AND`, `.v2i16` mask arithmetic), 86 (folded to float compares,
   so almost no integer work), 89 (`F32_TO_U32.rtz`, `U32_TO_F32`, a flat
   varying), 104 (`S32_TO_F32`, `F32_TO_S32`, `IADD_IMM`, `ICMP_OR.u32.eq`).
   Overlaps the integer-values group (`MaliError::IntValue`).
8. **Derivatives**: 138 only: `CLPER.i32.subgroup4.xor`, `lane_id`, the sign
   fix. Needs a `gl_FragCoord` read in fp16 (`U16_TO_F32 r59.h`).
9. **Flat varyings**: 89 (`LD_VAR_BUF_IMM.src_flat32`; the descriptor of the
   flat varying and the provoking vertex are a link matter).

## Cases that also need control flow

- **Real loop with a back edge**: 104 (`BRANCHZ ... offset:-8`, per-pixel
  bound; the loop-break work, study-loop).
- **Forward branches round an arm containing look-ups**: 148 (`TEX_SINGLE`
  inside `BRANCHZ.reconverge`; 24 unrolled guarded passes).
- **None** (select/unroll/discard only): 68, 70, 71, 73, 74, 76, 77, 81, 86,
  89, 138, 142, 182, 188. 70/71 need a loop **unroller** with a `break`
  folded to selects, not branches.

## Recommended implementation order

1. Groups 1-3 in the fragment stage (68, 74, 73, 81, then 142): compare as
   `FCMP_OR.v2f16`, `MUX.v2i16`, fused `DISCARD`. 73 and 81 first: the
   discard is already lowered, only the operand (a lane other than x, a
   varying lane narrowed to h0) is new.
2. The same compare/`CSEL` forms in the vertex stage (188), then group 4
   (182) and `fmax/fmin` there.
3. Group 5 and the loop unroller for 70/71 (they are the same word-for-word
   program; one fix unlocks both).
4. Group 7 bool part (76), then 86 (needs NIR's fold of `all(equal)`).
5. Group 6 (77) and group 8 (138): independent, each self-contained.
6. 89 (integer conversions, flat varying, vertex-side chain).
7. Control flow last: 104 (a real loop) and 148 (guarded look-ups), with the
   loop and texture-branch work. 104 also needs the `ineg`-of-constant fold.

## What the online compiler takes (plan #153 alu-lower)

- **73 branchlane** (byte-equal, `the_alu_population_is_byte_equal_to_mesa_26_2_1`):
  a mediump compare is a 16-bit `Fcmp` node of one lane (nir_fs.rs, the
  walk's `Flt`/`Fle`/`Feq` arm), selected as `FCMP_OR.v2f16.<cmp>.m1` with
  each source's half replicated (`.h00`/`.h11`, bir.rs `FcmpOrV2f16`); a
  mediump `demote_if` is `DISCARD.b32 mask.h00`, which opt.rs fuses as
  `bi_fuse_discard_fcmp` does for a 16-bit compare (the mask's swizzle
  composed into each source's): `DISCARD.f32.lt 0x38000000.h1, u2.w0.h1`.
  The compare's operands may be an fp16 uniform's component or a constant.
- **81 branchquad** stays refused (`DiscardForm`) at its varying operand: Mesa
  converts the lane alone (`FADD.f32 r0.h0, r1^, 0x0.neg`, which the selector
  already makes), but it also keeps the mediump `v_color` whole (`.v4` load,
  four vertex stores) although only y and w are read, where every highp
  reference (fvgtest_discard2: `.y` alone, one component) and pan_shc's
  mediump `cmp_*` compact. The GL path's rule for a partly read mediump
  varying needs a reference of its own (a mediump `gl_FragColor =
  vec4(v.y, v.w, 0, 1)` pair on the shim) before the layout changes.
- The vertex `fmax`/`fmin` rules of 182 (fsat) were written and dropped: the
  programs then stop at `VaryingNotFromAttribute` (the uniform-only flat
  varying, docs/notes/mali-varying-constants.md rule 3), so no byte-equal
  test could hold them. 188 predif stops there too.
- Every other case's next refusal is its control flow (an `if` or a loop in
  a mediump shader: 68, 70/71, 74, 142, 148) or the ops of the table
  (76, 77, 86, 89, 104, 138), in the order above.

## Remaining fragment ops (plan #156 triage-fsarith)

State at 7ad18d27: the fragment sources of `corpus-verdicts.txt` that still end
in `MaliError::Arithmetic`. The first refused op of each was found as in "How
the first op was found" (a scratch `eprintln!` at the five fragment sites of
`nir_fs.rs`, reverted; the walk stops at the first op, so a source can need
more than the one named). The names are the front end's `AluOp`; some are its
spelling of an op Mesa writes otherwise (`Ineg` for `abs(int)`, `Bnot` for
`step`).

| Source | Case | First refused op | Also needs |
|---|---|---|---|
| es100/loopcarry | 70 | `Fceil` (mediump arm of `walk_nodes`) | the `break` loop unrolled into selects (cflow-flatten; the program equals 71's) |
| es100/transc | 77 | `Fsin` (mediump arm) | `Fcos`, `Flog2`, `Fexp2`, a constant `* 0.25` |
| es100/vtransc | 77, 2nd program | `Fsin` (mediump arm) | `Frsq` of one fp16 lane |
| es100/deriv | 138 | `Fddx` (mediump arm) | `Fddy`, `gl_FragCoord` x and y as fp16 (`U16_TO_F32 r59`) |
| gltest/es3trunc_fs | 199 | `Ftrunc` (`alu32`) | a flat varying; the int ops are done by Mesa in the vertex stage (fsarith_199) |
| es100/dynidx | - | `Bcsel` on a non-constant condition (the guard in `walk_nodes`) | `Ieq` of an int uniform against 0..3 per lane; reference intloop_dynidx |
| es100/f2b | - | `Ffloor` (mediump arm) | `bool(float)` (`Fne`; `alu32` refuses `Fne`/`Ine` as `CompareForm`), `bcsel` of constants |
| es100/fmods | - | `Fexp2` (`alu32`) | `Flog2`, `Frsq`, `Ffloor`, `Fmin`, one 9-term chain |
| es100/int_div | - | `Idiv` by a non-constant (`alu32`) | `Ineg`, `Imul`, the reciprocal sequence; intloop_idiv is the mediump twin (case 79) |
| es100/varyings15 | 97 | `Ball` (mediump arm) | not an op: the vertex shader writes constants and Mesa folds them into the fragment shader (mixprec_varyings15's fs is two `MOV 0x3C000000`, no compare) |
| es300/derivs | - | `Fddx` (`alu32`, fp32) | `Fddy` on highp lanes; no reference |
| es300/gcops | - | `Ineg` (`alu32`: `abs(int)`) | `Ffloor`, `Fsqrt` (`FRSQ` then `FRCP`), `Ixor`, `& 255u`, `U8_TO_F32`; intuni_gcops |
| es300/ifelse_uint | - | `Ult` (`alu32`) | `Umod`/`Udiv` by the constants 7 and 3, `Bcsel` of vec4s; intuni_ifelse_uint |
| es300/int_div | - | `Ineg` (`alu32`) | `Imod`, `Udiv`/`Umod` by variables and by 65537, 2^31-1, 2^25-1, 3, `U2f`; no reference |
| es300/layersunused | - | `Bnot` (`alu32`: `step(edge, x)`) | an unused `sampler2DArray` uniform; no reference |
| es300/uops | - | `Udiv` (`alu32`) | `Umod`, `Ushr`, `Umin`/`Umax`/`Imin`/`Imax`, `Inot`, `F2u`, `Ult`/`Ule` selects, `fract`; intuni_uops |
| es300/flatlast | - | `U2f` (mediump arm) | the flat varying; alu_flatlast |
| gltest/f1_fs | - | `Flog2` (mediump arm) | no GPU op: its `log2`, `atan`, `inversesqrt` and int casts are all of constants, and Mesa folds the program to one `TEX_SINGLE` (intloop_f1_fs) |

### Mesa 26.2.1's sequence per op, and what has a reference

All the names below are in tools/mali/testdata. A `.frag` there that equals a
corpus source is that source's reference.

- **ffloor / fceil / ftrunc** (one `bi_fround_to`, bifrost_compile.c ~3275):
  `FROUND.f32.rtn / .rtp / .rtz` (s4_ffloor, s4_fceil; `rtz` in
  intloop_loop_exit), `FROUND.v2f16.rtp` on a mediump pair (alu_loopcarry).
  `ffract` is `FROUND.f32.rtn` then `FADD.f32 x, t.neg` (s4_ffract). No
  reference has a mediump `rtn` or `rtz`.
- **fsin / fcos** (`bi_lower_fsincos_32`, :2552): the same instructions at
  fp32 in both precisions: `FMA.f32` (x times a constant plus a bias),
  `FADD_IMM.f32 #0xC9400000`, `FMA.f32`, `FSIN_TABLE.u6`, `FCOS_TABLE.u6`,
  `FMA_RSCALE.f32`, `FMA.f32 .neg`, `FMA.f32.clamp_m1_1`, `FADD.f32`; cos
  takes the other table result. A mediump operand is `F16_TO_F32` first and
  the result is narrowed with `FADD.f32 r.h0, r, 0x0.neg`. References:
  alu_transc, alu_vtransc (mediump); intuni_sharedint (highp).
- **flog2 / fexp2** (:2915-2933; the `bi_lower_*` forms only with
  `BIFROST_NO_FP32_TRANSCENDENTALS`): log2 is `FREXPE.f32.log`,
  `S32_TO_F32`, `FADD_LSCALE.f32 -1.0, x`, `FLOGD.f32`, `FMA.f32`; exp2 is
  `IADD_IMM 0x18`, `FMA_RSCALE.f32`, `F32_TO_S32`, `FEXP.f32`. Reference
  alu_transc (mediump operands). A highp draw (fmods) is missing.
- **frsq / fsqrt / frcp**: `FRSQ.f32`; `sqrt` is `FRSQ.f32` + `FRCP.f32`
  (s4_fsqrt, intuni_gcops); `FRCP.f32` (s4_frcp); mediump `FRSQ.f16`
  (alu_vtransc).
- **fddx / fddy** (`bi_emit_derivative`, :1507): `CLPER.i32.subgroup4.xor`
  (lane 1 for x, 2 for y) and `FADD right, left.neg`. Unless every use
  ignores the sign (`nir_def_all_uses_ignore_sign_bit`) a sign fix follows:
  `lane_id` (for y first `LSHIFT_AND.i32 lane_id, 2`) shifted by `sz -
  ffs(axis)` and `LSHIFT_XOR`ed into the result. At fp16 the pair is packed
  first (`FMA.v2f16`) and the `CLPER` moves the pair. Reference alu_deriv
  (mediump). Highp (es300/derivs) is the `.i32`/`FADD.f32` form of the same
  sequence; no reference.
- **f2i / f2u / i2f / u2f**: `F32_TO_S32.rtz`, `F32_TO_U32.rtz`, `S32_TO_F32`,
  `U32_TO_F32`; `float(x & 255u)` is `U8_TO_F32 r.b0` (s4_iconv, intuni_gcops,
  intuni_uops). Mediump sources go through `F16_TO_S32.rtz` (intloop_idiv).
- **f2b** (`bool(float)`): NIR's `fneu(x, 0.0)`, `FCMP_OR.<f32|v2f16>.ne.m1`
  by `bi_translate_cmpf`; no reference draws it (the compiler says so at
  `Fne`), so es100/f2b is its reference.
- **integer shifts**: `ishr` is `ARSHIFT_OR.i32 x, n.b00, 0x0` (intuni_ishr);
  `ishl` is `LSHIFT_OR.i32` and `ushr` `RSHIFT_OR.i32`, a constant count
  riding in the byte-select of the second source (intuni_uops,
  intuni_ifelse_uint); `inot` is `LSHIFT_OR.i32 0, 0, x.not`.
- **iabs / imin / imax / umin / umax**: `IABS.s32`; `CSEL.<s32|u32>.<lt|gt>
  a, b, a, b` (intuni_gcops, intuni_uops).
- **udiv / umod / idiv / imod**: a variable divisor is a reciprocal:
  `U32_TO_F32`, `FRCP.f32`, `FMA.f32 rcp, <scale>`, `F32_TO_U32.rtz`, the
  high half of the product through a `U16_TO_U32` split of `IMUL.i32` and
  `IADD.s32`, `IMUL`/`ISUB` for the remainder, then two correction steps
  `ICMP_OR.u32.ge.m1` + `CSEL.u32.eq` (intuni_uops). A signed divide adds
  `IABS.s32`, a `LSHIFT_XOR` of the signs, `ICMP_OR.s32.lt.m1`, `ISUB`,
  `CSEL` (intloop_idiv, mediump). A constant divisor needs no reciprocal: the
  split-multiply chain with the magic constant and `RSHIFT_OR`; `%` is an
  `IMUL` by the divisor and `ISUB` (intuni_ifelse_uint: `% 7u`, `/ 3u`). The
  signed power-of-two divide is in the compiler already.
- **step**: `b2f(fge(x, edge))`, `FCMP_OR.f32.ge.m1` + `CSEL.u32.eq` as
  s4_b2f does for `lt`. layersunused needs its own draw to fix the compare
  direction and the constant arms.
- **any / all of lessThan**: `FCMP_OR.v2f16.lt.m1` per pair, the masks
  combined by `LSHIFT_OR/AND.v2i16` (alu_boolany).
- **dynamic vector subscript**: per lane `ICMP_OR.u32.eq.m1` against the
  index, `MKVEC.v2i16` of the two masks, `MUX.v2i16` chains (intloop_dynidx).

### The references made here

The Mesa 26.2.1 shim is built from `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`
into `$CONNECTED_DEPS_DIR/mesa-pan/shim/build` (`meson setup` with the
script's options plus `-Dlibdir=lib64`, `meson compile`; `inst/lib64` holds
symlinks to libEGL.so.1, libGLESv2.so.2, libgallium and the shim).
`tools/mali/testdata/fsarith_{70_loopcarry,77_transc,77_vtransc,138_deriv}.*`
are copies of the `alu_*` references (same sources, same shim).
`fsarith_199_es3trunc.*` is a new draw of es3trunc_vs.vert +
es3trunc_fs.frag. It shows that **Mesa moves the integer work of 199 into the
vertex stage**: the vertex shader does `F32_TO_S32.rtz`, `LSHIFT_OR`,
`ARSHIFT_OR`, `ISUB`, `S32_TO_F32`, `F32_TO_U32.rtz`, `RSHIFT_OR`,
`U32_TO_F32` on the uniforms and stores floats; the fragment shader loads one
`.v3.src_flat32` varying and runs only `FROUND.f32.rtz` (the second lane on
`r.neg`), `FADD_IMM` 2.0 / 4.0 and `FMA.f32` by 0.125. So 199's fragment side
needs `Ftrunc` plus flat varyings, and its integer ops are a vertex-stage
item (triage-vsarith / triage-intbool), not part of fs-int-ops.

Draws still missing (es300/derivs, es100/f2b, es100/fmods, es300/int_div,
es300/layersunused):

    tools/mali/drm_shim_ref.sh run <shim> V F target/<name>
    python3 -I tools/mali/pan_ref_diff.py extract target/<name>.trace --draw

### Proposed lowering items, by cases and sources unblocked

1. **fs-int-ops**: es300 uops, ifelse_uint, gcops, int_div, es100 int_div.
   (199's shifts are vertex-stage work.) `Ult/Ule`, `Ineg`/`iabs`, min/max, `Inot`, shifts, the
   div/mod family, `F2u`, `U2f`, `U8_TO_F32`. References exist except for the
   two int_div sources. Shares ops with triage-intbool: one owner.
2. **fs-transc**: 77, es100 transc, vtransc, fmods. fsin/fcos/flog2/fexp2 at
   fp32, `Frsq` at fp16. Highp draw for fmods.
3. **fs-round-conv**: 199 (fragment side: `Ftrunc` and a flat varying), f2b, gcops' `floor`, 70 (with
   cflow-flatten): `Ffloor/Fceil/Ftrunc/Fract`, `Fne`, the conversions at
   mediump.
4. **fs-deriv**: 138, es100 deriv, es300 derivs. One case, two sources.
5. **fs-constfold** (f1_fs): the front end folds `log2`, `atan`,
   `inversesqrt` and int casts of constants, as NIR does. intloop_f1_fs.
6. **fs-constvary** (97): fold the vertex stage's constants into the fragment
   shader at link (mali-varying-constants.md); mixprec_varyings15.
7. **fs-dynidx** (dynidx): `Bcsel` on `Ieq` per lane; intloop_dynidx.
