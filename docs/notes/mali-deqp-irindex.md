<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): `Defect(IrIndex)` in dEQP-GLES2: shapes, sites, root causes, Mesa references

**HOST-ONLY.** Plan #387 item `irindex-ref` (issue #7, ROADMAP stage 5): the
reference-and-triage half of `irindex-fix`. No compiler change. Every case
count in this note comes from `deqp-gles2` run on a host against the Mali
back end's C pair on the Linux OSAL (`tools/mali/deqp-host-measure.sh`;
method and limits in `mali-deqp-families.md` sections 1 to 3). What is real
is each shader's compile and link outcome and our info log, never a render
verdict. A board status quoted here is the NanoPC-T6's own, from the
committed slices.

The text: `Valhall back end defect: an invariant of the pipeline did not hold
(IrIndex)` (`MaliError::Defect(Defect::IrIndex)`,
`src/shaderc/online/mali/mod.rs`). It is not a refusal by name. The back end
looked up an IR value, a varying lane, a load or an encoding that an earlier
pass should have made, and did not find it. `mali-deqp-families.md` section
18 counted it as the first refusal of 187 must-pass cases at `9ac31f30`. It
suspected the `FsPrecision::of` re-parse (`mod.rs:1071`) and a regression
from intval step 1 (`ab9cde96`). The re-parse is **not** the site: none of
the 187 fire there. Five sites fire, with eight distinct causes (section 2).

## 1. Method

- Tip `3497799a`. CTS VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`
  (`tools/cts/deps.toml`) with `cts-yakogl-target.patch`, in
  `$CONNECTED_DEPS_DIR/VK-GL-CTS`.
- Measure (4 cores, `deqp-gles2` and the library built once under
  `build/host-deqp-mali/`):

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$PWD/build/measure" --jobs 3 --chunk 100
```

  It runs the 10,597 refused cases of the four committed slices
  `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608`. 6,132 have a host
  refusal at `3497799a` (6,196 at `9ac31f30`). All 10,597 began; 106
  processes exited 1 and 2 exited 139, the same as section 18.
  **187 cases have `Defect(IrIndex)` as their first refusal**: 185 directly
  in the fragment shader, 2 through a program link (`link error: the vertex
  shader for the program's fragment shader: ...`, the same fragment
  defect). That is the same count as at `9ac31f30`. All 187 are board
  `Fail`. 152 of them were board `intval` cases.
- Site and cause: each case's pair (`src/<case>.vert|.frag` of the measure)
  was compiled through `tests/mali_ref`'s `compile_pair` in a scratch copy
  of the tip (not committed). In that copy every `Defect::IrIndex`
  construction prints its `file:line` and a backtrace of the
  `shaderc/online/mali` frames. The eager `let defect = ...` sites were made
  lazy, and `eprintln` was added at the callers to print the looked-up
  lane, load or `(op, bits, width)`. All 187 reproduce the defect there.
  Line numbers in this note are those of `3497799a`.
- Regression check: the same 187 pairs were compiled at `ab9cde96^`
  (`cd758914`) and at each of the 22 commits under
  `src/shaderc/online/mali/` from `ab9cde96` to the tip (first parent),
  each in a scratch worktree. The "Reached at" column below is the first
  commit at which a case meets `Defect(IrIndex)`, and "Before" is its first
  refusal one commit earlier. At `ab9cde96^` 185 of the 187 are refused by
  name and 2 already meet `Defect(IrIndex)`. **None compiled at
  `ab9cde96^`.** At `ab9cde96`, 160 meet the defect and 6 compile; at
  `0fd576ea` 182; at `56bed483` all 187.
- References: Mesa 26.2.1's drm-shim, built the documented way into
  `$CONNECTED_DEPS_DIR/mesa-shim`
  (`tools/mali/drm_shim_ref.sh build "$CONNECTED_DEPS_DIR/downloads/mesa-26.2.1.tar.xz" "$CONNECTED_DEPS_DIR/mesa-shim"`).
  Each reference is the representative case's pair, verbatim from the
  measure. The vertex shader is the case's own. Each was drawn with:

```sh
MESA_SHIM_WORK="$CONNECTED_DEPS_DIR/mesa-shim" tools/mali/deqp_ref.sh \
    <case>.vert <case>.frag tools/mali/testdata/deqp_irindex_<shape> --instrument shim
```

  The G310 needs no separate reference (`mali-g310-vs-g610.md`).
- `tests/mali_deqp_irindex.rs` pins the nine references. At the tip all nine
  meet exactly `Defect(IrIndex)` (`refusals_pinned`, not ignored), and each
  byte-compare is `#[ignore]`d with its site and cause.

## 2. The shapes

`Ref` is `tools/mali/testdata/deqp_irindex_<ref>.*`, and its pair is the
representative case. The counts are first-refusal cases at `3497799a`. They
sum to **187** (64 + 60 + 18 + 15 + 10 + 6 + 6 + 5 + 3).

| Ref | Cases | dEQP groups | Board (status/family) | Site (`3497799a`) | Reached at (before) |
|---|--:|---|---|---|---|
| `pair_hi_unread` | 64 | shaders.swizzles.vector_swizzles 48, shaders.conversions.vector_to_scalar 12, shaders.conversions.vector_to_vector 4 | Fail/intval 64 | `nir_fs.rs:8146` (`index`) from `src32` `:10689`, in `select_highp` `:12174` | `ab9cde96` (IntValue) 64 |
| `pair_lo_unread` | 60 | shaders.swizzles.vector_swizzles 60 | Fail/intval 60 | `nir_fs.rs:8146` (`index`) from `load_lane` `:10484`, in `select_highp`'s load-block walk `:11872` | `ab9cde96` (IntValue) 60 |
| `load_mat3` | 18 | shaders.conversions.matrix_combine 7, matrix_to_matrix 6, scalar_to_matrix 3, shaders.functions.datatypes 1, shaders.linkage 1 | Fail: FRAGCOLOR_LANE 9, ARITH 6, varylink 2, intval 1 | `nir_fs.rs:10690` (`src32`, `lv.raw` of the load) in `select_highp` `:12174` | `ab9cde96` (ColourForm) 13; `56bed483` (Arithmetic, the vertex shader's) 5 |
| `b2f16_pair` | 15 | shaders.conversions.vector_to_vector 5, matrix_combine 4, vector_combine 4, shaders.operator.bool_compare.not 2 | Fail/intval 15 | `emit.rs:487` (`emit`'s fall-through) from `select_highp` `:12247` | `ab9cde96` (IntValue 13, ColourForm 2) |
| `mat4_pair_unread` | 10 | shaders.matrix.add, sub, mul, post_increment, post_decrement: 2 each (`dynamic_{lowp,mediump}_mat4*`) | Fail: FRAGCOLOR_LANE 6, constop 4 | `nir_fs.rs:8146` (`index`) from `vectorise` `:8713` (`lane_tree` `:9353`) | `0fd576ea` (ColourForm 6, ConstantFold 4) |
| `load_vec3_cmp` | 6 | shaders.operator.float_compare.{equal, greaterThan, greaterThanEqual, lessThan, lessThanEqual}.highp_vec3_fragment, binary_operator.equal.highp_vec3_fragment | Fail/intval 6 | `nir_fs.rs:10690` (as `load_mat3`) | `ab9cde96` (ColourForm) 6 |
| `val16_abs_fold` | 6 | shaders.functions.datatypes 2, shaders.preprocessor.basic 2, shaders.preprocessor.predefined_macros 2 | Fail/intval 6 | `nir_fs.rs:10697` (`src32`, a 16-bit ALU's `val`) in `select_highp` `:12174` | **`0fd576ea` (compiled at `ab9cde96`: a regression)** 6 |
| `sqrt_op_value` | 5 | shaders.operator.exponential.sqrt 4, shaders.random.scalar_conversion.fragment 1 | Fail: ARITH 4, CONST_FOLD 1 | `nir_fs.rs:13198` (`operand`, `P::Op`'s `val`) from `op` `:13260`, `select` `:10332` | `0fd576ea` (ConstantFold) 3; already IrIndex at `ab9cde96^` 2 (`sqrt.{highp,mediump}_vec4`) |
| `mat3_uniform_conv` | 3 | shaders.matrix.mul.uniform_{highp,mediump,lowp}_mat3_float_fragment | Fail/unitype 3 | `nir_fs.rs:13112` (`conv_of`) from `op` `:13260`, `select` `:10332` | `0fd576ea` (ColourForm) 3 |
| **Total** | **187** | | | | |

### 2.1 Root causes, and the fix each needs

**`pair_hi_unread`, `pair_lo_unread` (124): a two-lane conversion reads a lane
the colour never reads.** `bvec2 in0 = bvec2(v_in0); out0 = in0.x;` (or
`.yyyy`, `bool(in0)`, `ivec3(ivec4)`). Intval step 1 converts a varying
vector's aligned lane pair with one `F2f16` of width 2 (`V2F32_TO_V2F16`,
`vectorise_int_sources`; `select_highp` gives it arity 2, `:12171`). The
varying layout (`nir_fs.rs:8020-8075`) places only the lanes in `read[loc]`,
the lanes that reach a root (the colour, an `if`'s condition, a discard),
and sets `lane_of` for those alone. The pair's other lane is dead, so
`index` finds no `lane_of` entry and returns the defect. When the unread
lane is the pair's second, `src32` asks for it first
(`DIAG index loc=0 c=1 lane_of=[Some((0,0)), None, ..]`: `pair_hi_unread`,
64). When it is the first, `select_highp`'s load-block walk, which reads
each node's `src[..op.arity()]` (one source for `F2f16`), asks for it
(`loc=0 c=0 lane_of=[None, Some((0,1)), ..]`: `pair_lo_unread`, 60). This
was reached first at `ab9cde96`, the commit that made the conversion. Before
it, the shapes were refused as `IntValue`. It is a defect the lowering
exposed, not a regression: none compiled before. Fix: pair two lanes only
when both are read (else convert the read lane alone, `F32_TO_F16` or
`V2F32_TO_V2F16` with a lane Mesa draws, per the reference). Or keep the
pair, and have the layout and `index` give the unread lane the slot Mesa's
`LD_VAR` loads (the references show which: compare the `fs.main` words of
`deqp_irindex_pair_hi_unread` and `_pair_lo_unread`).

**`b2f16_pair` (15): no encoding for a two-lane `B2f` at fp16.**
`vec2(bvec2(v))`, `mat2(bvec2, bvec2)`, `not(bvec3)` into a colour.
`select_highp` (`:12247`) makes a row `(Alu::B2f, 16, 2)`, and `emit.rs`'s
table has no arm for it (`DIAG emit B2f bits=16 width=2`), so the
fall-through `_ => Err(Defect(IrIndex))` fires. Reached at `ab9cde96`.
Fix: emit what Mesa emits for this conversion, which is the `fs.main`
words of `deqp_irindex_b2f16_pair` (the reference decides whether it is
one two-lane instruction or two one-lane ones). Then add the arm to `emit`,
or split the row in `select_highp`.

**`load_mat3`, `load_vec3_cmp` (24): a conversion reads a load that was never
emitted.** The fragment shader reads a `mat3` varying (`varying mat3
out0`, `mat2(mat3)`, `mat3(vec3, vec3, vec3)` built in the vertex shader),
or two highp `vec4` varyings swizzled to `vec3` (`v_in0.zxy` against
`v_in1.yzx`) into a mediump colour. The pair `F2f16` of lanes `(Lane(1,0),
Lane(1,1))` maps through `index` to load `k = 1`, lane 0, and that load's
`lv.raw[1]` is all `None` (`DIAG raw loc=1 c=1 k=1 l=0 comps=4|2
raw=[None; 4] nvar=3|2`). `load()` (`nir_fs.rs:12922`) was never called for
the second compacted load before `src32` read it, so the walk emits loads
only for the slots another path asked for. Reached at `ab9cde96` (19, from
ColourForm) and, for the 5 `_vertex` cases whose vertex shader was refused
by `Arithmetic` until then, at `56bed483` (intval step 2, the vertex walk).
Fix: emit the load (`load(p, f, k, lv)`) wherever `src32` reads a varying
lane, as `conversion` does, and check the place of the load against the
reference's `LD_VAR_BUF_IMM` order.

**`mat4_pair_unread` (10): the mediump vectoriser pairs an unread lane.**
`res = v_mat4 + v_coords; gl_FragColor = vec4(res[0].xyz + res[1].yzw +
res[2].zwx + res[3].wxy, 1.0)` at mediump or lowp. Column 2 is read at
`z, w, x` only. `vectorise` (`nir_fs.rs:8713`, from `lane_tree` `:9353`)
pairs its lanes `x, y`, and `index` finds no `lane_of` for `y`
(`loc=2 c=1 lane_of=[Some, None, Some, Some]`). This is the same invariant as
`pair_*_unread`, in the mediump colour path. Reached at `0fd576ea`. Before
it, the shapes were refused as ColourForm or ConstantFold. Fix: as for
`pair_*_unread`, either a single lane where the partner is unread, or the
partner's slot as Mesa loads it (the reference decides).

**`val16_abs_fold` (6): a regression of `0fd576ea`.**
`bool isOk(float a, float b, float eps) { return abs(a-b) <= eps*abs(b) +
eps; }` with `a` a constant 0.0 (`out0 = 0.0`, `null_directive`, `__FILE__`)
or a sum of a varying's lanes (`float_mat4`, `struct_nested_struct`). At
`ab9cde96` the six **compiled, and `deqp_irindex_val16_abs_fold` was
byte-equal to Mesa** (scratch compare of the reference at `ab9cde96`: 0
differences). At `0fd576ea` (constop) they meet the defect: an `Ffma` (the
fused `eps*abs(b) + eps`) reads a 16-bit `Fabs` node (`DIAG val16 n=2
node=Alu(Fabs) src=[Lane(0,0)] bits=16 width=1`, `n=6 Alu(Fadd)` and
`n=1 Alu(Fneg)` in the other two), and `select_highp` gave that node no
value. `0fd576ea` added constant folding of `0.0 - b` and the `FmaA`
fusing of `abs` operands (op_abs, op_minus). The likeliest cause is that
the `abs` is now taken as a source modifier of one reader and skipped as a
node, while the `Ffma` still reads it as a value. Unverified beyond the
DIAG. Fix: give the node its value when any reader needs one. The test is
`deqp_irindex_val16_abs_fold` byte-equal again, as at `ab9cde96`.

**`sqrt_op_value` (5): the colour walk reads an operation before making
it.** `highp vec3 in0 = v_in0.zxy; res = sqrt(in0); mediump vec4 color;
color.xyz = res; color *= 0.30;`. The mediump colour walk's `operand`
(`nir_fs.rs:13198`) reads `val[x]` of a `P::Op` that `op` has not made yet
(order of the colour tree, or an operation the walk skipped). The `vec3`
forms and `scalar_conversion.fragment.84` reach it at `0fd576ea`, before
which they were refused as ConstantFold. The two `vec4` forms already met
`Defect(IrIndex)` at `ab9cde96^` (site not checked there). Fix: make each
`P::Op` before its first reader in `op` (by the colour tree's order), and
check the result against `deqp_irindex_sqrt_op_value`.

**`mat3_uniform_conv` (3): a varying's conversion the walk never made.**
`res = u_in0 * v_coords` (a `mat3` uniform by a float varying), `vec4(res[0]
+ res[1] + res[2], 1.0)`. `conv_of` (`nir_fs.rs:13112`) reads
`lv.conv[0][0]`, the fp16 conversion of `v_coords`, and the conversions of
load 0 are all `None` (`DIAG conv_of k=0 c=0 conv=[None; 4]`). The
conversions are made for the loads the colour's leaves name
(`conversion`), and this product's varying operand is not one of them.
Reached at `0fd576ea` (from ColourForm). Fix: list the varying of a
uniform-by-varying product among the converted leaves.

### 2.2 Is it a regression of intval step 1 (`ab9cde96`)?

No case compiled at `ab9cde96^`. For 179 of the 187, the three commits
did not break a compile. They moved a named refusal (`IntValue`,
`ColourForm`, `ConstantFold`, `Arithmetic`) onto a defect behind it: the
lowering got further and found an invariant that does not hold.

| Commit | Cases | Shapes |
|---|--:|---|
| `ab9cde96` (intval step 1) | 158 | `pair_*_unread` 124, `b2f16_pair` 15, `load_mat3` 13, `load_vec3_cmp` 6 |
| `56bed483` (intval step 2, vertex walk) | 5 | `load_mat3` (`_vertex` cases) |
| `0fd576ea` (constop) | 16 | `mat4_pair_unread` 10, `sqrt_op_value` 3, `mat3_uniform_conv` 3 |
| `0fd576ea` (constop), **regression** | 6 | `val16_abs_fold`: compiled at `ab9cde96`, byte-equal to Mesa |
| before `ab9cde96` | 2 | `sqrt_op_value` (`sqrt.{highp,mediump}_vec4_fragment`) |
| **Total** | **187** | |

So section 18's suspicion holds in part. `ab9cde96` is where 158 cases
first meet the defect, mostly bool-vector shaders (`pair_*_unread`,
`b2f16_pair`), but it broke no compile. The one **true regression** is
`val16_abs_fold` (6 cases), from `0fd576ea`.

### 2.3 Forecast (HOST-ONLY)

The fix (item `irindex-fix`) removes this row of the ranked table, 187
must-pass cases. With `pair_*_unread` and `b2f16_pair` lowered, 139 of the
152 board-intval cases should compile. Whether a case then compiles or meets
its next refusal is not known until the fix. Those shaders refused by name
before the lowerings reached them, and the walk past the defect is
untested. `val16_abs_fold`'s 6 compiled byte-equal before and should compile
again. Whether any of them pass on the board is the board rerun's question,
not the host's.

## 3. Caveats

- HOST-ONLY: compile and link outcomes only.
- The site is the first `Defect(IrIndex)` raised in a compile. The causes
  are read from the scratch DIAG output and the code, without a fix: each
  "Fix" line is a direction for `irindex-fix`, checked against the
  reference, not a verified change.
- The 2 link-error cases (`shaders.preprocessor.basic.null_directive_vertex`
  and `shaders.preprocessor.predefined_macros.file_vertex`, both
  `val16_abs_fold`) meet the same fragment defect through the program link.
  Their fragment shader is the `isOk` compare of a uniform.
- The representative of `load_mat3` is a `_fragment` case
  (`conversions.matrix_to_matrix.mat3_to_mat3_fragment`). Its `_vertex`
  siblings have the same fragment shader (`varying mat3 out0`) and the
  conversion in the vertex shader.

## 4. Every case, by shape

The 187 cases (prefix `dEQP-GLES2.functional.`), grouped by dEQP group.

### `pair_hi_unread` (64)

- `shaders.conversions.vector_to_scalar`: `bvec2_to_bool_fragment`, `bvec2_to_float_fragment`, `bvec2_to_int_fragment`, `bvec3_to_bool_fragment`, `bvec3_to_float_fragment`, `bvec3_to_int_fragment`, `bvec4_to_bool_fragment`, `bvec4_to_float_fragment`, `bvec4_to_int_fragment`, `ivec2_to_int_fragment`, `ivec3_to_int_fragment`, `ivec4_to_int_fragment`
- `shaders.conversions.vector_to_vector`: `bvec4_to_bvec3_fragment`, `bvec4_to_ivec3_fragment`, `bvec4_to_vec3_fragment`, `ivec4_to_ivec3_fragment`
- `shaders.swizzles.vector_swizzles`: `mediump_bvec2_r_fragment`, `mediump_bvec2_rr_fragment`, `mediump_bvec2_s_fragment`, `mediump_bvec2_ss_fragment`, `mediump_bvec2_x_fragment`, `mediump_bvec2_xx_fragment`, `mediump_bvec3_r_fragment`, `mediump_bvec3_rb_fragment`, `mediump_bvec3_rbr_fragment`, `mediump_bvec3_rrr_fragment`, `mediump_bvec3_s_fragment`, `mediump_bvec3_sp_fragment`, `mediump_bvec3_sps_fragment`, `mediump_bvec3_sss_fragment`, `mediump_bvec3_x_fragment`, `mediump_bvec3_xxx_fragment`, `mediump_bvec3_xz_fragment`, `mediump_bvec3_xzx_fragment`, `mediump_bvec4_r_fragment`, `mediump_bvec4_rrrr_fragment`, `mediump_bvec4_s_fragment`, `mediump_bvec4_ssss_fragment`, `mediump_bvec4_x_fragment`, `mediump_bvec4_xxxx_fragment`, `mediump_ivec2_r_fragment`, `mediump_ivec2_rr_fragment`, `mediump_ivec2_s_fragment`, `mediump_ivec2_ss_fragment`, `mediump_ivec2_x_fragment`, `mediump_ivec2_xx_fragment`, `mediump_ivec3_r_fragment`, `mediump_ivec3_rb_fragment`, `mediump_ivec3_rbr_fragment`, `mediump_ivec3_rrr_fragment`, `mediump_ivec3_s_fragment`, `mediump_ivec3_sp_fragment`, `mediump_ivec3_sps_fragment`, `mediump_ivec3_sss_fragment`, `mediump_ivec3_x_fragment`, `mediump_ivec3_xxx_fragment`, `mediump_ivec3_xz_fragment`, `mediump_ivec3_xzx_fragment`, `mediump_ivec4_r_fragment`, `mediump_ivec4_rrrr_fragment`, `mediump_ivec4_s_fragment`, `mediump_ivec4_ssss_fragment`, `mediump_ivec4_x_fragment`, `mediump_ivec4_xxxx_fragment`

### `pair_lo_unread` (60)

- `shaders.swizzles.vector_swizzles`: `mediump_bvec2_gggg_fragment`, `mediump_bvec2_tttt_fragment`, `mediump_bvec2_yyyy_fragment`, `mediump_bvec3_bbg_fragment`, `mediump_bvec3_ppt_fragment`, `mediump_bvec3_zzy_fragment`, `mediump_bvec4_a_fragment`, `mediump_bvec4_aaa_fragment`, `mediump_bvec4_aaaa_fragment`, `mediump_bvec4_aaag_fragment`, `mediump_bvec4_abg_fragment`, `mediump_bvec4_ar_fragment`, `mediump_bvec4_gga_fragment`, `mediump_bvec4_gggg_fragment`, `mediump_bvec4_q_fragment`, `mediump_bvec4_qpt_fragment`, `mediump_bvec4_qqq_fragment`, `mediump_bvec4_qqqq_fragment`, `mediump_bvec4_qqqt_fragment`, `mediump_bvec4_qs_fragment`, `mediump_bvec4_ttq_fragment`, `mediump_bvec4_tttt_fragment`, `mediump_bvec4_w_fragment`, `mediump_bvec4_www_fragment`, `mediump_bvec4_wwww_fragment`, `mediump_bvec4_wwwy_fragment`, `mediump_bvec4_wx_fragment`, `mediump_bvec4_wzy_fragment`, `mediump_bvec4_yyw_fragment`, `mediump_bvec4_yyyy_fragment`, `mediump_ivec2_gggg_fragment`, `mediump_ivec2_tttt_fragment`, `mediump_ivec2_yyyy_fragment`, `mediump_ivec3_bbg_fragment`, `mediump_ivec3_ppt_fragment`, `mediump_ivec3_zzy_fragment`, `mediump_ivec4_a_fragment`, `mediump_ivec4_aaa_fragment`, `mediump_ivec4_aaaa_fragment`, `mediump_ivec4_aaag_fragment`, `mediump_ivec4_abg_fragment`, `mediump_ivec4_ar_fragment`, `mediump_ivec4_gga_fragment`, `mediump_ivec4_gggg_fragment`, `mediump_ivec4_q_fragment`, `mediump_ivec4_qpt_fragment`, `mediump_ivec4_qqq_fragment`, `mediump_ivec4_qqqq_fragment`, `mediump_ivec4_qqqt_fragment`, `mediump_ivec4_qs_fragment`, `mediump_ivec4_ttq_fragment`, `mediump_ivec4_tttt_fragment`, `mediump_ivec4_w_fragment`, `mediump_ivec4_www_fragment`, `mediump_ivec4_wwww_fragment`, `mediump_ivec4_wwwy_fragment`, `mediump_ivec4_wx_fragment`, `mediump_ivec4_wzy_fragment`, `mediump_ivec4_yyw_fragment`, `mediump_ivec4_yyyy_fragment`

### `load_mat3` (18)

- `shaders.conversions.matrix_combine`: `bool_bvec2_int_vec4_bool_to_mat3_vertex`, `bool_float_int_vec2_bool_bvec2_float_to_mat3_vertex`, `float_bvec4_ivec2_bool_bool_to_mat3_vertex`, `ivec3_ivec3_ivec3_to_mat3_vertex`, `vec2_ivec2_float_float_int_bool_bool_to_mat3_vertex`, `vec3_vec3_vec3_to_mat3_fragment`, `vec3_vec3_vec3_to_mat3_vertex`
- `shaders.conversions.matrix_to_matrix`: `mat2_to_mat3_vertex`, `mat3_to_mat2_fragment`, `mat3_to_mat3_fragment`, `mat3_to_mat3_vertex`, `mat3_to_mat4_fragment`, `mat4_to_mat3_vertex`
- `shaders.conversions.scalar_to_matrix`: `bool_to_mat3_vertex`, `float_to_mat3_vertex`, `int_to_mat3_vertex`
- `shaders.functions.datatypes`: `float_mat3_fragment`
- `shaders.linkage`: `varying_type_mat3`

### `b2f16_pair` (15)

- `shaders.conversions.matrix_combine`: `bvec2_bvec2_to_mat2_fragment`, `bvec3_float_to_mat2_fragment`, `bvec4_bvec4_bvec4_bvec4_to_mat4_fragment`, `vec2_bvec2_to_mat2_fragment`
- `shaders.conversions.vector_combine`: `bvec2_bvec2_to_vec4_fragment`, `bvec2_float_to_vec3_fragment`, `bvec3_float_to_vec4_fragment`, `vec2_bvec2_to_vec4_fragment`
- `shaders.conversions.vector_to_vector`: `bvec2_to_vec2_fragment`, `bvec3_to_vec2_fragment`, `bvec3_to_vec3_fragment`, `bvec4_to_vec2_fragment`, `bvec4_to_vec4_fragment`
- `shaders.operator.bool_compare.not`: `bvec3_fragment`, `bvec4_fragment`

### `mat4_pair_unread` (10)

- `shaders.matrix.add`: `dynamic_lowp_mat4_float_fragment`, `dynamic_mediump_mat4_float_fragment`
- `shaders.matrix.mul`: `dynamic_lowp_mat4_float_fragment`, `dynamic_mediump_mat4_float_fragment`
- `shaders.matrix.post_decrement`: `dynamic_lowp_mat4_fragment`, `dynamic_mediump_mat4_fragment`
- `shaders.matrix.post_increment`: `dynamic_lowp_mat4_fragment`, `dynamic_mediump_mat4_fragment`
- `shaders.matrix.sub`: `dynamic_lowp_mat4_float_fragment`, `dynamic_mediump_mat4_float_fragment`

### `load_vec3_cmp` (6)

- `shaders.operator.binary_operator.equal`: `highp_vec3_fragment`
- `shaders.operator.float_compare.equal`: `highp_vec3_fragment`
- `shaders.operator.float_compare.greaterThan`: `highp_vec3_fragment`
- `shaders.operator.float_compare.greaterThanEqual`: `highp_vec3_fragment`
- `shaders.operator.float_compare.lessThan`: `highp_vec3_fragment`
- `shaders.operator.float_compare.lessThanEqual`: `highp_vec3_fragment`

### `val16_abs_fold` (6)

- `shaders.functions.datatypes`: `float_mat4_fragment`, `struct_nested_struct_fragment`
- `shaders.preprocessor.basic`: `null_directive_fragment`, `null_directive_vertex`
- `shaders.preprocessor.predefined_macros`: `file_fragment`, `file_vertex`

### `sqrt_op_value` (5)

- `shaders.operator.exponential.sqrt`: `highp_vec3_fragment`, `highp_vec4_fragment`, `mediump_vec3_fragment`, `mediump_vec4_fragment`
- `shaders.random.scalar_conversion.fragment`: `84`

### `mat3_uniform_conv` (3)

- `shaders.matrix.mul`: `uniform_highp_mat3_float_fragment`, `uniform_lowp_mat3_float_fragment`, `uniform_mediump_mat3_float_fragment`
