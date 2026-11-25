<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the dEQP-GLES2 vertex-shader loops that Mesa keeps

Plan #402 item `ref-vsloop2` (issue #20, ROADMAP stage 5). This item adds
references only. It changes no compiler code. The refusal is
`MaliError::LoopVertex`, "Valhall: a loop in a vertex shader is not lowered
yet". `docs/notes/mali-deqp-vsloop.md` covers step 1 of the lowering,
loops that Mesa unrolls or deletes. This note covers step 2, the loops that
Mesa keeps.

The references are `tools/mali/testdata/deqp_vsloop2_*`. The test is
`tests/mali_deqp_vsloop2.rs`:

```text
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsloop2
```

**HOST-ONLY.** Every count here comes from the board slices or from a
host-only compile. Nothing here ran on the NanoPC-T6.

## 1. The measure

1. **The case list.** The cases of the four slices
   `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv` whose
   refusal text holds "a loop in a vertex shader": 196 cases (176 Fail and
   20 NotSupported on the board).

   ```sh
   cat tools/cts/log/2026-11-24-t6-gles2-S[1234]-8ff01055/refusals.tsv |
       grep -F 'a loop in a vertex shader' | cut -f1 | sort -u > cases.txt
   tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS-host" \
       --slice tools/cts/log/2026-11-24-t6-gles2-S1-8ff01055 ... --slice ...-S4-8ff01055 \
       --cases cases.txt --out "$TMPDIR/m"
   ```

   This ran at the tip `c70d4813` with VK-GL-CTS `93bca01861b0`: a shallow,
   sparse clone without `doc/`, with `cts-yakogl-target.patch` and
   `external/fetch_sources.py`. All 196 cases began, and two processes
   exited 1 and were resumed. On the host, **187** cases still stop at
   LoopVertex. One stops at a constant op on a value that is not a finite
   normal float (`struct.uniform.loop_nested_struct_array_vertex`, plan
   #402 `ref-vsconst`). **8** compile: the constant `for` and `while`
   `nested*` cases, which `4aadf854` took (`mali-deqp-vsloop.md` §3). Since
   the measure at `63c6418d`, the 8 dynamic nested `for`/`while` cases have
   moved from UniformType to LoopVertex. The measure's output is not
   committed.
2. **Kept or not, by Mesa.** The 187 cases have 181 distinct programs (the
   measure's `programs/`). Each program was drawn once on Mesa 26.2.1's
   drm-shim (`tools/mali/drm_shim_ref.sh run <work> <vert> <frag> <out>
   --uniform-values`, `$CONNECTED_DEPS_DIR/mesa-shim`), and the shaderdb
   line's `N loops` was read. A case is **kept** when one of its programs
   has a loop left in its position or varying program. That is
   **110 cases** (90 Fail + 20 NotSupported). In the other **77** (77
   Fail), Mesa keeps no loop: that is step 1. Only one program keeps a loop
   in its position program (`1 loops`); every other loop is in the varying
   program.

   The planner's figure was 116 cases left after step 1. The 110 here is
   Mesa's own split. The 6 extra cases are most likely ones that Mesa
   unrolls but `proto-vsloop`'s step 1 does not take. That cannot be
   checked here: the prototype is not in the tree.

## 2. The shapes (HOST-ONLY)

Every one of the 196 cases is in exactly one row. The counts sum to 196.
"F" and "NS" are the board's status at `8ff01055`. "Site" is where the tip
refuses, by reading `src/shaderc/online/mali/nir.rs` at `c70d4813` (not
instrumented):

- **4829**: `vs_steps` puts `Step::Refuse(LoopVertex)` for a top-level loop
  that `vs_loop_shape` (`:4578`) does not take. These are loops with an
  `if` holding a `break` or `continue`, a `do_while` (no head test), or
  `while (true)`.
- **kept walk**: `vs_loop_shape` takes the loop (a `for`/`while` with a head
  test and a plain body), and the kept-loop walk (`vs_kept_begin`/
  `vs_kept_end`, `:5151`-`:5310`) refuses it. One such refusal is a
  mediump float carried from an attribute (`:5178`: `res = coords` from
  `a_coords`).

| # | shape | cases | board | Mesa | site | reference |
|---|---|---:|---|---|---|---|
| 1 | constant bound with a guard, `infinite_*`, `only_continue`, `unconditional_*` (`loops.*_constant_iterations`, not `101` and not the 8 below) | 37 | 37 F | unrolled | 4829 | step 1: `deqp_vsloop_con_*`, `deqp_loops_vs_{for,while,dowhile}_const`, `deqp_loops2_vs_do_con_nested` |
| 2 | uniform and dynamic `infinite_with_unconditional_break_{first,last}`, `only_continue`, `unconditional_break` | 24 | 24 F | one pass, or deleted | 4829 | step 1: `deqp_vsloop_uni_inf_break_last`, `deqp_vsloop_uni_only_continue`, `deqp_loops2_vs_for_uni_unconditional_break` |
| 3 | `functions.control_flow.{return_after_break,return_after_continue,return_in_loop,return_in_loop_if,return_in_nested_loop}` | 5 | 5 F | inlined, unrolled | 4829 | step 1: `deqp_vsloop_fn_*`, `deqp_loops2_vs_fn_return_in_loop` (none for `return_in_loop_if`) |
| 4 | `invariance.{highp,mediump,lowp}.loop_{1,2}` | 6 | 6 F | unrolled | 4829 | step 1: `deqp_loops2_vs_inv_*` |
| 5 | `return.return_in_{static_loop_*,dynamic_loop_always,infinite_loop}` | 5 | 5 F | unrolled | 4829 | step 1: `deqp_loops2_vs_ret_{static,static_dyn,static_loop_always,static_loop_never,infinite}` |
| 6 | constant `101_iterations` | 3 | 3 F | **kept** (101 > 32 passes) | 4829 / kept walk | `deqp_loops2_vs_{do,wh}_con_101_iterations` |
| 7 | uniform `basic_{highp,mediump,lowp}_float` | 9 | 9 F | **kept** | 4829 (`do_while`), kept walk | `deqp_loops2_vs_do_uni_basic_{highp,mediump}_float`, `deqp_loops2_vs_for_uni_basic_lowp_float`, `deqp_loops_vs_float_uni` |
| 8 | dynamic `basic_{highp,mediump,lowp}_float` | 9 | 9 F | **kept** | 4829 (`do_while`), kept walk | `deqp_loops2_vs_{do,wh}_dyn_basic_highp_float`, `deqp_loops2_vs_for_dyn_basic_mediump_float` |
| 9 | uniform and dynamic `conditional_body` | 6 | 6 F | **kept** | 4829 | `deqp_loops2_vs_{do,for,wh}_{uni,dyn}_conditional_body` |
| 10 | uniform and dynamic `conditional_break` | 6 | 6 F | **kept** | 4829 | `deqp_loops2_vs_{do,for,wh}_{uni,dyn}_conditional_break` |
| 11 | uniform `conditional_continue` | 3 | 3 F | **kept** | 4829 | `deqp_loops2_vs_{do,for,wh}_uni_conditional_continue` |
| 12 | **dynamic `conditional_continue`** | 3 | 3 F | **kept**, 1 loop | 4829 | **new**: `deqp_vsloop2_for_dyn_conditional_continue` |
| 13 | uniform `double_continue` | 3 | 3 F | **kept** | 4829 | `deqp_loops2_vs_wh_uni_double_continue` |
| 14 | **dynamic `double_continue`** | 3 | 3 F | **kept**, 1 loop | 4829 | **new**: `deqp_vsloop2_do_dyn_double_continue` |
| 15 | uniform `infinite_with_conditional_break` | 3 | 3 F | **kept** | 4829 | `deqp_loops2_vs_do_uni_infinite_with_conditional_break` |
| 16 | **dynamic `infinite_with_conditional_break`** | 3 | 3 F | **kept**, 1 loop | 4829 | **new**: `deqp_vsloop2_wh_dyn_infinite_with_conditional_break` |
| 17 | uniform `mixed_break_continue` | 3 | 3 F | **kept** | 4829 | `deqp_loops2_vs_for_uni_mixed_break_continue` |
| 18 | **dynamic `mixed_break_continue`** | 3 | 3 F | **kept**, 1 loop | 4829 | **new**: `deqp_vsloop2_for_dyn_mixed_break_continue` |
| 19 | uniform and dynamic `nested` | 6 | 6 F | **kept**, 2 loops | 4829 (`do_while`), kept walk | `deqp_loops2_vs_{do,for,wh}_uni_nested`, `deqp_loops2_vs_for_dyn_nested` |
| 20 | uniform and dynamic `nested_sequence` | 6 | 6 F | **kept**, 3 loops | 4829 (`do_while`), kept walk | `deqp_loops2_vs_for_uni_nested_sequence`, `deqp_loops2_vs_{do,wh}_dyn_nested_sequence` |
| 21 | uniform `nested_tricky_dataflow_1` | 3 | 3 F | **kept**, 2 loops | 4829 (`do_while`), kept walk | `deqp_loops2_vs_for_uni_nested_tricky_dataflow_1` |
| 22 | **dynamic `nested_tricky_dataflow_1`** | 3 | 3 F | **kept**, 2 loops | 4829 (`do_while`), kept walk | **new**: `deqp_vsloop2_do_dyn_nested_tricky_dataflow_1` |
| 23 | uniform `nested_tricky_dataflow_2` | 3 | 3 F | **kept**, 2 loops | 4829 (`do_while`), kept walk | `deqp_loops2_vs_for_uni_nested_tricky_dataflow_2` |
| 24 | **dynamic `nested_tricky_dataflow_2`** | 3 | 3 F | **kept**, 2 loops | 4829 (`do_while`), kept walk | **new**: `deqp_vsloop2_wh_dyn_nested_tricky_dataflow_2` |
| 25 | uniform `unconditional_continue` | 3 | 3 F | **kept** | 4829 | `deqp_loops2_vs_wh_uni_unconditional_continue` |
| 26 | **dynamic `unconditional_continue`** | 3 | 3 F | **kept**, 1 loop | 4829 | **new**: `deqp_vsloop2_for_dyn_unconditional_continue` |
| 27 | `functions.control_flow.{mixed_return_break_continue,return_after_loop_sequence}_vertex` | 2 | 1 F + 1 NS | **kept** | 4829 | `deqp_loops2_vs_fn_{mixed,after_seq}`, `deqp_loops3_vs_fn_{mixed,after_seq}` |
| 28 | `indexing.vector_subscript.vec{2,3,4}_dynamic_loop_subscript_write_*_vertex` | 18 | 18 NS | **kept** | 4829 / kept walk | `deqp_loops2_vs_idx_vec_*` |
| 29 | `return.return_in_dynamic_loop_{dynamic,never}_vertex` | 2 | 2 F | **kept** | 4829 | `deqp_loops2_vs_ret_{dyn,dynamic_loop_never}` |
| 30 | **`struct.uniform.dynamic_loop_nested_struct_array_vertex`** | 1 | 1 NS | **kept**, 1 loop (the inner loop unrolled) | kept walk | **new**: `deqp_vsloop2_struct_dyn_nested_array` |
| 31 | constant `{for,while}` `nested`, `nested_sequence`, `nested_tricky_dataflow_{1,2}` | 8 | 8 F | unrolled | none: they **compile** on the host | `deqp_loops_vs_nested_const` (MATCHED) |
| 32 | `struct.uniform.loop_nested_struct_array_vertex` | 1 | 1 F | - | another refusal: a non-finite constant op | none (plan #402 `ref-vsconst`) |
| | **total** | **196** | 176 F + 20 NS | | | |

Rows 1-5 (77 cases) are step 1. Rows 6-30 (110 cases, 90 F + 20 NS) are
the loops Mesa keeps, so step 2 (`lower-vsloop2`) must lower them. Of
those, 88 cases are shapes of earlier references and **22 cases (21 F + 1
NS) are the 8 new shapes**.

"Dynamic" and "uniform" are different shapes. A uniform count is
`i < ui_three`: a uniform bound, so the loop is uniform. A dynamic count
is `i < one*ui_three` with `mediump int one = int(a_one + 0.5)`, read from
an attribute, so the bound is per vertex and the loop diverges. The earlier
references hold the uniform twin of each of the seven new dynamic shapes.
Each new reference was drawn for one loop type (`for`, `while` or
`do_while`); the other two are in the same row. Where a body form already
had a reference, the three new loops were spread over the loop types.

## 3. What Mesa 26.2.1 does

All eight references were drawn with `tools/mali/deqp_ref.sh <vert> <frag>
tools/mali/testdata/deqp_vsloop2_<shape> --instrument shim`. The shim was
built in `$CONNECTED_DEPS_DIR/mesa-shim` with `drm_shim_ref.sh build` from
the tarball of `third_party/mesa-26.2.1/UPSTREAM`. First the self-check:
`deqp_vsloop_con_cond_break`, drawn again, was byte-equal to the committed
`.expect.json`. Every source is verbatim from the measure's `programs/`:
`cmp` was run against each one, and each `.license` names its program.

The position program has 24 instructions in every reference. The fragment
program is `gl_FragColor = vec4(v_color.rgb, 1.0)` (8 instructions), or
for the struct case the `v_color` passthrough (7). The loop is in the
varying program:

| shape | dEQP case drawn | Mesa's varying program | loop form (encoded branches) |
|---|---|---|---|
| `for_dyn_conditional_continue` | `loops.for_dynamic_iterations.conditional_continue_vertex` | 34 instrs, 1 loop | head `BRANCHZ.reconverge r4.h0, +10` out of the loop; the `continue` has no branch of its own; back edge `BRANCHZ.eq.reconverge 0x0, -13` |
| `do_dyn_double_continue` | `loops.do_while_dynamic_iterations.double_continue_vertex` | 34 instrs, 1 loop | the `do_while` test stays at the bottom: the `i == one*ui_two` guard is four `MUX.v2i16` selects, then `++i`, the exit `BRANCHZ.reconverge r5.h0, +3` and the two `MKVEC.v2i16` of the latch, back edge `-13` |
| `wh_dyn_infinite_with_conditional_break` | `loops.while_dynamic_iterations.infinite_with_conditional_break_vertex` | 29 instrs, 1 loop | `while (true)` with its `break` as the exit test: `BRANCHZ.reconverge r2.h0, +4`, back edge `-8` |
| `for_dyn_mixed_break_continue` | `loops.for_dynamic_iterations.mixed_break_continue_vertex` | 35 instrs, 1 loop | the head test and the `i == 3` break folded into one exit `BRANCHZ.reconverge r4.h0, +8`; the `i == 0` continue is a select; back edge `-14` |
| `do_dyn_nested_tricky_dataflow_1` | `loops.do_while_dynamic_iterations.nested_tricky_dataflow_1_vertex` | 37 instrs, 2 loops | inner loop first: `+3` / `-8`, then the outer exit `+1` and back edge `-16` |
| `wh_dyn_nested_tricky_dataflow_2` | `loops.while_dynamic_iterations.nested_tricky_dataflow_2_vertex` | 37 instrs, 2 loops | outer exit `+12`, inner exit `+3`, inner back edge `-8`, outer back edge `-16` |
| `for_dyn_unconditional_continue` | `loops.for_dynamic_iterations.unconditional_continue_vertex` | 29 instrs, 1 loop | the trailing `continue` leaves no branch: exit `+4`, back edge `-8` |
| `struct_dyn_nested_array` | `struct.uniform.dynamic_loop_nested_struct_array_vertex` | 110 instrs, 1 loop | outer `ui_two` loop kept (exit `+79`, back edge `-81`); the inner `ui_three` loop unrolled to 3 passes, bounded by `T b[3]`, its guards are the conditional `BRANCHZ.eq.reconverge r3/r7` (`+69`, `+45`, `+23`); there are three more forward `BRANCHZ.eq.reconverge 0x0` (`+2`, `+2`, `+4`) |

Every kept loop has one form. The counter starts in the block before the
loop (`IADD_IMM.i32.reconverge r3, 0x0, #0x0`, where the header begins).
The single exit is `BRANCHZ.reconverge <cond>.h0`, on the 16-bit boolean of
an `ICMP_*.m1`, with a forward offset past the back edge. It sits where the
source leaves the loop: at the top for `for` and `while`, after the body's
selects and the increment for `do_while`. The loop ends in an
unconditional back edge, `BRANCHZ.eq.reconverge 0x0` with a negative
offset. A `continue` or a `break` guard never gets a branch of its own:
the guarded statements become `MUX.v2i16` selects (`mixed_break_continue`:
the `i == 0` continue), and the `break` conditions are ORed into the one
exit test (`ICMP_AND` and `LSHIFT_OR` before the exit). Nested loops have
one exit and back-edge pair each. The 32-bit counter (`IADD_IMM.i32`)
is carried as it is. The mediump `vec4 res` is carried at fp16: it is
narrowed before the loop with `V2F32_TO_V2F16`, rebuilt in the latch with
`MKVEC.v2i16`, and widened after it with `F16_TO_F32`. That is the carried
mediump float that the kept walk refuses today.

The struct reference is drawn with its uniform members set
(`gl_ref: uniform vec2 s[1].b[2].b[1] at 23`, ...). Its FAU values follow
`README-deqp_ref.md` "Struct uniforms". `tests/mali_ref`'s `differences`
takes that layout into account only for the references listed in
`STRUCT_VALUES`. When `lower-vsloop2` takes this reference, it may need to
add the reference there, or compare it as other struct references are
compared.

## 4. For step 2

- At the tip, every new reference refuses with LoopVertex, and
  `refusals_pinned` pins that text. Per the plan, after step 1 they refuse
  at the refusing arm of `Step::JIf`. A lowering that moves a pin moves it
  in the same commit. When it makes a reference byte-equal, it removes the
  `#[ignore]` and the `REFUSED` entry and adds the name to `MATCHED`.
- The dynamic shapes carry a mediump `res` read from `a_coords` around the
  loop. The kept walk refuses such a value at `vs_kept_begin` (`:5178`),
  whatever the body is.
- Other cases with no reference: `functions.control_flow.return_in_loop_if_vertex`
  (step 1, 1 F) and `struct.uniform.loop_nested_struct_array_vertex` (1 F,
  stopped earlier by the constant-op refusal).
