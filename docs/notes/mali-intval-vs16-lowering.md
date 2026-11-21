<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): intval step 3, the vertex walk's mediump compares and integer uniforms, a lowering design

Plan #387, item `vs16-design` (HOST-ONLY, no board). It is a design for the
later item `intval-vs16`; it changes no code, test or reference. It takes up
what `docs/notes/mali-deqp-intval.md` section 8 leaves to "step 3" (the
mediump attribute compare in the vertex stage, about 100 failing cases and
the `DIFFERS` reference `opvs_logical_and`) and the vertex `UniformType` /
`UniformPrecision` shapes that `docs/notes/mali-deqp-families.md` section 18.3
(table "Integer and boolean values") gives to the same item: `vs_int_uniform`
346, `vs_struct_uniform` 116, `vs_mediump_*` 99 cases.

Citations: ours is `<file>:<line>` under `src/shaderc/online/mali/` at
`3497799a`; Mesa words are quoted from the `text` of the references' own
`.expect.json` (Mesa 26.2.1's disassembler, `va_disasm_ref`), not
re-disassembled here. A word pattern marked **(no reference)** is what the
Mesa rules predict by analogy and must be measured before it is coded.

## 0. Summary

1. **Six mechanisms**, M1 to M6 (section 5), over the two functions the
   vertex walk already has for 16 bits: `nir.rs` `vs_narrow_input` and the
   `vs_alu16`/`vs_alu16u` family, and the 16-bit `emit.rs` rows the fragment
   walk uses (`FCMP_OR.v2f16`, `ICMP_OR.v2u16`, `LSHIFT_OR.v2i16`,
   `S16_TO_S32`, `FMIN.v2f16` all exist: section 4.2). What is missing is the
   *NIR* of the vertex walk: a compare of mediump operands is built at 32
   bits, and an int uniform is refused before it is read.
2. **Two cheap gates first** (M5, M6): a `mediump int` uniform is refused by
   `UniformPrecision` although an int is always one raw 32-bit word
   (`mod.rs:1364`); an `int` uniform is refused by `UniformType` unless it is
   an index or a loop bound (`nir.rs:4456`). Lifting them is a few lines, but
   it moves the 99 and part of the 346 cases only to the family *behind* them
   (loops, indexing, random shaders), so the case counts of section 3 are
   first-refusal counts, not passes.
3. **A real hazard** (section 5, M5): `vs_is_uniform_word` takes *any*
   uniform word as a boolean (`0` / `~0`). With int uniforms allowed, `i2b`
   (`bool(int)`) and `ieq(b, true)` would read an int `1` as a true bool. The
   gate must be by the uniform's type (`bool_type`), as the fragment walk's
   `bool_dword` does.
4. **Three sessions** (section 6): vs16-1 `opvs_logical_and` and
   `opvs_not_equal_bvec3` byte-equal, no new reference; vs16-2 the integer
   uniform gates; vs16-3 the rest of the compare family, which needs the
   references of section 7 first.
5. **Cases** (section 3): about 87 named mediump/bool `_vertex` operator cases
   (60 float compares, 27 bool compares and logic), plus the 13 `selection`
   and `sequence` cases of the same text that the board's list does not show
   by name (the "about 100" of intval section 8); 4 `DIFFERS` references
   (`opvs_logical_and`, `vsapi_mat2`, `vsapi_struct`, and with the same
   narrowing helper `ck_lane_mediump`, `rnd_vs_mediump_kept`); the vertex
   integer-uniform rows are a gate, not a pass count.

## 1. What was read

- `docs/notes/mali-deqp-intval.md` sections 6 to 9 (section 8's "Left to step
  3"), `mali-deqp-families.md` section 18 (18.2 rows 3 and 17, 18.3 table),
  `mali-deqp-unitype.md` (sections 1, 2, 3.3, 3.4, "Lowering result"),
  `mali-int-uniforms.md`, `mali-vs-compare-clamp.md`, `mali-compare-forms.md`,
  `mali-precision.md` Part A.
- The references (all `tools/mali/testdata/`, drawn with `--uniform-values`):
  `deqp_intval_opvs_logical_and`, `deqp_intval_opvs_not_equal_bvec3`,
  `deqp_intval_opvs_less_ivec4`, `deqp_unitype_vsbvec3`,
  `deqp_unitype_vsapi_mat2`, `deqp_unitype_vsapi_struct`,
  `deqp_unitype_vsmat2_mediump`, `deqp_colourpair_ck_lane_mediump`,
  `deqp_colourpair_rnd_vs_mediump_kept`, `deqp_loops_vs_for_uni`,
  `deqp_loops_vs_select`, `deqp_arith_uniform_api_bool_compare_vs_none`.
  Also the `arith-ref` references of the same stage, read for section 4.6:
  `deqp_arith_op_bool_{all,any}_vs_none`, `deqp_arith_op_binary_{equal,
  not_equal}_vs_none` (bvec2), `deqp_arith_op_binary_{equal,not_equal}_vs_
  mediump` (ivec2 of mediump attributes), `deqp_arith_other_ball_vs_none`.
  There is no `deqp_unif2_*` and no `deqp_intval3_*` on this tip (the
  `intval3-ref` item is running; this note does not use it).
- The compiler: `nir.rs` (`uniform_of` :1045, `fp16_uniform` :1145,
  `index_only` :1163, `int_compare_only` :1186, `bool_type` :2951,
  `vs_is_uniform_word` :2958, `vs_is_bool` :2965, `vs_narrow_stored` :3011,
  `vs_widen` :3029, `vs_pairs16` :3110, `vs_alu16` :3135, `vs_uniform16`
  :3240, `vs_narrow_input` :3284, `vs_alu16u` :3321, `vs_reduce` :3612, the
  walk's `LoadUniform` arm :4452-4470 and its ALU arms :4800-5100),
  `select.rs` `vs_variant` :831 (the 16-bit operand swizzles :990-1020),
  `emit.rs` (the row table :44-225, the arms :270-460), `mod.rs` :1335-1365,
  and the fragment walk's analogues `nir_fs.rs` `mask16` :2103, `and_masks`
  :2128, `mask_op` :2226, `node16` :2746, `vec16` :2817, `int_lane` :2270.

## 2. The two refusals, where they are raised

| text | site | what it checks |
|---|---|---|
| `UniformPrecision` (99) | `mod.rs:1364`, the loop over the used uniforms after `UboLayout::vs` | not a member, not a bool, not `Precision::High`, and no fp16 word (`ubo.half(..)` is `None`). `nir::fp16_uniform` (:1145) makes fp16 words for float/vec/mat only, so **a `mediump`/`lowp` `int`, `ivec`, `uint` uniform lands here**, as does a mediump array or a struct member whose precision `uniform_precision` cannot find. |
| `UniformType` (346 + 116) | `nir.rs:4456` in the walk's `LoadUniform` arm | `u.gl_type == GL_INT` and the value is neither `index_only` nor `int_compare_only` (a loop head's compare). Other int types (`ivec*`, `uint`, `bool`) pass the arm; `uniform_of` (:1087) refuses only types it lists no arm for (`mat2x3`, samplers, ...). |
| `IntValue` | `nir.rs:4812`, `:5097` | a compare `ieq`/`ine` of two run-time ints with no `vs_is_bool` operand, and `udiv`/`umod`/`umin`/`umax` (ES 3.00, nothing to do here). |
| `Arithmetic` | `nir.rs:3612` `vs_reduce`, `_ => Err` | `any`/`all` of anything but the three shapes it takes (`ball(feq, 2)`, `bany(flt, 3)` against 0, `ball(ieq, 3)` of bool words). `opvs_not_equal_bvec3` ends here. |

The compare itself is **built at 32 bits**: the arms at :4892-4926 emit
`Alu::Fcmp` with `bits: 32` from the lanes `lane(w, ..)` gives, whatever
`m.prec_of(..)` says. Only `fmul`/`fadd`/`ffma`/`fneg` look at `Prec::Medium`
(:4608-4623, `vs_pairs16`, `vs_uniform16`). This is the whole of the
"mediump compare in the vertex stage" gap.

## 3. The shapes and their counts

Counts are the board's `2026-11-15-t6-gles2-S{1..4}-37305608`
`refusals.tsv` joined by case name (pre-step-1/2 texts, so every case below
has "an integer or boolean value" as its **board** refusal; the host's present
first refusal is in section 18.2 of the families note), restricted to
`_vertex` cases. They are first-refusal counts of cases, **not pass
forecasts**: the pass count needs the host measure of section 8.

### 3.1 The mediump compare family (section 8's "about 100")

| shape (dEQP name, `shaders.operator.`) | cases | mediump vertex input | Mesa form (section 4) |
|---|--:|---|---|
| `float_compare.{equal,notEqual,lessThan,lessThanEqual,greaterThan,greaterThanEqual}.{lowp,mediump}_vec{2,3,4}_vertex` | 36 | attributes `mediump vecN`, a `bvecN` of fp16 compares widened lane by lane into the colour | 4.1, 4.3 (no reference for the vector forms) |
| `binary_operator.{equal,not_equal}.{lowp,mediump}_{float,vec2,vec3,vec4}_vertex` | 16 | same, `==` of whole vectors (`all` of the lanes' `feq`; `!=` is `any` of `fneu`) | 4.3 (no reference) |
| `binary_operator.{less,less_or_equal,greater,greater_or_equal}.{lowp,mediump}_float_vertex` | 8 | scalar compare | 4.1 |
| `bool_compare.{all,any,equal,not,notEqual}.bvec{2,3,4}_vertex` | 15 | `greaterThan(a_in0.zxy, vec3(0.0))` of mediump attributes, then the bool builtin | 4.3 (`notEqual.bvec3` is the reference `opvs_not_equal_bvec3`; the rest no reference) |
| `binary_operator.{equal,not_equal}.{bool,bvec2,bvec3,bvec4}_vertex` | 8 | the same bool inputs, `==`/`!=` of whole `bvec` | 4.3 |
| `logical_and`, `logical_or`, `logical_xor`, `not` `.bool_vertex` | 4 | `a_in0.z > 0.0` of mediump attributes (the reference `opvs_logical_and` is `logical_and`) | 4.2 |
| `selection`, `sequence` (vertex, board text "integer or boolean") | 13 | **not classified here**: the names are not in the lists above, they read int/bool values (`mali-deqp-intval.md` section 2's G/C-VS shapes); counted because section 8's "about 100" is 87 + these | by the case |
| **named sum** | **87** (+13) | | |

The 12 `highp_*` rows of the same families (`float_compare.*.highp_vec*`,
`equal.highp_*`) and the 54 `int_compare` ones are **not** step 3: their
inputs are highp attributes or 32-bit ints, which the step-2 walk compiles
(`opvs_less_ivec4`).

### 3.2 The integer and boolean uniform shapes (section 18.3)

| shape | text | cases | first refusal | what the uniform is |
|---|---|--:|---|---|
| `vs_int_uniform` | `UniformType` | 346: `shaders.random` 160, `shaders.indexing` 82, `shaders.loops` 56 | `nir.rs:4456` | `uniform mediump int`/`int` read as a value (`ui_zero ... ui_six`, random programs' `int`/`ivec`) |
| `vs_struct_uniform` | `UniformType` | 116: `uniform_api.value` 66, `shaders.struct` 37, `uniform_api.info_query` 12 | `nir.rs:4456` for a member `int`; `uniform_of` for `mat4_mat2` | structs of `int`, `ivec4`, `bvec`, ... (`basic.int`, `basic_struct.int_ivec4`, `array_in_struct`/`struct_in_array` are array shapes and stay with uniarray) |
| `vs_mediump_basic` / `_struct` / `_array` | `UniformPrecision` | 99: `shaders.loops` 41, `uniform_api.value` 36, `shaders.indexing` 12 | `mod.rs:1364` | `uniform mediump int ui_*` (loops, indexing: the int part, about 53), mediump struct members and arrays (`uniform_api`, uniarray's) |
| `uniform_api` `bvec2/3/4` | `UniformType`/IntValue (now `Arithmetic`) | 27 vertex: `basic.bvecN`, `bvecN_api_int`, `bvecN_api_float` | `vs_reduce` for N != 3 | `bvecN` words, `a == b ? 1.0 : 0.0` against the expected constants |

Where each goes **after** the gate: the `ui_*` loop counters and bounds meet
`LoopVertex` (241) and `IndexedLocal`; the 94 NotSupported cases among
`UniformType` are indexing/loops shapes that stay NotSupported by the board
until those families land. Of the 346 + 116 + 99 first refusals the honest
statement is: **the gates are required by the whole population and complete
by themselves only for the cases whose int uniform feeds arithmetic or a
compare already lowered** (`opvs_*` arms: `iadd`, `imul`, `ilt/ine`, `i2f`,
`b2f`, `bcsel`), which is measured, not counted, in section 8.

## 4. How Mesa 26.2.1 compiles them (from the references)

### 4.1 `a_in0.z > 0.0`, `a_in1.x > 0.0` at `mediump`, `&&` (`opvs_logical_and`)

```
LD_ATTR_IMM.v3.f32.slot0.wait0 @r0:r1:r2, r60, r61, index:0x1, table:0x1   ; a_in0.xyz (up to the last lane read)
FADD.f32 r0.h0, r2^, 0x0.neg                  ; a_in0.z narrowed: f2f16 as an FADD with a half destination
LD_ATTR_IMM.f32.slot1.wait1 @r1, r60^, r61^, index:0x2, table:0x1          ; a_in1.x only
FADD.f32 r1.h0, r1^, 0x0.neg                  ; a_in1.x narrowed
FMIN.v2f16 r0, r0^.h00, r1^.h00               ; iand(flt(0,a), flt(0,b)) -> flt(0, fmin(a,b))
FCMP_OR.v2f16.lt.m1 r0, 0x0, r0^.h00, 0x0     ; 0 < fmin(a,b), the mask in the low half
S16_TO_S32 r0, r0^.h0                         ; i2i32 of the 16-bit mask, for the 32-bit select
CSEL.u32.eq r0, r0^, 0x0, 0x0, 0x3F800000     ; b2f: color.x = mask ? 1.0 : 0.0
```

then the three constant lanes as `IADD_IMM.i32 r4, 0x0, #0x0` (y, z) and
`#0x3F800000` (w), each stored: they already match (intval section 8, the
`kept` rule). One lane narrows alone (`FADD.f32 r.h0`, `-0.0` as the second
source), a pair together (`V2F32_TO_V2F16`, 4.3); the lane is narrowed
**where the compare reads it** (after its attribute load, before the other
load), and a loaded attribute vector is cut at the last lane any user reads
(`.v3` for `.z`, `.f32` for `.x` alone, as `vs_narrow_input` already does for
`vsmat2_mediump`).

### 4.2 What is already in `emit.rs`/`bir.rs`/`pack.rs` for this

`Alu::Fcmp(c)` at `(16, 1|2)` is `FCMP_OR.v2f16.<c>.m1` (:322); `Fmin/Fmax`
`(16, 1|2)` `FMIN/FMAX.v2f16` (:325-331); `Icmp(Eq|Ne)` `(16, 1)`
`ICMP_OR.v2u16` (:433, first source `lone`, i.e. a lane in both halves);
`Ior/Iand/Ixor` `(16, 1)` `LSHIFT_OR/AND/XOR.v2i16` (:446, the second source
read whole, a `.h11` moved first with `IADD.v2u16 x.h11, 0`); `I2i32`
`(32, 1)` `S16_TO_S32` (:390); `B2f` `(32, 1)` `MUX.i32` which `va_lower_isel`
makes the `CSEL.u32.eq` (the 32-bit select of `opvs_less_ivec4`); `Fabs`
`(16, 1)`. **No emit row is needed** for 4.1; the vertex NIR must produce
`Alu` nodes with `bits: 16` and these widths. `select.rs` :990-1020 already
gives a 16-bit operation's operands their halves (`.h0`, `.h01`, `.h00`).

### 4.3 `bvec3 != bvec3` of `greaterThan(mediump vec3 swizzle, 0.0)` (`opvs_not_equal_bvec3`)

```
LD_ATTR_IMM.v3.f32.slot0.wait0 @r0:r1:r2 ...index:0x1        ; a_in0.xyz
V2F32_TO_V2F16 r0, r0^, r1^                    ; (x, y) in one word
FADD.f32 r1.h0, r2^, 0x0.neg                   ; z alone
LD_ATTR_IMM.v3.f32.slot1.wait1 @r2:r3:r4 ...index:0x2          ; a_in1.xyz
V2F32_TO_V2F16 r2, r2^, r3^
FADD.f32 r3.h0, r4^, 0x0.neg
FCMP_OR.v2f16.lt.m1 r2, 0x0, r2^, 0x0          ; (0 < in1.x, 0 < in1.y): two lanes in one op
FCMP_OR.v2f16.lt.m1 r0, 0x0, r0^, 0x0          ; (0 < in0.x, 0 < in0.y)
ICMP_OR.v2u16.ne.m1 r4, r0.h11, r2.h00, 0x0    ; lane 2: in0.y != in1.x   (swizzled operands)
FCMP_OR.v2f16.lt.m1 r3, 0x0, r3^.h00, 0x0      ; 0 < in1.z
ICMP_OR.v2u16.ne.m1 r0, r0^.h00, r3^.h00, 0x0  ; lane 1: in0.x != in1.z
LSHIFT_OR.v2i16 r0, r4^.h00, 0x0.b00, r0^      ; any: (lane2 | lane1)
FCMP_OR.v2f16.lt.m1 r1, 0x0, r1^.h00, 0x0      ; 0 < in0.z
ICMP_OR.v2u16.ne.m1 r1, r1^.h00, r2^.h11, 0x0  ; lane 0: in0.z != in1.y
LSHIFT_OR.v2i16 r0, r0^.h00, 0x0.b00, r1^      ; any: (.. | lane0)
S16_TO_S32 r0, r0^.h0
CSEL.u32.eq r0, r0^, 0x0, 0x0, 0x3F800000
```

Facts to take from it: (a) the compares of a *pair* of lanes are one
`FCMP_OR.v2f16` of the narrowed word, the single lane its own; (b) the
bvec `!=` is not `bany(fne)`: `ine` of two 16-bit masks per lane
(`ICMP_OR.v2u16.ne`), `ior`ed in the order lane 2, lane 1, lane 0 (`any`
reduces `ior(ior(l2, l1), l0)`); `vsbvec3`'s `all(==)` on bool words is
y, x, then z (`vs_reduce`, 32-bit): the order depends on the reduction rule
and **must be measured per width**; (c) the compare order follows
`nir_move_comparisons` (each compare before its first user), and the first
`FCMP_OR` of the pair is the *second* operand's. The last two lines are the
select of 4.1.

### 4.4 Compares of fp16 uniform words and `|a - b| < 0.05` (`vsapi_mat2`, `vsapi_struct`)

```
FMAX.v2f16 r0, u5.w1.abs.h00, u5.w1.abs.h11     ; max(|m00|, |m01|): the two halves of one fp16 uniform word
FMAX.v2f16 r1, u6.w0.abs.h00, u6.w0.abs.h11
FMAX.v2f16 r0, r0^.h00, r1^.h00
FCMP_OR.v2f16.lt.m1 r0, r0^.h00, u6.w1, 0x0     ; max < 0.05 (the constant 0x2a66 in the FAU)
S16_TO_S32 r0, r0^.h0
CSEL.u32.eq r0, r0^, 0x0, 0x0, 0x3F800000
FMA.f32 r0, r0^, 0x0, r0^                        ; the exact ffma(x, 0.0, x) of the convergent lane
```

(`vsapi_mat2`, Mesa reads `|a - 0|` as `|a|`, folds the four `b2f` products
into `b2f(iand(...))` and `iand(flt(abs(x), c), flt(abs(y), c))` into one
`flt(fmax(abs x, abs y), c)`: the fragment walk's `and_masks` /
`cmp_product` have the same rules, `nir_fs.rs` :2128-2182.) `vsapi_struct`
adds the struct members stored fp32 (a struct's member is not lowered): they
are narrowed from the uniform word where read (`V2F32_TO_V2F16 r0, u6.w1,
r0^` after `MOV.i32 r0, u7.w0` to make a register of the second word; `FADD.f32
r2.h0, u7.w1, 0x0.neg` for a lone word; `MKVEC.v2i16` re-pairs the halves).
These two references need M1 (narrowing a uniform word), M2 (the `fmax`
rule with `abs`) and M3's `b2f(iand)` product fold, and are the last ones
in the order (section 6).

### 4.5 Integer uniforms in the vertex stage

An int uniform is one raw 32-bit word of the FAU/UBO at every precision
(`mali-int-uniforms.md` section 2; int16 is off, `lower_precision.cpp:176`),
and an arithmetic on it is the same NIR as on any other 32-bit int:

```
; deqp_loops_vs_for_uni   uniform mediump int ui_three;  for (ndx < ui_three)
IADD_IMM.i32.reconverge r2, 0x0, #0x0
ICMP_OR.s32.ge.m1 r1, r2, u5.w1, 0x0          ; the loop test: the uniform's word as the second source
BRANCHZ.reconverge r1^.h0, offset:4
...
IADD_IMM.i32 r2, r2^, #0x1                    ; ndx++
; deqp_loops_vs_select    ub_true ? ui_three : 0   (uniform bool ub_true, int ui_three)
MOV.i32 r1, u6.w0                             ; the int word moved into a register first
CSEL.u32.eq r1, u5.w1, 0x0, 0x0, r1^          ; word of the BOOL as the 32-bit mask: eq 0 ? 0 : ui_three
ICMP_OR.s32.ge.m1 r1, r2, r1^, 0x0
; deqp_arith_uniform_api_bool_compare_vs_none   bvec2 u == bvec2(false, true)
LSHIFT_OR.i32 r0, 0x0, 0x0.b00, u5.w1.not     ; ieq(y, true)... the constants decide not/or/and
LSHIFT_AND.i32 r0, u6.w0, 0x0.b00, r0^
CSEL.u32.eq r0, r0^, 0x0, 0x0, 0x3F800000
FMA.f32 r0, r0^, 0x0, r0^
; reslib_int_vs (fragment, section 9 of intval): float(ui):  S32_TO_F32 r1, u2.w0 ;  ui + 1:  IADD_IMM.i32 r2, u2.w0, #0x1
```

The `ivec`/`uint` loads are `uniform_of`'s `comps` words (`u2.w0`..`u3.w0`);
a mediump int/ivec is read exactly like a highp one (no `f2f16`, no
narrowing: `ints` never go through `lower_precision`). A mediump *float*
result of an int op (`float(ui)` stored to a mediump local) narrows after
the `S32_TO_F32` (`vs_narrow_stored`, already in place). The FAU word order
of a bvec/ivec uniform is the *use* order, not the declaration order
(`vsbvec3`'s `DIFFERS` is that; a separate defect, intval section 8 /
unitype "Lowering result", **not** part of this item).

### 4.6 The bvec2 forms the arith references already hold (read after the first draft of this note)

`arith-ref` committed vertex references of the same shapes at width two, all
`mediump vec4` attributes, `bvec2 in0 = greaterThan(a_in0.wy, vec2(0.0))`
(`tools/mali/testdata/deqp_arith_*`, quoted from their `text`):

```
; op_bool_all_vs_none: all(in0)             ; op_bool_any_vs_none: any(in0)
FADD.f32 r0.h0, r1^, 0x0.neg   (y)           same two narrowings
FADD.f32 r1.h0, r3^, 0x0.neg   (w)
FMIN.v2f16 r0, r0^.h00, r1^.h00              FMAX.v2f16 r0, r0^.h00, r1^.h00
FCMP_OR.v2f16.lt.m1 r0, 0x0, r0^.h00, 0x0    (same)
S16_TO_S32 r0, r0^.h0 ; CSEL.u32.eq r0, r0^, 0x0, 0x0, 0x3F800000

; op_binary_equal_vs_none: in0 == in1 (in1 = greaterThan(a_in1.yx, 0.0))
FADD.f32 r0.h0, r1^, 0x0.neg ; FADD.f32 r1.h0, r3^, 0x0.neg      ; in0.y, in0.w
LD_ATTR_IMM.v4 ... index:0x2 ; V2F32_TO_V2F16 r2, r2^, r3^        ; in1.x, in1.y as a pair
FCMP_OR.v2f16.lt.m1 r0, 0x0, r0^.h00, 0x0
FCMP_OR.v2f16.lt.m1 r2, 0x0, r2^.h10, 0x0                       ; the pair swapped (.h10): yx
ICMP_OR.v2u16.eq.m1 r0, r0^.h00, r2.h11, 0x0
FCMP_OR.v2f16.lt.m1 r1, 0x0, r1^.h00, 0x0
ICMP_OR.v2u16.eq.m1 r1, r1^.h00, r2^.h00, 0x0
LSHIFT_AND.v2i16 r0, r0^.h00, 0x0.b00, r1^      ; != is the same with .ne and LSHIFT_OR.v2i16
S16_TO_S32 r0, r0^.h0 ; CSEL.u32.eq r0, r0^, 0x0, 0x0, 0x3F800000
```

This settles what 4.1 and 4.3 left "by analogy": `any` of two `0 < x` masks
is `flt(0, fmax(a, b))` (`FMAX.v2f16`, the mirror of the `fmin` rule, so M2's
`ior` rule **has a reference**), `all` is `FMIN`; the width-two `bvec` `==`
is `ball_iequal2` = `iand` of two lanes' `ieq`, `!=` the `ior` of their `ine`
(`ICMP_OR.v2u16`, then `LSHIFT_AND` / `LSHIFT_OR.v2i16`); the pair of the
second operand is read swizzled in place (`r2^.h10`, `r2.h11`, `r2^.h00`)
without a move. Width three (`opvs_not_equal_bvec3`) differs from width two
only by the third lane's `FCMP`/`ICMP`/`LSHIFT_OR`. Width four is the only
one still without a reference.

`arith_op_binary_{equal,not_equal}_vs_mediump` is the *integer* neighbour:
`ivec2(a_in0.wy) == ivec2(a_in1.yx)` of mediump attributes, 32-bit (`F32_TO_S32.rtz`
per lane, then `ICMP_OR.u32.eq.m1` / `ICMP_AND.u32.eq.m1 r0, r1^, r2^, r0^` with
the first compare's mask as the accumulator, `ne` as `ICMP_OR.u32.ne.m1` twice).
No 16-bit form is involved; it is refused today only by `vs_reduce`'s width
(`Arithmetic`) and belongs to M3's `ball`/`bany` arm for ints (`opt.rs`
`fuse_cmp` makes the accumulator form of the vertex `opvs_less_ivec4`'s `ICMP`
chain already).

## 5. The mechanisms

Each names what changes, the functions, the Mesa pattern (section 4), the
cases (section 3), and the existing references that must stay byte-equal.

### M1. Narrow a mediump operand where a mediump compare reads it

*What.* In the vertex NIR walk, a compare (`Flt`, `Fle`, `Feq`, `Fne`) the
front end classed `Prec::Medium` (`m.prec_of(ValueId(id.0)) == Prec::Medium`
for the compare; **check first** that the front end gives the compare the
precision of its operands, the way `Fmul`/`Fadd` get theirs — `precision.rs`
`prec_of`, `Module::prec_of`; if a compare is `Unknown` the precision must
come from its operands' precision, as `vs_tex_read16` does for texels) reads
its operands through `vs_narrow_input` (an attribute's lane, narrowed:
:3284, whole vector in aligned pairs, an odd lane alone: `V2F32_TO_V2F16`,
`FADD.f32 rN.h0`), a constant as its fp16 half (`vs_pairs16`'s rule, `as_f16`),
a 16-bit lane as it is, a fp16 uniform word by `vs_alu16u`'s `Val::Pair`
rule. The compare node is `Alu::Fcmp(c)` with `bits: 16` and width 1 or 2
(`node16`'s analogue in the vertex walk: `NirNode { kind, src, bits: 16,
width }`).
*Functions.* `nir.rs`: a new `vs_cmp16` beside `vs_alu16` (called from the
arms at :4892-4926 under the same guard `Prec::Medium && vs_pairs16-or-uniform
operand`), reusing `vs_narrow_input` unchanged; `select.rs` `vs_variant`
:990-1020 reads the halves already; `emit.rs` rows exist.
*Pattern.* 4.1 lines 1-4 and the compares of 4.3 (pair compare of the
narrowed word, lone lane compare). An `Fle` is `Fge` with swapped sources
(GLSL's `<=`, `nir_fs.rs` :2910), `Feq/Fne` as `Feq/Fne` at 16.
*Cases.* The 60 float compares and the 27 bool ones (the bool inputs are
`a_in0.z > 0.0` compares); `opvs_logical_and`, `opvs_not_equal_bvec3`.
*Risk.* `vs_narrow_input` is the helper `vsmat2_mediump`, `vsmat3_mediump`
(MATCHED) use: do not change its node cache or its pair split; give the new
caller the same `Kind::Alu(F2f16)` node so the CSE (`position` search) keeps
one narrowing per vector.

### M2. The `fmin`/`fmax` rule and the 16-bit mask

*What.* `iand(flt(c, a), flt(c, b))` of two 16-bit masks with the **same
constant on the left** (GLSL's `a > 0.0` is `flt(0.0, a)`) is
`flt(c, fmin(a, b))` (`FMIN.v2f16` of two lanes in `.h00`, one
`FCMP_OR.v2f16.lt.m1 r, c, m.h00, 0`); the mirror of `nir_fs.rs` `and_masks`
(:2128-2139), which has the constant on the right and `fmax`. `ior` is
`flt(c, fmax(a, b))` by the same family of rules (nir_opt_algebraic.py
:1158-1165; **no reference**). Other pairs stay a 16-bit `iand`/`ior`/`ixor`
(`LSHIFT_*.v2i16`, `emit.rs` :446). The 16-bit mask reaches a 32-bit
consumer through `Alu::I2i32` (`S16_TO_S32 r, m.h0`) and then the existing
`B2f` (`CSEL.u32.eq`) or `Bcsel` arm.
*Functions.* `nir.rs` the `AluOp::Iand`/`Ior`/`Ixor` arms (:5008-5080):
when both operands are 16-bit masks (`vs_is_f16`-like `vs_mask16`: a 16-bit
`Fcmp`/`Icmp`/`Iand`/`Ior`/`Ixor` node, not a mix with 32-bit ones, which
stays refused as `Fs::mask_op` refuses), build the 16-bit node or the rule;
the `B2f`/`F2b`/`Bcsel`/`I2b`/`Inot` arms (:4908-4935, :5035-5090) gain
`vs_widen_mask`: `Alu::I2i32` of a 16-bit mask before a 32-bit use; also
`vs_is_bool` (:2965) must count a 16-bit mask node as a boolean.
*Pattern.* 4.1 lines 5-8.
*Cases.* `opvs_logical_and` (DIFFERS to byte-equal); `logical_and.bool_vertex`
(1); with `ior`/`ixor`/`not` the other three (references R2-R4 needed).

### M3. `any`/`all`, `ine`/`ieq` over 16-bit masks (the `bvec` family)

*What.* `vs_reduce` (:3612) takes the reduction of 16-bit masks: `bany` of
`ine(m_a, m_b)` lanes is the 16-bit `ior` chain of 4.3 (`ICMP_OR.v2u16.ne.m1`
of a lane of the first mask in both halves, `lone`, and the second's, then
`LSHIFT_OR.v2i16`), `ball` its `iand`/`ieq` dual; `bvec` `==`/`!=` is the
`all`/`any` of these (GLSL `==` of vectors is `ball_iequal`, `!=` is
`bany_inequal`). Pairs of `FCMP_OR.v2f16` serve two lanes of the operands
(4.3 line 7-8). The order of the `ior` chain and of the compares is Mesa's
(4.3 (b), (c)): lane 2, lane 1, lane 0 for width 3; the widths 2 and 4 **need
references** (R5, R6) before coding the order.
*Functions.* `nir.rs` `vs_reduce` (new arms before `_ => Err`), a helper
`vs_mask_chain` taking the lanes and the reduction; `Alu::Icmp(Ne)` at
`(16, 1)` and `Alu::Ior` at `(16, 1)` in the walk's table (emit rows exist,
:433, :446); `vs_is_bool` as M2.
*Pattern.* 4.3.
*Cases.* `opvs_not_equal_bvec3`; `bool_compare.*.bvec{2,3,4}` (15),
`binary_operator.{equal,not_equal}.{bool,bvec2..4}` (8) and the `lowp/mediump
vec{2,3,4}` `==`/`!=` of `binary_operator` (12) and `float_compare`'s `equal`/`notEqual` (12) — all after their references.

### M4. Mediump compare results into a colour, fp16 uniform operands

*What.* `vsapi_mat2`/`vsapi_struct`: the 0.05 compare of fp16 uniform words
(4.4): `abs` source modifiers on a 16-bit `Fmax` (`FMAX.v2f16 ..abs.h00,
..abs.h11`: one word's two halves), the product-of-selects fold
(`b2f(iand)` of `nir_fs.rs` `cmp_product`, :2160-2200), the narrowing of an
fp32 struct member's word (`V2F32_TO_V2F16 r0, u6.w1, r0^` after a `MOV.i32`).
*Functions.* `nir.rs` `vs_alu16u` / `vs_uniform16` grow `Fmax`, `Fabs`, the
compare; a vertex `cmp_product` ported from the fragment walk (the
`Fs::cmp_chain` state moves into `Nir`).
*Cases.* `deqp_unitype_vsapi_mat2`, `vsapi_struct`, and the `uniform_api`
mediump matrix/struct compares of the same `compare_*` helpers (30 of
`T3`/`P2`, unitype section 2) — a **large** port; put last, and only if
vs16-1 and 2 leave room.

### M5. Integer uniforms as values

*What.* (a) `mod.rs:1364`: a uniform of an integer type (`int`, `ivec2/3/4`,
`uint`, `uvec2/3/4`; `bool` already) is excluded from the `UniformPrecision`
test whatever its precision, since it is one raw 32-bit word (4.5):
`nir::int_type(gl_type)` beside `bool_type`. (b) `nir.rs:4456`: drop the
`index_only || int_compare_only` condition for `GL_INT`; the value is the
uniform node's raw lane, as an `ivec` already is. (c) The hazard of section 0:
`vs_is_uniform_word` (:2958) must be true only for a lane of a `Kind::Uniform`
node whose declaration is a `bool_type` (by `dword`, as `nir_fs.rs`'s
`bool_dword` does through `m.uniforms`; the vertex `Nir` needs the module or a
per-node flag at the `Uniform` node's creation), else `bool(int_uniform)`
(`I2b`, :5086) would become the word itself and `ieq(int, true)` would fold to
`int`; an int's `i2b` is `ine(w, 0)` = `Icmp(Ne, w, 0)` (the fall-through arm
:5087) and `vs_is_bool` (:2965) must not call an int word a bool.
(d) The `int_compare_only`/`index_only` helpers stay for the loop gate and
the uniform array index (uniarray); their uses in the arm go.
*Pattern.* 4.5 (no new word: the arms of step 2 take a `Val::Lane(uniform)`).
*Cases.* The gates for the 346/116/99 rows; see section 3.2: the count that
**passes** is measured.
*Risk.* `deqp_loops_vs_*` pin `UniformPrecision`/`UniformType` text (4 hits
in `tests/mali_deqp_loops.rs`): their pinned refusal moves to the loop's.
`tests/mali_deqp_uniarray.rs` (2 hits) the same.

### M6. A uniform-mediump `ivec`/`bvec` in structs and `info_query`

*What.* `vs_struct_uniform`: the struct members of int/ivec/bvec types.
`uniform_of` already sizes them; `mod.rs`'s `member` exemption (:1356)
already skips the precision gate; the remaining refusal is `nir.rs:4456`
for an `int` member (fixed by M5b) and `uniform_precision` finding no
declaration for a member of a nested struct or an array of structs (the
nested lookup is uniarray's, `mali-deqp-unitype.md` M3). No change beyond M5
for the flat struct shapes (`basic_struct.int_ivec4`, `shaders.struct.*`).
*Cases.* `uniform_api` `basic_struct.int_ivec4` 6, `shaders.struct` 37 (the
`ui_one` index shapes: `IndexedLocal` behind), `info_query` 12.

## 6. The split into sessions

1. **vs16-1 (one session, no new reference).** M1 + M2 + the M3 arm for
   `bvec3`, scalar compares: `opvs_logical_and` DIFFERS -> LOWERED,
   `opvs_not_equal_bvec3` REFUSED -> LOWERED (2 references byte-equal); host
   forecast: `logical_and.bool_vertex`, `not_equal.bvec3_vertex`/
   `bool_compare.notEqual.bvec3_vertex` and the scalar `float` compares at
   `{lowp,mediump}` (8) pass or move on, **8 + a few cases**; the other
   shapes of 3.1 keep the 32-bit path (no regression: the 16-bit path is
   entered only for a `Prec::Medium` compare whose operands are mediump
   attributes/constants/16-bit lanes, the guard of `vs_pairs16`).
2. **vs16-2 (a short session).** M5 (+ M6): the two gates and the
   `vs_is_uniform_word` fix; tests: the unit tests of `tests.rs` for
   `bool(int_uniform)` and `ieq(int, true)` (the hazard), the pinned
   refusals of `mali_deqp_loops.rs`/`mali_deqp_uniarray.rs` moved; the
   `deqp_loops_vs_*` references stay refused at the loop. Forecast: measured
   on the host with `tools/mali/deqp-host-measure.sh` (the whole 346 + 116 + 99
   move to their next refusal; passes only where nothing else is behind).
3. **vs16-3 (a session after the references of section 7).** M3 for widths
   2 and 4 and the whole-vector `==`/`!=`, `ior`/`ixor`/`not` (M2's rest), the
   `float_compare` vector forms; **M4** only if the references and time are
   there, otherwise a vs16-4. Forecast: the remaining of the 87 (about 70
   cases first refusal).

If vs16-1 does not fit one session, cut M3 (keep `opvs_logical_and`, M1+M2:
the first reference alone is the smallest byte-equal win and proves M1).

## 7. References still to generate (not generated here)

All with `tools/mali/deqp_ref.sh <pair>.vert <pair>.frag <prefix> --instrument
shim` (mediump: the shim, not pan_shc), drawn by `--uniform-values`; the pair
is dEQP's own generated shader (`shaders.operator`'s `ShaderOperatorCase`
`_vertex`), in `tools/mali/testdata/deqp_intval_*` by that note's naming.

| # | shape | for | needed because |
|---|---|---|---|
| R1 | `float_compare.lessThan.mediump_vec3_vertex` (and one `equal`, one `notEqual`) | M1/M3 vector forms | no reference of a `bvecN` of fp16 compares widened lane by lane into the colour |
| R2 | `logical_or.bool_vertex` | M2 | the `flt(0, fmax)` rule is held by `deqp_arith_op_bool_any_vs_none` (section 4.6); the scalar `||` of two `a_in.x > 0.0` itself is not, generate only if vs16-1 wants the byte proof |
| R3 | `logical_xor.bool_vertex` | M2 | `ixor` of two 16-bit masks |
| R4 | `not.bool_vertex` | M2 | `inot` of a 16-bit mask into the select (arms swapped?) |
| R5 | `bool_compare.equal.bvec4_vertex`, `.any`/`.all` `bvec3`/`bvec4` | M3 | order of the `ior`/`iand` chain at widths 3 and 4. **Width 2 is held** (section 4.6: `deqp_arith_op_bool_{all,any}_vs_none`, `op_binary_{equal,not_equal}_vs_none`), only 3 and 4 are missing |
| R6 | `binary_operator.equal.mediump_vec4_vertex`, `not_equal.mediump_float_vertex` | M3 | `==` of whole *float* vectors and scalar `fneu` at 16 (no float-vector or scalar `==` reference at 16 bits exists; the held `_vs_none`/`_vs_mediump` ones are bvec2 and ivec2) |
| R7 | `binary_operator.less.lowp_float_vertex` | M1 | confirm lowp = mediump words |
| R8 | `uniform_api.value.initial.render.basic.int_vertex`, `.ivec3_vertex`, `basic_struct.int_ivec4_vertex` (vertex) | M5 | the int/ivec compare of uniforms against constants (`ieq` + `ball`), no reference of a vertex int uniform compare other than the loop test |
| R9 | `uniform_api` `bvec2/4` with a true/false mix of the dEQP values | M5/M3 | the constant pattern decides the `inot`/`ior`/`iand` form (`deqp_arith_uniform_api_bool_compare_vs_none` is `bvec2(false, true)`; `vsbvec3` the all-false 3) |
| R10 | `shaders.indexing`/`loops` `ui_*` shapes that use an int uniform as a value (not a loop bound) | M5 | to see what Mesa makes of `ui_one` as an operand |

`intval3-ref` (running) may already cover R8-R10: **read its note
`docs/notes/mali-deqp-intval3.md` before generating**, and generate only what
it does not hold.

## 7a. The references of section 7: what was generated (plan #387, `posvary-lower`)

Mesa 26.2.1's drm-shim (`$CONNECTED_DEPS_DIR/mesa-shim`, built as section 1
of `mali-deqp-irindex.md` says), `tools/mali/deqp_ref.sh <pair>.vert
<pair>.frag tools/mali/testdata/deqp_vs16ref_<shape> --instrument shim`,
uniform values from `gl_ref.py`. 23 references, pinned by
`tests/mali_deqp_vs16ref.rs`. **Caveat:** the VK-GL-CTS checkout available
(`VK-GL-CTS-gles3`) holds only `data/gles3/shaders`, not the C++ that
generates `ShaderOperatorCase`/`ShaderUniformCase`, and no deqp-gles2 was
built, so the pairs are written in the form of the held dEQP-derived pairs
(`deqp_intval_opvs_*`, `deqp_arith_op_bool_*`, `deqp_arith_uniform_api_*`),
with their template and the swizzles `zxy`/`yzx` (width 3), `wzyx`/`yzwx`
(width 4). Each `.license` says so. The words are Mesa's for that source;
re-draw from a `.qpa` if a case's exact swizzle ever matters.

| R | reference(s) `deqp_vs16ref_*` | at the tip |
|---|---|---|
| R1 | `cmp_lt_mediump_vec3`, `cmp_eq_mediump_vec3`, `cmp_ne_mediump_vec3` | compile, varying region differs |
| R2 | `or_bool` | compiles, differs (23 vs 25 words) |
| R3 | `xor_bool` | compiles, differs (23 vs 26 words) |
| R4 | `not_bool` | compiles, differs (20 vs 22 words) |
| R5 | `bany_bvec3`, `ball_bvec3`, `bany_bvec4`, `ball_bvec4`, `bcmp_eq_bvec3`, `bcmp_ne_bvec3`, `bcmp_eq_bvec4`, `bcmp_ne_bvec4` | refused (arithmetic) |
| R6 | `eq_mediump_vec4` (refused), `ne_mediump_float` (compiles, differs) | |
| R7 | `lt_lowp_float`, `lt_mediump_float` | compile, differ |
| R8 | `uapi_int`, `uapi_struct_int_ivec4` (refused: uniform type), `uapi_ivec3` (refused: vertex mediump uniform) | |
| R9 | `uapi_bvec4_mix`, `uapi_bvec3_mix` | refused (arithmetic) |
| R10 | none generated: **held** by `deqp_loops2_vs_do_dyn_conditional_break` (`i == one*ui_one`), `deqp_loops2_vs_do_uni_infinite_with_conditional_break` (`i == ui_one`), `deqp_loops2_vs_ret_infinite` (`i += ui_zero`), `deqp_loops_vs_vector` (`ivec4(0, 1, ui_three, 0)`), `deqp_loops_vs_select`, and `deqp_arith_random_bany_vs_none` / `deqp_cfold2_random_mixed_f2i` (the `uniform mediump int r`/`b` of a random shader). No dEQP vertex shape uses an int uniform as a plain value outside these (R8's compare is the only other). | held |

What the words show (all are in the references' `vs.varying` text):

- **`any`/`all` at width 3** (`bany_bvec3`): three fp16 compares are not made.
  Mesa never builds bools: `any` is `FMAX.v2f16` of the three
  *attribute values* and the result is tested once with
  `FCMP_OR.v2f16.lt.m1 r0, 0x0, max, 0x0` (0 < max), then `S16_TO_S32` and
  `CSEL.u32.eq ... 0x3F800000`. `all` (`ball_bvec3`) is the same with
  `FMIN.v2f16`. The third lane's `-x` is the `FADD.f32 r1.h0, r2, 0x0.neg`
  that the `fmin/fmax` rule of M2 produces. So the `flt(0, fmax)` rule
  extends to width 3 as a chain `fmax(fmax(x.h1, y.h0-pair), z)`: the
  first `FMAX.v2f16` pairs lane 1 (`r0.h11`) with the negated third lane,
  the second pairs that with lane 0. **No `ior`/`iand` appears.**
- **`any`/`all` at width 4** (`bany_bvec4`, `ball_bvec4`) *do* make bools:
  two `FCMP_OR.v2f16.lt.m1` (one per register pair) and a `LSHIFT_OR.v2i16`
  chain (`LSHIFT_AND.v2i16` for `all`) with the 0 shift: first
  `r0.h11 | r0`, then `r0.h00 | (r1.h11 widened by IADD.v2u16)`, then
  `| r1`: the order is lane 1 with lane 0 of pair 0, then lane 3 of pair 1,
  then lane 2. So width 4 is a left-to-right chain over the swizzled lanes
  `(x.y, x.x)`, `.w`, `.z`: read the order from the reference's words, not
  from `any`'s definition (it is not lane order; the lowering must copy it).
- **`bool_compare` `==`/`!=` at width 3/4**: per-lane
  `FCMP_OR.v2f16.lt.m1` (the `> 0` compares), `ICMP_OR.v2u16.eq.m1`
  (`.ne.m1` for `!=`) of lane pairs, then `LSHIFT_AND.v2i16` (`LSHIFT_OR.v2i16`
  for `!=`) of the per-lane results: `==` is an AND chain of lane `eq`,
  `!=` an OR chain of lane `ne`, at 16 bits, as `ior`/`iand` over 16-bit
  masks (M3). Width 3 has 3 `ICMP_OR` and 2 `LSHIFT_AND`; width 4 has 4 and
  3. The chain is not in lane order (lanes 1/0 of the first operand are
  compared to lane 0 of the second first).
- **Scalar `||`, `^^`, `!`** (R2-R4): `or_bool` is `FMAX.v2f16` of the two
  negated attribute lanes and one `FCMP_OR.v2f16.lt.m1 0, max` (the M2 rule
  again: `a > 0 || b > 0` is `0 < fmax(a, b)`, no integer op). `xor_bool` is
  two `FCMP_OR.v2f16.lt.m1` and one `LSHIFT_XOR.v2i16`. `not_bool` is **not**
  an `inot`: it flips the compare to `FCMP_OR.v2f16.ge.m1 r0, 0x0, -a`
  (`0 >= -a`), i.e. `!(a > 0)` is `a >= 0` on the negated lane.
- **Whole float vectors and scalars** (R1, R6, R7): a `mediump` vertex
  `lessThan`/`equal`/`notEqual` on `vec3` and `<`/`!=` on a scalar are plain
  fp32 `FCMP_OR.f32.<op>.f1`, one per lane, **no fp16 narrowing**; only
  the `bvecN = greaterThan(x, 0.0)` shapes narrow. `lowp` and `mediump`
  `less` give the same word (`lt_lowp_float` equals `lt_mediump_float`), so
  R7 holds: lowp = mediump. The whole `vec4` `==` (`eq_mediump_vec4`) is
  `FCMP_OR`/`FCMP_AND.f32.eq.m1` pairs and one `LSHIFT_AND.i32` chain:
  `FCMP_AND` takes the first two lanes, then two single `eq` joined with
  `LSHIFT_AND.i32`.
- **Int and ivec uniform compares** (R8): `uniform int u == 0` is one
  `ICMP_OR.u32.eq.m1 r0, u5.w1, 0x0, 0x0` straight on the uniform's FAU
  word (the int is in a FAU word at the uniform's slot, `.w1`), then the
  usual `CSEL` 1.0/0. `ivec3`/`ivec4` uniforms (`compare_ivecN` is the
  product of `compare_int`) are **not** a chain of `ieq`: the compare against the all-zero constant
  becomes `CSEL.u32.gt dst, u.wK, r, u.wK, r` (an unsigned max of the lane
  words, so nonzero if any lane is) over the lanes, then one
  `ICMP_OR.u32.eq.m1 ..., 0x0`. The struct's `int` member compares first and
  its `ivec4` joins through a fourth `CSEL.u32.gt`.
- **`bvec3`/`bvec4` uniform mixes** (R9): with the constant pattern known
  Mesa uses the uniform words directly: `LSHIFT_OR.i32 0, 0, u.w.not` (a
  `not` on the uniform word) for a lane compared with `false`, `LSHIFT_AND.i32`
  chains joining them, with *no* `ICMP`. `bvec4(false, true, false, true)`:
  one `not` per false lane, three `LSHIFT_AND.i32`s on top; `bvec3(true,
  false, true)`: one `not`, two `LSHIFT_AND`. The pattern decides the form,
  as R9 expected: a `true` lane contributes its word alone, a `false` lane
  the negated word.

Held-not-generated rows and their notes: R10 as above. Two cases the old table
hoped for (`equal.bvec3_vertex` of `bool_compare`, `not_equal.mediump_float`)
were generated too (`bcmp_eq_bvec3`, `ne_mediump_float`). `uapi_ivec3` shows
the vertex mediump-uniform refusal that `unif2` and `vs16-2` share: its words
are the same fp32 FAU words as the highp ones, not an fp16 uniform.

## 8. Tests at risk, and how to forecast

At risk (each is pinned by exact bytes or text):

- `tests/mali_deqp_intval.rs`: `LOWERED` gains `opvs_logical_and` (now in
  `DIFFERS`) and `opvs_not_equal_bvec3` (now in `REFUSED` with the
  `Arithmetic` text); `refusals_pinned` and the `DIFFERS` loop assert "now
  byte-equal" — move them in the same commit.
- `tests/mali_deqp_colourpair.rs` `DIFFERS`: `ck_lane_mediump` (the `res =
  in0 + in1` of mediump attribute lanes: `FADD.v2f16 r0, r0^.h00, r1^.h10`)
  and `rnd_vs_mediump_kept` (fp16 `FMA.v2f16` of `coords`): they differ today
  by the fp32 sums; M1's narrowing reaches them only if the `Fadd`/`Fmul`
  arm also uses `vs_narrow_input` for attribute-only operands (it does for
  uniforms, `vs_uniform16`). They may turn byte-equal as a *side effect*: the
  test then fails with "now byte-equal; un-ignore its test" — move them to
  the matched list. If M1 is written as a compare-only path they do not move.
- `tests/mali_deqp_unitype.rs`: `MATCHED` `vsmat2_mediump`, `vsmat3_mediump`
  (the `vs_narrow_input` and `vs_alu16u` bytes must not change);
  `DIFFERS` `vsapi_mat2`, `vsapi_struct` (M4 only), `vsmat3_lowp`,
  `vsbvec3` (FAU order: untouched by this item), `refusals_pinned`.
- `tests/mali_deqp_arith.rs` (`REFUSED`, each entry the exact `Arithmetic`
  text, `refusals_pinned` asserts it): `op_bool_all_vs_none`,
  `op_bool_any_vs_none`, `op_binary_equal_vs_none`, `op_binary_not_equal_vs_none`
  and `op_binary_{equal,not_equal}_vs_mediump` are M3's shapes: when M3
  lowers one it must move to `LOWERED` in that file in the same commit (the
  design listed the three files named by the item and missed this one).
  `uniform_api_bool_compare_vs_none` (179 cases) is `vs_reduce` at width two
  over bool *uniform words*, step 8 of `mali-deqp-arith.md` and owned there by
  `intval-rest`, not by this item; it is the same arm as M3 and its
  byte-compare is the same file.
- `tests/mali_deqp_loops.rs` (4 mentions of the precision/type text) and
  `tests/mali_deqp_uniarray.rs` (2) by M5.
- `src/shaderc/online/mali/tests.rs` (2 mentions of "not highp"; the int
  probes in `every_reachable_gap_is_reached_by_an_es100_source`),
  `tools/mali/corpus-verdicts.txt` (1: a shader whose verdict moves) —
  regenerate with `tools/mali/corpus-verdicts.sh --write`;
  `tests/fixtures/mali_glmark2_verdicts.txt` forward only.
- Generated files do not move (`tests/reference/*.rs`, `src/*/internal.rs`).

Forecast (HOST-ONLY, for the implementing item): `tools/mali/deqp-host-measure.sh
--cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" --out "$PWD/build/measure" --jobs 3
--chunk 100` as in families section 18, before and after each step; the
shapes of 3.1 by case name against `tools/cts/log/2026-11-15-t6-gles2-S*/refusals.tsv`.

## 9. Open questions for the implementer

1. Does the front end give a compare `Prec::Medium`? (M1, first thing to
   check; a `Module::prec_of` dump of `opvs_logical_and`.)
2. Is `lowp` identical to `mediump` for attributes (R7)? `lower_precision`
   treats both as `GLSL_PRECISION_MEDIUM`/`LOW` with the same `f2f16`; expected
   yes, not measured.
3. The order of compares in 4.3 comes from `nir_move_comparisons` and the
   scheduler: the vertex walk already runs `nir_move_comparisons` for 32-bit
   compares (intval section 8); confirm it moves a 16-bit `Fcmp` too
   (`nir.c:3292-3314` lists the opcodes, not the width).
4. `opvs_not_equal_bvec3` with the lanes' `FCMP_OR` pair/lone grouping depends
   on `nir_lower_alu_width` pairing `x,y` and leaving `z`: the same rule as
   `vs_narrow_input`; a bvec4 pairs (x,y), (z,w) — R5 confirms.

## 10. Overlap with the arith plan (`mali-deqp-arith.md` step 8)

`vs_reduce` is shared. The arith note's step 8 "vs-reduce" holds 265 cases:
179 `uniform_api.bool_compare` (bool uniform words, 32-bit, `intval-rest`),
48 `shaders.operator` (`op.binary.equal/not_equal` 21 each, bool `all`/`any`),
24 `shaders.random` (`random_bany`: 22), `other.bany`/`ball` 4 each. The
operator and random parts (about 86) are this note's M3 (the 16-bit masks of
mediump attributes) and M3's `ivec` forms. So:

- the case count of M3 in section 3.1 and the 86 of the arith note are the
  same cases seen from two sides, not additive; the host measure (section 8)
  decides which text a case carries first;
- `intval-vs16` (M1 to M3) and `arith-lower-1` do not share a function today
  (`arith-lower-1`'s steps are the vertex transcendentals, the fragment `/` and
  the pair trees: `vs_reduce` is its step 8, later); whoever takes `vs_reduce`
  first adds the arms and moves the `mali_deqp_arith.rs` pins, the other
  rebases onto it. M3 as designed here is written as new arms *before*
  `_ => Err(Arithmetic)`, so the merge is a union of match arms;
- the 179 bool-uniform compares need no 16-bit form: `LSHIFT_OR.i32` /
  `LSHIFT_AND.i32` of the words with the constants deciding `.not`
  (`deqp_arith_uniform_api_bool_compare_vs_none`), the width-two sibling of
  `vsbvec3`'s `vs_reduce` arm (width three, y-x-z order). They are the
  largest single group behind the "integer uniform" gates (M5) and should be
  scheduled with vs16-2, not later, if `intval-rest` is not about to start:
  the same session adds the width-two (and four, after R9) arm of the 32-bit
  bool reduction.
