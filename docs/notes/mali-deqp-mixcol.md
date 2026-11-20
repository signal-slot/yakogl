<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the mixprec and colourpair refusals left at the tip, dEQP-GLES2 shapes and Mesa references (HOST-ONLY)

Plan #387, item mixcol-ref. This note is a study with references. It makes no
compiler change. Read `docs/notes/mali-deqp-mixprec.md` and
`docs/notes/mali-deqp-colourpair.md` first: they hold the shapes these
families had on the board and what was lowered since. This note covers three
fragment-shader refusals of `src/shaderc/online/mali/mod.rs`:

| text | key | board, 37305608 (first refusal) | tip `bee6896d` (first refusal, host) |
|---|---|---:|---:|
| `a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16)` (`MixedPrecision`) | MIX | 192 Fail | **166** (164 Fail, 2 NS) |
| `a colour pair of a constant then a varying lane` (`ColourPair`) | CK | 142 Fail | **61** Fail |
| `a colour pair of a varying lane and 0.0 (Mesa's U16_TO_U32)` (`ColourZeroHalf`) | LZ | 29 Fail | **16** Fail |

Fail and NS are the board's status of the case in the 37305608 slices.

**Every count here is HOST-ONLY.** The method:

- `tools/mali/deqp-host-measure.sh` runs `deqp-gles2` on the host against
  the Mali back end's library at the tip `bee6896d`, on VK-GL-CTS `93bca01`
  with `cts-yakogl-target.patch`.
- The case list is all 10,597 cases of
  `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/refusals.tsv` whose
  text is a compile refusal. The whole list is run, so a case that moved
  into these texts from another text is counted.
- A case's first refusal is read from its `.qpa`.
- Nothing says whether a case passes on the board.

The measure took 10,597 cases. 6,196 have a host refusal.

## 1. Where the board's cases went, and where the tip's came from

The board's cases of each text, at the tip:

| board text | cases | at the tip |
|---|---:|---|
| MIX | 192 | 75 compile with no refusal (44 host Pass, 31 host Fail, the black read-back). 56 `a texture coordinate other than a vec2 varying read whole or two constants`. 29 `a gl_FragColor lane other than the varying's own lane or a constant`. 19 `a vertex shader texture look-up other than 2D or cube ...`. 7 `a mediump fragment shader uniform other than ...`. 2 `a fragment shader operand of a form ...`. **4 still MIX.** |
| CK | 142 | 137 compile. 1 `a vertex shader whose attribute numbering waits for a fold ...`. **4 still CK.** |
| LZ | 29 | 27 compile. **2 still LZ.** |

The `fs_uniform_compare_harness` M1 shape of the mixprec note, the loop
rotations and the operator colour-pair harness are gone, which is the
lowering of plans #377 and #387 so far. **The tip's 243 cases are nearly all
new to these texts.** The compiler now gets past an earlier refusal, so each
case reaches its next one:

| tip text | cases | board text of those cases |
|---|---:|---|
| MIX | 166 | 94 `an integer or boolean value`, 38 `a constant operation on or to a value that is not a finite normal float`, 13 `gl_FragColor lane`, 7 `a constant product ...`, 5 `arithmetic other than ...`, 4 `a ... uniform that is not highp` / `a mediump fragment shader uniform other than ...`, 1 `more than 32 local variables`, **4 MIX** |
| CK | 61 | 29 constop, 12 constant product, 8 intval, 6 fclane, 1 arithmetic, 1 `two varying lanes of equal value`, **4 CK** |
| LZ | 16 | 6 constop, 6 fclane, 2 intval, **2 LZ** |

All 243 refusals are in the fragment shader. In every case the partner
vertex shader compiles: the measure's "next refusal" column is `-` for all
of them.

Out of scope here, but in the mixprec family of
`mali-deqp-families.md` section 9: `a fragment varying load or conversion of
a form no reference draws` has **43** cases at the tip (board 59; 3 of the
board's are still there, 40 came in). It has no reference in this item.

## 2. The shapes (HOST-ONLY, 243 cases)

A shape is the construct that makes the compiler refuse. Section 3 of
`mali-deqp-families.md` says how a shape is named. The shapes here were
found by a scratch probe. It compiled each case's pair from the `.qpa` with
the compiler at the tip (`mali_ref::compile_pair`), then compiled two
mechanical rewrites of the fragment shader. The probe is not committed.

- **`nohighp`**: every `highp` turned into `mediump`. 63 of the 71
  non-random MIX cases then compile, and one more moves to `gl_FragColor
  lane`. That makes the declared highp the cause for 64 cases.
- **`intunif`**: each `uniform mediump int x;` made `const int x = 3;`. 61
  of the 95 random MIX cases leave the text.
- **The rest**: the 34 random cases the rewrite does not move all convert a
  mediump float input to int (`int(v)`, `ivec4(.., v, ..)`).

The colour-pair shapes are read from the source. A shape uses the last
`gl_FragColor` assignment and the operation the pair is an operand of. The
`colourpair_rnd_fs_*` cases are the ones `mali-deqp-colourpair.md` already
references. The example is the shape's first case in mustpass order.

### 2.1 MIX (166)

| shape | cases (Fail/NS) | dEQP groups | example | reference |
|---|---|---|---|---|
| `int_uniform`: a `uniform mediump int` (or ivec) read as a float (`vec4(e, d, c, b) * a`) | 61 (61/0) | `shaders.random.texture` 22, `.scalar_conversion` 15, `.trigonometric` 7, `.swizzle` 6, `.conditionals` 5, `.comparison_ops` 3, `.exponential` 2, `fragment_ops.interaction.basic_shader` 1 | `fragment_ops.interaction.basic_shader.73` | `int_uniform` (`random.texture.fragment.50`) |
| `f2i_rnd`: a mediump float varying or uniform converted to int in a random shader (`int(b)`, `ivec4(s, q, r, q)`) | 34 (34/0) | `shaders.random.texture` 12, `.scalar_conversion` 9, `.conditionals` 4, `.swizzle` 3, `.comparison_ops` 2, `.exponential` 2, `.all_features` 1, `.trigonometric` 1 | `shaders.random.all_features.fragment.11` | `f2i_rnd` (`random.scalar_conversion.fragment.97`), `f2i_dead` (`random.scalar_conversion.fragment.27`) |
| `hp_scale`: a highp float result written into the mediump `color` of the operator harness, then `color * k + b` | 23 (23/0) | `shaders.operator.geometric` 8, `.unary_operator` 7, `.exponential` 4, `.angle_and_trigonometry` 2, `.common_functions` 2 | `shaders.operator.angle_and_trigonometry.degrees.highp_float_fragment` | `hp_scale` (`operator.unary_operator.minus.highp_float_fragment`) |
| `hp_bool`: a highp compare (`equal(in0, in1)`, `in0 == in1`) written into the mediump `color` as `vec2(res)` | 17 (17/0) | `shaders.operator.float_compare` 10, `.binary_operator` 7 | `shaders.operator.binary_operator.equal.highp_float_fragment` | `hp_bool` (`operator.float_compare.equal.highp_vec2_fragment`) |
| `hp_mat`: a highp matrix operation (`const highp mat2 in0`, `in0 + v_coords`) read into `gl_FragColor` | 15 (15/0) | `shaders.matrix.mul` 8, `.add` 4, `.sub` 3 | `shaders.matrix.add.const_highp_mat2_float_fragment` | `hp_mat` (the example) |
| `hp_param`: a `highp` function parameter in a mediump shader-library shader | 4 (4/0) | `shaders.functions.qualifiers` 3, `.datatypes` 1 | `shaders.functions.datatypes.struct_constructor_highp_in_fragment` | `hp_param` (`functions.qualifiers.in_highp_float_fragment`) |
| `int_uniform_struct`: an int uniform stored in a struct (a struct uniform, or `uniform int` into a local struct) then read as a float | 4 (4/0) | `shaders.linkage.uniform_struct_partial_{int_float,ivec2_ivec2,vec2_int}`, `shaders.struct.local.nested_struct_array_fragment` | `shaders.linkage.uniform_struct_partial_int_float` | `int_uniform_struct` (`struct.local.nested_struct_array_fragment`; `gl_ref.py` cannot set a struct uniform) |
| `hp_loop`: a highp float loop body into a mediump colour | 3 (1/2) | `shaders.loops.{do_while,for,while}_constant_iterations.basic_highp_float_fragment` | `shaders.loops.do_while_constant_iterations.basic_highp_float_fragment` | `hp_loop` (`loops.for_constant_iterations.basic_highp_float_fragment`) |
| `f2i_seq`: the sequence operator's `ivec4(v_in1.wzyx)` and `vec4(res)` | 3 (3/0) | `shaders.operator.sequence.no_side_effects.{highp,mediump,lowp}_vec4_ivec4_bvec4_fragment` | `...highp_vec4_ivec4_bvec4_fragment` | `f2i_seq` (`...mediump_vec4_ivec4_bvec4_fragment`) |
| `hp_seq`: a highp int sequence (`int(v_in1.x)`, `float(res)`) | 2 (2/0) | `shaders.operator.sequence.{no_side_effects,side_effects}.highp_float_int_fragment` | `...no_side_effects.highp_float_int_fragment` | `hp_seq` (the example) |

### 2.2 CK (61) and LZ (16)

Every one of them is the **operand form** that `mali-deqp-colourpair.md`
("What stays refused") left open. The pair is not a word of the colour. It
is an operand of a sum, product or multiply-add, and Mesa builds the word
before the operation.

| shape | text | cases | dEQP groups | example | reference |
|---|---|---|---|---|---|
| `ck_rnd_product`: `vec4(<constant>, <lane>, ..) * x` | CK | 20 | `shaders.random.basic_expression` 10, `.scalar_conversion` 5, `.conditionals` 2, `.comparison_ops` 1, `.swizzle` 1, `fragment_ops.interaction.basic_shader` 1 | `fragment_ops.interaction.basic_shader.97` | existing `colourpair_rnd_fs_ck_product` (+1 case, its own) |
| `ck_rnd_sum`: `x + vec4(<constant>, <lane>, ..)` with no product | CK | 17 | `shaders.random.basic_expression` 5, `.comparison_ops` 3, `.scalar_conversion` 3, `.conditionals` 2, `.swizzle` 2, `.texture` 1, `fragment_ops.interaction.basic_shader` 1 | `fragment_ops.interaction.basic_shader.15` | `ck_sum` (`random.comparison_ops.fragment.20`) |
| `ck_rnd_madd`: the pair in a product and a sum | CK | 9 | `shaders.random.basic_expression` 5, `.trigonometric` 2, `.comparison_ops` 1, `.conditionals` 1 | `shaders.random.basic_expression.combined.10` | existing `colourpair_rnd_fs_ck_madd` (+2 cases, its own) |
| `ck_scale`: the operator harness `color.yz = res; color = color * 0.5 + 0.5` where `res` is a copy of the varying (`+in0`, `in0++`, `in0--`) | CK | 9 | `shaders.operator.unary_operator.{plus,post_increment_result,post_decrement_result}.{highp,mediump,lowp}_vec2_fragment` | `...plus.highp_vec2_fragment` | `ck_scale` (mediump), `ck_scale_highp` |
| `ck_rnd_other`: the pair reaches the colour through earlier statements (`gl_FragColor = gl_FragColor * vec4(0.5, -0.0, b, a)`, a dead `if`) | CK | 2 | `shaders.random.basic_expression.fragment.96`, `fragment_ops.interaction.basic_shader.35` | `fragment_ops.interaction.basic_shader.35` | `ck_rnd_other` (`random.basic_expression.fragment.96`) |
| `colourpair_rnd_fs_zk_tree` | CK | 1 | `shaders.random.conditionals.fragment.73` | | existing `colourpair_rnd_fs_zk_tree` |
| `lz_scale`: the same harness with a float `res` in `color.x` | LZ | 9 | `shaders.operator.unary_operator.{plus,post_increment_result,post_decrement_result}.{highp,mediump,lowp}_float_fragment` | `...plus.highp_float_fragment` | `lz_scale` (mediump) |
| `lz_rnd`: `(lane, 0.0)` in a random product or sum | LZ | 3 | `shaders.random.conditionals.{combined,fragment}.63`, `.fragment.55` | `shaders.random.conditionals.combined.63` | `lz_rnd` (`random.conditionals.fragment.63`) |
| `lz_rnd_other`: built across statements (`gl_FragColor - e + vec4(d, c, float(b), a)`) | LZ | 2 | `shaders.random.basic_expression.{combined,fragment}.45` | `...combined.45` | `lz_rnd_other` (`...fragment.45`) |
| `colourpair_rnd_fs_uniform_tz_ck` | LZ | 2 | `shaders.random.basic_expression.{combined,fragment}.68` | | existing `colourpair_rnd_fs_uniform_tz_ck` |

The existing references of `mali-deqp-colourpair.md` pin 6 cases at the
tip: `rnd_fs_ck_product`, `rnd_fs_ck_madd` (2), `rnd_fs_zk_tree` and
`rnd_fs_uniform_tz_ck` (2). `tests/mali_deqp_colourpair.rs` already pins
their refusals. The shapes `ck_rnd_product` and `ck_rnd_madd` have the same
form as those references, so they have no new one.

## 3. The references

There are 18 references,
`tools/mali/testdata/deqp_mixcol_<shape>.{vert,frag,expect.json}`.

- **Source.** Each `.vert` and `.frag` is the case's dEQP text verbatim from
  the measure's `.qpa`. A `.license` sidecar names the case.
- **Draw.** Each was drawn with `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_mixcol_<shape> --instrument shim`: Mesa 26.2.1
  panfrost on drm-shim, GPU `g610`, `--uniform-values`, 64x64,
  `pan_ref_diff.py extract --draw`.
- **Build.** The shim is `drm_shim_ref.sh build` of the 26.2.1 tarball,
  whose sha256 matches `third_party/mesa-26.2.1/UPSTREAM`, in
  `$CONNECTED_DEPS_DIR/mesa-shim`.
- **Self-check.** The same build reproduces
  `deqp_mixprec_cmpv4.expect.json` byte for byte.
- **Determinism.** Two draws of each of the first 13 references are
  identical.
- **G310.** The words are the same as the G610's
  (`docs/notes/mali-g310-vs-g610.md`).

`tests/mali_deqp_mixcol.rs` pins every reference. Its test is `#[ignore]`d
with the refusal, and `refusals_pinned` asserts the exact `MaliError`
message of each reference and that every reference on disk is listed. The
compiler at `bee6896d` refuses all 18 with their family's text.

What Mesa emits: the fragment program, its instruction count, and what it
pins.

| reference | `fs.fragment` | what Mesa does |
|---|---|---|
| `hp_scale` | 9 | `FADD.f32.discard r0.h0, r0^.neg, 0x0.neg`: the highp negation is fp32, and its narrowing to the mediump colour is folded into the same instruction (`.h0` destination). Then `U16_TO_U32 r0, r0^.h0` makes the (lane, 0.0) word, and **one** `FMA.v2f16 r0, r0^, 0.5.h11, 0.5.h11` scales it. The (0.5, 1.0) word is a FAU `MOV`: the constant lanes of `color * 0.5 + 0.5` are folded. |
| `hp_bool` | 14 | `FCMP_OR.f32.eq.m1` per lane at fp32, then `MUX.v2i16 r, 0x0, 0x3C000000.h11, r.h00` (the `b2f16` of the compare as a 16-bit select). The colour words are `MKVEC.v2i16 r1, 0x0.h0, r1^.h0` (constant first) and `MKVEC.v2i16 r2, r0^.h0, u2.w0.h0`. |
| `hp_mat` | 15 | Four `FADD_IMM.f32` with the matrix constants and one `FADD.f32` at fp32. Each colour lane is narrowed by `FADD.f32 rN.h0, rN^, 0x0.neg` and paired by `MKVEC.v2i16`; the 1.0 is a FAU half. |
| `hp_param` | 16 | The highp parameter's `-a` and `2.0 * a` are fp32 (`FADD.f32 r1, r0, r0`, then `FADD.f32 r1.h0, r1^.neg`, which narrows at the return). Everything after the call is fp16: `FADD.v2f16`, `FMA.v2f16`, `FCMP_OR.v2f16`. |
| `hp_loop` | 11 | The loop is unrolled and folded. The highp lanes are narrowed by `V2F32_TO_V2F16` and one `FADD.f32 .h0`, then two `MKVEC.v2i16`. |
| `hp_seq` | 12 | `F32_TO_S32.rtz`, `IADD.s32`, `S32_TO_F32`, all 32-bit. One `FADD.f32 .h0` narrows, then `U16_TO_U32` and `FMA.v2f16` by 0.5. |
| `int_uniform` | 12 | `S32_TO_F32 r1, u2.w0` of the int uniform, then the product with the **mediump** varying lane is `FMA.f32` (fp32). Each lane is narrowed by `FADD.f32 .h0`, then `U16_TO_U32`: Mesa computes a mediump expression that reads an int-to-float result at fp32. |
| `int_uniform_struct` | 22 | `S32_TO_F32` of the int uniform. The mediump float uniforms it is added to are widened with `F16_TO_F32 r, u.w0.h0`: the sum is fp32 and narrowed by `FADD.f32 .h0`. The other lanes are fp16 (`FMA.v2f16`, `FADD.v2f16`). This is a real fp32/fp16 mix in one Mesa program. |
| `f2i_seq` | 25 | `F32_TO_S32.rtz` then `S32_TO_F32` per lane, 32-bit. Each lane is narrowed by `FADD.f32 .h0` and paired by `MKVEC.v2i16`, then `FMA.v2f16` by 0.5. ATEST reads `r0.h1`. |
| `f2i_rnd` | 7 | No conversion survives: the colour is folded to constants (`MOV.i32 r0, 0x0`, `MOV.i32 r1, u2.w0`), and no varying is read. |
| `f2i_dead` | 7 | The conversion is dead code (`int d = int(e) ..` and a global `a` never read). The colour is two FAU words, and there is **no `vs.varying` region** (the link removed the varying). |
| `ck_scale` / `ck_scale_highp` | 14 / 11 | One v4 load, the two lanes narrowed by `FADD.f32 .h0`, `MKVEC.v2i16 r1, 0x0.h0, r1^.h0` (constant **first**, inline zero), then `FMA.v2f16 r1, r1^, 0.5.h11, 0.5.h11` **on the word**. The second word is `MKVEC.v2i16 r0, r0^.h0, u2.w0.h0` (lane, FAU 1.0), then the same FMA. The highp variant differs only in register order and the final `MOV`s. |
| `lz_scale` | 9 | `U16_TO_U32 r0, r0^.h0` of the narrowed lane, then `FMA.v2f16` by 0.5, 0.5. The (0.5, 1.0) word is a FAU `MOV`. |
| `ck_sum` | 10 | `MKVEC.v2i16 r0, u2.w0.h0, r0^.h0` (FAU constant first) then `FADD_IMM.v2f16 r0, r0^, #0x4800BE00`: the constant addend is an immediate pair. The other word is a FAU `MOV`. |
| `ck_rnd_other` | 15 | `MKVEC.v2i16 r2, u2.w0.h0, r0.h0`, then `FMA.v2f16` by a FAU word. The second word is (lane, `.h1` lane), then `FMA.v2f16`. The `-0.0` lane is folded into the FAU word. |
| `lz_rnd` | 12 | `FMA.v2f16 r0, 0.5.h11, r0^.h00, -0` (the product at fp16 on a duplicated half), then two `U16_TO_U32` words. |
| `lz_rnd_other` | 13 | `V2F32_TO_V2F16`, `U16_TO_U32 r1, r0.h0`, then `FADD_IMM.v2f16` of the folded constant vector. `MKVEC.v2i16 r0, r0^.h1, u2.w0.h0`, then `FADD_IMM.v2f16`. |

The pattern is the same in all of them:

- **Precision.** Mesa computes each highp or int-derived value at fp32 and
  narrows it once, where a mediump value reads it. The narrowing is folded
  into the producer as `FADD.f32 rN.h0, x, -0`, or a `V2F32_TO_V2F16` pair
  when two adjacent lanes narrow together.
- **Integers.** Mediump ints stay 32-bit (`S32_TO_F32`, `F32_TO_S32.rtz`).
  An fp16 operand that meets an fp32 result is widened by `F16_TO_F32`.
- **Colour pairs.** A colour pair that is an operand is built **before** the
  operation: `MKVEC.v2i16` with the constant as src0, or `U16_TO_U32` for
  (lane, 0.0). The operation then runs once at `v2f16` on the word. Constant
  lanes of the result are folded into a FAU or immediate word.

## 4. Proposed lowering and forecast (HOST-ONLY)

Each step below byte-compares against the references above. A step removes
the `#[ignore]` and the `REFUSALS` entry of each reference it lifts.

1. **`hp_*`, the declared highp narrowed where it meets mediump** (64
   cases). Today the fragment walk refuses the meeting point (the
   `MixedPrecision` sites of `mali-deqp-mixprec.md` section 1, the "product
   or sum on an fp32 value of a mediump shader" arm). The step:
   - keep a highp subtree at fp32;
   - emit the fold `FADD.f32 .h0` (or `V2F32_TO_V2F16` for an aligned pair)
     at the edge where it is stored into a mediump local, a mediump
     parameter's return or `gl_FragColor`;
   - turn a highp bool into a float with Mesa's `MUX.v2i16` (`hp_bool`).

   **Forecast:** with every `highp` rewritten to `mediump`, 63 of the 64
   compile now and one moves to `gl_FragColor lane`. So up to 63 cases
   compile once the narrowing matches Mesa. The rewrite changes precision,
   so the forecast is an upper bound.
2. **`int_uniform*` and `f2i_*`, int conversions at 32 bits** (65 + 37
   cases):
   - `S32_TO_F32` and `F32_TO_S32.rtz` stay 32-bit;
   - their fp32 results flow into fp32 arithmetic, widening fp16 operands
     with `F16_TO_F32`, then narrow once at the colour;
   - dead and folded conversions (`f2i_dead`, `f2i_rnd`) must be removed
     before the precision check. Today the compiler refuses a mix that Mesa
     never emits.

   **Forecast:** with the int uniform made a constant, 19 of the 61
   `int_uniform` cases compile. The other 42 stop at a later text: 9
   `arithmetic other than ...`, 9 texture look-up, 6 `gl_FragColor lane`,
   5 CK, 2 LZ, 2 intval, 2 varying load, 2 operand form, 5 others. The
   `f2i_*` and `int_uniform_struct` cases have no forecast: no rewrite
   isolates them. 102 cases at most.
3. **Colour pair as an operand** (CK 61, LZ 16). Extend `vectorise` to
   build the `MKVEC.v2i16 <const>.h0, lane.h0` or `U16_TO_U32` word when
   the pair is an **operand**, then run the operation once on the word.
   `ck_scale`, `lz_scale` and `ck_sum` show the forms; the existing
   `rnd_fs_ck_product` and `rnd_fs_ck_madd` show the product and
   multiply-add forms. Fold the constant lanes of the result into a FAU or
   immediate word. That is the step `mali-deqp-colourpair.md` "For the item
   that takes the rest" names.

   **Forecast:** at most 77 cases. No rewrite isolates them, so their next
   refusal is unknown. Of the 18 operator cases (`ck_scale`, `lz_scale`),
   6 of 9 per text were `gl_FragColor lane` cases on the board, so expect
   fclane work to meet these.

Upper bound of all three: 243 must-pass cases leave these texts. Lowering
them does not by itself make a case pass on the board.

## 5. Issue comment (for the planner to post; agents do not post on issues)

> mixcol-ref (plan #387): host measure at bee6896d (HOST-ONLY,
> tools/mali/deqp-host-measure.sh over the 37305608 slices, VK-GL-CTS
> 93bca01). The mixprec text `a fragment shader that mixes highp and mediump
> or lowp` is the first refusal of 166 cases (board: 192), the colour-pair
> texts of 61 + 16 (board: 142 + 29). Only 4 + 4 + 2 of these are the
> board's own cases: the board's cases have moved on (75 + 137 + 27 compile,
> the rest to texture-coordinate, fclane and other texts). The tip's cases
> came in from intval (94 + 10), constop (38 + 35) and the others as those
> were lowered. Shapes: MIX `int_uniform` 61, `f2i` 37, `hp_*` 64, and
> `int_uniform_struct` 4. CK/LZ are all the pair-as-operand form: random
> sums and products 48 + 5, the operator harness's `color * 0.5 + 0.5` 18,
> and 6 already referenced. There are 18 new Mesa 26.2.1 references
> `tools/mali/testdata/deqp_mixcol_*`, pinned refused in
> tests/mali_deqp_mixcol.rs. Note: docs/notes/mali-deqp-mixcol.md (commit
> <sha>). Forecast (host rewrites): hp narrowing up to 63, int uniform 19,
> 243 at most for the three steps.
