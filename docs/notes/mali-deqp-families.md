<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the dEQP-GLES2 compile refusals by family and shape, measured on the host

**HOST-ONLY.** Plan #377 item `measure` (issue #20, ROADMAP stage 5, issue
#7). Every count in this note comes from `deqp-gles2` run on a host against
the Mali back end's C pair on the Linux OSAL, which keeps the command stream
instead of executing it: nothing is drawn, every `glReadPixels` reads black
and every dEQP render verdict of the run is meaningless. What is real, and
what this note counts, is each shader's compile and each program's link
outcome and our info log, the text of a `MaliError`
(`src/shaderc/online/mali/mod.rs`). A board status quoted here (Fail,
NotSupported) is the NanoPC-T6's own, from the slices, never the host's.

- Tip: `abcc9013` (plan #377's integration tip at the time of the run, a
  descendant of origin/main `1f051a7b` with the merged items of plans #373
  and #377; the compiler under `src/shaderc/online/mali/` as at that
  commit).
- CTS: VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` (the commit
  `DEQP_WORK` pins, `tools/cts/deps.toml`) with `cts-yakogl-target.patch`
  applied uncommitted, its external sources fetched.
- Cases: the 10,597 cases of the four slices
  `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/` whose `refusals.tsv`
  text holds `online shaderc: the code generator refused` (the board tree at
  `37305608`, docs/es2-cts-failures.md section M; the 14 Pass cases that log
  a refusal are among them, see section 2).
- Command (run on 2026-11-16, 4 cores, about 4 minutes for the run itself):

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$TMPDIR/deqp-host-measure-abcc9013" --jobs 3 --chunk 100
```

The script (`tools/mali/deqp-host-measure.sh`, its header says the rest)
builds `deqp-gles2` for the host as step 1 of `tools/cts/host-deqp-run.sh`
does and the library from a generated CMake project, runs the case list in
100-case chunks with `--deqp-log-shader-sources=enable` on 256x256 pbuffers
(the board's `DEQP_SURFACE`; at 64x64 the 53 `shaders.invariance` cases end
NotSupported before compiling anything), and writes per case the first
refusal, its family, its shape and both shader sources (`measure.tsv`,
`src/<case>.vert|.frag`), per shape one representative pair (`shapes/`), and
`summary.md` with the tables below. The per-case list (2.6 MB) is not
committed: rerun the command to get it.

## 1. The library, and how it differs from the Rust feature set

The plan names the Rust feature set `host-test,gpu-mali,shaderc-online,
no-shader-table` (`MALI_CONFIG` of `tools/check.sh`). `host-test` links
`std` and starts without a GPU back end: it is a compile error in the C
archive (Cargo.toml's note on the feature), and the `yakogl-capi` crate does
not forward it. The C pair dEQP loads is therefore built with the capi
crate's `osal-linux gpu-mali shaderc-online no-shader-table report-es2`, as
`tools/hostharness/glmark2/mali/CMakeLists.txt` builds the glmark2 host
archive, plus the shared pair the top-level `CMakeLists.txt` links
(`--whole-archive`, `tools/yakogl.ver`, `-lm`). The top-level CMake takes
`powervr|vivante|v3d` only, so the script generates its own
`CMakeLists.txt` under `build/host-deqp-mali/lib-src/`. Without `host-test`
the crate has no built-in compiler regions and `src/osal/linux.c` answers
the PowerVR's OSAL contract, so a C shim in the same project (`host_osal.c`,
the `GM2_MALI` table of `tools/hostharness/glmark2/shim.c`) lends the
compiler its scratch (4 MiB) and keep (512 KiB) regions at the library's
sizes and answers `OSAL_QUERY_MALI_GPU` (the T6's counts, as
`src/mali/backend_tests.rs`'s mock) and `OSAL_QUERY_MALI_TILER_HEAP`; its
submit completes at once. The compiler is the same code either way: the
features that differ (`host-test`, `report-es2`) touch no file under
`src/shaderc/online/mali/`.

## 2. What the host reproduced

HOST-ONLY. 10,597 cases asked, 10,597 began, 10,597 ended with a compile
refusal in their log on the host. No case was lost to a crash, a hang or a
timeout (106 processes: 4 exited 0, 102 exited 1, dEQP's "not every case
passed"). Of them:

- **10,577 have the same first refusal as on the board** (same stage, same
  `MaliError` text; a board text cut at the info log's end counts as equal
  when it is a prefix).
- **20 have a different first refusal on the host** (the tip moved from
  `37305608` to `abcc9013`): all 20 are `shaders.random.*` fragment or
  vertex shaders, and all 20 moved between the constant-folding and
  integer refusals (`CONST_FOLD`, `constop`, `intval`, `ARITH`, `LOCALS`,
  `varylink`). Section 11 lists them. They are counted below under their
  HOST family, so a family's host count can differ from section M's board
  count by a few cases.
- **0 not run, 0 without a refusal.**

The 14 Pass cases of section M that log a refusal are in the 10,597 (9
`shader_api.shader_source.*`, 5 expected-failure cases): the tally below
counts them in the family of their refusal with a board status of Pass, so
a family's Fail + NotSupported can be a few short of its case count.

### Families (HOST-ONLY)

| Family | Owner | Cases | Fail (board) | NotSupported (board) |
|---|---|---:|---:|---:|
| intval | this plan | 3224 | 3179 | 41 |
| varylink | this plan | 1068 | 1062 | 5 |
| unitype | this plan | 820 | 724 | 96 |
| uniarray | this plan | 272 | 267 | 5 |
| constop | this plan | 738 | 723 | 6 |
| mixprec | this plan | 251 | 210 | 41 |
| colourpair | this plan | 170 | 170 | 0 |
| loops | this plan | 774 | 106 | 668 |
| misc | this plan | 29 | 29 | 0 |
| ARITH (`arithmetic other than ...`, `three or more additions ...`, `a fragment shader operand of a form ... no reference draws`) | #367 | 1581 | 1522 | 59 |
| CONST_FOLD (`a constant product in a shape Mesa's algebraic rules move or fold`) | #367 | 465 | 465 | 0 |
| FMA (`a fused multiply-add other than a matrix product's`) | #367 | 100 | 100 | 0 |
| LOCALS (`more than 32 local variables`) | #367 | 160 | 144 | 16 |
| FRAGCOLOR_LANE (`a gl_FragColor lane other than the varying's own lane`) | #367 | 793 | 793 | 0 |
| TEX_COORD (`a texture coordinate other than ...`) | #367 | 109 | 109 | 0 |
| TEX_VERTEX (`a texture look-up other than texture2D(sampler2D, coordinate)`) | #367 | 27 | 27 | 0 |
| CONTROL_FLOW (`control flow (if, a loop, a return before the end)`) | #367 | 15 | 13 | 2 |
| other (`a vertex shader whose attribute numbering waits for a fold ...`, a text newer than the slices) | - | 1 | 1 | 0 |
| **Total** | | **10597** | **9644** | **939** |

9,644 Fail + 939 NotSupported + 14 Pass = 10,597: the same 9,644 and 939
section M.1 counts as "have a compile or link refusal in their own log".

The families are the refusal texts the plan names; the mapping text to
family is the `FAMILIES` table at the top of the script's Python. The #367
rows are listed so that every refused case of the slices is accounted for;
their shapes are not classified here (one row `367` each in `summary.md`).

## 3. How a shape is named

A shape is the code construct of the refusing shader that a lowering has to
handle, read from the shader source by the rules of the script's `RULES`
table, tried in order per family; the first rule that holds names the
shape. The rules are regular expressions over the GLSL (declarations of
uniforms and their types, `for`/`while`/`do`, `ivec`/`bvec` constructors,
the relational built-ins, `int(`/`bool(` conversions, constant built-in
calls, literals), not the compiler's refusal site: for the integer family
the compiler names no site, so the shape is the dominant int or bool
construct in the order a lowering would meet it (a random shader first,
then an int or bool uniform, an int loop counter, int vector operators,
bool vectors, conversions, the harness, logic, float compares). Two shapes
are dEQP's own harness rather than a case's feature:

- `harness_isok` (intval): the fragment half of every shader-library
  (`.test`) case, `bool RES = isOk(out0, ref_out0, 0.05); gl_FragColor =
  vec4(RES, RES, RES, 1.0);` with nothing else int or bool in the shader.
  The whole `shaders.preprocessor`, `shaders.constants`, `shaders.functions`
  and friends sit behind this one shape when their vertex shader compiles.
- `fs_uniform_compare_harness` (mixprec): the fragment half of every
  `uniform_api.*` render case, `mediump float compare_float(...) { return
  abs(a - b) < 0.05 ? 1.0 : 0.0; }` products into `gl_FragColor`.
- `random_mixed` (intval, constop): a `shaders.random.*` or
  `fragment_ops.interaction.*` generated shader, which mixes int and bool
  globals, `ivec`/`bvec` constructors with float and bool arguments,
  conversions and constant folds in one shader; no single construct names
  it. These need the general int lowering, not a shape-sized one.

"Next refusal" is what the partner shader of the same program says: dEQP
compiles both shaders before it links, so when the vertex shader is the
refusing one, the fragment shader's own refusal is known (its family is
given); `-` is a partner that compiled. The link and the draw are not
measured: a case whose partner compiled may still be refused at the link
(`compile_vs_linked`) or at the draw. So "unblocks N" below means: N cases
have this shape as their FIRST refusal; when the partner also refuses,
those cases move to the partner's family instead of passing.

Example cases are the first case, in mustpass order, of the shape's largest
dEQP group; the shape's pair under `tools/mali/testdata/deqp_shapes/` is
that case's shader pair verbatim from the `.qpa` (section 10).

## 4. intval: `an integer or boolean value is not lowered yet` (`MaliError::IntValue`, `mod.rs:413`) (HOST-ONLY)

HOST-ONLY, 3,224 cases (board: FS 1,861 Fail + 37 NS, VS 1,317 Fail + 4 NS;
host 3,179 Fail + 41 NS + 4 Pass). Plan #367 holds glmark2's int-counter
loops (`gm2_intloop_*`) and the `&&` of two float compares in a one-armed
`if` (`gm2_cf_ifand_vs`); `docs/notes/mali-int-loops.md` lists the forms
no #367 reference touches (idiv, int arithmetic, dynamic subscripts, int
literals, short-circuit logic).

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks (first refusal) / next refusal |
|---|---:|---|---|---|---|---|
| `ivec_operator`: `int` or `ivecN` locals through `+ - * / % ++ -- += <` and the int relational built-ins, converted from a float varying and back (`shaders.operator.{binary,unary}_operator`, `int_compare`, `sequence`; 936 of them) | 981 | 981 / 0 | 444 / 537 | this plan | `shaders.operator.binary_operator.add.highp_int_fragment` | 948 to a compiling partner; 31 to another intval shader, 2 to ARITH |
| `random_mixed`: generated shaders mixing int and bool globals, `ivec`/`bvec` constructors with float and bool arguments and conversions | 771 | 771 / 0 | 589 / 182 | this plan (needs the whole int lowering, not a shape) | `shaders.random.all_features.fragment.11` | 613; 53 to intval, 50 to constop (and the CONST_FOLD and ARITH forms #367 lowers) |
| `harness_isok`: the shader-library fragment harness alone (`bool RES = isOk(...)`, a float compare to a bool to `vec4(RES, ...)`) | 448 | 444 / 0 (4 Pass) | 0 / 448 | this plan | `shaders.preprocessor.basic.empty_directive_fragment` | 448: `shaders.preprocessor` 252, `shaders.functions` 57, `shaders.constants` 49, `shaders.scoping`, `shaders.declarations`, ...; partner compiled in all 448 |
| `bvec_relational`: `bvecN` values from `lessThan`/`equal`/`notEqual`/`any`/`all`/`not` or a `bvecN(...)` constructor, written to a colour or varying as `vec4(bvec)` | 330 | 330 / 0 | 163 / 167 | this plan | `shaders.operator.binary_operator.equal.bvec2_fragment` | 245; 46 to ARITH, 21 to intval |
| `int_uniform`: a `uniform int`/`ivecN` (the swizzle cases' `ivecN ref_out0` compared with `==`, `uniform_api`'s `compare_int`, the conversions' `uniform int`) | 315 | 275 / 40 | 9 / 306 | this plan (`docs/notes/mali-int-uniforms.md`) | `shaders.swizzles.vector_swizzles.mediump_ivec2_gggg_fragment` | 315, partner compiled in all |
| `bool_uniform`: a `uniform bool`/`bvecN` (`bvecN ref_out0 == out0`, `compare_bool(u_var, true)`) | 212 | 212 / 0 | 27 / 185 | this plan | `shaders.swizzles.vector_swizzles.mediump_bvec2_gggg_fragment` | 188; 20 to ARITH, 2 to FRAGCOLOR_LANE |
| `scalar_conversion`: `int(float)`, `bool(float)`, `float(int)`, `float(bool)` on scalars (`shaders.operator.*` bool results, `shaders.conversions.scalar_to_scalar`, `builtin_variable.max_*` reads of an int constant) | 105 | 105 / 0 | 69 / 36 | this plan | `shaders.operator.binary_operator.equal.bool_fragment` | 87; 18 to intval |
| `step_sign`: `step(edge, x)` or `sign(x)` on floats (the compiler's compare-to-float path) | 21 | 21 / 0 | 0 / 21 | this plan | `shaders.operator.common_functions.step.highp_float_fragment` | 21 |
| `bool_literal`: `true`/`false` as a value (a constant `?:`, `if (false, ...)`, a bool struct member, a constant-condition return) | 15 | 15 / 0 | 13 / 2 | this plan | `shaders.constant_expressions.complex_types.nested_struct_vertex` | 6; 7 to intval, 2 to CONTROL_FLOW |
| `int_loop_counter`: `for (int i = 0; i < N; i++)` with `i == 1`/`i > 1` in the body and a `return` or `break`/`continue` (`shaders.functions.control_flow`) | 11 | 11 / 0 | 4 / 7 | #367 (`gm2_intloop_*` hold the counter; the compare and the return in the body are not in any #367 reference) | `shaders.functions.control_flow.mixed_return_break_continue_fragment` | 7; 4 to intval |
| `float_compare`: a float compare as an `if` condition or a bool local, nothing else int (`shaders.conditionals.if`, `shaders.functions`) | 8 | 7 / 1 | 0 / 8 | this plan | `shaders.conditionals.if.if_else_fragment` | 8 |
| `other`: `#define FOO(a, b) (1 a 9) b 2` int arithmetic through the preprocessor, `__LINE__`, a `const int` expression | 5 | 5 / 0 | 2 / 3 | this plan | `shaders.preprocessor.predefined_macros.line_1_fragment` | 3; 2 to intval |
| `bool_logic`: `&&`/`||` of float compares in an `if` (`compound_statement`, `sequence_statement`) | 2 | 2 / 0 | 0 / 2 | this plan (#367: only the `&&` of two uniform compares in a one-armed `if`) | `shaders.conditionals.if.compound_statement_fragment` | 2 |

What a lowering order gets, by first refusal: the general int and bool
value lowering (registers, `ICMP`/`CSEL`, conversions, the relational
built-ins: steps 1 and 2 of `mali-int-loops.md`'s order) is the only thing
that moves `ivec_operator`, `random_mixed`, `bvec_relational` and
`scalar_conversion` (2,187 cases); `harness_isok` (448) and `float_compare`
(8) need only a float compare to a bool to a float, the smallest step; the
uniform shapes (527) need the int and bool uniform words of
`mali-int-uniforms.md` on top.

## 5. varylink (`mod.rs:499-506`): 1,068 cases (HOST-ONLY)

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `equal_lanes`: `two varying lanes of equal value (Mesa's link merges them)`; the vertex_arrays harness `v_color = vec4(u_colorScale * color, 1.0)` with `color = vec3(1.0)` scaled by one attribute, `draw.random`, the swizzle cases' repeated lanes | 553 | 548 / 5 | 553 / 0 | this plan | `vertex_arrays.single_attribute.first.byte_first24_offset16_stride17_quads256` | 454; 35 to FRAGCOLOR_LANE, 34 to intval |
| `equal_lanes_broadcast`: the same text where the varying is a broadcast `vecN(scalar)` (`v_color = vec3(res)`, `vec4(out0)` of a swizzle result) | 104 | 104 / 0 | 104 / 0 | this plan | `shaders.swizzles.vector_swizzles.mediump_ivec2_gggg_vertex` | 29; 69 to ARITH, 4 to FRAGCOLOR_LANE |
| `lane_no_attribute`: `a varying lane computed from no attribute (a constant or a uniform alone ...)`; `vertex_arrays.multiple_attributes` (`color.rg` from attributes, `.b` constant times a uniform), `draw.*`'s constant colour lanes | 252 | 252 / 0 | 252 / 0 | this plan (`mali-varying-constants.md` lowers the flat uniform-only lane; a flat lane beside an interpolated one is what is left) | `vertex_arrays.multiple_attributes.attribute_count.2` | 245; 3 to constop, 3 to CONST_FOLD |
| `position_unwritten`: `a gl_Position lane left unwritten`; the operator vertex harness: `gl_Position = a_position; ... color.xyz = res; v_color = color` with a `vec4(0.0, 0.0, 0.0, 1.0)` base (`shaders.operator.{unary_operator,common_functions,angle_and_trigonometry,geometric,exponential}` vertex cases) | 155 | 155 / 0 | 155 / 0 | this plan (study-varylink: the text names gl_Position but the shape is a varying built from a constant base with some lanes written) | `shaders.operator.angle_and_trigonometry.acos.highp_vec2_vertex` | 154; 1 to intval |
| `varying_unwritten`: `a varying lane left unwritten`; a declared varying never written (`shaders.linkage.varying_{2,3,4}`, `declarations.invalid_declarations.varying_in_fragment_main`) | 4 | 3 / 0 (1 Pass) | 4 / 0 | this plan | `shaders.linkage.varying_2` | 3; 1 to CONTROL_FLOW |

## 6. unitype (`mod.rs:415`, `:422`, `:425`): 820 cases (HOST-ONLY)

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `vs_mat_uniform`: VS `uniform mat2`/`mat3` (`shaders.matrix.*_vertex` with a `uniform` operand, `uniform_api` mat2/mat3) | 138 | 138 / 0 | 138 / 0 | this plan | `shaders.matrix.add.dynamic_highp_mat2_mat2_vertex` | 115; 12 to ARITH, 11 to mixprec |
| `vs_struct_uniform`: VS `uniform structType` (members float, vec, int, ivec, bool, arrays, nested structs, samplers; `uniform_api.*`, `shaders.struct.{local,uniform}`, `light_amount`) | 134 | 124 / 10 | 134 / 0 | this plan | `uniform_api.value.assigned.basic_array_assign_full.array_in_struct.int_ivec4_both` | 53; 39 to ARITH, 33 to unitype (the fragment half's struct) |
| `fs_mat_uniform`: mediump FS `uniform mat2`/`mat3`/`mat4` (`shaders.matrix.*_fragment`) | 119 | 119 / 0 | 0 / 119 | this plan | `shaders.matrix.add.dynamic_highp_mat2_mat2_fragment` | 119 |
| `vs_int_uniform`: VS `uniform int`/`ivecN` (`shaders.indexing.*` `ui_zero..ui_three` subscripts, `uniform_api`, random) | 117 | 33 / 84 | 117 / 0 | this plan (`mali-int-uniforms.md`, `mali-uniform-index.md`) | `shaders.indexing.matrix_subscript.mat2_dynamic_write_dynamic_loop_read_vertex` | 80; 18 to ARITH, 10 to FRAGCOLOR_LANE |
| `vs_bool_uniform`: VS `uniform bool`/`bvecN` (`uniform_api`) | 67 | 67 / 0 | 67 / 0 | this plan | `uniform_api.value.assigned.by_pointer.get_uniform.basic.bvec2_api_float_both` | 2; 33 to ARITH, 32 to FRAGCOLOR_LANE |
| `fs_struct_uniform`: mediump FS `uniform structType` | 55 | 55 / 0 | 0 / 55 | this plan | `uniform_api.value.assigned.basic_array_assign_full.array_in_struct.bool_bvec4_fragment` | 55 |
| `vs_mediump_struct`: `a vertex shader uniform that is not highp ... struct member`: a mediump struct uniform (`uniform_api` float/vec structs, `shaders.linkage.uniform_struct*`) | 53 | 53 / 0 | 53 / 0 | this plan | `uniform_api.value.assigned.by_pointer.get_uniform.basic_struct.float_vec4_both` | 1; 20 to intval, 16 to FRAGCOLOR_LANE, 16 to ARITH |
| `fs_int_uniform`: mediump FS `uniform int`/`ivecN` outside the admitted forms (`uniform_api` `compare_int`, `negative_api.shader.uniform_matrixfv_*`) | 52 | 52 / 0 | 0 / 52 | this plan | `uniform_api.value.assigned.basic_array_assign_full.basic_array.int_fragment` | 52 |
| `fs_bool_uniform`: mediump FS `uniform bool`/`bvecN` (and bool arrays) | 33 | 33 / 0 | 0 / 33 | this plan | `uniform_api.value.assigned.basic_array_assign_full.basic_array.bool_fragment` | 33 |
| `vs_mediump_array`: a mediump VS uniform array (`uniform mediump vec4 u_var[3]`, `float u_arr[4]` read at a run-time index) | 25 | 23 / 2 | 25 / 0 | this plan | `uniform_api.value.assigned.basic_array_assign_full.basic_array.vec4_both` | 3; 11 to mixprec, 11 to ARITH |
| `fs_highp_uniform`: `uniform highp vec4 u_color; gl_FragColor = u_color;` (`fragment_ops.scissor`): a highp uniform stored whole to the lowp colour | 17 | 17 / 0 | 0 / 17 | this plan | `fragment_ops.scissor.clear_color` | 17 (and the 17 scissor cases are otherwise plain draws) |
| `fs_other`: random fragment shaders with `uniform mediump float`/`vec` read in a form the text excludes, `builtin_variable.depth_range_fragment` (`gl_DepthRange`) | 7 | 7 / 0 | 0 / 7 | this plan | `shaders.random.basic_expression.combined.78` | 7 |
| `fs_vec_array`: mediump FS `uniform vec2 u_arr[4]` read at constant indices | 2 | 2 / 0 | 0 / 2 | this plan | `shaders.indexing.uniform_array.vec2_static_read_fragment` | 2 |
| `vs_mediump_basic`: `builtin_variable.depth_range_vertex` (`gl_DepthRange`, a mediump built-in uniform) | 1 | 1 / 0 | 1 / 0 | this plan | `shaders.builtin_variable.depth_range_vertex` | 1 |

## 7. uniarray (`mod.rs:417`, `:419`): 272 cases (HOST-ONLY)

Companion of `docs/notes/mali-deqp-uniarray.md` (study-uniarray), which read
the shapes from the case names; these are the measured ones.

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `bool_array`: `uniform bool u_var[3]` / `bvecN[]` read at constant indices through `compare_bool` (`uniform_api`) | 95 | 95 / 0 | 86 / 9 | this plan | `uniform_api.value.assigned.basic_array_assign_full.basic_array.bool_both` | 0 alone: 43 to ARITH, 32 to unitype, 9 to uniarray (the fragment half) |
| `array_in_struct`: a struct uniform with array members (`m2[3]`, `m3[3]`, nested) read at constant indices | 77 | 77 / 0 | 53 / 24 | this plan | `uniform_api.value.assigned.basic_array_assign_full.array_in_struct.float_vec4_both` | 25; 26 to ARITH, 23 to uniarray |
| `int_array`: `uniform int u_var[3]` / `ivecN[]` at constant indices | 46 | 46 / 0 | 46 / 0 | this plan | `uniform_api.value.assigned.basic_array_assign_full.basic_array.int_both` | 0 alone: 23 to unitype, 23 to ARITH |
| `float_array`: `uniform float u_var[3]` at constant indices (`uniform_api`, `shaders.indexing.uniform_array.float_static_*`, `state_query`) | 36 | 36 / 0 | 24 / 12 | this plan (`mali-uniform-arrays.md` lowered the run-time index form) | `uniform_api.value.assigned.basic_array_assign_full.basic_array.float_both` | 14; 11 to uniarray, 11 to ARITH |
| `vec_array`: `uniform vec2`/`vec3 u_arr[4]` at constant or run-time indices (`shaders.indexing.uniform_array`, random) | 12 | 8 / 4 | 8 / 4 | this plan | `shaders.indexing.uniform_array.vec2_dynamic_read_fragment` | 8; 2 to ARITH, 2 to mixprec |
| `runtime_index`: `a uniform read at a run-time index other than ...`: a mediump FS `vec4 u_arr[4]` read four times at int-uniform indices, or in a loop | 3 | 2 / 1 | 0 / 3 | this plan (`mali-uniform-index.md`) | `shaders.indexing.uniform_array.float_static_loop_read_fragment` | 3 |
| `struct_array`: `uniform structType u_var[2]` (an array of structs) | 2 | 2 / 0 | 2 / 0 | this plan | `shaders.struct.uniform.nested_struct_array_vertex` | 1; 1 to ARITH |
| `mat_array`: `uniform mat3 u_var0[5]` | 1 | 1 / 0 | 1 / 0 | this plan | `uniform_api.random.10` | 0 (1 to ARITH) |

The `_both` cases of `uniform_api` refuse the vertex shader first and the
fragment shader second with the fragment family (`unitype` or `uniarray`),
so a uniarray lowering alone moves few of them: their fragment halves need
the matching unitype or uniarray fragment form too.

## 8. constop (`mod.rs:449`, `:440`, `:531`): 738 cases (HOST-ONLY)

Companion of `docs/notes/mali-deqp-constop.md` (study-constop), which
reconstructed the generators; these are the measured shapes. #367 holds
`CONST_FOLD` and `ARITH_CONST` as glmark2 meets them (that note's section
5); all constop shapes below are this plan's.

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `zero_operand`: a constant operation with a `0.0` operand: the operator harness's `res = float(0.0)` / `vecN(0.0)` initialisers and `vec4(0.0, 0.0, 0.0, 1.0)` colour base folded with `color * k + 0.5` (`shaders.operator`, `shaders.matrix` increments, `shader_api.shader_source`'s `gl_Position = vec4(0.0)`) | 225 | 216 / 0 (9 Pass) | 18 / 207 | this plan | `shaders.operator.angle_and_trigonometry.degrees.highp_float_fragment` | 223; 1 to intval |
| `random_mixed`: generated shaders whose constant globals fold through int and bool arguments and non-normal results | 202 | 202 / 0 | 20 / 182 | this plan | `shaders.random.all_features.fragment.43` | 197; 2 to intval, 2 to constop |
| `neg_zero_variable`: `a -0.0 that a comparison reads through a variable ...` (`mod.rs:440`); random shaders with `asin(0.0)`, `ivec4(true, 0.0, ...)` folds | 116 | 116 / 0 | 70 / 46 | this plan (`NegZeroShared` in `mali-deqp-constop.md`) | `shaders.random.all_features.fragment.13` | 103; 7 to intval, 3 to CONST_FOLD |
| `const_call_common`: `const float cval = <abs,sign,floor,ceil,fract,mod,min,max,clamp,mix,step,smoothstep>(constants)` (`shaders.constant_expressions.builtin_functions.common`) | 94 | 94 / 0 | 29 / 65 | this plan | `shaders.constant_expressions.builtin_functions.common.ceil_float_fragment` | 65; 29 to intval |
| `const_call_transcendental`: the same with `exp`, `log`, `pow`, `sqrt`, `inversesqrt`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `radians`, `degrees` | 32 | 32 / 0 | 12 / 20 | this plan | `shaders.constant_expressions.builtin_functions.exponential.inversesqrt_float_fragment` | 20; 12 to intval |
| `const_call_geometric`: `length`, `distance`, `dot`, `cross`, `normalize`, `faceforward`, `reflect`, `refract`, `matrixCompMult` on constants | 24 | 24 / 0 | 24 / 0 | this plan | `shaders.constant_expressions.builtin_functions.geometric.distance_vec2_vertex` | 0 (all 24 to intval: the partner is the bool harness) |
| `const_other`: a constant fold elsewhere: `x == 1. && y == 2.` on constants, `shaders.loops` constant bodies, `shaders.matrix` constant operands, `shaders.function` aliasing | 24 | 18 / 6 | 2 / 22 | this plan | `shaders.loops.do_while_constant_iterations.basic_highp_float_fragment` | 24 |
| `const_call_other`: `const bool cval = equal(ivec4(1), ivec4(2)).w` and the other relational built-ins on int constants | 18 | 18 / 0 | 18 / 0 | this plan | `shaders.constant_expressions.builtin_functions.vector_relational.equal_ivec2_vertex` | 0 (18 to intval) |
| `fp16_constant`: `a constant fp16 does not hold ...` at the relink of `shaders.matrix.sub.const_*_mat3_mat3_vertex` | 3 | 3 / 0 | 0 / 3 | this plan (`Fp16Inexact`) | `shaders.matrix.sub.const_highp_mat3_mat3_vertex` | 3 |

## 9. mixprec (`mod.rs:428`, `:525`): 251 cases (HOST-ONLY)

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `fs_uniform_compare_harness`: `a fragment shader that mixes highp and mediump or lowp`: the `uniform_api` fragment harness (`compare_float`'s `abs(a - b) < 0.05 ? 1.0 : 0.0` products, `compare_bool`'s `a == b ? ...`, a mediump `v_vtxOut`), no `highp` declared: the mix is the compiler's own precision for the select or the bool | 186 | 186 / 0 | 0 / 186 | this plan (`mali-precision.md`, `mali-mixed-precision.md`) | `uniform_api.value.assigned.basic_array_assign_full.array_in_struct.sampler2D_samplerCube_both` | 186 (and these are the fragment halves behind the `_both` cases of sections 6 and 7) |
| `fs_varying_rgb_one`: `a fragment varying load or conversion of a form no reference draws`: `gl_FragColor = vec4(res.rgb, 1.0)` where `res` came from a mediump varying through a loop (`shaders.loops.*_fragment`, the 23 loop cases of section M.2 among them) | 54 | 13 / 41 | 0 / 54 | this plan | `shaders.loops.do_while_constant_iterations.basic_highp_int_fragment` | 54 |
| `fs_mixed_other`: random fragment shaders and `conversions.vector_combine.float_int_to_vec2_fragment` with a declared `highp` beside mediump values | 6 | 6 / 0 | 0 / 6 | this plan | `shaders.random.conditionals.fragment.27` | 6 |
| `fs_varying_other`: the varying-load text on a `return` form (`shaders.return.conditional_return_always_fragment`) or a random swizzle | 3 | 3 / 0 | 0 / 3 | this plan | `shaders.return.conditional_return_always_fragment` | 3 |
| `fs_varying_plain`: the varying-load text on `gl_FragColor = v_color;` alone (the vertex shader's varying precision is the shape) | 2 | 2 / 0 | 0 / 2 | this plan | `shaders.random.basic_expression.vertex.8` | 2 |

## 10. colourpair (`mod.rs:523`, `:528`): 170 cases (HOST-ONLY)

Both texts are refused at the link (`compile_vs_linked`: the board's
`refusals.tsv` says `link error: the vertex shader for the program's
fragment shader: ...`), so the shape is the pair: the operator vertex
harness writes `v_color = color` where `color` has constant lanes, and the
fragment shader is `gl_FragColor = v_color`.

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `constant_then_varying`: `a colour pair of a constant then a varying lane`: `color = vec4(0.0, 0.0, 0.0, 1.0); color.yz = res` (a constant lane before a varying lane in a pair) | 141 | 141 / 0 | 0 / 141 | this plan | `shaders.operator.binary_operator.add.highp_float_vec2_vertex` | 141 |
| `varying_and_zero`: `a colour pair of a varying lane and 0.0 (Mesa's U16_TO_U32)`: `color.x = res` then `.y` stays 0.0 (`default_vertex_attrib.float`, `binary_operator.*_float_vertex`) | 29 | 29 / 0 | 0 / 29 | this plan | `shaders.operator.binary_operator.add.highp_float_vertex` | 29 |

Board 142 + 29 = 171: `shaders.random.basic_expression.vertex.49` moved to
the newer attribute-numbering text (section 11).

## 11. loops (`mod.rs:493`, `:488`, `:491`, `:485`): 774 cases (HOST-ONLY)

Companion of the study-loops note (`docs/notes/mali-deqp-loops.md`) and of
`mali-vs-loops.md` (the design). #367 holds the plain `for` with a constant
or an int-uniform bound (`gm2_intloop_const_vs`, `gm2_cf_loop_uni_{vs,fs}`,
`gm2_intloop_uniform_{vs,fs}`); the rows marked #367 below are those forms.
A `for` whose counter indexes an array, a `while`, a `do-while` and a
varying-derived bound are this plan's. The loop texts are NotSupported on
the board for most cases (dEQP's "Loop type is not supported").

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `vs_for_index`: a VS `for` whose counter subscripts a vector, matrix or local array (`shaders.indexing.*_loop_*_vertex`) | 164 | 54 / 110 | 164 / 0 | this plan | `shaders.indexing.matrix_subscript.mat2_dynamic_loop_write_dynamic_loop_read_vertex` | 132; 16 to ARITH, 8 to loops |
| `vs_while`: a VS `while` (`shaders.loops.while_*_vertex`) | 105 | 0 / 105 | 105 / 0 | this plan | `shaders.loops.while_constant_iterations.101_iterations_vertex` | 103; 2 to intval |
| `vs_do_while`: a VS `do { } while` | 100 | 0 / 100 | 100 / 0 | this plan | `shaders.loops.do_while_constant_iterations.101_iterations_vertex` | 100 |
| `fs_kept_for_index`: `a loop Mesa keeps ... in a mediump fragment shader`: a `for` whose counter subscripts an array (`shaders.indexing.*_fragment`, `shaders.struct`) | 85 | 0 / 85 | 0 / 85 | this plan | `shaders.indexing.matrix_subscript.mat2_dynamic_loop_write_dynamic_loop_read_fragment` | 85 |
| `vs_for_uniform`: a VS `for` with an int-uniform bound (`for_uniform_iterations`, `for_dynamic_iterations`, `infinite_with_conditional_break`) | 69 | 0 / 69 | 69 / 0 | #367 | `shaders.loops.for_constant_iterations.infinite_with_conditional_break_vertex` | 69 |
| `fs_kept_for`: a kept mediump FS `for` (`for_uniform_iterations`, `for_dynamic_iterations`, `select_iteration_count`) | 67 | 0 / 67 | 0 / 67 | #367 | `shaders.loops.for_constant_iterations.select_iteration_count_fragment` | 67 |
| `fs_kept_while`: a kept mediump FS `while` | 66 | 0 / 66 | 0 / 66 | this plan | `shaders.loops.while_constant_iterations.select_iteration_count_fragment` | 66 |
| `vs_for_constant`: a VS `for` with a literal bound (`for_constant_iterations`, `shaders.invariance.*.loop_*`, `functions.control_flow`) | 50 | 48 / 2 | 50 / 0 | #367 | `shaders.loops.for_constant_iterations.101_iterations_vertex` | 29; 15 to unitype, 6 to intval |
| `fs_kept_do_while`: a kept mediump FS `do-while` | 50 | 0 / 50 | 0 / 50 | this plan | `shaders.loops.do_while_constant_iterations.select_iteration_count_fragment` | 50 |
| `vs_for_dynamic`: a VS `for` with a varying-derived bound (`basic_*_float_vertex` forms whose counter is a float) | 9 | 3 / 6 | 9 / 0 | this plan | `shaders.loops.for_constant_iterations.basic_highp_float_vertex` | 9 |
| `fs_unrolled_break`: `a break or continue behind a run-time condition in a loop a mediump fragment shader unrolls` | 6 | 0 / 6 | 0 / 6 | this plan (`mali-loop-break.md`) | `shaders.loops.do_while_uniform_iterations.conditional_break_fragment` | 6 |
| `fs_past_limits`: `a loop past Mesa's unroll limits ...` (`101_iterations_fragment`) | 3 | 1 / 2 | 0 / 3 | this plan | `shaders.loops.do_while_constant_iterations.101_iterations_fragment` | 3 |

Board: `a loop in a vertex shader` 392 NS + 105 Fail = 497, host 497 (the
sum of the `vs_*` rows); `a loop Mesa keeps` 268, host 268; the host total
of 774 is the four texts together.

## 12. misc: 29 cases (HOST-ONLY)

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `invariant_output` (`mod.rs:455`): `invariant gl_Position` / `invariant varying` (`shaders.invariance.*`, `qualification_order`, `preprocessor.pragmas`) | 12 | 12 / 0 | 12 / 0 | this plan | `shaders.invariance.highp.common_subexpression_0` | 0 alone: 9 to unitype (the invariance harness's fragment side), 2 to FRAGCOLOR_LANE, 1 to intval |
| `algebraic_rewrite` (`mod.rs:561`): `shaders.matrix.mul.const_*_mat2_float_fragment` (a constant matrix times a float) | 6 | 6 / 0 | 0 / 6 | this plan | `shaders.matrix.mul.const_highp_mat2_float_fragment` | 6 |
| `comparison_form` (`mod.rs:558`): `notEqual(vec2, vec2)` and the other float relational forms no reference draws | 3 | 3 / 0 | 0 / 3 | this plan | `shaders.constant_expressions.builtin_functions.vector_relational.notEqual_vec2_fragment` | 3 |
| `ir_values` (`mod.rs:540`): `uniform_api.random.{23,...}` programs over 2048 IR values | 3 | 3 / 0 | 2 / 1 | this plan | `uniform_api.random.23` | 1; 1 to ARITH, 1 to misc |
| `texture_lanes` (`mod.rs:476`): `shaders.struct.uniform.sampler_array_fragment` and `sampler_nested*` (a look-up result's lanes swizzled) | 2 | 2 / 0 | 0 / 2 | this plan | `shaders.struct.uniform.sampler_array_fragment` | 2 |
| `work_registers` (`mod.rs:536`): `shaders.matrix.mul.uniform_lowp_mat4_mat4_vertex` and its mediump twin | 2 | 2 / 0 | 2 / 0 | this plan | `shaders.matrix.mul.uniform_lowp_mat4_mat4_vertex` | 2 |
| `builtin_input` (`mod.rs:496`): `shaders.builtin_variable.fragcoord_xyz` | 1 | 1 / 0 | 0 / 1 | this plan (`mali-builtin-inputs.md`) | `shaders.builtin_variable.fragcoord_xyz` | 1 |

Board: 12 invariant + 2 + 3 + 3 + 2 + 1 + 6 = 29; host 29.

## 13. The 20 cases whose first refusal moved between `37305608` and `abcc9013`

All `shaders.random.*`. Board family -> host family (host text in short):

| Case | Board | Host |
|---|---|---|
| `shaders.random.all_features.fragment.37` | constop (-0.0 through a variable) | CONST_FOLD |
| `shaders.random.all_features.fragment.55` | ARITH | LOCALS |
| `shaders.random.all_features.fragment.86` | constop (-0.0 through a variable) | CONST_FOLD |
| `shaders.random.basic_expression.combined.18` | CONST_FOLD | varylink (two varying lanes of equal value) |
| `shaders.random.basic_expression.combined.85` | CONST_FOLD | intval |
| `shaders.random.basic_expression.fragment.85` | constop (non-finite fold) | CONST_FOLD |
| `shaders.random.basic_expression.vertex.49` | colourpair (constant then varying) | `a vertex shader whose attribute numbering waits for a fold Mesa's back end makes after it numbers the locations` (a text newer than the slices; "other") |
| `shaders.random.conditionals.vertex.41` | CONST_FOLD | constop (non-finite fold) |
| `shaders.random.conditionals.vertex.72` | LOCALS | CONST_FOLD |
| `shaders.random.exponential.fragment.12` | ARITH | intval |
| `shaders.random.exponential.fragment.{55,65,98}` | ARITH | constop (non-finite fold) |
| `shaders.random.exponential.vertex.55` | constop (non-finite fold) | CONST_FOLD |
| `shaders.random.exponential.vertex.98` | intval | ARITH |
| `shaders.random.scalar_conversion.combined.40` | intval | CONST_FOLD |
| `shaders.random.scalar_conversion.combined.9` | CONST_FOLD | intval |
| `shaders.random.scalar_conversion.vertex.4` | intval | CONST_FOLD |
| `shaders.random.texture.fragment.90` | constop (non-finite fold) | intval |
| `shaders.random.trigonometric.fragment.15` | CONST_FOLD | constop (-0.0 through a variable) |

They are the random shaders where #367's and #373's merged lowerings
between the two commits changed which refusal comes first. None of them
compiles on the host.

## 14. The committed pairs: `tools/mali/testdata/deqp_shapes/`

One `<family>_<shape>.vert` and `.frag` per shape this plan takes (71
pairs; the four shapes owned by #367 are not copied, their examples are in
the tables), each the example case's shader pair verbatim from the `.qpa`
(dEQP's generated source: Apache-2.0, The Khronos Group Inc.; a REUSE
`.license` sidecar per file names the case). `index.tsv` lists family,
shape, owner, cases, fail, notsupported (board statuses) and the example
case. The `ref-<family>` items make their `deqp_<family>_*` references
from these pairs (`tools/mali/deqp_ref.sh`); where a shape's example is the
harness rather than the construct (`harness_isok`, the `_both` halves), the
reference items should pick a case of the group they need from
`measure.tsv` of a fresh run.

## 15. Caveats

- The shape rules are regular expressions over the GLSL, tried in order;
  a shader with several constructs is named by the first rule that holds,
  which is the one a lowering meets first in the plan's view, not
  necessarily the one the compiler refused on. The refusal site is not
  logged; the study items have it.
- `random_mixed` is a bucket, not a shape: 973 cases (771 intval, 202
  constop) that no single lowering moves.
- Counts are first refusals only. A case passes only when every refusal of
  both shaders, the link and the draw are lowered; "next refusal" gives the
  partner shader's, the link's and the draw's are unmeasured.
- The host run reports dEQP statuses that mean nothing (Fail from black
  pixels); the script never prints them per case and this note quotes the
  board's.
- The run needs about 4 minutes; the two builds (deqp-gles2 713 steps, the
  library) about 40 minutes on 4 cores the first time and are reused from
  `build/host-deqp-mali/`.
