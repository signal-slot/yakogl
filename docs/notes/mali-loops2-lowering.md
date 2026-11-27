<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall v10): the open loop-lowering steps, their Mesa 26.2.1 references and forecasts

**HOST-ONLY.** Plan #387 item loops2-refs (issue #7, ROADMAP stage 5). This
is a design with references. Nothing under `src/` changed. It adds 80
references `tools/mali/testdata/deqp_loops2_*` (with `.license` and
`.nir.sha` files), the table `OPEN_STEPS` and two tests in
`tests/mali_deqp_loops2.rs`, and this note. Every count is a compile or link
outcome on the host at the integration tip `4d8a0b40`, never a render
verdict.

The steps of `loops-lower-1` (A0, S4, S3, S5, S6, S2-fs, S7) and of
`loops-lower-2` (S1, S8-S11, S2-vs) in
[`mali-loops-lowering.md`](mali-loops-lowering.md) sections 4-5 have partly
landed in trains 1 and 2: the plain mediump kept loop (S3), the `for (;;)`
break, the vertex kept loop with float counters (S8) and the counted
constant loops compile today (`COMPILED` in `tests/mali_deqp_loops2.rs`). The
prototype diff of the rest is lost (the plan's handoff). This note says
what is still open, measured case by case, and draws the references the
open steps lacked. Read with [`mali-loops-lowering.md`](mali-loops-lowering.md)
(Mesa's unroll decision, section 2; the kept-loop words, section 1.1),
[`mali-deqp-idxloc.md`](mali-deqp-idxloc.md) (the indexing half: IL-1..IL-6,
IL-V), [`mali-deqp-ctrlflow.md`](mali-deqp-ctrlflow.md) (its step 6, the
`if` in a kept `do`-`while`, is LP2/LP3 here) and
[`mali-lowering-schedule.md`](mali-lowering-schedule.md) (rows LS1-LS11).

## 0. Result in brief

- **202 of the 624 must-pass `functional.shaders.loops.*_iterations.*` cases
  are refused at the tip** (139 vertex, 63 fragment); 193 of them by a loop
  or control-flow text. All 193 fall into the nine steps of section 3.
  The other 9 refuse with other families' texts (6 `MixedPrecision`
  `basic_highp_float_fragment`, 3 `Arith` `select_iteration_count_fragment`
  with a dynamic bound).
- **No case of `shaders.loops` or `shaders.indexing` reaches
  `IndexedLocal` at the tip.** Its message is still in `mod.rs:512`, but the
  front end lowers every run-time local index to a select ladder
  (`tests.rs:1102`). The loop-indexed shapes stop at the loop texts or at
  other families' texts (section 5), as `mali-deqp-idxloc.md` 0 found.
- **Mesa compiles every one of the 80 shapes.** None was refused by the
  shim, and every draw ended with `glGetError 0x0`. 29 of them have no loop
  in Mesa's words: Mesa deletes the loop or unrolls it to straight code.
- **Many open steps target words the walk already makes.** 42 of the 80
  references have a word-for-word twin (all three programs) among the
  committed references. For 30 of them the twin compiles at the tip
  (section 2). So for LP1, LP4, LP5 and LP6 the work is mostly a
  normalisation before the walk, not new selection code.
- **Forecast** (section 4): LP1 12, LP4 37, LP5 18 and LP6 12 cases compile
  after a front-end normalisation, and the source transforms measured that
  (LP4 is the upper bound of its twins). LP2 frees 4 of its 12 that way. LP3
  (30), LP7 (30), LP8 (36) and IDX (10) need new walk code; their next
  refusal is not measured.

## 1. Method

- **The cases.** No `.qpa` of a host measure is in the tree. So the shaders
  were rebuilt from the generators `es2fShaderLoopTests.cpp` and
  `es2fShaderIndexingTests.cpp` of VK-GL-CTS `93bca01`, fetched from the
  Khronos repository at that commit. Two scratch Python ports, under
  `$CONNECTED_DEPS_DIR/loops2-gen` and not committed, write every case.
  - The rebuild gives the 624 loop and the 568 indexing names of
    `tools/cts/gles2-mustpass.txt`, the same set (diff empty).
  - **93 rebuilt loop pairs are byte-identical to the committed `.qpa`
    copies** (every `deqp_loops*_{fs,vs}_{for,wh,do}_*`), and so are 90
    indexing pairs (`deqp_idxloc_*` except the four `struct` ones, and every
    `deqp_loops2_*idx*`). The vertex and fragment `dynamic_subscript`
    writes keep the generator's alignment spaces.
- **The refusal at the tip.** A scratch test (not committed) ran
  `tests/mali_ref`'s `compile_pair` on every rebuilt pair, with the features
  `host-test,gpu-mali,shaderc-online,no-shader-table`.
- **The refusal site.** A throw-away copy of the tip in `$TMPDIR` printed
  `file!():line!()` at each construction of `LoopVertex`, `LoopMediump`,
  `GuardMediump`, `ControlFlow` and `LoopForm` in the walks. The edits were
  inline, so the line numbers are the tip's. The copy was deleted afterwards.
  The site in section 3 is the last one of the refusal's kind before the
  pair's compile returned.
- **"Pinned".** A case is pinned when a committed reference has its vertex
  and fragment sources byte for byte. The 80 new references are the
  unpinned cases picked per step: 8-12 per step, spread over `for`,
  `while` and `do`-`while` and over the `uniform`, `dynamic` and `constant`
  bounds.
- **References.** Mesa 26.2.1's drm-shim was built into
  `$CONNECTED_DEPS_DIR/mesa-shim` (`tools/mali/drm_shim_ref.sh build`). Each
  reference was drawn with

  ```sh
  MESA_SHIM_WORK="$CONNECTED_DEPS_DIR/mesa-shim" tools/mali/deqp_ref.sh \
      <case>.vert <case>.frag tools/mali/testdata/deqp_loops2_<name> --instrument shim
  ```

  and its `.nir.sha` with `tools/mali/nir_refs.sh`. To check that build,
  `deqp_loops2_fs_for_uni_nested`, `deqp_loops2_vs_for_uni_conditional_break`
  and `gm2_cf_ifelse_fs` were drawn again: each `expect.json` came out
  byte-equal to the committed one. So was `deqp_loops2_fs_for_uni_nested.nir.sha`.
  The G310 needs nothing separate (`mali-g310-vs-g610.md`).
- **Twins and forecasts.** A twin is a committed reference whose
  `vs.position`, `vs.varying` and `fs.fragment` words equal the new one's.
  Each forecast compiles a source transform: the case rewritten into the
  form a normalisation would hand the walk (section 4). It is then compared
  with `mali_ref::differences` against the case's own reference.
- **Notation.** `n Lk Fm` means n instructions with k backward and m
  forward `BRANCH*`. "+cap" means our kept loop's three pass-counter words
  (`LoopCap`), which `CapEdit` in `src/shaderc/online/mali/tests.rs` takes
  out before a compare.

## 2. Counts at `4d8a0b40`

| part | refused | of them pinned before | new references |
|---|---:|---:|---:|
| `shaders.loops`, fragment | 63 (`LoopMediump` 24, `ControlFlow` 20, `GuardMediump` 10, `MixedPrecision` 6, `Arith` 3) | 19 | 32 |
| `shaders.loops`, vertex (`LoopVertex`) | 139 | 46 | 44 |
| `shaders.indexing`, vertex, a run-time write then a loop read (`LoopVertex`) | 10 | 1 (`deqp_unif2_vs_int_uniform`) | 4 |

Twins of the new references that compile at the tip:

| new references | twin (words of all three programs equal) | twin at the tip |
|---|---|---|
| `fs_*_only_continue` (6) | `deqp_loops2_fs_for_uni_empty_body` | compiles; 2 differences (our kept empty loop; Mesa deletes it) |
| `fs_for_uni_unconditional_continue` | `deqp_loops_fs_for_uni`, `deqp_loops2_fs_for_uni_{101_iterations,single_iteration,function_call_inout}` | compiles, Mesa's +cap |
| `fs_{for,do}_dyn_unconditional_continue`, `fs_do_uni_unconditional_continue` | `deqp_loops2_fs_{for,do}_dyn_101_iterations`, `deqp_loops2_fs_do_uni_101_iterations` | compiles, Mesa's +cap |
| `fs_wh_{uni,dyn}_unconditional_continue` | `deqp_loops2_fs_wh_{uni,dyn}_101_iterations` | compiles; the `while (i++ < n)` counter order differs (`COMPILED` false) |
| `fs_{wh,do}_uni_infinite_with_conditional_break` | `deqp_loops_fs_inf_break` | compiles, Mesa's +cap |
| `vs_for_con_unconditional_break`, `vs_do_con_nested_sequence`, LP5 `*_break_last`, `vs_do_dyn_unconditional_break` | `deqp_loops_vs_nested_const` | **byte-equal** |
| `vs_do_con_unconditional_continue`, `vs_do_con_nested_tricky_dataflow_2` | `deqp_loops_vs_for_const`, `deqp_loops_vs_while_const`, `deqp_missing_loops_vs_for_constant` | **byte-equal** |
| `vs_do_con_only_continue`, `vs_for_con_infinite_with_unconditional_break_first`, LP5 `*_break_first` and `*_only_continue` | `deqp_loops_vs_dowhile_const` | **byte-equal** |
| `vs_for_uni_{function_call_inout,unconditional_continue}` | `deqp_loops_vs_for_uni` | compiles, Mesa's +cap |
| `vs_do_uni_unconditional_continue` | `deqp_loops_vs_dowhile_uni`, `deqp_loops2_vs_do_con_101_iterations` | compiles, Mesa's +cap |

The other twins are references that are themselves refused at the tip.
LP4's other seven references (`vs_for_con_conditional_continue` and the
rest) are twins of `deqp_loops2_vs_do_con_nested` and the `deqp_vsloop_con_*`
references, all 11 words. `fs_do_uni_double_continue` is a twin of
`deqp_loops2_fs_do_uni_conditional_continue`.

## 3. The steps: site, Mesa's shape, the lowering

Each step names its dEQP cases (`functional.shaders.loops.` unless
indexing), its measured site at the tip and the references that pin it
(prefix `deqp_loops2_`). The cases are all cases of the step at the tip,
pinned or not. The step names are this note's own. The column "design" maps
each one to `mali-loops-lowering.md`.

| step | design | cases | site at the tip | new references |
|---|---|---:|---|---:|
| LP1 | S4 + S3 | 12 | `nir_fs.rs:6759` `walk_nodes` | 12 |
| LP2 | S5 | 12 | `nir_fs.rs:5569` `walk_if16` (10), `:5603` (2) | 8 |
| LP3 | S6 | 30 | `nir_fs.rs:6218` `walk_kept_loop` (24), `:5603` `walk_if16` (6) | 12 |
| LP4 | S1 | 37 | `nir.rs:5039` `vs_steps` | 12 |
| LP5 | S1 (`wrapper_unroll`, `dead_cf`) | 24 | `nir.rs:5039` | 8 |
| LP6 | S8 | 12 | `nir.rs:5039` (6 `unconditional_continue`), `nir.rs:5786` `vs_kept_end` (6 `function_call_inout`) | 6 |
| LP7 | S9 | 30 | `nir.rs:6082` `walk` (16), `:5039` (8), `:5551` `vs_kept_begin` (6) | 10 |
| LP8 | S10 | 36 | `nir.rs:5039` | 8 |
| IDX | S2-vs / IL-V | 10 | `nir.rs:5806` `vs_kept_end` | 4 |
| | | **203** | | **80** |

### LP1 (fragment, 12): `continue` as the whole body or its last statement

- **Cases:** `{for,while,do_while}_{uniform,dynamic}_iterations.{only_continue,unconditional_continue}_fragment`.
- **Site:** `nir_fs.rs:6759`. A `Node::Continue` reached when `f.loops == 0`
  (the walk is in a kept loop, not an unrolled one) falls through to
  `ControlFlow`.
- **Mesa:**
  - `only_continue`: 8 words, no loop (`nir_opt_dead_cf`), the words of a
    pass-through shader. The `do`-`while` form is deleted too.
  - `unconditional_continue`: the plain kept loop of
    `mali-loops-lowering.md` 1.1: `for`/`do` 17/18 (`uni`) and 23 (`dyn`,
    the `IMUL.i32` of `one * ui_three` in the header), `while` 17/23. It is
    word for word the `single_statement` / `101_iterations` twin (section 2).
- **Lowering:** in the front end or at the top of `walk_kept_loop`, drop a
  `continue` that is the last node of a loop body. Then a body of no node
  is A0's `Delete` (S4: `dead_cf`, no side effect, nothing used after it).
  This needs no new selection.
- **References:** `fs_{for,wh,do}_{uni,dyn}_{only,unconditional}_continue` (all 12).

### LP2 (fragment, 12): run-time `break`/`continue` in a kept `while (true)` or `do`-`while`

- **Cases:** `{while,do_while}_{uniform,dynamic}_iterations.infinite_with_conditional_break_fragment` (4),
  `do_while_{uniform,dynamic}_iterations.{conditional_break,conditional_continue,double_continue,mixed_break_continue}_fragment` (8).
- **Site:** `nir_fs.rs:5569`. `walk_if16` refuses an exit other than
  `IfExit::Plain` (`GuardMediump`), because the loop was taken for
  unrolling with a run-time guard. For the two `mixed_break_continue`
  cases it is `:5603`, the two sides of the `if`/`else if` flowing
  differently (`ControlFlow`).
- **Mesa:** all are kept, one loop.
  - `infinite_with_conditional_break`, `while (true)` and `do {} while
    (true)`: the words of the `for (;;)` form (`deqp_loops_fs_inf_break`, 18
    words; 23 with the dynamic `IMUL.i32` of `one*ui_one` before
    `ICMP_OR.u32.eq.m1`). The head `if (i == n) break;` is the loop's
    only test, with no inverted compare.
  - `do`-`while` `conditional_continue`/`double_continue`
    (`fs_do_dyn_conditional_continue`, 28 words): the continue mask
    `ICMP_OR.u32.eq.m1 r4, r3, <two>` comes before the step `IADD_IMM`. The
    bound test follows. Four `MUX.v2i16 ... r4.h00` select the rotated
    against the unrotated halves, and two `MKVEC.v2i16` rebuild the pairs
    after the exit `BRANCHZ`. `double_continue` has the same words as
    `conditional_continue`, because its trailing `continue` is LP1's.
  - `do`-`while` `mixed_break_continue` (`fs_do_dyn_mixed_break_continue`,
    36 words, L1 F4) is **the only shape here with a second exit**.
    `ICMP_OR.u32.ne.m1` (`i != 0`) and `ICMP_AND.u32.eq.m1` (`i == 3`) feed a
    `BRANCHZ.eq.reconverge ... offset:14` out of the loop, behind three
    `U16_TO_U32` copies of the live halves. The continue is again four
    `MUX.v2i16`.
- **Lowering:**
  1. Normalise `while (true)` and `do { ... } while (true)` with a head
     `if (c) break;` to the `for (;;)` form the kept walk takes (4 cases).
  2. In `kept_loop`/A0, take a `do`-`while` whose body starts with
     `if (c) break;` or `if (c) continue;` as kept, not unrolled. Give it
     S5's masks in Mesa's order: the user's `i` before the step.
  3. For `mixed_break_continue` in a `do`-`while`, emit a second exit edge
     (a forward branch to after the back edge) instead of `LSHIFT_OR`
     merging it into the bound test. The `for`/`while` forms merge
     (`deqp_loops_fs_mixed`).
- **References:** `fs_do_dyn_conditional_continue`, `fs_do_{dyn,uni}_double_continue`,
  `fs_{do,wh}_{dyn,uni}_infinite_with_conditional_break`, `fs_do_dyn_mixed_break_continue`.
  Pinned before: `fs_do_{uni,dyn}_conditional_break`, `fs_do_uni_conditional_continue`.

### LP3 (fragment, 30): sequenced and nested kept loops

- **Cases:** `{for,while,do_while}_{uniform,dynamic}_iterations.{sequence,nested,nested_sequence,nested_tricky_dataflow_1,nested_tricky_dataflow_2}_fragment`.
- **Site:** `nir_fs.rs:6218`. `walk_kept_loop` refuses a second kept loop
  (`f.kept.is_some()`, a `Node::Loop` in the body, or `f.nir.nloops > 0`) in
  a mediump shader (`LoopMediump`, 24). The six `do`-`while`
  `nested`/`nested_tricky_dataflow_*` cases stop earlier, at `:5603`: the
  outer `do`-`while` is unrolled and its body's inner loop flows
  differently on the two sides.
- **Mesa:** a straight composition of the single kept loop (1.1).
  - `sequence` (`fs_wh_uni_sequence`, 25 words, L2 F2): the second loop
    keeps the first loop's counter register (`r2`, no new init). Between the
    loops come `MOV.i32.reconverge r1, r3^; MOV.i32 r3, r1^`, the
    out-of-SSA copies of the pair. For a `for` (`fs_for_dyn_sequence`, 33)
    the `i` declared outside both loops is the same counter.
  - `nested_sequence` (31-40 words, L3 F3): the two inner loops re-init the
    inner counter.
  - `nested_tricky_dataflow_1` in a `do`-`while`
    (`fs_do_uni_nested_tricky_dataflow_1`, 25 words, L2 F2): the inner
    loop runs on a copy of `coords` (`MOV.i32 r3, r0; MOV.i32 r4, r1`). The
    outer latch is `IADD_IMM` plus the test, with an empty exit `BRANCHZ
    ... offset:1` before the outer back edge (offset -14). The pairs are
    rebuilt after the loop (two `MKVEC.v2i16` around `ATEST`).
- **Lowering:** S6 of `mali-loops-lowering.md` 4. Lift the mediump gate of
  `:6218` for nested and sequenced loops, with one `LoopCap` per loop, the
  outer loop's 16-bit pair phis around the inner one's, and the inner
  counter re-initialised in the outer body. For the six `do`-`while`
  forms, A0 must keep the outer loop (its bound is a uniform) instead of
  unrolling it into `walk_if16`. The phi copies between loops are
  `bi_out_of_ssa` order: compare them first (risk in 8 of that note).
- **References:** `fs_{for_dyn,wh_uni,do_uni}_sequence`, `fs_{for_dyn,wh_uni,do_uni}_nested_sequence`,
  `fs_for_dyn_nested_tricky_dataflow_1`, `fs_wh_uni_nested_tricky_dataflow_{1,2}`,
  `fs_do_uni_nested_tricky_dataflow_{1,2}`, `fs_do_dyn_nested_tricky_dataflow_2`.
  Pinned before: `fs_{for,wh}_uni_nested`, `fs_for_uni_{sequence,nested_sequence,nested_tricky_dataflow_*}`,
  `fs_{for,wh}_dyn_nested`, `fs_do_{uni,dyn}_nested`.

### LP4 (vertex, 37): constant-bound loops with `break`, `continue`, `if` or nesting

- **Cases:** every refused `{for,while,do_while}_constant_iterations.*_vertex`
  (37). These are `conditional_*`, `double_continue`, `unconditional_*`,
  `mixed_break_continue`, `infinite_with_*`, `only_continue`, `nested*`,
  `sequence` and `conditional_body`.
- **Site:** `nir.rs:5039`. `vs_steps` takes a top-level loop only in
  `vs_loop_shape` (a counted or uniform-bounded `for` with a plain body);
  any other loop is `Step::Refuse(LoopVertex)`.
- **Mesa:** everything folds; there is no loop and no select. `vs.varying`
  is 11 words (two `V2F32_TO_V2F16` or none, the permuted attribute lanes
  stored as fp32), one of four programs. The program is fixed by
  `numIters % 4`: `deqp_loops_vs_for_const`, `deqp_loops_vs_nested_const`,
  `deqp_loops_vs_dowhile_const`, or the `deqp_loops2_vs_do_con_nested` /
  `deqp_vsloop_con_*` one. **The walk emits the first three byte-equal
  today** (section 2).
- **Lowering:** S1 of `mali-loops-lowering.md` 5. In the vertex walk,
  `simple_unroll`/`complex_unroll` a loop whose trip count folds: each
  pass's `break`/`continue`/`if` folds on the constant counter. Then the
  walk sees straight code, and its words are those it already makes. It
  needs A0's decision in `nir.rs` (it is shared with the fragment walk's
  `walk_passes`).
- **References:** `vs_for_con_{conditional_continue,double_continue,unconditional_break,conditional_body,infinite_with_unconditional_break_first}`,
  `vs_wh_con_{conditional_break,mixed_break_continue,infinite_with_conditional_break}`,
  `vs_do_con_{unconditional_continue,only_continue,nested_sequence,nested_tricky_dataflow_2}`.
  Pinned before: `vs_do_con_nested` and the `deqp_vsloop_con_*` references.

### LP5 (vertex, 24): run-time-bound loops Mesa deletes or wraps

- **Cases:** `{for,while,do_while}_{uniform,dynamic}_iterations.{infinite_with_unconditional_break_first,infinite_with_unconditional_break_last,unconditional_break,only_continue}_vertex`.
- **Site:** `nir.rs:5039`, as LP4.
- **Mesa:**
  - The loop is deleted: `only_continue` and `*_break_first` (11 words,
    `deqp_loops_vs_dowhile_const`'s).
  - It is straight code: `*_break_last` and the `do`-`while`
    `unconditional_break` (11 words, `deqp_loops_vs_nested_const`'s).
  - It is a `wrapper_unroll`: the `for`/`while` `unconditional_break`
    (`vs_wh_uni_unconditional_break`, 20 words). That is
    `ICMP_OR.s32.ge.m1 r2, 0x0, u5.w1` (the first test, `0 < bound`) and three
    `MUX.v2i16 ... r2.h00`, the words of `deqp_loops2_vs_for_uni_unconditional_break`.
- **Lowering:** A0's `Delete` and `wrapper_unroll` decisions in the vertex
  walk (S1's `unconditional_break` part, the vertex twin of S4). The
  wrapper is one guarded pass, a select on the first test. The fragment
  walk already makes it (`deqp_loops_fs_uncond_break`).
- **References:** `vs_for_uni_infinite_with_unconditional_break_last`,
  `vs_wh_dyn_infinite_with_unconditional_break_first`,
  `vs_do_uni_infinite_with_unconditional_break_first`,
  `vs_do_dyn_infinite_with_unconditional_break_last`, `vs_do_dyn_unconditional_break`,
  `vs_wh_uni_unconditional_break`, `vs_wh_uni_only_continue`, `vs_for_dyn_only_continue`.

### LP6 (vertex, 12): the plain kept loop behind a trailing `continue` or an `inout` call

- **Cases:** `{for,while,do_while}_{uniform,dynamic}_iterations.{unconditional_continue,function_call_inout}_vertex`.
- **Site:** `unconditional_continue` (6) is at `nir.rs:5039`: the
  `continue` node breaks `vs_loop_shape`. `function_call_inout` (6) is at
  `nir.rs:5786`, in `vs_kept_end`: after inlining, the carried pair's two
  lanes are not both fp16 at the latch, or are the phis themselves. The
  `inout` copy-back leaves the pair as found.
- **Mesa:** the vertex kept loop of 1.1, the words of `deqp_loops_vs_for_uni`
  (24 words, L1 F1) for `uni`. `do`-`while` matches
  `deqp_loops_vs_dowhile_uni`. `dyn` is 29-32 words.
- **Lowering:**
  - `continue`: the same drop as LP1.
  - `inout` call: copy the inlined callee's parameter back into the
    caller's local *before* the kept walk builds its phis, so that the
    latch value is the rebuilt pair. Today the latch sees the copy itself.
  - The six `dyn` forms also need Mesa's mediump `int(a_one + 0.5)` in the
    vertex walk: `FADD.f32 r0.h0, r0^, 0x0.neg`,
    `FADD.v2f16 r0, r0^.h00, 0x38000000.h11`, `F16_TO_S32.rtz`. We emit
    `FADD_IMM.f32 r0, r0^, #0x3F000000` (word 1). The existing
    `deqp_vsloop2_for_dyn_unconditional_continue` has the same difference.
- **References:** `vs_{for,do}_uni_function_call_inout`, `vs_wh_dyn_function_call_inout`,
  `vs_{for,do}_uni_unconditional_continue`, `vs_wh_dyn_unconditional_continue`.
  Pinned before: `deqp_vsloop2_for_dyn_unconditional_continue`.

### LP7 (vertex, 30): sequenced and nested kept vertex loops

- **Cases:** `{for,while,do_while}_{uniform,dynamic}_iterations.{sequence,nested,nested_sequence,nested_tricky_dataflow_1,nested_tricky_dataflow_2}_vertex`.
- **Site:**
  - `nir.rs:6082` (`walk`, "a loop in a loop: both unrolled, or no
    reference", 16: the `for`/`while` nested forms).
  - `nir.rs:5039` (8: the `do`-`while` nested forms, not `vs_loop_shape`).
  - `nir.rs:5551` (`vs_kept_begin`, 6 `sequence`). The second loop's
    carried locals are not two 32-bit lanes, because the first loop left
    them as fp16 pairs.
- **Mesa:** the fragment LP3 words in `vs.varying`, then the
  `F16_TO_F32` tail and the stores. `vs_wh_uni_sequence` is 32 words: two
  loops on one counter register, with the `MOV.i32.reconverge r1, r3^;
  MOV.i32 r3, r1^` copies between them. `vs_for_dyn_nested_sequence` is
  46 words, L3. `vs_do_uni_nested_tricky_dataflow_1` is 31 words, L2.
- **Lowering:** S9. The vertex walk's flat `Step` list needs a second
  `KeptLoop` record, nested (the outer phis around the inner's) or after
  the first. The second loop's carried values enter as the first loop's
  16-bit pairs. `vs_kept_begin`'s 32-bit check must accept a pair a kept
  loop left.
- **References:** `vs_{for_dyn,wh_uni,do_dyn}_sequence`, `vs_{for_dyn,wh_uni}_nested_sequence`,
  `vs_{wh,do}_dyn_nested`, `vs_do_uni_nested_tricky_dataflow_1`,
  `vs_wh_dyn_nested_tricky_dataflow_1`, `vs_for_dyn_nested_tricky_dataflow_2`.
  Pinned before: `vs_{for,do}_uni_nested`, `vs_for_dyn_nested`, `vs_for_uni_nested_*`,
  `vs_do_dyn_nested_sequence` and the `deqp_vsloop*` ones.

### LP8 (vertex, 36): `break`, `continue` and `if` in a kept vertex loop

- **Cases:** `{for,while,do_while}_{uniform,dynamic}_iterations.{conditional_break,conditional_continue,double_continue,mixed_break_continue,infinite_with_conditional_break,conditional_body}_vertex`.
- **Site:** `nir.rs:5039`: the `if`, `break` or `continue` node breaks
  `vs_loop_shape`.
- **Mesa:** S5's masks in `vs.varying`:
  - `conditional_continue`: 4 `MUX.v2i16` on the `i == n` mask
    (`vs_wh_dyn_conditional_continue`, 37 words).
  - `mixed_break_continue`, `for`/`while`: `LSHIFT_OR` of the break mask
    into the bound test (`vs_wh_uni_mixed_break_continue`, 33 words).
  - `mixed_break_continue`, `do`-`while`: a second exit branch
    (`vs_do_dyn_mixed_break_continue`, 42 words, L1 F4), as LP2's.
  - `infinite_with_conditional_break`: the head test only
    (`vs_wh_uni_infinite_with_conditional_break`, 24 words).
- **Lowering:** S10. Keep these in the vertex walk with the fragment walk's
  S5 mask forms (one exit mask, `MUX` selects for a `continue`'s skipped
  statements). Add the second exit edge for the `do`-`while`
  `mixed_break_continue`.
- **References:** `vs_for_uni_double_continue`, `vs_wh_dyn_{conditional_continue,double_continue}`,
  `vs_do_dyn_{conditional_continue,mixed_break_continue}`, `vs_wh_uni_mixed_break_continue`,
  `vs_for_dyn_infinite_with_conditional_break`, `vs_wh_uni_infinite_with_conditional_break`.
  Pinned before: `vs_{for,wh,do}_*_conditional_{body,break}`, `vs_for_uni_conditional_continue`,
  `vs_for_uni_mixed_break_continue` and the `deqp_vsloop2_*` ones.

### IDX (vertex, 10): a run-time-indexed write, then a loop that reads by the counter

- **Cases:** `indexing.tmp_array.{float,vec2,vec3,vec4}_dynamic_write_dynamic_loop_read_vertex`,
  `indexing.matrix_subscript.{mat2,mat3,mat4}_dynamic_write_dynamic_loop_read_vertex`,
  `indexing.vector_subscript.{vec2,vec3,vec4}_dynamic_subscript_write_dynamic_loop_subscript_read_vertex`.
  These are the 10 indexing cases outside the 62 vertex ones of
  `mali-deqp-idxloc.md`, which all have a loop *write*.
- **Site:** `nir.rs:5806` (`vs_kept_end`). A carried lane the body leaves
  as found, or an `Undef`: the array's lanes are carried as phis, but the
  body only reads them.
- **Mesa:**
  - The arrays and matrices have no loop. `partial_unroll` by the array
    length guards each pass on `k < ui_four`, the `ICMP_OR.s32.lt.m1
    0x...b?, u7.w0` + `MUX.v2i16` ladder of
    `vs_idx_tmp_float_dynw_dlr` words 48-57, with the last select against
    0. The four run-time stores before it are a mask tree:
    `ICMP_OR.s32.lt.m1 u.., <k>`, `LSHIFT_OR`/`LSHIFT_AND` (`.not`) and
    `MUX.v2i16` (words 4-44). That is 75 words for `float`, 103 for
    `vec4`, and 64 for `mat3` (s20).
  - The vector subscript **keeps the loop** (`vs_idx_vec3_dynsubw_dlr`, 40
    words, L1). The write is an `ICMP_OR.u32.eq.m1` +
    `MKVEC.v2i16`/`MUX.v2i16` tree, and the loop reads with an `lt` ladder
    (`ICMP_OR.s32.lt.m1 r3, 0x...b?` + `MUX.v2i16`).
- **Lowering:**
  - The arrays and matrices take S2-vs's `Partial { n }` rule (IL-V). Its
    guarded accumulation must also read the run-time-written lanes, so the
    store tree goes before the unrolled reads.
  - The vector takes S11's kept loop with the read ladder.
  - Neither needs `IndexedLocal`.
- **References:** `vs_idx_tmp_{float,vec4}_dynw_dlr`, `vs_idx_mat3_dynw_dlr`, `vs_idx_vec3_dynsubw_dlr`.
  Pinned before: `deqp_unif2_vs_int_uniform` (the `mat2` case).

## 4. Forecast (host, upper bounds)

Each "after" column compiles the case's source rewritten into the form the
step's normalisation hands the walk. The rewrites:

- LP1/LP6: drop the trailing `continue`; delete an `only_continue` loop.
- LP2: `while (true)` and `do {} while (true)` become `for (;;)`.
- LP5: delete a `*_break_first` or `only_continue` loop; make a
  `*_break_last` loop its body.
- LP6: inline `func(res)` as `res = res.yzwx`.

Each rewrite is compared with the case's reference where one exists. LP3,
LP7, LP8 and IDX need walk code that a source rewrite cannot stand in for;
their count is the cases that leave the loop texts, and their next refusal
is not measured.

| step | cases | after the rewrite (measured) | compares with Mesa | dEQP groups it unblocks |
|---|---:|---|---|---|
| LP1 | 12 | 12 compile | 6 `only_continue` byte-equal; 6 `unconditional_continue` Mesa's +cap (4 `for`/`do`), counter order (2 `while`, the `COMPILED` false defect) | `loops.{for,while,do_while}_{uniform,dynamic}_iterations` |
| LP2 | 12 | 4 compile (`infinite_with_conditional_break`), 8 still refused (6 `GuardMediump`, 2 `ControlFlow`) | the 4: Mesa's +cap | `loops.{while,do_while}_{uniform,dynamic}_iterations` |
| LP3 | 30 | not measured | - | `loops.*_{uniform,dynamic}_iterations` |
| LP4 | 37 | not measured: a full unroll is LP4's own code | the 12 references' twins are byte-equal at the tip | `loops.{for,while,do_while}_constant_iterations` |
| LP5 | 24 | 18 compile, 6 (`unconditional_break`) still refused | 8 of the 18 byte-equal (each one with a reference; the other 10 have none) | `loops.*_{uniform,dynamic}_iterations` |
| LP6 | 12 | 12 compile | 6 `uni` Mesa's +cap; 6 `dyn` also differ at the `a_one` narrowing (LP6) | `loops.*_{uniform,dynamic}_iterations` |
| LP7 | 30 | not measured | - | `loops.*_{uniform,dynamic}_iterations` |
| LP8 | 36 | not measured | - | `loops.*_{uniform,dynamic}_iterations` |
| IDX | 10 | not measured | - | `indexing.{tmp_array,matrix_subscript,vector_subscript}` |
| **sum** | **203** | | | |

Order of work, by cases freed per session and by what they share:

1. **LP4 + LP5** (61, vertex): one decision, A0's `Unroll`/`Delete`/`Wrapper`
   in `nir.rs`, which reaches words the walk already emits.
2. **LP1 + LP6's `continue` and `inout`** (24): one front-end normalisation
   plus the `inout` copy-back. The 6 `dyn` vertex forms then wait on the
   `a_one` narrowing.
3. **LP8** (36) with **LP2** (12): the S5 mask forms in both walks, and the
   one second-exit shape (`do`-`while` `mixed_break_continue`, 4 cases).
4. **LP3** (30) and **LP7** (30): nested and sequenced kept loops, fragment
   then vertex (S6, S9).
5. **IDX** (10) with IL-V of `mali-deqp-idxloc.md`.

The fragment counts overlap no other family. The vertex `dyn` cases (LP5-LP8,
about half of each) also meet the `a_one` narrowing of LP6. That
narrowing is the vertex twin of the fragment `v_one` form the S3 step
landed.

## 5. The test

`tests/mali_deqp_loops2.rs` adds:

- `OPEN_STEPS`: the 80 references, each with its step and its exact first
  refusal at the tip.
- `open_steps_refused`, which pins them.
- `open_steps_compares`, the byte compare, `#[ignore]`d.
- `references_listed`, which now counts them with `REFUSALS` and `COMPILED`.

The step that lifts a reference moves it out of `OPEN_STEPS` in the same
commit. Mesa keeps a loop in 51 of the 80 (LP1's six
`unconditional_continue`, LP2, LP3, LP6, LP7, LP8 and
`vs_idx_vec3_dynsubw_dlr`): compare those with `CapEdit`. The other 29
(LP1's six `only_continue`, LP4, LP5, IDX's three arrays and matrices) have
no loop in Mesa's words and are plain `mali_ref::assert_matches`.

```text
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_loops2
```

`open_steps_refused` passes unchanged on the result commits of the two
pending compiler items: `lower-train-2` (`13259474`) and `lower-fsint`
(`e8327e42`). On `lower-fsint`'s older base, the existing `compiled_pinned`
fails; that is not this item's table.

## 6. Caveats

- The sources are the generators' output rebuilt by hand, not `.qpa`
  copies. 93 + 90 rebuilt pairs are byte-identical to verbatim copies.
- The forecasts are source rewrites, not the lowering. They are exact
  where the rewrite is what Mesa's NIR does first (a trailing `continue`, a
  dead loop, `while (true)` as `for (;;)`). For LP3, LP7, LP8 and IDX
  nothing was measured past the loop text.
- The site of a vertex `Step::Refuse` (`nir.rs:5039`) is where the step is
  recorded. The error is returned later, when the walk reaches the step.
- Step names LP1-LP8 and IDX are this note's own. The schedule's
  LS1-LS11 are `mali-loops-lowering.md`'s, which the "design" column of
  section 3 maps.
