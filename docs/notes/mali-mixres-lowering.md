<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the MixedPrecision residue at 349f989f, case by case, and its steps (HOST-ONLY design)

Plan #387, item mixres-refs. This is a design note: nothing under `src/`
changes. It adds 23 Mesa 26.2.1 references
(`tools/mali/testdata/deqp_mixres_*`), pinned by `tests/mali_deqp_mixres.rs`,
and this note. It does not edit the earlier notes. Read
`mali-mixprec2-lowering.md` first: its steps M2-1 to M2-4c are the
skeleton here. `mali-mixcol-lowering.md` steps 3 and 5 are MX3 and MX5.

## 1. The measure

* **Tip.** `349f989f5392bea65c20e7663289e86481e521e0`, the integration tip
  of this item. Its `capi/Cargo.toml` declares the feature `vivante-st`
  twice (lines 41 and 85, both from `110080eb`). Cargo then refuses to load
  the workspace, so no cargo command runs at this tip. For the measure and
  the probes, line 41 was removed in the working tree and never committed.
  So `commit` in the measure's output says `349f989f-dirty`. The duplicate
  is a feature table entry and does not change the compiler.
* **CTS.** VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` (the
  commit of `tools/cts/deps.toml`). The tree was exported from the local
  `VK-GL-CTS-gles3` checkout at that commit, then
  `external/fetch_sources.py` was run and
  `tools/cts/cts-yakogl-target.patch` was applied. The script records
  `cts=adaf01cd`: that is the commit of that export, not a CTS commit.
* **Command** (4 cores):

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$PWD/build/measure" --jobs 3 --chunk 100
```

* **Result.** All 10,597 cases of the slices `2026-11-15-t6-gles2-S{1..4}-37305608`
  began. 106 chunk processes exited 1 (dEQP's "not every case passed"),
  and none crashed. 4,709 cases meet a refusal. **89 have `MixedPrecision`
  as their first refusal** (`measure.tsv`'s `host_refusal`). All 89 are
  fragment refusals and all are must-pass: 83 Fail and 6 NotSupported by the
  board's status at `37305608`. By group: `shaders.operator` 41,
  `shaders.random` 29, `shaders.matrix` 7, `shaders.loops` 6,
  `shaders.functions` 3, `fragment_ops.interaction` 2, `shaders.struct` 1.
  This is the same count as `mali-deqp-families.md` 19.1 at `e9033663`
  (89: operator 41, random 29, matrix 7).

### 1.1 How each case's site was found

The method of `mali-mixprec2-lowering.md` section 1:

1. A scratch copy of the tip in `$TMPDIR` (never this branch) got a marker
   at each `Fs::mixed` / `FsPrecision::mixed` setter that turns the flag
   on, and at each `return Err(MixedPrecision)`. The setters are
   `mod.rs:1209` (a declaration), `:1234` (a parameter),
   `nir_fs.rs:3466`, `:3873`, `:4759` (`fp16_operand`, a highp varying
   lane), `:4764` (`hp32_float`), `:5178` (`highp_locals_copy`), `:5211`,
   `:6801`, `:6959` and `:6968`. The returns are `mod.rs:1597`, `:1750`
   (the flag check), `nir_fs.rs:4821`, `:7062`, `:7285` (the "product or
   sum on an fp32 value" arm) and `:13563`.
2. A scratch test compiled the 89 pairs (`build/measure/src/<case>.vert|.frag`,
   verbatim from the `.qpa`) through `tests/mali_ref::compile_pair`. A
   walk return names the site. Otherwise the site is the first setter.
3. At `:7285` a second marker printed the operand that `ok` rejects.
4. **The flag alone.** The same copy, with the `mod.rs:1750` check
   skipped by an environment switch. **All 56 flag cases compile.**
5. **Dead output stores.** A text rewrite blanked every top-level
   `gl_FragColor = ...;` of `main` that comes before the last store that
   does not read `gl_FragColor`, when nothing else in between reads it.
   This is Mesa's result: the variables become SSA and only the last value
   reaches the output. The rewritten pairs were compiled at the tip and at
   81b3ce19.
6. **The unlanded results.** Plan #402's `lower-mixprec3`
   (`81b3ce1937caaeb46633326367e34bb4a469de73`) carries the highp-island
   compares and `overwritten_stores`, the M2-1 mechanism. Its tree,
   unmodified, compiled the same 89 pairs and the 23 new references.
   So did `lower-vsconst` (`2c0c659632fea5cffe35f036fe964a897c3b8065`,
   which folds a mediump shader's highp constant trees at fp32). The 23
   references were also compiled at `lower-fsint` (`e8327e42`),
   `lower-train-2` (`13259474`), `lower-vsloop` (`8180095c`) and
   `lower-vsloop` of plan #402 (`cbb6b45b`). `lower-train-2` and the two
   `lower-vsloop` results give the tip's outcome for all 23. `lower-fsint` refuses the six
   MR2 references with `Arithmetic`, but its base (`2b1cd2eb`) predates
   train 2's arith step 1b (`521a1624`). That is its base's text, not its
   change, so the test does not accept it.

Sites at the tip:

| site | what | cases |
|---|---|--:|
| `nir_fs.rs:7285` return | a mediump float uniform (`Uniform`, 16 bits) as an operand of an fp32 sum or product | 29 |
| `nir_fs.rs:7285` return | a 32-bit `Fneg` operand | 2 |
| `nir_fs.rs:7285` return | a 32-bit `F2f32` operand | 1 |
| `nir_fs.rs:7285` return | a highp varying lane (`Input`, 32 bits) | 1 |
| flag, setter `nir_fs.rs:4759` | a highp varying lane read by an fp16 operation | 34 |
| flag, setter `nir_fs.rs:5178` | `highp_locals_copy`: a highp local that is not a `gl_FragCoord` copy | 9 |
| flag, setter `mod.rs:1209` | a `const highp mat` | 7 |
| flag, setter `mod.rs:1234` | a highp parameter | 3 |
| flag, setter `nir_fs.rs:4764` | a value of a highp varying's fp32 island | 3 |
| | **total** | **89** |

## 2. Every case in a step

The step names are those of `mali-mixprec2-lowering.md` section 4
(M2-*), `mali-mixcol-lowering.md` step 3 (MX3) and new ones (MR*).
"81b3ce19" is the first refusal at `lower-mixprec3`'s result. HOST-ONLY:
"compiles" judges no pixels.

### 2.1 Per-step table

| step | mechanism (one line) | cases | dEQP groups | first refusal at 81b3ce19 | references |
|---|---|--:|---|---|---|
| M2-1 | an output store a later unconditional store overwrites is dead, including one inside a bare `{ }` block | 18 | `random.{texture,trigonometric,scalar_conversion,swizzle,exponential}.*` 16, `fragment_ops.interaction.basic_shader` 2 | off the text for all 18: compiles 4, fclane 4, `FsUniform` 3, `TexKind` 2, `Arithmetic` 2, `FsOperandForm` 2, `TexCoord` 1 | mixprec2 `rnd_sconv36`, `rnd_swz4`, `rnd_tex140`; **`rnd_block_store`** |
| MR1 | **new**: a store is also dead when only dead stores read it, as in the chain `gl_FragColor = x * gl_FragColor; gl_FragColor = gl_FragColor; gl_FragColor = y;` (M2-1's rule keeps it, because a later statement reads `gl_FragColor`) | 4 | `random.comparison_ops.fragment.36`, `random.scalar_conversion.{fragment,combined}.36`, `random.texture.fragment.45` | `MixedPrecision` for all 4. With the chain removed by text: compiles 2, `ArithConstant` 1, `TexKind` 1 | mixprec2 `rnd_sconv36` (it is this chain) |
| M2-2 | a live mediump float uniform as an operand of an fp32 sum or product, widened `F16_TO_F32 r, u.wN.hK` at each read | 9 | `random.conditionals.{fragment.1,84,97,combined.1}`, `random.texture.fragment.{27,116,137}`, `random.trigonometric.fragment.43`, `struct.local.nested_struct_array_fragment` | `MixedPrecision` for all 9 | mixprec2 `struct_nested`, mixcol `int_uniform_struct`; **`rnd_cond1`** |
| M2-3 | a raw highp varying lane, or a 32-bit `fneg` folded into the consumer's `.neg`, as an fp32 operand | 2 | `operator.sequence.side_effects.highp_float_int`, `random.texture.fragment.145` | `MixedPrecision` for both | mixprec2 `seq_side`, t3gap `mixprec2_rnd_tex145` |
| M2-4a | clear the flag of a highp parameter (`mod.rs:1234`) and of `highp_locals_copy` (`:5178`) for the sequence locals | 6 | `functions.qualifiers.{in,inout,out}_highp_float`, `operator.sequence.no_side_effects.highp_{float_int,vec4,vec4_ivec4_bvec4}` | `MixedPrecision` for all 6 | mixprec2 `hp_out_param`, mixcol `hp_param`, `hp_seq`; **`hp_inout_param`**, **`hp_seq_vec4`**, **`hp_seq_bvec4`** |
| M2-4b | fold a `const highp` matrix expression at fp32 and round it to fp16 once, then clear the flag of `mod.rs:1209` | 7 | `matrix.{add,sub}.const_highp_mat{2,3,4}_mat{2,3,4}`, `matrix.mul.const_highp_mat2_mat2` | `MixedPrecision` for all 7; at `lower-vsconst` 2c0c6596 the 6 `add`/`sub` cases compile, `mul` stays | mixprec2 `hp_mat2_add`; **`hp_mat3_add`**, **`hp_mat4_sub`**, **`hp_mat2_mul`** |
| M2-4c (compares) | the highp varying island at fp32 through a compare: `FCMP_OR.f32` and the `MUX.v2i16` b2f | 23 | `operator.binary_operator.{equal,greater,greater_or_equal,less,less_or_equal}.highp_*` 8, `operator.float_compare.*.highp_vec{2,3,4}` 15 | **compiles, all 23** | mixprec4 (6), mixprec3 `hp_bvec4`, mixcol `hp_bool`; **`hp_lt_vec3`**, **`hp_bin_eq_vec3`** (both byte-equal at 81b3ce19) |
| M2-4c (rest) | the island at fp32 through `fract`, `sqrt`, `length` and `distance`, narrowed once at the mediump consumer | 8 | `operator.common_functions.fract.highp_{float,vec3}`, `operator.exponential.sqrt.highp_{float,vec3,vec4}`, `operator.geometric.length.highp_{float,vec2}`, `operator.geometric.distance.highp_float` | `MixedPrecision` for all 8 | mixprec2 `hp_fract`, `hp_sqrt_vec3`, `hp_length_vec2`; **`hp_fract_vec3`**, **`hp_sqrt_float`**, **`hp_sqrt_vec4`**, **`hp_distance`** |
| MR2 | **new**: the island at fp32 through the fp32 transcendental kernels (`sin`, `cos`, `tan`, `log`, `log2`, `pow`). None of them is among mixprec2's 65 G310 cases; that they reached this text when an earlier refusal lifted (likely `Arithmetic`, arith step 1b) is assumed, not measured. Four are byte-equal once the flag is clear | 6 | `operator.angle_and_trigonometry.{sin,cos,tan}.highp_float`, `operator.exponential.{log,log2,pow}.highp_float` | `MixedPrecision` for all 6 | **`hp_sin`**, **`hp_cos`**, **`hp_tan`**, **`hp_log`**, **`hp_log2`**, **`hp_pow`** |
| MX3 | a highp float loop accumulator in a mediump shader (`highp_locals_copy`, `:5178`) with a uniform or a dynamic trip count. The constant-count shape, mixcol `hp_loop`, compiles at the tip | 6 | `loops.{for,while,do_while}_{uniform,dynamic}_iterations.basic_highp_float` (all 6 NotSupported on the board) | `MixedPrecision` for all 6 | mixcol `hp_loop`; **`hp_loop_for_uniform`**, **`hp_loop_while_dynamic`**, **`hp_loop_do_while_uniform`** |
| MX5 | width judged on live nodes only (the zero product `* vec4(0, 0, 1, a)`) | **0** | its references' cases (`random.scalar_conversion.fragment.97` and `.27`) compile at the tip; no case of the 89 has the shape | - | mixcol `f2i_rnd`, `f2i_dead` |
| | **total** | **89** | | 48 still `MixedPrecision`, 27 compile, 14 at other texts | |

### 2.2 The cases of each step

* **M2-1 (18).** `fragment_ops.interaction.basic_shader.{1,64}`,
  `random.exponential.fragment.91`, `random.scalar_conversion.{fragment,combined}.{45,86}`,
  `random.swizzle.fragment.{4,43}`, `random.texture.fragment.{5,18,36,47,61,140}`,
  `random.trigonometric.fragment.{6,42,91}`. The operand at `:7285` is a
  mediump uniform in 16 of them, `Fneg` in `texture.140` and `F2f32` in
  `texture.5`. At the tip with the dead stores removed by text, 17 of them
  meet the same first refusal as at 81b3ce19. The 18th,
  `basic_shader.1`, has its overwriting store inside a bare block, which
  the text rewrite does not reach; 81b3ce19 does reach it. So 81b3ce19's
  `overwritten_stores` is the whole of M2-1 for these cases.
* **MR1 (4).** Listed in the table. Each has `gl_FragColor = gl_FragColor;`
  between two dead stores.
* **M2-2 (9).** The same 9 as `mali-mixprec2-lowering.md` M2-2. With the
  dead stores removed they still refuse at `:7285` with a mediump uniform
  operand. `texture.137` has two dead stores and one live uniform operand.
* **M2-3 (2), M2-4a (6), M2-4b (7), MX3 (6).** Listed in the table.
* **M2-4c compares (23).** `binary_operator.{equal}.highp_{float,vec2,vec3,vec4}`,
  `binary_operator.{greater,greater_or_equal,less,less_or_equal}.highp_float`,
  and `float_compare.{equal,greaterThan,greaterThanEqual,lessThan,lessThanEqual}.highp_vec{2,3,4}`.
* **M2-4c rest (8), MR2 (6).** Listed in the table.

## 3. The 23 new references (Mesa 26.2.1)

Each was drawn with `tools/mali/deqp_ref.sh <pair> tools/mali/testdata/deqp_mixres_<name>
--instrument shim`: the G610 drm-shim built by `tools/mali/drm_shim_ref.sh build`
from the tarball of `third_party/mesa-26.2.1/UPSTREAM` (sha256 checked), with
`LLVM_DIR=/usr/lib/llvm-21`. The shim build first redrew
`deqp_mixprec2_hp_fract` and `deqp_mixcol_hp_loop` byte for byte. Then
`tools/mali/g310_check.sh 'deqp_mixres_*'` reported **23 identical, 0
differ**: G310v2 and G610 give the same words. Every pair is verbatim from
`build/measure/src/` (the `.qpa`). Each `.vert` and `.frag` has its
`.license` naming the case.

"Flag skipped" is the scratch copy of the tip with the `mod.rs:1750` check
off (section 1.1, item 4). It shows how far each step is from Mesa's words
once the refusal goes.

| reference (`deqp_mixres_`) | dEQP case (`functional.`) | step | Mesa `fs.fragment` words | at the tip | at 81b3ce19 | flag skipped at the tip |
|---|---|---|--:|---|---|---|
| `rnd_block_store` | `fragment_ops.interaction.basic_shader.1` | M2-1 | 8 | MIX | `FsOperandForm` | MIX (the `:7285` return) |
| `rnd_cond1` | `shaders.random.conditionals.fragment.1` | M2-2 | 14 | MIX | MIX | MIX (the `:7285` return) |
| `hp_inout_param` | `shaders.functions.qualifiers.inout_highp_float_fragment` | M2-4a | 13 | MIX | MIX | **byte-equal** |
| `hp_seq_vec4` | `shaders.operator.sequence.no_side_effects.highp_vec4_fragment` | M2-4a | 21 | MIX | MIX | 22 words against 21 |
| `hp_seq_bvec4` | `shaders.operator.sequence.no_side_effects.highp_vec4_ivec4_bvec4_fragment` | M2-4a | 25 | MIX | MIX | **byte-equal** |
| `hp_mat3_add` | `shaders.matrix.add.const_highp_mat3_mat3_fragment` | M2-4b | 7 | MIX | MIX (**byte-equal** at 2c0c6596) | one FAU word: ours `3f9a40cc`, Mesa `3f9a40cd` (folded at fp16, not at fp32) |
| `hp_mat4_sub` | `shaders.matrix.sub.const_highp_mat4_mat4_fragment` | M2-4b | 7 | MIX | MIX (**byte-equal** at 2c0c6596) | **byte-equal** |
| `hp_mat2_mul` | `shaders.matrix.mul.const_highp_mat2_mat2_fragment` | M2-4b | 7 | MIX | MIX | 15 words against 7: the product is not folded (Mesa: two `MOV.i32` of FAU words) |
| `hp_lt_vec3` | `shaders.operator.float_compare.lessThan.highp_vec3_fragment` | M2-4c | 16 | MIX | **byte-equal** | 17 words against 16 |
| `hp_bin_eq_vec3` | `shaders.operator.binary_operator.equal.highp_vec3_fragment` | M2-4c | 13 | MIX | **byte-equal** | 18 words against 13 |
| `hp_fract_vec3` | `shaders.operator.common_functions.fract.highp_vec3_fragment` | M2-4c | 18 | MIX | MIX | 20 words against 18 |
| `hp_sqrt_float` | `shaders.operator.exponential.sqrt.highp_float_fragment` | M2-4c | 12 | MIX | MIX | 12 words against 12, other words |
| `hp_sqrt_vec4` | `shaders.operator.exponential.sqrt.highp_vec4_fragment` | M2-4c | 24 | MIX | MIX | 23 words against 24 |
| `hp_distance` | `shaders.operator.geometric.distance.highp_float_fragment` | M2-4c | 10 | MIX | MIX | the vertex program differs: `vs.varying` 21 words ours, 5 Mesa's; one `vs.position` FAU word (`00000010` against `00000000`) |
| `hp_sin` | `shaders.operator.angle_and_trigonometry.sin.highp_float_fragment` | MR2 | 18 | MIX | MIX | **byte-equal** |
| `hp_cos` | `shaders.operator.angle_and_trigonometry.cos.highp_float_fragment` | MR2 | 18 | MIX | MIX | **byte-equal** |
| `hp_tan` | `shaders.operator.angle_and_trigonometry.tan.highp_float_fragment` | MR2 | 23 | MIX | MIX | 24 words against 23 |
| `hp_log` | `shaders.operator.exponential.log.highp_float_fragment` | MR2 | 16 | MIX | MIX | 16 words against 16, other words |
| `hp_log2` | `shaders.operator.exponential.log2.highp_float_fragment` | MR2 | 13 | MIX | MIX | **byte-equal** |
| `hp_pow` | `shaders.operator.exponential.pow.highp_float_fragment` | MR2 | 16 | MIX | MIX | **byte-equal** |
| `hp_loop_for_uniform` | `shaders.loops.for_uniform_iterations.basic_highp_float_fragment` | MX3 | 17 | MIX | MIX | 23 words against 17 |
| `hp_loop_while_dynamic` | `shaders.loops.while_dynamic_iterations.basic_highp_float_fragment` | MX3 | 20 | MIX | MIX | 27 words against 20 |
| `hp_loop_do_while_uniform` | `shaders.loops.do_while_uniform_iterations.basic_highp_float_fragment` | MX3 | 18 | MIX | MIX | 24 words against 18 |

What Mesa does in the new shapes:

* **MR2.** The highp varying is loaded at fp32. The kernel is fp32:
  `FSIN_TABLE.u6`/`FCOS_TABLE.u6` and `FMA_RSCALE.f32` for sin, cos and
  tan (tan adds `FRCP.f32` and a product), `FREXPE.f32.log`,
  `FLOGD.f32` and `FMA.f32` for log. One narrowing, `FADD.f32 r0.h0, r0^,
  0x0.neg`, comes before the mediump colour `FMA.v2f16`. The walk already
  builds this for sin, cos, log2 and pow (byte-equal with the flag
  skipped). So for those the step is only to stop `fp16_operand`
  (`nir_fs.rs:4759`) setting the flag when the reader is an fp32 kernel.
  tan and log need their kernel's word order.
* **MX3.** The counter is an fp32 value (`IADD_IMM.i32 r2, 0x0,
  #0x3D4CCCCD` for 0.05, `FADD.f32 r2, r2^, u2.w0`), and the compare is
  `FCMP_OR.f32.ge` against the uniform count. The colour's varying lanes
  are narrowed once before the loop (`V2F32_TO_V2F16`), and the loop body
  only swaps halves (`MKVEC.v2i16`). Ours is 6 or 7 words longer.
* **M2-4b.** All three are constant colours of two FAU words. Mesa folds
  `const highp` at fp32 and rounds to fp16 once: `hp_mat3_add`'s
  `3f9a40cd` against our `3f9a40cc`. `hp_mat2_mul` needs the matrix
  product folded.
* **M2-1, bare block.** `rnd_block_store`'s first store is overwritten by
  a store inside `{ ... }`, which is unconditional. Mesa keeps only the
  last value, so the fp32 `vec4(l, k, j, i) * vec4(-2, g, -2, -0.25)` of
  the mediump uniform `l` and the int uniform `k` leaves no words.

## 4. Order

Counts are the HOST-ONLY cases that leave `MixedPrecision` first. None of
them is a board result.

1. **M2-4c compares (23) and M2-1 (18)**: `lower-mixprec3` (plan #402,
   81b3ce19) already does both. 41 cases leave the text, 27 of them
   compile. If train 3c lands its own M2-1 and M2-4c instead, it should
   take 81b3ce19's `overwritten_stores` (bare blocks included) and
   check `hp_lt_vec3` and `hp_bin_eq_vec3` byte-equal.
2. **MR2 (6) together with M2-4a (6)**: both stop a flag where the walk
   already gives Mesa's words. With the flag skipped, 6 new references
   are byte-equal (`hp_sin`, `hp_cos`, `hp_log2`, `hp_pow`,
   `hp_inout_param`, `hp_seq_bvec4`). So was mixprec2's `hp_out_param`
   at 8133663c (its note, M2-4). This is the cheapest step. tan, log and `hp_seq_vec4` follow in the same session if their
   words come.
3. **MR1 (4)**: iterate M2-1's deadness, so that a store read only by dead
   stores is dead. This is a small change after M2-1, and 2 of its 4 cases
   then compile.
4. **M2-4b (7)**: `lower-vsconst` (plan #402, 2c0c6596) already folds
   the `add`/`sub` trees at fp32. 6 cases compile there, and `hp_mat3_add`
   and `hp_mat4_sub` are byte-equal. What is left is the matrix product
   fold for `hp_mat2_mul` (`matrix.mul.const_highp_mat2_mat2`, 1 case).
5. **M2-2 (9) and M2-3 (2)**: the `:7285` arm's operand forms, in one session.
6. **M2-4c rest (8)**: the fp32 island through `fract`, `sqrt`, `length`
   and `distance`. `hp_distance` also needs the vertex side: Mesa's vertex
   program is 5 `vs.varying` words and ours is 21.
7. **MX3 (6)**: the fp32 loop accumulator and counter. It shares the loop
   lowering of `mali-loops-lowering.md`. All 6 are board NotSupported
   (`shaders.loops`), so it goes last.
8. **MX5**: no case at the tip. Keep `f2i_rnd`/`f2i_dead` pinned, and
   plan no session.

Totals: 18 + 4 + 9 + 2 + 6 + 7 + 23 + 8 + 6 + 6 + 0 = **89**.

## 5. The test

`tests/mali_deqp_mixres.rs` (features `--no-default-features --features
host-test,gpu-mali,shaderc-online,no-shader-table`). Each reference has
an `#[ignore]`d `assert_matches` test. `references_parse` checks the
files and `FS_WORDS`. `refusals_pinned` accepts a reference's outcome at
349f989f, or the one measured at 81b3ce19 or 2c0c6596 (`PINS`):

* `hp_lt_vec3` and `hp_bin_eq_vec3`: byte-equal at 81b3ce19;
* `rnd_block_store`: refused with `FsOperandForm` at 81b3ce19;
* `hp_mat3_add` and `hp_mat4_sub`: byte-equal at 2c0c6596.

The test passes at 349f989f, at 81b3ce19 and at 2c0c6596. A step that
moves a reference to another outcome updates its pin in the same commit.

## 6. Reproducing

```text
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixres
# one reference (MESA_SHIM_WORK defaults to $CONNECTED_DEPS_DIR/mesa-shim)
tools/mali/deqp_ref.sh build/measure/src/<case>.vert build/measure/src/<case>.frag \
    tools/mali/testdata/deqp_mixres_<name> --instrument shim
tools/mali/g310_check.sh --scratch "$TMPDIR/g310" 'deqp_mixres_*'
# the 89 cases: measure.tsv rows whose host_refusal starts "a fragment shader that mixes highp"
awk -F'\t' '$8 ~ /mixes highp/' build/measure/measure.tsv
```
