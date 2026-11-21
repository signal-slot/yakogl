<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): intval-rest, the remaining fragment integer and boolean shapes, a lowering design

Plan #387, item `intval-rest-design` (HOST-ONLY, no board). A design for the
later item `intval-rest`; it changes no code, test, reference or tool. It is
the fragment-stage companion of `docs/notes/mali-intval-vs16-lowering.md`
(the vertex stage), and takes the rows `docs/notes/mali-deqp-families.md`
section 18.3 part 1 gives to `intval-rest`: `int_uniform` 101, `ivec_operator`
72, `random_mixed` 40, `bool_uniform` 31, `step_sign` 21,
`bool_literal` / `bool_logic` / `scalar_conversion` 6, the `fs_bool_uniform` /
`fs_int_uniform` rows of `FsUniform` and the `bool_array` / `int_array` rows of
`UniformArray`.

Citations: ours is `<file>:<line>` under `src/shaderc/online/mali/` at the tip
`bf0f3443`; Mesa's words are quoted from the `text` of the references' own
`.expect.json` (Mesa 26.2.1's disassembler), not re-disassembled. A word
pattern marked **(no reference)** is what the Mesa rules predict by analogy
and must be drawn before it is coded (section 7).

## 0. Summary

1. **The fragment `IntValue` of the tip is one catch-all.** A probe run of
   about 180 hand-written fragment shapes through the library at the tip
   (section 2.2) put a line number on every `IntValue` it got: they all came
   from `nir_fs.rs:2414`, the last arm of `Fs::int_lane`. What reaches it is
   three operations: **`ine` of two run-time ints** (`!=` of `int`/`ivec`,
   `notEqual`, `any`/`!=` of ivec), **`iand` / `ior` of two masks** that is not
   the `&&` of two fp32 compares (`bool && bool`, `||`, `(i > 1) && (i < 3)`,
   `u_a && u_b`), and **`i2b` of a run-time int** (`bool(int)`, `bvec2(ivec2)`).
   Everything else the 18.3 rows describe now meets another text first.
2. **The 18.3 counts are stale for this item.** They are the host measure at
   `9ac31f30`, before `intval-reslib` and the unif/mixprec work. At the tip the
   `int_uniform` (101) and `bool_uniform` (31) shapes do not stop at `IntValue`
   any more: `float(int uniform)` stops at `MixedPrecision` (mixcol M1),
   a compare result as a colour product at `ColourForm` (fclane C), `bvec3` /
   `bvec4` / `ivec == ivec` harness shapes at **`Defect(IrIndex)`** (item
   `irindex-fix`), `sign` at `Arithmetic` (arith step 3). Step 0 below is a
   re-measure, not a code step; the case counts of section 6 are bounds from
   the board's case names (section 3.2), not forecasts.
3. **Four steps** (section 6), ranked by the cases each makes compile *through*
   (not by the first refusal it lifts): step 1 `ine` + `i2b` (about 21 named
   cases, up to the 72 + 6 of the families rows), step 2 mask logic
   (`&&`, `||`, with Mesa's `fmin` / `fmax` rule for compares against a
   constant), step 3 the fragment int/bool uniform words and arrays (226 nominal
   cases, which compile through only with `fclane-lower` C and the vertex
   halves), step 4 integer selects. `step_sign` and `random_mixed` have no step of their own:
   they are other items' shapes (section 3.1).
4. **Two compiled-not-Mesa's hazards found** (section 5.4): `!(a == b)` of
   run-time ints compiles at the tip to `ICMP_OR.u32.eq.m1` + `LSHIFT_OR.i32
   .not`, where Mesa's algebraic pass gives `ine`; and the bvec4 / ivec4
   uniform compare chains compile (`c_bvec4`, `c_ivec4`) with no reference to
   say they are Mesa's order. Both go into the references of step 0.
5. **Nothing for `emit.rs` / `bir.rs` / `pack.rs`**: `Alu::Icmp(Ne)` at
   `(32, 1)` (`emit.rs:334`), `LSHIFT_AND/OR.i32`, `ICMP_AND`, the `.not`
   source modifier and the `MUX.v2i16` colour all exist. The steps are `Fs`
   walk, `opt.rs` fuse rules and the `UboLayout::fs16` arms.

## 1. What was read

- `docs/notes/mali-deqp-families.md` sections 4, 6, 18.2, 18.3;
  `mali-deqp-intval.md` (sections 5, 7, 8, 9: the fragment step 1, the order of
  compares measured on Mesa's per-pass NIR); `mali-int-alu.md` (op table),
  `mali-int-uniforms.md`, `mali-compare-forms.md` (`ine`), `mali-deqp-unif2.md`
  and `mali-unif2-lowering.md` (M1, steps 2, 3, 6), `mali-fclane-lowering.md`
  (family C, section 5.2 step 3), `mali-mixcol-lowering.md` (1.2 M1),
  `mali-arith-lowering.md` (steps 3 and 5), `mali-intval-vs16-lowering.md`.
- The references (all `tools/mali/testdata/`): `deqp_intval_opfs_*` (8),
  `deqp_intval_reslib_{bvec,float,int,mixed}_fs`, `deqp_unif2_{fs_bool_uniform,
  fs_int_uniform,arr_bool_array,arr_int_array}`, `deqp_mixcol_int_uniform`,
  `deqp_mixcol_int_uniform_struct`, `deqp_mixcol_hp_bool`, `cmp_ine`,
  `deqp_misc_cmp_ine0_hp`, `deqp_arith_op_selection_select_fs_none`,
  `deqp_arith_op_binary_not_equal_vs_mediump`, `deqp_fclane_bcast_compare_*`.
  There is **no `deqp_intval3_*` and no `mali-deqp-intval3.md` on this tip**
  (item `intval3-ref` is in review); this note does not use them.
- The compiler: `nir_fs.rs` `int_lane` :2258-2414, `mask_op` :2224,
  `and_masks` :2128, `mask16` :2103, `bool32` :2174, `Fs::op` :1393,
  `widen_local` :2800, the compare arm :5960-5989; `select.rs` `UboLayout::fs16`
  :578-662; `nir.rs` `uniform_of` :1045-1066 (`UniformArray` :1062);
  `mod.rs` :1425-1470; `emit.rs` :82, :145-167, :334, :433;
  `tests.rs` :1263 (the gap probe).
- Counts by case name: `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/
  refusals.tsv` (the board's text per case, at the old tip): fragment cases
  with "an integer or boolean value" are 1,902; section 3.2 groups them.

## 2. Where the fragment refuses, and what the tip compiles

### 2.1 The sites of `MaliError::IntValue` (fragment walk)

| site | what it checks | what lands there |
|---|---|---|
| `nir_fs.rs:2414` | last arm of `Fs::int_lane` (the 32-bit and mask int ops of the mediump walk) | `Ine` of two ints; `Iand` / `Ior` other than `and_fcmp32`'s pair; `I2b` of a non-uniform (`imin`/`imax`/`iabs` of ints meet `Arithmetic` in the probe, not this arm) |
| `nir_fs.rs:2232` | `Fs::mask_op`: two constants, two 32-bit masks, two 16-bit masks; any other mix | a 16-bit compare mask with a 32-bit compare or a bool word |
| `nir_fs.rs:2321` | `Bnot`/`Inot` of a 16-bit mask that is not an `flt`/`fge` compare | `!(a == b)` of two 16-bit masks, `!(x != y)` |
| `nir_fs.rs:1416` | `fmul(b2f(m), b)` where `b` is a constant or another `b2f` | a product of two selects the `cmp_product` rule does not fold |
| `nir_fs.rs:2806` | `widen_local`: no slot in the order for the `f2f32` | `isOk(float a, int b)` shapes with a local in another place |
| `nir_fs.rs:5989` | compare with an int's float on one side and an operand that is neither an int's float, a widened local nor a constant | `float(b) <= x` with a varying |

The refusals the 18.3 rows still name for the uniforms are not `IntValue`:

| text | site | what it checks |
|---|---|---|
| `FsUniform` | `select.rs:604-632` (`UboLayout::fs16`) | `GL_INT`, `GL_BOOL`, `*_VEC*` arms need `array_len <= 1`; a mediump int/bool/ivec/bvec **array** reaches `_ => Err(FsUniform)` (:621); a highp-declared float array :632 |
| `UniformArray` | `nir.rs:1062` (`uniform_of`; the fragment walk reaches it through `nir_fs.rs:7436` `uniform_extent`) | `array_element(u)` takes float, vec2, vec3, vec4 only (`nir.rs:1103`); the **vertex** stage of every `bool_array` / `int_array` case |
| `UniformType` (vs) | `nir.rs:4456` | the vertex walk: `vs16-2`'s (not this item's) |

### 2.2 What compiles at the tip (probe)

Method: a scratch integration test (deleted; nothing of it is in this change)
called `tests/mali_ref`'s `compile_pair` on pairs written by hand after the
dEQP templates (`shaders.operator.*` `in0`/`in1`/`res`/`color` fragments,
`uniform_api` `compare_*` harness, `shaders.conversions`, `shaders.swizzles`
`isOk` harness; the ES 2 sources are generated by dEQP's C++ and are not in
the tree, so these are **approximations of the shapes**, not the cases), with
a temporary `eprintln!(line!())` at each `IntValue` of `nir_fs.rs` (reverted;
`git diff` empty). About 180 pairs; `mediump` varyings, the passthrough or
`v_vtxOut = 1.0` vertex stage. The table is the outcome, not a word compare.

| shape (probe) | first refusal at the tip | owner of what comes next |
|---|---|---|
| `lessThan`/`<=`/`>`/`>=` of `int`, `ivec2..4` (`ivop_lt_*`, `ivop_le_ivec4`, `i_lt_vec`, `i_ge_vec`, `w_lt_int`, `w_ge_int`, `w_gt_int`) | compiles | words not byte-compared except `opfs_cmp_ivec3` |
| `==` of `int`, `ivec4` (`ivop_eq_*`, `w_eq_int`) | compiles | as above |
| **`!=` / `notEqual` of `int`, `ivec2..3`** (`ivop_ne_*`, `i_ne_*`, `w_ne_i`) | **`IntValue` :2414** | step 1 |
| `bool(int)`, `bvec2(ivec2)` (`cv_bool_i`, `cv_b_i`, `cv_ivec2_b`, `w_i2b`) | **`IntValue` :2414** | step 1 |
| **`bool && bool`, `||`** of fp16 compares or int compares (`bl_and`, `bl_or`, `w_and16`, `w_or16`, `r_cmp_chain`, `r_cmp_or`, `boollogic_*`) | **`IntValue` :2414** | step 2 |
| `u_a && u_b`, `(u_a == -9) && (u_b == 2) && (u_c == -2)` of separate uniforms (`ub_and`, `c_bool3`, `c_int3`, `c_bool_and`, `c_bool_or`) | **`IntValue` :2414** | step 2 |
| `^^`, `!`, `bool == bool` of compares (`bl_xor`, `bl_not`, `bl_ne`, `conv_bool_cmp_eq`) | compiles (`opfs_logical_xor`, `opfs_not`, `opfs_equal_bvec2`) | pinned |
| `!(i == 2)` (`w_not_ine`) | compiles **as `ICMP_OR.u32.eq.m1` + `LSHIFT_OR.i32 .not`** | step 1 (hazard, 5.4) |
| `bvec2` `==`/`!=`/`all`/`any`/`not`/`equal`/`notEqual` of `greaterThan` (`bc_*_2`), `all`/`any` of width 3, 4 | compiles | pinned by `reslib_bvec_fs`, `opfs_equal_bvec2` |
| `bvec3`/`bvec4` `==`, `!=`, `not`, `equal`, `notEqual` (`bc_*_3`, `bc_*_4`) | **`Defect(IrIndex)`** | `irindex-fix`, then references (7) |
| `x ? 1.0 : 0.0` of an int compare (`i_eq_const`, `i_ge`, `i_gt`, `i_le`, `i_lt_const`) | **`Defect(IrIndex)`** | `irindex-fix` |
| `isOk(ivec4 / bvec4, ..)` swizzle harness (`hs_int_ivec4_gggg`, `hs_bool_bvec4_gggg`, `hs_int_ivec2_gg`), `bvec3(vec3)` (`conv_bvec_of_vec`), `step(vec3|vec4)` | **`Defect(IrIndex)`** | `irindex-fix` |
| `isOk(int, int)`, `isOk(bvec2, bvec2)` harness (`hs_int_int`, `hs_bool_bvec2`) | compiles | |
| `compare_int` / `compare_ivecN` / `compare_bool` / `compare_bvecN` of a scalar or vector uniform, `result *= ..` into `vec4(result, result, result, 1.0)` (`ua_int`, `ua_ivec2..4`, `ua_bool`, `ua_bvec2..4`, `uc_*`) | **`ColourForm`** | `fclane-lower` C; its compare is the one `c_*` compiles |
| the same `compare_*` of a uniform **array** (`ua_int_arr3`, `ua_bool_arr3`, `ua_ivec2_arr2`, `ua_bvec2_arr2`) | **`FsUniform`** (`select.rs:621`) | step 3 / unif2 step 3 |
| `bool RES = (u_a == ..)` in `vec4(RES, RES, RES, 1.0)` of an `int`, `ivec2`, `ivec4`, `bvec2`, `bvec4`, two ints, `<` (`c_int1`, `c_ivec2`, `c_ivec4`, `c_bvec2`, `c_bvec4`, `c_int_two`, `c_int_lt`, `c_bool_one`, `c_bool_not`) | compiles | **no reference**: unverified (5.4) |
| `float(uniform int)`, `float(int) * varying` (`uc_float_of_int`, `ui_float_of_int`, `c_int_float`) | `MixedPrecision` | `mixcol-lower` M1 (`mixcol_int_uniform`) |
| `step(a, b)` mediump float / vec2 | compiles | |
| `step` highp | `MixedPrecision` (float, vec2), `IrIndex` (vec3, vec4) | mixcol, irindex |
| `sign(x)`, `abs(int)`, `min`/`?:` of ints (`sign_*`, `i_abs`, `ivop_min_int`, `i_sel`), `bool ? f : f` (`bl_sel`, `ub_select`, `ui_select`) | `Arithmetic` | `arith-lower-1`: sign (step 3), `bcsel` (step 5) |
| `int` arithmetic into the colour lanes (`ivop_sub_ivec2`, `ivop_neg_ivec3`, `ivop_postinc_int`, `ivop_addassign_ivec3`, `ivop_mulc_ivec4`, `cv_ivec2_f`, `i_mod`) | `ColourForm` | `fclane-lower` (E, B) |
| `if (ui_one > 0)` | `ControlFlow` | `ctrlflow` |

## 3. The shapes: what each 18.3 row becomes

### 3.1 Row by row

| row (18.3 part 1) | cases | at the tip (2.2) | goes to |
|---|--:|---|---|
| `ivec_operator` | 72 | `!=` and `notEqual` of int/ivec (`IntValue`); the `+ - * ++ -- +=` forms meet `ColourForm`; `/` and `%` meet the idiv step 1 took (`opfs_div_*`) or `ColourForm`; relational/`==` compile | **step 1** (`ne`); the rest fclane |
| `random_mixed` | 40 | generated programs: `bool(1)`, `int(c)`, `float(-2.0)` of a varying through a `vec3` constructor and `textureCube(..)`; `(i > 1) && (i < 3)`; `bool(float)` chains | **steps 1 and 2** for the `IntValue` part; then `TexKind` / `ColourForm` (not intval) |
| `int_uniform` | 101 | `uniform int` as a value: `float(u)` is `MixedPrecision`, `u == k` into a float colour `ColourForm`, `ivec == ivec` harness `IrIndex` | mixcol M1, fclane C, irindex-fix; **step 3** for the compare words |
| `bool_uniform` | 31 | `bvecN ref_out0 == out0` harness: width 2 compiles, width 3, 4 `IrIndex`; the `uniform bool` `compare_bool` `ColourForm` | irindex-fix, fclane C; **step 3** for `&&` of bool words |
| `step_sign` | 21 | `step`: mediump float/vec2 compile, highp `MixedPrecision`, vec3/vec4 `IrIndex`; `sign`: `Arithmetic` | **none of this item**: mixcol, irindex-fix, `arith-lower-1` step 3 |
| `bool_literal` (6 with the next two) | | `t ? a : b` with `t = true` compiles; `if (false, ..)` `ControlFlow` | none (`ctrlflow`) |
| `bool_logic` | 2 | `&&`/`||` of float compares in an `if`: `IntValue`, then `ControlFlow` | **step 2**, then `ctrlflow` |
| `scalar_conversion` | 6 | `bool(int)` of a run-time int | **step 1** (`i2b`) |
| `fs_int_uniform` (`FsUniform`) | 52 | `uniform mediump int u_var[3]` etc. (`FsUniform` :621) | **step 3** |
| `fs_bool_uniform` (`FsUniform`) | 33 | `uniform bool u_var[3]` | **step 3** |
| `int_array`, `bool_array` (`UniformArray`) | 47, 90 | the **vertex** stage's `uniform_of` first; the fragment half is `FsUniform` | vertex: unif2 steps 3 and 6 with `vs16-2`; fragment: **step 3** |

### 3.2 Case names (the board's old tip)

`refusals.tsv` of the four 2026-11-15 slices, first refusal "an integer or
boolean value" in a fragment shader (1,902 cases at the old tip; step 1 of
intval took most). The names of the groups this item's steps move, by dEQP
group (`dEQP-GLES2.functional.` removed):

| group | fs cases | step | note |
|---|--:|---|---|
| `shaders.operator.binary_operator.not_equal` | 16 | 1 | the int and ivec types of it (12 of 16 by the dEQP type list: 3 precisions x {int, ivec2..4}; verify with `--deqp-runmode=txt-caselist`) |
| `shaders.operator.int_compare.notEqual` | 9 | 1 | `ivec2..4` x 3 precisions |
| `shaders.operator.binary_operator.logical_and` / `logical_or` | 1 + 1 | 2 | `logical_xor` is `opfs_logical_xor` |
| `shaders.operator.bool_compare.{all,any,equal,not,notEqual}` | 15 | none | width 3, 4 `IrIndex` (probe) |
| `shaders.operator.selection.*_int*` | 12 | 4 | `bcsel` of ints |
| `shaders.operator.sequence.{no_,}side_effects` | 15 | 4 / fclane E | |
| `shaders.operator.common_functions.step` | 21 | none | |
| `shaders.conversions.*` (vector_combine 78, vector_to_vector 46, vector_to_scalar 39, scalar_to_vector 23, scalar_to_scalar 13, matrix_combine 12) | 211 | 1 (`i2b` of the `bool(int)` forms), irindex-fix (bvec3/4, ivec swizzles), fclane | the bool-from-int share is not countable from names |
| `shaders.swizzles.vector_swizzles` | 306 | none | the `isOk(ivecN/bvecN, ..)` harness: `IrIndex` at width 3, 4 |
| `shaders.random.*` | about 200 | 1, 2 (the `IntValue` part) | `texture` 55 meets `TexKind`, `conditionals` 23 `ControlFlow` |
| `shaders.conditionals.if`, `shaders.functions.control_flow` | 11, 12 | 2 | `ControlFlow` next |

**The named lower bound of step 1 is 21** (`not_equal` 12 + `notEqual` 9); the
families' 72 + 6 is the upper bound. The step 0 measure replaces both.

## 4. How Mesa 26.2.1 compiles them (from the references)

### 4.1 `ine` of an int (`ICMP_OR.u32.ne.m1`)

Ints are 32 bits whatever the precision (`caps->int16` false), so the compare
is the highp compare. The references:

```
; cmp_ine   (highp)   float(m != 3), m uniform int
MOV.i32.discard r0, 0x0                          ; the accumulator, a register: the immediate below takes the constant slot
ICMP_OR.u32.ne.m1 r0, u2.w0, 0x3020100.b3, r0^   ; m != 3: the constant 3 as byte 3 of the table word 0x3020100
MUX.v2i16.wait0126 r0, 0x0, 0x3C000000.h11, r0^.h00
; deqp_misc_cmp_ine0_hp   float(m != 0)
ICMP_OR.u32.ne.m1.discard r0, u2.w0, 0x0, 0x0    ; a zero operand: the constant slot is free for the accumulator 0x0
; deqp_arith_op_binary_not_equal_vs_mediump   ivec2(a_in0.wy) != ivec2(a_in1.yx) (vertex, mediump attributes)
F32_TO_S32.rtz r1, ..  ; per lane
ICMP_OR.u32.ne.m1 r0, r1^, r2^, r0^              ; the second lane's compare accumulates the first's mask (`any`)
```

So `ine(a, b)` of two run-time ints is `ICMP_OR.u32.ne.m1 d, a, b, 0x0` (`acc` a
register when the instruction carries an immediate or a second FAU word), and
`any(notEqual(ivecN, ivecN))` / `ivecN != ivecN` is the **accumulator chain**
(`opt.rs` `va_fuse_cmp` makes the `ior` of two compares one `ICMP_OR` with the
first's mask as the fourth source, as it already makes `ICMP_AND` for
`iand`). `bool(int)` is `ine(x, 0)`: `ICMP_OR.u32.ne.m1 d, x, 0x0, 0x0` (the
`cmp_ine0_hp` word). **No fragment reference draws it at mediump**: the
operands' narrowing (`FADD.f32 .h0`, `FMA.v2f16`, `F16_TO_S32.rtz`, as the
probe `w_eq_int` shows for `int(c * 4.0) == int(b.x * 4.0)`: `ICMP_OR.u32.eq.m1
r0, r0^, r1^, 0x0`) is the path `ieq` already takes, and only the compare row
should change. **(no reference)** for the fragment
`!=` of two run-time ints, of an ivec, and of `bool(int)`.

`!(a == b)`: `nir_opt_algebraic.py` folds `inot` of a comparison to the dual
comparison (the fragment walk's `Bnot` arm already relies on `flt`/`fge`,
`nir_fs.rs:2318`); the `ieq` -> `ine` and `ilt` -> `ige` rules are of the same
family but were **not read from the Mesa source here**: the reference of
section 7 row 5 settles it. If they hold, Mesa emits `ICMP_OR.u32.ne.m1` for `!(i == 2)`;
the tip's `w_not_ine` words are `ICMP_OR.u32.eq.m1 r0, r0^, 0x3020100.b2, 0x0`
then `LSHIFT_OR.i32 r0, 0x0, 0x0.b00, r0^.not`. **(no reference; hazard 5.4)**

### 4.2 A product of compares into a colour: the three-mask chain (`fs_int_uniform`, `arr_int_array`)

`result = v_vtxOut; result *= compare_int(u_var[i], k) ..` is the product of
three `b2f` selects, which Mesa makes `bcsel(iand(m0, m1, m2), v, 0.0)` and the
masks one chain:

```
; deqp_unif2_fs_int_uniform   u_var[0] == -9, u_var[1] == 2, u_var[2] == -2   (the varying folded to the constant 1.0)
MOV.i32.discard r0, 0x0
ICMP_OR.u32.eq.m1 r0, u2.w1, 0x3020100.b2, r0^   ; u_var[1] == 2 first (the order: nir_opt_move's, section 4.7)
ICMP_AND.u32.eq.m1 r0, u3.w0, u3.w1, r0^         ; u_var[2] == -2, the constant a FAU word of its own; AND into the mask
MOV.i32 r1, u4.w0                                ; -9: a second FAU word in one instruction needs a register
ICMP_OR.u32.eq.m1 r1, u2.w0, r1^, 0x0            ; u_var[0] == -9
LSHIFT_AND.i32 r0, r0^, 0x0.b00, r1^             ; the two masks
MUX.v2i16.wait0126 r0, 0x0, 0x3C000000.h11, r0^.h00   ; b2f: 0.0 / 1.0 (the constant 1.0 is the varying)
ATEST ..., then IADD.v2u16 r1, r0.h00, 0x0 ; MKVEC.v2i16 r2, r0^.h0, u4.w1.h0 ; MOV r0, r1 ; MOV r1, r2   ; fclane C
; deqp_unif2_arr_int_array   mediump int u_var[3]; -2, 4, 0; the varying is a real flat varying
ICMP_OR.u32.eq.m1 r0, u2.w1, 0x7060504.b0, r0^   ; == 4
MOV.i32 r1, u3.w1 ; ICMP_OR.u32.eq.m1 r1, u2.w0, r1^, 0x0     ; == -2 (the FAU word fffffffe)
ICMP_AND.u32.eq.m1 r0, u3.w0, 0x0, r0^           ; == 0
LSHIFT_AND.i32 r0, r0^, 0x0.b00, r1^
LD_VAR_BUF_IMM.f32.slot0.src_flat32.center.store.wait0 @r1, 0x0, index:0x0 ; FADD.f32 r1.h0, r1^, 0x0.neg
MUX.v2i16 r0, 0x0, r1^.h00, r0^.h00              ; bcsel(mask, varying, 0.0): Fs::op's `fmul(b2f(m), b)` rule (nir_fs.rs:1409)
```

The FAU of the two references is *not* the values the draw gave: `gl_ref.py`
cannot set an element of an array (all array words 0; `u3.w1` is the constant
-2, `u4.w0` -9); `unif2-lower-1`'s step U0 redraws them. The walk has every
piece of this at the tip for non-array uniforms: `c_int_two`, `c_ivec2`,
`c_ivec4` (compare of an `int_word` with `Const` or another word through
`int32(Icmp(Eq))`, `va_fuse_cmp`, the byte-immediate table, `MOV.i32` for a
second FAU word); what is missing is the `iand` of three masks
(`c_int3`: `IntValue`, step 2), the array layout (`FsUniform`, step 3) and
the colour (`ColourForm`, fclane C).

### 4.3 Bool uniforms (`fs_bool_uniform`, `arr_bool_array`)

```
; deqp_unif2_fs_bool_uniform   compare_bool(u_var[i], true) x3   ==  u_var[0] && u_var[1] && u_var[2]
MOV.i32.discard r0, 0x0
LSHIFT_AND.i32 r0, u2.w1, r0^.b00, u2.w0          ; two FAU words: the zero shift byte comes from a register
LSHIFT_AND.i32 r0, r0^, 0x0.b00, u3.w0
MUX.v2i16.wait0126 r0, 0x0, 0x3C000000.h11, r0^.h00
; deqp_unif2_arr_bool_array   [0] == false, [1] == true, [2] == true (the MOV.i32 r0, 0x0 first, as above)
LSHIFT_AND.i32 r0, u2.w1, r0^.b00, u2.w0          ; two words ANDed
LSHIFT_OR.i32 r1, 0x0, 0x0.b00, u3.w0.not         ; the `== false` operand: `.not` of its word (a move of it with the not)
LSHIFT_AND.i32 r0, r0^, 0x0.b00, r1^              ; (which element is which FAU word is the use order, not the index order)
LD_VAR_BUF_IMM.f32...src_flat32 ; FADD.f32 r1.h0 ; MUX.v2i16 r0, 0x0, r1^.h00, r0^.h00
```

A bool uniform word is a 32-bit mask (true `~0`); `ieq(b, true)` is the word,
`ieq(b, false)` is `.not`. The walk's `bool_word` / `mask_lane` (`nir_fs.rs:2285`,
`:2351-2380`) has both, and `c_bvec2` / `c_bvec4` already emit this chain
(`LSHIFT_OR.i32 .., u2.w1.not`, `LSHIFT_AND.i32`). What fails is the scalar
`&&` of two such words (`c_bool_and`: `Iand` reaches :2414). **The word a GL
`true` writes is 1, not `~0`** (`mali-unif2-lowering.md` section 5, step 2 "D1"):
the `.not` form is wrong with it; D1 lands before step 3 or with it.

### 4.4 `&&`, `||` of two masks (`bool && bool`)

The only fragment references of an `iand` are the shader library's fp32 pairs
(`reslib_int_vs`: `FCMP_AND.f32`), `reslib_mixed_fs` (`ICMP_AND.u32.eq.m1`) and
the vertex `opvs_logical_and`:

```
; deqp_intval_opvs_logical_and  (vertex, mediump attributes)   a_in0.z > 0.0 && a_in1.x > 0.0
FMIN.v2f16 r0, r0^.h00, r1^.h00        ; iand(flt(0, a), flt(0, b)) -> flt(0, fmin(a, b))
FCMP_OR.v2f16.lt.m1 r0, 0x0, r0^.h00, 0x0
; deqp_arith_op_bool_any_vs_none : the same with FMAX.v2f16 for `ior`
```

`mali-intval-vs16-lowering.md` section 4.1 and 4.6 have the full words. The
fragment walk's `Fs::and_masks` (`nir_fs.rs:2128`) has the rule but **for the
constant-second form only** (`flt(x, c) & flt(y, c)` -> `flt(fmax(x, y), c)`,
nir_opt_algebraic.py:1159); the `a > 0.0` of a dEQP `bool_logic` shape is
`flt(0, a)`, the constant first, where Mesa takes `fmin` (and `ior` takes the
mirror, `fmax`). Today `and_masks` would fall to `mask_op` and emit a plain
16-bit `LSHIFT_AND.v2i16`, not Mesa's: that is the other reason `Iand` is not
routed in `int_lane`, and the rule's missing half is part of step 2. For the
32-bit masks of int compares (`(i > 1) && (i < 3)`) and of bool words Mesa has
no float rule: the `ICMP_OR` / `ICMP_AND` accumulator chain of 4.2 and
`LSHIFT_AND.i32`. **(no reference)** for the fragment `&&`/`||` of fp16
compares, of two int compares, and of a 16-bit mask with a 32-bit one.

### 4.5 Conversions: what `int(float)`, `float(int)`, `float(bool)` already have

`F32_TO_S32.rtz` of a varying lane, `S32_TO_F32` + `FADD.f32 .h0`, the
`MUX.i32`-style `b2i` and `b2f` as the colour's `MUX.v2i16`, `bool(float)` as
`FCMP_OR.v2f16.ne.m1`, `bvec(vec)` pairs: all of `deqp_intval_opfs_*`,
`reslib_*_fs`, `mali-int-alu.md` (op table, 2). They are pinned
(`tests/mali_deqp_intval.rs`). The only conversion the fragment walk lacks is
`i2b` of a run-time int (4.1).

### 4.6 What a lane colour needs from the mask (fclane C, not this item)

`MUX.v2i16 r0, 0x0, 0x3C000000.h11, mask.h00`, the `IADD.v2u16 r1, r0.h00,
0x0` / `MKVEC.v2i16` / `MOV` broadcast of `bcast_compare_int` and
`bcast_compare_bool`. `fclane-lower` C consumes the mask `Val`s of this item:
section 8.

### 4.7 The order of compares

Measured in `mali-deqp-intval.md` section 7 ("The order of compares", NIR_DEBUG=
print_fs on the shim): `pan_nir_lower_bool_to_bitsize` runs before
`nir_opt_sink`/`nir_opt_move`; each compare lands at its first user; two that
share a first user swap at each of the two `nir_opt_move` runs, so the final
order is the program order for two and **needs the same measure for three**
(the `fs_int_uniform` chain above: u_var[1], u_var[2], u_var[0]: not program
order). `Fs`'s `move_compares` and `reduction_order` carry the rules; the
3-mask `iand` is the one shape no reference has drawn *except* these two.

## 5. The mechanisms

### M1. `ine` and `i2b` of run-time ints (step 1)

*What.* `int_lane`: an `AluOp::Ine` arm beside `Ieq` (:2388),
`self.int32(Alu::Icmp(Cmpf::Ne), [p, q, u], at)`; `AluOp::I2b if int(self, p)`
as `ine(p, 0)`; the 32-bit `Bnot`/`Inot` of an int compare folded to its
dual (`Icmp(Eq)` <-> `Icmp(Ne)`, `Lt` <-> `Ge`) as :2318 does for the 16-bit
float compares; `any`/`!=` of an ivec through the existing Bany/Ball arm
(`reduction_order`) and `va_fuse_cmp` for the `ior` accumulator.
*Why no emit work.* `emit.rs:334` builds `IcmpOrU32` for `Eq | Ne` at `(32, 1)`.
*Words.* 4.1. *Unblocks.* section 3.2: 21 named (up to 78 by the families
rows). The gap probe `tests.rs:1263` (`float(i != j)`) **compiles after this
step**: it must be moved in the same commit to a gap still refused with this
text. `iand` of two masks is refused until step 2, so the probe can be
`bool(v.x > 0.0) && (int(v.y) < 1)` (a 16-bit mask with a 32-bit one: the mix
of `mask_op` :2232, which step 2 leaves refused for want of a reference) and
then needs no second move.

### M2. `iand` / `ior` of masks (step 2)

*What.* `int_lane` arms `Iand`/`Ior`: both 16-bit masks through `and_masks`
extended with the constant-first rule (`flt(0, a) & flt(0, b)` -> `flt(0,
fmin(a, b))`; `ior` -> `fmax`, with the same `c == d` test) and `mask_op`; both
32-bit masks (int compares, bool words, `fcmp32`) `int32(Iand/Ior)` which
`va_fuse_cmp` folds to the `ICMP_AND`/`ICMP_OR` accumulator form (the pair of
compare + bool word: `LSHIFT_AND.i32 d, u_a, 0x0.b00, u_b`); the 16/32 mix
(`mask_op` :2232) only with a reference. `Bnot`/`Inot` of a non-compare mask
(:2321, `!(a == b)` of two masks, `!(x != y)`) with the `ine`/`ieq` fold.
*Words.* 4.2-4.4. *Unblocks.* `bool_logic` 2, `logical_and`/`or` 2, the random
`conditionals`, the `&&` of the `compare_bool` chain; mostly a prerequisite
(fclane C `bcast_compare_float` products of masks, step 3).

### M3. The fragment int/bool uniform words, arrays (step 3)

*What.* `UboLayout::fs16` (`select.rs:604-632`): drop `if u.array_len <= 1`
from the `GL_INT | GL_BOOL` and `*_VEC*` arms so an array is rows of words
(each element one split row, the constant index folds into the offset:
`deqp_unif2_arr_*` FS: `u2.w0`, `u2.w1`, `u3.w0`); `nir::array_element` (`nir.rs:1103`)
takes `GL_INT`, `GL_BOOL` and the vectors with the stride rule of the float
vectors (the same edit `mali-unif2-lowering.md` step 3 lists; whoever lands
first owns it, the other rebases); the `int_word` / `bool_word` predicates read
an element; the `MOV.i32 rN, uX.wY` for a second FAU word or a negative
constant (done for `c_ivec2`).
*Words.* 4.2, 4.3. *Un-ignores.* `deqp_unif2_fs_int_uniform`,
`deqp_unif2_fs_bool_uniform`, and the fragment half of `arr_int_array`,
`arr_bool_array` (vertex halves: unif2 steps 3 and 6, `vs16-2`); each is
`LOWERED` only when fclane C and the other stage are in. *Unblocks.* 226 cases
nominal (`fs_int_uniform` 52, `fs_bool_uniform` 33, `int_array` 47, `bool_array`
90 of the families' rows; the unif2 note counts 226 as 95 + 46 + 52 + 33).

### M4. Integer selects (step 4)

*What.* `bcsel` of int/bool arms in the fragment (`selection.*_int*` 12, `sequence`
side effects, `res = b ? i : j`), `CSEL.u32.eq` / `MUX.v2i16`. The bool
select is `arith-lower-1`'s step 5 (`op_selection_select_fs_none`, the `MUX.v2i16
.. r1^.h00, r0^.h00, r2^.h00` form); the int one has **no reference**. Take it
only if step 5 of arith has not landed.

### 5.4 Compiled at the tip, byte-compare unknown (hazards)

The probe found shapes that **compile and are not pinned**: a lowering that
lands on them can make them "compiled, not Mesa's" (the project's rule) or
leave one so. Step 0 draws them (7):

1. `!(i == 2)` of an int: ours `eq` + `.not`; Mesa's `ine` (4.1).
2. `bool RES = (u_a == ivec4(..))` (`c_ivec4`), `bvec4` (`c_bvec4`): the chain
   order (our `LSHIFT_OR .not` first, `MOV r1, 0x0` placement) is the walk's
   own; width 2 matches the pattern of `fs_bool_uniform`, 3 and 4 are not drawn.
3. `uniform int` `compare_int(u_var, k)` scalar (`c_int1`): the byte-immediate
   table for k = 4 (`0x7060504.b0`) follows `fs_int_uniform`'s 2, 3 and
   `cmp_ine`'s 3 but not 4 and above: the constants 4..15 and 16+ (FAU word)
   need the table's rule drawn once (`0xB0A0908.b1` for 9 appears in
   `intuni_*`).
4. ints through `int_word` in `Icmp(Lt/Ge)` with a uniform (`c_int_lt`).
5. `bvec3`/`bvec4` compares (`IrIndex` today; references once it compiles).
6. `ivec` `==` as `ball(ieq)` of width 3, 4 in the fragment (`reslib_mixed_fs`
   draws 3).
7. `x ? 1.0 : 0.0` of an int compare (`IrIndex` today).

## 6. The steps, one session each, ranked by cases

"Net" is the cases whose compile gets through the step with the partner
families as they are at the tip; "nominal" is the families' first-refusal
count. Both are bounds to be replaced by step 0's measure.

| # | step | nominal | net (bound) | needs first | un-ignores | new references |
|--:|---|--:|--:|---|---|---|
| 0 | measure and draw (no compiler change; the first hour of the session of step 1) | | | `irindex-fix` merged; `deqp-host-measure.sh --filter` of the groups of 3.2 at the new tip | none | `deqp_intrest_*`, section 7 (22 pairs) |
| 1 | M1: `ine`, `i2b`, the 32-bit `Inot` fold, ivec `!=` | 78 | 21..78 | step 0's references 1-5 (6 pairs) | none in the tree (adds its own) | 6 |
| 2 | M2: `iand`/`ior` of masks, the constant-first `fmin`/`fmax` rule | about 40 | 4..20 (plus the chain steps 3 and fclane C need) | step 1 (the `Ine`/`Icmp` masks the chains read), references 6-8 | none; `opvs_logical_and` stays `vs16-1`'s | 4 |
| 3 | M3: fragment int/bool uniform arrays and compare words | 226 | 0 until fclane C and `unif2` D1; then up to 85 (`fs_*` 52 + 33) and 137 with the vertex halves | steps 1, 2; `unif2-lower-1` (U0 redraw), D1, `fclane-lower` C | `unif2_fs_int_uniform`, `unif2_fs_bool_uniform`, the FS half of `arr_*` | 8 |
| 4 | M4: int selects | 12..27 | 12 | `arith-lower-1` step 5 not yet in | none | 2 |

Why this order. Step 1 is cheapest, has the named cases (21) and the gap probe,
and its mask is what 2 and 3 read. Step 2's net is small by itself but both
fclane C (`bcast_compare_*`, 229) and step 3 stand on a mask `&&` (the unif2
FS references are three-mask products). Step 3 has the largest nominal number
and the largest *dependency*: its 226 are fragment words, and every one of the
four references also needs the colour broadcast of fclane C (`IADD.v2u16` /
`MKVEC` / `MOV`), and the array ones the vertex halves; it is last for that,
not for size. If `fclane-lower` C lands before step 3, step 3's net becomes its
nominal at once. Each step is one session: one `Fs` arm set, one layout
change, 2-5 references, one new test file.

**Per-step contract.** Tests: `tests/mali_deqp_intrest.rs` (new; the
`mali_deqp_*` name the CI step runs, the `deqp_intrest_` prefix keeps the
references apart from `deqp_intval_*`), `LOWERED` / `REFUSED` pinned as the
other `mali_deqp_*` tests, header:
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_intrest`.
Regenerate in the same commit: nothing generated (no `internal.rs`); the
`tools/mali/test_pan_ref_diff.py` case-list pin (`test_ref_cases_lint`) if
`cases.txt` rows are added (step 1 of `intval` was red there once).
Run locally only the tests added or changed and `cargo fmt --check` on the
touched files (CLAUDE.md).

### 6.1 Step 1 in detail

Files: `nir_fs.rs` (`int_lane`, :2258-2414: `Ine`, `I2b`, `Inot` of a 32-bit
int compare; the reduction arm), `opt.rs` (`va_fuse_cmp` `ior` accumulator,
check first that it already fuses `ICMP_OR` of two `ne`: the vertex
`arith_op_binary_not_equal_vs_mediump` does), `tests.rs:1263` (the gap probe),
`mali_deqp_intrest.rs`. Pinned tests most at risk: `mali_deqp_intval.rs` (all
22), `mali_deqp_misc.rs` (`cmp_ine0_hp`, `cmp_notequal_vec2`, `cmp_fne_hp`),
`mali_deqp_arith.rs` (`op_binary_not_equal_fs_*` float `!=`: unchanged),
`mali_deqp_mixprec.rs` (`cmpv*` products), `src/shaderc/online/mali/tests.rs`
(`every_reachable_gap_is_reached_by_an_es100_source`, the `intbool_*` and
`intloop_*` pins), `tests/mali_glmark2_corpus.rs` and `corpus-verdicts.sh
--check` (a `!=` of ints in `loop_*` / `tri_int` moves from `IntValue` to a
later refusal: `tri_int.frag`'s `ine` is the named one in `mali-deqp-intval.md`
section 7).

## 7. References drawn (intval4-ref), and what they showed

Drawn by plan #387 item `intval4-ref` with Mesa 26.2.1's G610 drm-shim built
by `tools/mali/drm_shim_ref.sh build` (LLVM 21), one pair each through
`tools/mali/deqp_ref.sh <vert> <frag> tools/mali/testdata/deqp_intval4_<name> --instrument shim`
(`--uniform-values` is part of the shim instrument). The pairs are not the
cases' own `.qpa` text (no CTS run was made here, the checkout holds only
`data/`): they are written after the dEQP-GLES2 generator templates the earlier
references quote verbatim (`deqp_intval3_*`, `deqp_unif2_*`), in the same
spelling, so a `random` or hand-written shape says so. They are pinned by
`tests/mali_deqp_intval4.rs` (`REFUSED` / `LOWERED` / `DIFFERS`, one `#[ignore]`d
byte-compare per reference not yet matched, `refusals_pinned`).

Rows already held by an existing reference were **skipped** (no new draw).

| # | name | shape (dEQP case) | for step | drawn |
|--:|---|---|--:|---|
| 1 | `ne_int_fs` | `binary_operator.not_equal.mediump_int_fragment` | 1 | `deqp_intval4_ne_int_fs` (refused: integer/boolean) |
| 2 | `notequal_ivec3_fs` | `int_compare.notEqual.mediump_ivec3_fragment` | 1 | held by `deqp_intval3_opfs_notequal_ivec3_mediump` (and `deqp_intval_opfs_cmp_ivec3`) |
| 3 | `ne_ivec2_fs` | `binary_operator.not_equal.mediump_ivec2_fragment` | 1 | `deqp_intval4_ne_ivec2_fs` (refused: integer/boolean) |
| 4 | `bool_of_int_fs` | `conversions.scalar_to_scalar.int_to_bool_fragment` | 1 | held by `deqp_intval3_reslib_int_to_bool_fs` |
| 4 | `bvec2(ivec2)` | `conversions.vector_to_vector.ivec2_to_bvec2_fragment` | 1 | `deqp_intval4_bvec2_of_ivec2_fs` (refused: integer/boolean); width 4 is held by `deqp_intval3_reslib_ivec4_to_bvec4_fs` |
| 5 | `not_eq_int_fs` | `!(i == 2)` of a run-time int (a `random` shape) | 1 | `deqp_intval4_not_eq_int_fs` (**DIFFERS**: compiles, other words than Mesa's, hazard 5.4(1)) |
| 6 | `logical_and_fs` | `binary_operator.logical_and.bool_fragment` | 2 | held by `deqp_intval3_opfs_logical_and_bool` |
| 6 | `logical_or_fs` | `binary_operator.logical_or.bool_fragment` | 2 | `deqp_intval4_logical_or_fs` (refused: integer/boolean) |
| 7 | `and_icmp_fs` | `(i > 1) && (i < 3)` (random) | 2 | `deqp_intval4_and_icmp_fs` (refused: integer/boolean) |
| 8 | `uni_bool_and_or_fs` | `uniform bool u_a, u_b`: `&&`, `\|\|`, `!=` (hand-written, no dEQP case) | 2, 3 | `deqp_intval4_uni_bool_and_or_fs` (refused: integer/boolean) |
| 9 | `uapi_{int,ivec2,ivec3,ivec4,bool,bvec2,bvec3,bvec4}_fs` | the `uniform_api` `compare_*` harness, scalar `u_var`, constants of mixed sign | 3 | `deqp_intval4_uapi_<type>_fs`, all 8 (refused today with the **gl_FragColor lane** text, not the integer/boolean one: the compare product `result *= compare_*(...)` reaches the colour first) |
| 10 | `bvec4_cmp_fs` | `bool_compare.{equal,notEqual,all}.bvec4_fragment`, one pair with `all(equal)` and `any(notEqual)` | after `irindex-fix` | `deqp_intval4_bvec4_cmp_fs` (refused: `Defect(IrIndex)`) |
| 10 | `bvec3_cmp_fs` | the same at width 3 | after `irindex-fix` | `deqp_intval4_bvec3_cmp_fs` (**LOWERED**: already compiles byte-equal at the tip) |
| 11 | `sel_int_fs`, `sel_ivec2_fs` | `selection.mediump_int_fragment`, `..ivec2..` | 4 | `deqp_intval4_sel_int_fs`, `deqp_intval4_sel_ivec2_fs` (refused: the fragment **arithmetic** text, not integer/boolean: the `bcsel` of ints reaches `Arithmetic` first) |

(22 pairs in all: 19 drawn, 3 held by existing references:
`notequal_ivec3_fs`, `bool_of_int_fs` and `logical_and_fs`. Two of the rows'
shapes are mine where the table was loose: the `bool_compare` pairs are one
pair per width with `all(equal(a, b))` and `any(notEqual(a, b))` instead of three
separate dEQP cases, and `bvec2(ivec2)` is the reslib template at width 2.)

**Hazards found while drawing.**

- `not_eq_int_fs` (`!(i == 2)`, run-time int) **compiles but differs** from
  Mesa: a `DIFFERS` entry, a hazard for step 1 (it is the 5.4(1) shape; the
  lowering must make it byte-equal and move it to `LOWERED`).
- `uapi_*_fs` and `sel_*_fs` are refused by other texts first (gl_FragColor
  lane, fragment arithmetic): the integer/boolean lowering alone does not
  unblock them, steps of `fclane-lower` / `arith-lower` are also needed. Their
  `refusals_pinned` entries will change text when intval-rest lands; the
  byte-compare then needs the later steps.
- `bvec3_cmp_fs` is already byte-equal, but `bvec4_cmp_fs` hits `IrIndex`
  (`irindex-fix`): the width-4 difference is the `irindex` defect, not intval.

## 8. What each step leaves to `intval-vs16`, shares with loops and fclane

**Left to `intval-vs16` (and not touched here).** Every vertex shape:
`nir.rs:4456` (`UniformType`), `mod.rs:1364` (`UniformPrecision`), the vertex
`IntValue` of `nir.rs:4812`, `:5097`, `vs_reduce` (`Arithmetic`), the narrowed
mediump compare of attributes (`opvs_logical_and`, `opvs_not_equal_bvec3`), the
bool-word hazard of `vs_is_uniform_word` (the vertex takes *any* uniform word
as a bool, this item's `bool_word` is by type), the vertex int uniform words
(`vs_int_uniform` 346, `vs_struct_uniform` 116), the vertex halves of the
`int_array` / `bool_array` cases. The shared **words** are identical (ints are
32 bits in both stages: `ICMP_OR.u32.ne.m1`, `LSHIFT_AND.i32`, `.not`), the
shared **code** is `emit.rs`/`bir.rs`/`pack.rs` (nothing to add) and the
`opt.rs` fuse rules: whoever lands second re-runs `mali_deqp_intval.rs`,
`mali_deqp_unitype.rs` and `mali_deqp_loops.rs`. `vs16-1`'s `fmin`/`fmax`
mask rule is the same pass as step 2's (`nir_opt_algebraic.py:1159` family) in
another walk; the two must keep the same operand order test.

**Shared with loops.** The counters: `ICMP_OR.s32.lt/ge`, `IADD_IMM.i32`,
`S32_TO_F32` exist; the shapes loops adds that this item's steps carry are the
`i == 1` / `i != 2` of a counter in a body (`shaders.functions.control_flow`,
the `int_loop_counter` row, 11 cases, 4 to intval): step 1's `ine`. A kept
loop's bound read from an int uniform (`LoopMediump` 281, `LoopVertex` 241
with `ui_*`) needs the uniform word as an operand: `int_word` (exists) and
step 3's layout; a uniform array bound is step 3. Step 1's `Ine` must also
return the node form the loop's break test reads (`Alu::Icmp`, 32 bits).

**Shared with fclane.** The compare lanes: the mask `Val`s of steps 1 to 3 are
what `bcast_compare_{int,bool,float,array,const}` (233) turn into the colour
lane through `lane_tree`'s `B2f` arm. The contract: a mask is an
`Alu::Icmp`/`Fcmp`/`Ior`/`Iand` node, 16 bits (`mask16`) or 32 (`bool32`),
unchanged from the walk today; the chain forms (`ICMP_AND ... r0^`) are made
by `opt.rs` after the walk, so fclane sees only the nodes. Order: fclane C
after step 2 (the note of `mali-fclane-lowering.md` 5.2 says after
`intval-rest` and `arith-lower-1`); it may start with `bcast_compare_int` and
`_bool` (a single compare) before step 2.

## 9. Tests at risk and a forecast

At risk, all in the pinned set (`tests/mali_deqp_{intval,misc,mixprec,arith,
unitype,loops,loops2,uniarray,unif2,fclane}.rs`, `tests.rs`, `corpus-verdicts`):
step 1 and 2 add arms to a function every fragment int program passes; a
byte-compare of every `LOWERED` row is the check, not a run of the gate
(CLAUDE.md: local runs are the tests changed). `mali_deqp_unif2.rs`:
`refusals_pinned` pins `fs_bool_uniform` and `fs_int_uniform` at `FS_OTHER` and
`arr_*` at `ARRAY`; step 3 moves them in the same commit (to the `LANE` text of
fclane C until it lands). `mali_deqp_unitype.rs` `fsivec4`, `fsmulti` and
`mali_deqp_intval.rs` `rsg_cond_fs` pin `ColourForm`: unchanged by steps 1-4.

Forecast (HOST-ONLY, to be filled by step 0): the host measure at the tip,
`deqp-host-measure.sh --filter functional.shaders.operator`,
`--filter shaders.conversions`, `--filter shaders.random`,
`--filter uniform_api.value`; before and after each step the count of cases
whose first refusal is `IntValue` (fragment) and the cases that stop at the
next text. Section 3.2's lower bound (21) is checked against the per-case names
of `measure.tsv`.

## 10. For the planner

- Item order: `irindex-fix` before `intval-rest` step 1 (it hides bvec3/4,
  ivec harness and `?:` of compares, section 2.2); `intval-vs16` (vertex) in
  any order with steps 1-2 (no shared file but `opt.rs`); step 3 after
  `unif2-lower-1` (redraw of the array references, D1) and with or after
  `fclane-lower` C.
- Counts of 18.3 for this item are stale (`9ac31f30`); the first action of the
  implementation is step 0 (measure and the 20 references).
- Not this item, found on the way: `step` highp is `MixedPrecision` (mixcol),
  `sign` and int `abs`/`min`/`?:` are `Arithmetic` (arith 3, 5);
  `bvec3`/`bvec4` and `x ? 1.0 : 0.0` of an int compare are `IrIndex` (the
  triage of `irindex-ref` should list them: `bc_*_3/4`, `i_ge`, `i_gt`, `i_le`,
  `i_lt_const`, `i_eq_const`, `step_vec3/4`, `hs_int_ivec4_gggg`).
- The ES 2 shader sources are not in the tree (`$CONNECTED_DEPS_DIR/VK-GL-CTS-gles3`
  holds `gles3` data only); the host measure builds them from the CTS pin.
