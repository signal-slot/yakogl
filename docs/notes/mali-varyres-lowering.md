<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali `FsVaryingLoad` residue: the 124 first-refusal cases at `349f989f`, steps VR1-VR3

Date 2026-11-28. Plan #387 item `varyres-refs` (stage 5, issue #7; NanoPC-T6
Mali-G610, and the Verdin iMX95 Mali-G310, whose words are the G610's,
`mali-g310-vs-g610.md`). It adds 26 Mesa 26.2.1 references
(`tools/mali/testdata/deqp_varyres_*`) and `tests/mali_deqp_varyres.rs`. It
changes no code. **HOST-ONLY**: compile and link outcomes, nothing ran on a
board.

Section 19.1 of `mali-deqp-families.md` ranks `FsVaryingLoad` ("a fragment
varying load or conversion of a form no reference draws ...",
`MaliError::FsVaryingLoad`) 9th at `e9033663` with 123 cases. The schedule's
owner, `varyload-lower-1` (train 3c; `mali-lowering-schedule.md` 5.5 and 5.6,
references in `mali-deqp-varyload.md`), was sized for 22 cases. This note
measures the text again, names the refusing site of every case and maps each
case to `varyload-lower-1` or to a new step.

## 1. Measure

- Tip: the integration tip **`349f989f5392bea65c20e7663289e86481e521e0`**.
  `capi/Cargo.toml` at that tip has the `vivante-st` feature twice
  (`110080eb`), and cargo 1.99 will not load the workspace with it. The run
  dropped the second entry in the working tree only. That is not part of this
  item's change.
- `tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" --out build/measure --jobs 3 --chunk 100`,
  CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` (fetched at that commit,
  `external/fetch_sources.py`, `tools/cts/cts-yakogl-target.patch` applied),
  the 10,597 cases of the slices `2026-11-15-t6-gles2-S{1..4}-37305608`.
- All 10,597 began, 106 chunk processes exited 1, none crashed. 4,709 cases
  still meet a compile or link refusal (4,716 at `e9033663`).
- **124 cases have `FsVaryingLoad` as their first refusal** (123 at
  `e9033663`). All are fragment-stage refusals and all are Fail on the board.
  By group: `shaders.operator` 90 (binary_operator 64, common_functions 14,
  unary_operator 6, geometric 6), `shaders.random` 26, `shaders.indexing` 4,
  `shaders.return` 2, `shaders.matrix` 1, `fragment_ops.interaction` 1.
- Refusing site: a scratch copy of the tip (never committed) printed
  `line!()` at each of the eight `Err(MaliError::FsVaryingLoad)` of
  `src/shaderc/online/mali/nir_fs.rs`. It then compiled each case's program
  pair from `programs.tsv` (one per case) through `tests/mali_ref`'s
  `compile_pair`. All 124 were refused there too, each with the same text.

| Site (`nir_fs.rs` at `349f989f`) | Function | Cases | What it refuses |
|---|---|--:|---|
| `:9918` | `layout` | 76 | a highp varying of a mediump shader kept whole with lanes nobody reads, beside a varying whose read lanes alone are kept (the "straddle" of `deqp_mixprec3_hp_vec3_sum`) |
| `:11261` | `conversion_order` | 27 | one variable's converted lanes that come from more than one packed slot base |
| `:11335` | `conversion_order` | 18 | conversions that are not Mesa's set or order (a pair Mesa converts with its odd lane low, a lone pair) |
| `:9678` | `colour_keeps_whole` | 3 | a colour store beside a value of no measured kind, or an unknown lane of the varying alone |
| `:7382` (`walk_nodes`), `:9563`, `:9606` (`kept_whole`), `:12703` (`select`) | | 0 | |

`walk_nodes` refuses none of the 124 cases.

## 2. `varyload-lower-1` measured: plan #402's `lower-mixprec3` (`81b3ce19`)

`varyload-lower-1` has no code at this tip. Plan #402's finished item
`lower-mixprec3` (commit `81b3ce19`, branch
`connected/01a21f6f-c1d8-76ce-9751-bcc81024775c`, not landed) carries that
lowering, and its references are the `deqp_varyload_*` ones. This note takes
it as `varyload-lower-1`'s implementation (`Assumption` in the item's report).
`git cherry-pick --no-commit 81b3ce19` applies cleanly on `349f989f`. It was
instrumented the same way and given the same 124 pairs:

- **45 cases leave the text**: all 27 at `:11261` and all 18 at `:11335`.
  32 of them compile and 13 move to another text: `MixedPrecision` 9
  (`geometric.{distance,length}.highp_*`, `matrix.mul.const_highp_mat4_vec4`,
  `{mix,mod,reflect}.highp_float`), the link refusal "a varying at different
  offsets" 3 and `ColourForm` 1.
- **79 stay**: the 76 of `:9918` (at `:9990` in that tree, the same check)
  and the 3 of `:9678` (at `:9744`).
- Of the 32 that compile, the references say **20 are byte-equal and 12 are
  not Mesa's words**. The project's rule is that a shape Mesa compiles
  differently must not be left "compiled, not Mesa's", so those 12 get a
  step of their own, VR3.

## 3. Steps

| Step | Mechanism (one line) | Site | Cases | dEQP groups | References (`deqp_`) | Status |
|---|---|---|--:|---|---|---|
| **VR1a** | Mesa keeps the vector varying whole in `VAR0` (`LD_VAR_BUF_IMM.f32.v4` at index 0, only the read lanes used) and the scalar alone in `VAR1.x` (`.f32` at index 16). Compute at fp32, `FADD.f32 x, -0.0` to `.h00`, `MKVEC` the mediump colour. `layout` must place them so instead of refusing | `layout` `:9918` | 54 | `binary_operator` 48 (`{add,sub,mul,div}{,_assign_effect,_assign_result}.highp_{vec2,vec3}_float` and `highp_{ivec2,ivec3}_int`), `common_functions` 6 (`{min,max,mod}.highp_vec{2,3}_float`) | `varyres_st_vec2_float`, `varyres_st_vec3_float`, `varyres_st_ivec2_int`, `varyres_st_ivec3_int`; already drawn: `highpfs_hp_mod_vec3_float` | **new**, no item |
| **VR1b** | The same packing when the scalar is the first operand: Mesa compacts the scalar's read lane and the vector's into `VAR0` and loads one `.v4` at index 0 | `layout` `:9918` | 16 | `binary_operator` 16 (`{add,sub,mul,div}.highp_float_vec{2,3}`, `highp_int_ivec{2,3}`) | `varyres_st_float_vec2`, `varyres_st_int_ivec3`; already drawn: `mixprec3_hp_vec3_sum`, `audit_l1_op_mul_highp_int_ivec2_fs`, `missing_mixprec_fs_highp_declared` | **new**, no item |
| **VR1c** | VR1 with three highp varyings (a vector, a scalar, a third read in part) | `layout` `:9918` | 6 | `common_functions` 6 (`clamp`, `mix` `.highp_vec{2,3}_float`, `smoothstep.highp_float_vec{2,3}`) | `varyres_st_clamp_vec2`, `_clamp_vec3`, `_mix_vec2`, `_mix_vec3`, `_smooth_vec2`; already drawn: `highpfs_hp_smoothstep_float_float_vec3` | **new**, no item |
| **VR2** | Follow a colour lane through a local stored on a constant-false or constant-true path (a function's dead `return`), and through `gl_FragColor` stores that a later store overwrites, before `colour_keeps_whole` calls it unknown | `colour_keeps_whole` `:9678` | 3 | `return.conditional_return_{always,never}_fragment` 2, `random.swizzle.fragment.41` 1 | `varyres_ret_never`; already drawn: `gaps_mixprec_fs_varying_other` (= `_always`), `varyload_swz41` | **new**, no item (`81b3ce19` leaves all three refused) |
| `varyload-lower-1` | `conversion_order`'s lazy and swapped conversions, `overwritten_stores` (as `81b3ce19`) | `conversion_order` `:11261`, `:11335` | 33 | `random` 13 (`basic_expression` 8, `conditionals` 2, `comparison_ops` 1, `scalar_conversion` 1, `texture` 1; 3 of them to link offsets, `basic_expression.combined.33` to `ColourForm`), `unary_operator` 6, `geometric` 6, `indexing.varying_array` 4, `common_functions` 2, `matrix.mul` 1, `interaction` 1 | `varyres_interact17`, `varyres_mp_vec3_postinc`, `varyres_vary_array_vec2`, `varyres_rnd_cmp6`, `varyres_rnd_cond75`, `varyres_rnd_sconv6`; already drawn: `varyload_{rnd57,rnd31,basic33,cond19,tex110}`, `mixprec3_{mp,lp}_vec3_scale`, `mixprec3_vary_array_static`, `mixcol_lz_rnd_other`, `arith_op_common_mod_fs_highp`, `mixprec3_hp2_sum` | train 3c (existing). With `81b3ce19`: 20 byte-equal, 13 to the next text |
| **VR3** | `81b3ce19` compiles these with other words than Mesa's: the vertex varying stores or position FAU (`rnd_cmp15`, `rnd_swz36`: `vs.varying` 13 words against Mesa's 8; `rnd_basic30`: `vs.position` FAU and `vs.varying` 30 against 25), and the fragment program (`rnd_basic71`, `rnd_sconv42`, `rnd_sconv65`, `rnd_swz15`, `rnd_swz47`, `varyload_basic8`, `varyload_cond36`) | `conversion_order` `:11261`, `:11335` | 12 | `random.basic_expression` 4 (`.8` twice, `.30`, `.71`), `random.scalar_conversion` 3 (`fragment.65`, `combined.65`, `combined.42`), `random.swizzle` 3 (`.15`, `.36`, `.47`), `random.comparison_ops.fragment.15` 1, `random.conditionals.fragment.36` 1 | `varyres_rnd_{basic30,basic71,cmp15,sconv42,sconv65,swz15,swz36,swz47}`; already drawn: `varyload_basic8`, `varyload_cond36` | **new**: fix before or with `varyload-lower-1`, never after it |
| | **Total** | | **124** | | | |

54 + 16 + 6 + 3 + 33 + 12 = 124. Appendix A puts each case in its row.

Forecast (HOST-ONLY, upper bounds, no inflow), cases leaving the text:

- `varyload-lower-1` as `81b3ce19` stands moves 45 cases, but only 20 of
  them to Mesa's words. With VR3, 32 compile byte-equal (by reference) and 13
  move to `MixedPrecision` (9), link offsets (3) and `ColourForm` (1).
- VR1a-VR1c move 76. The next text of those cases cannot be measured until
  VR1 exists. The `div` and `int` operators probably next meet `Arithmetic`
  or `IntValue` (`div` 16 cases, the integer operands 32, 8 of them both).
- VR2 moves 3.
- After all of them, `FsVaryingLoad` has 0 first refusals. The text falls
  under the 100 line once `varyload-lower-1` lands (124 - 45 = 79). VR1 holds
  76 of those 79 and is one mechanism, so it is the step that takes the text
  to (near) 0.

## 4. The new references

All 26 were drawn with `tools/mali/deqp_ref.sh <pair> tools/mali/testdata/deqp_varyres_<name> --instrument shim`
(Mesa 26.2.1's gallium panfrost on the drm-shim, GPU g610, `--uniform-values`),
built with `tools/mali/drm_shim_ref.sh build` into
`$CONNECTED_DEPS_DIR/mesa-shim`. Each pair is the case's verbatim `.qpa`
source from the run of section 1, and each `.license` names its case.
`tools/mali/nir_refs.sh 'deqp_varyres_*'` wrote the `.nir.sha`. As a check of
the build, `nir_refs.sh 'deqp_varyload_*' deqp_mixprec3_hp_vec3_sum` found all
9 unchanged, and `deqp_varyload_swz41` redrawn gave an identical
`expect.json`. `return.conditional_return_always` is byte-identical to the
existing `deqp_gaps_mixprec_fs_varying_other` and was not drawn again.

| Reference | Case | Step | At `349f989f` | With `81b3ce19` (HOST-ONLY) |
|---|---|---|---|---|
| `st_vec2_float` | `binary_operator.add.highp_vec2_float` | VR1a | refused `:9918` | refused |
| `st_vec3_float` | `binary_operator.add.highp_vec3_float` | VR1a | refused `:9918` | refused |
| `st_ivec2_int` | `binary_operator.add.highp_ivec2_int` | VR1a | refused `:9918` | refused |
| `st_ivec3_int` | `binary_operator.add.highp_ivec3_int` | VR1a | refused `:9918` | refused |
| `st_float_vec2` | `binary_operator.add.highp_float_vec2` | VR1b | refused `:9918` | refused |
| `st_int_ivec3` | `binary_operator.add.highp_int_ivec3` | VR1b | refused `:9918` | refused |
| `st_clamp_vec2`, `st_clamp_vec3` | `common_functions.clamp.highp_vec{2,3}_float` | VR1c | refused `:9918` | refused |
| `st_mix_vec2`, `st_mix_vec3` | `common_functions.mix.highp_vec{2,3}_float` | VR1c | refused `:9918` | refused |
| `st_smooth_vec2` | `common_functions.smoothstep.highp_float_vec2` | VR1c | refused `:9918` | refused |
| `ret_never` | `return.conditional_return_never_fragment` | VR2 | refused `:9678` | refused |
| `interact17` | `fragment_ops.interaction.basic_shader.17` | varyload-lower-1 | refused `:11335` | byte-equal |
| `mp_vec3_postinc` | `unary_operator.post_increment_result.mediump_vec3` | varyload-lower-1 | refused `:11335` | byte-equal |
| `vary_array_vec2` | `indexing.varying_array.vec2_static_write_static_read` | varyload-lower-1 | refused `:11261` | byte-equal |
| `rnd_cmp6` | `random.comparison_ops.fragment.6` | varyload-lower-1 | refused `:11261` | link offsets (`'b'`) |
| `rnd_cond75` | `random.conditionals.fragment.75` | varyload-lower-1 | refused `:11261` | link offsets (`'a'`) |
| `rnd_sconv6` | `random.scalar_conversion.fragment.6` | varyload-lower-1 | refused `:11261` | link offsets (`'b'`) |
| `rnd_basic30` | `random.basic_expression.fragment.30` | VR3 | refused `:11261` | differs: `vs.position` FAU, `vs.varying` 30/25 |
| `rnd_basic71` | `random.basic_expression.fragment.71` | VR3 | refused `:11261` | differs: `fs.fragment` 15/15 |
| `rnd_cmp15` | `random.comparison_ops.fragment.15` | VR3 | refused `:11261` | differs: `vs.varying` 13/8 |
| `rnd_sconv42` | `random.scalar_conversion.combined.42` | VR3 | refused `:11335` | differs: `fs.fragment` 13/12 |
| `rnd_sconv65` | `random.scalar_conversion.fragment.65` | VR3 | refused `:11261` | differs: `fs.fragment` 8/8 |
| `rnd_swz15` | `random.swizzle.fragment.15` | VR3 | refused `:11261` | differs: `fs.fragment` 11/18 |
| `rnd_swz36` | `random.swizzle.fragment.36` | VR3 | refused `:11261` | differs: `vs.varying` 13/8 |
| `rnd_swz47` | `random.swizzle.fragment.47` | VR3 | refused `:11261` | differs: `fs.fragment` 11/11 |

"n/m" is our words / Mesa's. `tests/mali_deqp_varyres.rs` pins all 26 as
refused with the whole `FsVaryingLoad` text (`REFUSED`; `LOWERED` and
`DIFFERS` empty). When train 3c lands `81b3ce19` (or its successor), the
three byte-equal ones move to `LOWERED`, the three link-offset ones get the
link text in `REFUSED`, and the eight VR3 ones move to `DIFFERS` until VR3
fixes them, all in the same commit.

## 5. Relation to `varyload-lower-1` and T1, and order

- **`varyload-lower-1`** (train 3c) edits `conversion_order`, `select`,
  `walk_nodes` (the `-0.0 * b` fold) and `layout` (one hunk at `:9825`, the
  lazy slots, `81b3ce19`). VR3 is the same code and the same shapes, so it
  belongs in the same train, right after `varyload-lower-1` or folded into
  it. It must land before or with it, so that the 12 cases never ship
  compiled with other words than Mesa's.
- **VR1** edits `layout`'s slot placement (the straddle check and what
  replaces it) and the vertex side's matching varying stores
  (`link::VaryingOrder`). It shares `layout` with `varyload-lower-1`'s lazy
  slots, so it rebases after `varyload-lower-1`, not in parallel with it.
- **VR2** edits `colour_keeps_whole` / `ir_colour_lane`, which
  `varyload-lower-1` does not touch (its `overwritten_stores` is in `mod.rs`
  and feeds `ir_live`). It can go anywhere after `varyload-lower-1`. swz41's
  dead stores are the same family as `overwritten_stores`, so reuse it.
- **T1** (TexCoord cube sequence, train 4) edits `walk_nodes`' mediump cube
  arm. None of VR1-VR3 edits `walk_nodes`. Only `varyload-lower-1`'s
  `-0.0 * b` fold does, which is why the schedule already orders T1 after it.
  VR1-VR3 add no dependency to T1.
- **Order**: `varyload-lower-1` + VR3 (train 3c, 45 cases) first, then VR1a,
  VR1b, VR1c (76 cases, one mechanism, one item; a commit each), then VR2
  (3 cases). Placing them in a train is `schedule-refresh-4`'s job.

## 6. Caveats

- HOST-ONLY. "Byte-equal" and "differs" are per reference. A `combined.*`
  case shares only its fragment shader with its reference (its vertex shader
  differs), and a "same shape" case (`post_decrement_result`, the lowp
  variants, `vary_array` loop reads, the VR1 siblings with other operators)
  has no reference of its own.
- The board status is the NanoPC-T6's at `37305608` (the slices), not a
  rerun.
- `81b3ce19` is another plan's finished work, not landed. If train 3c
  carries a different diff, section 2's 20/12/13 split must be measured
  again with the references here.

## 7. Reproducing

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" --out "$PWD/build/measure" --jobs 3 --chunk 100
awk -F'\t' '$8 ~ /fragment varying load/' build/measure/measure.tsv     # the 124
tools/mali/deqp_ref.sh <case>.vert <case>.frag tools/mali/testdata/deqp_varyres_<name> --instrument shim
tools/mali/nir_refs.sh 'deqp_varyres_*'
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_varyres
```

## Appendix A. Every `FsVaryingLoad` first-refusal case, its site and step

| # | Case (`dEQP-GLES2.functional.`) | Site at `349f989f` | Step | Reference (`deqp_`) | With `81b3ce19` applied (HOST-ONLY) |
|--:|---|---|---|---|---|
| 1 | `shaders.operator.binary_operator.add.highp_ivec2_int_fragment` | `:9918` | VR1a | `varyres_st_ivec2_int` | `FsVaryingLoad` (:9990) |
| 2 | `shaders.operator.binary_operator.mul.highp_ivec2_int_fragment` | `:9918` | VR1a | `varyres_st_ivec2_int` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 3 | `shaders.operator.binary_operator.sub.highp_ivec2_int_fragment` | `:9918` | VR1a | `varyres_st_ivec2_int` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 4 | `shaders.operator.binary_operator.add.highp_ivec3_int_fragment` | `:9918` | VR1a | `varyres_st_ivec3_int` | `FsVaryingLoad` (:9990) |
| 5 | `shaders.operator.binary_operator.mul.highp_ivec3_int_fragment` | `:9918` | VR1a | `varyres_st_ivec3_int` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 6 | `shaders.operator.binary_operator.sub.highp_ivec3_int_fragment` | `:9918` | VR1a | `varyres_st_ivec3_int` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 7 | `shaders.operator.binary_operator.add.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` | `FsVaryingLoad` (:9990) |
| 8 | `shaders.operator.binary_operator.add_assign_effect.highp_ivec2_int_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 9 | `shaders.operator.binary_operator.add_assign_effect.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 10 | `shaders.operator.binary_operator.add_assign_result.highp_ivec2_int_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 11 | `shaders.operator.binary_operator.add_assign_result.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 12 | `shaders.operator.binary_operator.div.highp_ivec2_int_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 13 | `shaders.operator.binary_operator.div.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 14 | `shaders.operator.binary_operator.div_assign_effect.highp_ivec2_int_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 15 | `shaders.operator.binary_operator.div_assign_effect.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 16 | `shaders.operator.binary_operator.div_assign_result.highp_ivec2_int_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 17 | `shaders.operator.binary_operator.div_assign_result.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 18 | `shaders.operator.binary_operator.mul.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 19 | `shaders.operator.binary_operator.mul_assign_effect.highp_ivec2_int_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 20 | `shaders.operator.binary_operator.mul_assign_effect.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 21 | `shaders.operator.binary_operator.mul_assign_result.highp_ivec2_int_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 22 | `shaders.operator.binary_operator.mul_assign_result.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 23 | `shaders.operator.binary_operator.sub.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 24 | `shaders.operator.binary_operator.sub_assign_effect.highp_ivec2_int_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 25 | `shaders.operator.binary_operator.sub_assign_effect.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 26 | `shaders.operator.binary_operator.sub_assign_result.highp_ivec2_int_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 27 | `shaders.operator.binary_operator.sub_assign_result.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 28 | `shaders.operator.common_functions.max.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 29 | `shaders.operator.common_functions.min.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 30 | `shaders.operator.common_functions.mod.highp_vec2_float_fragment` | `:9918` | VR1a | `varyres_st_vec2_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 31 | `shaders.operator.binary_operator.add.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` | `FsVaryingLoad` (:9990) |
| 32 | `shaders.operator.binary_operator.add_assign_effect.highp_ivec3_int_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 33 | `shaders.operator.binary_operator.add_assign_effect.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 34 | `shaders.operator.binary_operator.add_assign_result.highp_ivec3_int_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 35 | `shaders.operator.binary_operator.add_assign_result.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 36 | `shaders.operator.binary_operator.div.highp_ivec3_int_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 37 | `shaders.operator.binary_operator.div.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 38 | `shaders.operator.binary_operator.div_assign_effect.highp_ivec3_int_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 39 | `shaders.operator.binary_operator.div_assign_effect.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 40 | `shaders.operator.binary_operator.div_assign_result.highp_ivec3_int_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 41 | `shaders.operator.binary_operator.div_assign_result.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 42 | `shaders.operator.binary_operator.mul.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 43 | `shaders.operator.binary_operator.mul_assign_effect.highp_ivec3_int_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 44 | `shaders.operator.binary_operator.mul_assign_effect.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 45 | `shaders.operator.binary_operator.mul_assign_result.highp_ivec3_int_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 46 | `shaders.operator.binary_operator.mul_assign_result.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 47 | `shaders.operator.binary_operator.sub.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 48 | `shaders.operator.binary_operator.sub_assign_effect.highp_ivec3_int_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 49 | `shaders.operator.binary_operator.sub_assign_effect.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 50 | `shaders.operator.binary_operator.sub_assign_result.highp_ivec3_int_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 51 | `shaders.operator.binary_operator.sub_assign_result.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 52 | `shaders.operator.common_functions.max.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 53 | `shaders.operator.common_functions.min.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 54 | `shaders.operator.common_functions.mod.highp_vec3_float_fragment` | `:9918` | VR1a | `varyres_st_vec3_float` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 55 | `shaders.operator.binary_operator.add.highp_int_ivec2_fragment` | `:9918` | VR1b | `audit_l1_op_mul_highp_int_ivec2_fs` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 56 | `shaders.operator.binary_operator.mul.highp_int_ivec2_fragment` | `:9918` | VR1b | `audit_l1_op_mul_highp_int_ivec2_fs` | `FsVaryingLoad` (:9990) |
| 57 | `shaders.operator.binary_operator.sub.highp_int_ivec2_fragment` | `:9918` | VR1b | `audit_l1_op_mul_highp_int_ivec2_fs` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 58 | `shaders.operator.binary_operator.add.highp_float_vec3_fragment` | `:9918` | VR1b | `mixprec3_hp_vec3_sum` | `FsVaryingLoad` (:9990) |
| 59 | `shaders.operator.binary_operator.div.highp_float_vec3_fragment` | `:9918` | VR1b | `mixprec3_hp_vec3_sum` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 60 | `shaders.operator.binary_operator.div.highp_int_ivec3_fragment` | `:9918` | VR1b | `mixprec3_hp_vec3_sum` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 61 | `shaders.operator.binary_operator.mul.highp_float_vec3_fragment` | `:9918` | VR1b | `mixprec3_hp_vec3_sum` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 62 | `shaders.operator.binary_operator.sub.highp_float_vec3_fragment` | `:9918` | VR1b | `mixprec3_hp_vec3_sum` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 63 | `shaders.operator.binary_operator.add.highp_float_vec2_fragment` | `:9918` | VR1b | `varyres_st_float_vec2` | `FsVaryingLoad` (:9990) |
| 64 | `shaders.operator.binary_operator.div.highp_float_vec2_fragment` | `:9918` | VR1b | `varyres_st_float_vec2` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 65 | `shaders.operator.binary_operator.div.highp_int_ivec2_fragment` | `:9918` | VR1b | `varyres_st_float_vec2` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 66 | `shaders.operator.binary_operator.mul.highp_float_vec2_fragment` | `:9918` | VR1b | `varyres_st_float_vec2` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 67 | `shaders.operator.binary_operator.sub.highp_float_vec2_fragment` | `:9918` | VR1b | `varyres_st_float_vec2` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 68 | `shaders.operator.binary_operator.add.highp_int_ivec3_fragment` | `:9918` | VR1b | `varyres_st_int_ivec3` | `FsVaryingLoad` (:9990) |
| 69 | `shaders.operator.binary_operator.mul.highp_int_ivec3_fragment` | `:9918` | VR1b | `varyres_st_int_ivec3` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 70 | `shaders.operator.binary_operator.sub.highp_int_ivec3_fragment` | `:9918` | VR1b | `varyres_st_int_ivec3` (same varying lanes) | `FsVaryingLoad` (:9990) |
| 71 | `shaders.operator.common_functions.smoothstep.highp_float_vec3_fragment` | `:9918` | VR1c | `highpfs_hp_smoothstep_float_float_vec3` | `FsVaryingLoad` (:9990) |
| 72 | `shaders.operator.common_functions.clamp.highp_vec2_float_fragment` | `:9918` | VR1c | `varyres_st_clamp_vec2` | `FsVaryingLoad` (:9990) |
| 73 | `shaders.operator.common_functions.clamp.highp_vec3_float_fragment` | `:9918` | VR1c | `varyres_st_clamp_vec3` | `FsVaryingLoad` (:9990) |
| 74 | `shaders.operator.common_functions.mix.highp_vec2_float_fragment` | `:9918` | VR1c | `varyres_st_mix_vec2` | `FsVaryingLoad` (:9990) |
| 75 | `shaders.operator.common_functions.mix.highp_vec3_float_fragment` | `:9918` | VR1c | `varyres_st_mix_vec3` | `FsVaryingLoad` (:9990) |
| 76 | `shaders.operator.common_functions.smoothstep.highp_float_vec2_fragment` | `:9918` | VR1c | `varyres_st_smooth_vec2` | `FsVaryingLoad` (:9990) |
| 77 | `shaders.return.conditional_return_always_fragment` | `:9678` | VR2 | `gaps_mixprec_fs_varying_other` | `FsVaryingLoad` (:9744) |
| 78 | `shaders.random.swizzle.fragment.41` | `:9678` | VR2 | `varyload_swz41` | `FsVaryingLoad` (:9744) |
| 79 | `shaders.return.conditional_return_never_fragment` | `:9678` | VR2 | `varyres_ret_never` | `FsVaryingLoad` (:9744) |
| 80 | `shaders.operator.common_functions.mod.highp_float_fragment` | `:11261` | varyload-lower-1 | `arith_op_common_mod_fs_highp` | `MixedPrecision` |
| 81 | `shaders.random.basic_expression.combined.45` | `:11261` | varyload-lower-1 | `mixcol_lz_rnd_other` (same fragment shader, other vertex shader) | compiles, byte-equal to the reference |
| 82 | `shaders.random.basic_expression.fragment.45` | `:11261` | varyload-lower-1 | `mixcol_lz_rnd_other` | compiles, byte-equal to the reference |
| 83 | `shaders.matrix.mul.const_highp_mat4_vec4_fragment` | `:11335` | varyload-lower-1 | `mixprec3_hp2_sum` (same family; next text `MixedPrecision`) | `MixedPrecision` |
| 84 | `shaders.operator.common_functions.mix.highp_float_fragment` | `:11261` | varyload-lower-1 | `mixprec3_hp2_sum` (same family; next text `MixedPrecision`) | `MixedPrecision` |
| 85 | `shaders.operator.geometric.distance.highp_vec2_fragment` | `:11335` | varyload-lower-1 | `mixprec3_hp2_sum` (same family; next text `MixedPrecision`) | `MixedPrecision` |
| 86 | `shaders.operator.geometric.distance.highp_vec3_fragment` | `:11335` | varyload-lower-1 | `mixprec3_hp2_sum` (same family; next text `MixedPrecision`) | `MixedPrecision` |
| 87 | `shaders.operator.geometric.distance.highp_vec4_fragment` | `:11335` | varyload-lower-1 | `mixprec3_hp2_sum` (same family; next text `MixedPrecision`) | `MixedPrecision` |
| 88 | `shaders.operator.geometric.length.highp_vec3_fragment` | `:11335` | varyload-lower-1 | `mixprec3_hp2_sum` (same family; next text `MixedPrecision`) | `MixedPrecision` |
| 89 | `shaders.operator.geometric.length.highp_vec4_fragment` | `:11335` | varyload-lower-1 | `mixprec3_hp2_sum` (same family; next text `MixedPrecision`) | `MixedPrecision` |
| 90 | `shaders.operator.geometric.reflect.highp_float_fragment` | `:11261` | varyload-lower-1 | `mixprec3_hp2_sum` (same family; next text `MixedPrecision`) | `MixedPrecision` |
| 91 | `shaders.operator.unary_operator.plus.lowp_vec3_fragment` | `:11335` | varyload-lower-1 | `mixprec3_lp_vec3_scale` | compiles, byte-equal to the reference |
| 92 | `shaders.operator.unary_operator.plus.mediump_vec3_fragment` | `:11335` | varyload-lower-1 | `mixprec3_mp_vec3_scale` | compiles, byte-equal to the reference |
| 93 | `shaders.indexing.varying_array.float_static_write_static_loop_read` | `:11261` | varyload-lower-1 | `mixprec3_vary_array_static` (same shape, loop read) | compiles, byte-equal to the reference |
| 94 | `shaders.indexing.varying_array.float_static_write_static_read` | `:11261` | varyload-lower-1 | `mixprec3_vary_array_static` | compiles, byte-equal to the reference |
| 95 | `shaders.random.basic_expression.combined.33` | `:11261` | varyload-lower-1 | `varyload_basic33` (same fragment shader, other vertex shader) | `ColourForm` |
| 96 | `shaders.random.basic_expression.fragment.33` | `:11261` | varyload-lower-1 | `varyload_basic33` | compiles, byte-equal to the reference |
| 97 | `shaders.random.conditionals.fragment.19` | `:11335` | varyload-lower-1 | `varyload_cond19` | compiles, byte-equal to the reference |
| 98 | `shaders.random.basic_expression.combined.31` | `:11335` | varyload-lower-1 | `varyload_rnd31` (same fragment shader, other vertex shader) | compiles, byte-equal to the reference |
| 99 | `shaders.random.basic_expression.fragment.31` | `:11335` | varyload-lower-1 | `varyload_rnd31` | compiles, byte-equal to the reference |
| 100 | `shaders.random.basic_expression.combined.57` | `:11261` | varyload-lower-1 | `varyload_rnd57` (same fragment shader, other vertex shader) | compiles, byte-equal to the reference |
| 101 | `shaders.random.basic_expression.fragment.57` | `:11261` | varyload-lower-1 | `varyload_rnd57` | compiles, byte-equal to the reference |
| 102 | `shaders.random.texture.fragment.110` | `:11335` | varyload-lower-1 | `varyload_tex110` | compiles, byte-equal to the reference |
| 103 | `fragment_ops.interaction.basic_shader.17` | `:11335` | varyload-lower-1 | `varyres_interact17` | compiles, byte-equal to the reference |
| 104 | `shaders.operator.unary_operator.post_decrement_result.lowp_vec3_fragment` | `:11335` | varyload-lower-1 | `varyres_mp_vec3_postinc` (same shape) | compiles, byte-equal to the reference |
| 105 | `shaders.operator.unary_operator.post_decrement_result.mediump_vec3_fragment` | `:11335` | varyload-lower-1 | `varyres_mp_vec3_postinc` (same shape) | compiles, byte-equal to the reference |
| 106 | `shaders.operator.unary_operator.post_increment_result.lowp_vec3_fragment` | `:11335` | varyload-lower-1 | `varyres_mp_vec3_postinc` (same shape) | compiles, byte-equal to the reference |
| 107 | `shaders.operator.unary_operator.post_increment_result.mediump_vec3_fragment` | `:11335` | varyload-lower-1 | `varyres_mp_vec3_postinc` | compiles, byte-equal to the reference |
| 108 | `shaders.random.comparison_ops.fragment.6` | `:11261` | varyload-lower-1 | `varyres_rnd_cmp6` | `link: varying offsets` |
| 109 | `shaders.random.conditionals.fragment.75` | `:11261` | varyload-lower-1 | `varyres_rnd_cond75` | `link: varying offsets` |
| 110 | `shaders.random.scalar_conversion.fragment.6` | `:11261` | varyload-lower-1 | `varyres_rnd_sconv6` | `link: varying offsets` |
| 111 | `shaders.indexing.varying_array.vec2_static_write_static_loop_read` | `:11261` | varyload-lower-1 | `varyres_vary_array_vec2` | compiles, byte-equal to the reference |
| 112 | `shaders.indexing.varying_array.vec2_static_write_static_read` | `:11261` | varyload-lower-1 | `varyres_vary_array_vec2` | compiles, byte-equal to the reference |
| 113 | `shaders.random.basic_expression.combined.8` | `:11261` | VR3 | `varyload_basic8` (same fragment shader, other vertex shader) | compiles, **differs** from the reference |
| 114 | `shaders.random.basic_expression.fragment.8` | `:11261` | VR3 | `varyload_basic8` | compiles, **differs** from the reference |
| 115 | `shaders.random.conditionals.fragment.36` | `:11261` | VR3 | `varyload_cond36` | compiles, **differs** from the reference |
| 116 | `shaders.random.basic_expression.fragment.30` | `:11261` | VR3 | `varyres_rnd_basic30` | compiles, **differs** from the reference |
| 117 | `shaders.random.basic_expression.fragment.71` | `:11261` | VR3 | `varyres_rnd_basic71` | compiles, **differs** from the reference |
| 118 | `shaders.random.comparison_ops.fragment.15` | `:11261` | VR3 | `varyres_rnd_cmp15` | compiles, **differs** from the reference |
| 119 | `shaders.random.scalar_conversion.combined.42` | `:11335` | VR3 | `varyres_rnd_sconv42` | compiles, **differs** from the reference |
| 120 | `shaders.random.scalar_conversion.combined.65` | `:11261` | VR3 | `varyres_rnd_sconv65` (same fragment shader, other vertex shader) | compiles, **differs** from the reference |
| 121 | `shaders.random.scalar_conversion.fragment.65` | `:11261` | VR3 | `varyres_rnd_sconv65` | compiles, **differs** from the reference |
| 122 | `shaders.random.swizzle.fragment.15` | `:11261` | VR3 | `varyres_rnd_swz15` | compiles, **differs** from the reference |
| 123 | `shaders.random.swizzle.fragment.36` | `:11261` | VR3 | `varyres_rnd_swz36` | compiles, **differs** from the reference |
| 124 | `shaders.random.swizzle.fragment.47` | `:11261` | VR3 | `varyres_rnd_swz47` | compiles, **differs** from the reference |
