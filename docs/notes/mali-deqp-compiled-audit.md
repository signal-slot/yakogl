<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): are the once-refused dEQP shaders that now compile Mesa's words? An audit at `7b540fee`

**Status: audit** (plan #387, item `compiled-audit`; issues #20, #7).
HOST-ONLY. The audit itself changed nothing under `src/`, `tests/` or
`tools/`; its scratch harness, the sample and every reference lived under
`$TMPDIR` and are gone with the task; section 2 says how to make them again.
Item `audit-ref-2` (section 4.10) then drew a Mesa reference for 38 of the
shapes below (`tools/mali/testdata/deqp_audit_*`, pinned by
`tests/mali_deqp_audit.rs`); the rank table and the M1 table name them.

The project's rule is that a shape compiles to the words Mesa 26.2.1
generates, never "compiled, not Mesa's". The host cannot see rendering.
`docs/notes/mali-deqp-families.md` section 18.1 found 4,401 cases that the
board refused at `37305608` and that no longer met a refusal on the host at
`9ac31f30`. This note asks whether their shaders are now Mesa's words. For
any that are not, it asks where in the compiler the difference comes from.

## 0. Summary

- **Tip:** `7b540fee70f91c784120de3e1fca8cd8a9fa4421` (the integration tip;
  the host measure's `commit` file reads `7b540fee`).
- **Population:** the host measure gives 4,465 of the 10,597 board-refused
  cases with no code-generator refusal (6,132 still refused). 4,463 ran to a
  verdict (2 are the `NoResult` crashes of section 18.4). 4,428 of them log
  at least one program whose shaders compiled and whose link succeeded. That
  gives **3,588 distinct (vertex, fragment) source pairs**.
- **Sample:** **300 distinct pairs**, stratified by dEQP group (the first two
  name components after `functional`), weighted by case count, seed 387.
  Together they are the shaders of 1,017 cases.
- **Result:** **190 pairs byte-equal** to Mesa 26.2.1 (872 cases) and
  **108 differ** (138 cases). 2 have no reference, because `gl_ref.py`
  cannot draw a struct of samplers (section 6).
- **Population estimate:** weighting each dEQP group's equal/differ share
  by its cases, about **1,450 of the 4,428 compiled cases (33 %)** run a
  pair that is not Mesa's words, and about 2,950 run Mesa's.
- **Causes (section 4):** nine causes, ranked by a wrong value first,
  then estimated cases. One is a miscompile: `ivec(bvec)` in a mediump
  fragment shader reads lane x for lane y (`nir_fs.rs:12205-12210`,
  13 cases). The
  largest is vertex mediump/lowp arithmetic kept at 32 bits where Mesa
  narrows it to fp16 (50 of the 108 pairs, about 710 cases). The others are
  fragment compares at fp16 where Mesa keeps fp32; instruction order;
  attribute-load order; varying layout and load width; Mesa's extra fp16 in
  the fragment shader; a set of one-off forms; and one FAU-word difference.
- **Board forecast (section 5):** the 13 miscompiled cases are expected to
  Fail. Apart from them, the differing pairs compute Mesa's values up to
  rounding (fp16 versus fp32, a fused or reassociated operation, one
  `-0.0`/NaN case, one 1-ulp constant). Most differ in form only (order,
  registers, layout, width). Of the rest, the fp16-versus-fp32 compares are
  the ones most likely to Fail, where a near-threshold compare can flip.
  The board rerun (item `board-rerun-1`) decides.

## 1. Inputs

- The library: `tools/mali/deqp-host-measure.sh` at the tip (section 1 of
  `mali-deqp-families.md`), with VK-GL-CTS
  `93bca01861b0e3ef3c387027a9791e6d065f900c` in
  `$CONNECTED_DEPS_DIR/VK-GL-CTS`. It was fetched here the documented way
  (`tools/cts/deps.toml` `DEQP_WORK`: the pinned commit, shallow, then
  `external/fetch_sources.py`), with `cts-yakogl-target.patch` applied,
  uncommitted.
- The case list: the slices `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608`
  (10,597 cases whose board log holds a code-generator refusal), as in
  section 18.
- Mesa 26.2.1: the drm-shim build of `tools/mali/drm_shim_ref.sh build`, from
  the tarball of `third_party/mesa-26.2.1/UPSTREAM` (sha256 checked), in
  `$CONNECTED_DEPS_DIR/mesa-shim`. Before the audit it was checked
  against two committed references, byte for byte (`cmp` of the
  `expect.json`): `gm2_cf_ifelse_fs` and `deqp_negzero_constglobal_fs`.

## 2. Method

1. **Measure.**

   ```sh
   tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
       --out "$TMPDIR/measure" --jobs 3 --chunk 100
   ```

   All 10,597 cases began; 106 processes exited 1 and 2 exited 139 (the
   two `scalar_conversion.*.96` crashes of section 18.4, still present).
   The run reports 6,132 cases with a host refusal (6,196 at `9ac31f30`).
2. **Pairs.** A scratch parser reads every `c_*.qpa`, with the measure's
   own `RE_UNIT` regular expression extended to keep `CompileStatus` and
   `LinkStatus`. Its input is each case of `measure.tsv` with no host
   family and a status other than `NotRun` or `NoResult`. It keeps every
   program of the case with both sources, `CompileStatus="OK"` on both
   shaders, `LinkStatus="OK"`, and no refusal text in any of its logs. The
   kept programs are deduplicated by the exact text of the pair. Each
   distinct pair takes the dEQP group of most of its cases.
3. **Sample.** Each group's quota is `max(1, round(330 · cases / 4,428))`,
   capped at its number of pairs (330 because the caps bring the total down
   to 300). Within a group, pairs are drawn without replacement, weighted by
   the number of cases that run them (Python `random.Random(387)`).
4. **Mesa's words.** For each pair:
   `tools/mali/deqp_ref.sh <pair>.vert <pair>.frag <out> --instrument shim`.
   This is the drm-shim draw with `--uniform-values`, then
   `pan_ref_diff.py extract --draw`: the byte reference of
   `README-deqp_ref.md`, the same for every pair whatever its precision.
   It fails for 6 pairs with struct uniforms ("--uniform-values does not
   set a Struct uniform"). Four of them were drawn as
   `deqp_unitype_vsstruct_*` are drawn: `drm_shim_ref.sh run` without
   `--uniform-values`, then `extract --draw`. Every uniform is 0 there, and
   `mali_ref::differences` sees the struct member and compares at 0. The
   other two (`uniform_api.info_query.basic_struct.sampler2D_samplerCube_vertex`
   and `unused_uniforms.sampler2D_samplerCube_vertex`, a struct of samplers)
   fail in Mesa's draw with `GL_INVALID_OPERATION`, because gl_ref.py binds
   no sampler member. They have no reference.
5. **Ours, and the compare.** A scratch test in a throwaway clone of the
   tip (`tests/zz_audit.rs`, never in this tree) runs, for each pair,
   `mali_ref::compile_pair` (what glCompileShader and glLinkProgram do, as
   in `tests/mali_deqp_negzero.rs`). It then runs `mali_ref::differences`
   against the reference and prints each difference with both
   disassemblies. The feature set is
   `--no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table`.
   The same harness compiled all 3,588 pairs once: every one compiles
   through `compile_pair`.
6. **Causes.** Each differing pair is tagged by what differs, per region:
   - fp16 operations Mesa has and ours lacks, or the reverse;
   - the same opcode multiset in another order;
   - `LD_ATTR_IMM` order;
   - `LD_VAR_BUF_IMM` offset or width;
   - FAU words only.

   It is then given the first tag in that order of priority. One or more
   representatives of each cause were read in full. The compiler site
   comes from reading `src/shaderc/online/mali/` at the tip (section 4).
7. **Estimate.** For each dEQP group, the share of the sampled cases whose
   pair differs (weighted by cases, pairs without a reference left out) is
   multiplied by the group's compiled cases. The same is done per cause.

## 3. Results

| | Pairs | Cases they cover |
|---|--:|--:|
| sampled | 300 | 1,017 |
| byte-equal to Mesa 26.2.1 | **190** | 872 |
| differ | **108** | 138 |
| no reference (struct of samplers) | 2 | 7 |

By dEQP group (sampled pairs; the estimate is of the group's compiled
cases whose pair differs):

| dEQP group | compiled cases | distinct pairs | sampled | equal | differ | est. differing cases |
|---|--:|--:|--:|--:|--:|--:|
| shaders.operator | 1,384 | 1,384 | 103 | 64 | 39 | 524 |
| shaders.swizzles | 414 | 414 | 31 | 16 | 15 | 200 |
| vertex_arrays.single_attribute | 368 | 3 | 3 | 3 | 0 | 0 |
| shaders.conversions | 301 | 301 | 22 | 6 | 16 | 219 |
| shaders.constant_expressions | 260 | 260 | 19 | 17 | 2 | 27 |
| shaders.preprocessor | 256 | 254 | 19 | 18 | 1 | 13 |
| shaders.random | 252 | 238 | 19 | 10 | 9 | 115 |
| vertex_arrays.multiple_attributes | 203 | 7 | 7 | 7 | 0 | 0 |
| uniform_api.value | 190 | 65 | 14 | 4 | 9 | 128 |
| shaders.matrix | 138 | 138 | 10 | 4 | 6 | 83 |
| shaders.loops | 119 | 116 | 9 | 9 | 0 | 0 |
| shaders.functions | 118 | 118 | 9 | 5 | 4 | 52 |
| shaders.indexing | 87 | 87 | 6 | 4 | 2 | 29 |
| draw.random | 51 | 4 | 4 | 4 | 0 | 0 |
| shaders.constants | 49 | 49 | 4 | 4 | 0 | 0 |
| uniform_api.info_query | 34 | 2 | 2 | 0 | 1 | 34 |
| shaders.linkage | 31 | 31 | 2 | 1 | 1 | 16 |
| shaders.scoping | 30 | 30 | 2 | 2 | 0 | 0 |
| draw.draw_elements | 26 | 1 | 1 | 1 | 0 | 0 |
| draw.draw_arrays | 24 | 0 (their pairs count under draw.random) | 0 | | | |
| fragment_ops.interaction | 16 | 16 | 1 | 1 | 0 | 0 |
| uniform_api.random | 14 | 14 | 1 | 1 | 0 | 0 |
| shaders.builtin_variable | 11 | 11 | 1 | 1 | 0 | 0 |
| attribute_location.bind_aliasing | 10 | 10 | 1 | 0 | 1 | 10 |
| shaders.conditionals, default_vertex_attrib.float, shaders.struct, shaders.return, shaders.qualification_order, shaders.function, shaders.misc, attribute_location.bind | 38 | 31 | 8 | 8 | 0 | 0 |
| attribute_location.bind_max_attributes | 3 | 3 | 1 | 0 | 1 | 3 |
| shaders.fragdata | 1 | 1 | 1 | 0 | 1 | 1 |
| **total** | **4,428** | **3,588** | **300** | **190** | **108** | **≈1,454** |

Every sampled pair of `shaders.loops`, `vertex_arrays.*`, `draw.*`,
`shaders.constants` and `shaders.scoping` is Mesa's. The groups with the
largest share of differing pairs are `shaders.conversions` (16 of 22),
`uniform_api.value` (9 of 13 with a reference), `shaders.matrix` (6 of 10)
and `shaders.swizzles` (15 of 31). The stage that differs:
- 57 pairs differ in the vertex stage only;
- 38 in the fragment stage only;
- 13 in both.

The two `other` pairs (a different secondary offset, and a varying program
where Mesa has none) are counted with their stages.

## 4. The differing causes, ranked as fix candidates

Each pair is counted once, under its primary cause (the first tag of
section 2, step 6); the tags it also carries are listed. "Est. cases" is the
section 2 step 7 estimate over the 4,428 compiled cases. Sites are
`file:line` under `src/shaderc/online/mali/` at `7b540fee`; a site marked
*(inferred)* was read, not run. The rank weighs a wrong value first, then
the estimated cases.

| Rank | Cause | Sampled pairs (cases) | Est. cases | Value can differ from Mesa's | Site | References (`tools/mali/testdata/`, tests in `tests/mali_deqp_audit.rs`) |
|--:|---|--:|--:|---|---|---|
| 1 | D1 `b2i32` of a 16-bit mask reads the low lane for every lane | 1 (1); 13 (13) over its whole shape | 13 | **yes, wrong value** | `nir_fs.rs:12205-12210` | `deqp_audit_d1_conv_bvec2_int_to_ivec3_fs`, `_d1_conv_bool_bool_to_ivec2_fs`, `_d1_conv_bvec3_to_ivec3_fs`, `_d1_conv_bvec4_to_ivec2_fs`, `_d1_conv_vec2_bvec2_to_ivec4_fs` |
| 2 | V1 vertex mediump/lowp arithmetic at fp32, Mesa fp16 | 50 (76) | ≈710 | rounding only (ours is more precise) | `nir.rs:4619-4642` | `deqp_audit_v1_conv_bool_to_bool_vs`, `_v1_swz_mediump_bvec2_grg_vs`, `_v1_op_postinc_lowp_int_vs`, `_v1_op_sqrt_mediump_vec4_vs`, `_v1_matrix_div_dynamic_mediump_mat3_vs`, `_v1_uniform_initial_float_vs`, `_v1_uniform_info_float_both` |
| 3 | F1 fragment compare of two fp32 varying lanes built at fp16, Mesa fp32 | 17 (19) | ≈228 | a compare of values within one fp16 step | `nir_fs.rs:5950-6005`, `:11538-11641` | `deqp_audit_f1_op_greaterthan_lowp_vec4_fs`, `_f1_op_equal_mediump_vec4_fs`, `_f1_swz_mediump_vec3_rb_fs`, `_f1_swz_mediump_vec3_sp_fs`, `_f1_swz_mediump_vec3_x_fs`, `_f1_swz_mediump_vec4_wwwy_fs`, `_f1_op_distance_mediump_float_fs`, `_f1_conv_vec3_float_to_mat2_fs`, `_f1_conv_float_to_mat3_fs`, `_f1_uniform_info_mat4_fs` |
| 4 | S1 same opcodes, Mesa's instruction order not reproduced | 12 (12) | ≈162 | no | `nir_fs.rs:321-614`, `:11597` | `deqp_audit_s1_swz_mediump_vec4_yyyy_fs`, `_s1_matrix_add_dynamic_lowp_mat2_vs` |
| 5 | A1 vertex attribute loads in another order | 11 (11) | ≈135 | no | `nir.rs:6190-6233` | `deqp_audit_a1_op_greaterthan_highp_ivec2_vs`, `_a1_op_greater_or_equal_highp_float_vs` |
| 6 | M1 one-off folds and forms Mesa's algebraic rules give | 8 (10) | ≈89 | rounding, `-0.0`/NaN for some | section 4.6 | `deqp_audit_m1_cexpr_greaterthan_ivec2_vs`, `_m1_matrix_mul_uniform_highp_mat2_vs`, `_m1_uniform_initial_mat4_vs`; the `shaders.random.*` ones: none (section 4.10) |
| 7 | L1 varying load width, order or a varying Mesa does not keep | 5 (5), and the tag on 15 more | ≈64 | no | `nir_fs.rs:7544`, `:8089-8098`, `:4331-4336`; `nir.rs:3091-3104` | `deqp_audit_l1_op_mul_highp_int_ivec2_fs`, `_l1_op_greaterthan_mediump_ivec2_fs`, `_l1_matrix_sub_uniform_mediump_mat3_vs` |
| 8 | R1 Mesa's fragment shader keeps fp16 operations ours fuses or orders otherwise | 4 (4) | ≈53 | rounding of a fused FMA | `vec16.rs:1003-1025` | `deqp_audit_r1_idx_vec2_static_loop_write_component_read_fs`, `_r1_swz_mediump_vec4_xyzw_vs`, `_r1_swz_mediump_vec4_qpts_vs`; `random.conditionals.fragment.59`: none (section 4.10) |
| 9 | K1 a constant folded with one rounding more (FAU word, 1 ulp) | 1 (1) | ≈14 | 1 ulp | `nir_fs.rs:3375-3378`, `nir.rs:2205`, `:2212` | `deqp_audit_k1_cexpr_length_vec3_fs` |

### 4.1 D1: `b2i32` of a 16-bit mask widens the wrong lane (a miscompile)

- **What differs.** `shaders.conversions.vector_combine.bvec2_int_to_ivec3_fragment`
  (`bvec2 in0 = bvec2(v_in0); out0 = ivec3(in0, in1); out0 == ref_out0`):
  - Mesa widens lane x with `IADD.v2u16 r0, r0^.h00, 0x0`, lane y with
    `IADD.v2u16 r2, r0.h11, 0x0`, and gives each lane its own `CSEL` and
    `ICMP_OR` against `u2.w0` / `u2.w1`.
  - Ours widens `.h00` once and compares that one value against both
    `u2.w1` and `u2.w0`.

  So `in0.y` is read as `in0.x`.
- **Site.** `nir_fs.rs:12205-12210`: the `Alu::B2i` arm for a 32-bit
  `b2i32` of a 16-bit mask matches `Val::Lane(c, _)` and builds
  `Vec2(s[0].swz(Swz::H00), …)`. It ignores the lane, so the second
  component is the first component's half. The row CSE then merges the two
  lanes. No `MaliError` gate covers it.
- **Cases.** Every compiled pair whose fragment shader has a `bvec`/`bool`
  and an `int`/`ivec` at mediump or lowp was checked: 270 pairs, each
  against its own Mesa reference. 198 are equal and 71 differ. **13 show
  this defect** (fewer `.h11` widenings than Mesa), all `shaders.conversions`:
  - `vector_combine.`:
    - `bool_bool_bool_bool_to_ivec4`, `bool_bool_bool_to_ivec3`,
      `bool_bool_to_ivec2`;
    - `bvec2_bvec2_to_ivec4`, `bvec2_float_to_ivec3`, `bvec2_int_to_ivec3`,
      `bvec3_float_to_ivec4`, `vec2_bvec2_to_ivec4`.
  - `vector_to_vector.`: `bvec2_to_ivec2`, `bvec3_to_ivec2`, `bvec3_to_ivec3`,
    `bvec4_to_ivec2`, `bvec4_to_ivec4`.

  All 13 are `_fragment` cases.
- **Fix candidate:** take the lane's own half (`H11` for an odd lane).
  Mesa's words for the shape are the sampled pair's reference.
- **Fixed (issue #11306).** The `b2i32` arm widens the half the lane's
  source names. Four of the five D1 references are byte-equal now
  (`LOWERED` in `tests/mali_deqp_audit.rs`). Of the issue's 10 `to ivec`
  cases, 9 pairs are byte-equal; `bvec3_float_to_ivec4` and the fifth
  reference (`d1_conv_bool_bool_to_ivec2_fs`) still differ in form only:
  Mesa narrows and compares two varying lanes as one pair, ours one at a
  time, each lane on its own mask.

### 4.2 V1: vertex mediump/lowp arithmetic at fp32 (Mesa narrows to fp16)

- **What differs.** Mesa's `lower_precision` makes the mediump and lowp
  operations of a vertex shader 16-bit:
  - `FADD.f32 rX.h0, x, -0` or `V2F32_TO_V2F16` to narrow;
  - `FCMP_OR.v2f16` then `S16_TO_S32` + `CSEL` for `bool()` and compares;
  - `FMA.v2f16`, `FRCP.f16`, `FRSQ.f16`;
  - `F32_TO_S32`/`S32_TO_F32` for a lowp `int(float)`;
  - `F16_TO_F32` before the store.

  Ours computes all of these at fp32: `FCMP_OR.f32`, `FMA.f32`,
  `FROUND.rtz`, `FRSQ.f32`.
- **Sites.**
  - `nir.rs:4619-4642`: the catch-all `IrOp::Alu(a, s)` arm of the
    vertex walk widens every operand (`vs_widen`, `nir.rs:3029`) and never
    reads `Prec` again.
  - The 16-bit arms are only `vs_alu16` (`nir.rs:3135`, taken at
    `:4607-4611` for `Fmul`/`Fadd` whose lanes are all constants or 16-bit,
    `vs_pairs16`, `:3110`) and `vs_alu16u` (`nir.rs:3321`, `:4614-4618`, fp16
    uniforms).
  - Per form:
    - `bool(x)`: `F2b` → `Fcmp(Ne)`, `nir.rs:4909-4913`;
    - compares: `nir.rs:4900-4901`, `:4923`, `:4926`;
    - `sqrt`/`inversesqrt`: `nir.rs:4884-4889`;
    - `i2f(f2i(a))` → `Ftrunc`: `nir.rs:4977-4978`. It also skips the
      `I2f` narrowing at the mediump-local store, `nir.rs:4534`;
    - an attribute stored to a mediump local is kept fp32: `nir.rs:4536`.
  - A mediump vertex uniform is read as an fp16 half (`LoadUniform`,
    `nir.rs:4485-4495`, `fp16_uniform` `nir.rs:1145`, `select.rs:963-970`)
    and widened at once. The compare that reads it is fp32, where Mesa's is
    `FCMP_OR.v2f16` against an fp16 constant (`uniform_api.*`).
- **Gate.** None. `MaliError::VertexMediump` and `VsFp16Operation` were
  only ever on unlanded branches (`docs/notes/mali-precision.md`, "On the
  unlanded branches"). `UniformPrecision` (`mod.rs:1363-1364`) checks a
  uniform's storage, not the operations on it.
- **Cases (sample).**

  | dEQP group | Pairs |
  |---|--:|
  | shaders.operator | 18 |
  | shaders.conversions | 8 |
  | shaders.swizzles | 7 |
  | uniform_api.value | 6 |
  | shaders.matrix | 3 |
  | shaders.random | 3 |
  | attribute_location.bind_aliasing | 1 |
  | attribute_location.bind_max_attributes | 1 |
  | shaders.functions | 1 |
  | shaders.indexing | 1 |
  | uniform_api.info_query | 1 |

  Examples:
  - `conversions.scalar_to_scalar.bool_to_bool_vertex`;
  - `swizzles.vector_swizzles.mediump_bvec2_grg_vertex`;
  - `operator.unary_operator.post_increment_result.lowp_int_vertex`;
  - `operator.exponential.sqrt.mediump_vec4_vertex`;
  - `matrix.div.dynamic_mediump_mat3_mat3_vertex`;
  - `uniform_api.info_query.basic.float_both`.

  `highp_*_vertex` operator cases are here too: their
  `mediump vec4 color = … * 0.1 + 0.5` tail is mediump. 17 of the 50 also
  load their attributes in another order (A1).
- **Owner.** `docs/notes/mali-intval-vs16-lowering.md` covers part:
  - M1, session vs16-1: the compares `Flt`/`Fle`/`Feq`/`Fne`;
  - M2: the `S16_TO_S32` + `CSEL` widening;
  - M4, vs16-3: the fp16-uniform compare of `uniform_api`.

  It does **not** name `F2b` (`bool()` of a mediump float), the
  `F32_TO_S32`/`S32_TO_F32` of a lowp `int()`, the 16-bit
  `FRSQ`/`FRCP`, or mediump arithmetic on fp32 attribute lanes in general.
  The largest part of this cause has no owner.

### 4.3 F1: a fragment compare of two varying lanes built at fp16 (Mesa: fp32)

- **What differs.** `operator.float_compare.greaterThan.lowp_vec4_fragment`
  and the other `float_compare.*` / `binary_operator.equal.mediump_vec4_fragment`
  cases:
  - Ours converts the varyings with `V2F32_TO_V2F16` and compares with
    four one-lane `FCMP_OR.v2f16.lt.f1`.
  - Mesa compares the loaded fp32 lanes, `FCMP_OR.f32.lt.m1`, and builds
    the colour with `MUX.v2i16`.

  Mesa's late rule `flt(f2fmp(a), f2fmp(b)) → flt(a, b)`
  (`nir_opt_algebraic.py`) keeps a compare of two unconverted varying lanes
  at fp32. The project's notes already say so:
  `docs/notes/mali-compare-forms.md` ("the mediump compare of two varyings
  is a 32-bit `FCMP_OR.f32`") and `mali-mixed-precision.md`.
- **Sites.**
  - The mediump `Flt`/`Fle`/`Feq` arm `nir_fs.rs:5950-6005` builds every
    compare with `f.node(Alu::Fcmp(..))`, a 16-bit node (`nir_fs.rs:1366-1374`),
    even over raw fp32 varying lanes.
  - `bool_colour` (`nir_fs.rs:11366-11423`) accepts it.
  - `convert_inputs16` (`nir_fs.rs:11538-11641`, called from `mod.rs:1531-1539`)
    then inserts the `F2f16` of each lane.
  - The rule is already applied to `discard`: `Fs::discard`,
    `nir_fs.rs:1845-1853`, refuses the same compare with `FsOperandForm`.
- **Other forms in this cause.**
  - `swizzles.vector_swizzles.mediump_vec3_rb_fragment`, `_sp_`, `_x_`,
    `_vec4_wwwy_`: `all(lessThanEqual(abs(a-b), eps*abs(b)+eps))`. Ours
    leaves two one-lane `FCMP_OR.v2f16` where Mesa builds one vector
    compare from a `MKVEC.v2i16`. `vectorize16`'s `same` check,
    `nir_fs.rs:11076-11080`, pairs only identical sources *(inferred)*.
  - `operator.geometric.distance.mediump_float_fragment`: ours lowers
    `distance(float, float)` to `sqrt(x*x)`, which is `FMA` + `FRSQ.f16` +
    `FRCP.f16` (`builtin_call.rs:882-892`, `nir_fs.rs:6336-6355`). Mesa has
    `FADD.v2f16 .abs`, `abs(a-b)` for one component *(where Mesa does
    it is inferred)*.
  - `conversions.matrix_combine.vec3_float_to_mat2_fragment`,
    `scalar_to_matrix.float_to_mat3_fragment`,
    `uniform_api.info_query.basic.mat4_fragment`: ours lacks Mesa's
    `iand(fge(e,a), fge(e,b)) → fge(e, fmax(a,b))`. Only `and_masks`,
    `nir_fs.rs:2128-2139`, exists, and the `Ball` arm, `:6158-6187`, does
    not call it *(inferred)*.
- **Cases (sample).** 17 pairs:
  - 9 `shaders.operator`, 4 `shaders.swizzles`, 2 `shaders.conversions`,
    1 `shaders.linkage`, 1 `uniform_api.value`;
  - 9 of them also read a varying at another order or width (L1).

### 4.4 S1: the same instructions in another order

- **What differs.** The same opcode multiset with other registers. For
  example `swizzles.vector_swizzles.mediump_vec4_yyyy_fragment`, `isOk`
  `all(lessThanEqual(abs(a-b), eps*abs(b)+eps))`: Mesa computes `abs(b)`
  and the `FMA` first, then the subtractions, and ours the reverse. Also
  the `conversions.matrix_combine.*_to_mat2_vertex` fragment shaders, where
  a `V2F32_TO_V2F16` stands with its load in ours and before its `FADD` in
  Mesa.
- **Sites.**
  - `order_keys`, `nir_fs.rs:321-524`: the statement, then the IR index.
  - `swapped_compares`, `nir_fs.rs:551-614`: it skips a compare of more
    than one component (`:564`), so a vector `lessThanEqual` keeps its
    operands in source order. Its comment cites `deqp_varylink_swz_xx` for
    that.
  - `convert_inputs16` gives each conversion its load's key, at
    `nir_fs.rs:11597`.
  - The pressure scheduler (`sched.rs:280-341`) keeps the order when the
    pressures tie.

  Mesa's order looks like the built-in `lessThanEqual(x, y)` becoming
  `fge(y, x)` with both arguments grafted *(inferred)*. That would
  contradict the `swz_xx` measurement, so the rule needs Mesa's per-pass
  NIR (`drm_shim_ref.sh run` with `NIR_DEBUG=print_fs`) before it is coded.
- **Cases (sample).** 12 pairs:
  - 5 `shaders.conversions`, 3 `shaders.functions`, 2 `shaders.swizzles`,
    1 `shaders.matrix`, 1 `shaders.preprocessor`;
  - one of them, `matrix.add.dynamic_lowp_mat2_mat2_vertex`, is a vertex
    shader.

### 4.5 A1: vertex attribute loads in another order

- **What differs.** The `LD_ATTR_IMM` words, same indices, in another
  order. In `operator.int_compare.greaterThan.highp_ivec2_vertex` ours
  loads index 1 (`v4`) first and Mesa index 2 (`v2`) first. Also
  `int_compare.*_ivec4_vertex` and `binary_operator.greater_or_equal.highp_float_vertex`.
  The registers that follow differ with them.
- **Site.** `opt_move` (`nir.rs:6144`, called at `:5632`) puts each load
  just before its first user. `move_before_users` (`nir.rs:6190-6233`):
  when two loads share a user, the one already in place stays and the
  other lands after it (`:6214-6221`). The `greaterThan` → `Flt(y, x)` swap
  (`builtin_call.rs:1962-1981`) makes Mesa convert `in1` first *(inferred)*.
  Whether Mesa runs `nir_opt_move` twice on the vertex stage was not
  checked.
- **Cases (sample).**
  - 11 pairs as the primary cause (10 `shaders.operator`,
    1 `shaders.fragdata`), and the tag on 17 more (V1).
  - The value does not change. A fix needs the same per-pass NIR as S1.

### 4.6 M1: one-off forms (8 pairs)

| Example (`dEQP-GLES2.functional.`) | What differs | Value | Ours (site) | Mesa's rule | Reference |
|---|---|---|---|---|---|
| `shaders.random.conditionals.vertex.42` | `FADD_IMM -7.0` then `FADD_IMM +2.0`; Mesa one `FADD_IMM -5.0` | fp32 rounding of `(h-7)+2` vs `h-5` | `vs_fadd`, `nir.rs:2663-2682` (no constant reassociation through the `v_color` variable; `reassociate_constant`, `glsl.rs:1428`, cannot see it) | `op(op(a,#b),#c) → op(a, op(b,c))` | none (a `rsg` random shader, section 4.10) |
| `shaders.matrix.mul.uniform_highp_mat2_mat2_vertex` | two FMA chains and an `FADD`; Mesa one chain of four FMAs | rounding order | `dot_addend`, `nir.rs:5896-5899` ("not taken") | `fadd(ffma(a,b,fmul(c,d)), e) → ffma(a,b,ffma(c,d,e))` | `deqp_audit_m1_matrix_mul_uniform_highp_mat2_vs` |
| `shaders.random.conditionals.fragment.10` | `FMA` by the FAU pair `(0, -1)`; Mesa folds lane x to `+0.0` | `0*x`: `-0.0`/NaN vs `+0.0` | per-pair zero rule, `nir_fs.rs:5891-5904` (needs both lanes zero) | `fmul(a, 0.0) → 0.0` per lane | none (`rsg`) |
| `shaders.random.comparison_ops.fragment.21` | `vec2(-(f*0.25), 1.0)` as `FMA` by `(-1, 1)`; Mesa `FADD .neg` + `MKVEC` | no | `nir_fs.rs:8526-8536` (the late `vec2(fneg a, b) → fmul` rule applied unconditionally) | rule gated by `is_only_used_as_float` *(inferred)* | none (`rsg`) |
| `shaders.constant_expressions.builtin_functions.vector_relational.greaterThan_ivec2_vertex` | `LSHIFT_OR .not` + `MUX(0, 1.0, ~b)`; Mesa `MOV` + `MUX(1.0, 0, b)` | no | fragment `B2f` arm, `nir_fs.rs:2331`, `int_lane` `:2371-2377` (the vertex walk has the rule, `nir.rs:4915-4919`) | `b2f(inot a) → bcsel(a, 0.0, 1.0)` | `deqp_audit_m1_cexpr_greaterthan_ivec2_vs` |
| `shaders.conversions.vector_combine.bvec2_int_to_ivec3_fragment` | D1, section 4.1 | **wrong** | `nir_fs.rs:12205-12210` | | `deqp_audit_d1_conv_bvec2_int_to_ivec3_fs` |
| `uniform_api.value.initial.get_uniform.basic.float_vertex`, `.mat4_vertex` | fp32 compare of a widened fp16 uniform; Mesa `FCMP_OR.v2f16` against `2a66` | fp16 vs fp32 compare | `nir.rs:4923` (V1) | `lower_precision` | `deqp_audit_v1_uniform_initial_float_vs`, `deqp_audit_m1_uniform_initial_mat4_vs` |

Three more pairs carry an `other` tag (counted under V1 and F1):
- `shaders.fragdata.valid_dynamic_index`: Mesa blends `gl_FragData[i]`
  into all 8 targets and ours into one. Target 0 is the same, and
  GLES 2.0 binds no other. Ours makes `gl_FragData` one element at slot 0
  (`sema.rs:581`, `lower.rs:1495-1523`). `FragOutput` is never raised for a
  run-time index (`nir_fs.rs:7862`).
- `shaders.linkage.uniform_struct_vec4`: a varying computed from
  uniforms only, which Mesa's link moves into the fragment shader (no
  varying program). Ours stores it and loads it flat (`nir.rs:5467`).
  The fragment shader also keeps `(s+d)-d`, which Mesa cancels; the
  vertex walk's `cancelled_sum` (`nir.rs:2293`) has no fragment
  counterpart.
- `attribute_location.bind_max_attributes.vec3`: V1 at 16 attributes,
  with a different secondary offset.

### 4.7 L1: varying load width, order, or a varying Mesa does not keep

- **What differs.**
  - `operator.binary_operator.mul.highp_int_ivec2_fragment`: ours
    `LD_VAR_BUF_IMM .v3`, Mesa `.v4` (it keeps the varying whole). The
    vertex shader stores another lane set.
  - `operator.int_compare.greaterThan.mediump_ivec2_fragment`: the same
    layout; Mesa issues the load at `0x10` first.
  - `matrix.sub.uniform_mediump_mat3_mat3_vertex`: ours folds the
    constant `1.0` lane into the fragment shader (flat `.v3`), where Mesa
    keeps it in the flat `.v4`.
- **Sites.**
  - The width is the kept lanes, `nir_fs.rs:8089-8098`. `kept_whole`
    returns false at once for a highp varying (`nir_fs.rs:7544`), before
    the `vector_int` rule (`:7584-7587`) that would keep `ivec2(v.wy)`'s
    varying whole.
  - The load order is `var_key`, the first IR read (`nir_fs.rs:4331-4336`);
    Mesa places each load at its first use *(inferred)*.
  - The constant lane: `uniform_sum16` (`nir.rs:3091-3104`) accepts
    `Uniform`/`Fadd` but not `Fadd(a, Fneg(b))`, so `computed` is false
    (`nir.rs:4578-4582`).
  - The slot order itself is `generic_layout`, `select.rs:151-186`
    (Mesa's comparator, `mali-partial-varyings.md`); no sampled pair
    traced to it.
- **Cases (sample).** 5 pairs as the primary cause (2 `shaders.operator`,
  2 `shaders.random`, 1 `shaders.matrix`), and the tag on 15 more (9 F1, 3 V1, 3 A1).

### 4.8 R1: Mesa's fragment shader keeps fp16 operations ours fuses or orders otherwise

- **What differs.**
  - `indexing.vector_subscript.vec2_static_loop_subscript_write_component_read_fragment`:
    ours fuses `tmp.y + tmp.x` into one `FMA.v2f16`; Mesa keeps the
    product (`FMA …, -0`), builds `tmp` with `MUX.v2i16`, then `FADD.v2f16`.
  - `random.conditionals.fragment.59`: Mesa `FADD.v2f16` of a uniform
    where ours folds to `FADD_IMM`.
  - `swizzles.mediump_vec4_xyzw_vertex` and `_qpts_vertex` (fragment
    `isOk`): Mesa does the `.zw` pair first and puts `ATEST` after the
    colour `MKVEC`.
- **Site.** `fuse`, `vec16.rs:1003-1025` (from `vec16.rs:512`). Ours fuses
  where Mesa's product feeds the vector insert, not the `fadd`
  *(inferred)*. The orders are `nir_fs.rs:8829` `conversion_order` and the
  colour placement *(inferred, not pinned)*.
- **Cases (sample).** 4 pairs.

### 4.9 K1: a constant folded with one rounding more

- **What differs.** `constant_expressions.builtin_functions.geometric.length_vec3_fragment`:
  FAU word `3fddb3d8`, Mesa `3fddb3d7` (`sqrt(3)`, 1 ulp).
- **Site.** Mesa's GLSL constant evaluator computes `sqrtf`, correctly
  rounded. Ours keeps `Fsqrt` to the fragment walk, lowers it to
  `frcp(frsq(x))` (`nir_fs.rs:3375-3378`), and folds that as
  `1/(1/sqrt)`, two roundings (`nir.rs:2205`, `:2212`). `ConstantFold`
  covers only non-finite and non-normal values.
- **Cases.** 1 sampled pair; about 14 cases by the group estimate.

### 4.10 The references (item `audit-ref-2`)

`tools/mali/testdata/deqp_audit_<name>.{vert,frag,expect.json}`, drawn with
`tools/mali/deqp_ref.sh <vert> <frag> <prefix> --instrument shim` (Mesa
26.2.1 G610 drm-shim). `tests/mali_deqp_audit.rs` has one `#[ignore]`d compare
test per reference, `differs_pinned` (each still compiles and still differs;
at the tip all 38 do) and `references_parse`. The pairs are **rebuilt** from
dEQP's generator code (VK-GL-CTS `93bca018`, sparse fetch of
`modules/gles2`, `data/gles2`; no run log): the test header says which
template each one follows and what is not dEQP's (the literal a `uniform_api`
case compares against). At the tip every one compiles and none is byte-equal (the first
difference `mali_ref::differences` prints is in the fragment program for the
D1, F1, S1 and R1 shapes, in the varying or position program for the V1, A1,
M1 and L1 ones, and a FAU word for K1).

Not drawn, with the reason:

| Shape | Why none |
|---|---|
| `shaders.random.conditionals.vertex.42`, `.fragment.10`, `.fragment.59`, `comparison_ops.fragment.21`, and the two `shaders.random.*` pairs of L1 | the text is made by dEQP's random shader generator (`rsg`) from a seed, which needs a built dEQP; the sparse sources hold the generator, not its output |
| `attribute_location.bind_aliasing`, `bind_max_attributes.vec3` (V1/other) | the pair needs `glBindAttribLocation`; item `board-baseline-rows` adds explicit bindings to `gl_ref.py` |
| `uniform_api.info_query.*basic_struct.sampler2D_samplerCube_vertex` | no reference could be drawn either at the audit (section 6) |
| the other 8 `D1` shapes and the other V1/F1/S1/A1 pairs of the sample | same cause as a drawn shape; five D1 shapes are drawn, the ones that differ in their widening (`bool`, `bvec2`/`int`, `bvec3`, `bvec4`, `vec2`/`bvec2`) |

## 5. Board forecast for the compiled cases

HOST-ONLY. The host reads black, so this is a forecast from the words.

- **Likely Fail (13 cases, measured over the whole shape, not sampled):**
  D1. The `bvec`/`bool` → `ivec` conversion fragment cases compare lane y
  of the result against lane x of the input. dEQP's inputs mix true and
  false lanes, so all 13 are expected to Fail. They were Fail on the board
  at `37305608` (refused then).
- **May Fail, a minority:**
  - F1 (≈228 cases): a compare at fp16 of two fp32 varyings flips when the
    two are within one fp16 step. dEQP's `float_compare` inputs are spread
    and rarely that close; expect most to Pass.
  - The fp16-uniform compares of V1 in `uniform_api.*` (`|u - c| < 0.05`
    at fp32, where Mesa compares at fp16): a value near the tolerance can
    flip.
  - M1's `0*x` pair (`random.conditionals.fragment.10`), if `x` is
    infinite or NaN there.
- **Expected Pass:** everything else, about 1,200 cases (V1 apart from the
  uniform compares, S1, A1, L1, R1, K1, the rest of M1). These are the
  same operations in another order, layout or width, or the more precise
  fp32 of a mediump value. The board passed 125 of 127 `shaders.operator`
  mediump vertex cases and 114 of 116 lowp ones that already compiled at
  `37305608` (`refusals.tsv` and `results.tsv` of the slices). Whether
  those pairs were byte-equal then is not known, so this is weak evidence
  that V1's fp32 arithmetic passes, not proof.
- **Not forecast:** the 2 struct-of-samplers pairs with no reference; and
  the 11 `shaders.random.*.vertex.*` cases of section 6, which are
  refused at the link.

The forecast says nothing of the 2,950 equal cases' rendering beyond
"Mesa's words". Their state, descriptors and draw are outside this audit.

## 6. Caveats and side findings

- **The sample is by distinct pair.** A case with several programs counts
  in the group of each pair it runs, and the estimates (≈) carry the
  sampling error of 300 of 3,588 pairs. The sample's within-group draws
  favour pairs run by many cases.
- **What is compared.** The comparison is `mali_ref::differences`: the
  words of `vs.position`, `vs.varying` and `fs.fragment`, work registers,
  preload, FAU words for gl_ref.py's uniform values, the secondary offset
  and the padding. It runs `compile_pair` with no attribute-location map
  (the unit tests' `compile_linked` path and the loop caps are not
  reached, `tests/mali_ref/mod.rs`). The `attribute_location.*` cases bind
  locations in dEQP; their words here are for gl_ref.py's locations.
  Descriptors and the command stream are not compared.
- **The equal count is for the pair, not the case.** A case whose sampled
  pair is equal can run another pair that is not.
- **11 "compiled" cases are refused at the link.** These are
  `shaders.random.{basic_expression,conditionals,scalar_conversion,swizzle}.vertex.*`:
  `basic_expression.vertex.30`, `.61`, `.63`, `.97`; `conditionals.vertex.54`,
  `.73`, `.91`; `scalar_conversion.vertex.59`, `.78`; `swizzle.vertex.15`,
  `.30`. Their link log reads "online shaderc: a Mali link whose fragment
  shader reads part of a varying (the link keeps only those components) is
  not lowered yet". That text lacks "the code generator refused", which is
  what `deqp-host-measure.sh`'s `REFUSED` matches. So section 18.1 counts
  them as compiling. The other 24 compiled cases without a pair are
  17 `fragment_ops.scissor.*` (no program in the log), 4
  `shaders.constants.const_float_assign_*_vertex` (a correct compile error,
  dEQP Pass) and 3 `state_query.shader.uniform_value_*`.
- **Still present at the tip:** the two SIGSEGVs of section 18.4
  (`shaders.random.scalar_conversion.combined.96`, `.fragment.96`, 2
  processes exit 139).
- **Two pairs have no reference.** These are
  `uniform_api.info_query.basic_struct.sampler2D_samplerCube_vertex` and
  `uniform_api.info_query.unused_uniforms.sampler2D_samplerCube_vertex`.
  gl_ref.py binds no sampler member of a struct, and Mesa's draw fails
  with `GL_INVALID_OPERATION`. The `gl_ref.py` struct work of
  `fclane2-struct` may cover them.

## 7. To redo

1. The measure: section 2 step 1.
2. A parser of the `.qpa` over `measure.tsv`, as section 2 step 2.
3. `deqp_ref.sh --instrument shim` per pair.
4. A test file in a scratch clone that loops over a list calling
   `mali_ref::compile_pair` and `mali_ref::differences`, building each
   `mali_ref::Reference` from the files (its fields are public).

About 1 hour 30 minutes on 4 cores the first time: the CTS and Mesa builds
take most of it; the 300 references take about 10 minutes.

## 8. Issue text (for the person to post)

> **Mali: 108 of 300 sampled once-refused dEQP-GLES2 shader pairs compile but are not Mesa 26.2.1's words (one miscompile)**
>
> Audit at integration tip `7b540fee` (plan #387 item `compiled-audit`,
> `docs/notes/mali-deqp-compiled-audit.md`). Of the 10,597 dEQP-GLES2 cases
> the NanoPC-T6 refused at `37305608`, 4,428 now compile and link on the
> host (3,588 distinct shader pairs). A stratified sample of 300 pairs was
> byte-compared against Mesa 26.2.1 (drm-shim reference,
> `tools/mali/deqp_ref.sh --instrument shim`; `tests/mali_ref`):
> **190 equal, 108 differ, 2 without a reference**. The estimate is about
> 1,450 compiled cases running a pair that is not Mesa's.
>
> Causes, ranked (sites under `src/shaderc/online/mali/` at `7b540fee`):
> 1. **Miscompile.** The `b2i32` of a 16-bit mask widens the low half for
>    every lane (`nir_fs.rs:12205-12210`). `ivec(bvec2)` reads lane x for
>    lane y. 13 `shaders.conversions.*bvec*/bool*_to_ivec*_fragment` cases,
>    expected to Fail on the board.
> 2. Vertex mediump/lowp arithmetic at fp32 where Mesa narrows to fp16
>    (`nir.rs:4619-4642`, `F2b` `:4909`, compares `:4900-4926`, `Ftrunc`
>    fold `:4977`). There is no gate. About 710 cases. Partly in
>    `mali-intval-vs16-lowering.md` (M1, M2, M4); `bool()`, lowp `int()`,
>    `FRSQ/FRCP.f16` and arithmetic on fp32 attribute lanes have no owner.
> 3. Fragment compares of two fp32 varying lanes built at fp16, where Mesa
>    compares at fp32 (`nir_fs.rs:5950-6005` with `convert_inputs16`
>    `:11538`). `Fs::discard` already refuses the same form
>    (`nir_fs.rs:1845-1853`). About 228 cases.
> 4. Instruction order (`nir_fs.rs:321-614`, `:11597`), about 162 cases;
>    vertex attribute-load order (`nir.rs:6190-6233`), about 135. Both need
>    Mesa's per-pass NIR before a rule is coded.
> 5. One-off folds Mesa's algebraic rules make and ours does not, about 89
>    cases: constant reassociation, ffma chain, `fmul(a, 0)` per lane,
>    `b2f(inot)`. Plus varying width/order, about 64 (`nir_fs.rs:7544`,
>    `:8089`); a fused FMA, about 53 (`vec16.rs:1003`); and a 1-ulp
>    `length()` constant, about 14 (`nir_fs.rs:3375`).
>
> Side findings:
> - 11 `shaders.random.*.vertex.*` cases are refused at the link with
>   "a Mali link whose fragment shader reads part of a varying ... is not
>   lowered yet". `deqp-host-measure.sh` does not count that text as a
>   refusal (`REFUSED` matches only "the code generator refused").
> - The two `scalar_conversion.*.96` SIGSEGVs are still present.
>
> Proposed order: (1) the `b2i32` lane fix with a pinned reference;
> (2) a gate or lowering for F1, the one-line fp32 rule; (3) extend the
> vs16 items to the unowned V1 forms; (4) the rest after the board rerun
> shows which cause, if any, fails.
