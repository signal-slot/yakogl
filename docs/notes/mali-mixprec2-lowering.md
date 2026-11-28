<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): what is left of the mixed-precision refusal, and how to lower it (HOST-ONLY design)

Plan #387, item mixprec2-design. This is a design note. Nothing under
`src/` changes. It adds ten Mesa 26.2.1 references
(`tools/mali/testdata/deqp_mixprec2_*`), pinned refused by
`tests/mali_deqp_mixprec2.rs`, and this note.

Read first: `mali-mixcol-lowering.md` (its step 3, "MX3", is the
declared-highp half of this family; its sections 7 and 8 hold the measured
sites at a7260420 and the `ir_live` change), `mali-mixed-precision.md`
("Where Mesa inserts the conversions"), `mali-precision.md` (Mesa's width
rule) and `mali-deqp-mixprec.md`, `mali-deqp-mixprec3.md`,
`mali-deqp-mixprec4.md` (the earlier pins).

## 1. The refusal and its sites at 8133663c

The text is `MaliError::MixedPrecision`
(`src/shaderc/online/mali/mod.rs:440`):

> Valhall: a fragment shader that mixes highp and mediump or lowp float
> values (Mesa's fp32 and fp16) is not lowered yet

The tip is `8133663ca5da6042860053ff2c68484fdcaf7c9a`. All line numbers
below are from it. The refusal has two kinds of site:

* **The flag.** `mod.rs:1748` (`if f.mixed { return Err(MixedPrecision) }`)
  runs after the whole fragment walk. Thirteen places set `Fs::mixed` (or
  `FsPrecision::mixed`, which is copied into it at `nir_fs.rs:4904`).
  The residue reaches these first:
  * `mod.rs:1207`: a `const` or local float declared at the other
    precision;
  * `mod.rs:1232`: a parameter declared at the other precision;
  * `nir_fs.rs:4576` and `:4581` (`fp16_operand`): a highp varying's lane,
    or a value of its fp32 island, read by an fp16 operation;
  * `nir_fs.rs:4981`: `highp_locals_copy` found a highp local that is not a
    `gl_FragCoord` copy;
  * the others, `:3283`, `:3690`, `:5014`, `:6590`, `:6748`, `:6757` and
    `glsl.rs:1622`, are reached first by no residue case. Nor is
    `mod.rs:1665` (a float uniform of a highp shader that is not highp).
* **Walk returns.** `nir_fs.rs:7074` is the "product or sum on an fp32
  value" arm (`ir::Op::Alu(Fmul | Fadd)` with an fp32 operand). It returns
  when an operand is in none of the forms `ok` accepts: an fp32 lane, a
  32-bit select, an fp16 value, a constant, or a raw mediump varying lane.
  The other walk returns are `mod.rs:1595`, `nir_fs.rs:4633`, `:6851`
  and `:13040`. No residue case reaches them.

### How the sites were measured

* **Case list.** The Verdin iMX95 console logs of the whole-tree run
  (`tools/cts/log/2026-11-2{4,5}-imx95-gles2-*-e63ab28e/console/*.log`,
  Mali-G310) hold the text on **175** lines. For **157** cases it is the
  first refusal, and all 157 are in `tools/cts/gles2-mustpass.txt`. This is
  the "175 G310 lines" of the plan. (The plan names the run "main
  a3d8d2de". The images of these logs say `e63ab28e`.)
* **Host measure.** `tools/mali/deqp-host-measure.sh --cases <the 157>` was
  run at 8133663c (HOST-ONLY). It used deqp-gles2 from
  `$CONNECTED_DEPS_DIR/VK-GL-CTS-host` (`tools/cts/deps-env.sh --build
  HOST_DEQP_CTS`). All 157 cases began. Of their first refusals:

| first refusal on the host at 8133663c | cases |
|---|--:|
| **MixedPrecision (this family)** | **65** |
| none (the pair compiles; HOST-ONLY, no pixels judged) | 33 |
| `gl_FragColor` lane (fclane) | 22 |
| arithmetic | 13 |
| texture look-up / texture coordinate | 9 |
| fragment operand form | 4 |
| fragment varying load | 3 |
| integer or boolean value | 3 |
| constant product | 2 |
| `Defect(LaneCycle)`, texture lanes, link offsets | 3 |

  The 92 that moved were lifted after e63ab28e (fclane-lower-1's
  MX1/MX4/A/B/D, the `ir_live` change of mixcol section 8, the vs16
  steps). They are not this family's.
* **Sites.** A scratch worktree of 8133663c in `$TMPDIR` (not this
  branch) printed a marker at every setter and return above. A scratch
  test compiled the 65 pairs through `tests/mali_ref::compile_pair`, the
  sources verbatim from the measure's `.qpa`. The markers were never in
  this branch.
* **The train.** The same 65 pairs compiled at intval-rest's result
  commit `224cc3f7`: all 65 still refuse with this text. The train does not
  reach this family, and the pins of `tests/mali_deqp_mixprec2.rs` hold in
  either merge order.

### The 65 cases, by site

| first setter or return | cases | dEQP-GLES2 groups (`functional.shaders.`) |
|---|--:|---|
| return `nir_fs.rs:7074`; the operand is a **mediump float uniform** (`Kind::Uniform`, 16 bits) | 26 | `random.texture.fragment` (8), `random.scalar_conversion.{fragment,combined}` (5), `random.trigonometric.fragment` (4), `random.conditionals.{fragment,combined}` (4), `random.swizzle.fragment` (2), `random.comparison_ops.fragment` (1), `random.exponential.fragment` (1), `struct.local.nested_struct_array_fragment` (1) |
| return `:7074`; the operand is a 32-bit `Fneg` or `F2f32` | 3 | `random.texture.fragment.{140,145,5}` |
| return `:7074`; the operand is a highp varying lane (`Input`, 32 bits, other-precision bit set) | 1 | `operator.sequence.side_effects.highp_float_int_fragment` |
| flag; setter `nir_fs.rs:4576` (`fp16_operand`, highp varying lane) | 21 | `operator.binary_operator.{equal,greater,greater_or_equal,less,less_or_equal}.highp_*` (7), `operator.float_compare.*.highp_vec{2,4}` (10), `operator.exponential.sqrt.highp_{float,vec3,vec4}` (3), `operator.common_functions.fract.highp_float` (1) |
| flag; setter `mod.rs:1207` (a `const highp mat`) | 7 | `matrix.{add,sub}.const_highp_mat{2,3,4}_mat{2,3,4}`, `matrix.mul.const_highp_mat2_mat2` |
| flag; setter `mod.rs:1232` (a highp parameter) | 3 | `functions.qualifiers.{in,inout,out}_highp_float_fragment` |
| flag; setter `nir_fs.rs:4981` (`highp_locals_copy`) | 2 | `operator.sequence.no_side_effects.highp_{float_int,vec4_ivec4_bvec4}` |
| flag; setter `nir_fs.rs:4581` (`hp32_float`) | 2 | `operator.geometric.length.highp_{float,vec2}` |

The 30 `:7074` returns (rows 1 to 3) are 28 `random.*` cases, the
struct case and `seq_side`. Every case is must-pass. All are fragment
cases (the text is fragment-only).

## 2. The residue against the existing pins

These already pin some of the 65 shapes. They belong to other items and
are not edited here:

| pinned by | shape | covers here |
|---|---|---|
| `mali_deqp_mixprec4.rs` | `gt_vec2`, `eq_vec4`, `le_vec2`, `ge_vec4`, `bin_ge_float`, `bin_eq_vec4` | the `float_compare` / `binary_operator` highp compares (the setter-4576 row) |
| `mali_deqp_mixprec3.rs` | `hp_bvec4` (`lessThan` vec4) | same |
| `mali_deqp_mixcol.rs` | `hp_bool` (`equal` vec2), `hp_param` (`in highp float`), `hp_seq` (`no_side_effects.highp_float_int`), `int_uniform_struct` | the setter-1232, -4981 rows; the `F16_TO_F32` uniform widening |

The new references pin the shapes these do not draw:

| reference (`deqp_mixprec2_`) | dEQP case (verbatim from the `.qpa`) | site | Mesa `fs.fragment` words |
|---|---|---|--:|
| `struct_nested` | `struct.local.nested_struct_array_fragment` | `:7074`, mediump uniform | 22 |
| `rnd_sconv36` | `random.scalar_conversion.fragment.36` | `:7074`, mediump uniform, **in an overwritten store** | 9 |
| `rnd_swz4` | `random.swizzle.fragment.4` | `:7074`, mediump uniform, in an overwritten store | 12 |
| `rnd_tex140` | `random.texture.fragment.140` | `:7074`, 32-bit `Fneg`, in an overwritten store | 10 |
| `seq_side` | `operator.sequence.side_effects.highp_float_int_fragment` | `:7074`, highp varying lane | 15 |
| `hp_sqrt_vec3` | `operator.exponential.sqrt.highp_vec3_fragment` | flag, setter `:4576` | 18 |
| `hp_fract` | `operator.common_functions.fract.highp_float_fragment` | flag, setter `:4576` | 12 |
| `hp_length_vec2` | `operator.geometric.length.highp_vec2_fragment` | flag, setter `:4581` | 14 |
| `hp_mat2_add` | `matrix.add.const_highp_mat2_mat2_fragment` | flag, setter `mod.rs:1207` | 7 |
| `hp_out_param` | `functions.qualifiers.out_highp_float_fragment` | flag, setter `mod.rs:1232` | 13 |

All ten were drawn with `tools/mali/deqp_ref.sh --instrument shim` (G610
drm-shim, `--uniform-values`). `tools/mali/g310_check.sh
'deqp_mixprec2_*'` says **10 identical, 0 differ**: G310v2 and G610 give
the same words.

## 3. Where Mesa converts (read from the ten references)

* **A highp value is computed at fp32 and narrowed once, where a mediump
  value reads it.** The highp varying is loaded at fp32
  (`LD_VAR_BUF_IMM.f32 ... src_f32`), and so is every mediump varying.
  The highp operation stays fp32:
  * `hp_fract`: `FROUND.f32.rtn`, then `FADD.f32 r0, r0^, r1^.neg`;
  * `hp_sqrt_vec3`: per lane `FRSQ.f32` then `FRCP.f32`;
  * `hp_length_vec2`: `FMA.f32` for the sum of squares, then `FRSQ.f32`
    and `FRCP.f32`.

  The narrowing goes where the mediump `color` takes the value:
  `FADD.f32 rN.h0, rN^, 0x0.neg`, one per lane. Then `U16_TO_U32` builds a
  (lane, 0.0) word, or `MKVEC.v2i16` pairs two lanes. The colour
  arithmetic (`color * 0.1 + 0.5`) is `FMA.v2f16` on the pair word.
* **A mediump float uniform read by an fp32 operation is widened from its
  fp16 FAU half.** In `struct_nested`, `float(s[0].c) + uf_four - uf_three`
  is `S32_TO_F32 r4, u2.w0` (the int uniform, a 32-bit FAU word),
  `F16_TO_F32 r1, u3.w0.h0` and `F16_TO_F32 r3, u2.w1.h0`, then
  `FADD.f32 r1, r4^, r1^` and `FADD.f32 r1, r1^, r3^.neg`. The result is
  narrowed by `FADD.f32 r1.h0, r1^, 0x0.neg`. This is `lower_precision`'s
  `f162f` at the edge of the lowered tree: the int conversion is a
  32-bit `CANT_LOWER` operand, so the sum it feeds stays fp32.
* **An int to float to int chain stays 32-bit.** In `seq_side` it is
  `F32_TO_S32.rtz`, `IADD_IMM.i32 #0x1`, `S32_TO_F32`, `FADD.f32` with the
  raw highp lane, `F32_TO_S32.rtz`, `S32_TO_F32`, then the one narrowing.
* **A highp constant expression is folded at fp32.** `hp_mat2_add` is a
  constant colour: `MOV.i32 r0, u2.w0`, `MOV.i32 r1, u2.w1`, with no
  varying and no arithmetic. Each lane is folded at fp32 and rounded to
  fp16 once (FAU half `0x2e66` for `res[0][1] + res[1][1]`).
* **The highp parameter of `hp_out_param` folds away.** `a = -1.0` reaches
  `f` as a constant. Everything left is fp16 (`FADD.v2f16`, `FMA.v2f16`,
  `FCMP_OR.v2f16`), the same shape as mixcol's `hp_param`.
* **An overwritten output store leaves no words.** In `rnd_sconv36`,
  `rnd_swz4` and `rnd_tex140`, every `gl_FragColor = ...` that a later
  unconditional store replaces is gone. Mesa's NIR writes the output
  once (the variables are lowered to SSA before codegen), so only the
  last store's operands reach the back end.
  `rnd_tex140` is only `V2F32_TO_V2F16` twice plus two `MKVEC.v2i16`, the
  swizzle of `a.rbag`. `rnd_sconv36` is one `FMA.f32` of the two raw mediump
  varying lanes (`float(d) * a`, the last store's `w` lane) and one
  narrowing; its other lanes are constants. In `rnd_sconv36`'s colour pair (0.0, x), the zero half is
  the inline `0x0.h0` (`MKVEC.v2i16 r1, 0x0.h0, r0^.h0`), not a FAU half.

## 4. Lowering steps, with a host forecast

All counts are HOST-ONLY first-refusal moves among the 65 cases above
(all must-pass), measured with the scratch probes of section 1. They are
never board results. "Off the text" means the case no longer refuses with
`MixedPrecision` first. Whether it then passes needs the words to match
and a board run.

### M2-1: drop overwritten output stores before the walk (19 cases)

`ir_live` (`nir_fs.rs:4874`, mixcol section 8) is flow-insensitive: a
store that a later store overwrites stays live, so its operands are judged.
Mesa removes such a store. The step: in `main`'s top-level straight-line
code, a `gl_FragColor` store is dead when a later unconditional store
writes every lane it wrote, and no statement in between reads
`gl_FragColor`. A store inside an `if`, or one read by a later store (for
example `gl_FragColor = gl_FragColor * x`), is kept. Mark the dead store
not live, so the walk never sees its operands.

* Probe: the same rule applied to the 28 `random.*` sources by text.
  **19 of the 28** leave the text: 6 compile
  (`random.comparison_ops.fragment.36`, `exponential.fragment.91`,
  `scalar_conversion.fragment.36`, `texture.fragment.36`,
  `texture.fragment.47`, `trigonometric.fragment.91`), and 13 meet other
  texts. Those 13 are fclane 3, mediump fragment uniform 2, texture
  look-up 3, texture coordinate 1, arithmetic 2, operand form 1, constant
  product 1. The 9 left on the text (8 with a uniform operand, 1 `Fneg`) are in `if` arms or are live reads
  (M2-2, M2-3).
* Words: the stripped `rnd_sconv36` compiles to 9 words, 7 of them Mesa's.
  Ours has `MKVEC.v2i16 r1, u2.w0.h0, r0^.h0` and `MOV.i32 r0, u2.w1`.
  Mesa has the zero half inline, `MKVEC.v2i16 r1, 0x0.h0, r0^.h0`, and
  `MOV.i32 r0, u2.w0`. So the step also needs a zero constant half of a
  `MKVEC` to be the inline `0x0`, not a FAU half. That shifts the FAU
  layout by one word.
* References: `rnd_sconv36` (byte-equal target), `rnd_swz4` and
  `rnd_tex140`. These two then meet operand form and fclane: their
  `#[ignore]` stays, and the refusal text in `REFUSED` changes.
* Groups: `random.texture.fragment`, `random.scalar_conversion.*`,
  `random.trigonometric.fragment`, `random.swizzle.fragment`,
  `random.comparison_ops.fragment`, `random.exponential.fragment`.
* At risk: `tests/mali_deqp_mixcol.rs` (`f2i_rnd`'s zero product is not
  this) and `tests/mali_fragment_random.rs`.
  `tests/mali_glmark2_corpus.rs` is safe: no glmark2 shader stores
  `gl_FragColor` twice.

### M2-2: a mediump float uniform as an operand of an fp32 sum or product (9 cases)

At `nir_fs.rs:7074`, `ok` should accept a `Kind::Uniform` of 16 bits
(the fp16 FAU block) and widen it: `F16_TO_F32 r, u.wN.hK`, with the half
chosen by the uniform's place in the word. Mesa emits it at each read,
not hoisted (`struct_nested` reads `u3.w0.h0` and `u2.w1.h0`). The
`widen` helper already makes `F16_TO_F32` from a register half for an fp16
result. This step adds the FAU-half source.

* Cases: the 9 that are still on the text after M2-1 with a uniform
  operand: `random.conditionals.{fragment.1,84,97, combined.1}`,
  `random.texture.fragment.{27,116,137}`, `random.trigonometric.fragment.43`
  and `struct.local.nested_struct_array_fragment`. mixcol's
  `int_uniform_struct` has the same widening.
* References: `struct_nested`, and mixcol's `int_uniform_struct`.
* Forecast: 9 leave the text. How many compile is not measured. The
  random ones carry `if` arms (ctrlflow) and texture look-ups.

### M2-3: the other fp32 operands at `:7074` (2 cases)

* `seq_side`: a highp varying lane (the other-precision bit set) read
  raw by an fp32 sum: `FADD.f32 r0, r0^, r1^` on the load's own word, as
  `raw` already does for a mediump lane. The step widens `raw` to a highp
  lane of a shader with no default precision. Reference: `seq_side`.
* `random.texture.fragment.145`: a 32-bit `Fneg` operand. Its own
  `fneg` folds into the consumer's source modifier (`.neg`). No reference
  is drawn. Draw it from this case's `.qpa` when the step starts.
* (`random.texture.fragment.5`'s `F2f32` operand leaves the text under
  M2-1.)

Forecast: 2 leave the text.

### M2-4: declared highp values in a mediump shader, at fp32 (35 cases), mixcol MX3

This is mixcol step 3 ("MX3", **no item** in
`mali-lowering-schedule.md`). The probe of mixcol section 7.1 was
repeated at 8133663c: a scratch environment switch skipped the
`mod.rs:1748` check. **All 35 flag cases compile.** Compared with the
references:

| reference | with the flag skipped |
|---|---|
| `hp_out_param` | **byte-equal** |
| `hp_mat2_add` | differs in one FAU half only: ours `0x2e68`, Mesa `0x2e66` (we fold the lanes at fp16, Mesa at fp32 and rounds once) |
| `hp_fract` | 12 against 12 words: ours narrows at the load (`FADD.f32 r0.h0`) and does `FROUND.v2f16`/`FADD.v2f16`; Mesa does `FROUND.f32`/`FADD.f32` and narrows after |
| `hp_sqrt_vec3` | 20 against 18: ours `V2F32_TO_V2F16` at the load and `FRSQ.f16`/`FRCP.f16`; Mesa `.f32` per lane |
| `hp_length_vec2` | 15 against 14: ours narrows first and computes in `FMA.v2f16`/`FRSQ.f16`; Mesa `FMA.f32`, `FRSQ.f32` |

So the flag is all that refuses. The work is the width of the walk, not a
new refusal. Sub-steps, one session each:

* **M2-4a, the setters that are only declarations (5 cases, 1 already
  byte-equal).** Stop setting the flag at `mod.rs:1232` (a highp
  parameter whose value folds or is fp16 in Mesa's program) and at
  `nir_fs.rs:4981` (`highp_locals_copy`), where the walk already gives
  Mesa's words. `hp_out_param` becomes byte-equal. mixcol's `hp_param` and
  `hp_seq` are the checks: section 7.1 found `hp_seq` byte-equal and
  `hp_param` differing. Groups: `functions.qualifiers.*_highp_float_fragment`
  and `operator.sequence.no_side_effects.highp_*`. Forecast: 5 off the
  text, and at least `out_highp_float` byte-equal.
* **M2-4b, fold highp constants at fp32 (7 cases).** For `mod.rs:1207`,
  fold a `const highp` expression at fp32 and round it to fp16 once, at
  the mediump consumer (`hp_mat2_add`'s `0x2e66`). The helper is
  `mesa_fold.rs`. Then clear the flag. Groups: `matrix.{add,sub,mul}.const_highp_*`.
  Forecast: 7 off the text. `hp_mat2_add` is byte-equal if only the
  rounding differs.
* **M2-4c, the highp varying island at fp32 (23 cases).** Setters
  `nir_fs.rs:4576` and `:4581`. A highp varying's lane, and every
  operation on it until a mediump variable takes the value, stay fp32:
  `Fs::op32`, `fp32_lane`, and the fp32 transcendental kernels of arith
  step 1. The narrowing goes at the mediump consumer, `FADD.f32 rN.h0,
  r^, 0x0.neg`, folded into a single fp32 `FADD` producer where Mesa does
  that. The colour words are `U16_TO_U32` and `MKVEC.v2i16` as now.
  * References: `hp_fract`, `hp_sqrt_vec3`, `hp_length_vec2`, plus the
    compares of `mixprec4`, `mixprec3`'s `hp_bvec4` and mixcol's
    `hp_bool`, `hp_scale`, `hp_mat`.
  * Groups: `operator.binary_operator.*.highp_*_fragment`,
    `operator.float_compare.*.highp_vec{2,4}_fragment`,
    `operator.exponential.sqrt.highp_*`,
    `operator.common_functions.fract.highp_float`,
    `operator.geometric.length.highp_{float,vec2}`.
  * Forecast: 23 off the text (all compile in the probe; the words
    differ until the island is fp32).
  * At risk: `tests/mali_fragcoord16.rs` (`highp_locals_copy`),
    `mixprec_tests.rs` and `tests.rs:1218-1230`. Those pins are the
    *other* direction, a mediump value in a highp shader, and must still
    refuse.

### Order and totals

| step | mechanism | cases off the text | references |
|---|---|--:|---|
| M2-1 | overwritten output stores dead, zero half inline | 19 (6 compile) | `rnd_sconv36`, `rnd_swz4`, `rnd_tex140` |
| M2-4a | declaration-only setters cleared | 5 | `hp_out_param` (+ mixcol `hp_param`, `hp_seq`) |
| M2-4b | highp constants folded at fp32 | 7 | `hp_mat2_add` |
| M2-4c | highp varying island at fp32 | 23 | `hp_fract`, `hp_sqrt_vec3`, `hp_length_vec2` (+ mixprec3/4, mixcol `hp_*`) |
| M2-2 | fp16 uniform widened into an fp32 op | 9 | `struct_nested` (+ mixcol `int_uniform_struct`) |
| M2-3 | raw highp lane, fp32 `fneg` operand | 2 | `seq_side` |
| | **total** | **65** | |

M2-1 goes first: it is cheapest and lifts the most. It is a liveness
change, the same kind as mixcol section 8, and it is the only step with a
measured compile count. M2-4a can land at once as well. M2-4c is the
largest piece of work. It shares `op32` and the transcendental kernels
with arith step 1 (train intval-rest), so it should start after the
train lands. M2-2 and M2-3 are small arms at `:7074` and can be one
session. Every step edits `src/shaderc/online/mali/**`, which the train
holds now: none can start before it lands.

## 5. Reproducing

```text
# the test (every reference loads, every refusal pinned)
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixprec2
# a reference (MESA_SHIM_WORK defaults to $CONNECTED_DEPS_DIR/mesa-shim)
tools/mali/deqp_ref.sh <case>.vert <case>.frag tools/mali/testdata/deqp_mixprec2_<shape> --instrument shim
# G310 against G610
tools/mali/g310_check.sh --scratch "$TMPDIR/g310" 'deqp_mixprec2_*'
# the host measure of the 157 cases (case list from the iMX95 console logs:
# the first "refused this" line after each "Test case" line)
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS-host" --cases <list> --out "$TMPDIR/hm"
```

## 6. Landed (plan #387, item `mali-train-3c`)

HOST-ONLY: byte-compares against the Mesa references, no board run.

### M2-1

`mod.rs` `fragment_live` is the fragment liveness of both the walk
(`nir_fs.rs` `ir_live`) and the link (`live_varying_loads`). Two kinds of
store are no store and read nothing:

* an output store of `main`'s own statement list that a later store of
  that list to the same output replaces, with no `return` between
  (`overwritten_outputs`; `rnd_swz4`, `rnd_tex140`);
* a store to a local that a later store of the same statement list
  overwrites whole before a live read (`nir::overwritten_stores`, the
  vertex walk's rule). A shader that reads `gl_FragColor` writes it
  through a local, so `rnd_sconv36`'s stores are of this kind. The
  liveness and the overwritten set are solved together: `gl_FragColor =
  gl_FragColor` falls first, then the store it read.

The second kind is not limited to `gl_FragColor`: dEQP's `highp float res
= float(0.0); res = ...` drops its first store too (`highp_locals_copy`
skips a store the walk did not see).

`rnd_sconv36`'s differing words were not a FAU-half rule (section 4's
guess). Lane z of `vec4(-0.75, int(e), -0.75, float(d)) * vec4(c, b, 0, a)`
is a constant times a constant zero beside a computed lane: the GLSL
compiler folds nothing, and `nir_opt_algebraic`'s `fmul(a, 0.0)` runs
before `nir_opt_constant_folding`, so the lane is `+0.0` (ours was the
product's `-0.0`, a FAU half). The fp32 arm of the walk gives that lane
the rule's 0.0.

| reference | before | after |
|---|---|---|
| `deqp_mixprec2_rnd_sconv36` | MixedPrecision | **byte-equal** |
| `deqp_mixprec2_rnd_swz4` | MixedPrecision | refused: fragment operand form |
| `deqp_mixprec2_rnd_tex140` | MixedPrecision | refused: `gl_FragColor` lane |
| `deqp_constprod_constchain_fs` | constant product | **byte-equal** |
| `deqp_mixcol_hp_seq` | MixedPrecision | **byte-equal** |
| `deqp_texmisc_tk_rnd_bias_proj` | texture look-up kind | **byte-equal** |
| `deqp_varyload_tex110` | fragment varying load | **byte-equal** |
| `deqp_cfold2_random_mixed_rcp16` | differs (`vs.varying`) | **byte-equal** |
| `deqp_arith_random_frcp_fs_mediump` | texture look-up kind | refused: arithmetic |
| `deqp_arith_random_fsin_fs_mediump` | arithmetic | refused: `gl_FragColor` lane |
| `deqp_intrnd_trunc_atan_fs` | integer or boolean value | refused: constant fold |
| `deqp_intval3_rsg_exp76_fs` | integer or boolean value | refused: arithmetic |
| `deqp_fsint2_rnd_trig93` | integer or boolean value | refused: arithmetic |
| `deqp_texmisc_tk_rnd_bias_two` | texture look-up kind | refused: `gl_FragColor` lane |

The other 1,390 Mali host tests are unchanged.
