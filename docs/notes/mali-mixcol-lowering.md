<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): lowering design for the mixprec and colourpair leftovers (HOST-ONLY design)

Plan #387, item mixcol-design. This is a design note. It changes nothing
under `src/`, `tests/` or `tools/`. It reads the 18 Mesa 26.2.1 references of
item mixcol-ref (`tools/mali/testdata/deqp_mixcol_*`, pinned refused by
`tests/mali_deqp_mixcol.rs`, counts in `docs/notes/mali-deqp-mixcol.md`) and
the compiler at `a4b63f35`, and says how to lower each shape to Mesa's words
in one-session steps.

Read first: `mali-deqp-mixcol.md` (the shapes and counts; this note does not
repeat its group tables), `mali-deqp-mixprec.md`, `mali-deqp-colourpair.md`,
`mali-mixed-precision.md`, `mali-precision.md` (Mesa's width rule),
`mali-deqp-fclane.md` (the `gl_FragColor`-lane family these overlap),
`mali-deqp-intval.md`, and section 18 of `mali-deqp-families.md` (row 13
MixedPrecision 166, `ColourPair` and `ColourZeroHalf` rows).

**What is measured and what is read.** The case counts are mixcol-ref's, a
host measure at `bee6896d` (HOST-ONLY: first refusal of the 10,597
compile-refused cases of the 37305608 slices, never a board result). The
Mesa sequences below are read from the references' `fs.fragment` regions
(the `text` of each `expect.json`, from the drm-shim draw). **The refusal
site attributed to each shape is read from the source, not from a run**:
no build was made for this note. Step 0 below makes the site a measured
fact before any lowering.

## 1. Mesa's sequences, per reference

Notation: `.h0` writes (or reads) the low half; `x.h00` is a half
duplicated; every `FADD.f32 rN.h0, x, 0x0.neg` is Mesa's narrowing of an
fp32 value to fp16 folded into its producer (`x + -0.0`, the `.h0`
destination); a `V2F32_TO_V2F16` narrows two lanes at once. Constants a
fp32 op reads are fp16 literals widened by the `.h1` swizzle
(`0x38000000.h1` is 0.5; mali-mixed-precision.md line 81). Register names
change between references; the words are what is pinned.

The shared fact, true of all 18: **a mediump varying is loaded at fp32**
(`LD_VAR_BUF_IMM.f32 ... src_f32`) and narrowed where an fp16 operation or
the colour reads it, and **the colour word is built at fp16 and written as
`BLEND.v4.f16`** from two registers.

### 1.1 MIX, declared highp (`hp_*`, 64 cases)

| ref | cases | Mesa's sequence of `fs.fragment` (colour `BLEND` and `ATEST` left out) |
|---|--:|---|
| `hp_scale` | 23 | `FADD.f32.discard r0.h0, r0^.neg, 0x0.neg` (the highp `-in0`, narrowed in the same op), `U16_TO_U32 r0, r0^.h0` (the colour pair (lane, 0.0)), `FMA.v2f16 r0, r0^, 0x38000000.h11, 0x38000000.h11` (`* 0.5 + 0.5` on the word). The pair (0.5, 1.0) of the other word is a FAU `MOV.i32 r1, u2.w0`. |
| `hp_bool` | 17 | per lane `FCMP_OR.f32.eq.m1 r, a, b, 0x0` at fp32, then `MUX.v2i16 r, 0x0, 0x3C000000.h11, r^.h00` (the `b2f16` as a 16-bit select). Words: `MKVEC.v2i16 r1, 0x0.h0, r1^.h0` (constant first) and `MKVEC.v2i16 r2, r0^.h0, u2.w0.h0` (lane, FAU 1.0). |
| `hp_mat` | 15 | four `FADD_IMM.f32 r, v, #const` (the matrix columns as immediates), `FADD.f32 r1, r2^, r1^` (the lane sum), each colour lane narrowed alone by `FADD.f32 rN.h0, rN^, 0x0.neg`, paired by `MKVEC.v2i16 r0, r0^.h0, r2^.h0` and `MKVEC.v2i16 r1, r1^.h0, u2.w0.h0`. |
| `hp_param` | 4 | the highp parameter's `-a` and `2.0 * a` are fp32 (`FADD.f32 r1, r0, r0`; `FADD.f32 r1.h0, r1^.neg, 0x0.neg` narrows at the return); everything after the call is fp16 (`FADD.v2f16`, `FMA.v2f16`, `FCMP_OR.v2f16.ge.f1`, `IADD.v2u16`). `f` (mediump) is narrowed alone: `FADD.f32 r0.h0, r0^, 0x0.neg`. |
| `hp_loop` | 3 | the loop is unrolled and folded (`res.yzwx` three times is one rotation). `V2F32_TO_V2F16 r0, r0^, r1^` narrows lanes 0 and 1, `FADD.f32 r2.h0, r3^, 0x0.neg` lane 3; then `MKVEC.v2i16 r1, r2^.h0, r0.h0` and `MKVEC.v2i16 r2, r0^.h1, u2.w0.h0`. The highp counter leaves no word. |
| `hp_seq` | 2 | `F32_TO_S32.rtz r0, r0^`, `IADD.s32 r0, r0^, r0^`, `S32_TO_F32 r0, r0^` (all 32-bit), `FADD.f32 r0.h0, r0^, 0x0.neg`, `U16_TO_U32 r0, r0^.h0`, `FMA.v2f16 r0, r0^, 0x38000000.h11, 0x0.neg`. The sequence operator's first operand `in0 + in2` is dead and gone. |

### 1.2 MIX, an int read as a float or a float converted to int (`int_uniform*`, `f2i_*`, 102 cases)

| ref | cases | Mesa's sequence |
|---|--:|---|
| `int_uniform` | 61 | `S32_TO_F32.discard r1, u2.w0` (the int uniform is a FAU word, no `F16_TO_F32`), `FMA.f32 r0, r1^, r0^, 0x0.neg` (product with the **raw fp32 varying lane**, not a narrowed one), `FMA.f32 r1, 0x34000000.h1, r2^, 0x0.neg` (constant 0.25 as a widened half), `FADD.f32 r1.h0, r1^, 0x0.neg` and `FADD.f32 r0.h0, r0^, 0x0.neg` (narrow each lane), `U16_TO_U32 r1, r1^.h0`, `U16_TO_U32 r0, r0^.h0` (the colour lanes (x, 0.0), (z, 0.0)). |
| `int_uniform_struct` | 4 | `S32_TO_F32 r4, u2.w0` (int member), `F16_TO_F32 r1, u3.w0.h0` and `F16_TO_F32 r3, u2.w1.h0` (the **mediump float uniforms widened** from the fp16 FAU block), `FADD.f32 r1, r4^, r1^`, `FADD.f32 r1, r1^, r3^.neg`, `FADD.f32 r1.h0, r1^, 0x0.neg`. The other three lanes are fp16 from the narrowed varying (`V2F32_TO_V2F16 r2, r2^, r3^`, `FMA.v2f16 ... .h00` by FAU words, `FADD.v2f16 r3, r2.h00, r2.h00`). |
| `f2i_seq` | 3 | per lane `F32_TO_S32.rtz r, r^` then `S32_TO_F32 r, r^`, then `FADD.f32 r.h0, r^, 0x0.neg`, then `MKVEC.v2i16 r2, r3^.h0, r2^.h0` and `MKVEC.v2i16 r0, r1^.h0, r0^.h0` and `FMA.v2f16 r, r^, 0x38000000.h11, 0x0.neg` (`color * 0.5`). The `in0 + vec4(in1)` of the sequence's first operand is dead. |
| `f2i_rnd` | 34 | no conversion survives: `ATEST ... 0x0`, `MOV.i32 r0, 0x0`, `MOV.i32 r1, u2.w0`, `BLEND`. No varying load. |
| `f2i_dead` | (in the 34) | the conversion is dead: `NOP`, `MOV.i32 r0, u2.w0`, `MOV.i32 r1, u2.w1`: two FAU words. Not even a `vs.varying` region. |

(`f2i_rnd` and `f2i_dead` are the two forms of mixcol-ref's 34-case
`f2i_rnd` shape: `ivec4`/`int()` of a varying or uniform times constants
that fold the lane to 0, or a dead assignment.)

### 1.3 CK and LZ, the pair as an operand (77 cases)

| ref | cases | Mesa's sequence |
|---|--:|---|
| `ck_scale` (mediump) / `ck_scale_highp` | 9 | one `LD_VAR_BUF_IMM.v4`, `FADD.f32.discard r0.h0, r1^, 0x0.neg` and `FADD.f32 r1.h0, r3^, 0x0.neg` (the two lanes narrowed alone), `MKVEC.v2i16 r1, 0x0.h0, r1^.h0` (**constant first**, the zero an inline immediate), `FMA.v2f16 r1, r1^, 0x38000000.h11, 0x38000000.h11` on the word; `MKVEC.v2i16 r0, r0^.h0, u2.w0.h0` (lane, FAU 1.0) and the same `FMA.v2f16`. The highp variant differs in register order alone. |
| `lz_scale` | 9 | `FADD.f32.discard r0.h0, r2^, 0x0.neg`, `U16_TO_U32 r0, r0^.h0` (lane, 0.0), `FMA.v2f16 r0, r0^, 0x38000000.h11, 0x38000000.h11`; the (0.5, 1.0) word a FAU `MOV`. |
| `ck_sum` | 17 (random sums) | `FADD.f32.discard r0.h0, r0^, 0x0.neg`, `MKVEC.v2i16 r0, u2.w0.h0, r0^.h0` (FAU constant first), `FADD_IMM.v2f16 r0, r0^, #0x4800BE00` (the folded constant addend as an **immediate pair**); the other word a FAU `MOV.i32`. |
| `ck_rnd_other` | 2 | `LD_VAR_BUF_IMM.v3` then `V2F32_TO_V2F16.discard r0, r0^, r1^` (lanes 0, 1 narrowed together) and `FADD.f32 r1.h0, r2^, 0x0.neg`; `MKVEC.v2i16 r2, u2.w0.h0, r0.h0` (constant, lane) then `FMA.v2f16 r2, r2^, u2.w1, 0x0.neg` (the multiplier pair is one FAU word, `-0.0` in the second half); `MKVEC.v2i16 r0, r1^.h0, r0^.h1` (lane, lane) then `FMA.v2f16 r0, u3.w0, r0^, 0x0.neg`. |
| `lz_rnd` | 3 | `FADD.f32.discard r0.h0, r1^, 0x0.neg`, `FMA.v2f16 r0, 0x38000000.h11, r0^.h00, 0x0.neg` (the product at fp16 on a duplicated half), `FADD.f32 r1.h0, r2^, 0x0.neg`, then `U16_TO_U32 r1, r1^.h0` and `U16_TO_U32 r2, r0^.h0`. |
| `lz_rnd_other` | 2 | `V2F32_TO_V2F16.discard r0, r1^, r0^` (the pair's order swapped), `U16_TO_U32 r1, r0.h0`, `FADD_IMM.v2f16 r1, r1^, #0x38004480`, `MKVEC.v2i16 r0, r0^.h1, u2.w0.h0`, `FADD_IMM.v2f16 r0, r0^, #0xBC00C700`. |
| the 6 already referenced | 6 | `tests/mali_deqp_colourpair.rs`: `rnd_fs_ck_product`, `rnd_fs_ck_madd` (2 cases), `rnd_fs_zk_tree`, `rnd_fs_uniform_tz_ck` (2): the same forms; no new reference. |

The word, not the lane, is the operand: Mesa builds the pair (`MKVEC.v2i16`
with the constant as src0, or `U16_TO_U32` for (lane, 0.0)) **before** the
operation and runs the operation once at `v2f16`. Constant lanes of the
result are folded into a FAU word or an `FADD_IMM.v2f16` pair.

mixcol-ref's CK table rows sum to 58 for the 61 cases of its text
(ck_rnd_product 20, ck_rnd_sum 17, ck_rnd_madd 9, ck_scale 9, ck_rnd_other 2,
zk_tree 1); three cases of the 61 are not attributed to a row there. The
step that lifts the CK text should re-measure which three (step 4's
re-measure); the mechanism ranking below is not affected.

## 2. Where the compiler refuses, and what it lacks

### 2.1 MIX: three kinds of cause

1. **The declaration's `mixed` flag** (`FsPrecision::of`, `mod.rs:1093-1159`;
   reported at `mod.rs:1570`). It is set for a float local, const or
   function return of the other precision, a block `precision` statement or
   a parameter of the other width, with one exception: an ES 1.00 mediump
   shader's highp local is the walk's to judge (`FsPrecision::highp_locals`)
   and passes only when it is a `gl_FragCoord` copy
   (`highp_locals_copy`, `nir_fs.rs:814`, set into `f.mixed` at `:4239`).
   **Every `hp_*` reference is stopped here** (`hp_scale`: locals `in0`,
   `res`; `hp_bool`, `hp_seq`: the same; `hp_mat`: the top-level
   `const highp mat2`; `hp_param`: the `highp` parameter; `hp_loop`: the
   `highp float ndx` counter). What it lacks: the compiler has no notion of
   a highp *value* in a mediump shader. A highp-declared float expression is
   Mesa's fp32 expression; `Fs::op32` and the `fp32_lane` family
   (`nir_fs.rs:2638`) already compute an fp32 subtree in a mediump shader
   (the case 79 and 42 forms), and `Fs::widen_store` narrows (below), but no
   path starts from a *declared* precision.
2. **The walk's "product or sum on an fp32 value of a mediump shader" arm**
   (`nir_fs.rs:5791-5810`, `return Err(MixedPrecision)` at `:5806`). Its
   operand test `ok` accepts an fp32 lane (`fp32_lane`: `I2f`, `U2f`,
   `Fmul`, `Fadd`, `Ffma` at 32 bits, `:2638`), an fp32 select, an fp16
   lane or a constant. **A raw mediump varying lane (a `Kind::Input` lane,
   32 bits, not yet narrowed) and a mediump float uniform are in none of
   them**, but they are exactly what Mesa's `FMA.f32 r0, r1^, r0^` reads in
   `int_uniform`, and what `F16_TO_F32 r, u.w0.h0` widens in
   `int_uniform_struct`. This is the likely stop of `int_uniform` (65) and
   of the `i2f`/`f2i` conversions' neighbours in `f2i_seq`; read from the
   source, to be confirmed by step 0.
3. **A `mixed` set on a node the fold later kills** (`f2i_rnd`, `f2i_dead`,
   34 cases). `f.mixed |= ...` is set while the node is built
   (`mark_mixed` `:3035`, `fp16_operand` `:3883`, `:5466`, `:5614`, the
   `tex32` refusal `:3925`), and `mark_live` (`:1607`) runs after the walk.
   Mesa folds `x * 0.0` (nir_opt_algebraic `fmul a, 0.0 -> 0.0`) and deletes
   a dead assignment **before** any width question, so the conversion never
   reaches code. Here the flag is already set. What it lacks: the cause must
   be recorded on the node, and judged only if the node is live.

### 2.2 CK and LZ: the pair is a leaf, not an operand

All the refusals sit in the fragment pair builder:

- `ColourPair`, `nir_fs.rs:8857` (in `conversion_order`): any tree that
  holds a `Leaf::Mk { first: true, .. }` that is not the colour root.
- `ColourPair`, `:8792` (`vectorise`'s `(None, None, Const, load lane)` arm):
  the constant-first pair, allowed only when `f.colour` has it as a whole
  word (`word` closure at `:8789`). In `ck_scale` the pair *is* a colour
  word but also the operand of the `* 0.5 + 0.5`, so the word is read by an
  operation, not stored.
- `ColourPair`, `:10649` (`word_of`, highp shader, a constant then a lane).
- `ColourZeroHalf`, `:8757` (`vectorise`, lane then 0.0 outside a colour
  word) and `:9151` (`lane_word`, unless `vec_tree` or `uniform_sum`).

What they lack: a `P::Op` node over `P::Leaf(Leaf::Mk { first: true })` or
`Leaf::Zext`. `lanes_of` and the converted-lane table `tree` already
carry the leaf's lane; what is missing is the *emission* of the word before
the op (`select`, `:9830`) and the lane-conversion accounting
(`conversion_order`, `:8829`) for a lane that is read alone and as half of
the same word.

## 3. Mechanisms ranked by must-pass cases

| rank | mechanism | refs | cases | note |
|--:|---|---|--:|---|
| M4 | the colour pair as an operand | `ck_scale`, `ck_scale_highp`, `lz_scale`, `ck_sum`, `ck_rnd_other`, `lz_rnd`, `lz_rnd_other`, and the 6 of `mali_deqp_colourpair` | 77 (CK 61 + LZ 16) | overlaps fclane step 1 |
| M1 | an fp32 expression reading a raw mediump lane or uniform | `int_uniform`, `int_uniform_struct`, `f2i_seq` | 68 (61 + 4 + 3) | |
| M3 | a declared highp value in a mediump shader | `hp_scale` 23, `hp_bool` 17, `hp_mat` 15, `hp_param` 4, `hp_loop` 3, `hp_seq` 2 | 64 | 55 of 64 also need an fclane step |
| M2 | a width cause on a dead or folded node | `f2i_rnd`, `f2i_dead` | 34 | |

The four sum to **243**, mixcol-ref's total (MIX 166 + CK 61 + LZ 16).
Lowering M1 to M4 does not by itself make a case compile: the forecast
bounds of mixcol-ref (63 `hp_*`, 19 `int_uniform` with a rewrite) say a
case meets its next refusal. In particular, *after* M3 is lowered, the
`hp_scale`, `hp_bool` and `hp_mat` cases (55) are expected to stop at
`gl_FragColor lane` (fclane's text), because their colour lane is a
computed fp32 lane (this prediction is read, not measured: re-measure with
`--filter shaders.operator` and `shaders.matrix` after step 3).

### 3.1 Overlap with the `gl_FragColor`-lane family

`mali-deqp-fclane.md` section 5 owns these forms:

- fclane step 1, the operator colour pairs: "a computed fp32 lane narrowed
  alone (`FADD.f32 r.h0, r^, 0x0.neg`) as a pair half, the `MKVEC.v2i16`
  ... and the word's scale-and-bias `FMA.v2f16 word, word^, ...`". This is
  `hp_scale` (highp negation narrowed, `U16_TO_U32`, `FMA.v2f16` by 0.5) and
  the operator half of `ck_scale`/`lz_scale` with a **load** lane in place of
  a computed one. **The word emission is the same code**; the difference is
  the leaf (a load lane versus a narrowed computed lane). Do M4 steps 1 and
  2 below first for the load lanes; fclane-lower-1 then adds the computed
  lane to the same `vectorise`/`lane_tree` arms. Both edit `vectorise`
  (`:8415`), so they serialize in the plan: whichever lands second rebases
  on the other, and the later one's references do not change (the first's
  words are a subset).
- fclane step 2, the broadcast of one computed lane (compare mask): the
  `hp_bool` reference (17). It needs `intval-rest`/`arith-lower-1`'s compare
  lowerings (`FCMP_OR.f32.eq.m1` at fp32 and `MUX.v2i16`).
- fclane step 3, the matrix lane sums: `hp_mat` (15), `FADD_IMM.f32` of the
  scalar varying with literal columns.

`mali-fclane-lowering.md` (design in progress, plan item fclane-design) was
not merged when this note was written; its step numbers may differ. The
mapping above is to `mali-deqp-fclane.md` section 5.

### 3.2 Overlap with intval and the uniform families

- `intval-vs16` (step 3: mediump int compares and int uniforms in the
  **vertex** walk; `mali-intval-vs16-lowering.md` not merged at this
  writing). No mixcol shape is a vertex shader: all 243 cases refuse in the
  fragment shader with a compiling vertex shader. The fragment int uniform
  `S32_TO_F32 r, u2.w0` already exists (intval's fs step 1, `ab9cde96`);
  what is left in `int_uniform` is the *mix* (M1), not the integer. The two
  designs share one thing to check: that `fp32_lane`/`op32` treat the
  int-derived value the same way in both walks.
- `int_uniform_struct` (4 cases) reads a struct uniform and an int member:
  the `uniform_struct_partial_*` linkage cases need the struct-uniform
  support of the unif2 family (`mali-deqp-unif2.md`; plan item unif2-design).
  The M1 part (the widened float uniforms, `F16_TO_F32 u.w0.h0`) is here;
  the struct *layout* is not. `gl_ref.py` cannot set a struct uniform, so the
  reference is a draw with default values.
- `f2i_seq` and `hp_seq` (the sequence operator) read `F32_TO_S32.rtz` and
  `IADD.s32`: the int ALU of intval's fs step 1. They are M1/M3 only for the
  width.
- `hp_loop` (3) depends on the constant-iteration unrolling that
  `mali-deqp-loops.md` section 9 gives to loops-lower-1; the case lists
  `loops.*_constant_iterations.basic_highp_float`, which are loop-family
  cases in the dEQP groups too. Count them once, under whichever lands the
  highp loop counter.

## 4. Implementation steps (one session each)

The common check: byte-compare the reference with
`tests/mali_deqp_mixcol.rs` (remove the `#[ignore]` of the lifted test and
its `REFUSALS` entry in the same commit), and `cargo fmt --check`. Do not
run `tools/check.sh`. The tests most at risk are named per step. Features
for the test: `--no-default-features --features
host-test,gpu-mali,shaderc-online,no-shader-table` (the header of each).

### Step 0: make the site a measured fact (no case moves)

`Fs::mixed` is a `bool` (`nir_fs.rs:1078`), so a refusal does not say which
of the seventeen sites set it. Replace it by the line (or a `u8` cause code)
of the setter and expose it to a host test, then run the 18 references and
write the actual site of each beside section 2 above. This note's site
attributions for M1 and M2 are the likeliest, not proven. It changes no
output, so the pinned tests (`tests/mali_deqp_*`, `tests/mali_glmark2_corpus.rs`)
cannot move. If a site differs, correct steps 2 to 4 before starting them.

### Step 1: the pair as an operand, the operator harness (M4, 18 cases)

Reference lifts: `ck_scale`, `ck_scale_highp`, `lz_scale` (the `color * 0.5 +
0.5` harness over a load lane; the `unary_operator.{plus,post_increment_result,
post_decrement_result}` groups' 18 cases: 9 `_vec2`, 9 `_float`).

- `vectorise` (`:8415`): in the `(None, None, Const, load lane)` arm
  (`:8789-8792`), when the pair is an operand of a `P::Op` whose other
  operands are constants or uniform words (the `FMA` scale and bias), build
  `Leaf::Mk { first: true }` instead of refusing. Likewise the lane-then-0.0
  arm (`:8757`) builds `Leaf::Zext`.
- `conversion_order` (`:8829`): drop the `:8857` `ColourPair` for a
  `Leaf::Mk { first: true }` under a `P::Op`; the lane's narrowing is
  `FADD.f32 rN.h0, rM^, 0x0.neg` (what `convert` makes for a lane read
  alone, `:12887`).
- `select` (`:9830`): emit the `MKVEC.v2i16 r, <const>.h0, r^.h0` with the
  zero as an **inline** `0x0.h0` and a nonzero constant as a FAU half, or
  `U16_TO_U32 r, r^.h0`, then the op on the word (`FMA.v2f16 r, r^,
  0x38000000.h11, 0x38000000.h11`). The (0.5, 1.0) other word is a FAU
  `MOV.i32` (the fold already exists for `lz_scale`'s `u2.w0`).
- `word_of` (`:10649`): for a highp shader the `(Const, lane)` arm builds
  `Word::Lanes` as the mediump arm above it does, so `ck_scale_highp` (a
  highp varying read in a mediump shader, `f.other` bit set) works.

Words to match: section 1.3, `ck_scale` and `lz_scale`. Un-ignores the
three tests. Forecast (HOST-ONLY, upper bound): 18 cases leave the CK/LZ
text; mixcol-ref reports that 6 of 9 per text were also `gl_FragColor lane`
on the board, so expect fclane to meet some. At risk:
`tests/mali_deqp_colourpair.rs` (`ck_lane_highpcolour`, `lane_zero_load`,
`rnd_fs_kword_ck`: the existing words for a pair as a *colour word* must be
unchanged; keep `Leaf::Mk { first: true }` as a root exactly as now) and
`tests/mali_glmark2_corpus.rs` (the verdicts of the glmark2 shaders that
draw `vec4(c, x, ..)`; `tools/mali/corpus-verdicts.txt` must not move).

### Step 2: the pair as an operand, random sums and products (M4, 59 cases)

Reference lifts: `ck_sum`, `ck_rnd_other`, `lz_rnd`, `lz_rnd_other`, and in
`tests/mali_deqp_colourpair.rs` `rnd_fs_ck_product`, `rnd_fs_ck_madd` (2),
`rnd_fs_zk_tree`, `rnd_fs_uniform_tz_ck` (2): remove their `#[ignore]` and
their `refusals_pinned` entries.

- the same `vectorise` and `conversion_order` edits as step 1, now for
  `P::Op(Fadd | Fmul | Ffma)` over a pair and a *non-constant* operand (a
  second pair, as in `ck_rnd_other`'s `FMA.v2f16 r0, u3.w0, r0^`).
- the constant lanes of the result: `FADD_IMM.v2f16 r, r^, #imm` for a sum
  whose addend is a folded constant pair (`ck_sum` `#0x4800BE00`,
  `lz_rnd_other` `#0x38004480` and `#0xBC00C700`); `FMA.v2f16` with a FAU
  pair word (`ck_rnd_other` `u2.w1`, whose second half is `-0.0`); the
  `FMA.v2f16 r, 0x38000000.h11, r^.h00` form of `lz_rnd` (a duplicated
  half). `lane_word` (`:9151`, `lane_tree`, `:9338`) must accept `vec_tree`'s
  form for any lone tree (the 0.0 half is `U16_TO_U32`), per
  `mali-deqp-colourpair.md`'s plan.
- the conversion order: `V2F32_TO_V2F16 r0, r0^, r1^` (`ck_rnd_other`,
  lanes 0 and 1 together) versus `V2F32_TO_V2F16 r0, r1^, r0^`
  (`lz_rnd_other`, swapped) is `conversion_order`'s decision; the pinned
  words say which. This is the part most likely to need a second session
  (the `FsVaryingLoad` text of mixprec section 2.2 sits at `:7043` in the
  same function).

Forecast: 59 cases at most (CK 52, LZ 7). The three unattributed CK cases
of section 1.3 are found by the re-measure. At risk:
`tests/mali_deqp_colourpair.rs` as a whole, and
`tests/mali_deqp_fclane.rs`, because step 2 and fclane step 1 both edit
`conversion_order`; `tests/mali_fragment_random.rs`, whose random shaders
use the same trees.

### Step 3: a declared highp value in a mediump shader (M3, 64 cases, 9 compile alone)

Reference lifts that can match on their own: `hp_param` (4), `hp_seq` (2),
`hp_loop` (3, with the loop lowering). `hp_scale`, `hp_bool` and `hp_mat`
(55) lift only with fclane steps 1 to 3 (section 3.1).

- `FsPrecision::of` (`mod.rs:1093-1159`): stop setting `f.mixed` for a
  highp local, a highp const (top level and local), a highp parameter and a
  highp function return of an ES 1.00 mediump shader. Record the *name* (the
  `highp_locals` hash table, `[u64; 4]`, is the mechanism; extend it to the
  parameters, consts and the function return) so the walk knows a store to
  that local is an fp32 value.
- the walk (`nir_fs.rs`, the `StoreLocal` and `Call` arms around `:5791`):
  a value stored to a highp-declared local, passed as a highp parameter or
  returned from a highp function keeps its fp32 node (`Fs::op32`,
  `fp32_lane`); **a read of the local into a mediump operation takes the
  narrowing** (`FADD.f32 rN.h0, r^, 0x0.neg` folded into the producer when
  the producer is a single fp32 `FADD`, as in `hp_scale` and `hp_param`;
  `V2F32_TO_V2F16` for an aligned pair, as in `hp_loop`) and a read into a
  highp operation stays fp32. Mesa's rule is `mali-precision.md` Part A.3 to
  A.5 (`find_lowerable_rvalues`, `lower_variables`): this step is its first
  consumer for locals of a *fragment* shader; the vertex side and the
  `FragCoord` copy (`highp_locals_copy`) are the references to keep green.
- `highp_locals_copy` (`:814`): keep the `gl_FragCoord`-copy check as the
  case it was for; the rest of the highp locals leave it.
- a highp constant folded at fp32 and narrowed once at its mediump consumer
  (`hp_mat`'s `FADD_IMM.f32` columns; `hp_loop`'s unrolled constants):
  `nir_opt_constant_folding` at fp32, the narrowing at the edge, consistent
  with `reassociate_constant` (`glsl.rs:1456`) and the `Fmul|Fadd` constant
  arm (`nir_fs.rs:5816`).

Words to match: section 1.1 (`hp_param`, `hp_seq`, `hp_loop`). Forecast:
the three own cases compile; mixcol-ref's rewrite bound (63 of 64 once every
`highp` is `mediump`) is the ceiling. At risk: `tests/mali_deqp_mixprec.rs`
(the declared-precision pins of the mixprec references),
`tests/mali_fragcoord16.rs` (`highp_locals_copy`), `tests/mali_glmark2_corpus.rs`
(a glmark2 shader with a highp local in a mediump shader would change
verdict from refused to compiled: none is expected, the corpus's mixed
shaders are the `mixprec_*` references; check `corpus-verdicts.txt`).
`src/shaderc/online/mali/tests.rs:1218-1227` (`MixedPrecision` unit pins:
rows 1220, `mediump vec4 t = v` in a highp shader, and `:6298` must still
refuse: those are the *other* direction, a mediump local in a highp shader).

### Step 4: an fp32 expression reading a raw mediump lane or uniform (M1, 68 cases)

Reference lifts: `int_uniform` (61), `int_uniform_struct` (4, with the struct
layout of the unif2 family, section 3.2), `f2i_seq` (3).

- `nir_fs.rs:5802-5809` (the "product or sum on an fp32 value" arm): extend
  `ok` to accept a raw mediump `Kind::Input` lane (the varying loaded at
  fp32) and a mediump float uniform (`F16_TO_F32 r, u.w0.h0`, a widening
  `widen`'s `:2662` already makes for an fp16 *result*; add the FAU-block
  uniform source). The FMA then reads the lane as loaded:
  `FMA.f32 r0, r1^, r0^, 0x0.neg` (`int_uniform`), no narrowing before it.
- constants of an fp32 op are fp16 literals widened by `.h1`
  (`0x34000000.h1` for 0.25; `op32`, `:2014`): check that the FAU/immediate
  packing (`fau.rs`) emits this rather than `0x3E800000`; the reference pins
  it.
- the narrowing at the colour: each lane `FADD.f32 rN.h0, rN^, 0x0.neg`,
  then `U16_TO_U32 rN, rN^.h0` for a (lane, 0.0) word (a constant then
  `MKVEC.v2i16` for (lane, 0.5)): `lane_word` (`:9151`) and `Leaf::Zext`.
  `int_uniform`'s colour is exactly two `U16_TO_U32` words, so this step and
  step 2's `Leaf::Zext` operand overlap in `lane_word`; do step 2 first or
  land the `Zext` colour word (a root, no operand) here.
- `f2i_seq`: the `F32_TO_S32.rtz` then `S32_TO_F32` chain per lane
  (`AluOp::F2i` at `:2304` is `Alu::F2i32` for an fp16 lane or a uniform);
  a raw fp32 varying lane converted must take the same `F2i32`, and the
  `I2f` result is already an `fp32_lane` (`:2638`).

Words to match: section 1.2 `int_uniform`, `f2i_seq`, `int_uniform_struct`.
Forecast: mixcol-ref's host rewrite shows 19 of the 61 `int_uniform` cases
compile and 42 stop at a later text (9 arithmetic, 9 texture look-up, 6
fclane, 5 CK, 2 LZ, 2 intval, 2 varying load, 2 operand form, 5 others).
At risk: `tests/mali_deqp_intval.rs` (fs int uniforms, `S32_TO_F32 u`),
`tests/mali_deqp_mixprec.rs`, `tests/mali_glmark2_corpus.rs` (a shader that
reads `float(int uniform)` in a mediump fragment shader).

### Step 5: judge width on live nodes only (M2, 34 cases)

Reference lifts: `f2i_rnd`, `f2i_dead`.

- record the cause of each width flag on the node it came from (step 0's
  cause code, stored per node in a small bitset beside `Fs::live`) instead
  of a global `bool`; after `mark_live` (`:1607`), `mixed` is true only if a
  flagged node is live. The refusal at `mod.rs:1570` and the other
  `f.mixed` readers (`:1466`, `:4239`, `:4271`) take it from there.
- Mesa's folds that make the node dead before the question: `fmul x, 0.0`
  -> 0 and `float(int(x)) * 0.0` (the algebraic folds of
  `mali-algebraic-folds.md`), and the dead `int d = int(e) - int(i)`. Check
  that the walk applies them *before* the conversion node is flagged; if it
  does not, the flag must wait for `mark_live`.
- the colour words are FAU words: `MOV.i32 r0, 0x0`, `MOV.i32 r1, u2.w0`;
  `f2i_dead` has no `vs.varying` region (the link already removes an unread
  varying; `mali-deqp-varylink.md`). `NOP` replaces the empty program.

Words to match: section 1.2, `f2i_rnd` and `f2i_dead`. Forecast: 34 cases
at most; no rewrite isolates them. The change is small if step 0 shows one
site. At risk: every test that pins a refusal by `MixedPrecision`
(`src/shaderc/online/mali/tests.rs:1218-1227`, `mixprec_tests.rs`): a
refusal on a *dead* node would stop being one; none of the pinned shapes is
dead code.

## 5. Order and totals

| step | mechanism | cases | refs lifted (own) |
|--:|---|--:|---|
| 0 | the site recorded | 0 | none |
| 1 | M4 operator harness | 18 | `ck_scale`, `ck_scale_highp`, `lz_scale` |
| 2 | M4 random pairs | 59 | `ck_sum`, `ck_rnd_other`, `lz_rnd`, `lz_rnd_other`, the 6 of colourpair |
| 4 | M1 | 68 | `int_uniform`, `f2i_seq`, (`int_uniform_struct` with unif2) |
| 3 | M3 | 64 (9 alone) | `hp_param`, `hp_seq`, `hp_loop` |
| 5 | M2 | 34 | `f2i_rnd`, `f2i_dead` |

Steps 1, 2 and 4 are ordered by cases and by shared code (`Leaf::Zext` and
`lane_word` first). Step 3 is a prerequisite for 55 cases that also need
fclane steps 1 to 3, so it is worth scheduling it just before fclane's
lowering items rather than ahead of them. Step 5 is independent and
cheapest to verify; it can go first if one session is idle.
`tests/mali_deqp_mixcol.rs` has 18 `#[ignore]`d tests: the steps above lift
`ck_scale`, `ck_scale_highp`, `lz_scale`, `ck_sum`, `ck_rnd_other`,
`lz_rnd`, `lz_rnd_other` (7), `int_uniform`, `f2i_seq` (2),
`hp_param`, `hp_seq`, `hp_loop` (3), `f2i_rnd`, `f2i_dead` (2) = 14;
`hp_scale`, `hp_bool`, `hp_mat` and `int_uniform_struct` stay ignored with
the other family's text until its step lands.

## 6. Files

This note: `docs/notes/mali-mixcol-lowering.md`. The references and test it
reads: `tools/mali/testdata/deqp_mixcol_*`, `tests/mali_deqp_mixcol.rs`,
`tests/mali_deqp_colourpair.rs`. The compiler functions named above are in
`src/shaderc/online/mali/nir_fs.rs` and `mod.rs`.

## 7. Measured sites at a7260420

HOST-ONLY, a measurement of the 18 `deqp_mixcol_*` references at the tip
`a7260420` (plan #387, item mixcol-sites). A scratch test compiled each
pair through `tests/mali_ref::compile_pair` (features `host-test,gpu-mali,
shaderc-online,no-shader-table`, as `tests/mali_deqp_mixcol.rs`) with
temporary `eprintln!` markers at every `return Err(MixedPrecision |
ColourPair | ColourZeroHalf)` and at every setter of `Fs::mixed`; the
markers and the test were removed again, nothing under `src/` or `tests/`
changed. Line numbers are those of `a7260420` and drift. "Site" is where
the refusal returns; "setters" are the places that set `Fs::mixed` before
the final check at `mod.rs:1577` (the `FsPrecision::of` flag
`mixed: prec.mixed` at `nir_fs.rs:4170`, `highp_locals_copy` at `:4239`,
`fp16_operand` at `:3883`). All 18 texts are the ones of
`tests/mali_deqp_mixcol.rs` (`REFUSALS`): nothing moved since mixcol-ref.

| reference | text | site | step (design) | step measured |
|---|---|---|---|---|
| `ck_scale` | CK | `nir_fs.rs:8792` (`vectorise`) | 1 | 1 |
| `ck_scale_highp` | CK | `:8792`; setters `:3883` x2 and `:4239` already fired | 1 | **1 and 3**: step 1 alone ends at `mod.rs:1577` (inferred from the setters, the run stops at `:8792` first) |
| `ck_sum` | CK | `:8792` | 2 | 2 |
| `ck_rnd_other` | CK | `:8792` | 2 | 2 |
| `lz_scale` | LZ | `nir_fs.rs:8757` (`vectorise`) | 1 | 1 |
| `lz_rnd` | LZ | `:8757` | 2 | 2 |
| `lz_rnd_other` | LZ | `:8757` | 2 | 2 |
| `int_uniform` | MIX | `nir_fs.rs:5806` (product or sum on an fp32 value) | 4 | 4 (site confirmed) |
| `int_uniform_struct` | MIX | `:5806` | 4 | 4 (site confirmed) |
| `f2i_seq` | MIX | `:5806` | 4 | 4 (site confirmed) |
| `f2i_rnd` | MIX | `:5806`, **not** the `mod.rs:1577` flag | 5 | **4, then 5**: see 7.2 |
| `f2i_dead` | MIX | `:5806`, **not** the flag | 5 | **4, then 5**: see 7.2 |
| `hp_scale` | MIX | `mod.rs:1577` (flag); setters `:3883`, `:4239` | 3 (needs fclane 1-3) | **3** alone, plus word changes (7.1) |
| `hp_bool` | MIX | `mod.rs:1577`; setters `:3883` x4, `:4239` | 3 (needs fclane 2) | **3** alone, plus word changes, vertex words too (7.1) |
| `hp_mat` | MIX | `mod.rs:1577`; setters `:4170`, `:3883` x4, `:4239` | 3 (needs fclane 3) | **3** alone, plus word changes (7.1) |
| `hp_param` | MIX | `mod.rs:1577`; setter `:4170` only | 3 (alone) | **3** plus word changes (7.1) |
| `hp_loop` | MIX | `mod.rs:1577`; setter `:4239` only | 3 (alone, with the loop lowering) | **3** (flag relief alone, no loop lowering) |
| `hp_seq` | MIX | `mod.rs:1577`; setters `:3883` x2, `:4239` | 3 (alone) | **3** (flag relief alone) |

### 7.1 The read-not-measured prediction of section 3, settled

Section 3 predicted that after step 3 the `hp_scale`, `hp_bool` and
`hp_mat` cases stop at `gl_FragColor lane` (fclane's text). **It is false at
the tip.** To measure it, one run skipped the final `f.mixed` check at
`mod.rs:1577` (a scratch environment switch, not kept). That check runs
after the whole walk, so every walk refusal had already stood first: with it skipped, all six `hp_*` references **compile**,
none meets a `gl_FragColor lane` or any other text. Compared byte for byte
with their references in that run:

- `hp_loop`, `hp_seq`: **byte-equal** in every region. The flag relief
  alone lifts them; `hp_loop` needs no loop lowering (it unrolls).
- `hp_param`: 14 words against 16. Mesa computes `FADD.f32 r1, r0, r0` and
  narrows `FADD.f32 r1.h0, r1^.neg, 0x0.neg` at fp32; ours folds the highp
  `+in0`/`abs` chain into `FMA.v2f16`/`FADD.v2f16` words at fp16 from the
  first word. The walk lacks the fp32 value, not a refusal.
- `hp_scale`: 10 words against 9 and a different FAU: ours keeps `FADD.f32
  r0.h0, r0^, 0x0.neg` plus `FMA.v2f16 ... u2.w0` where Mesa has the negate
  folded in the narrowing (`r0^.neg`) and a `0x38000000.h11` scale.
- `hp_mat`: 13 words against 15: Mesa folds the highp `const mat2` columns
  at fp32 (`FADD_IMM.f32 #0x3ECCCCCD`, `#0x3F333333`, ...) and narrows each
  lane; ours adds the columns at fp16 (`FADD.v2f16` with FAU halves).
- `hp_bool`: differs in the **vertex** regions too (`vs.position` FAU
  words, `vs.varying` 18 words against 32: Mesa's `LEA_BUF_IMM` precedes the
  `LD_ATTR_IMM` and the store addressing differs). Cause not narrowed here:
  the vertex side of `hp_bool` is a finding for step 3's owner (it may be
  the varylink/store-order family, not mixcol).

Consequence for the plan: the 55 cases of `hp_scale`, `hp_bool`, `hp_mat`
do **not** wait for fclane steps 1 to 3 to compile; they wait for step 3's
own work (highp values kept at fp32 and narrowed at the mediump consumer,
the highp constant fold at fp32) and are byte-compared like any other.
Step 3's relief is three setters, not one: `FsPrecision::of` (`:4170`, `hp_param`,
`hp_mat`), `highp_locals_copy` (`:4239`, five of the six) and
`fp16_operand` (`:3883`, the highp varying read in a mediump shader:
`hp_scale`, `hp_bool`, `hp_mat`, `hp_seq`, `ck_scale_highp`). The design
names only the first and the second. Caveat: skipping the check is a probe,
not a lowering; a real relief that changes how these setters judge may make
the walk take other arms.

### 7.2 Step 5's mechanism is wrong for `f2i_rnd` and `f2i_dead`

Section 2.1 cause 3 and step 5 say the refusal of these two is a `f.mixed`
flag set on a node that `mark_live` or a fold later kills, reported at
`mod.rs:1570`. **Measured: neither sets `Fs::mixed`.** Both return
`MixedPrecision` from the walk at `nir_fs.rs:5806`, the same arm as
`int_uniform` and `f2i_seq`, because an operand of the product or sum (a raw
mediump varying lane or uniform converted through `int(b)`, `int(e)`) is in
none of the arm's accepted forms. The nodes are dead or multiplied by 0.0
(`f2i_dead`: `a` and `d` are never read; `f2i_rnd`: `* vec4(0, 0, 1, a)`),
so Mesa never sees the conversion, but the walk returns before any fold or
`mark_live`. So:

- step 4's `ok` extension would turn these two from a refusal into a
  compiled shader whose words are not checked (they would hold the
  conversions Mesa folded away); step 5's job is to make the arm not run
  for a node the folds kill (the fold before the width question), not to
  record a cause code for `mark_live`;
- the "record the cause on the node" part of step 5 and step 0's cause code
  are only needed for the `Fs::mixed` setters of the `hp_*` and
  `ck_scale_highp` rows, where the flag is global and every one of those
  references is live code.

### 7.3 References whose step changes

1. `ck_scale_highp`: 1 becomes 1 and 3.
2. `f2i_rnd`, `f2i_dead`: 5 becomes 4 and 5, with step 5 redefined (7.2).
3. `hp_scale`, `hp_bool`, `hp_mat`: need step 3's own word work, not fclane
   steps 1 to 3 (7.1); `hp_bool` also differs in the vertex words.
4. `hp_param`: step 3 does not lift it alone; its words differ (7.1).
5. `hp_loop`, `hp_seq`: unchanged step, but the flag relief alone byte-equals
   them, so step 3 can land with these two first.

Unchanged (9): `ck_scale`, `ck_sum`, `ck_rnd_other`, `lz_scale`, `lz_rnd`,
`lz_rnd_other`, `int_uniform`, `int_uniform_struct`, `f2i_seq`; their site
is confirmed as the read one. Changed or refined: the 9 above (items 1 to 5).

## 8. Step 5 landed: the walk builds nothing for a dead IR value

HOST-ONLY. 7.2 asks step 5 to keep the walk from judging a node that
Mesa's folds and `nir_opt_dce` kill, before any width or form question.
`nir_fs.rs` `ir_live` does the dead-code half of that. Before the walk, it
marks the IR instructions that the output stores, the discards and the
`if` conditions read, to a fixed point through values and locals
(`mod.rs` `mark_reads`, the closure `live_varying_loads` uses for the link).
`walk_nodes` skips every unmarked instruction, so a dead value sets no
flag, meets no refusal and builds no node. The liveness is
flow-insensitive: a store that a later one overwrites stays live, so the
walk keeps at least what Mesa keeps. The zero-product fold (`x * 0.0`)
is not part of it; that is still `f2i_rnd`'s step.

What moved, at the tip it was made on (the `tests/mali_*.rs` binaries and
`--lib shaderc::online::mali`, features `host-test,gpu-mali,shaderc-online,
no-shader-table`, ignored tests included; nothing that passed before fails):

| reference | before | now |
|---|---|---|
| `deqp_mixcol_f2i_seq` | MIX | byte-equal |
| `deqp_mixcol_f2i_dead` | MIX | compiles, one FAU half `-0.0` where Mesa has `0.0` (`80003c00` against `00003c00`) |
| `deqp_gaps_mixprec_fs_mixed_other` | MIX | byte-equal |
| `deqp_ctrlflow_fs_int_cond` | `ControlFlow` | byte-equal (the `if`'s sides store nothing live) |
| `deqp_intval3_rsg_sconv55_fs` | `IntValue` | byte-equal |
| `deqp_negzero_mainplain_fs` | `Arithmetic` | byte-equal |
| `deqp_cfold2_random_mixed_rcp16` | `ConstantFold` | compiles; the vertex program keeps a varying Mesa's link removes |
| `deqp_intrnd_dead_bool_cmp_fs`, `deqp_misc2_algebraic_random13` | `IntValue` | `Defect(LaneCycle)` (`mali-deqp-defects.md` fix C) |
| `deqp_intrnd_dead_cond_fs` | `IntValue` | `FsOperandForm` |
| `deqp_intrnd_dead_ivec_uniform_fs`, `deqp_misc2_algebraic_random46`, `deqp_intval3_opfs_sequence_bool_vec2_mediump` | `IntValue` | `gl_FragColor` lane (fclane) |
| `deqp_intval3_rsg_swizzle12_fs` | `IntValue` | `FsUniform` |

The femtovg `filter_image` and `filter_image_nodiscard` programs build 71
and 81 fewer NIR nodes (1082 and 1070). Their BIR, FAU and immediate counts
and their words are unchanged: `mark_live` already dropped those nodes
after the walk. `f2i_rnd` stays MIX: its conversions are read through
`* vec4(0, 0, 1, a)`, a product that is zero in some lanes only, which
neither the liveness nor the link's `zero_product` folds.
