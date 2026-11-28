<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the dEQP-GLES2 vertex-shader `sequence` and `function_call_inout` loops

Plan #402 item `ref-vsloop3` (issue #20, ROADMAP stage 5). This item adds
references only. It changes no compiler code. The refusal is
`MaliError::LoopVertex`, "Valhall: a loop in a vertex shader is not lowered
yet". Two vertex loop shapes are in neither design:
`docs/notes/mali-deqp-vsloop.md` (step 1, the loops Mesa unrolls) and
`docs/notes/mali-deqp-vsloop2.md` (step 2, the loops Mesa keeps). They are
`loops.*.sequence_vertex` and `loops.*.function_call_inout_vertex`.
`docs/notes/mali-loops2-lowering.md` covers them as parts of its steps LP7
and LP6. This note gives both shapes complete reference sets and names the
refusal site of each at the tip.

The references are `tools/mali/testdata/deqp_vsloop3_*`. The test is
`tests/mali_deqp_vsloop3.rs`: a byte-compare through `tests/mali_ref` per
reference, each one `#[ignore]`d with its refusal, and a non-ignored
`refusals_pinned` test that holds each refusal's exact text.

## 1. The cases

All 12 vertex cases of the two shapes (`{for,while,do_while}` x
`{uniform,dynamic}` iterations) are Fail on the board at `8ff01055`. The
`constant_iterations` twins pass. None of the 12 appear among the 196 VS loop
refusals of `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv`,
because at `8ff01055` they stopped earlier at a vertex uniform refusal:

| board refusal at `8ff01055` | cases |
|---|---:|
| "a uniform other than a float, vec2, vec3, vec4 or mat4" (the `dynamic` ones) | 6 |
| "a vertex shader uniform that is not highp (Mesa's fp16 uniform), other than ..." (the `uniform` ones) | 6 |

`lower-vsunif` lowered both of those refusals, so the 12 cases now reach the
loop walk. They are new cases in the VS loop count that criterion c4 measures
(105 or fewer). Their six `function_call_return` twins took the same path,
but they compile at the tip.

HOST-ONLY at `c28470db`: each of the 12 programs was compiled through
`tests/mali_ref`'s `compile_pair`, and each one gives the VS loop refusal.
The sources are rebuilt from `es2fShaderLoopTests.cpp`'s
`createSpecialLoopCase` (VK-GL-CTS 05ca62d, its templates and specialisation
parameters line for line). They are not from a `.qpa`: no host deqp-gles2
build was made for this item. The same rebuild gives the six existing
`deqp_loops2_vs_*_{sequence,function_call_inout}` sources byte for byte. The
refusal site was found by instrumenting a scratch copy (a `line!()` print at
every `LoopVertex`). That instrumentation was never committed.

## 2. The shapes and their references

| shape | case | reference | Mesa 26.2.1, varying program | site at `c28470db` |
|---|---|---|---|---|
| `sequence` | `for_uniform_iterations.sequence_vertex` | **new** `deqp_vsloop3_for_uni_sequence` | 2 loops kept, 33 instrs | `nir.rs:5573` (`vs_kept_begin`) |
| | `for_dynamic_iterations.sequence_vertex` | `deqp_loops2_vs_for_dyn_sequence` | 2 loops, 39 instrs | same |
| | `while_uniform_iterations.sequence_vertex` | `deqp_loops2_vs_wh_uni_sequence` | 2 loops, 32 instrs | same |
| | `while_dynamic_iterations.sequence_vertex` | **new** `deqp_vsloop3_wh_dyn_sequence` | 2 loops, 38 instrs | `nir.rs:5573` |
| | `do_while_uniform_iterations.sequence_vertex` | **new** `deqp_vsloop3_do_uni_sequence` | 2 loops, 32 instrs | `nir.rs:5573` |
| | `do_while_dynamic_iterations.sequence_vertex` | `deqp_loops2_vs_do_dyn_sequence` | 2 loops, 38 instrs | same |
| `function_call_inout` | `for_uniform_iterations.function_call_inout_vertex` | `deqp_loops2_vs_for_uni_function_call_inout` | `func` inlined, 1 loop kept, 24 instrs | `nir.rs:5808` (`vs_kept_end`) |
| | `for_dynamic_iterations.function_call_inout_vertex` | **new** `deqp_vsloop3_for_dyn_function_call_inout` | inlined, 1 loop, 29 instrs | `nir.rs:5808` |
| | `while_uniform_iterations.function_call_inout_vertex` | **new** `deqp_vsloop3_wh_uni_function_call_inout` | inlined, 1 loop, 27 instrs | `nir.rs:5808` |
| | `while_dynamic_iterations.function_call_inout_vertex` | `deqp_loops2_vs_wh_dyn_function_call_inout` | inlined, 1 loop, 32 instrs | same |
| | `do_while_uniform_iterations.function_call_inout_vertex` | `deqp_loops2_vs_do_uni_function_call_inout` | inlined, 1 loop, 24 instrs | same |
| | `do_while_dynamic_iterations.function_call_inout_vertex` | **new** `deqp_vsloop3_do_dyn_function_call_inout` | inlined, 1 loop, 29 instrs | `nir.rs:5808` |
| **total** | 12 cases (12 Fail) | 6 new, 6 earlier | | |

The instruction and loop counts are the shaderdb line of the
`MESA_SHADER_VARYING` program (`BIFROST_MESA_DEBUG=shaderdb`). Every
position program has 24 instructions and 0 loops, because the loops feed only
`v_color`. Mesa unrolls nothing in these shapes, since every bound is a
uniform (`ui_N`) or a per-vertex `one*ui_N`. "Same" in the site column means
the earlier reference's case refuses at the same site. Their pins in
`tests/mali_deqp_loops2.rs` hold the same refusal text.

## 3. What Mesa does

All six new references were drawn with
`tools/mali/deqp_ref.sh <vert> <frag> tools/mali/testdata/deqp_vsloop3_<name> --instrument shim`.
The shim is `$CONNECTED_DEPS_DIR/mesa-shim`, built with
`drm_shim_ref.sh build` from the 26.2.1 tarball (sha256 as in
`third_party/mesa-26.2.1/UPSTREAM`). As a self-check, three earlier
references drawn again were byte-equal to their committed `.expect.json`:
`deqp_loops2_vs_wh_uni_sequence`,
`deqp_loops2_vs_for_uni_function_call_inout` and
`deqp_vsloop2_for_dyn_unconditional_continue`. Each `.nir.sha` is from
`tools/mali/nir_refs.sh` (`deqp_loops2_vs_wh_uni_sequence` came out `SAME`).

- **`sequence`: kept, two loops one after the other.** Each loop has the
  vsloop2 form: one exit test `BRANCHZ.reconverge` forward, the mediump
  `res` carried as two `MKVEC.v2i16` fp16 pairs, and an unconditional back
  edge `BRANCHZ.eq.reconverge 0x0`. The second loop starts on the
  first loop's counter (`for (; i < ...)`, `while (i++ < ...)`,
  `do ... while (++i < ...)`) and on its fp16 pairs. Between the two
  loops Mesa moves the counter and one pair through registers
  (`deqp_vsloop3_for_uni_sequence`: `MOV.i32 r1, r2^`,
  `MOV.i32.reconverge r2, r3^`, `MOV.i32 r3, r2^`). After the second
  loop comes the `F16_TO_F32` tail and the three `STORE.i32.estream`.
- **`function_call_inout`: inlined, then one kept loop.** After
  `nir_inline_functions` the `inout` call is `res = res.yzwx`, and the loop
  is the plain kept loop of `deqp_loops_vs_for_uni` (24 instructions for
  `for`/`do_while` uniform). The `while` form and the `dynamic` bounds carry
  a second counter register (`IADD_IMM.i32 r3, r5, #0x1`, `MOV.i32 r2, r5^`
  in `deqp_vsloop3_wh_uni_function_call_inout`, 27 instructions).

## 4. What a lowering needs

- **`sequence`** (`vs_kept_begin`, `nir.rs:5573`): the kept-loop walk
  takes a carried pair only when each lane is a 32-bit value before the
  loop. The second loop of a sequence sees the first loop's fp16 pairs, so
  it refuses. The lowering must:
  - accept a pair that an earlier kept loop left as the second loop's entry
    value, with no repacking;
  - give the vertex walk's flat `Step` list a second `KeptLoop` record after
    the first;
  - carry the shared counter out of the first loop into the second, with
    Mesa's register moves between the loops.

  This is LP7's "after the first" half in `mali-loops2-lowering.md`. Nested
  loops are its other half.
- **`function_call_inout`** (`vs_kept_end`, `nir.rs:5808`): at the latch,
  the carried pair's lanes are the phis themselves, or are not both fp16.
  The inlined `inout` copy-back leaves the local as the walk found it, so
  `(v0, v1) == (Lane(id, 0), Lane(id, 1))` holds. The lowering must write the
  inlined callee's parameter back into the caller's local before the kept
  walk reads the latch, so that the latch value is the rebuilt pair (LP6's
  `inout` item). The three `dynamic` forms also need Mesa's mediump
  `int(a_one + 0.5)` narrowing in the vertex walk, as for every vertex `dyn`
  form (LP6, and `deqp_vsloop2_for_dyn_unconditional_continue`).

Counts for criterion c4: lowering `sequence` frees 6 cases, and lowering
`function_call_inout` frees 6. That is 12 of the VS loop refusals at the
tip, all Fail on the board.
