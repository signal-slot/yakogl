<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the dEQP-GLES2 control-flow and discard refusals, their shapes and Mesa 26.2.1 references

**Status: study** (plan #387, item `ctrlflow-ref`; issues #20, #7). Nothing
under `src/` changed. The references are `tools/mali/testdata/deqp_ctrlflow_*`,
pinned by `tests/mali_deqp_ctrlflow.rs`. Measured at the integration tip
**`5cb806246171`** (2026-11-21). Mesa paths are relative to `src/` of the
26.2.1 tree.

Two refusal texts had no reference set and no owner in plan #387
(`mali-deqp-families.md` section 18.2, rows 18 and 26):

- `MaliError::ControlFlow`: "Valhall: control flow (if, a loop, a return
  before the end) is not lowered yet" (`src/shaderc/online/mali/mod.rs:411`).
- `MaliError::DiscardForm`: "Valhall: a discard other than a highp fragment
  shader's `if (a < b) discard;` with no else (an unconditional one, one
  among other statements, one under another if, a mediump shader's) is not
  lowered yet" (`mod.rs:483-485`).

The earlier notes on both are `mali-control-flow.md` (gl_test cases 68, 71,
74, 142, 148: the flatten and the kept branch that landed) and
`mali-discard.md` (the frame side and the highp and case-81 discard forms).

## 1. How this was measured

- **Host measure at the tip:** `tools/mali/deqp-host-measure.sh --cts
  "$CONNECTED_DEPS_DIR/VK-GL-CTS" --jobs 3 --chunk 100` over the 10,597
  board-refused cases of the slices `2026-11-15-t6-gles2-S{1..4}-37305608`
  (section 18 of `mali-deqp-families.md`). VK-GL-CTS `93bca018` with
  `cts-yakogl-target.patch`, rebuilt into `$CONNECTED_DEPS_DIR` the way
  `tools/cts/deps.toml` does. All 10,597 cases began; 106 processes exited
  1 and 2 exited 139 (the two `scalar_conversion` crashes of section 18.4);
  **6,132** cases end with a host refusal (6,196 at `9ac31f30`). HOST-ONLY:
  compile and link outcomes only.
- **The cases:** `measure.tsv` rows whose first refusal is either text:
  **73 `ControlFlow`** and **16 `DiscardForm`**, the same counts as at
  `9ac31f30`.
- **Shapes:** each refusing shader was reduced by a scratch line reducer
  (not committed). It drops lines while the refusal stays, compiling through
  `tests/mali_ref`'s `compile_pair`; a vertex shader is reduced against a
  constant fragment shader, so the partner's own refusal cannot stand in.
  A shape is the construct left, read together with the unreduced source
  (an `else if` or an `if` under an `if` makes a case `nested` even when the
  reducer found a smaller refusing line). The grouping is a heuristic over
  the text, not an instrumented refusal site.
- **What Mesa does, per case:** every one of the 89 pairs was drawn once on
  the drm-shim (`tools/mali/drm_shim_ref.sh run ... --uniform-values`, not
  committed), and the refusing stage's final disassembly was classed:
  `gone` (no `CSEL`, `MUX`, kept `BRANCHZ` or `DISCARD` left: the condition
  folded or the `if`'s results are dead), `select` (a `CSEL`/`MUX`, no
  branch), `kept-loop`, `discard`. A select Mesa writes another way (for
  example `FCMP_OR ... .f1`, a compare whose result is the 1.0/0.0 of a
  `?:`) counts as `gone`: `affect_ternary` is the one case seen.
- **References:** `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_ctrlflow_<name> --instrument shim`, on the
  drm-shim that `tools/mali/drm_shim_ref.sh build` made from the pinned
  `mesa-26.2.1.tar.xz` into `$CONNECTED_DEPS_DIR/mesa-shim`. The README's
  self-check (`gm2_cf_ifelse_vs`, `gm2_cf_ifelse_fs`) was byte-equal. Each
  pair is verbatim from the host measure's `.qpa`, with a `.license` file
  per shader as the `deqp_loops2_*` pairs have.

## 2. Counts at `5cb80624`

| Text | Cases | Fail / NS (board) | VS / FS | dEQP groups | Mesa's output |
|---|--:|---|---|---|---|
| `ControlFlow` | 73 | 69 / 4 | 39 / 34 | shaders.random.conditionals 46 (vertex 16, combined 20, fragment 10), shaders.conditionals.if 7, fragment_ops.interaction.basic_shader 6, shaders.loops 4, shaders.linkage 3, shaders.struct 2, shaders.operator.sequence 2, shaders.algorithm 1, shaders.builtin_variable 1, shaders.return 1 | gone 42 (VS 30, FS 12), select 27 (VS 9, FS 18), kept-loop 4 |
| `DiscardForm` | 16 | 16 / 0 | 0 / 16 | shaders.discard 16 | discard 12, gone 4 |

The 4 NotSupported are the `shaders.loops.do_while_*` cases (section 3).
Another 21 cases of the measure have `ControlFlow` as their **next**
refusal (`next_family` 367:CONTROL_FLOW): 3 of the 73 above and 18 whose
first refusal is another text (unitype 8, varylink 3, constop 3,
367:CONST_FOLD 3, 367:ARITH 1). They are not counted here.

## 3. The shapes

The name is the reference's without `deqp_ctrlflow_`; a shape's count is
the cases whose construct it is, `gone`/`select` the Mesa class of section 1.

### 3.1 `ControlFlow`, vertex shaders (39)

| Shape | Cases | Mesa | Groups | References | The construct, and the refusing site |
|---|--:|---|---|---|---|
| `vs_nested`: an `if` (or `else if`) on a side of an `if` | 32 | gone 25, select 7 | random.conditionals.combined 15, .vertex 13, conditionals.if 3 (`if_elseif_vertex`, `if_elseif_else_vertex`, `mixed_if_elseif_else_vertex`), algorithm 1 (`hsl_to_rgb_vertex`) | `vs_nested` (`random.conditionals.vertex.10`, gone), `vs_nested_combined` (`.combined.41`, gone), `vs_elseif` (`conditionals.if.if_elseif_vertex`, select), `vs_nested_hsl` (`algorithm.hsl_to_rgb_vertex`, select) | `vs_steps`' `side` takes straight code only: an `IrNode::If` under a side is `_ => ControlFlow` (`nir.rs:3779`). The random cases' conditions are mostly constants (`if (bool(0))`, `if (true)`, `if (g)` of a `const bool`) or their result is dead |
| `vs_varying_side`: a varying written on one side only of a flat `if` | 6 | gone 4, select 2 | linkage `varying_5`, `_6`, `_7`; random.conditionals.vertex.1, .59, .combined.30 | `vs_varying_one_side` (`linkage.varying_5`, gone: the fragment shader reads no varying), `vs_varying_else_side` (`.vertex.59`, gone), `vs_varying_select` (`.vertex.1`, select) | `vs_join`: a varying set on one side only is `ControlFlow` (`nir.rs:3885`; a lane one side leaves undefined, `:3877`) |
| `vs_statement_side`: an `if` of an integer compare whose side is an unused expression statement | 1 | gone | random.conditionals.vertex.90 | `vs_statement_side` | an unreferenced comparison statement under an `if`: `side`'s limit or the join (not instrumented) |

### 3.2 `ControlFlow`, fragment shaders (34; all mediump)

| Shape | Cases | Mesa | Groups | References | The construct, and the refusing site |
|---|--:|---|---|---|---|
| `fs_int_cond`: the condition is an `int`/`ivec` compare (or a `bool` of one) | 12 | gone 7, select 5 | interaction.basic_shader 68, 69, 72, 82; random.conditionals.fragment 23, 45, 79, 88, 90; .combined 45, 88, 90 | `fs_int_cond` (`interaction.basic_shader.69`, gone: the `bvec4` it sets is dead), `fs_int_cond_colour` (`random.conditionals.fragment.79`, select) | `walk_if16`: a condition that is neither a boolean lane nor a 16-bit float compare (`nir_fs.rs:4613`) |
| `fs_float_cond_side`: a 16-bit float compare, a side that reads a varying or sets a local other than the colour | 5 | gone 2, select 3 | interaction.basic_shader 24, 25; random.conditionals.fragment 14, 51, 92 | `fs_float_cond_varying` (`.fragment.51`, select), `fs_float_cond_colour_read` (`interaction.basic_shader.24`, gone: the colour is written again after it) | `walk_if16`'s `alu_only`: a side with a varying load or more than the peephole's operations (`nir_fs.rs:4641`) |
| `shaderlib_and` (shader-library compare: `RES = RES && isOk(out1, ...)`) | 4 | select | conditionals.if `complex_condition`, `compound_statement`, `sequence_condition`, `sequence_statements` (`_vertex`: the vertex shader computes, the fragment shader compares two outputs) | `fs_shaderlib_and` (`compound_statement_vertex`) | the front end's `&&` is an `if` that sets a `bool` local; its select of a boolean under a float compare has no reference (`nir_fs.rs:4657`) |
| `fs_vec_cond`: a vector `==` condition (`bool(e == e)`, `else if (l == k)`) | 3 | gone 2, select 1 | random.conditionals.fragment 28, 43; .combined.43 | `fs_vec_cond` (`.fragment.28`, select) | the condition is an `all()` of lane compares: not a single compare's mask (`nir_fs.rs:4613`) |
| `fs_struct_select`: a struct local assigned under an `if` | 2 | select | struct.local `conditional_assignment`, `nested_conditional_assignment` | `fs_struct_select` (`conditional_assignment_fragment`) | sides that read varyings and an int uniform (`:4641`) |
| `fs_side_effect`: a function with a global side effect under `||` or `?:` | 2 | select 1, gone 1 | operator.sequence.side_effects `affect_or`, `affect_ternary` | `fs_side_effect_or`, `fs_side_effect_ternary` | the inlined calls' stores to globals under the front end's `if` |
| `fs_frontfacing`: `if (gl_FrontFacing)` setting the colour on each side | 1 | select | builtin_variable.frontfacing | `fs_frontfacing` | the condition is the face bit, not a compare (`:4613`) |
| `fs_return_static_loop`: an early `return` inside a one-pass constant loop of a called function | 1 | select | return.return_in_static_loop_dynamic_fragment | `fs_return_static_loop` | the walk ends with a flow that is not `Next` (`nir_fs.rs:4234`): the return flag the front end makes |
| `fs_loop_if`: an `if` in the body of a uniform- or dynamic-bound `do`-`while` | 4 (all NS) | kept-loop | loops.do_while_{uniform,dynamic}_iterations.{conditional_body, mixed_break_continue}_fragment | `fs_loop_break_continue` (`do_while_uniform_iterations.mixed_break_continue_fragment`); `conditional_body` is `deqp_loops_fs_dowhile_if` (`tests/mali_deqp_loops.rs`, the same pair and the same words, so not drawn again) | the kept loop's body `if` in a mediump shader (`walk_if16` under `walk_kept_loop`) |

### 3.3 `DiscardForm` (16; all `shaders.discard`, mediump)

The 16 cases are four wrappers (`basic`, `function`, `static_loop`,
`function_static_loop`) times four conditions. The wrappers vanish in Mesa:
the function is inlined and the two-pass static loop unrolled to its second
pass, so each condition's fragment words are the same for every wrapper
(measured: `disc_basic_dynamic` = `disc_function_dynamic`,
`disc_basic_always` = `disc_function_static_loop_always`,
`disc_basic_uniform` = `disc_static_loop_uniform`, every region). The
`varylink` reference `deqp_varylink_discard` has `disc_basic_dynamic`'s words
too.

| Shape | Cases | References | Mesa's fragment shader | The refusing site (likeliest, not instrumented) |
|---|--:|---|---|---|
| `disc_always`: `discard;` after the colour | 4 | `disc_basic_always`, `disc_function_static_loop_always` | 9 words: `DISCARD.f32.eq.discard 0x0, 0x0` first (demote), then the varying load, `ATEST` and `BLEND` as without it | `ir::Op::Discard` outside an `if` (`nir_fs.rs:6828`) |
| `disc_never`: `if (false) discard;` | 4 | `disc_basic_never` | 8 words, no discard: the plain colour | the `if` of a constant condition whose side is a discard (`nir_fs.rs:4496`) |
| `disc_uniform`: `if (ui_one > 0) discard;` (mediump int uniform) | 4 | `disc_basic_uniform`, `disc_static_loop_uniform` | 10 words: `ICMP_OR.s32.lt.m1 r0, 0x0, u2.w0, 0x0`, `DISCARD.f32.ne r0, 0x0` | `discard`: the condition is not a float compare (`nir_fs.rs:1833-1835`) |
| `disc_dynamic`: `if (v_coords.x + v_coords.y > 0.0) discard;` | 4 | `disc_basic_dynamic`, `disc_function_dynamic` | 9 words: `V2F32_TO_V2F16` of x and y, `DISCARD.f32.lt r0.neg.h0, r0.h1` (`0 < x + y` is `-x < y`, the sum folded into the compare) | `cmp_leaf`: a computed operand (`nir_fs.rs:9075`) |

## 4. What Mesa does with them

- **No branch outside the loops.** In all 85 non-loop cases the refusing
  stage keeps no `BRANCHZ`: `nir_opt_peephole_select` flattens every `if`
  (both bifrost's call, limit 64, and the GL front end's earlier call). The
  `if`s that survive become selects; the rest are gone before the back end
  (constant conditions, `nir_opt_dce` of results nothing reads, a varying the
  link drops).
- **`gone` is the largest class: 42 of the 73 `ControlFlow` cases and 4 of
  the 16 discards.** Their reference output has no trace of the `if`:
  `vs_nested_combined` and `vs_varying_one_side` have an empty varying
  program; `fs_int_cond` and `fs_float_cond_colour_read` are 7-word
  fragment shaders (a uniform or constant colour). Our walk refuses the
  `if` before any liveness or constant folding of its condition.
- **Vertex selects are 32-bit `CSEL.u32.eq`** on the condition (as
  `vsarith_188_predif`), one per differing lane, inner `if`s first. A
  mediump vertex shader computes in fp16: `vs_elseif` (`precision mediump
  float` in the vertex shader) is `FCMP_OR.v2f16.ge.m1` of the attribute's
  `FADD.f32 .h0` conversion, `S16_TO_S32` of each mask half, two `CSEL.u32`
  (the `else if` ladder: `2.0`, `1.0`, `0.0`); `vs_nested_hsl` is 59 varying
  words of `v2f16` arithmetic, 8 `FCMP`, 11 `CSEL`/`MUX`.
- **Fragment selects come in two widths.** A select whose sources are fp16
  values is `MUX.v2i16` on a 16-bit mask (`fs_frontfacing`:
  `ICMP_OR.u32.eq` of the face register `r58`, then `MUX.v2i16` of the
  constant halves; `fs_return_static_loop`: two `MUX.v2i16` of the varying's
  halves on `FCMP_OR.v2f16.ge`). A select whose sources are a varying's
  32-bit loads, an int uniform or an integer condition is taken **before**
  the conversion to fp16: `CSEL.u32.eq` of the loaded lanes, then the
  `FADD.f32 .h0` conversion of the selected value (`fs_vec_cond`,
  `fs_struct_select`, `fs_int_cond_colour`). The mask of a 16-bit compare
  is widened by `S16_TO_S32`; masks are combined by `LSHIFT_AND.v2i16` or
  `.i32` (`fs_shaderlib_and`'s `&&`: two `FCMP_OR.v2f16.ge` masks,
  `LSHIFT_AND.v2i16`, then `MUX.v2i16 0x0, 0x3C00` for the colour).
- **The kept loops** (`fs_loop_if`) keep the `do`-`while` (`BRANCHZ` back
  edge) and flatten the body's `if` into `MUX.v2i16` on an
  `ICMP_OR.s32.lt` of the counter; `mixed_break_continue` folds the `break`
  into the exit test (`mali-deqp-loops.md` section 7, `fs_mixed`).
- **Discards** are one `DISCARD.f32` each, never a branch (section 3.3).

## 5. Lowering order: one-session steps, ranked by cases

Cases are those whose first refusal the step removes; a case passes only
when nothing else in it refuses (the measure's `next_family` is empty for
76 of the 89, so most stop at this refusal).

| # | Step | Cases | References to make byte-equal | Overlaps |
|--:|---|--:|---|---|
| 1 | **cf-dead**: before the walks, fold an `if` whose condition is a constant to its taken side, and drop an `if` whose sides write only locals, varyings or outputs nothing reads after it (the link's reads for a varying, `compile_vs_linked`). No new instruction form: the output is the straight code Mesa leaves | 45 (CF 41: VS 30, FS 11; discard 4 `disc_never`) | `vs_nested`, `vs_nested_combined`, `vs_varying_one_side`, `vs_varying_else_side`, `vs_statement_side`, `fs_int_cond`, `fs_float_cond_colour_read`, `disc_basic_never` | constant folding of globals and constant conditions is the constop/negzero work (`negzero-lower`, `cfold-ref`): use one folder; the random cases also hold int conversions (intval, vertex step 2 landed `56bed483`) |
| 2 | **disc-forms**: `discard;` outside an `if` (`DISCARD.f32.eq 0x0, 0x0` at its place), a discard under an integer compare (`ICMP` then `DISCARD.f32.ne mask, 0`), and one under a compare of a computed fp16 value (the `fadd` folded into the compare's operands) | 12 | `disc_basic_always`, `disc_function_static_loop_always`, `disc_basic_uniform`, `disc_static_loop_uniform`, `disc_basic_dynamic`, `disc_function_dynamic` | the integer compare is intval's (`intval-rest`); the function and static-loop wrappers need no work once inlining and the unroll run first, as they do |
| 3 | **fs-select32**: a mediump fragment `if` whose condition is an integer or vector compare, or whose sides read varyings or an int uniform: `CSEL.u32.eq` before the fp16 conversion | 11 (fs_int_cond 5, fs_float_cond_side 3, fs_vec_cond 1, fs_struct_select 2) | `fs_int_cond_colour`, `fs_float_cond_varying`, `fs_vec_cond`, `fs_struct_select` | the integer compares (`ICMP_OR.s32`, `F16_TO_S32` of an fp16 uniform) are `intval-rest`'s: after it |
| 4 | **vs-nested-select**: a nested `if` and a one-sided varying in the vertex walk, as `CSEL.u32` from the inside out (the join's phi of the earlier value) | 9 (vs_nested 7, vs_varying_side 2) | `vs_varying_select`, `vs_elseif`, `vs_nested_hsl` | 4 of them (`conditionals.if.*_vertex`, `hsl_to_rgb_vertex`) are mediump vertex shaders: the fp16 vertex walk of `intval-vs16` (`vs16-design`) first |
| 5 | **fs-mask-logic**: `&&`/`||` and `gl_FrontFacing` conditions, the inlined side effects, the early-return flag of an unrolled one-pass loop: `LSHIFT_AND.v2i16` of masks, `MUX.v2i16`; `affect_ternary`'s `?:` of 1.0/0.0 is one `FCMP_OR.v2f16.lt.f1` (the compare writes the float) | 8 (shaderlib_and 4, side_effect 2, frontfacing 1, return_static_loop 1) | `fs_shaderlib_and`, `fs_frontfacing`, `fs_side_effect_or`, `fs_side_effect_ternary`, `fs_return_static_loop` | `shaderlib_and` is the shader-library compare fragment shader: the other `_vertex` cases with two outputs meet it too once their vertex shader compiles (intval-reslib's fragment compare); `fs_return_static_loop` uses the static-loop unroll of `mali-deqp-loops.md` |
| 6 | **with loops-lower-1**: the `if` in a kept mediump `do`-`while` body | 4 (all NS) | `fs_loop_break_continue`, `deqp_loops_fs_dowhile_if` | owned by the loops items (`LoopMediump`, `mali-deqp-loops.md` F2, `loops-lower-1`); not a separate step |

Steps 1 and 2 have no dependency and can run now (step 2's `disc_uniform`
part after `intval-rest`, or with an integer compare added for the discard
alone). Steps 3 to 5 wait for the integer and mediump vertex work named.
Step 6 belongs to the loops items.

## 6. Caveats

- HOST-ONLY: the compile outcome at the tip; a case that compiles here is
  not a board pass.
- Only the slices' 10,597 board-refused cases are measured (as section 18.6
  of `mali-deqp-families.md`).
- The shape and the "refusing site" columns come from reduced sources and
  reading the code, not from an instrumented compiler: a case can hold
  several constructs, and the first one the walk meets is the one it
  reports.
- The Mesa class (`gone`/`select`/...) reads the final disassembly only.
- The references are drawn with `--uniform-values`; a lowering compares
  them with `mali_ref::assert_matches` as they are.

## 6a. Measured sites at a7260420

MEASURED, HOST-ONLY. Every `deqp_ctrlflow_*` reference (27) was compiled
with `mali_ref::compile_pair` (as `tests/mali_deqp_ctrlflow.rs`, features
`host-test,gpu-mali,shaderc-online,no-shader-table`) at the integration tip
`a7260420`, in a scratch test that was not committed, with a temporary
`eprintln!(file!(), line!())` at every `MaliError::ControlFlow` /
`DiscardForm` construction in `vec16.rs`, `nir.rs` and `nir_fs.rs` (all
reverted; no code or test is changed). Sites are `nir_fs.rs` unless
`nir.rs` is named; line numbers are those of `a7260420` and drift with the
files. The text is the refusal `refusals_pinned` pins; it is unchanged for
all 27: **no reference's first refusal is another text** (no intval,
loop or guard text hides a ctrlflow one), so no item owns a moved
reference. `nir.rs:3821` is not a site of its own: `vs_steps` emits
`Step::Refuse(flat(..).err())` for an `if` whose side `flat` refused, and
the error comes from `side` (`nir.rs:3774` or `:3779`, printed first).
"Step" is the row of section 5.

| Reference | Text | Site | What the site is | Step |
|---|---|---|---|--:|
| `disc_basic_always` | DiscardForm | `:6828` | `ir::Op::Discard` outside an `if` | 2 |
| `disc_function_static_loop_always` | DiscardForm | `:1833` | `discard()`: the condition is a constant, not a lane (the note guessed `:6828`) | 2 |
| `disc_basic_never` | DiscardForm | `:1833` | `discard()`: constant `false` condition (the note guessed `:4496`) | 1 |
| `disc_basic_uniform` | DiscardForm | `:1835` | `discard()`: the condition is an integer compare, not an `Fcmp` | 2 |
| `disc_static_loop_uniform` | DiscardForm | `:1835` | as `disc_basic_uniform` | 2 |
| `disc_basic_dynamic` | DiscardForm | `:9075` | `cmp_leaf`: a computed operand (`x + y`) | 2 |
| `disc_function_dynamic` | DiscardForm | `:9075` | as `disc_basic_dynamic` | 2 |
| `fs_int_cond` | ControlFlow | `:4657` | `walk_if16`'s `sel`: a join of two lanes that is no float-compare `bcsel` (an integer compare; its `bvec4` is dead). The note guessed `:4613`: the integer compare passes `mask_lane` | 1 |
| `fs_int_cond_colour` | ControlFlow | `:4625` | the colour differs between the sides and the condition is not a 16-bit `Fcmp` (note: `:4613`) | 3 |
| `fs_float_cond_colour_read` | ControlFlow | `:4641` | `alu_only` fails: a side holds more than `PEEPHOLE_LIMIT` ops or a non-ALU node | 1 |
| `fs_float_cond_varying` | ControlFlow | `:4641` | as above: a side reads a varying | 3 |
| `fs_vec_cond` | ControlFlow | `:4613` | the condition (an `all()` of lane compares) is neither a mask lane nor a 16-bit `Fcmp` | 3 |
| `fs_struct_select` | ControlFlow | `:4657` | `sel`: a select of a boolean or mask lane at the join (note: `:4641`) | 3 |
| `fs_shaderlib_and` | ControlFlow | `:4641` | `alu_only` fails (the `&&` `if`'s side; note: `:4657`) | 5 |
| `fs_side_effect_or` | ControlFlow | `:4657` | `sel`: a global's value joined under the `||` `if` | 5 |
| `fs_side_effect_ternary` | ControlFlow | `:4657` | `sel`: as `fs_side_effect_or` | 5 |
| `fs_frontfacing` | ControlFlow | `:4625` | the face bit passes `mask_lane`; the colour differs per side and the condition is no 16-bit `Fcmp` (note: `:4613`) | 5 |
| `fs_return_static_loop` | ControlFlow | `:4625` | a side's flow is not `Flow::Next` (the return flag) (note: `:4234`) | 5 |
| `fs_loop_break_continue` | ControlFlow | `:4625` | the `if` of a kept loop's body: the colour differs per side | 6 (`loops-lower-1`) |
| `vs_nested` | ControlFlow | `nir.rs:3779` (via `:3821`) | `side`: an `IrNode::If` inside a side | 1 (dead conditions) |
| `vs_nested_combined` | ControlFlow | `nir.rs:3779` | as `vs_nested` | 1 |
| `vs_elseif` | ControlFlow | `nir.rs:3779` | as `vs_nested` | 4 |
| `vs_nested_hsl` | ControlFlow | `nir.rs:3774` | `side`: more than `PEEPHOLE_LIMIT_GL` ALU ops in a side (note: a nested `if`) | 4 |
| `vs_statement_side` | ControlFlow | `nir.rs:3774` | `side`: more than `PEEPHOLE_LIMIT_GL` ops (the unused statement's compare chain) | 1 |
| `vs_varying_select` | ControlFlow | `nir.rs:3774` | `side` limit, before `vs_join` (note: `:3885`) | 4 |
| `vs_varying_one_side` | ControlFlow | `nir.rs:3885` | `vs_join`: a varying set on one side only | 1 |
| `vs_varying_else_side` | ControlFlow | `nir.rs:3885` | as `vs_varying_one_side` | 1 |

What the measure changes for the steps: (a) `disc_basic_never` and
`disc_function_static_loop_always` both stop in `discard()` on a constant
condition, so step 1's `if (false) discard;` and step 2's `discard;` share
the one constant-condition form (`DISCARD.f32.eq 0x0, 0x0` for `true`, none
for `false`); (b) four fragment references (`fs_int_cond_colour`,
`fs_frontfacing`, `fs_return_static_loop`, `fs_loop_break_continue`) pass
the condition test and stop at `:4625`, on the per-side colour, so their
lowering is the colour join of the select, not a condition form; (c)
`vs_nested_hsl`, `vs_statement_side` and `vs_varying_select` stop on the
side's op limit (`:3774`), so a step 4 that only adds nested `if`s to `side`
does not reach them before the limit is lifted.

## 7. Issue comment (to post on the plan #387 tracking issue, or #20)

> **ctrlflow-ref (plan #387): ControlFlow and DiscardForm at `5cb80624`.**
> Host measure (`tools/mali/deqp-host-measure.sh`, VK-GL-CTS 93bca018, the
> 10,597 cases of the slices `37305608`; HOST-ONLY): 6,132 cases still
> refused. **`ControlFlow` 73** (VS 39 / FS 34; 69 Fail, 4 NS) and
> **`DiscardForm` 16** (all `shaders.discard`) are first refusals.
> Shapes and Mesa 26.2.1 output: `docs/notes/mali-deqp-ctrlflow.md`;
> references `tools/mali/testdata/deqp_ctrlflow_*` (27 pairs), pinned by
> `tests/mali_deqp_ctrlflow.rs` (every reference ignored with its refusal,
> `refusals_pinned` runs). No `src/` change.
> Finding: Mesa keeps no branch in any of the 85 non-loop cases, and in 42
> of the 73 `ControlFlow` cases (and the 4 `if (false) discard;`) the `if`
> is gone entirely (constant condition or dead result). The proposed order:
> (1) fold constant and dead `if`s before the walk, 45 cases; (2) the
> discard forms, 12; (3) 32-bit fragment selects after intval-rest, 11;
> (4) vertex nested `if`s and one-sided varyings, 9 (4 need the mediump
> vertex walk); (5) mask logic (`&&`, `gl_FrontFacing`, side effects, the
> early return), 8; (6) the 4 `do`-`while` body `if`s go with loops-lower-1.
