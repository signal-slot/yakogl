<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): fsintres, the fragment integer/boolean residue, a lowering design

Plan #387, item `fsintres-design`. **HOST-ONLY**: design and Mesa references only.
No change to the compiler, and nothing was run on a board. Earlier trains lowered
most of "an integer or boolean value" in the fragment walk
(`mali-intval-rest-lowering.md`, `mali-deqp-fsint.md`, `mali-deqp-fsint2.md`,
`tests/mali_deqp_intval*.rs`, `tests/mali_deqp_fsint*.rs`). This note measures
what is still left and draws Mesa 26.2.1 references for the forms that no
reference covers yet (`tools/mali/testdata/deqp_fsintres_*`, pinned by
`tests/mali_deqp_fsintres.rs`). It also orders the steps that lift them.

Citations: ours is `<file>:<line>` under `src/shaderc/online/mali/` at the tip
`6e9e515c`. Mesa's words are quoted from the `text` of the references' own
`.expect.json` (Mesa 26.2.1's disassembler).

## 0. Summary

1. **154 fragment cases at the tip, 69 after `lower-fsint`.** The iMX95 run
   (`tools/cts/log/2026-11-2{4,5}-imx95-gles2-*-e63ab28e`: 191 fragment lines
   with this text) was re-run on the host at `6e9e515c`. 154 cases still stop
   first at "an integer or boolean value", all in the fragment shader. On the
   result of the pending `lower-fsint` item (plan #402, `e8327e42`: decrements,
   lowp `step`, mixed-width masks), 55 of them compile, 30 move to other texts
   and **69 still refuse with this text**. Those 69 are this note's residue.
2. **47 of the 69 had no reference.** 22 are covered by existing
   `deqp_intval*`, `deqp_fsint*` and `deqp_intrnd_*` references: the same fragment
   source, or the same Mesa words (section 2). This item drew **11 references for
   the new forms**. Four more draws gave Mesa words that are byte for byte those of
   an intval reference, so they were dropped (section 3.2). The G310 words equal the
   G610 words for all 11 (`tools/mali/g310_check.sh`: 11 identical).
3. **One form is most of it: dynamic indexing of a local array or matrix.**
   Twenty cases (`indexing.tmp_array.*_dynamic_write_*`,
   `indexing.matrix_subscript.mat4_dynamic_write_*`) stop at the catch-all of
   `Fs::int_lane` (`nir_fs.rs:3122`) on an **`iand` of two 32-bit index masks**.
   Those masks come from Mesa's if-else tree for a store at a run-time index
   (`mali-indexed-locals.md` 1: `nir_lower_indirect_derefs_to_if_else_trees`,
   arrays of up to 8 elements). The tree's leaves and the read side are already
   lowered for case 80 (plan #336). What is missing is the mask logic of the write
   side: `ilt` against byte immediates, then `LSHIFT_OR/AND.i32` with `.not`
   (section 4.1).
4. **Then come the harness forms of the shader library:** an `imul` by a folded
   constant (8), a bool negation that is not a 16-bit `lt`/`ge` (5), a mixed
   int/bool `bvec` (4), plus seven small groups (section 2). Six numbered steps
   (section 5). The first two take 28 of the 69 first refusals.
5. **Order:** after `lower-fsint` (plan #402) has landed, and with or after train
   4 (schedule 5.5a). Step 1 should come right after the vertex step VT2 of train 3
   (commit 7, `mali-vsunif-lowering.md` V2), whose index ladder it mirrors in the
   fragment walk (section 6).

## 1. What was measured

- **The case set.** Every case that one of the six iMX95 slices recorded as
  refused (`refusals.tsv`, 5,677 cases). Of those, 191 lines carry
  "an integer or boolean value", all of them "refused this fragment shader". The
  plan's text says 197 lines at `a3d8d2de`. The slices in the tree are those of
  `e63ab28e`, and they give 191.
- **The run.** `tools/mali/deqp-host-measure.sh --cts $CONNECTED_DEPS_DIR/VK-GL-CTS
  --slice <each iMX95 slice>` at `6e9e515c` (VK-GL-CTS `93bca01` with
  `cts-yakogl-target.patch`). Result: 5,677 cases, 4,685 with a host refusal,
  57 processes ended `exit=1`. **154 cases** have this text as their first
  refusal, every one at stage `fs`. The measure needs one local edit to build:
  see section 8.
- **The residue after `lower-fsint`.** The logged program of each of the 154
  cases was compiled through `tests/mali_ref`'s `compile_pair` (the GL link
  path), first at the tip and then at `e8327e42` (`lower-fsint`, not yet
  merged). At the tip all 154 refuse with this text. At `e8327e42` 55 compile,
  69 refuse with this text, and the other 30 refuse with: gl_FragColor lane 18,
  varying load 5, arithmetic 3, operand form 1, mixed precision 1, control
  flow 1, link offsets 1. The 85 that leave this text are those of
  `mali-deqp-fsint.md`: `common_functions.step` 21, the four
  `unary_operator.*_decrement_*` groups 48, `random` 10, `vector_combine` 4
  and `functions.{misc,datatypes}` 1 each.
- **The site.** The 69 were compiled at the tip once more, with a temporary
  print on each `Err(MaliError::IntValue)` of `nir_fs.rs` and `nir.rs` and on the
  `AluOp` at the catch-all. The print was not committed, and the tree is
  unchanged.

## 2. The 69 residue cases, by form

"Site" is the return at the tip. "Op" is the NIR operation the catch-all
(`nir_fs.rs:3122`, the last arm of `Fs::int_lane`) receives. "Reference" names
the reference that pins the form: one of this item's (**bold**) or an existing
one.

| # | Form | Cases (first refusal) | Site, op | dEQP groups (`dEQP-GLES2.functional.`) | Reference |
|--:|---|--:|---|---|---|
| A | store at a run-time index into a local `float`/`vecN` array or `mat4` column, read back statically, dynamically or by a loop | **20** | `nir_fs.rs:3122`, `Iand` | `shaders.indexing.tmp_array.{float,vec2,vec3,vec4}_dynamic_write_{static,dynamic,static_loop,dynamic_loop}_read_fragment` (16), `shaders.indexing.matrix_subscript.mat4_dynamic_write_*_fragment` (4) | **`idx_float_dyn_dyn`, `idx_vec4_dyn_dyn`, `idx_vec2_static_loop`, `idx_vec4_dyn_loop`, `idx_mat4_dyn_static`**; `deqp_intval3_idx_tmp_vec3_dynwrite_fs` (= `vec3_dynamic_write_static_read`) |
| B | harness int compare of an `imul` by a constant that the front end has not folded yet (`out`/`inout` int parameter, an int array by value) | **8** | `nir_fs.rs:3122`, `Imul` | `shaders.functions.qualifiers.{out,inout}_{,lowp_,highp_}int_fragment` (6), `shaders.functions.array_arguments.{global,local}_in_int_fragment` (2) | **`fn_arr_int`**; `deqp_intval3_reslib_inout_int_fs` (its Mesa words are those of `out_int` and `inout_highp_int`, section 3.2) |
| C | bool negation of a mask that is not a 16-bit `flt`/`fge` (`!a` of `bool(v)`, a bool array by value, overloads) | **5** | `nir_fs.rs:2995`, `Bnot`/`Inot` | `shaders.functions.qualifiers.in_bool_fragment`, `shaders.functions.datatypes.bool_bool_fragment`, `shaders.functions.array_arguments.{global,local}_in_bool_fragment`, `shaders.functions.overloading.user_func_arg_bool_types_fragment` | **`fn_arr_bool`**; `deqp_intval3_reslib_func_bool_fs` (= `bool_bool`, and the words of `user_func_arg_bool_types`), `deqp_intval4_uapi_bool_fs` (the words of `in_bool`) |
| D | `bvecN(...)` of an int and a bool or float, compared with a `uniform bvecN` (a 32-bit and a 16-bit mask in one `iand`/`ior`) | **4** | `nir_fs.rs:2889` (`mask_op`) | `shaders.conversions.vector_combine.{int_bool_to_bvec2,bool_float_int_to_bvec3,bool_float_ivec2_to_bvec4,bool_float_int_bool_to_bvec4}_fragment` | **`vcomb_int_bool_bvec2`**; `deqp_fsint_conv_bool_float_int_to_bvec3` (still refused after `lower-fsint`) |
| E | int compare in a fragment shader that is compiled against the linked vertex shader's constants (the shader-library `*_vertex` cases whose fragment shader is the pass-through) | **9** | `nir_fs.rs:7572` | `shaders.constants.int_{0..4}_vertex` (5), `shaders.constant_expressions.trivial.int_vertex`, `shaders.constant_expressions.operators.math_{int,ivec}_vertex` (2), `shaders.random.conditionals.fragment.70` | `deqp_intval_reslib_int_vs`, `deqp_intval3_reslib_const_int_vs` (the same fragment source as the eight `*_vertex` cases); `conditionals.fragment.70` not drawn |
| F | `f2b` of a float lane (random shaders) | **6** | `nir_fs.rs:3122`, `F2b` | `shaders.random.conditionals.{combined,fragment}.6`, `shaders.random.conditionals.fragment.31`, `shaders.random.scalar_conversion.{combined,fragment}.72`, `shaders.random.texture.fragment.72` | `deqp_intrnd_bool_uniform_select_comb`, `deqp_intrnd_trunc_lane_fs`, `deqp_intval3_rsg_sconv72_fs` (same sources) |
| G | `b2f` of a compare `&&` a compare, the `isOk` pair of `conditionals.if` | **4** | `nir_fs.rs:3122`, `B2f` | `shaders.conditionals.if.{complex_condition,compound_statement,sequence_condition,sequence_statements}_fragment` | `deqp_intval3_cond_complex_fs`, `deqp_intval3_cond_compound_fs`. The two `sequence_*` sources differ from `compound_statement` only by the comma operator, so their NIR is the same (not drawn) |
| H | an int that is both tested and converted (`bool(i)` and `float(i)` of one int) | **3** | `nir_fs.rs:3092`, `I2b` | `shaders.random.scalar_conversion.{fragment,combined}.68`, `shaders.random.scalar_conversion.fragment.44` | **`rnd_sconv68`**; `fragment.44` not drawn |
| I | `iadd` of a run-time int that Mesa drops or turns into an address | **4** | `nir_fs.rs:3122`, `Iadd` | `fragment_ops.interaction.basic_shader.96`, `shaders.random.trigonometric.fragment.70`, `shaders.struct.{local,uniform}.nested_struct_array_dynamic_index_fragment` | **`inter_96`**; the other three not drawn |
| J | `float(i)` of an int that is also tested | **2** | `nir_fs.rs:3111`, `I2f` | `shaders.random.scalar_conversion.{fragment,combined}.58` | `deqp_intrnd_int_uniform_lanes_fs` (same source) |
| K | `ine` of an int lane against a constant | **2** | `nir_fs.rs:3122`, `Ine` | `fragment_ops.interaction.basic_shader.85`, `shaders.random.conditionals.fragment.12` | **`inter_85`**; `deqp_intval3_rsg_cond12_fs` (same source) |
| L | `&&` of two bool masks | **1** | `nir_fs.rs:2858` (`and16`) | `shaders.operator.binary_operator.logical_and.bool_fragment` | `deqp_intval3_opfs_logical_and_bool` (same source) |
| M | `||` of two bool masks | **1** | `nir_fs.rs:3122`, `Ior` | `shaders.operator.binary_operator.logical_or.bool_fragment` | `deqp_intval4_logical_or_fs` (same source) |
| | **total** | **69** | | | |

The case counts are the iMX95 slices' cases (Fail or NotSupported on the board),
not the 17,165 must-pass count. Every case in the table is a must-pass case of
`tools/cts/gles2-mustpass.txt`.

## 3. The references

### 3.1 The eleven `deqp_fsintres_*`

Each pair is the program dEQP-GLES2 writes for the case, verbatim from the
measure's `.qpa` (`programs/<hash>.vert|.frag`). Each was drawn with
`tools/mali/deqp_ref.sh --instrument shim` (G610 drm-shim, `--uniform-values`).
The `.nir.sha` siblings come from `tools/mali/nir_refs.sh`. "fs words" is the
`fs.fragment` region's word count (`FS_WORDS` in the test). At the tip, and at
`e8327e42` and `13259474` (`lower-train-2`), every one is refused with exactly
"Valhall: an integer or boolean value is not lowered yet".

| Ref (`deqp_fsintres_*`) | dEQP case | Form | fs words |
|---|---|---|--:|
| `idx_float_dyn_dyn` | `shaders.indexing.tmp_array.float_dynamic_write_dynamic_read_fragment` | A, scalar elements | 65 |
| `idx_vec4_dyn_dyn` | `shaders.indexing.tmp_array.vec4_dynamic_write_dynamic_read_fragment` | A, four lanes | 98 |
| `idx_vec2_static_loop` | `shaders.indexing.tmp_array.vec2_dynamic_write_static_loop_read_fragment` | A, two lanes, `for (i < 4)` read | 59 |
| `idx_vec4_dyn_loop` | `shaders.indexing.tmp_array.vec4_dynamic_write_dynamic_loop_read_fragment` | A, `for (i < ui_four)` read: Mesa unrolls to the 4 elements, with no branch | 92 |
| `idx_mat4_dyn_static` | `shaders.indexing.matrix_subscript.mat4_dynamic_write_static_read_fragment` | A, `mat4` columns | 85 |
| `fn_arr_int` | `shaders.functions.array_arguments.global_in_int_fragment` | B, `-1 * ivec4` against a `uniform ivec4` | 28 |
| `fn_arr_bool` | `shaders.functions.array_arguments.global_in_bool_fragment` | C, `bvec4` of `!` lanes against a `uniform bvec4` | 24 |
| `vcomb_int_bool_bvec2` | `shaders.conversions.vector_combine.int_bool_to_bvec2_fragment` | D, two lanes | 18 |
| `rnd_sconv68` | `shaders.random.scalar_conversion.fragment.68` | H | 13 |
| `inter_96` | `fragment_ops.interaction.basic_shader.96` | I, every int value dead | 7 |
| `inter_85` | `fragment_ops.interaction.basic_shader.85` | K, folded by Mesa | 9 |

### 3.2 Drawn and dropped: the words already pinned

These four were drawn too, and their `fs.fragment` words were byte-equal to an
existing reference's. They are not committed. The lowering of that reference
covers them, but its byte-compare does not run their sources. The step that lifts
the reference should compile these four cases' sources too, as a check:

| dEQP case | Same Mesa words as |
|---|---|
| `shaders.functions.qualifiers.out_int_fragment` | `deqp_intval3_reslib_inout_int_fs` |
| `shaders.functions.qualifiers.inout_highp_int_fragment` | `deqp_intval3_reslib_inout_int_fs` |
| `shaders.functions.overloading.user_func_arg_bool_types_fragment` | `deqp_intval3_reslib_func_bool_fs` (and `deqp_boolpix_bw_fn_out_bool_fs`) |
| `shaders.functions.qualifiers.in_bool_fragment` | `deqp_intval4_uapi_bool_fs` (and five other references) |

`inter_96`'s words also equal those of about thirty other references (a pass-through
`b - a` colour). It is kept because its source, all dead int values, is a form
that no intval reference has.

## 4. Mesa's words

### 4.1 Form A: the index ladder of a store at a run-time index

`idx_float_dyn_dyn` (four stores `arr[ui_k] = coords * c_k`, then the reads
`arr[ui_k]`). Per store, Mesa compares the index with the tree's three split points
3, 2, 1. The points are byte lanes of one constant word, against a zero register
`r2`. It then builds each element's "this one" mask and selects:

```
ICMP_OR.s32.lt.m1 r3, u2.w1, 0x3020100.b3, r2      idx < 3
ICMP_OR.s32.lt.m1 r4, u2.w1, 0x3020100.b2, r2      idx < 2
LSHIFT_OR.i32 r5, r4, 0x0.b00, r3                  (idx < 2) | (idx < 3)
MUX.v2i16 r5, r1.h00, r0.h00, r5^.h00              element 3 <- value or old
LSHIFT_OR.i32 r6, 0x0, 0x0.b00, r4.not             !(idx < 2)
LSHIFT_AND.i32 r6, r3, 0x0.b00, r6^                (idx < 3) & !(idx < 2): element 2
MUX.v2i16 r6, r0.h00, r1.h00, r6^.h00
ICMP_OR.s32.lt.m1 r7, u2.w1, 0x1000000.b3, r2      idx < 1
...
```

- The compares are 32-bit (`.s32`) although the uniforms are `mediump int`:
  the index of a deref is not lowered to 16 bits. The masks are 32-bit and the
  selects read their low half (`.h00`).
- `iand` and `ior` of two such masks are `LSHIFT_AND.i32` / `LSHIFT_OR.i32` with
  `0x0.b00` (shift 0). `inot` is a `.not` source modifier on an `LSHIFT_OR.i32`
  with zero. These are the mask words of `mali-intval-rest-lowering.md` 4.2 and 4.4
  (`LSHIFT_AND.i32 r0, r0^, 0x0.b00, r1^` of two `ICMP_OR.u32` masks in
  `deqp_unif2_fs_int_uniform`; `LSHIFT_OR.i32 ... .not`, its hazard 5.4).
- With two or more lanes per element (`idx_vec2_static_loop`, `idx_vec4_*`,
  `idx_mat4_*`), the middle elements' masks are widened (`IADD.v2u16 r3, r3^.h00,
  0x0`) and the select is `CSEL.v2u16.eq r3, r3^, 0x0, r0, r1` instead of a
  `MUX.v2i16`. The outer elements keep `MUX.v2i16`.
- The read at a run-time index (`*_dynamic_read`) is the same tree with the
  index's own `ilt` masks used directly as `MUX.v2i16` selectors, no `iand`
  (`idx_float_dyn_dyn`, the second half: `ICMP_OR.s32.lt.m1 r9, u2.w0,
  0x1000000.b3, r2`, `MUX.v2i16 r9, r8.h00, r1.h00, r9^.h00`).
- A loop read to a constant 4 (`static_loop`) or to a uniform bound
  (`dynamic_loop`) leaves no loop in the words. `idx_vec4_dyn_loop` (92 words)
  differs from `idx_vec4_dyn_dyn` (98 words) in the read half: 22 against 30
  `MUX.v2i16` and 14 against 12 `ICMP_OR`. The write half has the same counts
  of `CSEL`, `LSHIFT_*` and `IADD`.

### 4.2 Form B: `imul` by a folded constant

`fn_arr_int` (`f = func(arr)` returns `-1`, then `out0 = f * ivec4(arr)`). Mesa
folds `imul(-1, x)` to `ineg`, which is `ISUB.s32 rX, 0x0, rX^` per lane after
`F16_TO_S32.rtz`. Then comes one `ICMP_OR.u32.eq.m1` per lane against
`ref_out0`'s words. The first two compares are merged into an
`ICMP_AND.u32.eq.m1 ..., r3^`, and the rest are joined by `LSHIFT_AND.i32`. That
gives a 28-word program. Our walk gets the `imul` before the constant is known
(the parameter is written through `out`/`inout` or an array copy), and it
reaches the catch-all.

### 4.3 Form C: bool arrays by value

`fn_arr_bool`. `bvec4(v)` lanes are `FCMP_OR.v2f16.ne.m1` (16-bit masks). Each `!`
is `LSHIFT_OR.v2i16 r, 0x0, 0x0.b00, r^.not`. The compares against the `uniform
bvec4` are `ICMP_OR.v2u16.eq.m1 ..., u3.w1.h00` (one 16-bit half of each bool
word), and they are chained by `LSHIFT_AND.v2i16`. Our `Bnot`/`Inot` arm
(`nir_fs.rs:2992-2996`) takes only a 16-bit `flt`/`fge`, which it turns into the
opposite compare. Mesa keeps the `.not` when the mask is an `fne`.

### 4.4 Form D: one 32-bit and one 16-bit mask

`vcomb_int_bool_bvec2`. The int lane is `F16_TO_S32.rtz`, `ICMP_OR.u32.ne.m1 r1,
r1^, 0x0` (`bool(int)`), then `ICMP_OR.u32.eq.m1 r1, r1^, u2.w0` (32-bit). The
bool lane is `FCMP_OR.v2f16.ne.m1`, then `ICMP_OR.v2u16.eq.m1 r0, r0^.h00,
u2.w1.h00` (16-bit). They are joined by **`LSHIFT_AND.v2i16 r0, r0^.h00, 0x0.b00,
r1^`**: a 16-bit `iand` that reads the 32-bit mask's low half, the
`pan_nir_lower_bool_to_bitsize` rule `nir_fs.rs:2840-2847` quotes for compares.
`mask_op` (`nir_fs.rs:2883-2890`) takes two masks of one width only.

### 4.5 Forms H, I, K

- `rnd_sconv68`. `float(b)` of `b = int uniform + 7` is `IADD_IMM.i32 r1, u2.w1,
  #0x7`, `S32_TO_F32`. `bool(a)` of an int uniform in a colour lane is
  `ICMP_OR.u32.ne.m1 r1, u2.w0, 0x0, 0x0`, `MUX.v2i16 r1, 0x0, 0x3C000000.h11,
  r1^.h00`, `U16_TO_U32`. The `I2b`-with-`I2f` guard (`nir_fs.rs:3092`) refuses
  it: the guard was written for `deqp_intrnd_int_uniform_lanes_fs`, where the same
  int is converted both ways. Here the two conversions read *different* ints
  (`a`, `b`), so the guard is too wide.
- `inter_96`. Every int value is dead (`k != k` is false, `f`, `e` are overwritten,
  and the colour is `b - a`). Mesa's 7 words read only `u2.w0`, `u2.w1` (the folded
  colour) and have no integer word at all. Our walk refuses the dead `iadd(d, -1)`
  before dead code is removed.
- `inter_85`. `ivec3(true, d, c) != b` with `b = ivec3(0, -1, 3)`: lane x is
  `1 != 0`, so the `any` is true and Mesa drops the branch. Its words are
  `F16_TO_F32` + `FROUND.f32.rtz` of `int(e)` (`e` a mediump uniform) and a
  constant colour. Our `int_lane` sees `ine(lane, -1)` (lane y) before the `any`
  is folded.

## 5. Steps

Numbered by the first refusals each lifts at `e8327e42`. A step is one commit of a
train pull request. Each step moves its references from `REFUSED` to `LOWERED` in
`tests/mali_deqp_fsintres.rs` (or in the existing test that holds them) in the
same commit, and re-runs that file and the `deqp_intval*`/`deqp_fsint*` tests.
"Forecast" is an upper bound: the cases whose first refusal goes away. Whether they
then compile through depends on what comes next, which was not measured.

1. **FR1, the index ladder (form A, 20 cases).** In `Fs::int_lane`, take `iand`,
   `ior` and `inot` of 32-bit `ilt`/`ige` masks of an int index
   (`ICMP_OR.s32.lt.m1` against `0x3020100.bK` / `0x1000000.b3`), as
   `LSHIFT_AND.i32` / `LSHIFT_OR.i32 ... .not` (4.1). Then select with
   `MUX.v2i16 .h00` for single-lane elements and the outer elements of a
   multi-lane one, and with `IADD.v2u16` + `CSEL.v2u16.eq` for the middle ones.
   The leaf selects and the read side exist (`mali-indexed-locals.md`, case 80).
   References: the five `idx_*` and `deqp_intval3_idx_tmp_vec3_dynwrite_fs`.
   dEQP groups: `shaders.indexing.tmp_array` (16) and
   `shaders.indexing.matrix_subscript` (4) `*_dynamic_write_*_fragment`.
2. **FR2, `imul` by a constant that folds late (form B, 8 cases).** Fold the
   constant through the `out`/`inout` parameter and the array copy before the
   walk (the K-fold helper `mesa_fold.rs`, or `nir::fold_int` once the constant
   is known). Then `imul(-1, x)` becomes `ineg`, which `int_lane` already emits
   as `ISUB.s32 0, x` (`nir_fs.rs:3109`). The four-lane harness compare chain is
   `ICMP_OR`, `ICMP_AND`, `LSHIFT_AND.i32` (4.2). References: `fn_arr_int`,
   `deqp_intval3_reslib_inout_int_fs` (+ the two cases of 3.2). Groups:
   `shaders.functions.qualifiers` (6), `shaders.functions.array_arguments` (2).
3. **FR3, the harness bool forms (forms C and D, 9 cases).** Allow `Bnot`/`Inot`
   of any 16-bit mask as `LSHIFT_OR.v2i16 0, 0.b00, m.not`, keeping the
   compare-inversion rule for `flt`/`fge` where Mesa uses it. Let `mask_op` take a
   16-bit mask with a 32-bit one as `LSHIFT_AND.v2i16 m16.h00, 0.b00, m32` (4.3,
   4.4). References: `fn_arr_bool`, `vcomb_int_bool_bvec2`,
   `deqp_intval3_reslib_func_bool_fs`, `deqp_intval4_uapi_bool_fs`,
   `deqp_fsint_conv_bool_float_int_to_bvec3`. Groups:
   `shaders.functions.{qualifiers,datatypes,array_arguments,overloading}` (5),
   `shaders.conversions.vector_combine` (4).
4. **FR4, the linked-vertex fragment compare (form E, 9 cases).** This step
   belongs to `mali-intval-rest-lowering.md` and `deqp_intval_reslib_int_vs`: the
   compare arm at `nir_fs.rs:7560-7572` ("another operand ... has no reference").
   No new reference. Draw `random.conditionals.fragment.70` before coding, if this
   step is taken.
5. **FR5, the random conversions (forms F, H, J, 11 cases).** Narrow the guard of
   `nir_fs.rs:3092` and `:3111` to one int that is both tested and converted
   (`rnd_sconv68` against `deqp_intrnd_int_uniform_lanes_fs`), and take `f2b` of a
   lane (`deqp_intrnd_*`, `deqp_intval3_rsg_sconv72_fs`). Groups:
   `shaders.random.{scalar_conversion,conditionals,texture}`.
6. **FR6, dead and foldable ints (forms I and K, 6 cases), and the `b2f` /
   mask-logic remnants (G, L, M, 6 cases).** Run dead-code removal and the `any`
   fold before `int_lane` sees `iadd`/`ine` (`inter_96`, `inter_85`; this is close
   to cf-dead, schedule 5.5a commit 1). G, L, M are intval-rest step 2 (M2, mask
   logic) with the references named in section 2. The two `struct.*.
   nested_struct_array_dynamic_index` and `trigonometric.fragment.70` are not
   drawn: they need FR1's ladder plus struct offsets and a transcendental, and they
   are one case each.

Sum of first refusals: 20 + 8 + 9 + 9 + 11 + 12 = 69.

## 6. Order relative to schedule 5.4 and 5.5

- Every step edits `nir_fs.rs` (`int_lane`, `mask_op`, `and16`), which plan
  #402's `lower-fsint` also changes. **All steps come after `lower-fsint` has
  merged.** The line numbers above are those of `6e9e515c` and will move.
- Train 3 (5.4) is the vertex walk and the uniforms (3b), and the mixed precision
  and operand forms (3c). None of their steps writes `int_lane`'s mask arms, so
  fsintres does not block them and they do not block it. **FR1 should follow train
  3's commit 7 (VT2 + VT3)**: VT2 builds the same ladder in the vertex walk
  (`mali-vsunif-lowering.md` V2: `tmp_dynwrite`, `mat2_dynwrite`, `ICMP_OR.s32.lt.m1`).
  The fragment ladder should reuse its mask helpers rather than write a second
  copy.
- Train 4 (5.5a) is fixed: cf-dead, audit, the `vectorise` steps, K2/K4/K5, T1,
  loops. FR2 uses the K folds (K2/K4, `mesa_fold.rs`), and FR6 uses cf-dead's dead
  code removal. Proposed: **FR1 and FR3 as a fragment-int group appended to train
  4 after commit 8** (they share `int_lane` and no function with `vectorise`).
  **FR2 after K4** (commit 10). **FR6 after cf-dead** (commit 1). If train 4 is
  already too long, the same group is a train 5 of its own, and FR1 alone (20
  cases) is the one to keep.
- The loop steps' index ladder (LS7 of `mali-loops-lowering.md`, `deqp_loops2_fs_idx_*`,
  `deqp_idxloc_*`: stores at a *loop* index) uses the same mask words. Whichever
  lands first writes them, and the other rebases.

## 7. Reproducing

```sh
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fsintres
# a reference again (shim built with drm_shim_ref.sh build):
tools/mali/deqp_ref.sh tools/mali/testdata/deqp_fsintres_<shape>.vert \
    tools/mali/testdata/deqp_fsintres_<shape>.frag "$TMPDIR/deqp_fsintres_<shape>" --instrument shim
tools/mali/g310_check.sh 'deqp_fsintres_*'
```

The measure: `tools/mali/deqp-host-measure.sh --cts <VK-GL-CTS> --slice
tools/cts/log/2026-11-24-imx95-gles2-S1a-e63ab28e ...` (all six iMX95 slices),
then `awk -F'\t' '$8 ~ /integer or boolean/' measure.tsv`.

## 8. Caveats

- At `6e9e515c` `capi/Cargo.toml` declares the feature `vivante-st` twice (lines
  41 and 85; `110080eb` added the second). Cargo then refuses to load the
  workspace, so neither the measure's library nor any `cargo test` builds. The
  measure and the tests here were run with line 41 deleted locally. The edit is
  not part of this item, and it needs its own fix on main.
- The commit of `lower-mixprec3` (plan #402, `81b3ce19`) is not reachable from
  this clone, so the pins were not checked against it.
- The case counts are first refusals on the host, with the iMX95 run's refused
  cases as the case set. A case that passed or failed for another reason on the
  board is not in it.
