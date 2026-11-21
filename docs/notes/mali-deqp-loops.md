<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall v10): the dEQP-GLES2 loop refusals, Mesa 26.2.1's loops, and a two-step lowering plan

Plan #377, item study-loops (issue #20; stage 5, issue #7). A study:
nothing under `src/` or `tools/` changed and no reference was made. It
covers four refusal texts of section M of `docs/es2-cts-failures.md`, the
tally of the 37305608 tree (2026-11-15, slices
`tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/`, their `refusals.tsv`
read per case):

| `MaliError` | text (`src/shaderc/online/mali/mod.rs`) | cases |
|---|---|---:|
| `LoopVertex` | `:493` "a loop in a vertex shader" | VS 392 NotSupported + 105 Fail |
| `LoopMediump` | `:487-489` "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)" | FS 268 NotSupported |
| `GuardMediump` | `:490-492` "a break or continue behind a run-time condition in a loop a mediump fragment shader unrolls (Mesa's ladder of fp16 selects)" | FS 6 NotSupported |
| `LoopForm` | `:484-486` "a loop past Mesa's unroll limits or with two run-time breaks, or a kept loop with ..." | FS 2 NotSupported + 1 Fail |

774 cases in all. The Fail ones are the loops dEQP requires of every GLSL
ES 1.00 implementation (`for` with a constant bound, Appendix A): the
vertex `for_constant_iterations` cases, the `static_loop` indexing cases,
`invariance.*.loop_*` and a few more. Every other loop is optional, so dEQP
reports its compile refusal as NotSupported (`es2fShaderLoopTests.cpp`,
`ShaderLoopCase::init`: a `CompileFailed` under the uniform or dynamic
requirement becomes `NotSupportedError("Loop type is not supported")`).
A board rerun after the lowering turns a NotSupported into Pass or Fail by
the draw, so the NotSupported count is the one this family moves.

`docs/notes/mali-deqp-families.md` (item measure) did not exist when this
was written: the shapes below come from the case names, the generators
(read from VK-GL-CTS main on GitHub, which the tree's own copies of the
shapes in `tools/shaders/es100/loop_*.{frag,vert}`, `contloops.frag` and
`dynidx_loop.frag` agree with) and the walk's code. Item ref-loops checks
them against the measure's committed pairs (`tools/mali/testdata/deqp_shapes/`).

## 0. What was read first (plan #367's material, and what it owns)

- `docs/notes/mali-control-flow.md`: the five gl_test control-flow cases;
  the mediump guard ladder (cflow_71_deadloop, `guarded16`) and the kept
  mediump branches around look-ups (cflow_148_texbranch, `kept16`), both
  routed through `select_highp`.
- `docs/notes/mali-int-loops.md`: the eleven `intloop_*` references; "Kept,
  N" loops and how to read `BRANCHZ`, `ICMP_OR.s32.ge.m1`, the FAU-word
  bounds, the byte-lane constants; its six-step order (step 5, "real
  loops", is what loops-rt did for highp).
- `docs/notes/mali-vs-loops.md`: the design for `LoopVertex`
  (`loop_nested.vert`, `loop_uniform.dowhile.vert`, both dEQP shapes),
  measured on Mesa 26.2.1: the varying program's 53 words, the do-while's
  rotated form, five lowering steps. Nothing of it is implemented.
- `docs/notes/mali-loop-break.md`: Part 1 (`complex_unroll` of case 87,
  lowered) and Part 2 (the kept loops of loops-rt: `loop_exit`,
  `builtin_98`, `divnest`, `builtin_45` byte-equal; `loop_nested`,
  `loop_nested_vs`, `contloops` refused at `LoopMediump`, `LoopVertex`,
  `GuardMediump`, the three-way split "mediump blocks, then the mediump
  guard ladder on top, then the vertex stage").
- `docs/notes/mali-loop-board-failures.md`: the hidden pass counter every
  kept loop gets (`LOOP_CAP` 4096, counting down, `CapEdit` in the tests),
  a deliberate deviation from Mesa that the vertex stage will need too.
- The glmark2 references `gm2_intloop_{cond,const,func,lamp,uniform}_{fs,vs}`
  and `gm2_cf_{loop_const_fs,loop_uni_fs,loop_uni_vs,loop1_if_fs,
  constloop_struct_fs}` (`tools/mali/testdata/`): int counters from 0 with
  a constant or an int-uniform bound, a float local `d = fract(3.0 * d)`
  in the body, in a vertex shader with `v_dummy`, `u_mvp` and
  `gl_Position`, or in a mediump fragment shader on `gl_FragCoord`.
  `docs/notes/mali-glmark2-forecast.md` rows 30-32 (`loop:...`): VS `3cb0`
  is `LOOP_VERTEX`, FS `3678` is `LOOP_FP16`, both open.

**What #367 holds, left out here.** The glmark2 `loop` scene's shapes: a
vertex loop with an int-uniform bound carrying one fp32 float
(`gm2_cf_loop_uni_vs`, `gm2_intloop_uniform_vs`), a constant-bound vertex
loop of five passes (`gm2_intloop_const_vs`: Mesa unrolls it), and a
mediump fragment loop with an int-uniform bound carrying one fp16 float
(`gm2_cf_loop_uni_fs`, `gm2_intloop_uniform_fs`). Whichever item of #367
lowers those lands the vertex `LoopRegion`, the vertex blocks, and the
first mediump kept loop. This note takes the dEQP shapes they leave:
carried fp16 *vectors* (the `yzwx` rotation, two `MKVEC.v2i16` a pass),
nested and sequenced loops, `while` and `do`/`while` forms, `break` and
`continue` under a run-time condition inside a kept loop, a bound that is
a select or a product, a constant bound past 32 passes, float counters,
loops around array and vector subscripts, and the vertex loops with a
constant bound of every body shape. The plan in section 5 is written so
that it stands whether #367's loop items land before or after it (section
5, "Order against #367").

## 1. The refusal sites (970d7912)

| site | walk | what refuses |
|---|---|---|
| `src/shaderc/online/mali/nir.rs:2660` | vertex, `vs_steps` | a top-level `IrNode::Loop` becomes `Step::Refuse(MaliError::LoopVertex)`; `build` returns it at `:2871` where the node stands (an earlier refusal comes first) |
| `nir.rs:2635` | vertex, `vs_steps` (`side`) | a `Loop` inside an `if` side: the same error, where the `if` would be flattened |
| `src/shaderc/online/mali/nir_fs.rs:3909-3910` | fragment, `walk_kept_loop` | `!prec.highp`: a loop [`kept_loop`] decided Mesa keeps, in a mediump shader (`LoopMediump`) |
| `nir_fs.rs:3406-3407` | fragment, `walk_if16` | a pass's guard or `continue` (`IfExit` not `Plain`) in a mediump loop the walk unrolls (`GuardMediump`) |
| `nir_fs.rs:9170-9176` | fragment, `select_highp` | a `guarded16` ladder with a lone fp16 product (`GuardMediump`, deadloop_* pins; not a dEQP shape) |
| `nir_fs.rs:3535-3536` | fragment, `walk_passes` | more than `MAX_UNROLL_ITERATIONS` (32, `:69`) passes (`LoopForm`: the 101-pass loops) |
| `nir_fs.rs:3677-3678` | fragment, `walk_loop` | an unrolled body over `LOOP_UNROLL_LIMIT * MAX_UNROLL_ITERATIONS` (`:70`, 26 * 32) nodes (`LoopForm`) |
| `nir_fs.rs:3571-3572` | fragment, `walk_guard` | a second run-time `break` in an unrolled loop (`LoopForm`) |
| `nir_fs.rs:3288` | fragment, `walk_if` | a side ending in `break`/`continue` in a form neither a guard nor a kept loop's (`LoopForm`) |
| `nir_fs.rs:3917, 3924, 3962, 3993, 3996, 4003, 4039, 4045, 4057` | fragment, `walk_kept_loop` | a kept loop with no instruction, the colour written inside, a carried lane the body never defines, a body that always breaks, a head test that took no counter, a lane left as found, more than two or no `break` edges, two edges with different colours (`LoopForm`) |
| `nir_fs.rs:4186, 4191, 4200` | fragment, `walk_nodes` | `if (c) { step; continue; }` not at a kept body's top, or whose then side is more than the step (`LoopForm`) |
| `nir_fs.rs:3427-3428` | fragment, `walk_if16` | a mediump `if` whose side ends in `break` or `continue` (`ControlFlow`: the do-while bodies with an `if`, section 2) |
| `nir_fs.rs:4216-4217` | fragment, `walk_nodes` | a bare `continue` in a kept loop (`f.loops == 0`): `ControlFlow` |

How a loop reaches them (`walk_nodes`, `nir_fs.rs:4207-4214`): a
`Node::Loop` whose body's first control-flow node is `if (c) {} else
break;` (the front end's `Lower::break_unless`, `src/shaderc/online/lower.rs:2658`,
emitted for `for` and `while` at the body's head and for `do`/`while` at
its tail, `:2287-2298`) with `c` not foldable from constants and the
locals' constant lanes is a kept loop (`kept_loop`, `:3718-3727`,
`ir_folds` `:3731`); every other loop is unrolled pass by pass
(`walk_loop`, `:3655`; `walk_passes`, `:3525`). The decision is by the
first non-instruction node alone, which is right for every `for` and
`while` of dEQP and wrong for two forms (section 2, F4 and F5). The vertex
walk has no loop at all: `vs_steps` (`nir.rs:2619-2683`) is one straight
list of `Step`s with flattened `if`s (`Step::If/Else/EndIf`, `vs_join`),
and the vertex selection (`select.rs`) has no block, branch or successor
(`grep Branchz|new_block|add_successor select.rs`: none).

The three pins in `src/shaderc/online/mali/tests.rs:1201, 1211-1213`
(`while (true) { gl_Position = p; break; }`, `intloop_loop_nested.frag`,
`intloop_contloops.frag`, `intloop_loop_nested_vs.vert`) and the
`deadloop_*` pins (`:8702`, `:8725`) are what a lowering moves.

## 2. The dEQP shapes

### 2.1 What the generators write

`es2fShaderLoopTests.cpp` makes `shaders.loops.<type>_<count>_iterations.*`
for `type` in `for`, `while`, `do_while` and `count` in `constant`,
`uniform`, `dynamic`, each body for a vertex and a fragment variant:

- The fragment variant: `precision mediump float;`, `varying mediump vec4
  v_coords;` (`v_one` for a dynamic count), `mediump vec4 coords =
  v_coords; mediump vec4 res = coords;`, the loop, `gl_FragColor =
  vec4(res.rgb, 1.0);`. The vertex shader is a pass-through (`v_coords =
  a_coords`). The vertex variant: the same loop in the vertex shader on
  `mediump vec4 coords = a_coords;`, `v_color = vec4(res.rgb, 1.0)`
  (`varying mediump vec3 v_color` in the generic cases), the fragment
  shader `gl_FragColor = vec4(v_color.rgb, 1.0)`. So **every fragment loop
  is in a mediump shader and every carried vector is fp16 in both
  stages** (`coords`, `res` are `mediump vec4`; a vertex `a_coords` is
  `highp`, converted once).
- The bound: a literal (`i < 3`), an int uniform (`ui_three`, `ui_zero` ..
  `ui_six`, `ui_oneHundredOne`), or `one * ui_three` with `int one =
  int(a_one + 0.5)` (vertex) or `int(v_one + 0.5)` (fragment). The
  counter: `for (<prec> int i = 0; i < N; i++)`, `int i = 0; while (i++ <
  N)`, `int i = 0; do {..} while (++i < N)`. The generic `basic_<prec>_<int|float>`
  cases also use a float counter from 0.05 stepping by 1/3, a uniform
  fraction, or the fraction times `one`.
- The special bodies, all on `res = res.yzwx;`: `empty_body`,
  `single_statement`, `compound_statement`, `sequence_statement`,
  `single_iteration`, `no_iterations` (bound 0; not for do-while),
  `101_iterations`, `select_iteration_count` (bound `ub_true ? N : 0`, a
  bool uniform), `pre_increment`, `post_increment`, `unconditional_continue`
  (code after it dead), `only_continue`, `conditional_continue` (`if (i ==
  TWO) continue;`), `double_continue`, `conditional_break`,
  `unconditional_break`, `mixed_break_continue` (`if (i == 0) continue;
  else if (i == 3) break;`), `conditional_body` (`if (i < 2) res =
  res.yzwx;`), `infinite_with_unconditional_break_{first,last}`,
  `infinite_with_conditional_break` (`for (;;) { if (i == ONE) break; ..;
  i++; }`), `vector_counter` (an `ivec4` counter, `i.x += i.y`),
  `sequence`, `nested`, `nested_sequence`, `nested_tricky_dataflow_{1,2}`
  (the inner loop reassigns `res` or `coords`), `function_call_return`,
  `function_call_inout` (the rotation in a function). `ONE`, `TWO`, `THREE`
  are literals, `ui_*`, or `one*ui_*` by the count type.

`es2fShaderIndexingTests.cpp` makes the `*_static_loop_*` and
`*_dynamic_loop_*` cases of `tmp_array`, `vector_subscript`,
`matrix_subscript`, `uniform_array` and `varying_array`: `for (int i = 0;
i < 4; i++)` or `for (int i = 0; i < ui_four; i++)` (the vector and matrix
cases: `i < <vecLen>` or the uniform named for it) around `arr[i] =
T(coords); coords = coords * 0.5;` (a write), `res += arr[i];` or `tmp[i]
= coords.x; coords = coords.yzwx * 0.5;` (a read), the arrays and `res`
`mediump`, the counter a plain `int`. The tree's copy of one is
`tools/shaders/es100/dynidx_loop.frag`.

### 2.2 The shapes and their counts

Counts are cases of the 37305608 tree whose `refusals.tsv` entry is one of
the four texts, split by status (NS = NotSupported). "Next" is the refusal
the walk would meet after the loop, read from the code; the measure's
"next refusal" column is the authority once it exists.

| # | shape | FS now (cases) | VS now (cases) | Mesa 26.2.1 (section 3) | next after the loop |
|---|---|---|---|---|---|
| **F1** | one counted loop, or several in sequence or nested, body rotates `res` (basic_*_int, empty_body, single/compound/sequence_statement, single_iteration, no_iterations, pre/post_increment, only_continue, unconditional_continue, function_call_*, sequence, nested*, conditional_body for/while), bound `ui_*` or `one*ui_*` | `LoopMediump`, NS: for 24, while 24, do_while 20 (uniform) and the same bodies with a dynamic bound 71 (139 in all); plus `shaders.{struct,scoping,return}` 10 | `LoopVertex`, NS 160 (uniform and dynamic, all three loop types) | kept, one `loop {}` per source loop; 16-bit vec2 phis for `res`, `MKVEC.v2i16` pairs in the body | dynamic: `imul(ui, one)` of `int(v_one + 0.5)` (`IntValue`, study-intval step 1 and 2); `function_call_*`: none (inlined); `return_in_dynamic_loop_*`: the front end's return flag (`ret_dynloop_never` is `LoopMediump` too) |
| **F2** | a `break` or `continue` under a run-time condition inside a kept loop (conditional_break, conditional_continue, double_continue, mixed_break_continue, infinite_with_conditional_break) | `LoopMediump` NS 16 (for/while uniform); `GuardMediump` NS 6 (do_while uniform 3, infinite_with_conditional_break 3); `ControlFlow` NS 2 (do_while uniform conditional_body, mixed_break_continue); dynamic: `IntValue` first (9) | `LoopVertex` NS 45 | kept; `continue` as a branch to the latch or a select, `break` as a second exit edge with phi copies (divnest's form, lowered for highp) | the `ieq` of the counter with a uniform (`ICMP_OR.u32.eq`; `Fs::int_lane` has `ieq` against a constant only, `nir_fs.rs:1559-1625`, study-intval) |
| **F3** | a bound no constant decides though no uniform is read: select_iteration_count (`ub_true ? N : 0`) | `LoopMediump` NS 9 (all count types) | `LoopVertex` NS 9 | kept (no limiting terminator: the bound is a `bcsel` of a bool uniform) | `i2b`/`bcsel` of the bool uniform word (`UniformType` in the VS, `nir.rs:3236`) |
| **F4** | a constant bound past 32 passes: 101_iterations constant | `LoopForm` NS 2 + Fail 1 | `LoopVertex` Fail 1 + NS 2 | **kept** (trip count 101 > `max_unroll_iterations` 32), a constant bound in the compare | none |
| **F5** | a kept loop in a form `kept_loop` misreads: a do-while whose body holds an `if` (the tail test is not the first control node), `for (;;) { if (c) break; .. }` (the head is `if (c) break;`, not `if (c) {} else break;`) | the `GuardMediump` 6 and `ControlFlow` 2 of F2 | (in F2) | kept, as F2 | as F2 |
| **F6** | a loop whose last statement is `break` with a run-time bound: unconditional_break uniform/dynamic | `LoopMediump` NS 4 (`FsVaryingLoad` 4, Fail 1 for the rest) | `LoopVertex` NS 8, Fail 1 | **unrolled** by `wrapper_unroll` (the loop ends in `break`): one guarded pass, flattened to selects, no loop | the flattened `if` on `i < ui` (`walk_if16`), then `FsVaryingLoad` as the constant cases |
| **F7** | a float counter with a uniform or dynamic step (basic_*_float uniform/dynamic) | `LoopMediump` NS 36 (in F1's count) | `LoopVertex` NS 36 (in F1's count) | kept; the counter an fp16 lane (`FADD.v2f16`, `FCMP_OR.v2f16.ge`), the cap's OR a separate `LSHIFT_OR` | `uf_*` as fp16 uniform lanes (taken) |
| **F8** | vector_counter uniform (`ivec4 i`, `i.x += i.y`) | `LoopMediump` NS 3 (constant: `FsVaryingLoad` 3; dynamic: `IntValue` 3) | `LoopVertex` NS 3 (`UniformType` 6 for constant and dynamic) | kept; the induction variable one lane of a vector (nir_loop_analyze.c:1252-1260), the others constants | the `ivec4` constant lanes (`IntValue` in the VS) |
| **F9** | the vertex loops Mesa unrolls: every body with a constant bound of at most 32 passes, `for`/`while`/`do_while` | (the FS walk unrolls them already: Pass 50, `FsVaryingLoad` 54, `ConstantFold` 9) | `LoopVertex` **Fail 29** (for_constant, the STANDARD requirement) + NS 6 (for_constant's dynamic-requirement bodies) + NS 34 (while_constant) + NS 36 (do_while_constant) + Fail 4 (`functions.control_flow.return_after_*`), Fail 15 (`invariance.*.loop_*`), Fail 2 (`scoping.valid.for_init_*`), Fail 2 (`struct.*.loop_*`) | unrolled (`simple_unroll`; `complex_unroll` when a `break` is behind a run-time condition, then flattened) | `conditional_*`/`mixed_*` constant: a nested flattened `if` (`vs_steps` refuses a nested `if`, `ControlFlow`, `nir.rs:2636`); `invariance`: `Invariant` (study-misc); `return_after_*`: the return flag |
| **F10** | an array, vector, matrix, uniform-array or varying-array subscript by the counter of a loop with the bound `ui_four` (or the vector's own uniform): `indexing.*_dynamic_loop_*`, `struct.*.dynamic_loop_*` | `LoopMediump` NS 80 (tmp_array 24, vector_subscript 30, matrix_subscript 18, uniform_array 4, varying_array 4) + struct 5 | `LoopVertex` NS 110 (tmp_array 24, vector_subscript 30, matrix_subscript 18, uniform_array 4, varying_array 32) + struct 2 | **partially unrolled by the array's length** (`guess_loop_limit`, `partial_unroll`), then the leftover loop is dead: no loop expected, to be measured (section 3.3) | the subscript's select ladder (`IndexedLocal` in a kept loop, `stored_lanes` `nir_fs.rs:3845`; `dynidx`'s `ieq` ladder, study-uniarray) |
| **F11** | the same subscripts in a loop with a constant bound: `indexing.*_static_loop_*` vertex | (unrolled already) | `LoopVertex` Fail 54 (tmp_array 16, vector_subscript 18, matrix_subscript 12, uniform_array 4, varying_array 8) | unrolled | the constant subscripts (study-uniarray's `_static_loop` forms) |

Sum: FS 268 `LoopMediump` + 6 `GuardMediump` + 3 `LoopForm`; VS 392 NS +
105 Fail `LoopVertex`. The `ControlFlow` 2 are counted in section M under
"control flow", not in this family's rows.

The gl_test and glmark2 corpora have nothing of F3, F4, F6, F7, F8, F10
and F11, and only `loop_nested*` and `contloops` of F1/F2 and F5.

## 3. What Mesa 26.2.1 does

Source: `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1` (the tarball tree of
`tools/mali/README-pan_shc.md`). Lines are of that tree.

### 3.1 Unroll or keep: `nir_opt_loop_unroll`

Run twice: by the GL front end's optimisation loop
(`src/compiler/glsl/gl_nir_linker.c:112-115`, when
`max_unroll_iterations` is set) and by bifrost's (`src/panfrost/compiler/bifrost/bifrost_nir.c:268`,
in `bi_optimize_loop` `:202-276`, after `nir_opt_loop` `:247` and
`nir_opt_if` `:262-265`, before `nir_opt_remove_phis` `:269`). The
options: `max_unroll_iterations = 32` (`bifrost_compile.h:140`; the state
tracker's own copy `src/mesa/state_tracker/st_program.c:651`),
`force_indirect_unrolling = shader_in | shader_out | function_temp`
(`:142-143`), `force_indirect_unrolling_sampler` (`:144`).

`process_loops` (`src/compiler/nir/nir_opt_loop_unroll.c:990-1164`),
per loop, innermost first:

1. `:1070-1077` no limiting terminator and not complex: a loop whose last
   block ends in `break` is `wrapper_unroll` (`:547`): the break goes, the
   rest nests in the terminators' continue sides (at most 3 exits). This
   is F6 (`unconditional_break`) and `infinite_with_unconditional_break_last`.
2. `:1082-1088` no limiting terminator, one terminator, no nested loop,
   not yet partially unrolled, and a *guessed* trip count: `partial_unroll`
   (`:755`) by that count. The guess comes from an array indexed by the
   induction variable (`nir_loop_analyze.c:1228`, `guess_loop_limit`,
   `find_array_access_via_induction` `:400-427`: an array's or matrix's
   length, `:418-419`, or a vector's element count, `:421-422`; set as
   `guessed_trip_count` `:1342-1350`). This is F10 (section 3.3).
3. `:1112-1114` no limiting terminator (the trip count unknown: the bound
   a uniform, a varying, a select of a uniform) or a nested loop: the loop
   is kept. F1-F3, F5, F7, F8.
4. `:1116-1117` `check_unrolling_restrictions` (`:899-931`): `trip_count
   <= max_iter` (32) and `instr_cost * trip_count <= 32 * LOOP_UNROLL_LIMIT`
   (26, `:36`), or `force_unroll` with an exact count (`:921-922`). A
   constant bound past 32 is kept: F4.
5. `:1119-1121` an exact count: `simple_unroll` (`:105`). `:1124-1149`
   two terminators, one limiting: `complex_unroll` (`:338`); `:1151-1155`
   one terminator with the count unknown but bounded:
   `complex_unroll_single_terminator` (`:466`). F9.

Which pass round sees the loop matters for the fragment vs vertex counts:
the GL front end's loop (with its peephole limit 8,
`nir.rs:PEEPHOLE_LIMIT_GL`) runs before the uniforms become UBO loads and
before `lower_precision`'s fp16 is final; bifrost's after. The walk models
both calls already (`select_highp`, `nir_fs.rs:9135-9139`).

### 3.2 The kept loop in bifrost

`emit_loop` (`src/panfrost/compiler/bifrost/bifrost_compile.c:3738-3769`):
a `continue_block` (the header, `loop_header = true`, `:3748-3751`), the
body, a `JUMP` back to the header (`:3757-3759`), the `break_block` after
(`:3763`); `break` and `continue` are jumps to those blocks (`bi_emit_jump`,
`:85-94`); an `if` is `BRANCHZ.i16.eq cond.h0` over the then side and a
`JUMP` from its end (`emit_if`, `:3697-3735`). Then `bi_opt_control_flow.c`
removes the simple blocks (`is_simple_block` `:15`, `try_remove_simple_block`
`:98-147`, `inverse_cmpf` `:68`): a `break`'s block is simple, so the
`if` that skipped it is inverted and retargeted at the exit, which is why
a head test is `BRANCHZ` (taken when the mask is non-zero) and an `if`
around work is `BRANCHZ.eq`. `.reconverge` and the back edge's words are
measured in `mali-int-loops.md` ("How to read the Valhall output") and
transcribed by `flow.rs` (`insert_flow_control_nops` `:215`, the
scoreboard worklist over the back edge `:219-221`; `mark_last` `:545`).
The fragment walk already builds all of this for a highp shader
(`nir::LoopRegion`, `nir.rs:551-566`; the edges and blocks in
`select_highp`, `nir_fs.rs:9222-9253`; the jumps in `end_block`,
`:9978-10002`).

**Phis across the back edge.** `nir_lower_vars_to_ssa` (bifrost_nir.c:213)
makes one phi per carried variable at the header; `nir_lower_phis_to_scalar`
with `bi_vectorize_filter` (`:1090`) splits them to at most 32 bits each
(`bifrost_compile.c:3600-3601` asserts `bit_size * num_components <= 32`;
`vectorize_vec2_16bit`, `bifrost_compile.h:127`), so **a carried mediump
`vec4` is two 16-bit `vec2` phis** when its lanes stay paired, and four
16-bit scalar phis when a lane is written alone. `bi_out_of_ssa`
(`src/panfrost/compiler/bifrost/bi_ra.c:999-1057`) lowers each phi to a
move at the end of every predecessor (`:1022-1043`; a constant source is
`IADD_IMM.i32 r, 0x0, #k` on Valhall, `:1033-1040`) and one where the phi
stood (`:1045-1056`), propagated into the writer when it can be. Measured
forms: a paired phi's copy is `MOV.i32` of the whole word
(`intloop_loop_nested.expect.json` words 3-4, 9, 17; `mali-loop-break.md`
"loop_nested"), a scalar 16-bit phi's copy is `U16_TO_U32 r, r.h0`
(`intloop_contloops` words 51-54), the `yzwx` rotation two `MKVEC.v2i16`
(`r3 = (r6.h1, r2.h0)`, `r2 = (r2.h1, r6.h0)`), and a `continue` whose
other side is ALU alone becomes `MUX.v2i16` selects (`opt_loop_last_block`,
`nir_opt_loop.c:209`, then the peephole): `contloops` words 40-55. A
boolean carried across the back edge takes the width of its first source
(`pan_nir_lower_bool_to_bitsize.c:234-264`, `lower_phi_instr`). The int
counter is a 32-bit phi (`IADD_IMM.i32 rc, rc^, #0x1` in place), its test
`ICMP_OR.s32.ge.m1 rt, rc, u<bound>, 0x0` reading the FAU word of the int
uniform (`u2.w0` ..; the vertex stage's after its own `u0..u5.w0`).

The vertex stage emits the same blocks in the **varying** program
(`mali-vs-loops.md`, "Measured: loop_nested.vert": 53 words, `4 loops`,
`u5.w1`/`u6.w0` the bounds) and the usual 24-word position program when
the loop feeds only varyings; a loop that feeds `gl_Position` appears in
both programs (not measured). The do-while's test-before-body rotation
(`loop_uniform.dowhile.vert`: `++i`, the compare, the exit reads the phi
through a shifted swizzle) is measured there too.

### 3.3 The indexing loops (F10): what the source says, to be measured

(Measured since, in section 7, "What the references settle": the
`tmp_array` loops are gone, but a `vector_subscript` loop is kept.)

For `for (int i = 0; i < ui_four; i++) arr[i] = T(coords); coords *= 0.5;`
over `arr[4]`: the bound is not constant, so there is no limiting
terminator; `guess_loop_limit` gives 4 from the deref; `partial_unroll`
(`nir_opt_loop_unroll.c:755-800`) clones the body four times, each in the
continue side of the previous copy's test (`complex_unroll_loop_body`
`:237`), removes the accesses that would be out of bounds
(`remove_out_of_bounds_induction_use` `:782`) and re-inserts the loop,
marked `partially_unrolled`, in the innermost continue side (`:792-794`).
The re-inserted loop has no array access left: for the write loops only
`coords *= 0.5` and the counter remain, read by nothing after the loop;
for the read loops `res += arr[i]` is gone, so `res` passes through
unchanged. Such a loop has no side effect and no used result, so
`nir_opt_dead_cf` (bifrost_nir.c:244) should delete it. **Expected: no
back edge in Mesa's program, the four copies' guards `i < ui_four` (i a
constant per copy) flattened to selects, i.e. the `*_dynamic_loop_*`
cases unroll to the `*_static_loop_*` words plus a select ladder on
`ui_four`.** This is read from the code, not run: ref-loops draws
`tmp_array.float_dynamic_loop_write_static_read_fragment` and its vertex
twin first and reads `shaderdb`'s `loops` count. If it is 0, F10 is not a
kept-loop lowering at all but a `partial_unroll` transcription in both
walks (section 5, step 2c); if it is 1, F10 is F1 with a subscript ladder
inside.

## 4. Where our compiler stands, per shape

- **F1, F7 (fragment)**: `kept_loop` is right; `walk_kept_loop` refuses at
  its first line. Everything after it (`stored_lanes`, the header phis in
  `registration_order`, the cap, the head test's inverse compare and
  `ior` with the counter, the exit edges) is stage-neutral and already
  runs for highp. What is missing is in the selection: the mediump route
  of `mod.rs:1437-1477` sends a shader with blocks to `select_highp` only
  for `guarded16`, `kept16` or `frag32`; a kept mediump loop needs the
  same route (`select_highp` with `fs16`'s UBO layout), the phis as
  16-bit nodes (`bits: 16` in `walk_kept_loop` `:3966-3970` already sets
  them for a mediump float local), `vectorize16` (`:8787`) pairing the
  carried lanes into `vec2` phis where NIR keeps them paired, and the emit
  rows for a 16-bit two-lane phi copy (`MOV.i32`), a one-lane one
  (`U16_TO_U32`) and the `vec2` rebuild (`MKVEC.v2i16 a.h1, b.h0`).
  `branch_forms` (`:8606`) and `ladder_forms` (`:8707`) refuse a phi or a
  `Vec2` of the forms the new route needs; they are the gates to widen.
- **F2 (fragment)**: `walk_if` already makes a `break` under a run-time
  condition an exit edge (`brk`, `:3272-3282`) and `walk_nodes` the
  `if (c) { step; continue; }` form (`:4184-4203`), both highp-proven on
  divnest. The `continue` dEQP writes is `if (i == TWO) continue;` with
  no step inside (the front end copies the step before each `continue`,
  `Lower::loop_body`, so the then side is `step; continue`), which is the
  form taken. `double_continue` has two such `if`s in sequence: the second
  is not at the body's top (`:4185-4186`, `LoopForm`). `mixed_break_continue`
  has `if (i == 0) continue; else if (i == 3) break;`: an else side with a
  `break`, no reference. The compare is `ieq(i, ui_two)` of two run-time
  ints, refused by `Fs::int_lane` (`:1624`) today; its lowering is
  study-intval's step 1, or ref-loops' reference carries it.
- **F3**: the bound `ub_true ? ui_three : 0` is a `bcsel` of a bool
  uniform word: `kept_loop` sees a non-foldable test, right; the `bcsel`
  needs the bool-uniform read (`Fs::int_lane` has `i2b` of a bool uniform;
  the VS has `UniformType`).
- **F4**: `walk_passes` refuses at pass 33. The fix is in `kept_loop`: a
  constant trip count past 32 (or a cost past 26 * 32, `unroll_cost`) is
  a kept loop. The cap: `loop_cap_limit` reads the constant count (101), so
  the counter never fires.
- **F5**: `kept_loop` must look at the loop's *terminators* as
  `nir_loop_analyze` does, not at the first control node: a do-while whose
  tail test does not fold is kept whatever its body holds; `for (;;) { if
  (c) break; }` with `c` not foldable is kept with its exit test in the
  `if (c) break;` polarity (the compare not inverted; `try_remove_simple_block`
  then gives the same `BRANCHZ` on the direct compare). Both forms also
  need the mediump `walk_if16` to leave `break`/`continue` sides to the
  kept-loop code, which it refuses at `:3427`.
- **F6**: `kept_loop` says kept, Mesa unrolls (`wrapper_unroll`). The
  walk's `walk_passes` would unroll it correctly (pass 1 ends in `Flow::Break`)
  if `kept_loop` returned false for a body whose last node is `Break`.
- **F8**: the induction variable is a lane of an `ivec4` local; the walk
  keeps locals per lane, so the counter's lane is a scalar phi as Mesa's.
  The other lanes are constants folded away (`nir_opt_remove_phis`).
- **F9, F11 (vertex, unrolled)**: nothing in the vertex walk unrolls. The
  fragment's `walk_loop`/`walk_passes`/`walk_guard` are the model: a pass
  walked with the counter folded (`fold_int`, `nir.rs:815-860`, has `iadd`,
  `ilt`, `ieq` of constants), the test `if (c) {} else break;` taken on
  the folded side, the `break` ending the chain, the cost rule. The vertex
  walk's int arithmetic on *run-time* values is `IntValue` (`nir.rs:3211`,
  `:3236`), but an unrolled counter never reaches it. A `for` with a
  run-time `break` (`conditional_break` constant) is `complex_unroll`:
  nested `if`s, flattened by the peephole (limit 64) into `bcsel`s, which
  `vs_steps` refuses as a nested `if` (`:2636`); those stay `ControlFlow`
  after step 1 unless `vs_join` learns nesting (not in this plan's scope:
  they are 10 Fail at most, `for_constant_iterations.{conditional_*,
  mixed_*}_vertex` and `double_continue_vertex`).
- **F1-F8 (vertex, kept)**: `mali-vs-loops.md` section 3, steps 1-5, as
  written: int uniforms by value in `build`, fp16 vectors in the vertex
  walk (`V2F32_TO_V2F16` pairs exist, `nir.rs:2273-2277`; `MKVEC.v2i16`
  swizzles and 16-bit vector phis do not), `Step::LoopBegin/LoopEnd` into
  a `LoopRegion`, blocks in the vertex selection (`select.rs` has none;
  `bir::Prog` has `new_block`/`add_successor`, `bir.rs:1322, 1332`, and
  `flow.rs`, `opt.rs`, `ra.rs`, `sched.rs` take back edges since
  loops-rt), the pass counter, the do-while rotation.
- **F10**: see 3.3; the walk's `IndexedLocal` (`stored_lanes` `:3845`)
  refuses an indexed store in a kept loop in any case.

## 5. Lowering plan: two steps, one session each

Each step is byte-compared to `deqp_loops_*` references that item
ref-loops makes from the measure's pairs with `tools/mali/deqp_ref.sh`
(item ref-tool; the pan_shc/drm-shim method of `tools/mali/testdata/gm2_*`),
in `tests/mali_deqp_loops.rs` through `tests/mali_ref/mod.rs`
(`Reference::load`, `compile_pair`, `assert_matches`, `assert_refused`).
A kept loop's words carry the pass counter, which the shared compare does
not edit: a kept-loop reference is compared in `src/shaderc/online/mali/tests.rs`
with `CapEdit` (`mali-loop-board-failures.md`, "The byte-equality tests"),
as the five highp ones are, and `tests/mali_deqp_loops.rs` pins the
unrolled ones and asserts the kept ones compile (`assert_refused` moved to
a compile check). dEQP's bounds are at most 101, so the counter never ends
a loop early; the deviation stays. Any glmark2 verdict that moves updates
`tests/fixtures/mali_glmark2_verdicts.txt` in the same commit, and
`tools/mali/corpus-verdicts.txt` moves `es100/loop_nested*.frag`,
`loop_uniform.frag`, `loop_nested.vert`, `loop_uniform.dowhile.vert`,
`contloops.frag`, `dynidx_loop.frag` (`corpus-verdicts.sh --write`).

### Step 1 (lower-loops-1): the vertex loops Mesa unrolls, and the mediump fragment kept loop

Two parts in two files; (a) moves the family's Fail, (b) its largest
NotSupported block.

**(a) `nir.rs`: unrolled vertex loops (F9, F11; 104 Fail, 76 NS).** In
`vs_steps`, replace `Step::Refuse(LoopVertex)` at `:2660` by the passes'
steps: walk the body as `walk_passes` does, pass after pass, folding the
counter (`fold_int`) so the head test `if (c) {} else break;` is a
`Step::If` on a constant whose side the walk's `Step::EndIf` arm takes
(the `Val::Const` arm, `:2886-2890`), until a pass ends in `break`; the
`MAX_UNROLL_ITERATIONS` and `unroll_cost` rules of the fragment walk
decide (past them the loop stays `LoopVertex`, which step 2 takes). A
`for`'s step before each `continue` (`Lower::loop_body`) and the
`while`'s `i++` in the test are instructions of the pass; a `do`/`while`
puts the test last. A loop in an `if` side (`:2635`) stays refused. The
mediump `res = res.yzwx` passes are fp16 nodes with `V2F32_TO_V2F16` of
`a_coords` once and `F16_TO_F32` at the stores: the unrolled varying
program of `gm2_intloop_const_vs`'s shape with a vector instead of a
float, which no vertex reference has yet; its words (whether the three
rotations fold into one swizzle of the halves, as NIR's `vec2` rebuilds
and copy propagation suggest) come from the reference.

**(b) `nir_fs.rs`: the mediump kept loop (F1, F7, part of F2; 268 NS).**
Lift `:3909-3910`; route a mediump shader with a `LoopRegion` to
`select_highp` with the `fs16` layout (`mod.rs:1461-1470`, a `kept16`-like
predicate over `f.nir.nloops`); pair the carried lanes (`vectorize16`
takes one-lane nodes; a header phi of two lanes of one local whose
sources are a `Vec2` or a pair of the same node is one 16-bit two-lane
phi); emit rows `(Phi, 16, 2)` as `MOV.i32`, `(Phi, 16, 1)` as
`U16_TO_U32`, `(Vec2, 16, 2)` of two halves as `MKVEC.v2i16`; the
colour's fp16 pairs after the loop go to the blend as `select` sends them
(`vec4(res.rgb, 1.0)`: the loop's pairs and a constant half). Widen
`branch_forms`/`ladder_forms`
to a phi, a `Vec2` and an `Icmp` of 32-bit ints. `kept_loop` gets the
three decision fixes of F4, F5 and F6 (a constant count past the limits
is kept; the terminators decide, not the first control node; a body
ending in `break` is unrolled), so `GuardMediump` and `ControlFlow` of F5
become the kept path and `LoopForm` of F4 compiles. `walk_if16` passes a
side ending in `break` or `continue` on to `walk_if` when `f.kept` is set.

References for step 1 (ref-loops; the dEQP case each is copied from, both
stages drawn as the pair dEQP links):

- `deqp_loops_vs_for_const`: `for_constant_iterations.basic_mediump_int_vertex`
  (the plain unrolled rotation).
- `deqp_loops_vs_while_const`, `deqp_loops_vs_dowhile_const`:
  `while_constant_iterations.single_statement_vertex`,
  `do_while_constant_iterations.post_increment_vertex` (the test's place).
- `deqp_loops_vs_nested_const`: `for_constant_iterations.nested_sequence_vertex`.
- `deqp_loops_vs_idx_static`: `indexing.tmp_array.vec4_static_loop_write_static_loop_read_vertex` (F11, with study-uniarray's static subscripts).
- `deqp_loops_fs_for_uni`: `for_uniform_iterations.basic_mediump_int_fragment` (= `tools/shaders/es100/loop_uniform.frag`; `intloop_loop_nested` is the nested twin and is already in the tree).
- `deqp_loops_fs_while_uni`, `deqp_loops_fs_dowhile_uni`:
  `while_uniform_iterations.basic_mediump_int_fragment`,
  `do_while_uniform_iterations.basic_mediump_int_fragment` (the do-while's
  rotated form in the fragment stage).
- `deqp_loops_fs_float_uni`: `for_uniform_iterations.basic_mediump_float_fragment` (F7).
- `deqp_loops_fs_101`: `for_constant_iterations.101_iterations_fragment` (F4: the constant bound's form).
- `deqp_loops_fs_dowhile_if`: `do_while_uniform_iterations.conditional_body_fragment` (F5).
- `deqp_loops_fs_uncond_break`: `for_uniform_iterations.unconditional_break_fragment` (F6: no loop expected).
- `deqp_loops_fs_select`: `for_uniform_iterations.select_iteration_count_fragment` (F3; needs the bool uniform read).

Expected after step 1: VS Fail 105 -> about 10 (`conditional_*`/`mixed_*`
constant `for`, F9's nested flattened `if`s; the `invariance` and
`return_after_*` ones move to their next refusal); FS NS 268 -> the
`IntValue` of the dynamic bounds (about 70, study-intval) and the
subscript ladders of F10 (85, pending 3.3).

### Step 2 (lower-loops-2): kept vertex loops, the fragment `break`/`continue` forms, and F10

**(a) `nir.rs` + `select.rs`: kept vertex loops (F1-F8 vertex; 392 NS).**
`mali-vs-loops.md` section 3 as written, in its order: the `int` uniform
read by value for the compare (`nir.rs:2927`: a `GL_INT` uniform not
`index_only` is `UniformType`, today's first refusal after the loop,
`mali-vs-loops.md` section 1), the
fp16 vector phis and `MKVEC.v2i16` in the vertex walk (step 1a made the
nodes; this makes them carried), `Step::LoopBegin`/`LoopEnd` into a
`LoopRegion` with the header phis in `registration_order`, blocks and the
jumps in the vertex selection for the varying program (and the position
program when the loop feeds `gl_Position`: a reference with such a loop is
needed first, `deqp_loops_vs_position` below), the pass counter, the
do-while rotation. `loop_nested_vs` (`intloop_loop_nested_vs`) and
`loop_uniform.dowhile.vert` are the two references already in the tree
(the second as `.stderr` counts only: 29 words, 1 loop).

**(b) `nir_fs.rs`: `break` and `continue` under a run-time condition in a
mediump kept loop (F2, F5; 24 NS)** on top of step 1b: the second exit
edge with phi copies (`walk_if`'s `brk`), a `continue` whose rest is ALU
alone as `MUX.v2i16` selects (contloops words 40-55: `opt_loop_last_block`
then the peephole, the degenerate `MUX` kept), two `continue`s in sequence
(`double_continue`: the second `if` is not at the top, `:4185`; Mesa's
`nir_lower_continue_constructs` moves the rest of the body into each
`if`'s else side in turn), `mixed_break_continue`'s `else if (..) break;`,
and `infinite_with_conditional_break`'s head `if (i == ui_one) break;`
polarity. The `ieq` of two run-time ints is study-intval's; whichever
lands first carries it.

**(c) F10 by the measurement of 3.3 (85 FS NS, 112 VS NS).** If the
reference has no loop: a `partial_unroll` transcription in both walks (the
guess from the subscripted array's length, four guarded copies in the
guard-nesting of `walk_guard`, the out-of-bounds copies dropped, the
leftover loop dead), then the subscripts are study-uniarray's static
forms. If it has one: F1's kept loop with an `IndexedLocal` select ladder
inside (`stored_lanes` lifted for a local whose index is the counter).
The ES 1.00 cases set `ui_four = 4`, equal to the array length, so either
output draws the same picture. Measured (section 7): the `tmp_array`
references have no loop and no read of `ui_four`, but the
`vector_subscript` one keeps both loops.

References for step 2:

- `deqp_loops_vs_for_uni`: `for_uniform_iterations.basic_mediump_int_vertex` (one kept loop, one carried fp16 vector; `intloop_loop_nested_vs` is the nested one).
- `deqp_loops_vs_dowhile_uni`: `do_while_uniform_iterations.basic_mediump_int_vertex` (= `tools/shaders/es100/loop_uniform.dowhile.vert`, no `expect.json` yet).
- `deqp_loops_vs_float_uni`: `for_uniform_iterations.basic_highp_float_vertex` (F7 at fp32 in the vertex stage).
- `deqp_loops_vs_position`: a loop whose result reaches `gl_Position` (not a dEQP case: `gm2_cf_loop_uni_vs`'s shape; if #367 has drawn it, reuse theirs).
- `deqp_loops_vs_select`, `deqp_loops_vs_vector`: `select_iteration_count_vertex`, `vector_counter_vertex` (F3, F8).
- `deqp_loops_fs_cond_break`, `deqp_loops_fs_cond_continue`, `deqp_loops_fs_double_continue`, `deqp_loops_fs_mixed`: the four `for_uniform_iterations.*_fragment` bodies of F2 (`intloop_contloops` holds the `while` form).
- `deqp_loops_fs_inf_break`: `for_uniform_iterations.infinite_with_conditional_break_fragment` (F5's `if (c) break;` head).
- `deqp_loops_fs_idx_dyn`, `deqp_loops_vs_idx_dyn`: `indexing.tmp_array.float_dynamic_loop_write_static_read_{fragment,vertex}` (3.3: drawn first, `shaderdb`'s `loops` read before anything else of (c) is planned).
- `deqp_loops_fs_vec_dyn`: `indexing.vector_subscript.vec4_dynamic_loop_subscript_write_dynamic_loop_subscript_read_fragment` (= `tools/shaders/es100/dynidx_loop.frag`'s family).

Expected after step 2: VS NS 392 -> the `UniformType`/`IntValue` of
dynamic bounds and `select_iteration_count`'s bool (about 90, study-intval
and study-unitype); FS 0 of this family left; F10 by (c).

### Order against #367

#367 owns the glmark2 `loop` scene (`LOOP_VERTEX` `3cb0`, `LOOP_FP16`
`3678`): a kept int-uniform loop carrying one float, in each stage. If
#367's items land first, step 1b and step 2a shrink to the dEQP forms
(fp16 vector phis, `MKVEC.v2i16`, nesting, do-while, the decision fixes)
over their block emitters; if this plan's land first, #367's shaders are
F1 with a scalar instead of a vector and should compile from step 1b
(fragment) and step 2a (vertex) without more. Either way the two must not
both add a vertex block emitter: the item that runs second rebases on the
first's `LoopRegion` use in `nir.rs`/`select.rs`. The decision fixes of
`kept_loop` (F4-F6) and the vertex unroll (step 1a) touch neither
glmark2 shader's words (`gm2_intloop_const_vs` is a five-pass `for` the
vertex unroll would take; its verdict in the fixture moves forward and is
checked against its reference in the same commit).

## 6. Caveats

- Counts are the 37305608 tree's first refusals; a case's *next* refusal
  is read from the code (section 2.2, "next"), not measured. The measure
  item's table replaces that column.
- The generators were read in summary form (the fetch tool quotes at most
  a line); the loop headers and bodies above agree with the tree's own
  copies (`loop_nested.{frag,vert}`, `loop_uniform.frag`,
  `loop_uniform.dowhile.vert`, `dynidx_loop.frag`, `contloops.frag`'s
  comment) and with the case names. ref-loops works from the committed
  pairs, not from this text.
- 3.3 (the indexing loops partially unrolled and the rest deleted) is an
  inference from `nir_opt_loop_unroll.c` and `nir_loop_analyze.c`; whether
  `nir_opt_dead_cf` really removes the leftover loop, and in which pass
  round, is the first thing ref-loops measures.
- That Mesa keeps `select_iteration_count`'s loop even with a constant
  count type (the bound `ub_true ? 3 : 0` is a `bcsel` of a UBO load,
  never a constant for the compiler) is inferred from `nir_loop_analyze`'s
  need for a constant limit (`:1228-1235`), not drawn.
- The vertex position program of a loop that feeds `gl_Position`, and the
  board's behaviour on a kept vertex loop (the pass counter), are the open
  points of `mali-vs-loops.md` section 6, still open.
- The do-while test-before-body rotation is measured in the vertex stage
  only; the fragment stage's form (`deqp_loops_fs_dowhile_uni`) is assumed
  the same NIR rewrite. (Measured since, in section 7: it is.)

## 7. References (item ref-loops, made 2026-11-16)

Each reference is `tools/mali/testdata/deqp_loops_<name>.{vert,frag,expect.json}`.
The shaders are dEQP-GLES2's text (Khronos, Apache-2.0), and each has a
`.license` sidecar that names the case and the generator lines. There are 26
references for the 27 names of section 5. `deqp_loops_vs_position` was not
made, as section 5 allows: #367's `gm2_cf_loop_uni_vs` is that shape. It is
a vertex loop with an int-uniform bound that feeds `gl_Position`, and its
loop is in the **position** program (51 instructions, 1 loop; the varying
program is empty). It was made again with this build and came out byte-identical.

**Source.** The tip's `tools/mali/testdata/deqp_shapes/` has no pair for any
of the 26 cases. Every pair was therefore **rebuilt** from VK-GL-CTS
`5bb4c7e8` (main, 2026-11-16), substituting the templates the way
`StringTemplate` does:

- `L` = `modules/gles2/functional/es2fShaderLoopTests.cpp`:
  `createGenericLoopCase` `:284-492` for the `basic_*` cases;
  `createSpecialLoopCase` `:495-1273` for the rest, with the bodies at the
  lines in the table.
- `I` = `modules/gles2/functional/es2fShaderIndexingTests.cpp`:
  `createTmpArrayCase` `:523-708` and `createVectorSubscriptCase` `:738-931`.
- The uniform names are from `getIntUniformName` /
  `getFloatFractionUniformName` (`modules/glshared/glsShaderRenderCase.cpp:872-950`).

The rebuild was checked against the measure's verbatim `.qpa` text. All
seven `deqp_shapes/loops_*` pairs that come from these two generators
(`loops_fs_kept_while`, `loops_fs_kept_do_while`, `loops_fs_past_limits`,
`loops_fs_unrolled_break`, `loops_vs_for_dynamic`, `loops_vs_while`,
`loops_vs_do_while`; 14 files) came out byte-identical. That covers the
generic and the special generators and all three loop types. The tab
characters dEQP writes (`\t` before a `while`/`do` counter) are kept.

**Command** (the same for all 26):

```sh
LLVM_DIR=/usr/lib/llvm-21 tools/mali/drm_shim_ref.sh build \
    $CONNECTED_DEPS_DIR/downloads/mesa-26.2.1.tar.xz $CONNECTED_DEPS_DIR/mesa-shim   # once
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim \
    tools/mali/testdata/deqp_loops_<name>.vert tools/mali/testdata/deqp_loops_<name>.frag <out>/deqp_loops_<name> \
    --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract <out>/deqp_loops_<name>.trace --draw \
    > tools/mali/testdata/deqp_loops_<name>.expect.json
```

The tarball's sha256 matches `third_party/mesa-26.2.1/UPSTREAM`. The draw
is 64x64, triangles, with `--uniform-values`, as `deqp_ref.sh` draws:
gl_ref.py gives an `int` uniform its values as integers and a `bool` one
(`ub_true`) true where its component is even. A uniform's value does not
change the program: `gm2_intloop_*` were made without the flag, and the
build gives `gm2_intloop_uniform_fs` and `gm2_intloop_const_vs` again byte
for byte, apart from the uniforms' FAU words. The build was checked
first: `gm2_cf_loop_uni_vs` and `gm2_cf_ifelse_fs`, made with
`--uniform-values` as they were, are byte-identical. All 26 references were
made twice, and the two sets of `expect.json` are byte-identical.

**Comparing.** A kept loop (`loops` 1 or 2 below) carries the pass counter
the walk adds (`mali-loop-board-failures.md`), which these words do not
have. Such a reference is compared in `src/shaderc/online/mali/tests.rs`
with `CapEdit`, not through `tests/mali_ref/mod.rs` (section 5). The others
are plain byte compares. `mod.rs` computes the FAU words for
`--uniform-values`, so a test with these references must expect 0 for
every uniform word.

### Step 1 (lower-loops-1)

`instrs/loops` are the shaderdb counts of the program the shape is about
(`vary` = the vertex varying program, `frag` = the fragment program). The
position program of every reference is the same 24-word pass-through. The
"`I`" lines in the source column are the case's own lines in the generator.

| reference | dEQP-GLES2 case (`functional.shaders.`) | source (rebuilt) | instrs/loops | what it pins |
|---|---|---|---|---|
| `vs_for_const` | `loops.for_constant_iterations.basic_mediump_int_vertex` | `L:284-492` (for `:418-424`) | vary 11/0 | (a) the vertex unroll. Three `yzwx` rotations fold into the store order: no fp16 conversion is left, and the three lanes of the fp32 attribute are stored as `.wxy` |
| `vs_while_const` | `loops.while_constant_iterations.single_statement_vertex` | `L:796-798`, `WHILE_LOOP` `:965-967` | vary 11/0 | (a) `while(i++ < 3)`: the same words as `vs_for_const` |
| `vs_dowhile_const` | `loops.do_while_constant_iterations.post_increment_vertex` | `L:1069-1073` | vary 11/0 | (a) the test after the body, 4 passes (`i++ < 3`) |
| `vs_nested_const` | `loops.for_constant_iterations.nested_sequence_vertex` | `L:702-711` | vary 11/0 | (a) nested and sequenced constant loops, 9 passes |
| `vs_idx_static` | `indexing.tmp_array.vec4_static_loop_write_static_loop_read_vertex` | `I:523-708` (`:602-609`, `:636-640`) | vary 30/0 | (a) F11: an unrolled array write and read, constant subscripts, fp16 round trips (`V2F32_TO_V2F16`, `F16_TO_F32`) |
| `fs_for_uni` | `loops.for_uniform_iterations.basic_mediump_int_fragment` | `L:284-492` | frag 17/1 | (b) the mediump kept loop: `MOV.i32` of the carried pair, `ICMP_OR.s32.ge.m1` against `u2.w0`, `BRANCHZ` exit, two `MKVEC.v2i16` for `yzwx`, `IADD_IMM` counter, the back edge |
| `fs_while_uni` | `loops.while_uniform_iterations.basic_mediump_int_fragment` | `L:284-492` (while `:425-433`) | frag 17/1 | (b) the same words as `fs_for_uni` |
| `fs_dowhile_uni` | `loops.do_while_uniform_iterations.basic_mediump_int_fragment` | `L:284-492` (do `:434-442`) | frag 18/1 | (b) the do-while rotation in the fragment stage: the counter's `IADD_IMM` comes before the compare, and the exit takes the pre-rotation pair (a third `MKVEC.v2i16` after the loop) |
| `fs_float_uni` | `loops.for_uniform_iterations.basic_mediump_float_fragment` | `L:284-492` | frag 17/1 | (b) F7: an fp16 float counter (`FADD.v2f16`, `FCMP_OR`) against `uf_one` |
| `fs_101` | `loops.for_constant_iterations.101_iterations_fragment` | `L:680-683` | frag 18/1 | (b) F4: kept. The constant 101 is a FAU word (`u2.w0` = `0x65`), and the program has the words of `fs_dowhile_uni` (Mesa's first pass is unconditional, so the `for` rotates). It is not unrolled to 101 |
| `fs_dowhile_if` | `loops.do_while_uniform_iterations.conditional_body_fragment` | `L:1151-1154` | frag 22/1 | (b) F5: a do-while with an `if` in its body is kept. The `if` is `MUX.v2i16` selects, not a branch |
| `fs_uncond_break` | `loops.for_uniform_iterations.unconditional_break_fragment` | `L:656-659` | frag 13/0 | (b) F6: no loop (`wrapper_unroll`), one `ICMP_OR` on `ui_three` and three `MUX.v2i16` |
| `fs_select` | `loops.for_uniform_iterations.select_iteration_count_fragment` | `L:628-630` | frag 19/1 | (b) F3: kept. The bound is `CSEL.u32.eq` of the bool uniform's word (`u2.w0`) between `ui_three` (`u2.w1`) and 0, computed in the loop header |

### Step 2 (lower-loops-2)

| reference | dEQP-GLES2 case (`functional.shaders.`) | source (rebuilt) | instrs/loops | what it pins |
|---|---|---|---|---|
| `vs_for_uni` | `loops.for_uniform_iterations.basic_mediump_int_vertex` | `L:284-492` | vary 24/1 | (a) a kept vertex loop in the varying program: `V2F32_TO_V2F16` pairs, the same loop as `fs_for_uni` against `u5.w1`, `F16_TO_F32` per stored lane |
| `vs_dowhile_uni` | `loops.do_while_uniform_iterations.basic_mediump_int_vertex` | `L:284-492` | vary 24/1 | (a) the vertex do-while rotation (`loop_uniform.dowhile.vert`'s shape, now with words) |
| `vs_float_uni` | `loops.for_uniform_iterations.basic_highp_float_vertex` | `L:284-492` | vary 24/1 | (a) F7, vertex: a highp float counter (`FCMP_OR` against `uf_one`) |
| `vs_select` | `loops.for_uniform_iterations.select_iteration_count_vertex` | `L:628-630` | vary 26/1 | (a) F3, vertex: the `CSEL` bound of `ub_true` |
| `vs_vector` | `loops.for_uniform_iterations.vector_counter_vertex` | `L:675-678` | vary 24/1 | (a) F8: the `ivec4` counter is one scalar phi. The words are the same as `vs_for_uni` |
| `fs_cond_break` | `loops.for_uniform_iterations.conditional_break_fragment` | `L:651-654` | frag 19/1 | (b) F2: there is no second exit edge. The break folds into the head's exit test: `ICMP_OR.u32.eq.m1` of the counter and `ui_two` (two run-time ints), `LSHIFT_OR.i32` with the bound's mask, one `BRANCHZ` |
| `fs_cond_continue` | `loops.for_uniform_iterations.conditional_continue_fragment` | `L:632-635` | frag 21/1 | (b) F2: the `continue` becomes `MUX.v2i16` selects, with no branch |
| `fs_double_continue` | `loops.for_uniform_iterations.double_continue_fragment` | `L:646-649` | frag 21/1 | (b) the same words as `fs_cond_continue` |
| `fs_mixed` | `loops.for_uniform_iterations.mixed_break_continue_fragment` | `L:669-673` | frag 23/1 | (b) `if (i == 0) continue; else if (i == 3) break;`. The break folds into the exit test (`ICMP_AND.u32.eq` against a byte-lane 3, gated by `i != 0`, then `LSHIFT_OR` with the bound's mask). The continue becomes four `MUX.v2i16` on `i != 0` |
| `fs_inf_break` | `loops.for_uniform_iterations.infinite_with_conditional_break_fragment` | `L:594-598` | frag 18/1 | (b) F5: the head `if (i == ui_one) break;` |
| `fs_idx_dyn` | `indexing.tmp_array.float_dynamic_loop_write_static_read_fragment` | `I:523-708` (`:612-618`, `:622-628`) | frag 11/0 | (c) F10, a `tmp_array`: **no loop and no read of `ui_four`**. The program is the static sum (`FMA.v2f16` by 0.5, 0.25, and the `0x30003000` FAU constant) |
| `vs_idx_dyn` | `indexing.tmp_array.float_dynamic_loop_write_static_read_vertex` | `I:523-708` | vary 23/0 | (c) F10, vertex: the same, with no loop |
| `fs_vec_dyn` | `indexing.vector_subscript.vec4_dynamic_loop_subscript_write_dynamic_loop_subscript_read_fragment` | `I:738-931` (`:836-842`, `:885-888`) | frag 44/2 | (c) F10, a vector subscript: **both loops kept**. The write is an `ICMP_OR.u32.eq` ladder against byte-lane constants into `MUX.v2i16`, and the read is an `ICMP_OR.s32.lt` ladder |

### What the references settle

- **F10 is two shapes.** For an *array* subscripted by the counter
  (`tmp_array`, and by the same rule `matrix_subscript`, `uniform_array`,
  `varying_array`), the first measured case leaves no loop: the
  `guess_loop_limit`/`partial_unroll` copies are left. The copies'
  guards on `ui_four` also go, because a guarded-off element is undefined.
  What remains is the `static_loop` program (step 2c: a transcription, no
  kept loop). A *vector* subscript keeps its loops (`fs_vec_dyn`), with the
  index ladders inside: F1 plus `IndexedLocal` in a kept loop. Only the
  `tmp_array` case was drawn; the matrix, uniform-array and varying-array
  `dynamic_loop` cases are expected to follow the array rule, and the
  measure's `loops_fs_kept_for_index` / `loops_vs_for_index` pairs
  (`matrix_subscript`) are the ones to draw to confirm it.
- **F2 has no second exit edge in dEQP's forms.** In `fs_cond_break` and
  `fs_mixed`, the `break` behind `i == N` is OR-ed into the loop's single
  exit test, and a `continue` becomes `MUX.v2i16` selects. Step 2b's
  "second exit edge with phi copies" is therefore not needed for these
  cases. What they need is the 32-bit `ICMP_OR`/`ICMP_AND` of the counter
  and `LSHIFT_OR` into the exit mask.
- **The fragment do-while** rotates as the vertex one does
  (`fs_dowhile_uni`). `fs_101` is the same program, because a `for` whose
  first test is known to pass gets the same rotation.
- **Same words from different forms:** `for`/`while` (`fs_for_uni` =
  `fs_while_uni`, `vs_for_const` = `vs_while_const`), the `ivec4` counter
  (`vs_vector` = `vs_for_uni`), and one or two `continue`s
  (`fs_cond_continue` = `fs_double_continue`). They are kept as separate
  references because each pins a different front-end path to the same
  words.
- **The constant vertex loops** need no fp16 at all: the rotations of a
  mediump copy of a highp attribute fold into the store order
  (`vs_for_const`, 11 words).

## 8. Lowering map (at 008306cb)

Item map-loops of plan #377. Read at the tip `008306cb`; the sites of
section 1 have moved since `970d7912`, and the lines below are the tip's. A
map only: no code, test or reference changed. Paths are under
`src/shaderc/online/mali/` unless they start with `tests/` or `tools/`.
Read: `mod.rs` (the texts `:484-493`, the mediump route `:1437-1471`),
`nir.rs` (`vs_steps`, `build`, `fold_int`, `LoopRegion`), `nir_fs.rs` (the
walk, `select_highp`, `end_block`, `branch_forms`, `ladder_forms`,
`vectorize16`), and the back-end files a back edge reaches (`flow.rs`,
`sched.rs`, `ra.rs`, `opt.rs`, `emit.rs`): they take blocks and a back edge
since loops-rt (`mali-loop-break.md` Part 2) and raise none of the loop
refusals.

### 8.1 The raise sites at the tip

| error | file:function (line) | condition |
|---|---|---|
| `LoopVertex` | `nir.rs:vs_steps` (`:2770`) | any top-level `IrNode::Loop` of `main`: a `Step::Refuse`, returned by `build` where the node stands |
| `LoopVertex` | `nir.rs:vs_steps`, closure `flat` (`:2745`) | a `Loop` inside an `if` side (an earlier `Tex`/`ControlFlow` refusal of the side comes first, `:2737-2744`) |
| `LoopMediump` | `nir_fs.rs:walk_kept_loop` (`:3914-3916`) | `!prec.highp`; reached from `walk_nodes` (`:4212`) when `kept_loop` (`:3723`) says Mesa keeps the loop |
| `GuardMediump` | `nir_fs.rs:walk_if16` (`:3410-3412`) | a `break`/`continue` guard (`exit` not `IfExit::Plain`) in a loop the walk unrolls in a mediump shader (`fs_inf_break`: `kept_loop` misses its `if (c) break;` head) |
| `GuardMediump` | `nir_fs.rs:select_highp` (`:9183-9185`) | `guarded16` with a lone fp16 `Fmul` (deadloop_* pins; not a dEQP shape) |
| `LoopForm` | `nir_fs.rs:walk_passes` (`:3540-3542`) | pass 33 (`MAX_UNROLL_ITERATIONS`, `:69`): `fs_101` |
| `LoopForm` | `nir_fs.rs:walk_loop` (`:3673`, `:3682-3684`) | a body with no instruction; an unrolled body over `LOOP_UNROLL_LIMIT * MAX_UNROLL_ITERATIONS` (`:70`) |
| `LoopForm` | `nir_fs.rs:walk_guard` (`:3576-3578`) | a second run-time guard in one unrolled loop |
| `LoopForm` | `nir_fs.rs:walk_if` (`:3293`) | `exit` and the sides' flows in no accepted combination |
| `LoopForm` | `nir_fs.rs:walk_kept_loop` (`:3922, 3929, 3967, 3998, 4001, 4008, 4044, 4050, 4062`) | the kept loop's own forms (section 1 of this note lists them) |
| `LoopForm` | `nir_fs.rs:walk_nodes` (`:4191, 4196, 4205`) | `if (c) { step; continue; }` not at the body's top, or a then side larger than the step |
| `ControlFlow` | `nir_fs.rs:walk_if16` (`:3433`) | a mediump `if` whose side ends in `break`/`continue` (`fs_dowhile_if`'s pin) |
| `ControlFlow` | `nir_fs.rs:walk_nodes` (`:4222`) | a bare `continue` or any node with no arm |

Which refusal each reference pins today is `REFUSALS` in
`tests/mali_deqp_loops.rs:180-207`. Four differ from section 2's "now"
column; the map follows the test: `fs_uncond_break` and `fs_idx_dyn` are
`LoopMediump` (`kept_loop` says kept for both; Mesa unrolls the first and
deletes the second's loop), `fs_inf_break` is `GuardMediump`,
`fs_dowhile_if` is `ControlFlow` (`:3433`), `fs_101` is `LoopForm`, and
`vs_vector` is `UniformType` first (the unitype family's).

The decision is `kept_loop` (`nir_fs.rs:3723-3733`): the body's first
non-instruction node must be `if (c) {} else break;` with `c` not foldable
(`ir_folds`, `:3737`). The vertex walk has no such decision, no loop
`Step` and no block in `select.rs`.

### 8.2 The split: the study's, with two changes

Step 1 is (a) the vertex unroll and (b) the mediump fragment kept loop;
step 2 is the kept vertex loop, the F2 forms and F10. Reason: (a) and (b)
share nothing but the unroll limits (vertex walk against fragment walk), so
lower-loops-1 may be two commits; the kept vertex loop (2a) needs (b)'s
16-bit phi nodes and rows and a vertex block emitter, so it comes after.
The references (section 7) force two changes to section 5:

1. **Step 2b is smaller.** `fs_cond_break` and `fs_mixed` have no second
   exit edge and `fs_cond_continue`/`fs_double_continue` no branch: the
   `break` is OR-ed into the loop's one exit mask and a `continue` is
   `MUX.v2i16` selects. So 2b is a fold of the guard into the mask in
   `walk_if`/`walk_nodes`, not a new edge with phi copies.
2. **`fs_uncond_break`, `fs_dowhile_if`, `fs_select`, `fs_101` stay in
   step 1** (decision fixes in `kept_loop`/`walk_passes`), as the test's
   `ignore` strings already say. `fs_inf_break` is also a `kept_loop`
   decision but its words need 2b's guard fold, so it stays in step 2.

### 8.3 Step 1 (lower-loops-1)

**(a) Vertex unroll, `nir.rs:vs_steps`** (F9, F11; Fail 104, NS 76).

*Raise sites:* `nir.rs:2770`; `:2745` stays (a loop in an `if` side).

*Mesa's unroll to reproduce* (section 3.1): `simple_unroll` for an exact
trip count of at most 32 passes with `instr_cost * trip <= 32 * 26`
(`nir_opt_loop_unroll.c:899-931`); the do-while's test after the body; the
`for` step before each `continue`. The fragment twin is
`walk_loop`/`walk_passes` (`nir_fs.rs:3660, 3530`); share its limits
(`:69-70`, `unroll_cost :3699`) by making them `pub(super)`.

*Change:* replace the arm at `nir.rs:2770` by a pass walker: per pass push
the body's `Step::Instr`s with the counter folded by `fold_int`
(`nir.rs:816`: `iadd`, `ilt`, `ieq` of constants), take the head test on its
folded side, end the chain on a `break`, and refuse `LoopVertex` past 32
passes or the cost bound (those are step 2's kept loops). The main work: the
vertex `build` is one straight list keyed by IR value id, so a body's
second copy needs a per-pass rebinding of ids to `Val`s (the fragment
walk's `vals`/`locals` do it); do it in the arena `scratch`. The mediump
`yzwx` passes fold into the store order (`vs_for_const`: 11 words, no
fp16), so no new emit row. `vs_idx_static` (30 words) uses the existing
`V2F32_TO_V2F16`/`F16_TO_F32` forms (`nir.rs:2273-2277`) and constant
subscripts of a local array: if the vertex walk does not fold an
`IndexedLocal` with a constant index, that is lower-uniarray's, and
`vs_idx_static` stays ignored with its pin moved.

**(b) Fragment mediump kept loop** (F1, F3, F4, F6, F7, the F5 do-while;
`nir_fs.rs`, `mod.rs`).

*Raise sites:* `nir_fs.rs:3914-3916`, `:3540-3542` (`fs_101`), `:3433`
(`fs_dowhile_if`), and `kept_loop`'s answer (`fs_uncond_break`).

*Changes:*
1. `kept_loop` (`:3723`): (i) a loop whose last node is `Break` returns
   false (`wrapper_unroll`; `walk_passes` then unrolls it); (ii) a
   constant trip count past `MAX_UNROLL_ITERATIONS`, or a cost past the
   limit, returns true (`check_unrolling_restrictions`; today the head
   test folds at pass 1 and the loop is unrolled to a `LoopForm`); this
   needs a small pre-walk of the counter (constant start, step, bound),
   restricted to the `for (int i = 0; i < N; i++)` forms; `loop_cap_limit`
   (`:3884`) already reads the 101; (iii) a do-while whose tail test does
   not fold is kept whatever the body holds: look at the last control node
   as well as the first (the tail test, `lower.rs:2287-2298`).
2. `walk_kept_loop` `:3914`: lift the `!prec.highp` return. Header phis
   already take `bits: 16` for a mediump float local (`:3966` region); two
   lanes of one local become one 16-bit two-lane `Phi` (Mesa's two `vec2`
   phis for `res`), four scalars only when a lane is written alone,
   through `vectorize16` (`:8796`).
3. `mod.rs:1461`: add a kept-loop predicate (a `kept16` sibling over
   `f.nir.nloops`) to the `select_highp` route, with the `fs16` layout
   (`:1465-1470`).
4. `branch_forms` and `ladder_forms` (`:8615, 8716`): accept a 16-bit
   two-lane `Phi`, a `Vec2` (`MKVEC.v2i16`) and the 32-bit counter's
   `Icmp`/`Iadd`.
5. Emit rows: `(Phi, 16, 2)` copy as `MOV.i32`, `(Phi, 16, 1)` as
   `U16_TO_U32`, `(Vec2, 16, 2)` of two halves as `MKVEC.v2i16`, and the
   do-while's third `MKVEC.v2i16` after the loop (`fs_dowhile_uni`,
   `fs_101`). Blocks, jumps and `end_block` (`:9987`) are unchanged.
6. `fs_float_uni`: the fp16 counter phi (`FADD.v2f16`, `FCMP_OR.v2f16`)
   and the cap's OR as a separate `LSHIFT_OR` (the cap code at `:3998-4010`
   uses `ior` of an int compare).
7. `fs_select`: `CSEL.u32.eq` of the bool uniform's word needs the
   bool-uniform read in `Fs::int_lane` (`:1564`): study-intval's. Take it
   from lower-intval-1 if merged, else add it here and tell that item.
8. `fs_dowhile_if`: after 1(iii) the body's `if` reaches `walk_if16` with
   no side ending in a jump, so `:3433` does not fire: `MUX.v2i16`
   selects, 22 words.
9. The hidden pass counter stays (`mali-loop-board-failures.md`);
   kept-loop references compare in `src/shaderc/online/mali/tests.rs` with
   `CapEdit`, as section 5 says.

*Tests step 1 un-ignores*, in order, in `tests/mali_deqp_loops.rs` (each:
its `#[ignore]` and its `REFUSALS` entry go; a kept-loop one asserts a
compile there and is byte-compared in `tests.rs`):
`vs_for_const`, `vs_while_const`, `vs_dowhile_const`, `vs_nested_const`,
`vs_idx_static`, then `fs_uncond_break`, `fs_for_uni`, `fs_while_uni`,
`fs_dowhile_uni`, `fs_101`, `fs_float_uni`, `fs_dowhile_if`, `fs_select`.

*glmark2 verdicts that may move* (`tests/fixtures/mali_glmark2_verdicts.txt`
`:92-97`): row 30 and 31 `vs 3cb0` (the constant-bound vertex loop, the
`gm2_intloop_const_vs` shape, unrolled by (a)) go to `ok` or the next
refusal; row 32 `vs 3cb0` (int-uniform bound) stays; row 32 `fs 3678` (the
mediump kept loop, (b)) goes to `ok`; rows 30 and 31 `fs e016`/`c7ea` are
the gl_FragCoord refusal of another family and stay. In
`tools/mali/corpus-verdicts.txt`: `tools/shaders/es100/loop_uniform.frag`,
`loop_nested.frag`, `loop_nested.while.frag`, `loop_nested.dowhile.frag`,
`discard_dynloop.frag`, `ret_dynloop_never.frag` leave `LoopMediump`
(`ret_dynloop_never` to the return flag's refusal); `dynidx_loop.frag`
waits for step 2c; `loop_nested.vert`, `loop_uniform.dowhile.vert` and
`contloops.frag` stay. Regenerate with `corpus-verdicts.sh --write` and
check each moved row against its reference where one exists.

*Risks:*
- (a)'s per-pass rebinding changes `build`, not a leaf; the position
  program must stay the 24-word pass-through in every reference.
- 1(i)-(iii) change decisions highp loops already take byte-equal
  (`intloop_*`, `divnest`, `loop_exit`, `builtin_98`): gate them so those
  references and the `ok` corpus rows pass; run those tests, not only the
  new file.
- 1(ii) needs a counter analysis that agrees with `nir_loop_analyze`;
  refuse (leave the old `LoopForm`) any form but the plain counter.
- `fs_select` depends on intval; do not duplicate the bool read.
- The cap counter is in no reference.

### 8.4 Step 2 (lower-loops-2)

Prerequisites: step 1 (a) and (b); lower-unitype for `vs_vector`'s `ivec4`
and the int uniform by value (`nir.rs:3050-3052`: a `GL_INT` uniform that
is not `index_only` is `UniformType`, the first refusal after a vertex
loop); lower-intval for the `ieq` of two run-time ints.

**(a) Kept vertex loop** (F1-F3, F7, F8; NS 392). *Raise sites:*
`nir.rs:2770`; `:2745` stays; `nir.rs:3050-3052` after it.
*Change*, `mali-vs-loops.md` section 3 in its order: (1) the int uniform
by value for a loop compare; (2) fp16 vector phis and `MKVEC.v2i16` in
the vertex nodes (step 1(b)'s rows, shared); (3) `Step::LoopBegin`/`LoopEnd`
in `vs_steps` filling a `LoopRegion` (`nir.rs:560-570`), header phis in
`registration_order`; (4) blocks, branch and successors in `select.rs`
(`bir::Prog::new_block`, `add_successor`, `bir.rs:1322, 1332`): the loop
is in the varying program (`vs_for_uni`: 24 words, 1 loop) and the
position program stays the 24-word pass-through; a loop feeding
`gl_Position` is in the position program (`gm2_cf_loop_uni_vs`, 51 words),
which is the reference for that case, not a new one; (5) the pass counter;
(6) the do-while rotation (`vs_dowhile_uni`).

**(b) Fragment `break`/`continue` forms** (F2, F5). *Raise sites:*
`nir_fs.rs:3410-3412`, `:4191-4205`, `:3293`. *Change* (section 7): fold a
`break` behind a run-time compare into the loop's one exit mask
(`ICMP_OR.u32.eq.m1` of counter and uniform, `LSHIFT_OR` with the bound's
mask: `fs_cond_break`, `fs_mixed`) through `Fs::int_lane`'s `ieq` of two
run-time ints (`:1564`); a `continue` is four `MUX.v2i16` on the compare
(`fs_cond_continue`); `fs_double_continue` has the same words, so the
second `continue` joins the same selects instead of the not-at-top
refusal (`:4185-4191`): move the rest of the body into the else side as
`nir_lower_continue_constructs` does, then flatten by `walk_if16`'s select
path. `fs_inf_break`: `kept_loop` also accepts the head `if (c) break;`
(then side `break`, else empty).

**(c) F10** (FS 85, VS 112 NS). Two shapes (section 7). *Arrays*
(`fs_idx_dyn`, `vs_idx_dyn`): no loop and no read of `ui_four`: a
`partial_unroll` transcription: a third answer of `kept_loop`, "unroll by
the guess" (the subscripted array's length), `walk_loop` and the vertex
pass walker take the guessed count and drop the guards, then the static
subscripts of study-uniarray. The fragment raise is `:3914`, because
`kept_loop` says kept today. *Vectors* (`fs_vec_dyn`): F1's kept loop with
an `IndexedLocal` ladder inside: `stored_lanes` (`:3842-3880`) accepts an
indexed store whose index is the counter; the ladder is `ICMP_OR.u32.eq`
into `MUX.v2i16` (write) and `ICMP_OR.s32.lt` (read).

*Tests step 2 un-ignores*, in order: `fs_idx_dyn`, `vs_idx_dyn` (no block
code), `fs_cond_break`, `fs_mixed`, `fs_cond_continue`,
`fs_double_continue`, `fs_inf_break`, `fs_vec_dyn`, then the kept vertex
loops `vs_for_uni`, `vs_dowhile_uni`, `vs_float_uni`, `vs_select`,
`vs_vector` (last: also needs unitype's `ivec4`; its pin is `UniformType`
until that item moves).

*glmark2 verdicts that may move:* rows 30-32 `vs 3cb0` (a kept vertex
loop; row 32, and rows 30/31 if their bound is not constant). #367 owns
that shape: whoever lands first moves the rows in
`tests/fixtures/mali_glmark2_verdicts.txt` and the other rebases.
`corpus-verdicts.txt`: `loop_nested.vert`, `loop_uniform.dowhile.vert`,
`contloops.frag`, `dynidx_loop.frag`, `discard_dynloop.frag`.

*Risks:* the vertex block emitter is the one large new piece (no highp
precedent in `select.rs`), and #367's vertex loop item adds the same one
(section 5, "Order against #367"); the position-program loop has one
reference (`gm2_cf_loop_uni_vs`) and no dEQP shape; (b) must keep
`contloops`' words (`MUX.v2i16` at 40-55) and `divnest`'s
`if (c) { step; continue; }` branch form byte-equal; (c)'s loop-free
expectation is drawn for `tmp_array` only, the matrix, uniform-array and
varying-array cases are inferred; the cap is a deliberate deviation in no
reference.

## 9. At 9ac31f30: remaining shapes and lowering steps

Plan #387 item loops-ref. A study plus references: nothing under `src/`
changed, no existing reference or test was edited. It adds
`tools/mali/testdata/deqp_loops2_*` (127 references, section 9.7),
`tests/mali_deqp_loops2.rs` (pins each one's refusal) and this section. Every
number below that comes from the host measure is **HOST-ONLY**: it is a
compile or link outcome of `deqp-gles2` against the Mali back end on the
host, never a render verdict (`tools/mali/deqp-host-measure.sh` header).

### 9.1 Measure

```sh
DEQP_WORK=$CONNECTED_DEPS_DIR/VK-GL-CTS tools/mali/deqp-host-measure.sh --out <dir>
```

CTS `93bca01` (`tools/cts/deps.toml`) with the yakogl target; the three
target files were written by hand with the contents of
`tools/cts/cts-yakogl-target.patch` (`git apply` was not available in the
session). The script takes the cases the board slices
`tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608` refused at compile
(10,597), runs them all (its `--filter` is a case-name substring, so the loop
texts cannot be selected before the run) and reads each case's first refusal
from the `.qpa`. The cases below are those whose **first host refusal is a
loop text**; the shaders of each case were taken from the same `.qpa`.

| text (`MaliError`) | at 37305608 (board slices, section 0) | at 9ac31f30 (host) |
|---|---:|---:|
| `LoopVertex` "a loop in a vertex shader" | 497 (392 NS + 105 Fail) | **241** (212 NS + 29 Fail) |
| `LoopMediump` "a loop Mesa keeps (a run-time bound) in a mediump fragment shader ..." | 268 | **281** |
| `GuardMediump` "a break or continue behind a run-time condition ..." | 6 | **12** |
| `LoopForm` "a loop past Mesa's unroll limits ..." | 3 | **3** |
| total | 774 | **537** (507 NS + 30 Fail) |

Of the 774 cases the board's loop texts blocked, 507 still stop at a loop
text; the rest moved on: 98 to `UniformType` (the unitype family), 79 to
`constant operation on ... not a finite normal float` (constop), 60 compile
with no refusal at all, 9 to varylink, 9 to the constant fold, 8 to
uniarray, 4 to intval. 30 cases that other texts blocked now first stop at a
loop (21 intval, 5 varylink, 4 locals). 011bade0 (glmark2's vertex loops) is
the first vertex loops that compile (`nir.rs` `vs_kept_begin`: one kept loop
with fp32 carried lanes and a head test on an `int` uniform, plus the counted
unroll); most of the vertex cases that left the count did so for the texts
that come before the loop in the walk (the unitype, constop and other rows
above), not because a loop shape was lowered.

**Loops hidden behind another text (HOST-ONLY, not in the 537).** The vertex
cases `shaders.loops.*_{uniform,dynamic}_iterations.*_vertex` with an `int`
counter or bound (97 cases: `basic_*_int`, `single_statement`,
`sequence`, `select_iteration_count`, `101_iterations`, `vector_counter`, ...)
stop at the unitype text (`a vertex shader uniform that is not highp (Mesa's
fp16 uniform) ...` / `a uniform other than a float, ...`) before the walk
reaches the loop; another 4 stop at control flow (`do_while ... conditional_body_fragment`)
and 3 at mixprec. The same holds for 216 `shaders.indexing.*loop*` cases
(constop 82, unitype 38, FragColor lane 24, arithmetic 20, varylink 10,
intval 10, uniarray 8, mixprec 2, and 22 that compile). When unitype lands,
the first group falls onto `LoopVertex` (mostly step S8 of section 9.5): the
loop count of that part is **at least 97 more cases**, so section 9.5's
vertex steps are larger than their rows say.

### 9.2 The shapes and their counts (HOST-ONLY)

One row per (stage, shape); the three bound columns count cases by the
bound kind of the dEQP generator (`for_constant_iterations`,
`..._uniform_...`, `..._dynamic_...`; for the indexing and other groups the
count is in the "uniform" column, the bound being `ui_four` or the group's
own). "Mesa keeps / no loop" is read from the shaderdb line of Mesa's
program for **every** case of the row (all 537 were drawn; the 5 `struct.uniform` ones only as far as the compile, section 9.6): the
number of cases whose Mesa program has a loop (`N loops` in any of the
position, varying and fragment programs) and the number that Mesa unrolls
or deletes completely. Shape bodies of L1: `basic_{highp,mediump,lowp}_{int,float}`,
`empty_body`, `single_statement`, `compound_statement`, `sequence_statement`,
`pre_increment`, `post_increment`, `function_call_{return,inout}`,
`vector_counter`, `sequence`, `only_continue`, `unconditional_continue`,
`single_iteration`, `no_iterations`. L2: `nested`, `nested_sequence`,
`nested_tricky_dataflow_{1,2}`. L4: `conditional_{break,continue}`,
`double_continue`, `mixed_break_continue`, `infinite_with_conditional_break`,
`infinite_with_unconditional_break_{first,last}`, `unconditional_break`.

| stage | shape | const | uniform | dynamic | cases | Mesa keeps / no loop | NS / Fail | text |
|---|---|---:|---:|---:|---:|---|---|---|
| fs | L1 one counted loop, plain body (L1, bodies below) |  | 59 | 59 | 118 | 106 / 12 | 118 / 0 | LoopMediump 118 |
| fs | L2 nested or sequenced loops (L2) |  | 12 | 12 | 24 | 24 / 0 | 24 / 0 | LoopMediump 24 |
| fs | L3 `if` in the body (L3) |  | 2 | 2 | 4 | 4 / 0 | 4 / 0 | LoopMediump 4 |
| fs | L4 `break`/`continue` (L4) |  | 16 | 16 | 32 | 28 / 4 | 32 / 0 | LoopMediump 20, GuardMediump 12 |
| fs | L5 bound a select of a bool uniform (L5) | 3 | 3 | 3 | 9 | 9 / 0 | 9 / 0 | LoopMediump 9 |
| fs | L6 101 iterations (L6) | 3 | 3 | 3 | 9 | 9 / 0 | 8 / 1 | LoopMediump 6, LoopForm 3 |
| fs | idx indexing.matrix_subscript |  | 18 |  | 18 | 0 / 18 | 18 / 0 | LoopMediump 18 |
| fs | idx indexing.tmp_array |  | 28 |  | 28 | 4 / 24 | 28 / 0 | LoopMediump 28 |
| fs | idx indexing.uniform_array |  | 4 |  | 4 | 0 / 4 | 4 / 0 | LoopMediump 4 |
| fs | idx indexing.varying_array |  | 4 |  | 4 | 0 / 4 | 4 / 0 | LoopMediump 4 |
| fs | idx indexing.vector_subscript |  | 30 |  | 30 | 30 / 0 | 30 / 0 | LoopMediump 30 |
| fs | other discard |  | 5 |  | 5 | 4 / 1 | 5 / 0 | LoopMediump 5 |
| fs | other return |  | 3 |  | 3 | 2 / 1 | 3 / 0 | LoopMediump 3 |
| fs | other scoping |  | 2 |  | 2 | 0 / 2 | 2 / 0 | LoopMediump 2 |
| fs | other struct |  | 6 |  | 6 | 4 / 2 | 6 / 0 | LoopMediump 6 |
| vs | L1 one counted loop, plain body (L1, bodies below) | 6 | 15 | 15 | 36 | 24 / 12 | 34 / 2 | LoopVertex 36 |
| vs | L2 nested or sequenced loops (L2) | 12 | 12 | 12 | 36 | 24 / 12 | 32 / 4 | LoopVertex 36 |
| vs | L3 `if` in the body (L3) | 3 | 3 | 3 | 9 | 6 / 3 | 8 / 1 | LoopVertex 9 |
| vs | L4 `break`/`continue` (L4) | 24 | 24 | 24 | 72 | 30 / 42 | 67 / 5 | LoopVertex 72 |
| vs | L6 101 iterations (L6) | 3 |  |  | 3 | 3 / 0 | 2 / 1 | LoopVertex 3 |
| vs | idx indexing.matrix_subscript |  | 12 |  | 12 | 0 / 12 | 12 / 0 | LoopVertex 12 |
| vs | idx indexing.tmp_array |  | 16 |  | 16 | 0 / 16 | 16 / 0 | LoopVertex 16 |
| vs | idx indexing.varying_array |  | 16 |  | 16 | 0 / 16 | 16 / 0 | LoopVertex 16 |
| vs | idx indexing.vector_subscript |  | 18 |  | 18 | 18 / 0 | 18 / 0 | LoopVertex 18 |
| vs | other functions |  | 7 |  | 7 | 2 / 5 | 1 / 6 | LoopVertex 7 |
| vs | other invariance |  | 6 |  | 6 | 0 / 6 | 0 / 6 | LoopVertex 6 |
| vs | other return |  | 7 |  | 7 | 2 / 5 | 4 / 3 | LoopVertex 7 |
| vs | other struct |  | 3 |  | 3 | 1 / 2 | 2 / 1 | LoopVertex 3 |
| | **sum** | | | | **537** | | | |

Sum 537 (fs 296: LoopMediump 281, GuardMediump 12, LoopForm 3; vs 241). Of
the 537, Mesa keeps a loop in 334 and unrolls or deletes the loop in 203
(fs 72, vs 131): **those 203 need no loop code**, only the decision that
the loop is not a loop (section 9.4). The 12 GuardMediump cases are all
`shaders.loops.*` L4: `do_while_{uniform,dynamic}_iterations.{conditional_break,
conditional_continue,double_continue,infinite_with_conditional_break}` (8)
and `{for,while}_{uniform,dynamic}_iterations.infinite_with_conditional_break` (4);
Mesa keeps each as one loop (`fs_do_uni_conditional_break`: 20 instructions,
1 loop). The 3 LoopForm cases are the constant `101_iterations` fragment
loops (Mesa keeps them: 17-18 instructions, 1 loop; `fs_wh_con_101_iterations`).
The 4 fs `L3` rows are `for`/`while` only: the `do_while` form of
`conditional_body` first stops at `ControlFlow` (another family).

### 9.3 Against the earlier references and pins

- **Existing references** (`deqp_loops_*`, section 7, pinned in
  `tests/mali_deqp_loops.rs`): of the 532 drawable cases, 80 have Mesa words
  **identical** to one of them (`vs_dowhile_const` 20, `vs_nested_const` 17,
  `fs_for_uni` 7, `vs_for_const` 6, `fs_float_uni` 4, `fs_dowhile_uni` 3,
  `fs_inf_break` 3, `fs_idx_dyn` 2, `vs_idx_dyn` 2, `vs_idx_static` 2,
  `fs_101` 2, `vs_float_uni` 2, `fs_double_continue` 2, `fs_select` 2,
  `fs_uncond_break` 2, `fs_vec_dyn`, `fs_cond_break`, `fs_mixed`,
  `vs_vector` 1 each). **43 of them (42 vertex, 1 fragment: `discard.dynamic_loop_never`) are
  words of the three vertex constant references, which compile byte-equal today** (`vs_for_const`, `vs_while_const`,
  `vs_dowhile_const` pass in `mali_deqp_loops.rs`): their dEQP bodies
  (`only_continue`, `infinite_with_unconditional_break_first`,
  `nested_sequence`, ..., a `discard_dynamic_loop_never`) differ from the
  references' only in a form the vertex unroll walker refuses (a `break` or
  `continue` as the first or last statement, a nested or sequenced
  loop). The unroll's walker needs those forms, not new words (step S1).
- **Their pins moved since 37305608.** At the tip the reference refusals
  are: `vs_for_uni`, `vs_dowhile_uni`, `vs_select`, `vs_vector` at the
  unitype texts (the hidden loops above), `vs_idx_static` at constop,
  `vs_float_uni`, `vs_idx_dyn`, `vs_nested_const` at `LoopVertex`, the
  fragment ones at `LoopMediump` (except `fs_dowhile_if` `ControlFlow`,
  `fs_inf_break` `GuardMediump`, `fs_101` `LoopForm`).
  `fs_uncond_break` and `fs_idx_dyn` are pinned `LoopMediump` although Mesa
  has no loop for them (the decision, step S4/S2).
- **The new references** (`deqp_loops2_*`, 127, section 9.7) add one
  reference for every (stage, shape, bound, loop type) cell of 9.2 that no
  other reference's words equal: 204 of the 532 cases have words identical
  to a new reference, 80 to an old one, and the remaining 248 sit in a cell
  (stage, body class L1-L6, bound kind, loop type) with a reference and
  differ from it in the body's rotation count, the counter's precision or
  the loop type (`for` against `while` only changes the words
  when the body's `yzwx` count does). Every drawable case is covered one
  way or the other. The 5 `struct.uniform` loop cases are not drawable
  (section 9.6).
- Confirmed by the draws: section 7's inference that **only the
  `tmp_array` and the other array subscripts lose their loop**:
  `matrix_subscript` (fs 18, vs 12), `uniform_array` (4) and `varying_array`
  (fs 4, vs 16) have no loop in Mesa's program either; **`vector_subscript`
  keeps it** (fs 30, vs 18; `vs_idx_vec_w_dynsub` even carries a loop in
  the **position** program, 29 instructions, 1 loop); and
  `tmp_array` with a `const_write_dynamic_loop_read` keeps one loop
  (`fs_idx_tmp_vec4_cd`: 141 instructions, 1 loop; 4 cases).

### 9.4 What Mesa does (from the shaderdb of the references)

Instruction counts are Mesa's `shaderdb`; the position program is the
24-word pass-through in every reference except
`vs_idx_vec_w_dynsub` (29 words, a loop) and the `vs_inv_*` ones (58-71,
the invariant position). "L" is the number of loops of the program.

- **Fragment, a loop Mesa keeps** (224 of the 296): one `loop {}` per source
  loop (`for`, `while` and `do/while` give the same loop, the `do/while`
  with its test moved before the body and the exit value's pair taken
  before the rotation: `fs_do_uni_*` is one instruction longer, 18 against
  17). The carried `mediump vec4` is **two 16-bit `vec2` phis** across the
  back edge: before the loop two `V2F32_TO_V2F16` narrow the varying, each
  pass is `MKVEC.v2i16` (two a pass for `yzwx`), the counter is a 32-bit
  `IADD_IMM` and the exit is `ICMP_OR.s32.ge` plus `BRANCHZ`. A **dynamic**
  bound (`one * ui_n`, `one = int(v_one + 0.5)`) adds
  `LD_VAR_BUF_IMM`, `FADD.f32`, `FADD.v2f16` (+0.5), `F16_TO_S32.rtz`
  before the loop and an `IMUL.i32` of the uniform in the loop **header**
  (Mesa does not hoist it): `fs_for_dyn_basic_mediump_int` is 23 instructions
  against `fs_for_uni_basic_mediump_int`'s 17 (the varying
  program has 20 instructions, not 15, for `a_one`). Counts: plain 17-21, nested 23 (2L),
  `nested_sequence` 32 (3L), a `break`/`continue` 19-26 (the `continue` is
  `MUX.v2i16` selects, the `break` is OR-ed into the exit mask: section 7),
  `select_iteration_count` 19-20 (a `CSEL` bound), `101_iterations` 17-18
  (kept: 101 passes past the limit of 32), a float counter 17-21
  (`FADD.v2f16`/`FCMP_OR.v2f16`), the vector subscript 25-58 (an
  `ICMP_OR.u32.eq` ladder of selects inside the loop).
- **Fragment, no loop** (72): `empty_body` and `only_continue` (the loop is
  deleted: 8 instructions, `fs_for_uni_empty_body`), `unconditional_break`
  (`wrapper_unroll`, one guarded pass), the array subscripts of
  `indexing.*` (partial unroll by the array's length, then the leftover loop
  is dead: `fs_idx_tmp_float_dd` 38, `fs_idx_mat4_dd` 60, `fs_idx_uni_vec4`
  34, `fs_idx_var_vec4_sd` 44 instructions, all 0 loops), `scoping.valid.while_*`
  and two `struct.local` forms.
- **Vertex, a loop Mesa keeps** (110 of the 241): the loop is in the
  **varying** program (the position program stays 24 words): plain
  `for` with a uniform or dynamic bound 24-27 instructions, 1 loop (the
  float counter of `vs_for_uni_basic_lowp_float`), `conditional_body` 28,
  `conditional_break` 26, `conditional_continue` 28, `mixed` 30,
  nested 30-36 (2L), `nested_sequence` 39 (3L), `101_iterations` constant 24
  (kept, `vs_do_con_101_iterations`, `vs_wh_con_101_iterations`), the
  vector-subscript loops 36-61, and four of the `functions`/`return`
  cases (`vs_fn_after_seq` 14, `vs_fn_mixed` 20, `vs_ret_dyn` 27).
- **Vertex, unrolled or deleted** (131): every constant bound of at most 32
  passes (45 cases, `vs_do_con_nested`: 11 instructions, 0 loops, a lane
  permutation folded into the store order, as `vs_for_const`), the
  uniform-bound loops Mesa still unrolls (`unconditional_break` 20: wrapper
  unroll; `only_continue` and `infinite_with_unconditional_break_first`
  delete the loop; 3 plain and 18 L4 cases), the array subscripts (44:
  `vs_idx_tmp_vec4_dd` 73, `vs_idx_mat4_dldl` 58, `vs_idx_var_vec4_dd` 83),
  `invariance.*.loop_{1,2}` (48-63) and the `return` / `functions` cases
  that leave no loop (`vs_ret_static` 4).

### 9.5 Lowering map: eleven one-session steps, ranked by cases

Cases are the 537 above, split by what the lowering must do (the rows sum
to 537). "Forecast" is an upper bound of **cases that stop first at a loop
text and would leave it**, HOST-ONLY: the next refusal of those cases is not
measured (a case's partner shader's is: none of the 537 has a partner
refusal other than the loop's), so a case may stop at the next text; the
board's pass count is for the board rerun. Numbers in parentheses are
Fail cases (dEQP requires those).

| rank | step | cases | dEQP groups | what changes |
|---:|---|---:|---|---|
| 1 | **S3** fragment kept loop, plain body, uniform or dynamic bound (the `for`/`while`/`do_while` of 9.4) | 128 | `loops.{for,while,do_while}_{uniform,dynamic}_iterations.{basic_*,single_statement,...}_fragment` 106, `conditional_body` 4, `select_iteration_count` 9, `101_iterations` 9 | `walk_kept_loop` without the `!prec.highp` return (section 8.3(b)), 16-bit two-lane phis, the `LoopForm` decision for 101, `CSEL` bound, dynamic bound (`int(v_one + 0.5) * ui_n`: intval's) |
| 2 | **S2** array subscripts that Mesa unrolls and deletes | 94 | `indexing.{tmp_array,matrix_subscript,uniform_array,varying_array}.*dynamic_loop*` fs 50, vs 44 | a third `kept_loop` answer: partially unroll by the array's length, drop the guards on `ui_four` (section 8.4(c)); the static subscripts of study-uniarray |
| 3 | **S1** vertex counted loops Mesa unrolls (with jumps, nested, `return`) | 87 (27 Fail) | `loops.*_vertex` 69, `functions.control_flow.*` 5, `invariance.*.loop_*` 6, `return.return_in_static/infinite_loop` 5, `struct.uniform.loop_*` 2 | the vertex unroll walker takes `break`/`continue` as first or last statement, an `if` in the body, nested or sequenced loops, `return`; **42 of these have words equal to references that already compile** (9.3). No kept loop, no vertex block |
| 4 | **S7** fragment vector-subscript loops and the other kept ones in `discard`/`return`/`struct` | 44 | `indexing.vector_subscript.*dynamic_loop*` 30, `indexing.tmp_array.*const_write_dynamic_loop_read` 4, `discard.dynamic_loop_*` 4, `return.return_in_dynamic_loop_*` 2, `struct.{local,uniform}.dynamic_loop_*` 4 | S3's loop plus an `IndexedLocal` ladder (`ICMP_OR.u32.eq` into `MUX.v2i16`: section 8.4(c)) |
| 5 | **S10** vertex kept loop with `break`/`continue`/`if` | 36 | `loops.*_{uniform,dynamic}_iterations.{conditional_*,double_continue,mixed_*,infinite_*,conditional_body}_vertex` | S8 plus the exit mask and `MUX` forms of section 7 |
| 6 | **S8** vertex kept loop, plain body | 32 | `loops.*_{uniform,dynamic}_iterations.basic_*_float_vertex` 24 (the int ones: behind unitype, 97 more), `101_iterations_vertex` 3, `return_in_dynamic_loop_*_vertex` 3, `functions.control_flow` 2 | extend 011bade0's vertex kept loop (`nir.rs` `vs_kept_begin`/`vs_kept_head`/`vs_kept_end`): 16-bit carried lanes (`bits != 32` refuses), the float counter's head test (only `Icmp` is taken), the `LOOP_CAP` limit of a 101 bound, an output stored in the body |
| 7 | **S5** fragment kept loop with `break`/`continue` | 28 | `loops.*_iterations.{conditional_*,double_continue,mixed_*,infinite_*}_fragment` (12 of them GuardMediump) | S3 plus the second-exit forms (section 8.4(b)); `kept_loop`'s head `if (c) break;` |
| 8 | **S6** fragment nested or sequenced kept loops | 24 | `loops.*_iterations.{nested,nested_sequence,nested_tricky_dataflow_1,2}_fragment` | S3 recursive: an outer loop's phis around the inner's |
| 9 | **S9** vertex nested or sequenced kept loops | 24 | `loops.*_{uniform,dynamic}_iterations.{nested,...}_vertex` | S8 with nested regions (`vs.nloops` against `MAX_LOOPS`) |
| 10 | **S4** fragment loops Mesa deletes or unrolls (decision only) | 22 | `loops.*.empty_body`/`only_continue` 12, `unconditional_break` 4, `scoping.valid.while_*` 2, `struct.local` 2, `discard`/`return` 2 | `kept_loop` says unrolled/deleted (section 8.3(b)1(i)); **no new code** |
| 11 | **S11** vertex vector-subscript loops | 18 | `indexing.vector_subscript.*dynamic_loop*_vertex` | S8 plus the index ladder; one program (`vs_idx_vec_w_dynsub`) has the loop in the **position** program |

Sum: 128 + 94 + 87 + 44 + 36 + 32 + 28 + 24 + 24 + 22 + 18 = **537**. The rank is
by cases; the **order of work** differs by dependency:

1. **S4 and S3 together** (150 cases): S4 is the decision, S3 the first
   fragment kept loop. They bring the 16-bit phi rows, the dynamic bound and
   the `LoopForm`/`CSEL` fixes that S5-S7 reuse. 128 of the 150 are NotSupported,
   so the board count moves by the NotSupported to Pass or Fail change.
2. **S1** (87, 28 of them Fail): independent of the rest (no block, no
   kept loop), starts from 43 cases whose words already compile. It is
   the only step with Fail cases of dEQP's required forms, so it comes first
   if the owner's pass count is the Fail one.
3. **S2** (94): the array rule, one decision and the static subscripts.
4. **S5, S6, S7** (96) on S3.
5. **S8** (extending the existing vertex kept loop, 32) then **S9, S10, S11**
   (78): the other vertex kept loops. Unitype's 97 hidden cases come on top of S8-S10 when it lands (section 9.1).

Per step the checks are the references of 9.7: a case's words are
compared with the reference of its cell, then (kept loops) with `CapEdit`.

### 9.6 What cannot be compared byte for byte, and why

- **Every kept loop** (99 of the 127 references: fs 56, vs 43): our walk adds
  the hidden pass counter (`LOOP_CAP`, `mali-loop-board-failures.md`), which
  Mesa's words do not have. `mali_ref::assert_matches` cannot take it; the
  reference is compared with `CapEdit` in `src/shaderc/online/mali/tests.rs`
  and here only the compile is asserted. A deliberate deviation, as for
  `fs_for_uni`.
- **The 5 `struct.uniform` loop cases** (fs `dynamic_loop_{nested_,}struct_array`
  2, vs 3): `tools/mali/gl_ref.py` does not set struct-member uniforms, so
  the draw fails and there is no expectation. Their Mesa shaderdb is known
  (the unrolled `loop_nested_struct_array_vertex`, the kept
  dynamic ones); a reference needs `gl_ref.py` taught the member names.
- **The loop in the position program** (`vs_idx_vec_w_dynsub`, 29
  instructions, 1 loop): `mali_ref` compares both programs, but the kept
  loop there has no precedent but glmark2's `gm2_cf_loop_uni_vs`; compare it
  with `CapEdit` too.
- **The reference is the case's own shaders, drawn once.** `--uniform-values`
  sets each `int` uniform to 16 and each `bool` true where its component is
  even: it does not change the words (section 7), but the FAU words of the
  expectation carry those values, so a test must expect 0 for the uniform
  words as `deqp_loops_*` do. A texture-reading case (`fs_disc_texture`) is
  drawn with gl_ref's texture; its sampler descriptor is the board's own and
  is not a loop question.
- **Not a reference but a measurement**: `deqp-host-measure.sh` has no
  render; a case that compiles gets its verdict from the board rerun.

### 9.7 The references

`tools/mali/testdata/deqp_loops2_<name>.{vert,frag,expect.json}` and the
`.license` sidecars of the two shaders. The shaders are the cases' own, taken
verbatim from the `.qpa` of the host measure (CTS `93bca01`); a name
reads `<stage with the loop>_<type: for|wh|do>_<bound: uni|con|dyn>_<body>`
(`wh` = `while`, `do` = `do_while`, `uni` an int uniform, `con` a literal, `dyn`
`one * ui_n`), or `idx`/`ret`/`fn`/`inv`/`disc`/`scope`/`struct` for the other
groups. Each was drawn with

```sh
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim \
    tools/mali/testdata/deqp_loops2_<name>.vert tools/mali/testdata/deqp_loops2_<name>.frag <out>/deqp_loops2_<name> \
    --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract <out>/deqp_loops2_<name>.trace --draw \
    > tools/mali/testdata/deqp_loops2_<name>.expect.json
```

on Mesa 26.2.1 built by `drm_shim_ref.sh build` (G610). `tests/mali_deqp_loops2.rs`
asserts each pair's exact first refusal (`refusals_pinned`) and that the list
and the directory agree (`references_listed`); no pair compiles today. A
step that lowers one moves its entry and adds the compare. The columns:
Mesa's instruction count of the position / varying / fragment program (the
loops in parentheses), the number of the 532 drawn cases whose Mesa words
equal this reference's (the reference included), and the refusal at the tip.

| reference | dEQP case (`functional.shaders.`) | Mesa instrs (pos / vary / frag) | cases with the same words | refusal |
|---|---|---|---:|---|
| `fs_disc_always` | `discard.dynamic_loop_always` | 24 / 11 / 16 (1L) | 1 | LoopMediump |
| `fs_disc_dyn` | `discard.dynamic_loop_uniform` | 24 / 11 / 17 (1L) | 1 | LoopMediump |
| `fs_disc_dynamic` | `discard.dynamic_loop_dynamic` | 24 / 15 / 20 (1L) | 1 | LoopMediump |
| `fs_disc_texture` | `discard.dynamic_loop_texture` | 24 / 15 / 25 (1L) | 1 | LoopMediump |
| `fs_do_con_select_iteration_count` | `loops.do_while_constant_iterations.select_iteration_count_fragment` | 24 / 15 / 20 (1L) | 1 | LoopMediump |
| `fs_do_dyn_101_iterations` | `loops.do_while_dynamic_iterations.101_iterations_fragment` | 24 / 20 / 23 (1L) | 1 | LoopMediump |
| `fs_do_dyn_basic_highp_float` | `loops.do_while_dynamic_iterations.basic_highp_float_fragment` | 24 / 20 / 21 (1L) | 1 | LoopMediump |
| `fs_do_dyn_conditional_break` | `loops.do_while_dynamic_iterations.conditional_break_fragment` | 24 / 20 / 26 (1L) | 1 | GuardMediump |
| `fs_do_dyn_nested` | `loops.do_while_dynamic_iterations.nested_fragment` | 24 / 20 / 31 (2L) | 1 | LoopMediump |
| `fs_do_dyn_select_iteration_count` | `loops.do_while_dynamic_iterations.select_iteration_count_fragment` | 24 / 20 / 24 (1L) | 1 | LoopMediump |
| `fs_do_uni_101_iterations` | `loops.do_while_uniform_iterations.101_iterations_fragment` | 24 / 15 / 18 (1L) | 1 | LoopMediump |
| `fs_do_uni_basic_highp_float` | `loops.do_while_uniform_iterations.basic_highp_float_fragment` | 24 / 15 / 18 (1L) | 1 | LoopMediump |
| `fs_do_uni_basic_mediump_float` | `loops.do_while_uniform_iterations.basic_mediump_float_fragment` | 24 / 15 / 18 (1L) | 2 | LoopMediump |
| `fs_do_uni_conditional_break` | `loops.do_while_uniform_iterations.conditional_break_fragment` | 24 / 15 / 20 (1L) | 1 | GuardMediump |
| `fs_do_uni_conditional_continue` | `loops.do_while_uniform_iterations.conditional_continue_fragment` | 24 / 15 / 22 (1L) | 2 | GuardMediump |
| `fs_do_uni_nested` | `loops.do_while_uniform_iterations.nested_fragment` | 24 / 15 / 25 (2L) | 1 | LoopMediump |
| `fs_do_uni_select_iteration_count` | `loops.do_while_uniform_iterations.select_iteration_count_fragment` | 24 / 15 / 20 (1L) | 1 | LoopMediump |
| `fs_for_con_select_iteration_count` | `loops.for_constant_iterations.select_iteration_count_fragment` | 24 / 15 / 19 (1L) | 2 | LoopMediump |
| `fs_for_dyn_101_iterations` | `loops.for_dynamic_iterations.101_iterations_fragment` | 24 / 20 / 23 (1L) | 1 | LoopMediump |
| `fs_for_dyn_basic_mediump_float` | `loops.for_dynamic_iterations.basic_mediump_float_fragment` | 24 / 20 / 21 (1L) | 4 | LoopMediump |
| `fs_for_dyn_basic_mediump_int` | `loops.for_dynamic_iterations.basic_mediump_int_fragment` | 24 / 20 / 23 (1L) | 6 | LoopMediump |
| `fs_for_dyn_conditional_body` | `loops.for_dynamic_iterations.conditional_body_fragment` | 24 / 20 / 27 (1L) | 1 | LoopMediump |
| `fs_for_dyn_conditional_break` | `loops.for_dynamic_iterations.conditional_break_fragment` | 24 / 20 / 26 (1L) | 1 | LoopMediump |
| `fs_for_dyn_nested` | `loops.for_dynamic_iterations.nested_fragment` | 24 / 20 / 30 (2L) | 1 | LoopMediump |
| `fs_for_dyn_select_iteration_count` | `loops.for_dynamic_iterations.select_iteration_count_fragment` | 24 / 20 / 24 (1L) | 2 | LoopMediump |
| `fs_for_uni_101_iterations` | `loops.for_uniform_iterations.101_iterations_fragment` | 24 / 15 / 17 (1L) | 1 | LoopMediump |
| `fs_for_uni_basic_highp_float` | `loops.for_uniform_iterations.basic_highp_float_fragment` | 24 / 15 / 17 (1L) | 2 | LoopMediump |
| `fs_for_uni_compound_statement` | `loops.for_uniform_iterations.compound_statement_fragment` | 24 / 15 / 21 (1L) | 2 | LoopMediump |
| `fs_for_uni_conditional_body` | `loops.for_uniform_iterations.conditional_body_fragment` | 24 / 15 / 21 (1L) | 1 | LoopMediump |
| `fs_for_uni_empty_body` | `loops.for_uniform_iterations.empty_body_fragment` | 24 / 15 / 8 | 13 | LoopMediump |
| `fs_for_uni_function_call_inout` | `loops.for_uniform_iterations.function_call_inout_fragment` | 24 / 15 / 17 (1L) | 8 | LoopMediump |
| `fs_for_uni_nested` | `loops.for_uniform_iterations.nested_fragment` | 24 / 15 / 23 (2L) | 1 | LoopMediump |
| `fs_for_uni_nested_sequence` | `loops.for_uniform_iterations.nested_sequence_fragment` | 24 / 15 / 32 (3L) | 1 | LoopMediump |
| `fs_for_uni_nested_tricky_dataflow_1` | `loops.for_uniform_iterations.nested_tricky_dataflow_1_fragment` | 24 / 15 / 30 (2L) | 1 | LoopMediump |
| `fs_for_uni_nested_tricky_dataflow_2` | `loops.for_uniform_iterations.nested_tricky_dataflow_2_fragment` | 24 / 15 / 24 (2L) | 1 | LoopMediump |
| `fs_for_uni_sequence` | `loops.for_uniform_iterations.sequence_fragment` | 24 / 15 / 26 (2L) | 1 | LoopMediump |
| `fs_for_uni_single_iteration` | `loops.for_uniform_iterations.single_iteration_fragment` | 24 / 15 / 17 (1L) | 1 | LoopMediump |
| `fs_idx_mat4_dd` | `indexing.matrix_subscript.mat4_dynamic_loop_write_dynamic_read_fragment` | 24 / 15 / 60 | 1 | LoopMediump |
| `fs_idx_mat4_sd` | `indexing.matrix_subscript.mat4_static_write_dynamic_loop_read_fragment` | 24 / 15 / 43 | 1 | LoopMediump |
| `fs_idx_tmp_float_dd` | `indexing.tmp_array.float_dynamic_loop_write_dynamic_read_fragment` | 24 / 15 / 38 | 1 | LoopMediump |
| `fs_idx_tmp_float_dldl` | `indexing.tmp_array.float_dynamic_loop_write_dynamic_loop_read_fragment` | 24 / 15 / 22 | 2 | LoopMediump |
| `fs_idx_tmp_vec4_cd` | `indexing.tmp_array.vec4_const_write_dynamic_loop_read_fragment` | 24 / 0 / 141 (1L) | 1 | LoopMediump |
| `fs_idx_tmp_vec4_sd` | `indexing.tmp_array.vec4_static_write_dynamic_loop_read_fragment` | 24 / 15 / 35 | 1 | LoopMediump |
| `fs_idx_uni_float` | `indexing.uniform_array.float_dynamic_loop_read_fragment` | 24 / 0 / 30 | 1 | LoopMediump |
| `fs_idx_uni_vec4` | `indexing.uniform_array.vec4_dynamic_loop_read_fragment` | 24 / 0 / 34 | 1 | LoopMediump |
| `fs_idx_var_vec4_sd` | `indexing.varying_array.vec4_static_write_dynamic_loop_read` | 24 / 75 / 44 | 1 | LoopMediump |
| `fs_idx_vec_direct_w` | `indexing.vector_subscript.vec4_direct_write_dynamic_loop_subscript_read_fragment` | 24 / 15 / 25 (1L) | 1 | LoopMediump |
| `fs_idx_vec_static_w` | `indexing.vector_subscript.vec4_static_loop_subscript_write_dynamic_loop_subscript_read_fragment` | 24 / 15 / 28 (1L) | 1 | LoopMediump |
| `fs_idx_vec_w_direct` | `indexing.vector_subscript.vec4_dynamic_loop_subscript_write_direct_read_fragment` | 24 / 15 / 33 (1L) | 1 | LoopMediump |
| `fs_idx_vec_w_dynsub` | `indexing.vector_subscript.vec4_dynamic_loop_subscript_write_dynamic_subscript_read_fragment` | 24 / 15 / 58 (1L) | 1 | LoopMediump |
| `fs_ret_dyn` | `return.return_in_dynamic_loop_dynamic_fragment` | 24 / 15 / 21 (1L) | 1 | LoopMediump |
| `fs_ret_dynamic_loop_never` | `return.return_in_dynamic_loop_never_fragment` | 24 / 15 / 17 (1L) | 1 | LoopMediump |
| `fs_scope_while` | `scoping.valid.while_condition_variable_hides_local_variable_fragment` | 24 / 3 / 14 | 2 | LoopMediump |
| `fs_struct_local` | `struct.local.dynamic_loop_struct_array_fragment` | 24 / 11 / 22 | 1 | LoopMediump |
| `fs_wh_con_101_iterations` | `loops.while_constant_iterations.101_iterations_fragment` | 24 / 15 / 17 (1L) | 1 | LoopForm |
| `fs_wh_dyn_101_iterations` | `loops.while_dynamic_iterations.101_iterations_fragment` | 24 / 20 / 23 (1L) | 1 | LoopMediump |
| `fs_wh_dyn_basic_highp_float` | `loops.while_dynamic_iterations.basic_highp_float_fragment` | 24 / 20 / 20 (1L) | 2 | LoopMediump |
| `fs_wh_dyn_conditional_body` | `loops.while_dynamic_iterations.conditional_body_fragment` | 24 / 20 / 30 (1L) | 1 | LoopMediump |
| `fs_wh_dyn_conditional_break` | `loops.while_dynamic_iterations.conditional_break_fragment` | 24 / 20 / 25 (1L) | 1 | LoopMediump |
| `fs_wh_dyn_nested` | `loops.while_dynamic_iterations.nested_fragment` | 24 / 20 / 30 (2L) | 1 | LoopMediump |
| `fs_wh_uni_101_iterations` | `loops.while_uniform_iterations.101_iterations_fragment` | 24 / 15 / 17 (1L) | 1 | LoopMediump |
| `fs_wh_uni_compound_statement` | `loops.while_uniform_iterations.compound_statement_fragment` | 24 / 15 / 20 (1L) | 2 | LoopMediump |
| `fs_wh_uni_conditional_body` | `loops.while_uniform_iterations.conditional_body_fragment` | 24 / 15 / 24 (1L) | 1 | LoopMediump |
| `fs_wh_uni_conditional_break` | `loops.while_uniform_iterations.conditional_break_fragment` | 24 / 15 / 19 (1L) | 1 | LoopMediump |
| `fs_wh_uni_conditional_continue` | `loops.while_uniform_iterations.conditional_continue_fragment` | 24 / 15 / 24 (1L) | 1 | LoopMediump |
| `fs_wh_uni_double_continue` | `loops.while_uniform_iterations.double_continue_fragment` | 24 / 15 / 24 (1L) | 1 | LoopMediump |
| `fs_wh_uni_nested` | `loops.while_uniform_iterations.nested_fragment` | 24 / 15 / 23 (2L) | 1 | LoopMediump |
| `vs_do_con_101_iterations` | `loops.do_while_constant_iterations.101_iterations_vertex` | 24 / 24 (1L) / 8 | 2 | LoopVertex |
| `vs_do_con_nested` | `loops.do_while_constant_iterations.nested_vertex` | 24 / 11 / 8 | 24 | LoopVertex |
| `vs_do_dyn_basic_highp_float` | `loops.do_while_dynamic_iterations.basic_highp_float_vertex` | 24 / 26 (1L) / 8 | 1 | LoopVertex |
| `vs_do_dyn_conditional_body` | `loops.do_while_dynamic_iterations.conditional_body_vertex` | 24 / 34 (1L) / 8 | 1 | LoopVertex |
| `vs_do_dyn_conditional_break` | `loops.do_while_dynamic_iterations.conditional_break_vertex` | 24 / 32 (1L) / 8 | 1 | LoopVertex |
| `vs_do_dyn_nested_sequence` | `loops.do_while_dynamic_iterations.nested_sequence_vertex` | 24 / 45 (3L) / 8 | 1 | LoopVertex |
| `vs_do_uni_basic_highp_float` | `loops.do_while_uniform_iterations.basic_highp_float_vertex` | 24 / 24 (1L) / 8 | 1 | LoopVertex |
| `vs_do_uni_basic_mediump_float` | `loops.do_while_uniform_iterations.basic_mediump_float_vertex` | 24 / 24 (1L) / 8 | 2 | LoopVertex |
| `vs_do_uni_conditional_body` | `loops.do_while_uniform_iterations.conditional_body_vertex` | 24 / 28 (1L) / 8 | 1 | LoopVertex |
| `vs_do_uni_conditional_break` | `loops.do_while_uniform_iterations.conditional_break_vertex` | 24 / 26 (1L) / 8 | 1 | LoopVertex |
| `vs_do_uni_conditional_continue` | `loops.do_while_uniform_iterations.conditional_continue_vertex` | 24 / 28 (1L) / 8 | 2 | LoopVertex |
| `vs_do_uni_infinite_with_conditional_break` | `loops.do_while_uniform_iterations.infinite_with_conditional_break_vertex` | 24 / 24 (1L) / 8 | 3 | LoopVertex |
| `vs_do_uni_nested` | `loops.do_while_uniform_iterations.nested_vertex` | 24 / 31 (2L) / 8 | 1 | LoopVertex |
| `vs_fn_after_seq` | `functions.control_flow.return_after_loop_sequence_vertex` | 24 / 14 (1L) / 13 | 1 | LoopVertex |
| `vs_fn_mixed` | `functions.control_flow.mixed_return_break_continue_vertex` | 24 / 20 (1L) / 13 | 1 | LoopVertex |
| `vs_fn_return_in_loop` | `functions.control_flow.return_in_loop_vertex` | 24 / 4 / 13 | 5 | LoopVertex |
| `vs_for_dyn_basic_mediump_float` | `loops.for_dynamic_iterations.basic_mediump_float_vertex` | 24 / 27 (1L) / 8 | 4 | LoopVertex |
| `vs_for_dyn_conditional_body` | `loops.for_dynamic_iterations.conditional_body_vertex` | 24 / 33 (1L) / 8 | 1 | LoopVertex |
| `vs_for_dyn_conditional_break` | `loops.for_dynamic_iterations.conditional_break_vertex` | 24 / 32 (1L) / 8 | 1 | LoopVertex |
| `vs_for_dyn_nested` | `loops.for_dynamic_iterations.nested_vertex` | 24 / 36 (2L) / 8 | 1 | LoopVertex |
| `vs_for_uni_basic_lowp_float` | `loops.for_uniform_iterations.basic_lowp_float_vertex` | 24 / 24 (1L) / 8 | 4 | LoopVertex |
| `vs_for_uni_conditional_body` | `loops.for_uniform_iterations.conditional_body_vertex` | 24 / 28 (1L) / 8 | 1 | LoopVertex |
| `vs_for_uni_conditional_break` | `loops.for_uniform_iterations.conditional_break_vertex` | 24 / 26 (1L) / 8 | 1 | LoopVertex |
| `vs_for_uni_conditional_continue` | `loops.for_uniform_iterations.conditional_continue_vertex` | 24 / 28 (1L) / 8 | 2 | LoopVertex |
| `vs_for_uni_mixed_break_continue` | `loops.for_uniform_iterations.mixed_break_continue_vertex` | 24 / 30 (1L) / 8 | 1 | LoopVertex |
| `vs_for_uni_nested` | `loops.for_uniform_iterations.nested_vertex` | 24 / 30 (2L) / 8 | 1 | LoopVertex |
| `vs_for_uni_nested_sequence` | `loops.for_uniform_iterations.nested_sequence_vertex` | 24 / 39 (3L) / 8 | 1 | LoopVertex |
| `vs_for_uni_nested_tricky_dataflow_1` | `loops.for_uniform_iterations.nested_tricky_dataflow_1_vertex` | 24 / 36 (2L) / 8 | 1 | LoopVertex |
| `vs_for_uni_nested_tricky_dataflow_2` | `loops.for_uniform_iterations.nested_tricky_dataflow_2_vertex` | 24 / 31 (2L) / 8 | 1 | LoopVertex |
| `vs_for_uni_unconditional_break` | `loops.for_uniform_iterations.unconditional_break_vertex` | 24 / 20 / 8 | 2 | LoopVertex |
| `vs_idx_mat4_dldl` | `indexing.matrix_subscript.mat4_dynamic_loop_write_dynamic_loop_read_vertex` | 24 / 58 / 7 | 1 | LoopVertex |
| `vs_idx_tmp_vec4_dd` | `indexing.tmp_array.vec4_dynamic_loop_write_dynamic_read_vertex` | 24 / 73 / 7 | 1 | LoopVertex |
| `vs_idx_tmp_vec4_dldl` | `indexing.tmp_array.vec4_dynamic_loop_write_dynamic_loop_read_vertex` | 24 / 50 / 7 | 1 | LoopVertex |
| `vs_idx_var_vec4_dd` | `indexing.varying_array.vec4_dynamic_loop_write_dynamic_read` | 24 / 83 / 114 | 1 | LoopVertex |
| `vs_idx_var_vec4_dldl` | `indexing.varying_array.vec4_dynamic_loop_write_dynamic_loop_read` | 24 / 83 / 44 | 1 | LoopVertex |
| `vs_idx_vec_dldl` | `indexing.vector_subscript.vec4_dynamic_loop_subscript_write_dynamic_loop_subscript_read_vertex` | 24 / 46 (2L) / 8 | 1 | LoopVertex |
| `vs_idx_vec_w_direct` | `indexing.vector_subscript.vec4_dynamic_loop_subscript_write_direct_read_vertex` | 24 / 36 (1L) / 8 | 1 | LoopVertex |
| `vs_idx_vec_w_dynsub` | `indexing.vector_subscript.vec4_dynamic_loop_subscript_write_dynamic_subscript_read_vertex` | 29 (1L) / 61 (1L) / 8 | 1 | LoopVertex |
| `vs_inv_highp_1` | `invariance.highp.loop_1` | 70 / 63 / 15 | 1 | LoopVertex |
| `vs_inv_lowp_loop_1` | `invariance.lowp.loop_1` | 43 / 30 / 15 | 1 | LoopVertex |
| `vs_inv_mediump_1` | `invariance.mediump.loop_1` | 58 / 48 / 15 | 1 | LoopVertex |
| `vs_inv_mediump_2` | `invariance.mediump.loop_2` | 71 / 52 / 15 | 1 | LoopVertex |
| `vs_ret_dyn` | `return.return_in_dynamic_loop_dynamic_vertex` | 24 / 27 (1L) / 8 | 1 | LoopVertex |
| `vs_ret_dynamic_loop_never` | `return.return_in_dynamic_loop_never_vertex` | 24 / 24 (1L) / 8 | 1 | LoopVertex |
| `vs_ret_infinite` | `return.return_in_infinite_loop_vertex` | 24 / 11 / 8 | 20 | LoopVertex |
| `vs_ret_static` | `functions.control_flow.return_in_nested_loop_vertex` | 24 / 4 / 13 | 5 | LoopVertex |
| `vs_ret_static_dyn` | `return.return_in_static_loop_dynamic_vertex` | 24 / 15 / 8 | 1 | LoopVertex |
| `vs_ret_static_loop_always` | `return.return_in_static_loop_always_vertex` | 24 / 11 / 8 | 2 | LoopVertex |
| `vs_ret_static_loop_never` | `return.return_in_static_loop_never_vertex` | 24 / 11 / 8 | 1 | LoopVertex |
| `vs_wh_con_101_iterations` | `loops.while_constant_iterations.101_iterations_vertex` | 24 / 24 (1L) / 8 | 1 | LoopVertex |
| `vs_wh_dyn_basic_highp_float` | `loops.while_dynamic_iterations.basic_highp_float_vertex` | 24 / 26 (1L) / 8 | 2 | LoopVertex |
| `vs_wh_dyn_conditional_body` | `loops.while_dynamic_iterations.conditional_body_vertex` | 24 / 36 (1L) / 8 | 1 | LoopVertex |
| `vs_wh_dyn_conditional_break` | `loops.while_dynamic_iterations.conditional_break_vertex` | 24 / 31 (1L) / 8 | 1 | LoopVertex |
| `vs_wh_dyn_nested_sequence` | `loops.while_dynamic_iterations.nested_sequence_vertex` | 24 / 44 (3L) / 8 | 1 | LoopVertex |
| `vs_wh_uni_conditional_body` | `loops.while_uniform_iterations.conditional_body_vertex` | 24 / 31 (1L) / 8 | 1 | LoopVertex |
| `vs_wh_uni_conditional_break` | `loops.while_uniform_iterations.conditional_break_vertex` | 24 / 26 (1L) / 8 | 1 | LoopVertex |
| `vs_wh_uni_conditional_continue` | `loops.while_uniform_iterations.conditional_continue_vertex` | 24 / 31 (1L) / 8 | 1 | LoopVertex |
| `vs_wh_uni_double_continue` | `loops.while_uniform_iterations.double_continue_vertex` | 24 / 31 (1L) / 8 | 1 | LoopVertex |
| `vs_wh_uni_nested` | `loops.while_uniform_iterations.nested_vertex` | 24 / 30 (2L) / 8 | 1 | LoopVertex |
| `vs_wh_uni_unconditional_continue` | `loops.while_uniform_iterations.unconditional_continue_vertex` | 24 / 24 (1L) / 8 | 1 | LoopVertex |

### 9.8 Caveats

- Counts are **HOST-ONLY**: the host measure compiles and links, it does not
  render. The board's rerun is the authority on pass counts.
- The shape row of a case is read from its name and its first refusal; the
  next refusal after the loop is not measured (it cannot be until the loop
  is lowered), so every "forecast" is an upper bound.
- The 97 hidden vertex loops (9.1) and the 216 `indexing.*loop*` cases behind
  other texts are the other half of the loop mass; they are counted when their
  first text lifts, and are not in the 537 or in the step table.
- The 127 references are one per cell of 9.2; a case that differs from its
  cell's reference (the `while` against the `for` of a body with a
  different `yzwx` count) is found by the case's own words only when a
  step needs it: `deqp-host-measure.sh --filter <case>` followed by the
  commands of 9.7 on the case's `.qpa` shaders makes it.

## 10. The mediump kept loop and the step-2 fragment forms (spike-loops-3, at 7b2246c0)

Plan #377 item spike-loops-3, a read-only spike: a working patch reported to
the planner, not committed. HOST-ONLY: compile and word comparisons, no
board run.

**What compiles, and how it compares.** A kept loop carries the hidden pass
counter (`LOOP_CAP`, `mali-loop-board-failures.md`), so its words are
compared in `src/shaderc/online/mali/tests.rs` with `CapEdit`
(`deqp_loops_<name>_is_mesa_26_2_1_with_the_loop_cap`, every region, fact
and FAU word; an `int` uniform's word gl_ref.py's integer). Eight
references are Mesa 26.2.1's words with the cap's three words (four with a
`LSHIFT_OR`) added and its registers moved, nothing else: step 1b's
`fs_for_uni`, `fs_while_uni`, `fs_dowhile_uni` (the base) and step 2b's
`fs_cond_break`, `fs_cond_continue`, `fs_double_continue`, `fs_mixed`,
`fs_inf_break`. Of the 127 `deqp_loops2_*`, 25 compile, 17 of them Mesa's
words with the cap's in the same sense (`tests/mali_deqp_loops2.rs`
`COMPILED`); the other 8 differ in form only (the `while (i++ < n)`
counter, which Mesa carries as `i + 1`; a loop Mesa deletes, `empty_body`,
`scope_while`; a look-up's kept branch, `disc_texture`).

**Design** (`nir_fs.rs`):

1. Route (`mod.rs`, `loop16`): a mediump shader with a kept loop goes to
   `select_highp` with the fp16 uniform block, its varying lanes an fp16
   operation reads converted first (`convert_inputs16`, the dynamic bound's
   `int(v_one + 0.5)`), without that route's `inputs16` pairing and code
   motion.
2. Phis (`walk_kept_loop`): an aligned pair of a mediump vector's lanes the
   body stores is one 16-bit phi of two lanes (Mesa's two `vec2` phis of
   `res`); its pre-header source the varying's `V2F32_TO_V2F16`
   (`Fs::pair16`), its latch source the pair as it stands (a `vec2`,
   `MKVEC.v2i16`, placed at the local's last store or after the selects it
   reads, the later pair's first; another pair's phi when the body swaps
   them, `compound_statement`). A lone fp16 lane carried (the fp16 float
   counter, glmark2's `gm2_intloop_uniform_fs`) keeps `LoopMediump`.
3. Jumps: a kept loop's `if` whose side ends in `break`/`continue` is a
   region (`walk_if_sides`, `jump16`), as in a highp shader.
4. `if (c) break;` right after the head test (only instructions between) is
   one terminator with it (`merged_break`; `opt_loop_merge_terminators`):
   `ior(c, !head)`, `LSHIFT_OR` into the head's mask. With `if (c0)
   continue; else if (c3) break;` the break's terminator is `iand(c3, !c0)`,
   the head's compare and the cap's words sinking after it (`sink_after`,
   `nir_opt_move` of comparisons), and the continue's selects read `!c0`.
5. `if (c) { step; continue; }` in a mediump body: the rest of the body is
   flattened into `bcsel(c, before, after)` (`MUX.v2i16` on the 32-bit
   mask's low half, `walk_if16`); a bare `continue` ending the body (its
   step's copy before it, the dead step after) is the body's end.
6. `kept_loop`: a lone run-time `if (c) break;` (the body's one control
   node) is the loop's terminator, taken as it is (`fs_inf_break`).
7. The array rule (`guessed_trips`, both walks): a loop with a run-time
   bound whose body writes a local array by its counter (the front end's
   one local per element, stored through a `bcsel` ladder of the counter's
   compares with constants) is unrolled by the array's length with the
   bound dropped (`guess_loop_limit`/`partial_unroll`, the leftover loop
   dead). A loop that reads the elements keeps Mesa's guards on the bound,
   which this rule does not make: it is not unrolled, and a mediump kept
   one is refused. `fs_idx_dyn` and `vs_idx_dyn` are then word for word
   their constant-bound twins (`fs_idx_dyn_is_its_constant_bound_twin`):
   the fragment one differs from Mesa only in the sum's FMA fusion (#367
   lower-fma), the vertex one refuses at the constop text.

Refused on purpose (no reference, or found wrong while measuring): nested or
sequenced mediump kept loops (S6; `loop_nested*.frag`, `fs_vec_dyn`'s
second loop); a discard under a flattened `if` in a kept loop
(`disc_dynamic`'s `if (i > 0) if (..) discard;` lost its outer condition);
an indexed store in a kept loop (`fs_vec_dyn`, F10 vectors).

**Left** (per reference, at this patch): `fs_101` (`LoopForm`: needs the
`kept_loop` past-32 decision and the first-pass rotation), `fs_uncond_break`
(`LoopForm`: `wrapper_unroll` decision), `fs_dowhile_if` (`ControlFlow`:
the do-while's tail test), `fs_select` (the bool uniform's `bcsel`),
`fs_float_uni` (`LoopMediump`: the lone fp16 counter's `U16_TO_U32` copy
and `FCMP_OR.v2f16` head), `fs_vec_dyn` (F10 vector ladders, two loops),
`fs_idx_dyn` (FMA), every vertex kept loop (`vs_float_uni`, `vs_for_uni`,
`vs_dowhile_uni`, `vs_select`, `vs_vector`: 16-bit carried pairs, the float
head test and the stores' `F16_TO_F32` in `nir.rs`/`select.rs`; the last
four behind the unitype texts first), `vs_idx_dyn`/`vs_idx_static`
(constop), `vs_nested_const` (the vertex unroll of nested loops).
