<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the dEQP-GLES2 vertex-shader loops left at 8ff01055

Plan #402 item `ref-vsloop` (issue #20, ROADMAP stage 5). This item adds
references only. It changes no compiler code. The refusal is
`MaliError::LoopVertex`, "Valhall: a loop in a vertex shader is not lowered
yet" (`src/shaderc/online/mali/mod.rs:509`). In the whole-tree NanoPC-T6 run
at `8ff01055` (`docs/es2-cts-failures.md` section M; the slices
`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv`) it is the
first refusal of 196 cases: **176 Fail and 20 NotSupported**. The run at
`37305608` had 105 Fail and 392 NotSupported.

The references are `tools/mali/testdata/deqp_vsloop_*`. The test is
`tests/mali_deqp_vsloop.rs`:

```text
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsloop
```

Every count here comes from the board slices or from a host-only compile.
Nothing here ran on the NanoPC-T6.

## 1. The growth from 105 to 176 Fail

The 71 extra Fail cases are not new refusals. The slices of both runs show
where they came from:

| movement | cases |
|---|---|
| `loops.*_{constant,uniform,dynamic}_iterations` vertex cases with the same refusal in both runs: NotSupported at 37305608, Fail at 8ff01055 | 35 + 54 + 54 = 143 |
| cases that had another first refusal at 37305608 and now stop at the loop: `return.return_in_*_loop_*_vertex` 7, `functions.control_flow.{return_in_loop,return_in_loop_if,return_in_nested_loop,mixed_return_break_continue}_vertex` 4 | 11 |
| cases that left the family | 312 |
| of those, now Pass: 56 constant loops, 3 uniform loops, 4 `indexing.uniform_array`, 4 `scoping.valid`, 1 `functions.control_flow` | 68 |
| of those, still not Pass but stopped by another first refusal: 91 uniform and dynamic loops, 142 `indexing.*` cases, 9 `invariance`, 2 `struct.uniform` | 244 |

The family as a whole went from 497 cases to 196. The NotSupported-to-Fail
change is a change of status in the board run, with the same first refusal.
This item does not explain it. It is noted for plan #377's acceptance c7,
which compares the Fail counts with those at 37305608.

## 2. How the cases were measured

1. **The case list.** The 196 lines of the four slices' `refusals.tsv`
   whose text holds "a loop in a vertex shader", with each case's status
   from `results.tsv` (section 3). `--filter` matches case names, not
   refusal text, so the list was passed as a file.
2. **The host measure** at `63c6418d` (the references' first commit; it
   changes no compiler code, so the compiler is the one of `e65567ea`),
   VK-GL-CTS `93bca01` fetched shallow into `$CONNECTED_DEPS_DIR` with the
   yakogl target patch and its external sources:

   ```text
   grep -h 'a loop in a vertex shader' tools/cts/log/2026-11-24-t6-gles2-S{1,2,3,4}-8ff01055/refusals.tsv | cut -f1 > cases.txt
   tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS-host" \
       --slice tools/cts/log/2026-11-24-t6-gles2-S1-8ff01055 ... --slice ...-S4-8ff01055 \
       --cases cases.txt --out "$TMPDIR/measure-vsloop"
   ```

   All 196 cases began. 188 have a compile refusal on the host: 179
   LoopVertex (159 Fail and 20 NotSupported on the board), 8 UniformType
   and 1 constant op. 8 compile (section 3). The output is not committed.
3. **The sources.** Every reference's pair is verbatim from the measure's
   `programs/` (each `.license` sibling names its program). A first draft, made
   before VK-GL-CTS could be fetched, rebuilt all eight vertex shaders from
   the generator's template and the shader library. Six of them were
   byte-equal to the `.qpa`. Two were not, and were drawn again from the verbatim text:
   `uni_inf_break_last` (the draft declared an unused `int i`) and
   `uni_only_continue` (whitespace). Mesa's records for both are the same
   bytes as before.

## 3. The shapes

Groups of the 196 cases (status at 8ff01055) and the reference that holds
each shape:

| group | cases | first refusal on the host (measure at 63c6418d) | reference |
|---|---|---|---|
| constant bound, a guard or `continue` that folds once unrolled: `conditional_{body,break,continue}`, `double_continue`, `mixed_break_continue`, `unconditional_{break,continue}`, `infinite_with_conditional_break`; each of `for`, `while`, `do_while` | 24 Fail | LoopVertex | **new**: `con_cond_break`, `con_mixed`, `con_double_continue`, `con_inf_cond_break` |
| `infinite_with_unconditional_break_{first,last}`, every loop type and count | 18 Fail | LoopVertex | **new**: `uni_inf_break_last` |
| `only_continue`, every loop type and count | 9 Fail | LoopVertex | **new**: `uni_only_continue` |
| `functions.control_flow.return_after_{break,continue}_vertex` | 2 Fail | LoopVertex | **new**: `fn_after_break` |
| `functions.control_flow.return_in_nested_loop_vertex` | 1 Fail | LoopVertex | **new**: `fn_nested` |
| constant `101_iterations` (Mesa keeps it: 101 passes is more than 32) | 3 Fail | LoopVertex | `deqp_loops2_vs_{do,wh}_con_101_iterations` |
| constant `nested`, `nested_sequence`, `nested_tricky_dataflow_{1,2}` | 12 Fail | `do_while`: LoopVertex. `for` and `while`: **compile** on the host (see below) | `deqp_loops2_vs_do_con_nested`, `deqp_loops_vs_nested_const` (MATCHED) |
| uniform and dynamic counts, kept by Mesa: `basic_*_float`, `conditional_*`, `double_continue`, `infinite_with_conditional_break`, `mixed_break_continue`, `nested*`, `unconditional_{break,continue}` | 90 Fail | LoopVertex, except the 8 dynamic `for`/`while` `nested*` cases: UniformType (an `int` uniform, plan #402 `ref-vsunif`) | `deqp_loops2_vs_{for,wh,do}_{uni,dyn}_*`, `deqp_loops_vs_float_uni` |
| `functions.control_flow.{return_in_loop,mixed_return_break_continue}_vertex` 2 Fail, `return_after_loop_sequence_vertex` 1 NS | 2 Fail + 1 NS | LoopVertex | `deqp_loops2_vs_fn_*`, `deqp_loops3_vs_fn_*` |
| `return.return_in_{static,dynamic,infinite}_loop_*_vertex` | 7 Fail | LoopVertex | `deqp_loops2_vs_ret_*` |
| `invariance.{highp,mediump,lowp}.loop_{1,2}` | 6 Fail | LoopVertex | `deqp_loops2_vs_inv_*` |
| `indexing.vector_subscript.vec{2,3,4}_dynamic_loop_subscript_write_*_vertex` | 18 NS | LoopVertex | `deqp_loops2_vs_idx_vec_*` |
| `functions.control_flow.return_in_loop_if_vertex` | 1 Fail | LoopVertex | none: the eight slots are used |
| `struct.uniform.dynamic_loop_nested_struct_array_vertex` | 1 NS | LoopVertex | none: the eight slots are used |
| `struct.uniform.loop_nested_struct_array_vertex` | 1 Fail | a constant operation on a value that is not a finite normal float (plan #402 `ref-vsconst`) | none |

The new references cover **54 Fail** cases. The earlier references cover
139 cases. Three cases have no reference.

**Constant nested `for` and `while` loops have been taken since 8ff01055.**
`{for,while}_constant_iterations.{nested,nested_sequence,nested_tricky_dataflow_1,_2}_vertex`
(8 cases) were refused with LoopVertex on the board at 8ff01055. In the
host measure every shader of those 8 cases compiles and links. The verbatim
`deqp_loops_vs_nested_const` (`nested_sequence_vertex`) is byte-equal (its
test in `tests/mali_deqp_loops.rs` runs). Commit `4aadf854`, which comes
after `8ff01055`, taught `vs_loop_shape` and `put_loop` the counted loop
nested in a counted loop. So these 8 cases should leave the family at
the next board run. Whether they pass there is for that run to say: the
host draws nothing. The `do_while` twins (`deqp_loops2_vs_do_con_nested`)
are still refused.

## 4. What Mesa 26.2.1 does

All eight references were drawn with `tools/mali/deqp_ref.sh <vert> <frag>
tools/mali/testdata/deqp_vsloop_<shape> --instrument shim`. The shim is
Mesa 26.2.1's drm-shim, built in `$CONNECTED_DEPS_DIR/mesa-shim` with
`drm_shim_ref.sh build` from the tarball whose sha256 is in
`third_party/mesa-26.2.1/UPSTREAM`. Two self-checks passed first: the
README's `gm2_cf_ifelse_vs` and `deqp_loops_vs_nested_const` were drawn
again, and both were byte-equal apart from the `trace` name. The shim was
built again later in `$CONNECTED_DEPS_DIR/mesa-shim`, and
`con_cond_break` drawn on it was byte-equal to the committed record. The two
verbatim redraws (section 2) were made on that build. Mesa's
shaderdb line shows **0 loops** in the position, varying and fragment
programs of every reference. **Mesa unrolls or deletes every loop here.**

| shape | dEQP case drawn | what Mesa does | the varying program |
|---|---|---|---|
| `con_cond_break` | `loops.for_constant_iterations.conditional_break_vertex` | unrolled (3 passes). `i == 2` folds, so two `yzwx` rotations are left | 11 words: `LD_ATTR_IMM.v4.f32`, then the three lanes stored as `.zwx` (the rotations folded into the store order) |
| `con_mixed` | `loops.for_constant_iterations.mixed_break_continue_vertex` | unrolled (5 passes). Pass 0 continues and pass 3 breaks: two rotations | the same 11 words as `con_cond_break` |
| `con_double_continue` | `loops.while_constant_iterations.double_continue_vertex` | unrolled. Pass 1 continues: two rotations | the same 11 words |
| `con_inf_cond_break` | `loops.do_while_constant_iterations.infinite_with_conditional_break_vertex` | `do {..} while (true)` with `if (i == 1) break;`: unrolled to two passes | the same 11 words |
| `uni_inf_break_last` | `loops.do_while_uniform_iterations.infinite_with_unconditional_break_last_vertex` (body `do { res = res.yzwx; break; } while (true);`) | the loop ends in `break`, so it is one pass (`wrapper_unroll`): one rotation | 11 words, stored as `.yzw` |
| `uni_only_continue` | `loops.do_while_uniform_iterations.only_continue_vertex` | the loop has no effect and is deleted. The uniform bound is not read | 11 words, `LD_ATTR_IMM.v3.f32` stored as `.xyz` |
| `fn_after_break` | `functions.control_flow.return_after_break_vertex` | inlined, unrolled to 5 negations, folded to `-in0` | 4 words: `LD_ATTR_IMM.f32`, `FADD.f32 r0, r0.neg, -0`, the store |
| `fn_nested` | `functions.control_flow.return_in_nested_loop_vertex` | inlined. The nested loops unroll and the return at `i == 1` folds to `-in0` | the same 4 words as `fn_after_break` |

The position program is the same 24 words in all eight. The fragment
program is the loop cases' `gl_FragColor = vec4(v_color.rgb, 1.0)` (8
words) or the shader library's `isOk` compare (13 words).
`constant_shapes_are_one_program` in the test pins that the four `con_*`
records are equal, and so are the two `fn_*` records.

## 5. What the vertex walk lacks

Every new reference stops in the same place in the vertex walk.
`vs_steps` (`src/shaderc/online/mali/nir.rs:4377`) refuses at `:4417` a
top-level loop that `vs_loop_shape` (`:4316`) does not take. That check
takes a body made only of instructions, one head test `if (c) {} else
break;` and, after the head test, nested loops of the same form. So the
walk unrolls a counted loop with a plain body:
`deqp_loops_vs_{for,while,dowhile}_const` and `deqp_loops_vs_nested_const`
are byte-equal. It refuses each of these:

- an `if` in the body that holds a `break` or a `continue` (every `con_*`);
- no head test, as in `do {..} while (true)` (`con_inf_cond_break`,
  `uni_inf_break_last`);
- a `continue` (`uni_only_continue`);
- the `if`s that the inlined `return` and `break` leave in a function's
  loop (`fn_*`).

None of these needs a kept loop to match Mesa.
The lowering (plan #402 `lower-vsloop`) can be "unroll the constant trip
count, fold the guards, then drop the loop" (as Mesa's `simple_unroll` and
`complex_unroll` do), and "drop a loop with no effect".

- The `break`-last loop is one pass whatever its bound.
- The functions are already inlined by the front end. Their loops have
  constant bounds.

When a lowering takes a shape, it removes the `#[ignore]` and the
`REFUSED` entry and adds the name to `MATCHED` in the same commit.
