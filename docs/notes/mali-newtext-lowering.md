<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the small refusal texts no step owned, a lowering design

Plan #387, item `newtexts-refs` (HOST-ONLY, no board). Section 19.1 of
`docs/notes/mali-deqp-families.md` (measured at `e9033663`) lists nine small
refusal texts with no owner. This note measures them again at the tip and
gives each text its shape, the site that refuses it, how Mesa 26.2.1 compiles
it, and a step (NT1, NT2, ...) or the existing owner. It also adds the Mesa
references of the shapes no earlier reference draws
(`tools/mali/testdata/deqp_newtext_*`) and their pins
(`tests/mali_deqp_newtext.rs`). No compiler change.

## 1. Measure

- **Tip: `070608e37a20f6512a301d09f1e72088b0ee9a4c`** (main, which contains
  `349f989f` and train 2's `a27396fa0`).
- Cases: the 10,597 cases of the slices `2026-11-15-t6-gles2-S{1..4}-37305608`,
  as in sections 18 and 19 of `mali-deqp-families.md`.
- CTS: `93bca01861b0e3ef3c387027a9791e6d065f900c`, fetched at that commit
  alone, `external/fetch_sources.py` run, `tools/cts/cts-yakogl-target.patch`
  applied with `git apply`.
- Command (4 cores):

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$PWD/build/measure" --jobs 3 --chunk 100
```

- All 10,597 cases began. 106 chunk processes exited 1 (dEQP's "not every
  case passed") and none crashed. 4,701 cases meet a compile or link refusal
  (4,716 at `e9033663`).
- Rule: one text per case, the **first** refusal in `measure.tsv`'s
  `host_refusal` column (as in section 19). The refusing site was found by
  compiling each case's logged pair in-process through `tests/mali_ref`'s
  `compile_pair`, with a temporary line print at each `Err` of these variants.
  The print was not committed.
- **57 cases** have one of the nine texts as their first refusal. All 57 are
  Fail on the board; none is NotSupported. The item text says "about 45": the
  difference is `FragmentTooLarge`, which rose from 7 to 19. Train 2's
  `a27396fa0` lowered the texts that stopped these `uniform_api` struct cases
  first, so they now reach this text.

## 2. The table

"Covered" means an earlier reference already draws the refusing stage's
shader: the fragment shader for a fragment refusal, the vertex shader for a
vertex refusal, the pair for a link refusal. The comparison ignores comments
and whitespace. "New" counts the cases whose shape is drawn here (section 5).

| # | Text (`MaliError`) | Stage | Cases | Covered + new | dEQP groups | Refusing site | Step |
|--:|---|---|--:|---|---|---|---|
| 1 | `FragmentTooLarge` | fs | 19 | 10 + 9 | uniform_api.value 12, uniform_api.info_query 4, uniform_api.random 3 | `nir_fs.rs:10609` `Pairs::push` (`MAX_PAIRS` = `MAX_PAIR_NODES` = 255, `nir_fs.rs:53`) | **NT1** |
| 2 | link: varying offsets (`MALI_LAYOUTS_DIFFER`) | link | 14 | 0 + 14 | shaders.random 13, shaders.operator 1 | `link.rs:388` (`link_shaders_mali`, `differ` after `pair`) | **NT2** |
| 3 | `Defect(LaneCycle)` | fs | 11 | 10 + 1 | shaders.random 11 | `nir_fs.rs:12148` `pair_node` (via `lane_tree` / `vectorise`) | fix C, train 4 commit 5 (`mali-lowering-schedule.md` 5.5a) |
| 4 | `UboFallback` | vs | 5 | 1 + 4 | light_amount 5 | `fau.rs:204` `push_ubo` | **NT3** |
| 5 | `TexLanes` | fs | 3 | 0 + 3 | shaders.random 2, shaders.struct 1 | `nir_fs.rs:11276` `vectorise` (texel arm) | **NT4** |
| 6 | `TooManyValues` | vs | 2 | 0 + 2 (1 drawn) | uniform_api.random 2 | `nir.rs:6004` `walk` (`MAX_VALUES` = 2048, `nir.rs:65`) | **NT5** |
| 7 | `PrecisionUnknown` | fs | 1 | 0 + 1 | shaders.random 1 | `nir.rs:3620` `class_of` | **NT6** |
| 8 | `TexHighp` | fs | 1 | 0 + 1 | state_query.shader 1 | `mod.rs:1651` (`compile_stage`, the look-ups' sampler precision) | **NT7** |
| 9 | `Fp16Inexact` | fs | 1 | 0 + 1 | shaders.matrix 1 | `nir_fs.rs:10850` `vectorise` (`f16_rtne`, `select.rs:78`) | **NT8** |
| | **Total** | | **57** | **21 + 36** | | | |

None of the nine texts is near the plan's line of 100 cases. Together they
hold 57 of the 4,701 first refusals.

## 3. Per text: the shape, Mesa's words, the step

### 3.1 NT1, `FragmentTooLarge` (19)

Shape: a fragment shader that compares a uniform struct of `mat4`/`mat2`
arrays (`array_in_struct`, `struct_in_array`, `nested_structs_arrays`, all
`mat4_mat2`) or a large random `uniform_api` shader. Every component compare
is ANDed into one colour, so the fragment colour's pair graph needs more
than 255 nodes.

Refusing site: `Pairs::push` (`nir_fs.rs:10607-10609`). The pair index is a
`u8` (`push` returns `u8`, and the `P::Op` operands are `[u8; 3]`). Raising
`MAX_PAIR_NODES` therefore means widening the index to `u16` in `Pairs`, `P`
and `pair_node`, not only changing the constant. The `Table::FragPairNodes`
row (`mod.rs:844`) follows the constant.

Mesa: an ordinary fragment program of 128 to 153 words
(`deqp_newtext_ftl_*`). There is no limit of this kind in Mesa.

Cases covered by earlier references:

- `deqp_fsunif2_struct_mat_array`: `array_in_struct.mat4_mat2_fragment`,
  5 cases (info_query, `basic_array_assign_full`, `basic_array_assign_partial`,
  `by_pointer.get_uniform`, `by_pointer.render`).
- `deqp_fsunif2_struct_mat_nested`: `nested_structs_arrays.mat4_mat2_fragment`,
  3 cases (info_query, `by_pointer.get_uniform`, `by_pointer.render`).
- `deqp_fsunif2_random_13`: `uniform_api.random.13`.
- `deqp_fsunif3_random_81`: `uniform_api.random.81`.

Cases drawn here (9):

- `ftl_struct_in_array_both`: `struct_in_array.mat4_mat2_both`, 3 cases.
- `ftl_struct_in_array_fs`: `struct_in_array.mat4_mat2_fragment`, 3 cases.
- `ftl_nested_initial`: `value.initial.{get_uniform,render}.nested_structs_arrays.mat4_mat2_fragment`,
  2 cases.
- `ftl_random20`: `uniform_api.random.20`, 1 case.

Forecast (host): at most 19 cases leave the text. Which text each case meets
next is not known until NT1 lands: the walk stops at the first refusal.

### 3.2 NT2, link: "varying at different offsets" (14)

Shape: a `shaders.random.*.fragment.*` pair (and
`operator.sequence.side_effects.highp_vec4_fragment`) whose fragment shader
declares and textually reads several varyings. Every read except one or two
is dead: the colour that reads them is overwritten later (for example
`gl_FragColor = e * d * ...; ...; gl_FragColor = vec4(bool(c), c, ...)` in
`comparison_ops.fragment.17`). Each stage compiles alone. The link then finds
the read varying at offset 0 in the fragment shader and at a later offset in
the vertex shader, because the vertex shader still stores the dead outputs
first.

Refusing site: `link_shaders_mali` (`link.rs:353-422`). `pair` returns
`Pairing::Differ`, and `differ` writes `MALI_LAYOUTS_DIFFER` (`link.rs:388`).
The fragment shader is not rebuilt for the vertex order: the vertex shader's
order keeps the dead outputs, so there is nothing a fragment order can fix.
The vertex relink (`compile_vs_linked`, which `tests/mali_ref`'s
`compile_pair` calls) keeps every output that the fragment shader's varying
record lists. The record lists the dead reads too.

Mesa: `gl_nir_link_varyings` removes the outputs that the fragment shader
does not read once its dead loads are gone, and compacts the rest. In
`deqp_newtext_link_bexpr9`, for example, the vertex shader has seven varyings
and stores only `f`, as one `STORE.i32.estream` at offset 0 (`vs.varying` is
3 words). The fragment shader reads it with `LD_VAR_BUF_IMM ... index:0x0`.

The step: the fragment varying record lists only the varyings the
fragment code reads after its folds (or the link takes the read set from the
fragment program, not the record), and the vertex relink drops the other
outputs and packs the rest as Mesa does. The rule is the one the record
already uses for "an input no code reads has no slot" (`link.rs:469-477`),
carried over to the vertex side.

Forecast (host): 14. Both stages of every case compile today, so a link that
matches Mesa byte for byte makes all 14 compile and link on the host.

### 3.3 `Defect(LaneCycle)` (11), owned by fix C

Owner: fix C of `docs/notes/mali-deqp-defects.md` sections 3, 4 and 4.1,
which is commit 5 of train 4 (`mali-lowering-schedule.md` 5.5a). No NT step.
The cases fix C covers, and the earlier reference of each fragment shader:

- `scalar_conversion.fragment.96` and `.combined.96`: `deqp_defects_lanecycle_fs`
  (the same fragment shader).
- `basic_expression.fragment.58` and `.combined.58`: `deqp_kfold_c_expr58_fs`.
- `comparison_ops.fragment.13`: `deqp_intrnd_dead_bool_cmp_fs` and
  `deqp_misc2_algebraic_random13`.
- `comparison_ops.fragment.29`: `deqp_kfold_c_cmp29_fs`.
- `scalar_conversion.fragment.29` and `.combined.29`: `deqp_kfold_c_conv29_fs`.
- `scalar_conversion.fragment.74` and `.combined.74`: `deqp_kfold_c_conv74_fs`.
- `texture.fragment.83`: no earlier reference. It is drawn here as
  `deqp_newtext_lanecycle_tex83` (10 fragment words: the cycle next to a
  `TEX_SINGLE`).

### 3.4 NT3, `UboFallback` (5)

Shape: `light_amount.{10,12,14,16}omni` and `.16dir`. The vertex shader's
`uniform Light u_lights[N]` and the three matrices need more uniform words
than the FAU's 128.

Refusing site: `push_ubo` (`fau.rs:113-215`). It pushes ranges until one no
longer fits (`break`, as `bi_pick_ubo` does). A source whose word was not
pushed is then refused (`fau.rs:204`).

**Does Mesa push the rest to a UBO? Yes.** `bi_opt_push_ubo` pushes what fits
and keeps the other `load_ubo`s as loads from UBO 0. On v10 these are
`LD_PKA.i32` / `.i96` / `.i128` with the byte offset as a pushed FAU
immediate or a register. In `deqp_newtext_ubo_light_10omni`'s `vs.varying`
(276 words, all 128 FAU words used), for example:
`LD_PKA.i96.unsigned @r6:r7:r8, u63.w0, 0x0` and
`LD_PKA.i128.unsigned @r4:r5:r6:r7, u63.w1, 0x0`, where `u63` holds the offsets
`0x210` and `0x260`. `deqp_intval5_light_amount_0_16` (the `16omni` case) has
the same form.

**Does the driver support it? Yes, on both stages.**

- `emit_stage_uniforms` (`src/mali/frame.rs:3045-3095`) uploads the uniform
  block rows the shader loads as UBO 0.
- The vertex stage's resource table gets its UBO table (`frame.rs:3489`,
  `TABLE_UBO`), and so does the fragment stage's.
- The compiler already emits `LD_PKA` for a uniform array element read at a
  run-time index (`select.rs:1294-1311`, `:1388-1399`). The frame uploads the
  block when the program loads it (`UboLayout::loaded`, `ubo_mask` bit 0,
  `select.rs:539-542`).

What is missing is in the compiler only. `push_ubo` runs after selection, so
the step has to decide before register allocation which loads Mesa leaves
unpushed (the same `bi_pick_ubo` walk, done early). It then selects those as
`LD_PKA` of their range, and lists their rows in the meta's `ubo` rows so that
the frame uploads them.

Covered: `light_amount.16omni` (`deqp_intval5_light_amount_0_16`, pinned in
`tests/mali_deqp_intval5.rs`). Drawn here: `ubo_light_{10,12,14}omni` and
`ubo_light_16dir`.

Forecast (host): 5.

### 3.5 NT4, `TexLanes` (3)

Shape: a look-up's result read in another lane order, for example
`texture2D(a, b).qspt` (`shaders.random.texture.fragment.44`, `.55`), or a
swizzled look-up of a sampler in a nested struct
(`shaders.struct.uniform.sampler_nested_fragment`).

Refusing site: `vectorise`'s texel arm (`nir_fs.rs:11270-11276`). It admits
only a texel's own pairs in order, or swapped in an output word.

Mesa: a 16-bit `TEX_SINGLE`, then one `MKVEC.v2i16` per output word from the
texel halves (`deqp_newtext_texlanes_tex44`:
`MKVEC.v2i16 r2, r1.h1, r0.h0` and `MKVEC.v2i16 r1, r1^.h0, r0^.h1`). The
step adds a texel-lane leaf that reads any half of any texel word, built as
`MKVEC.v2i16`, as Mesa's `bi_lower_mkvec_swz` emits it.

Forecast (host): 3.

### 3.6 NT5, `TooManyValues` (2)

Shape: `uniform_api.random.3` and `.79`. These are vertex shaders with many
struct uniforms and samplers (28 look-ups in `.79`), more than
`MAX_VALUES` = 2048 IR values (`nir.rs:65`, refused at `nir.rs:6004`).

Mesa: a vertex program of 860 varying words and 825 fragment words
(`deqp_newtext_values_random79`).

The step: raise the IR tables (`MAX_VALUES`, and the 1536-instruction limit
the message names), with the `Table` rows that follow them. This is a table
size change only. It costs scratch memory on the Zephyr boards, and its own
commit should measure that.

`uniform_api.random.3` has **no reference**. Its sampler struct members are
of two types (`samplerCube m1[1]`, `sampler2D m2[1]`, ...). gl_ref.py does not
set struct members that are samplers, so with `--zero-uniforms` every member
reads unit 0, and Mesa refuses the draw (`GL_INVALID_OPERATION` in
`glDrawArrays`). Drawing it needs gl_ref.py to give struct sampler members
their own units, which is a tool change outside this item.

Forecast (host): 2.

### 3.7 NT6, `PrecisionUnknown` (1)

Shape: `shaders.random.trigonometric.fragment.51`, `asin(gl_FragColor - vec4(c, -8, b, a))`.
Here `a` is a `const int` folded from `ivec3(...).p`, and the colour is a sum
of folded `const float`s. A folded constant reaches the full-width built-in
with no precision class.

Refusing site: `class_of` (`nir.rs:3618-3622`).

Mesa: a mediump fragment program of 52 words (`deqp_newtext_precision_trig51`).
The operand is built at fp16 (`V2F32_TO_V2F16` of the varyings, `FADD.v2f16`
with the folded constants as fp16 immediates such as `0xBC00BC00`). The
`asin` itself is expanded at fp32 (`F16_TO_F32`, then an `FMA.f32`
polynomial and `FRSQ.f32`). So the folded constants are read at the
operand's mediump class. The step gives a folded constant the class of the
operation that reads it (docs/notes/mali-precision-frontend.md section 2.2),
not of the full-width built-in after it.

Forecast (host): 1.

### 3.8 NT7, `TexHighp` (1)

Shape: `state_query.shader.uniform_value_sampler`. The fragment shader has no
precision statement and reads `uniform highp sampler2D`:
`gl_FragColor = vec4(texture2D(s, vec2(0.0)).x)`.

Refusing site: `mod.rs:1645-1652`. A look-up through a highp sampler in a
shader that is not highp is refused.

Mesa: **the look-up here is 16-bit**:
`TEX_SINGLE.slot0.skip.reserved.16.r.2d.computed`, then
`IADD.v2u16 r0, r0^.h00, 0x0` broadcasts lane x
(`deqp_newtext_texhighp_sampler_value`, 11 words). The message's premise
("Mesa keeps its look-up 32-bit") does not hold for this shape: the result
feeds the mediump `gl_FragColor` only. The step admits a highp sampler whose
result is read only at mediump, which is what Mesa's `lower_precision` does.
It keeps the refusal for a result read at highp.

Forecast (host): 1.

### 3.9 NT8, `Fp16Inexact` (1)

Shape: `shaders.matrix.sub.const_highp_mat3_mat3_vertex`. The vertex shader
computes a constant `v_color` from two `const highp mat3`. Mesa's link moves
it into the fragment shader as a constant colour. Its x is a sum of constants
that should be zero but is not exactly zero in fp32: it lies below fp16's
normal range.

Refusing site: `vectorise` (`nir_fs.rs:10850`). `f16_rtne` (`select.rs:78`)
returns `None` for an exponent outside -14..15.

Mesa: `_mesa_float_to_half` makes it -0.0. The fragment program is the
constant colour, 7 words, with FAU `b8008000 3c003266` (x = `0x8000`,
y = -0.5, z ≈ 0.2, w = 1.0) (`deqp_newtext_fp16_sub_highp_mat3`). The step
makes `f16_rtne` follow `_mesa_float_to_half` below the normal range
(subnormal, then a signed zero), and keeps the refusal for values above it.

Forecast (host): 1.

## 4. Order of work

By cases, as plan #387 orders its work:

1. **NT1** `FragmentTooLarge`, 19 cases. The pair index is widened to `u16`.
2. **NT2** link dead varyings, 14 cases. This is the firmest forecast: both
   stages already compile.
3. (fix C, 11 cases, stays in train 4 as commit 5.)
4. **NT3** `UboFallback`, 5 cases.
5. **NT4** `TexLanes`, 3 cases.
6. **NT5** `TooManyValues`, 2 cases.
7. **NT6** `PrecisionUnknown`, **NT7** `TexHighp`, **NT8** `Fp16Inexact`,
   1 case each.

NT4 and NT8 edit `vectorise`, as fix C and the other train 4 `vectorise`
steps do. They should go after train 4's commit 5 so that each rebases on
the cycle fix. NT1 changes `Pairs` and `P`, which every `vectorise` step
reads, so it should land first or before train 4's `vectorise` run. NT2
(`link.rs`, the varying record), NT3 (`fau.rs`, `select.rs`) and NT5 (`nir.rs`
tables) share no function with the others.

## 5. The references (`tools/mali/testdata/deqp_newtext_*`)

30 references are written, one per shape that no earlier reference draws.
They cover 35 of the 36 new cases. The 36th, `uniform_api.random.3`, has no
reference (section 3.6). Each pair is verbatim from the `.qpa` that
`deqp-host-measure.sh` logged. Each was drawn with
`tools/mali/deqp_ref.sh <vert> <frag> tools/mali/testdata/deqp_newtext_<name> --instrument shim`
on Mesa 26.2.1's drm-shim, built with
`tools/mali/drm_shim_ref.sh build` into `$CONNECTED_DEPS_DIR/mesa-shim`. The
shim reproduced `gm2_cf_ifelse_fs` byte for byte before the draws (the
self-check of `README-deqp_ref.md`). Every draw uses `--uniform-values`,
except `texlanes_struct_sampler_nested` and `values_random79`, which use
`--zero-uniforms` because they have a sampler in a struct. Each `.nir.sha` is
from `tools/mali/nir_refs.sh 'deqp_newtext_*'` (30 written, 0 no draw). Each
`.vert` and `.frag` has a `.license` that names its dEQP case.

| Reference | dEQP case | Text | fs words |
|---|---|---|--:|
| `ftl_struct_in_array_both` | uniform_api.info_query.struct_in_array.mat4_mat2_both (+2) | FragmentTooLarge | 131 |
| `ftl_struct_in_array_fs` | uniform_api.info_query.struct_in_array.mat4_mat2_fragment (+2) | FragmentTooLarge | 128 |
| `ftl_nested_initial` | uniform_api.value.initial.get_uniform.nested_structs_arrays.mat4_mat2_fragment (+1) | FragmentTooLarge | 152 |
| `ftl_random20` | uniform_api.random.20 | FragmentTooLarge | 153 |
| `link_seq_side_highp_vec4` | shaders.operator.sequence.side_effects.highp_vec4_fragment | link | 25 |
| `link_bexpr9`, `link_bexpr53`, `link_bexpr85` | shaders.random.basic_expression.fragment.{9,53,85} | link | 11, 7, 12 |
| `link_cmp17`, `link_cmp41`, `link_cmp45` | shaders.random.comparison_ops.fragment.{17,41,45} | link | 9, 11, 9 |
| `link_cond41`, `link_cond47` | shaders.random.conditionals.fragment.{41,47} | link | 9, 9 |
| `link_conv17`, `link_conv32`, `link_conv53` | shaders.random.scalar_conversion.fragment.{17,32,53} | link | 9, 8, 7 |
| `link_swz17` | shaders.random.swizzle.fragment.17 | link | 9 |
| `link_tex4` | shaders.random.texture.fragment.4 | link | 19 |
| `lanecycle_tex83` | shaders.random.texture.fragment.83 | Defect(LaneCycle) | 10 |
| `ubo_light_10omni`, `_12omni`, `_14omni`, `_16dir` | light_amount.{10omni,12omni,14omni,16dir} | UboFallback | 8 each |
| `texlanes_tex44`, `texlanes_tex55` | shaders.random.texture.fragment.{44,55} | TexLanes | 11, 11 |
| `texlanes_struct_sampler_nested` | shaders.struct.uniform.sampler_nested_fragment | TexLanes | 20 |
| `values_random79` | uniform_api.random.79 | TooManyValues | 825 |
| `precision_trig51` | shaders.random.trigonometric.fragment.51 | PrecisionUnknown | 52 |
| `texhighp_sampler_value` | state_query.shader.uniform_value_sampler | TexHighp | 11 |
| `fp16_sub_highp_mat3` | shaders.matrix.sub.const_highp_mat3_mat3_vertex | Fp16Inexact | 7 |

The G310 needs no references of its own (`docs/notes/mali-g310-vs-g610.md`).
`tools/mali/g310_check.sh --since <rev>` repeats that check for these files.

## 6. The test (`tests/mali_deqp_newtext.rs`)

Features: `--no-default-features --features
host-test,gpu-mali,shaderc-online,no-shader-table`. The file follows
`tests/mali_deqp_intval5.rs`:

- One `#[ignore]`d byte-compare per reference (30).
- `refusals_pinned` checks the exact refusal text of each reference
  (`REFUSED`). The link refusals are pinned with the varying they name.
- `references_parse` checks the files, the `.license`s and the fragment word
  counts above.
- The byte-compare reads the uniform values gl_ref.py set, struct members
  included (`differences_members(.., true)`). The two `--zero-uniforms`
  references are compared with every uniform 0.

Run at the tip: 2 passed, 30 ignored. `cargo fmt --check` and `cargo clippy`
(feature set above) are clean for the file.

The gate's lists of `tests/mali_deqp_*.rs` in `tools/check.sh` do not name
this file yet, nor `mali_deqp_fmares`, `_highpfs`, `_kfold` or `_t3gap`.
Adding it is a one-line change to `tools/check.sh`, which is outside this
item.
