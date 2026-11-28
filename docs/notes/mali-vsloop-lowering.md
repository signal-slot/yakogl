<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall v10): lowering the vertex-loop refusal, shape by shape

**HOST-ONLY.** Plan #387 item `vsloop-refs` (issue #7, ROADMAP stage 5).
This is a design with references. Nothing under `src/` changed. The refusal
is `MaliError::LoopVertex`, "Valhall: a loop in a vertex shader is not
lowered yet" (`src/shaderc/online/mali/mod.rs:511`). In the Verdin iMX95
(Mali-G310) dEQP-GLES2 run of 2026-11-24/25 (build `e63ab28e`, the plan's
"main `a3d8d2de`"; `tools/cts/log/2026-11-2*-imx95-gles2-*-e63ab28e/`), it is
the first refusal of **179 cases: 159 Fail and 20 NotSupported**. This note
orders the work that removes the text and forecasts what each step frees.

Read with:

- [`mali-deqp-vsloop.md`](mali-deqp-vsloop.md) (step 1's eight references);
- [`mali-deqp-vsloop2.md`](mali-deqp-vsloop2.md) (the loops Mesa keeps, the
  196-case table of the NanoPC-T6 run at `8ff01055`);
- [`mali-loops2-lowering.md`](mali-loops2-lowering.md) (steps LP1-LP8 and
  IDX of the `shaders.loops` and `shaders.indexing` groups);
- [`mali-deqp-idxloc.md`](mali-deqp-idxloc.md) (V4, the vector subscript in
  a kept vertex loop);
- [`mali-loops-lowering.md`](mali-loops-lowering.md) (the design steps
  S1-S11 and the loop cap).

This item adds 16 references `tools/mali/testdata/deqp_vsloop_*` (with
`.license` and `.nir.sha` files), the tables `G310_SHAPES` and `SAME_WORDS`
and three tests in `tests/mali_deqp_vsloop.rs`, and this note:

```text
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsloop
```

## 0. Result in brief

- **The 179 G310 lines are the 196 NanoPC-T6 lines at `8ff01055` minus 17.**
  8 are the constant nested `for`/`while` loops, which pass on the G310. 8
  are the dynamic nested `for`/`while` loops, which stop at the int-uniform
  refusal on the G310. 1 is `struct.uniform.loop_nested_struct_array_vertex`,
  which stops at the non-finite constant refusal. **At the tip the 8 dynamic
  nested loops stop at LoopVertex again** (host compile of the generated
  `for_dynamic_iterations.nested*_vertex`). So the family a lowering faces is
  **187 cases**: the 179, plus those 8.
- **Mesa leaves no loop in 77 of the 179** (step V1). It keeps a loop in the
  other 102, and in the 8 above.
- **Step V1 is done in plan #402 `lower-vsloop`** (`cbb6b45b`, not landed
  when this was written). On that commit, 65 of the 77 are byte-equal (one of
  them inferred from its sibling). 4 compile but differ. 6 move to the
  constant-product refusal, and 2 stay at LoopVertex.
- **21 of the kept loops already compile at the tip** (step V2: float
  counters, 101 passes). They left LoopVertex after the G310 run. Their words
  are Mesa's plus the deliberate loop cap.
- **What is left is kept-loop walk code**: V3 single exit plus selects (36),
  V4 multiple exits (8), V5 nested loops (24 with the 8), V6 vector subscript
  writes (18), V7 the rest (3). Every step except V7 is a step that
  `mali-loops2-lowering.md` or `mali-deqp-idxloc.md` already names (section 4).

## 1. Method

1. **The case list.** The lines of the G310 slices' `refusals.tsv` whose text
   holds "a loop in a vertex shader":

   ```sh
   cat tools/cts/log/2026-11-2*-imx95-gles2-*-e63ab28e/refusals.tsv |
       grep -F 'a loop in a vertex' | cut -f1 | sort -u      # 179
   ```

   Status from the same slices' `results.tsv`: 159 Fail, 20 NotSupported.
   The NotSupported ones are the 18 `indexing.vector_subscript.*` and
   `functions.control_flow.return_after_loop_sequence_vertex` and
   `struct.uniform.dynamic_loop_nested_struct_array_vertex`.
2. **The sources.** No `.qpa` of a host measure was at hand. VK-GL-CTS
   `93bca01861b0` was fetched sparse (`es2fShaderLoopTests.cpp`,
   `es2fShaderReturnTests.cpp`, `data/gles2/shaders/functions.test` and three
   others) into `$CONNECTED_DEPS_DIR/VK-GL-CTS-src`. A scratch Python port of
   `createSpecialLoopCase` and `createGenericLoopCase` (in `$TMPDIR`, not
   committed) writes the vertex cases. It is **byte-equal to all 106
   committed verbatim loop sources** of the case kinds it writes (every
   `deqp_loops*_vs_*` and `deqp_vsloop*_` reference of those kinds). The
   10 other committed loop sources are kinds the port does not write
   (`sequence`, `function_call_inout`, `post_increment`, ...). The
   `functions.control_flow` and `return` sources are the verbatim
   `deqp_vsloop_fn_nested` and `deqp_loops2_vs_ret_dyn` frames, with the
   case's body. Each `.license` says which.
3. **Host outcome.** A scratch test (not committed) ran `tests/mali_ref`'s
   `compile_pair` on all 148 `shaders.loops.*` cases of the 187. It ran
   **at the tip `1e1380b1`** and **on `cbb6b45b`** (plan #402
   `lower-vsloop`, step V1, in a scratch worktree). Each case's own Mesa words
   were drawn into scratch, and `mali_ref::differences` compared them. The
   other 39 cases (indexing, functions, return, invariance, struct) were
   measured through the committed references that hold their shapes.
4. **References.** Mesa 26.2.1's drm-shim was built in
   `$CONNECTED_DEPS_DIR/mesa-shim` (`tools/mali/drm_shim_ref.sh build`, the
   tarball of `third_party/mesa-26.2.1/UPSTREAM`, sha256 `c47e81bd...`). As a
   self-check, `deqp_vsloop_con_cond_break` was drawn again. Its
   `expect.json` was byte-equal apart from the `trace` name. Each new one was
   drawn with

   ```sh
   tools/mali/deqp_ref.sh <case>.vert <case>.frag tools/mali/testdata/deqp_vsloop_<shape> --instrument shim
   tools/mali/nir_refs.sh deqp_vsloop_<shape>
   ```

   Every draw ended with `glGetError 0x0`. The G310 needs nothing separate
   (`mali-g310-vs-g610.md`).

**Local build note.** At `1e1380b1`, `capi/Cargo.toml` declares
`vivante-st` twice (lines 41 and 85, since `110080eb`). Every cargo command
in the workspace then fails with "duplicate key". The test was run with line
85 dropped in the working tree only. That change is not part of this item.

## 2. The 179 lines by shape

"V" is the step of section 4. "Mesa" is the shaderdb line of the reference
that holds the shape (`N loops` in `vs.varying`; `vs.position` and
`fs.fragment` have none). "cbb6b45" is the outcome on plan #402's step V1.

| V | shape (dEQP `functional.shaders.`) | G310 lines | Mesa | at the tip | on cbb6b45 |
|---|---|---:|---|---|---|
| V1 | `loops.*_constant_iterations.*` with a guard, `break`, `continue`, `infinite_*`, `only_continue`, `unconditional_*`, `conditional_body` (not `101`, not `nested*`) | 37 | unrolled, 11 words | LoopVertex | **37 byte-equal** |
| V1 | `loops.*_{uniform,dynamic}_iterations.{infinite_with_unconditional_break_{first,last},unconditional_break}` | 18 | one pass or `wrapper_unroll` | LoopVertex | 14 byte-equal; 4 `{for,while}_{uniform,dynamic}` `unconditional_break` differ (15/19 words against 20/25: the `wrapper_unroll` select at fp32, where Mesa narrows with `V2F32_TO_V2F16` and selects with `MUX.v2i16`) |
| V1 | `loops.*_{uniform,dynamic}_iterations.only_continue` | 6 | deleted | LoopVertex | **6 byte-equal** |
| V1 | `functions.control_flow.{return_after_break,return_after_continue,return_in_loop,return_in_loop_if,return_in_nested_loop}` | 5 | inlined, unrolled, 4 words (`-in0`) | LoopVertex | 4 byte-equal by reference; `return_after_continue` has no reference (assumed like `return_after_break`) |
| V1 | `return.return_in_{static_loop_always,static_loop_never,infinite_loop}` | 3 | one pass, 11 words | LoopVertex | **3 byte-equal** |
| V1 | `return.return_in_static_loop_dynamic` (`deqp_loops2_vs_ret_static_dyn`), `return.return_in_dynamic_loop_always` (**new** `ret_dynamic_loop_always`) | 2 | one pass, 11 words | LoopVertex | **LoopVertex**: the return behind a run-time condition, and the always-return in a uniform-bound loop |
| V1 | `invariance.{highp,mediump,lowp}.loop_{1,2}` | 6 | unrolled | LoopVertex | constant-product refusal (`ArithConstant`, plan #387 `vsconstprod-design`) |
| V2 | `loops.*_{uniform,dynamic}_iterations.basic_{lowp,mediump,highp}_float` | 18 | **kept**, 1 loop, 24-27 words | **compiles**: Mesa's + cap (4 words); in the two `do`-`while` dynamic forms checked word by word, the two header products also stand in source order | same |
| V2 | `loops.*_constant_iterations.101_iterations` | 3 | **kept**, 1 loop, 24 words | **compiles**: Mesa's + cap (3 words, the `#0x66` count); `while (i++ < 101)` 4 words, counter order | same |
| V3 | `loops.*_{uniform,dynamic}_iterations.{conditional_body,conditional_break,infinite_with_conditional_break,conditional_continue,double_continue,unconditional_continue}` | 36 | **kept**, 1 loop: one exit `BRANCHZ.reconverge`, the guarded statements as 4 `MUX.v2i16` | LoopVertex | LoopVertex |
| V4 | `loops.*_{uniform,dynamic}_iterations.mixed_break_continue` | 6 | **kept**, 1 loop: `for`/`while` merge the `i == 3` break into the exit test; `do`-`while` has a second exit edge | LoopVertex | LoopVertex |
| V4 | `return.return_in_dynamic_loop_{dynamic,never}` | 2 | **kept**, 1 loop: the return as a second exit | LoopVertex | LoopVertex |
| V5 | `loops.*_{uniform,dynamic}_iterations.{nested,nested_sequence,nested_tricky_dataflow_1,nested_tricky_dataflow_2}` | 16 (+8) | **kept**, 2-3 loops | LoopVertex | LoopVertex |
| V6 | `indexing.vector_subscript.vec{2,3,4}_dynamic_loop_subscript_write_*_read` | 18 NS | **kept**, 1 loop: an `ICMP_OR.u32.eq` + `MUX.v2i16` write ladder | LoopVertex | LoopVertex |
| V7 | `functions.control_flow.{mixed_return_break_continue,return_after_loop_sequence}` | 2 (1 NS) | **kept** | LoopVertex | LoopVertex (refused by name: a jump three ifs deep, a loop reading an earlier loop's local) |
| V7 | `struct.uniform.dynamic_loop_nested_struct_array` | 1 NS | **kept**, the inner loop unrolled | LoopVertex | LoopVertex |
| | **total** | **179** | | | |

The "+8" in V5 are the dynamic nested `for`/`while` loops, at UniformType on
the G310 board and at LoopVertex on the host at the tip. The 148
`shaders.loops` cases are measured case by case. The other 39 are measured
through the committed reference of their shape.

## 3. The 16 new references

The committed references held 114 of the 179 cases byte for byte (their
`.license` names the case). The other 65 had only a sibling: the same body
for another loop type or bound kind. The 16 here are the siblings that matter
for the steps: every `do`-`while` form whose `for`/`while` twin alone was
pinned, the two V1 shapes without any reference, and the hidden V5 form.
Names drop the `deqp_vsloop_` prefix. "Same words" means all three programs
are equal to the named reference (`SAME_WORDS`, test `g310_same_words`).

| reference | dEQP case | V | Mesa `vs.varying` | same words as | tip | cbb6b45 |
|---|---|---|---|---|---|---|
| `do_con_conditional_body` | `loops.do_while_constant_iterations.conditional_body_vertex` | V1 | 11 words, 0 loops | `deqp_vsloop_con_cond_break` | LoopVertex | byte-equal |
| `fn_return_in_loop_if` | `functions.control_flow.return_in_loop_if_vertex` | V1 | 4 words, 0 loops | `deqp_vsloop_fn_after_break` | LoopVertex | byte-equal |
| `for_dyn_infinite_with_unconditional_break_last` | `loops.for_dynamic_iterations.infinite_with_unconditional_break_last_vertex` | V1 | 11 words, 0 loops | `deqp_vsloop_uni_inf_break_last` | LoopVertex | byte-equal |
| `ret_dynamic_loop_always` | `return.return_in_dynamic_loop_always_vertex` | V1 | 11 words, 0 loops | `deqp_loops2_vs_ret_static_loop_always` | LoopVertex | **LoopVertex** |
| `for_con_101_iterations` | `loops.for_constant_iterations.101_iterations_vertex` | V2 | 24 words, 1 loop | - | compiles: 27 words (cap) | same |
| `do_dyn_basic_mediump_float` | `loops.do_while_dynamic_iterations.basic_mediump_float_vertex` | V2 | 27 words, 1 loop | - | compiles: 31 words (cap; the two header `FMA.v2f16` in source order) | same |
| `for_dyn_double_continue` | `loops.for_dynamic_iterations.double_continue_vertex` | V3 | 34 words, 1 loop, 4 `MUX` | - | LoopVertex | LoopVertex |
| `do_uni_double_continue` | `loops.do_while_uniform_iterations.double_continue_vertex` | V3 | 28 words, 1 loop, 4 `MUX` | - | LoopVertex | LoopVertex |
| `for_uni_infinite_with_conditional_break` | `loops.for_uniform_iterations.infinite_with_conditional_break_vertex` | V3 | 24 words, 1 loop | `deqp_loops2_vs_do_uni_infinite_with_conditional_break` | LoopVertex | LoopVertex |
| `do_dyn_infinite_with_conditional_break` | `loops.do_while_dynamic_iterations.infinite_with_conditional_break_vertex` | V3 | 29 words, 1 loop | `deqp_vsloop2_wh_dyn_infinite_with_conditional_break` | LoopVertex | LoopVertex |
| `wh_dyn_mixed_break_continue` | `loops.while_dynamic_iterations.mixed_break_continue_vertex` | V4 | 38 words, 1 loop, one exit | - | LoopVertex | LoopVertex |
| `do_uni_mixed_break_continue` | `loops.do_while_uniform_iterations.mixed_break_continue_vertex` | V4 | 37 words, 1 loop, two exits | - | LoopVertex | LoopVertex |
| `do_uni_nested_sequence` | `loops.do_while_uniform_iterations.nested_sequence_vertex` | V5 | 39 words, 3 loops | - | LoopVertex | LoopVertex |
| `wh_uni_nested_tricky_dataflow_1` | `loops.while_uniform_iterations.nested_tricky_dataflow_1_vertex` | V5 | 36 words, 2 loops | - | LoopVertex | LoopVertex |
| `for_dyn_nested_tricky_dataflow_1` | `loops.for_dynamic_iterations.nested_tricky_dataflow_1_vertex` (one of the 8 hidden) | V5 | 42 words, 2 loops | - | LoopVertex | LoopVertex |
| `do_dyn_nested_tricky_dataflow_2` | `loops.do_while_dynamic_iterations.nested_tricky_dataflow_2_vertex` | V5 | 37 words, 2 loops | - | LoopVertex | LoopVertex |

What the words show:

- **The loop type and the bound kind do not change Mesa's program where
  Mesa unrolls.** Six of the 16 are word for word an earlier reference
  (the table). So V1's references cover its 77 lines whatever the loop type.
- **Where Mesa keeps the loop, the type matters.** None of the ten kept
  references equals its `for`/`while` sibling. The `do`-`while` test sits at
  the bottom, after the body's selects and the step. A `do`-`while`
  `mixed_break_continue` has a second exit edge. In `for`/`while`, the break
  is ORed into the head test (as LP2/LP8 of `mali-loops2-lowering.md`
  found for the fragment and vertex twins).
- **Every kept loop has one form** (`mali-deqp-vsloop2.md` 3). The counter
  starts at `IADD_IMM.i32.reconverge rN, 0x0, #0x0`. The mediump `res` is
  carried as two fp16 pairs (`V2F32_TO_V2F16` before, `MKVEC.v2i16` in the
  latch, `F16_TO_F32` after). Each loop ends in `BRANCHZ.eq.reconverge 0x0`
  with a negative offset.
- The dynamic forms read `one = int(a_one + 0.5)` as Mesa's fp16
  `FADD.f32 rX.h0, .., 0x0.neg`, `FADD.v2f16`, `F16_TO_S32.rtz`
  (`mali-loops2-lowering.md` LP6). Roughly half of every kept step's lines
  are dynamic and meet that narrowing.

## 4. The ordered steps

Each step names the cases it takes, the shared design step, and a host
forecast. "Leaves" means the case stops refusing with LoopVertex. "Byte-equal"
is measured against the case's own Mesa words, or its reference's.

### V1: unroll or delete what Mesa does (77 lines): plan #402 `lower-vsloop`

- **Shared with:** loops2 **LP4 + LP5**, design **S1**
  (`mali-loops-lowering.md` 5). It also covers the `functions.control_flow`,
  `return` and `invariance` shapes that loops2 did not count (16 lines).
- **Status:** implemented on `cbb6b45b` (`vs_put_jloop`, `vs_dead_loop`). It
  is in review, not on the tip.
- **Forecast (measured on `cbb6b45b`):** **75 of the 77 leave.** 65 are
  byte-equal: 37 + 14 + 6 loops, 4 + 1 functions (one inferred), 3 return. 4
  compile but differ: the `unconditional_break` wrapper at fp32. **6
  `invariance` cases move to `ArithConstant`**, so they need the constant
  product family too. **2 stay at LoopVertex**:
  `return_in_static_loop_dynamic` and `return_in_dynamic_loop_always`.
- **Left after `cbb6b45b`:**
  - **V1a.** A return behind a run-time condition in a one-pass loop, and an
    always-taken return in a uniform-bound loop. Both are one guarded pass in
    Mesa (`ret_dynamic_loop_always` has `ret_static_loop_always`'s words). The
    rule is: a loop whose body ends every pass in a jump is one pass whatever
    its bound. Then a run-time `if (c) return` becomes the guarded tail
    `cbb6b45b` already makes for `if (c) {..}`.
  - **V1b.** The `wrapper_unroll` pass of the uniform/dynamic
    `unconditional_break` in Mesa's fp16 form: `V2F32_TO_V2F16`, then
    `MUX.v2i16` on the `0 < bound` mask (`deqp_loops2_vs_for_uni_unconditional_break`,
    `tests/mali_deqp_loops2.rs` DIFFERS on `cbb6b45b`). It is the vertex use
    of the mediump pair carrying V3 needs, so it may land with V3.

### V2: kept loops that compile already (21 lines): no LoopVertex work

- **Shared with:** design **S8** (landed in trains 1 and 2;
  `tests/mali_deqp_loops2.rs` `COMPILED`).
- **Forecast:** all 21 left LoopVertex after the G310 run. That is measured
  at the tip: 18 `basic_*_float` and 3 `101_iterations`. Their words are
  Mesa's plus the loop cap (3 words, or 4 after a float test). The cap is a
  deliberate deviation (`docs/notes/mali-loop-board-failures.md`), compared
  by `CapEdit` in `src/shaderc/online/mali/tests.rs`. Two known order defects
  remain. In the dynamic float forms checked word by word
  (`vs_do_dyn_basic_highp_float`, `do_dyn_basic_mediump_float`), the two
  header products stand in source order, and Mesa swaps them. The other
  dynamic forms have the same word counts (Mesa's + 4), but they were not
  aligned word by word. The `while (i++ < 101)` form puts the
  counter step before the compare of the old `i`. They are not LoopVertex
  work. They belong to whichever item takes the `COMPILED` `false` entries.

### V3: a kept loop with one exit and selects (36 lines)

- **Cases:** `{for,while,do_while}_{uniform,dynamic}_iterations.{conditional_body,conditional_break,infinite_with_conditional_break,conditional_continue,double_continue,unconditional_continue}_vertex`.
- **Shared with:** loops2 **LP8** (30 of them) and **LP6**'s trailing
  `continue` (the 6 `unconditional_continue`). Design S10 and S8.
- **Lowering, in order:**
  1. Drop a `continue` that is the last statement of a loop body (LP1/LP6).
     `unconditional_continue` is then the plain kept loop V2 compiles, and
     `double_continue` becomes `conditional_continue`
     (`do_uni_double_continue`, 4 `MUX`).
  2. Make `while (true)` and `do {..} while (true)` with `if (c) break;` the
     `for (;;)` form (LP2 step 1). Mesa's words are equal across the three
     (`SAME_WORDS`: `for_uni_` = `do_uni_infinite_with_conditional_break`,
     `do_dyn_` = `wh_dyn_`).
  3. In the vertex kept walk (`vs_kept_begin`/`vs_kept_end`), take an `if`
     in the body: a `break` becomes part of the single exit test, and a
     `continue` or a guarded body becomes `MUX.v2i16` selects of the carried
     fp16 pairs on the `ICMP_*.m1` mask. This is the fragment walk's S5 mask
     form. For `do`-`while`, the test stays after the selects and the step.
  4. The dynamic forms also need Mesa's fp16 `int(a_one + 0.5)` (LP6).
- **Forecast:** 36 leave LoopVertex when 1-3 land. The 18 uniform forms are
  expected Mesa's + cap. The 18 dynamic forms wait on step 4 for byte
  equality. The next refusal past LoopVertex is not measured (no walk code
  exists to stand in for it).
- **References:** `for_dyn_double_continue`, `do_uni_double_continue`,
  `for_uni_infinite_with_conditional_break`, `do_dyn_infinite_with_conditional_break`
  (new), and loops2's LP8/LP6 and vsloop2's dynamic ones.

### V4: a kept loop with two exits (8 lines)

- **Cases:** `mixed_break_continue` (6) and
  `return.return_in_dynamic_loop_{dynamic,never}` (2).
- **Shared with:** loops2 **LP8**'s `mixed_break_continue` and **LP2** step
  3 (the fragment `do`-`while` second exit).
- **Lowering:** after V3, in `for`/`while`, OR the `i == 3` break mask into
  the head test (`LSHIFT_OR`, one exit: `wh_dyn_mixed_break_continue`). In
  `do`-`while`, emit a second forward exit edge
  (`do_uni_mixed_break_continue`). A `return` in a kept loop is a break plus
  the return value carried out of the loop (`deqp_loops2_vs_ret_dyn`,
  `_dynamic_loop_never`).
- **Forecast:** 8 leave. The 3 dynamic `mixed` forms also wait on V3 step 4.

### V5: nested and sequenced kept loops (16 + 8 = 24 lines)

- **Cases:** `{for,while,do_while}_{uniform,dynamic}_iterations.{nested,nested_sequence,nested_tricky_dataflow_1,nested_tricky_dataflow_2}_vertex`.
  The 8 dynamic `for`/`while` forms are counted at UniformType on the G310,
  but they reach LoopVertex at the tip.
- **Shared with:** loops2 **LP7**, design **S9**.
- **Lowering:** a second `KeptLoop` record in the vertex walk, nested (the
  outer loop's fp16 pair phis around the inner loop's) or after the first.
  The inner counter is re-initialised in the outer body. `vs_kept_begin`
  must accept a carried pair that a kept loop left as fp16. One cap per loop.
- **Forecast:** 24 leave, with 12 dynamic forms on V3 step 4. Not measured
  further.
- **References:** `do_uni_nested_sequence` (3 loops),
  `wh_uni_nested_tricky_dataflow_1`, `for_dyn_nested_tricky_dataflow_1`,
  `do_dyn_nested_tricky_dataflow_2` (new), loops2's LP7 and vsloop2's.

### V6: vector subscript writes in a kept loop (18 lines, all NotSupported)

- **Shared with:** `mali-deqp-idxloc.md` **V4** / **IL-V**, design **S11**.
- **Lowering:** the kept loop of V3 with the write ladder
  (`ICMP_OR.u32.eq` against byte constants into `MUX.v2i16`), and the read
  ladder. See idxloc section 3.
- **Forecast:** 18 leave LoopVertex. They then stop at `ArithConstant`
  (idxloc's measure: all 18), so they need constprod K2-K5 too. NotSupported
  on the board is the case's own status (dynamic loops). It changes only
  when the driver reports the support.

### V7: the rest (3 lines)

- `functions.control_flow.mixed_return_break_continue_vertex` and
  `return_after_loop_sequence_vertex` (`deqp_loops3_vs_fn_{mixed,after_seq}`).
  `cbb6b45b` refuses them on purpose: a jump three ifs deep, and a loop whose
  jump conditions read a local stored by an earlier loop. Mesa keeps a loop
  in both. They need V4 (returns as exits) and V5 (sequenced loops).
- `struct.uniform.dynamic_loop_nested_struct_array_vertex`
  (`deqp_vsloop2_struct_dyn_nested_array`, 110 words): the outer loop kept,
  the inner one unrolled to 3 guarded passes. It needs V3 plus V1's unroll
  inside a kept loop, and the struct uniform values (`STRUCT_VALUES`).

### Order and forecast

| order | step | G310 lines | shares | leaves LoopVertex (host) | byte-equal (host) |
|---:|---|---:|---|---:|---|
| 1 | V1 (`lower-vsloop`, `cbb6b45b`) | 77 | LP4, LP5, S1 | 75 (6 to `ArithConstant`) | 65 (measured, 1 inferred) |
| 2 | V1a + V1b | (2 + 4 of V1) | S1 | 2 | 6 expected |
| - | V2 (landed) | 21 | S8 | 21 (already, at the tip) | 0: Mesa's + cap |
| 3 | V3 | 36 | LP8, LP6, LP2 step 1, S10 | 36 | 18 uniform expected as Mesa's + cap (not measured); 18 dynamic wait on the `a_one` narrowing |
| 4 | V4 | 8 | LP8, LP2 step 3 | 8 | not measured |
| 5 | V5 | 16 (+8) | LP7, S9 | 24 | not measured |
| 6 | V6 | 18 | idxloc V4/IL-V, S11 | 18 | 0 until constprod K2-K5 |
| 7 | V7 | 3 | V4, V5 | 3 | not measured |
| | **total** | **179 (+8)** | | **187** | |

LoopVertex cases on the host, step by step:

| after | of the 179 | with the 8 hidden |
|---|---:|---:|
| the tip (V2 landed) | 158 | 166 |
| step 1 (V1) | 83 | 91 |
| step 2 (V1a) | 81 | 89 |
| steps 3-5 (V3, V4, V5) | 21 | 21 |
| step 6 (V6) | 3 | 3 |
| step 7 (V7) | 0 | 0 |

The 83 left after V1 are V3 36, V4 8, V5 16, V6 18, V7 3 and V1a 2.

The plan's goal counts must-pass cases blocked by a first refusal of 100 or
more. LoopVertex falls below 100 at the tip as soon as V1 lands, because V2
has already taken 21.

## 5. The test

`tests/mali_deqp_vsloop.rs` (plan #402 `ref-vsloop`) gains:

- `G310_SHAPES`: the 16 references, each with its step and its tip outcome.
  An outcome is `Refused(text)`, `RefusedOrMatched(text)` or
  `Compiled { ours, mesa }`. `RefusedOrMatched` marks the three V1 shapes
  that `cbb6b45b` makes byte-equal. The pin takes both outcomes until that
  commit lands. Then the entry becomes a match. That is how the test passes
  in either landing order: checked at the tip, and with this diff applied to
  `cbb6b45b`.
- `g310_shapes_pinned` (runs): each outcome, exactly. A `Compiled` entry
  must differ only in `vs.varying`, by the pinned word counts.
- `g310_same_words` (runs): the six `SAME_WORDS` pairs are one Mesa program.
- `g310_shapes_compare` (`#[ignore]`): the byte compare of all 16.
- `refusals_pinned` counts the new files with the old tables.

A step that moves an entry moves it in the same commit. The kept-loop
references compare through `CapEdit`, as `tests/mali_deqp_loops2.rs`
`COMPILED` does.

## 6. Caveats

- The sources are the generators rebuilt, not `.qpa` copies (section 1). The
  rebuild is byte-equal to 106 of 106 verbatim copies of the kinds it writes.
- "Byte-equal on `cbb6b45b`" is a scratch measure on a commit that is not on
  the tip. It holds only if `lower-vsloop` lands unchanged.
- `return_after_continue_vertex` has no reference. Its V1 outcome is assumed
  from `return_after_break_vertex` (same shape, the `continue` in place of the
  `break`).
- Steps V3-V7 are designs. Their next refusal past LoopVertex is not
  measured. The forecast is "leaves the text", not "passes".
- `deqp_loops2_vs_ret_static` is not a `return.*` shape. Its `.license`
  names `functions.control_flow.return_in_nested_loop_vertex`, and its
  `.vert` is byte-equal to `deqp_vsloop_fn_nested.vert`. The `return.*`
  rows above use `ret_static_loop_{always,never}`, `ret_static_dyn`,
  `ret_infinite` and the new `ret_dynamic_loop_always`. (It is a loops2 file.
  It is left as it is.)
