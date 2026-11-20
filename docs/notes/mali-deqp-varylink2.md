<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the varying link refusals left at the tip, in dEQP-GLES2 shapes

Plan #387, item varylink2-ref (ROADMAP stage 5, issue #7). The varying
layout refusals (the varylink family of `docs/notes/mali-deqp-families.md`
sections 2, 5 and 17) were studied and partly lowered by plan #377
(`docs/notes/mali-deqp-varylink.md`, commits 22eb4440 and 09adb275, pinned by
`tests/mali_deqp_varylink.rs`). This note measures what is left of them at
the tip, groups it into shapes, and pins every shape no `deqp_varylink_*`
reference covers with a Mesa 26.2.1 reference:
`tools/mali/testdata/deqp_varylink2_<shape>.{vert,frag,expect.json}`, tested
by `tests/mali_deqp_varylink2.rs`. Nothing under `src/` changed.

The four texts (`MaliError`, `src/shaderc/online/mali/mod.rs`; each ends in
"is not lowered yet"):

| key | text |
|---|---|
| equal | `two varying lanes of equal value (Mesa's link merges them)` (`VaryingDuplicate`) |
| noattr | `a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites)` (`VaryingNotFromAttribute`) |
| pos | `a gl_Position lane left unwritten` (`PositionLane`) |
| unwritten | `a varying lane left unwritten` (`VaryingForm`) |

## 1. How it was measured (HOST-ONLY)

- **Tip: cc89a1cf** (the integration tip of plan #387 this item started
  from; `src/shaderc/online/mali/` as at that commit).
- CTS: VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` in
  `$CONNECTED_DEPS_DIR/VK-GL-CTS` (one-commit clone, `external/fetch_sources.py`,
  the three files of `tools/cts/cts-yakogl-target.patch` added uncommitted).
- Mesa: 26.2.1 from the tarball of `third_party/mesa-26.2.1/UPSTREAM`
  (sha256 checked), the shim built by `tools/mali/drm_shim_ref.sh build` into
  `$CONNECTED_DEPS_DIR/mesa-shim`. Self-check: `deqp_ref.sh --instrument shim`
  of `deqp_varylink_draw_single` is byte-equal (`cmp`) to the committed
  `expect.json`.
- The whole measure, not only the varylink cases: the 10,597 cases of the
  slices `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/` whose refusal is
  a compile refusal, so that a case which reaches a varylink text only now is
  counted too.

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" --out build/vl2/measure --stage build
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" --out build/vl2/measure --stage run
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" --out build/vl2/measure --stage extract
```

10,597 of 10,597 cases began; the 108 processes exited 1 (106, dEQP's "not
every case passed") or 139 (2: a crash, after which the chunk went on in a
fresh process, so no case was lost). The board's status quoted below
(Fail, NotSupported) is the NanoPC-T6's at 37305608; the host's dEQP
verdicts mean nothing (nothing is drawn). A "stage" `fs` in `measure.tsv` for
an equal row is a **link** refusal: its info log reads `link error: the
vertex shader for the program's fragment shader: online shaderc: the code
generator refused this fragment shader: Mali(Valhall: two varying lanes of
equal value ...)`, that is, both shaders compile alone and the vertex shader
rebuilt for the fragment shader's reads (`compile_vs_for_reads`) refuses.

## 2. The 1,067 board rows of 37305608 at the tip

1,067 cases had one of the four texts as their first (and only) refusal in
`refusals.tsv` at 37305608 (656 equal, 252 noattr, 155 pos, 4 unwritten;
`docs/notes/mali-deqp-families.md` counts 1,068 on the host). At cc89a1cf:

- **878 compile and link on the host** (no refusal in the log).
- **63 still meet a varylink text first** (38 equal, 4 equal → noattr,
  11 noattr, 6 pos, 4 unwritten).
- **126 now meet another text first**: ARITH (`arithmetic other than +, -,
  *`, #367) 71; the fragment shader's `a discard other than a highp fragment
  shader's if (a < b) discard;` 16; FMA (`a fused multiply-add other than a
  matrix product's`) 16; FRAGCOLOR_LANE 11; `a loop Mesa keeps ... in a
  mediump fragment shader` 5; TEX_COORD 4; `Valhall back end defect: an
  invariant of the pipeline did not hold` 2; colourpair 1.

| 37305608 text | dEQP group | rows | varylink at the tip | compile | another text first |
|---|---|---:|---:|---:|---|
| equal | `vertex_arrays.single_attribute` | 368 | 0 | 368 | |
| equal | `shaders.swizzles` | 126 | 33 | 30 | ARITH 63 |
| equal | `draw.random` | 51 | 0 | 51 | |
| equal | `shaders.indexing` (`vector_subscript`) | 27 | 0 | 27 | |
| equal | `shaders.discard` | 25 | 0 | 0 | discard 16, loops 5, TEX_COORD 4 |
| equal | `shaders.conversions` | 18 | 4 | 6 | ARITH 6, back end defect 2 |
| equal | `draw.draw_arrays`, `draw.draw_elements` (single) | 14 | 0 | 14 | |
| equal | `shaders.random` | 13 | 4 (as noattr) | 4 | FRAGCOLOR_LANE 4, colourpair 1 |
| equal | `attribute_location` | 7 | 0 | 4 | ARITH 2, FRAGCOLOR_LANE 1 |
| equal | `shaders.matrix` (`mul`) | 6 | 0 | 0 | FRAGCOLOR_LANE 6 |
| equal | `shaders.linkage` | 1 | 1 | 0 | |
| noattr | `vertex_arrays.multiple_attributes` | 203 | 0 | 203 | |
| noattr | `draw.draw_arrays`, `draw.draw_elements` | 36 | 0 | 36 | |
| noattr | `shaders.random` | 10 | 9 | 1 | |
| noattr | `shaders.struct` | 2 | 2 | 0 | |
| noattr | `fragment_ops.interaction` | 1 | 0 | 1 | |
| pos | `shaders.operator` | 153 | 6 | 131 | FMA 16 |
| pos | `shaders.functions`, `shaders.qualification_order` | 2 | 0 | 2 | |
| unwritten | `shaders.linkage`, `shaders.declarations` | 4 | 4 | 0 | |
| | **total** | **1,067** | **63** | **878** | **126** |

**28 cases meet a varylink text first at the tip and another one at
37305608** (their earlier refusal was lowered since): 14 intval (13 random
`combined` and `interaction` shaders, `random.basic_expression.vertex.89`),
9 loops (`indexing.vector_subscript.*_dynamic_loop_subscript_read_vertex`,
NS), 2 ARITH (`scalar_to_matrix.bool_to_mat{2,4}_vertex`), 2 CONST_FOLD, 1
LOCALS (`indexing.tmp_array.float_const_write_dynamic_loop_read_vertex`,
NS). So **91 cases carry a varylink text first at
cc89a1cf** (63 + 28), against 1,067 at 37305608.

## 3. The shapes at the tip

Named as in `mali-deqp-families.md` section 3, from the refusing shader's
source. Cases are first refusals at cc89a1cf; Fail/NS is the board's status
at 37305608; "37305608" is how many of the same cases carried a varylink text
first then.

| shape | text, where | cases (Fail / NS) | 37305608 | example case (dEQP-GLES2.functional.) | reference |
|---|---|---:|---|---|---|
| `swz_mixed`: a repeated lane beside a distinct lane of one attribute vector (`in0.grg`, `yyw`, `abba`, `xyyx`; only float `mediump_vec*`: all-equal `xx`/`gggg` and all-distinct swizzles compile) | equal, link | 33 (33 / 0) | 33 (equal, among 126 swizzles) | `shaders.swizzles.vector_swizzles.mediump_vec2_grg_vertex` | `deqp_varylink2_swz_mixed` |
| `prec_mixed`: the same with arithmetic lanes (`vec4(in0, 2.0*in0, -in0, -in0)`), a lowp vertex varying read mediump | equal, link | 1 (1 / 0) | 1 | `shaders.linkage.varying_differing_precision_3` | `deqp_varylink2_prec_mixed` |
| `mat2`: a matrix varying of one scalar (`mat2(in0)`, `mat4(in0)`: equal diagonal, 0.0 elsewhere) | equal, link | 6 (6 / 0) | 4 (2 were ARITH) | `shaders.conversions.scalar_to_matrix.float_to_mat2_vertex` | `deqp_varylink2_mat2` |
| `loop_bcast`: `vec3(res)` of a value a run-time loop (`i < ui_two`) carries | equal, vertex shader alone | 9 (0 / 9) | 0 (all 9 were loops) | `shaders.indexing.vector_subscript.vec2_direct_write_dynamic_loop_subscript_read_vertex` | `deqp_varylink2_loop_bcast` |
| `vec_uniform`: a uniform scalar of its own in a `vec4(...)` beside attribute (or texel) lanes, the shader's only varying (`random.basic_expression.vertex.{27,46,86,89,93}`, `struct.uniform.sampler{,_array}_vertex`) | noattr, vertex shader alone | 7 (7 / 0) | 6 (1 was intval) | `shaders.random.basic_expression.vertex.27` | `deqp_varylink2_vec_uniform`; the struct arm is `varymerge_structsampler` |
| `scalar_uniform`: generated shaders with several varyings, one of them (or one lane) of uniforms or constants alone beside interpolated ones (`random.{basic_expression,scalar_conversion,conditionals}.combined.*`, `fragment_ops.interaction.basic_shader.*`); a bucket: some also hold a `vec_uniform` lane | noattr, vertex shader alone | 24 (24 / 0) | 9 (13 were intval, 2 CONST_FOLD) | `shaders.random.conditionals.combined.75` | `deqp_varylink2_scalar_uniform` |
| `loop_uniform`: `vec4(res, 0.0, 0.0, 1.0)` with `res` summed by a run-time loop over a local array of constants | noattr, vertex shader alone | 1 (0 / 1) | 0 (was LOCALS) | `shaders.indexing.tmp_array.float_const_write_dynamic_loop_read_vertex` | `deqp_varylink2_loop_uniform` |
| `fract_vec2`: shape P with `fract` (`color.yz = fract(in0)`, `vec2`/`vec3` at every precision) | pos, vertex shader alone | 6 (6 / 0) | 6 (of P's 155) | `shaders.operator.common_functions.fract.highp_vec2_vertex` | `deqp_varylink2_fract_vec2` |
| U: a declared varying never written (`linkage.varying_{2,3,4}`, `declarations.invalid_declarations.varying_in_fragment_main`, 1 Pass) | unwritten, vertex shader alone | 4 (3 / 0) | 4 | `shaders.linkage.varying_2` | `deqp_varylink_unwritten` (existing) |
| | | **91** | 63 | | |

Every reference pair is the example case's pair verbatim from the host
run's `.qpa`, plus a header comment naming the case (after `#version 100`
where the source has one). `scalar_uniform` is `conditionals.combined.75`
rather than the first case of its bucket because its fragment shader reads
the uniform-only scalar `a` (the fragment shaders of `combined.13` and
`interaction.basic_shader.27` never read theirs, so by our reading Mesa's link
drops the lane and the reference would show nothing of the rule; not drawn).

At the tip all eight references are refused with the text of their shape
(`refusals_pinned`); the swizzle, linkage and matrix pairs compile alone and
are refused at the link, exactly as their dEQP cases.

## 4. What Mesa 26.2.1 does at the link

Read from the shim's NIR and disassembly (`BIFROST_MESA_DEBUG=shaders`,
`drm_shim_ref.sh run ... --uniform-values`; the `.stderr` is not committed,
rerun the command to get it). "Stores" are the IDVS varying program's
`STORE.i32.estream`, "load" the fragment shader's `LD_VAR_BUF_IMM`.

| reference | regions (non-zero words: position, varying, fragment) | Mesa's varying layout |
|---|---|---|
| `swz_mixed` | 24, 7, 21 | `out0` is `VAR0.xy`, smooth: **two** stores (`in0.y`, `in0.x`); lane 2 is merged into lane 0. FS: one `.v2` smooth load, the third lane a copy of the first |
| `prec_mixed` | 24, 13, 18 | `var` is `VAR0.xyz`, smooth: three stores (`in0`, `in0 + in0`, `-in0`); `w` is merged into `z` |
| `mat2` | 24, 17, 17 | `packed:out0[0],out0[1]`, `VAR0.xyzw`, smooth: **four** stores (`in0`, 0.0, 0.0, `in0`). Nothing is merged and the two 0.0 are stored, not propagated. FS: one `.v4` load |
| `loop_bcast` | 24, 23, 8 | `v_color` is `VAR0.xyz`, smooth: three stores of **one** value (`f2f32` of the loop's fp16 sum). Not merged |
| `vec_uniform` | 24, 19, 10 | `v_color` is `VAR0.xyzw`, smooth: four stores, lane 1 the uniform `g` as a plain store (`F16_TO_F32 u5[1].h00`), no `ffma(x, 0.0, x)`, no flat slot |
| `scalar_uniform` | 24, 6, 9 | the fragment shader reads only `a` (its `if (i)` is constant false, `m` is the constant -1.75): `a` is `VAR0.x`, **flat** (`src_flat32`), stored once as `FMA.f32 x, 0, x` (the inf-to-NaN store of a convergent flat lane); `g`, `f` and `m` have no slot |
| `loop_uniform` | 104, 109, 7 | `v_color` is `VAR0.xyzw`, **smooth**: four stores, `res` and the constants 0.0, 0.0, 1.0 all stored. The position program keeps the array writes (104 words) |
| `fract_vec2` | 24, 29, 7 | `v_color` is `VAR0.xyzw`, smooth: four stores, each `F16_TO_F32` of an fp16 lane: `fract` and `* 0.80 + 0.10` are computed at fp16 on the two defined lanes, the constant lanes 0.10 and 0.90 stored |

What the rows say, against the rules 22eb4440 implemented
(`mali-deqp-varylink.md`, "The rule: a lane is as divergent, and as
constant, as its definition"):

- `swz_mixed`, `prec_mixed`: E1 at the link, unchanged rule: lanes that
  resolve to one scalar (`nir_scalar_resolved` through the swizzle, the
  `fneg` twice CSE'd) merge, also beside distinct lanes.
- `loop_bcast`, `loop_uniform`, `fract_vec2`: the lanes are components of
  one vector operation (the mediump `vec3(res)` / `vec4(res, 0, 0, 1)` is a
  vector `f2f32`, `fract` a vector op), so they are distinct keys and their
  constant components are stored, which is the 22eb4440 origin rule. In
  `loop_uniform` the sum is of a local array (scratch) under a uniform bound,
  and Mesa interpolates it: by our reading `load_scratch` is
  vertex-divergent in `nir_divergence_analysis`, so the vector op is
  divergent. Not verified in the source.
- `mat2`: the matrix varying is lowered to one packed vec4 by the GLSL
  linker (`lower_packed_varyings`, the `packed:` name) and none of its lanes
  is merged or propagated. Why `nir_opt_varyings` leaves it (an indirectly
  addressable array slot, or the packed var's components being one vector
  `vec4` of a non-constant def) is **not determined**; the reference is
  what holds.
- `vec_uniform`: the study's step 6 (`nir_opt_varyings.c:5389-5396`,
  `fs_assign_slot_groups_separate_qual`) proper: a convergent scalar of its
  own takes an unused component of the interpolated vec4 as an interpolated
  lane, a plain store. The same as `varymerge_structsampler`.
- `scalar_uniform`: at the link only the lane the fragment shader reads is
  kept, and it is the flat uniform-only lane `varyconst_*` already lower.
  The refusal is the vertex shader compiled **alone**, which has no Mesa
  counterpart.

## 5. Proposed lowering and forecast

All in `src/shaderc/online/mali/` (`nir.rs`, `nir_fs.rs`, `link.rs`); none
done here. The forecast is HOST-ONLY: the cases whose first refusal is the
shape and whose partner shader compiles alone (the `next_family` of
`measure.tsv` is empty); the link and the draw after the shape are not
measured.

| step | shapes | change | cases | partner compiles |
|---|---|---|---:|---:|
| 1 | `fract_vec2` | The `Ffract` arm of the vertex walk (`nir.rs:4869-4877`) floors every lane up to `width` at `l == 0` (`held`) through `lane(w, s[0], i)` without the undefined-operand rule the lane loop has: a lane past the operand's reaches `alu_sized` as `Val::Undef` and raises `PositionLane` (`nir.rs:2120`). Give `held[i]` `Val::Undef` for an undefined operand lane. Then the pair compiles and, like `deqp_varylink_op_abs_vec2`, should differ only in the vertex fp16 arithmetic (mixprec's vertex side) | 6 | 6 |
| 2 | `swz_mixed`, `prec_mixed` | The program's vertex shader for the fragment shader's reads (`reads == Some`): a lane whose (origin, value) equals a stored lane's is skipped by `seen`, not refused, when the class also has a distinct neighbour; the fragment side already copies the loaded lane (`swz_xx`). First action: print `(reads, equal)` in `build` for `swz_mixed` (the map's E1 open item: the read mask likely names the merged lane) | 34 | 34 |
| 3 | `mat2` | A matrix varying's lanes are neither equal classes nor propagated constants, in both stages (`push_equal`, `consts.push`, `nir_fs.rs` `layout`); every lane stored | 6 | 6 |
| 4 | `loop_bcast`, `loop_uniform` | A component of a vector op over a phi (or a scratch read) keeps its own origin: three stores of one value, constants stored, interpolated. Check `fsload_dup_phi` (pinned refused, `tests.rs`) against a reference first: it is the join case the map said to keep refused, and `loop_bcast` says Mesa stores such lanes rather than refusing | 10 | 10 |
| 5 | `vec_uniform`, `scalar_uniform`, `varymerge_structsampler` | The study's step 6: one placement function in `link.rs` used by `nir.rs build` and `nir_fs.rs layout`: interpolated lanes first, then each convergent lane in an unused interpolated component (plain store), only the rest flat; the `flat && smooth` raises go. The vertex shader compiled alone lays its lanes out the same way and does not refuse | 31 | 11 |
| - | U (4) | `mali-deqp-varylink.md` step 5, pinned by `deqp_varylink_unwritten` | 4 | 3 |

Steps 1 to 5 move 87 first refusals; 67 of them have a partner that
compiles alone, the most that can pass on the board after the lowering
(HOST-ONLY: 33 swizzles, 1 linkage, 6 conversions, 9 indexing NS, 7 random
vertex and struct, 6 operator, 1 tmp_array NS, 4 random combined). None of
the shapes blocks 100 must-pass cases any more; the family that blocked
1,067 at 37305608 blocks 91 at cc89a1cf.

## 6. Found outside this item

- **193 cases** of the 10,597 meet `Valhall back end defect: an invariant of
  the pipeline did not hold (...)` first at cc89a1cf (e.g.
  `attribute_location.bind_aliasing.max_cond_*` (`Encode...`),
  `conversions.matrix_combine.*` (`IrIndex...`)): a defect text, not a
  refusal; worth its own issue. 2 of them are former varylink rows.
- Two `deqp-gles2` processes of the run ended with SIGSEGV (exit 139); the
  chunks went on in fresh processes and every case began. Which case crashed
  was not isolated.

## 7. Issue comment (for the planner to post; agents do not post)

> varylink at cc89a1cf (plan #387 item varylink2-ref,
> `docs/notes/mali-deqp-varylink2.md`): the host measure
> (`tools/mali/deqp-host-measure.sh`, all 10,597 refused cases of the
> 37305608 slices) finds 91 cases whose first refusal is a varying layout
> text, against 1,067 at 37305608: of those 1,067, 878 now compile and link on
> the host, 126 meet another text first (ARITH 71, discard 16, FMA 16,
> FRAGCOLOR_LANE 11, ...) and 63 still meet a varylink text; 28 cases reach
> one only now. Shapes: `swz_mixed` 33 and `prec_mixed` 1 (equal lanes beside
> a distinct lane, at the link), `mat2` 6, `loop_bcast` 9 NS, `vec_uniform` 7,
> `scalar_uniform` 24, `loop_uniform` 1 NS, `fract_vec2` 6, U 4. Eight Mesa
> 26.2.1 references `tools/mali/testdata/deqp_varylink2_*`, all refused and
> pinned by `tests/mali_deqp_varylink2.rs`. Notable: Mesa keeps every lane of
> a matrix varying (`mat2(in0)`: four stores, the 0.0 included), and stores a
> broadcast of a loop value three times. Also found: 193 cases meet a
> `Valhall back end defect` invariant text first at the tip.
