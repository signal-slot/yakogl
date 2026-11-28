<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the residue of the three vertex varying refusals (posvaryres, design)

Plan #387, item posvaryres-refs. A design and Mesa 26.2.1 references for the
dEQP-GLES2 cases whose **first** refusal at the integration tip is one of the
three vertex varying texts: `VaryingForm` ("a varying lane left unwritten"),
`VaryingNotFromAttribute` ("a varying lane computed from no attribute") and
`PositionLane` ("a gl_Position lane left unwritten"). No compiler change.
The posvary design ([`mali-posvary-lowering.md`](mali-posvary-lowering.md),
steps P1 to P5, counted at `9ac31f30`, before trains 1 and 2) is the
starting point; this note measures what is left of it at the tip and what
came in since, and splits it into steps.

References: `tools/mali/testdata/deqp_posvaryres_*` (30 pairs),
`tests/mali_deqp_posvaryres.rs` (every one pinned refused, `refusals_pinned`).

## 1. Measure

- **Tip measured: `349f989f5392bea65c20e7663289e86481e521e0`** (the
  integration tip of this item), HOST-ONLY. The script names it
  `349f989f-dirty` because of the one-line workaround below.
- Command (4 cores), the method of `mali-deqp-families.md` section 19:

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$PWD/build/measure" --jobs 3 --chunk 100
```

- CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`, fetched at that commit
  alone, `external/fetch_sources.py` run, `tools/cts/cts-yakogl-target.patch`
  applied with `git apply`. Cases: the 10,597 of the slices
  `2026-11-15-t6-gles2-S{1..4}-37305608` (the script's default).
- **Workaround, not committed:** at this tip (and on `origin/main`
  `c28470db`) `capi/Cargo.toml` defines the feature `vivante-st` twice
  (lines 41 and 85, from `75a7d901` and `110080eb`). Every cargo command
  fails to load the workspace ("duplicate key"), so the measure's library
  and every `cargo test` fail to build. The measure and the tests of this
  item ran with the second line removed in the working tree only.
- Result: 10,597 cases, 4,709 with a host refusal, no crash. **90 cases**
  have one of the three texts as their first refusal: `VaryingForm` 52 (2
  Fail, 50 NotSupported), `VaryingNotFromAttribute` 35 (34 Fail, 1
  NotSupported), `PositionLane` 3 (NotSupported). These are the counts of
  section 19.1 at `e9033663` (52, 35, 3): nothing that landed since changed
  them.
- **Sites.** Each case's program (its `programs.tsv` pair) went through
  `tests/mali_ref`'s `compile_pair` with a temporary `eprintln!` of
  `file!():line!()` at every construction of the three variants (`nir.rs`,
  `nir_fs.rs`, `mod.rs`, `motion.rs`, `codegen.rs:719`, `backend.rs:1240`)
  and of the state there (the `fill` lanes, the classes of `const_flat`,
  the unwritten lane). All 90 reproduce. The instrumentation was reverted;
  nothing under `src/` changed. Lines are those of `349f989f`.
- **Two probes**, scratch only and reverted: (a) the **linked** vertex
  compile alone (`compile_vs_linked` for the compiled fragment shader, then
  the link, with no stand-alone vertex compile before it) for the 34
  `VaryingNotFromAttribute` cases with a layout site; (b) the run-time-index
  stores rewritten as constant-index stores (`arr[ui_one] =` to `arr[1] =`)
  for the 52 `VaryingForm`/`PositionLane` index cases, to see what such a
  case meets next. Probe (b) changes the shader, so it only names the next
  text; it is not a byte compare.

| site at `349f989f` | text | cases | what it is |
|---|---|--:|---|
| `nir.rs:7936` (`fill`: a convergent lane before an interpolated one) | `VaryingNotFromAttribute` | 31 | 24 refused only by the vertex shader compiled alone (PV1), 7 also at link (PV4) |
| `nir.rs:7699` (`const_flat` beside an interpolated lane) | `VaryingNotFromAttribute` | 2 | refused only alone (PV1) |
| `mod.rs:2154` (`vs_constants`: `consts.full()`) | `VaryingNotFromAttribute` | 1 | 19 constant lanes, past `link::MAX_CONSTS` = 16 (PV5) |
| `nir.rs:7675` (no attribute behind a lane: a phi of uniforms) | `VaryingNotFromAttribute` | 1 | posvary P5 (`loop_phi_array`) |
| `nir.rs:7626` (`Val::Undef` in a stored lane) | `VaryingForm` | 52 | 49 a store at a run-time index (vsunif V2), 3 a store at a loop counter's index (PV2) |
| `nir.rs:2263` (`alu_sized`: an `Undef` operand) | `PositionLane` | 3 | V2's store read by `dot` (PV3) |
| | **total** | **90** | |

## 2. What changed since the posvary design

- P1 (`unwritten_*`), P2 (`fract_*`), P3 (scalar flat after a full vec4)
  and the `tex_uniform_w` part of P4 have landed (trains 1 and 2;
  `tests/mali_deqp_posvary.rs` `MATCHED`). None of the 90 cases is one of
  their shapes any more.
- **The 52 `VaryingForm` cases are not posvary's.** Section 19.1 of
  `mali-deqp-families.md` gives them to the posvary P-steps; their site is
  the unwritten-lane check, but the cause is a store at a run-time index
  (`tmp[ui_one] = ..`), which `docs/notes/mali-vsunif-lowering.md` step
  **V2** already designs (50 cases there: the 49 index cases here plus
  `shaders.struct.local.loop_struct_array_vertex`) with the references
  `deqp_vsunif_{vec2,tmp,mat2}_dynwrite` and `deqp_vsunif_varying_array`.
  V2 is in train 3b (`mali-train-3b`). They inflowed (+48 since section 18)
  when train 2's vertex loops and constant folds let them reach the varying
  store.
- **Most of the `VaryingNotFromAttribute` cases are not a packing problem.**
  26 of the 34 layout refusals come from the vertex shader **compiled alone**
  (`glCompileShader`), which has no fragment reads and so lays out every
  declared lane, including lanes the fragment shader never reads (a
  `gl_FragColor` assignment the next one overwrites; 23 of the 34 fragment
  shaders assign `gl_FragColor` two to six times). The linked compile, built
  for the fragment shader's reads, does not meet the refusal for those 26,
  and for 7 of them it is already byte-equal to Mesa (section 4.1). Posvary
  section 9 item 3 saw the dead reads in `rnd13`/`c30` and looked for them
  in the fragment record; the answer is that the refusing compile is the
  stand-alone one. The posvary references `flat_sep_vec4_bs51` and
  `flat_lane_vec4_c30`, pinned `NO_ATTR` in `tests/mali_deqp_posvary.rs`,
  are of this kind: their linked compile meets a fragment text instead
  (`Arithmetic`, `FsVaryingLoad`); `flat_lane_vec4_v86` is not (PV4).

## 3. The steps

| step | mechanism | cases (Fail / NS) | dEQP groups | references | depends on |
|---|---|--:|---|---|---|
| **PV1** | the stand-alone vertex compile answers `glCompileShader` without refusing a layout only the link decides (`fill` order, `const_flat`) | 26 (26 / 0) | `fragment_ops.interaction.basic_shader.*` 7, `shaders.random.{basic_expression,conditionals,scalar_conversion}.combined.*` 19 | `alone_bs19`, `alone_bs27`, `alone_bs60`, `alone_c53`, `alone_s87`, `constflat_s15`, `fill_three_vec4` (byte-equal once linked); `constflat_c70`, `fill_flat_after_vec4`, `fill_two_vec4`, `lane_fill_flat_two_vec4`, `lane_fill_two_vec4` (then a fragment text) | none |
| **PV4** | the linked layout: convergent lanes of a varying laid out before its interpolated lanes move after them (posvary P4's rest, its M1c) | 7 (7 / 0) | `shaders.random.basic_expression.vertex.{18,27,46,86,89}`, `comparison_ops.vertex.33`, `swizzle.vertex.27` | `vlane_v18`, `vlane_v27`, `vlane_v46`, `vlane_v89`, `vlane_cmp33`, `vlane_swz27`, and `deqp_posvary_flat_lane_vec4_v86` (the pair of `vertex.86`) | PV1 (same code, `fill`) |
| **PV5** | more than 16 constant lanes: all propagated into the fragment shader | 1 (1 / 0) | `shaders.random.scalar_conversion.combined.37` | `consts_full_s37` | none |
| **V2** (vsunif) | a store at a run-time index into a local vector, matrix, array or a varying array: Mesa's `nir_lower_indirect_derefs` selects | 49 (0 / 49) | `shaders.indexing.{vector_subscript 12, tmp_array 12, matrix_subscript 9, varying_array 16}` | vsunif's four, plus `mat4_dynw`, `tmp_float_dynw`, `tmp_vec4_dynw_loop`, `vary_float_dynw`, `vec4_dynw_dynr` | vsunif V3 (fp16 selects, for byte equality) |
| **PV3** | V2's store read by a `dot` (`*_dynamic_subscript_write_direct_read_vertex`): the undef operand of `alu_sized` | 3 (0 / 3) | `shaders.indexing.vector_subscript.vec{2,3,4}_dynamic_subscript_write_direct_read_vertex` | `vec2_dynw_dot`, `vec3_dynw_dot`, `vec4_dynw_dot` | V2 (no own change expected) |
| **PV2** | a store at a loop counter's index (`rgb[i] = s[2-i].a` in a vertex loop) | 3 (2 / 1) | `shaders.struct.local.loop_struct_array_vertex`, `shaders.struct.uniform.{,dynamic_}loop_struct_array_vertex` | `struct_local_loop`, `struct_unif_loop`, `struct_unif_dynloop` | the vertex loop unrolling (plan #402 `lower-vsloop`); the dynamic one also the loops design's kept vertex loops |
| **P5** (posvary) | a loop's phi of uniforms and constants stored in a varying | 1 (0 / 1) | `shaders.indexing.tmp_array.float_const_write_dynamic_loop_read_vertex` | `deqp_posvary_loop_phi_array` (the same pair) | the scratch array (vsunif V4) and the int-uniform vertex loop (`LoopVertex`) |
| | **total** | **90 (36 / 54)** | | 30 new + 6 existing | |

The appendix maps each of the 90 cases to its step and reference.

### 3.1 PV1: the stand-alone vertex compile (26 cases)

Site: `nir.rs:7936` (`fill`, 24 cases) and `nir.rs:7699` (`const_flat`, 2),
reached from `online_compile` for `SHC_STAGE_VERTEX`. `backend.rs`
`or_without_consts` (`:1243-1273`) already answers this compile for two other
link-only refusals (a constant lane, `VaryingNotFromAttribute` from the
constants; two equal lanes, `VaryingDuplicate`) by building the shader
without those lanes, and its comment states the rule this step relies on:
"The record only answers glCompileShader: the program's vertex shader is
built at link for the fragment shader's reads (`vs_for_fragment`)". Mesa
does the same in effect: st/mesa compiles a stage at link, with the other
stage's reads (`nir_opt_varyings`).

Mechanism: when the stand-alone compile is refused at one of the two layout
sites, answer `glCompileShader` with a record whose layout cannot be refused
(for example every convergent lane flat after the interpolated ones, or the
interpolated lanes alone), and leave the layout to the linked compile. The
link then refuses what it really cannot build (PV4's 7 cases stay refused
there). What must hold: the stand-alone record is never the program's vertex
shader on the Mali (check `link_by_sources` and `vs_for_fragment`, and that a
relink failure reaches `glLinkProgram`'s status and log, the dEQP verdict
either way), and the `VaryingNotFromAttribute` of `codegen.rs:719` (more
read entries than 16) is not reached.

Measured (probe a): the linked compile of the 26 gives

| linked outcome | cases | references |
|---|--:|---|
| byte-equal to Mesa (vertex and fragment words, FAU, secondary offset) | 7 | `alone_bs19`, `alone_bs27`, `alone_bs60`, `alone_c53`, `alone_s87`, `constflat_s15`, `fill_three_vec4` |
| the fragment shader alone is refused: `ColourForm` 6, `Arithmetic` 2, `FsUniform` 2, `FsVaryingLoad` 2, `FsOperandForm` 2, `ArithConstant` 2, `ControlFlow` 1, `FragCoordMediump` 1 | 18 | `fill_two_vec4`, `fill_flat_after_vec4` (`FsUniform`), `lane_fill_flat_two_vec4` (`Arithmetic`), `constflat_c70` (`ArithConstant`) |
| the linked vertex shader: `FsOperandForm` (`relink`) | 1 | `lane_fill_two_vec4` |

So PV1 removes 26 cases from `VaryingNotFromAttribute`; 7 then compile and
link with Mesa's words (HOST-ONLY: their board verdict is not known), 19
move to fragment texts already ranked in section 19.1. Mesa's words for the
seven: six fragment shaders are the 7-word constant colour (the reads that
survive are constants Mesa propagates); five of those have no varying
program, `alone_c53` (12 lanes laid out by the stand-alone compile) stores
the four interpolated lanes of one vec4 (22 `vs.varying` words), and
`constflat_s15` keeps a 20-word varying program and a 10-word fragment. The posvary pins `flat_sep_vec4_bs51` and
`flat_lane_vec4_c30` move with this step (to `Arithmetic` and
`FsVaryingLoad`); the PV1 session repins them in `tests/mali_deqp_posvary.rs`.

### 3.2 PV4: convergent lanes before interpolated ones, linked (7 cases)

Site: `nir.rs:7936` in the linked compile too. The lanes measured there:
one varying (location 0) whose convergent lanes come first or in between,
`[(0,0,c),(0,2,i),(0,3,i)]` (`vlane_v89`, `v86`), `[(0,0,c),(0,1,i),(0,3,c)]`
(`vlane_v46`), `[(0,0,i),(0,1,c),(0,2,i),(0,3,i)]` (`vlane_v27`), `[(0,0,c),
(0,3,i)]` (`vlane_swz27`), `[(0,0,c),(0,1,i),(0,2,i),(0,3,i)]`
(`vlane_cmp33`); `vlane_v18` the same kind (its probe line was lost; its
first refusal and site are those of the other six).
Mesa (`vlane_v46`, `vs.varying`): the interpolated lane first (`LD_ATTR_IMM`,
`FADD.f32 r0, u6.w0, r0.neg`, store word 0), then the bare uniform
(`F16_TO_F32 u5.w1.h0`, word 1) and the uniform expression (`FMA.v2f16
0x38000000.h11, u5.w1.h00`, `F16_TO_F32`, word 2), **no `FMA x, 0, x`**: both
convergent lanes fill the interpolated vec4's free words. This is posvary
section 2's rule and its section 4.3 step 3 (M1c) with section 9 items 4
and 5: a per-lane word instead of the per-varying offset, and one varying
record per run of words (`link.rs` `pair`, `compile_module`'s
`VaryingSlot`s). The `fill` function's own comment names the case it
refuses ("a convergent lane before an interpolated one ... would move").

### 3.3 PV5: more constant lanes than the table holds (1 case)

`shaders.random.scalar_conversion.combined.37` stores 19 constant lanes
(`a`, `b`, `l`, `o` vec4s and `c`, `d`, `j`; `b` and `o` computed from
constants, `j` reassigned). `link::MAX_CONSTS` is 16 (`link.rs:109`) and
`vs_constants` refuses a full table (`mod.rs:2154`); the linked compile
then meets `nir.rs:7651` (a constant lane not kept). Mesa
(`consts_full_s37`) propagates every lane: no `vs.varying` program, the
fragment shader the 7-word constant colour (FAU `3f700000`, `3a803b00`,
`3b800000`). Mechanism: a table of `4 * MAX_VARYINGS` lanes (or the
fragment's read count). Small and independent.

### 3.4 V2 and PV3: stores at a run-time index (49 + 3 cases)

The design is vsunif V2 (`mali-vsunif-lowering.md` section 4, V2): each
element `e` gets `bcsel(index == e, x, old)` from `nir_lower_indirect_derefs`,
fp16 `MUX.v2i16` selects on `ICMP_OR` of the int uniform's FAU word, and it
needs V3 to be byte-equal. This item adds references for the sub-shapes
vsunif's four do not draw: `mat4_dynw` (four columns, 92 `vs.varying`
words), `tmp_float_dynw` (scalar elements, 65), `tmp_vec4_dynw_loop` (the
reads in a constant loop, 88), `vec4_dynw_dynr` (a dynamic write and a
dynamic read of a vec4, 68) and `vary_float_dynw` (a float varying array,
58; its fragment shader reads `var[0..3]`, 12 words).

Probe (b), the writes made constant: 35 of the 49 and all 3 PV3 cases then
compile; the 14 others are `varying_array` cases whose **fragment** shader
meets a text of its own: `Arithmetic` 8 (`*_dynamic_read`,
`*_dynamic_loop_read`: the fragment's run-time index read), `FsVaryingLoad`
4 (`float`, `vec2` static reads), `ColourForm` 2 (`vec3`). PV3's three
`*_direct_read_vertex` cases fail one step earlier than the other twelve
`vector_subscript` cases only because `dot(tmp, ..)` reads the unwritten
lanes as an operand (`alu_sized`, the misnamed `PositionLane`, posvary
section 5); once V2 writes the lanes they are ordinary operands. The
`vec{2,3,4}_dynw_dot` references are their byte targets (20, 31 and 44
`vs.varying` words).

### 3.5 PV2: a store at a loop counter's index (3 cases)

`rgb[i] = s[2-i].a; alpha += s[i].b;` in `for (int i = 0; i < 3; i++)`
(`struct.local` with a local struct array, `struct.uniform` with a uniform
one) and in `i < ui_three` (`dynamic_loop_struct_array`). Site
`nir.rs:7626`: lane 0 of `v_color` is undef. Probes: the loop unrolled by
hand compiles; the loop with only the `rgb[i]` store refuses with lanes
`x, y, z` unwritten (mask 7), so the store at the counter's index is what
leaves them undef, not the struct read. Mesa:

- `struct_unif_loop` (22 `vs.varying` words): unrolled, the index constant;
  every lane is a uniform, stored **flat** (`FMA.f32 r2, u7.w1, 0x0, u7.w1`:
  the mediump struct member is read as its fp32 FAU word), the int sum
  `IADD.s32` of the three members and `S32_TO_F32`.
- `struct_unif_dynloop` (40 words): the loop bounded by `ui_three` and the
  array length 3 becomes guarded selects, `ICMP_OR.s32.lt.m1` of `ui_three`
  against `0x3020100.b2` / `0x1000000.b3` and `CSEL.u32.eq`, one
  `BRANCHZ.reconverge` for `ui_three <= 0`, the lanes narrowed to fp16
  (`FADD.f32 r.h0, u7.w0, 0x0.neg`) and widened at the store, all four flat.
- `struct_local_loop` (19 words), the local-array form of the first.

Mechanism: in the vertex walk's loop unrolling, a store whose index is the
counter takes the counter's constant in each copy; the dynamic bound needs
the kept-loop or guarded form of the loops design. vsunif V2 counts
`struct.local.loop_struct_array_vertex` among its 50; it is the same
mechanism as the two `struct.uniform` ones and has no vsunif reference, so
whichever step comes first takes it (this note counts it under PV2 only).

### 3.6 P5: the loop phi (1 case)

As posvary section 7: `deqp_posvary_loop_phi_array` is the case's own pair,
pinned `NO_ATTR`. It needs the 40-element array in thread-local storage
(vsunif V4) and the int-uniform vertex loop; keep it last.

## 4. Order and forecast

1. **PV1** first: one place (`backend.rs` `or_without_consts` or the
   stand-alone compile path), no layout code, 26 cases off the text, 7
   byte-equal at once. Repins `tests/mali_deqp_posvary.rs`
   (`flat_sep_vec4_bs51`, `flat_lane_vec4_c30`) and this test's seven
   `LOWERED` and five fragment texts.
2. **PV5**: `link::MAX_CONSTS` and the table, 1 case.
3. **PV4** (posvary M1c): the per-lane word layout and the run records, 7
   cases; with PV1 done, `fill`'s refusal is reached by the linked compile
   alone. Take `flat_lane_vec4_v86` with it.
4. **V2 + PV3** in train 3b (vsunif), with V3; the five new index
   references and the three `dot` ones are byte targets for it. 52 cases.
5. **PV2** after the vertex loop unrolling of plan #402 `lower-vsloop`; the
   dynamic form with the loops design's kept vertex loops. 3 cases.
6. **P5** last (vsunif V4, `LoopVertex`). 1 case.

After all of them the three texts have no first-refusal case left
(`VaryingNotFromAttribute` 35 by PV1 26, PV4 7, PV5 1, P5 1; `VaryingForm`
52 by V2 49 and PV2 3; `PositionLane` 3 by PV3). The cases move on: of the
35 + 52 measured by the probes, 7 + 0 are byte-equal, 38 compile in probe
(b) (not a byte compare), the rest meet fragment texts (19 after PV1, 14
after V2) or the step's own refusal at link (PV4's 7, PV5's 1).

## 5. Checks of the references

- Drawn with `tools/mali/deqp_ref.sh <vert> <frag> <prefix> --instrument
  shim` on Mesa 26.2.1's drm-shim (`drm_shim_ref.sh build` from the
  `third_party/mesa-26.2.1/UPSTREAM` tarball, sha256 checked), G610; the
  shim reproduced `deqp_posvary_flat_lane_vec4_c30.expect.json` byte for
  byte first.
- `.nir.sha` from `tools/mali/nir_refs.sh 'deqp_posvaryres_*'`: 30 written,
  0 no draw; it redraws every pair and requires the same words.
- `tools/mali/g310_check.sh 'deqp_posvaryres_*'`: 30 identical on the
  G310v2 and the G610.
- One pair drawn first, `shaders.random.basic_expression.combined.30`, is
  `deqp_posvary_flat_lane_vec4_c30` byte for byte; it was dropped. No other
  `deqp_posvaryres_*` pair equals an existing reference.

## Appendix: the 90 cases

"board" is the NanoPC-T6 status of the slices; "next" is probe (a) for PV1,
PV4 and PV5 (the linked compile alone) and probe (b) for V2 and PV3 (the
writes made constant), empty where no probe was run.

| # | case (`dEQP-GLES2.functional.`) | board | first refusal | site at `349f989f` | step | reference | next (host probe) |
|--:|---|---|---|---|---|---|---|
| 1 | `fragment_ops.interaction.basic_shader.19` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | `deqp_posvaryres_alone_bs19` | byte-equal (linked compile) |
| 2 | `fragment_ops.interaction.basic_shader.23` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | `deqp_posvaryres_lane_fill_flat_two_vec4` | `Arithmetic` |
| 3 | `fragment_ops.interaction.basic_shader.27` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | `deqp_posvaryres_alone_bs27` | byte-equal (linked compile) |
| 4 | `fragment_ops.interaction.basic_shader.29` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | `deqp_posvaryres_fill_three_vec4` | byte-equal (linked compile) |
| 5 | `fragment_ops.interaction.basic_shader.51` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `Arithmetic` |
| 6 | `fragment_ops.interaction.basic_shader.60` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | `deqp_posvaryres_alone_bs60` | byte-equal (linked compile) |
| 7 | `fragment_ops.interaction.basic_shader.77` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `ColourForm` |
| 8 | `shaders.random.basic_expression.combined.12` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | `deqp_posvaryres_fill_two_vec4` | `FsUniform` |
| 9 | `shaders.random.basic_expression.combined.27` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `ColourForm` |
| 10 | `shaders.random.basic_expression.combined.30` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | `deqp_posvary_flat_lane_vec4_c30` (same pair) | `FsVaryingLoad` |
| 11 | `shaders.random.basic_expression.combined.47` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `ColourForm` |
| 12 | `shaders.random.basic_expression.combined.49` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `ColourForm` |
| 13 | `shaders.random.basic_expression.combined.53` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | `deqp_posvaryres_alone_c53` | byte-equal (linked compile) |
| 14 | `shaders.random.basic_expression.combined.63` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `FsOperandForm` |
| 15 | `shaders.random.basic_expression.combined.66` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `ArithConstant` |
| 16 | `shaders.random.basic_expression.combined.70` | Fail | `VaryingNotFromAttribute` | `nir.rs:7699` | PV1 | `deqp_posvaryres_constflat_c70` | `ArithConstant` |
| 17 | `shaders.random.basic_expression.combined.74` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | `deqp_posvaryres_fill_flat_after_vec4` | `FsUniform` |
| 18 | `shaders.random.basic_expression.combined.77` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `FsOperandForm` |
| 19 | `shaders.random.basic_expression.combined.86` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `ColourForm` |
| 20 | `shaders.random.conditionals.combined.51` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `ControlFlow` |
| 21 | `shaders.random.conditionals.combined.75` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `FsVaryingLoad` |
| 22 | `shaders.random.scalar_conversion.combined.15` | Fail | `VaryingNotFromAttribute` | `nir.rs:7699` | PV1 | `deqp_posvaryres_constflat_s15` | byte-equal (linked compile) |
| 23 | `shaders.random.scalar_conversion.combined.18` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `ColourForm` |
| 24 | `shaders.random.scalar_conversion.combined.26` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | shape of the PV1 references | `FragCoordMediump` |
| 25 | `shaders.random.scalar_conversion.combined.51` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | `deqp_posvaryres_lane_fill_two_vec4` | link: `FsOperandForm` |
| 26 | `shaders.random.scalar_conversion.combined.87` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV1 | `deqp_posvaryres_alone_s87` | byte-equal (linked compile) |
| 27 | `shaders.random.basic_expression.vertex.18` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV4 | `deqp_posvaryres_vlane_v18` | the same text at link (the step itself) |
| 28 | `shaders.random.basic_expression.vertex.27` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV4 | `deqp_posvaryres_vlane_v27` | the same text at link (the step itself) |
| 29 | `shaders.random.basic_expression.vertex.46` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV4 | `deqp_posvaryres_vlane_v46` | the same text at link (the step itself) |
| 30 | `shaders.random.basic_expression.vertex.86` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV4 | `deqp_posvary_flat_lane_vec4_v86` (same pair) | the same text at link (the step itself) |
| 31 | `shaders.random.basic_expression.vertex.89` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV4 | `deqp_posvaryres_vlane_v89` | the same text at link (the step itself) |
| 32 | `shaders.random.comparison_ops.vertex.33` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV4 | `deqp_posvaryres_vlane_cmp33` | the same text at link (the step itself) |
| 33 | `shaders.random.swizzle.vertex.27` | Fail | `VaryingNotFromAttribute` | `nir.rs:7936` | PV4 | `deqp_posvaryres_vlane_swz27` | the same text at link (the step itself) |
| 34 | `shaders.random.scalar_conversion.combined.37` | Fail | `VaryingNotFromAttribute` | `mod.rs:2154` | PV5 | `deqp_posvaryres_consts_full_s37` | the same text at link (the step itself) |
| 35 | `shaders.indexing.tmp_array.float_const_write_dynamic_loop_read_vertex` | NotSupported | `VaryingNotFromAttribute` | `nir.rs:7675` | P5 | `deqp_posvary_loop_phi_array` (same pair) |  |
| 36 | `shaders.indexing.matrix_subscript.mat2_dynamic_write_dynamic_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_mat2_dynwrite / deqp_posvaryres_mat4_dynw | compiles (constant-index probe) |
| 37 | `shaders.indexing.matrix_subscript.mat2_dynamic_write_static_loop_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_mat2_dynwrite / deqp_posvaryres_mat4_dynw | compiles (constant-index probe) |
| 38 | `shaders.indexing.matrix_subscript.mat2_dynamic_write_static_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | `deqp_vsunif_mat2_dynwrite` (same pair) | compiles (constant-index probe) |
| 39 | `shaders.indexing.matrix_subscript.mat3_dynamic_write_dynamic_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_mat2_dynwrite / deqp_posvaryres_mat4_dynw | compiles (constant-index probe) |
| 40 | `shaders.indexing.matrix_subscript.mat3_dynamic_write_static_loop_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_mat2_dynwrite / deqp_posvaryres_mat4_dynw | compiles (constant-index probe) |
| 41 | `shaders.indexing.matrix_subscript.mat3_dynamic_write_static_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_mat2_dynwrite / deqp_posvaryres_mat4_dynw | compiles (constant-index probe) |
| 42 | `shaders.indexing.matrix_subscript.mat4_dynamic_write_dynamic_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_mat2_dynwrite / deqp_posvaryres_mat4_dynw | compiles (constant-index probe) |
| 43 | `shaders.indexing.matrix_subscript.mat4_dynamic_write_static_loop_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_mat2_dynwrite / deqp_posvaryres_mat4_dynw | compiles (constant-index probe) |
| 44 | `shaders.indexing.matrix_subscript.mat4_dynamic_write_static_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | `deqp_posvaryres_mat4_dynw` | compiles (constant-index probe) |
| 45 | `shaders.indexing.tmp_array.float_dynamic_write_dynamic_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_tmp_dynwrite / deqp_posvaryres_tmp_float_dynw | compiles (constant-index probe) |
| 46 | `shaders.indexing.tmp_array.float_dynamic_write_static_loop_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_posvaryres_tmp_vec4_dynw_loop | compiles (constant-index probe) |
| 47 | `shaders.indexing.tmp_array.float_dynamic_write_static_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | `deqp_posvaryres_tmp_float_dynw` | compiles (constant-index probe) |
| 48 | `shaders.indexing.tmp_array.vec2_dynamic_write_dynamic_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_tmp_dynwrite / deqp_posvaryres_tmp_float_dynw | compiles (constant-index probe) |
| 49 | `shaders.indexing.tmp_array.vec2_dynamic_write_static_loop_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_posvaryres_tmp_vec4_dynw_loop | compiles (constant-index probe) |
| 50 | `shaders.indexing.tmp_array.vec2_dynamic_write_static_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_tmp_dynwrite / deqp_posvaryres_tmp_float_dynw | compiles (constant-index probe) |
| 51 | `shaders.indexing.tmp_array.vec3_dynamic_write_dynamic_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_tmp_dynwrite / deqp_posvaryres_tmp_float_dynw | compiles (constant-index probe) |
| 52 | `shaders.indexing.tmp_array.vec3_dynamic_write_static_loop_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_posvaryres_tmp_vec4_dynw_loop | compiles (constant-index probe) |
| 53 | `shaders.indexing.tmp_array.vec3_dynamic_write_static_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_tmp_dynwrite / deqp_posvaryres_tmp_float_dynw | compiles (constant-index probe) |
| 54 | `shaders.indexing.tmp_array.vec4_dynamic_write_dynamic_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_tmp_dynwrite / deqp_posvaryres_tmp_float_dynw | compiles (constant-index probe) |
| 55 | `shaders.indexing.tmp_array.vec4_dynamic_write_static_loop_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | `deqp_posvaryres_tmp_vec4_dynw_loop` | compiles (constant-index probe) |
| 56 | `shaders.indexing.tmp_array.vec4_dynamic_write_static_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | `deqp_vsunif_tmp_dynwrite` (same pair) | compiles (constant-index probe) |
| 57 | `shaders.indexing.varying_array.float_dynamic_write_dynamic_loop_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `Arithmetic` |
| 58 | `shaders.indexing.varying_array.float_dynamic_write_dynamic_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `Arithmetic` |
| 59 | `shaders.indexing.varying_array.float_dynamic_write_static_loop_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `FsVaryingLoad` |
| 60 | `shaders.indexing.varying_array.float_dynamic_write_static_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | `deqp_posvaryres_vary_float_dynw` | `FsVaryingLoad` |
| 61 | `shaders.indexing.varying_array.vec2_dynamic_write_dynamic_loop_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `Arithmetic` |
| 62 | `shaders.indexing.varying_array.vec2_dynamic_write_dynamic_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `Arithmetic` |
| 63 | `shaders.indexing.varying_array.vec2_dynamic_write_static_loop_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `FsVaryingLoad` |
| 64 | `shaders.indexing.varying_array.vec2_dynamic_write_static_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `FsVaryingLoad` |
| 65 | `shaders.indexing.varying_array.vec3_dynamic_write_dynamic_loop_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `Arithmetic` |
| 66 | `shaders.indexing.varying_array.vec3_dynamic_write_dynamic_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `Arithmetic` |
| 67 | `shaders.indexing.varying_array.vec3_dynamic_write_static_loop_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `ColourForm` |
| 68 | `shaders.indexing.varying_array.vec3_dynamic_write_static_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `ColourForm` |
| 69 | `shaders.indexing.varying_array.vec4_dynamic_write_dynamic_loop_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `Arithmetic` |
| 70 | `shaders.indexing.varying_array.vec4_dynamic_write_dynamic_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | `Arithmetic` |
| 71 | `shaders.indexing.varying_array.vec4_dynamic_write_static_loop_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_varying_array / deqp_posvaryres_vary_float_dynw | compiles (constant-index probe) |
| 72 | `shaders.indexing.varying_array.vec4_dynamic_write_static_read` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | `deqp_vsunif_varying_array` (same pair) | compiles (constant-index probe) |
| 73 | `shaders.indexing.vector_subscript.vec2_dynamic_subscript_write_component_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_vec2_dynwrite | compiles (constant-index probe) |
| 74 | `shaders.indexing.vector_subscript.vec2_dynamic_subscript_write_dynamic_subscript_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_vec2_dynwrite | compiles (constant-index probe) |
| 75 | `shaders.indexing.vector_subscript.vec2_dynamic_subscript_write_static_loop_subscript_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_vec2_dynwrite | compiles (constant-index probe) |
| 76 | `shaders.indexing.vector_subscript.vec2_dynamic_subscript_write_static_subscript_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_vsunif_vec2_dynwrite | compiles (constant-index probe) |
| 77 | `shaders.indexing.vector_subscript.vec3_dynamic_subscript_write_component_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_posvaryres_vec4_dynw_dynr / deqp_vsunif_vec2_dynwrite | compiles (constant-index probe) |
| 78 | `shaders.indexing.vector_subscript.vec3_dynamic_subscript_write_dynamic_subscript_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_posvaryres_vec4_dynw_dynr / deqp_vsunif_vec2_dynwrite | compiles (constant-index probe) |
| 79 | `shaders.indexing.vector_subscript.vec3_dynamic_subscript_write_static_loop_subscript_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_posvaryres_vec4_dynw_dynr / deqp_vsunif_vec2_dynwrite | compiles (constant-index probe) |
| 80 | `shaders.indexing.vector_subscript.vec3_dynamic_subscript_write_static_subscript_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_posvaryres_vec4_dynw_dynr / deqp_vsunif_vec2_dynwrite | compiles (constant-index probe) |
| 81 | `shaders.indexing.vector_subscript.vec4_dynamic_subscript_write_component_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_posvaryres_vec4_dynw_dynr / deqp_vsunif_vec2_dynwrite | compiles (constant-index probe) |
| 82 | `shaders.indexing.vector_subscript.vec4_dynamic_subscript_write_dynamic_subscript_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | `deqp_posvaryres_vec4_dynw_dynr` | compiles (constant-index probe) |
| 83 | `shaders.indexing.vector_subscript.vec4_dynamic_subscript_write_static_loop_subscript_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_posvaryres_vec4_dynw_dynr / deqp_vsunif_vec2_dynwrite | compiles (constant-index probe) |
| 84 | `shaders.indexing.vector_subscript.vec4_dynamic_subscript_write_static_subscript_read_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | V2 (vsunif) | shape of deqp_posvaryres_vec4_dynw_dynr / deqp_vsunif_vec2_dynwrite | compiles (constant-index probe) |
| 85 | `shaders.indexing.vector_subscript.vec2_dynamic_subscript_write_direct_read_vertex` | NotSupported | `PositionLane` | `nir.rs:2263` | PV3 (with V2) | `deqp_posvaryres_vec2_dynw_dot` | compiles (constant-index probe) |
| 86 | `shaders.indexing.vector_subscript.vec3_dynamic_subscript_write_direct_read_vertex` | NotSupported | `PositionLane` | `nir.rs:2263` | PV3 (with V2) | `deqp_posvaryres_vec3_dynw_dot` | compiles (constant-index probe) |
| 87 | `shaders.indexing.vector_subscript.vec4_dynamic_subscript_write_direct_read_vertex` | NotSupported | `PositionLane` | `nir.rs:2263` | PV3 (with V2) | `deqp_posvaryres_vec4_dynw_dot` | compiles (constant-index probe) |
| 88 | `shaders.struct.local.loop_struct_array_vertex` | Fail | `VaryingForm` | `nir.rs:7626` | PV2 | `deqp_posvaryres_struct_local_loop` |  |
| 89 | `shaders.struct.uniform.dynamic_loop_struct_array_vertex` | NotSupported | `VaryingForm` | `nir.rs:7626` | PV2 | `deqp_posvaryres_struct_unif_dynloop` |  |
| 90 | `shaders.struct.uniform.loop_struct_array_vertex` | Fail | `VaryingForm` | `nir.rs:7626` | PV2 | `deqp_posvaryres_struct_unif_loop` |  |
