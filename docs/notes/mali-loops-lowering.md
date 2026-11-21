<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall v10): function-level design of the loop lowering (`LoopMediump`, `GuardMediump`, `LoopForm`, `LoopVertex`)

Plan #387 item loops-design. A design: nothing under `src/`, `tests/` or
`tools/mali/testdata/` changed. It turns the eleven steps S1-S11 that
[`mali-deqp-loops.md`](mali-deqp-loops.md) section 9.5 ranked into function-level
work for the two items that implement them: **loops-lower-1** (the
fragment steps S4, S3, S5, S6, S2-fs, S7: 296 cases) and **loops-lower-2**
(the vertex steps S1, S2-vs, S8, S9, S10, S11: 241 cases). The case counts
are section 9.2's (537 = 281 `LoopMediump` + 12 `GuardMediump` + 3
`LoopForm` + 241 `LoopVertex`), HOST-ONLY: a compile or link outcome of
`deqp-gles2` on the host, never a render verdict. Tip `5b3fb8c2`.

Read with it: [`mali-deqp-loops.md`](mali-deqp-loops.md) (the shapes, the
references, section 9.5 the step table), [`mali-loop-break.md`](mali-loop-break.md)
and [`mali-int-loops.md`](mali-int-loops.md) (how Valhall words of a kept loop
read), [`mali-vs-loops.md`](mali-vs-loops.md), [`mali-control-flow.md`](mali-control-flow.md),
[`mali-loop-board-failures.md`](mali-loop-board-failures.md) (the hidden pass
counter every kept loop carries, a deliberate deviation from Mesa).

## 0. What was measured and what was only read

- **Measured** (this item): the words of the 153 `tools/mali/testdata/deqp_loops_*` and
  `deqp_loops2_*` references (the `expect.json` words of `vs.position`,
  `vs.varying`, `fs.fragment`) through Mesa 26.2.1's own Valhall
  disassembler, `tools/mali/va_disasm_ref.c` built from the vendored
  `disasm.py` and `ISA.xml` as `tools/mali/va_disasm_ref.sh` does. The script
  recompiles the C file at each call, so the build was done once into
  `$CONNECTED_DEPS_DIR/loopsd` and the 153 fixtures disassembled with that
  binary (the input a fixture line wants is the word's 8 bytes in memory
  order, which is the `expect.json` word reversed). No Mesa build was needed and
  no reference was redrawn. Every "words" excerpt below is that disassembly,
  trimmed to the loop; the register numbers are Mesa's.
- **Read, not run**: Mesa 26.2.1's `nir_opt_loop_unroll.c`,
  `nir_loop_analyze.c`, `nir_opt_dead_cf.c` (tree
  `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`), and the compiler's code at the
  tip. Statements about why Mesa chose a form are marked **(read)** when the
  words do not prove them.
- **Not settled here**: the host forecast after each step (the next refusal of a
  case that leaves its loop text is not measured: section 9.5 says the same);
  the board's pass count belongs to the board rerun.

Differences from section 8/9.5 that matter because the code moved since
`008306cb`:

1. The vertex walk **does** have a loop now (plan #367, `011bade0`): `nir.rs`
   `vs_loop_shape`, `Step::LoopBegin/Head/LoopEnd`, `VsLoop::{Unroll,Kept}`,
   `vs_kept_begin/head/end` and `variant_dce`. It takes **one** loop of
   `instructions + one head test` with 32-bit carried lanes and an integer
   test. Section 8.4(a)'s "the vertex block emitter is the one large new piece"
   is therefore smaller: the emitter exists, the missing parts are 16-bit
   phis, other bodies and nesting (section 5).
2. Integer values on the fragment side exist: `Fs::int_lane` has `Ieq` of two
   run-time ints (`nir_fs.rs:2388`, `:3429`), `Imul`, `f2i32` as `F16_TO_S32.rtz`
   (`:2253`). So the `ICMP_OR.u32.eq` of a counter and a uniform and the
   dynamic bound's `IMUL.i32` are not new in S3/S5.
3. `mediump` ints in the **vertex** stage still stop at the `UniformType` text
   (97 cases hide behind it, section 9.1 of the loops note; item
   intval-vs16/vs16-design owns it). S8, S9, S10 count only what leaves
   `LoopVertex` first; the 97 come on top when that item lands.

## 1. What Mesa 26.2.1 emits, per step

Words are `fs.fragment` unless `vs.varying` is said. Notation: `rP`, `rQ` are
the two 16-bit-pair registers of the carried `mediump vec4`, `rC` the 32-bit
counter, `rT` the test mask. The shapes are dEQP's (loops note section 2.1).

### 1.1 Common to every loop Mesa keeps (S3, S5, S6, S7, S8-S11)

`deqp_loops_fs_for_uni` (`for (int i = 0; i < ui_three; i++) res = res.yzwx;`,
17 instructions, 1 loop):

```
 1 LD_VAR_BUF_IMM.f32.slot0.v4 @r0:r1:r2:r3, r61^, index:0x0      ; the varying
 2 V2F32_TO_V2F16.discard r0, r0^, r1^                              ; narrow to two vec2 f16
 3 V2F32_TO_V2F16 r1, r2^, r3^
 4 IADD_IMM.i32.reconverge r2, 0x0, #0x0                            ; counter init, opens the loop
 5 MOV.i32 r3, r1^                                                  ; the copy of the pair the body reads old
 6 ICMP_OR.s32.ge.m1 r1, r2, u2.w0, 0x0                             ; header test, bound = FAU word
 7 BRANCHZ.reconverge r1^.h0, offset:4                              ; non-zero = leave
 8 MKVEC.v2i16 r1, r3.h1, r0.h0                                     ; res.yzwx: two pair rebuilds
 9 MKVEC.v2i16 r0, r0^.h1, r3^.h0
10 IADD_IMM.i32 r2, r2^, #0x1                                       ; counter in place
11 BRANCHZ.eq.reconverge 0x0, offset:-7                             ; back edge, unconditional
12 NOP.wait0126
13 ATEST ... ; MKVEC.v2i16.wait r1, r3^.h0, u2.w1.h0 ; BLEND ...    ; epilogue
```

What it fixes:

- **Block structure** (`emit_loop`, bifrost_compile.c:3738-3769): pre-header
  (the counter init carries `.reconverge`: it opens the loop), a header
  (test and `BRANCHZ` out), the body, the latch (`IADD_IMM`, back edge
  `BRANCHZ.eq.reconverge 0x0`), then the exit block. The exit branch's target
  is the instruction after the back edge. A **nested** loop is the same inside
  the outer body (`fs_for_uni_nested`: the inner `IADD_IMM.i32.reconverge` is
  the inner pre-header, the outer latch `IADD_IMM r2` and `MOV.i32 r1, r4^`
  copy follow the inner back edge, then the outer back edge, offset -13).
- **Where the counter lives**: a 32-bit register from `IADD_IMM #0` up, in place
  (`IADD_IMM.i32 r2, r2^, #0x1`); a `mediump int` is **not** narrowed. The
  test is `ICMP_OR.s32.ge.m1` against the FAU word of the `int` uniform
  (`u2.w0`: uniforms are 32-bit words, in declaration order, here `ui_three` and
  the colour constant `u2.w1`). A **dynamic** bound (`one * ui_n`, `one =
  int(v_one + 0.5)`) is `LD_VAR_BUF_IMM` of the varying, `FADD.f32 r0.h0` (narrow),
  `FADD.v2f16 r0, r0.h00, 0x38000000.h11` (+0.5), `F16_TO_S32.rtz r0, r0.h0`
  before the loop and **`IMUL.i32 r2, u2.w0, r0` in the header** (not hoisted:
  `deqp_loops2_fs_for_dyn_basic_mediump_int`, 23 instructions against 17).
- **The carried `mediump vec4` is two 16-bit `vec2` phis** (`r0` = x,y; `r1`/`r3` =
  z,w); the body rebuilds each pair from halves with `MKVEC.v2i16`. The old
  second pair is copied (`MOV.i32 r3, r1^`) because the first rebuild writes
  `r1`; this is the out-of-SSA copy of `bi_out_of_ssa` (bi_ra.c:1022-1043)
  and its order is the register allocator's, not the walk's.
- **A float counter** (F7): `IADD_IMM.i32.reconverge r2, 0x0, #0x2A66` (0.05 in f16
  low half), `FCMP_OR.v2f16.ge.m1 r1, r2.h00, u2.w1.h00, 0x0`, step
  `FADD.v2f16 r2, r2^.h00, u2.w0.h00` (`deqp_loops_fs_float_uni`). In the vertex
  stage a highp float counter is fp32: `FCMP_OR.f32.ge.m1`, `FADD.f32`, init
  `#0x3D4CCCCD` (`deqp_loops_vs_float_uni`).
- **The bound a select of a bool uniform** (F3, `deqp_loops_fs_select`): `MOV.i32
  r1, u2.w1; CSEL.u32.eq r1, u2.w0, 0x0, 0x0, r1^` in the **header** (the bool
  word is the first source, 0 the second, then the two arms), then the usual
  `ICMP_OR`. 19 instructions.
- **A constant bound past 32 passes** (F4, `deqp_loops_fs_101`, `deqp_loops2_fs_*_101_iterations`):
  kept, and the words of a `do`/`while` loop: the constant 101 is a FAU word
  (`u2.w0` = 0x65), the counter's `IADD_IMM` comes **before** the test, and the
  exit takes the pre-rotation pair, so a third `MKVEC.v2i16` follows the loop
  (`deqp_loops_fs_dowhile_uni`, 18 instructions: Mesa peels the first,
  always-true test, which makes a `for` rotate like a `do`).
- **The do-while** (test after the body): the same blocks with the counter's
  `IADD_IMM` first, the test `ICMP_OR.s32.ge.m1 r1, r2, u2.w0` next, the exit
  `BRANCHZ ... offset:3` over the rebuilds, and the third `MKVEC.v2i16` after
  the loop (words 4-18 of `deqp_loops_fs_dowhile_uni`).

In a **vertex** shader the loop is the same program in `vs.varying`; its tail
is `F16_TO_F32` per stored lane and the stores (`deqp_loops_vs_for_uni`: 24
instructions; `vs.position` stays the 24-word pass-through). The kept-loop
words are word for word the fragment's (`IADD_IMM.i32.reconverge`, `MOV.i32`,
`ICMP_OR.s32.ge.m1 ... u5.w1`, `BRANCHZ`, two `MKVEC.v2i16`, `IADD_IMM`, back
edge): the bound is a vertex FAU word (`u5.w1` after the position program's
`u0..u5`). One exception is measured: `vs_idx_vec_w_dynsub` has a loop in
`vs.position` too (5 words: the counter and its test only; 29 instructions), see S11.

### 1.2 S4: loops Mesa deletes or unrolls (decision only), fragment, 22 cases

- **Deleted**: `empty_body`, `only_continue`
  (`deqp_loops2_fs_for_uni_empty_body`: 8 instructions, no `BRANCHZ`). Mesa
  removes the loop in `nir_opt_dead_cf` (**read**: `node_is_dead`,
  nir_opt_dead_cf.c:156-260: no side effect, no phi after, no value defined inside
  and used outside): the only thing the loop defines is its own counter.
- **`wrapper_unroll`** (`unconditional_break`, last statement `break`;
  nir_opt_loop_unroll.c:1067-1073): one guarded pass. `deqp_loops_fs_uncond_break`
  (13 instructions, 0 loops): `ICMP_OR.s32.ge.m1 r2, 0x0, u2.w0` (the first
  test, i = 0, against the bound; the counter is a constant), then
  `MUX.v2i16 r3/r0/r1, ..., r2.h00` x3, i.e. `res.yzwx` selected on the mask.
- **Arrays** (`scoping.valid.while_*`, `struct.local.dynamic_loop_struct_array`,
  `discard`/`return` forms with no loop): no loop in the words, see S2 for the
  array rule; `deqp_loops2_fs_scope_while` is straight code (14 words),
  `fs_struct_local` is a `CSEL.u32.eq` ladder (22 words, 0 loops).

### 1.3 S2: array subscripts by the counter (fs 50, vs 44)

Mesa calls `partial_unroll` (nir_opt_loop_unroll.c:755-800, **read**) when a
loop has no limiting terminator, one terminator, no nested loop and a
`guessed_trip_count` from `guess_loop_limit` (the length of the array the
counter indexes, nir_loop_analyze.c:400-427); it clones the body `n` times,
each in the continue side of the previous copy's `i < ui_four` test and re-inserts
a loop that has no use left (deleted by `dead_cf`). What the words show about
what is left of the `i < ui_four` tests (measured, the rule is mine):

- **A guarded store vanishes**: the select between the new and the old (undefined)
  element is the new element. `deqp_loops_fs_idx_dyn` (write loop dynamic,
  read static): 11 instructions, no read of `ui_four`; the static read is
  `FMA.v2f16 ... 0x38000000.h11`, `0x34000000.h11`, `u2.w0` = the sum of the
  three. `deqp_loops2_fs_idx_tmp_float_dd` (dynamic write loop, **dynamic read by
  `ui_zero..ui_three`**): 38 instructions, no `ui_four`, an `ICMP_OR.s32.lt.m1 r, u,
  0x1000000.b3/0x3020100.b3/b2, r3` + `MUX.v2i16` ladder per read (one per
  uniform index).
- **A guarded accumulation stays**: `res += arr[i]` under `i < ui_four` is a
  select with the old `res`. `deqp_loops2_fs_idx_tmp_float_dldl` (22 words):
  `ICMP_OR.s32.lt.m1 r5, 0x7060504.b0, u2.w0, r4` ... four compares of `ui_four`
  against `3, 2, 1, 0` each followed by `MUX.v2i16`, and the final `0 < ui_four`
  select of `res` against 0. `fs_idx_tmp_vec4_sd`, `fs_idx_mat4_sd`,
  `fs_idx_var_vec4_sd` (44) are the same ladder at vec4.
- **A guarded accumulation with a load in it stays as branches**:
  `deqp_loops2_fs_idx_uni_float`/`uni_vec4` (uniform arrays are loads that the
  peephole may not speculate): `ICMP_OR.s32.lt.m1 r0, 0x0, u3.w0; BRANCHZ.eq.reconverge`
  nested four deep, `MOV.i32.reconverge` joins (30 and 34 instructions, 0
  loops, no back edge).
- **Not this rule**: `deqp_loops2_fs_idx_tmp_vec4_cd` (4 cases): a constant-written
  vec4 array is placed in thread-local storage (`STORE.i64.force ...
  thread_local_pointer.w0`, `LOAD.i64.force`; 141 instructions, one loop that reads
  `thread_local_pointer + (i << 3)`). That is a scratch array, not loop lowering.

The vertex words are the same decisions with fp32 stores
(`deqp_loops_vs_idx_dyn`: 23 instructions, 0 loops, `FMA.v2f16` sum, `F16_TO_F32`
and the three constant stores).

### 1.4 S5: `break` and `continue` in a kept loop (fs 28), no second exit edge

All measured as ALU into the **one** exit mask or `MUX` selects, never a
second branch:

- **`conditional_break`** (`deqp_loops_fs_cond_break`, 19 instr.): `ICMP_OR.s32.ge.m1 r1,
  r2, u2.w1, 0x0` (bound), `ICMP_OR.u32.eq.m1 r4, r2, u2.w0, 0x0` (`i == TWO`),
  `LSHIFT_OR.i32 r1, r4^, 0x0.b00, r1^` (the OR of the two masks: a shift by 0 ORed
  with the third source), `BRANCHZ.reconverge r1^.h0`. One exit.
- **`infinite_with_conditional_break`** (`deqp_loops_fs_inf_break`, 18): the
  head `if (i == ui_one) break;` is the whole test, `ICMP_OR.u32.eq.m1 r1, r2,
  u2.w0` and `BRANCHZ.reconverge r1^.h0` (taken = leave: **no inverted compare**,
  the front end's `if (c) break;` polarity) and the third `MKVEC.v2i16` after the
  loop, as a do-while: the for(;;) body's break is the loop's first test.
- **`conditional_continue`/`double_continue`** (`deqp_loops_fs_cond_continue`, 21;
  `double_continue` has the same words): the body after the `continue` is
  moved into the else side and flattened: `ICMP_OR.u32.eq.m1 r3, r2, u2.w0`,
  four `MUX.v2i16 ... r3.h00` of the pre/post rotation halves, two `MKVEC.v2i16`,
  then the step `IADD_IMM`; the head test and its `BRANCHZ ... offset:9` come
  first (the loop is still kept: the head's bound is a run-time uniform).
- **`mixed_break_continue`** (`deqp_loops_fs_mixed`, 23): `ICMP_OR.u32.ne.m1 r3,
  r2, 0x0` (`i != 0`), `ICMP_AND.u32.eq.m1 r4, r2, 0x3020100.b3, r3` (`i == 3`
  gated by `i != 0`), the bound compare, `LSHIFT_OR.i32` of the two, `BRANCHZ`; the
  continue is four `MUX.v2i16` on `r3` (`i != 0`).
- **do-while with a break/continue** (`deqp_loops2_fs_do_uni_conditional_break`, 20;
  `..._continue`, 22): the test comes after the counter's `IADD_IMM`; the break
  mask `ICMP_OR.u32.eq.m1 r1, r2, u2.w0` is computed **before** the increment
  (word 6, then `IADD_IMM`, then the bound compare, then `LSHIFT_OR`): the pre-increment
  counter is what the user's `i` was.
- **`conditional_body`** (`deqp_loops_fs_dowhile_if`, 22; `for`/`while` forms
  `deqp_loops2_fs_for_uni_conditional_body` 21, `fs_wh_uni_conditional_body` 24): an `if` of
  pure ALU in the body is `ICMP_OR.s32.lt.m1 r3, r2, 0x3020100.b2` and four
  `MUX.v2i16`, no branch.

### 1.5 S6: nested and sequenced loops (fs 24)

`fs_for_uni_nested` (23): the inner pre-header is inside the outer body, so the
inner counter is re-initialised on each outer pass (`IADD_IMM.i32.reconverge r3,
0x0, #0x0`); the carried pair that the inner loop only reads is copied (`MOV.i32
r4, r1^`) before the inner header, and the outer latch ends with `IADD_IMM r2`,
`MOV.i32 r1, r4^`, and the outer back edge (offset -13; the inner back edge -7).
`fs_for_uni_nested_sequence` (32, 3 loops): two sequential inner loops; the
second inner loop's counter reuses `r3` and the pair copies between them are
`MOV.i32 r1, r4^; IADD_IMM.i32.reconverge r3, 0x0, #0x0; MOV.i32 r4, r1^`.
`fs_for_uni_sequence` (26, two **top-level** loops): the second loop's counter
is `r1 = MOV.i32 r2^` and `MOV.i32.reconverge r2, r3^; MOV.i32 r3, r2^`: a
register swap through the out-of-SSA copies. `nested_tricky_dataflow_2` (24):
the inner loop rewrites `coords`, so the outer latch rebuilds both pairs after
the inner loop (`MKVEC.v2i16 r1, r0.h1, r4.h0; MKVEC.v2i16 r0, r4^.h1, r0^.h0`).
Every one of these is a straight composition of the single loop's words;
the **register assignment of the phi copies** is what to compare first (see risks).

### 1.6 S7: kept loops with an index ladder, `discard`, `return`, structs (fs 44)

- **Vector subscript** (`deqp_loops2_fs_idx_vec_w_direct`, 33; `fs_idx_vec_direct_w`, 25):
  write is `ICMP_OR.u32.eq.m1` of the counter against byte-lane constants
  (`0x3020100.b3`, `.b2`, `0x1000000.b3`, `0x0`) and `MKVEC.v2i16` packs of two
  compare masks used as the third `MUX.v2i16` source (two pair selects at
  once); read is `ICMP_OR.s32.lt.m1` + `MUX.v2i16` + `FADD.v2f16`. The pairs (`r2`,
  `r3`) of the written vector are carried (`IADD_IMM r2, 0x0, #0x0` before the
  loop opens it: the header is `.reconverge` on the counter).
- **`discard` in the loop** (`deqp_loops2_fs_disc_dyn`, 17): `CSEL.s32.lt r1, r0, u2.w0, r0,
  u2.w0` (min), `ICMP_OR.s32.lt.m1 r1, 0x0, r1^`, `DISCARD.f32.ne r1^, 0x0` in
  the body; `NOP.discard` first. The loop is kept.
- **`return` in a dynamic loop** (`deqp_loops2_fs_ret_dyn`, 21): the return is an
  exit mask: `FCMP_OR.v2f16.ge.m1` of the float compare, `LSHIFT_OR.v2i16`
  merged into the bound compare, the colour held in `r2/r4`.
- **Out of scope here**: `fs_idx_tmp_vec4_cd` (thread-local array, section 1.3),
  4 cases.

### 1.7 S1: vertex loops Mesa unrolls (vs 87)

- **Constant bound, any body (`break`, `continue`, `if`, nested, sequenced)**:
  everything folds; the result is the **same words** as the three constant
  references (`deqp_loops_vs_for_const`: 11 words, `v_color` stored as the
  permuted attribute lanes; `deqp_loops2_vs_do_con_nested`: the identical
  11-instruction program). A `compound`/nested body adds nothing.
- **`wrapper_unroll`** (`unconditional_break` with a uniform bound,
  `deqp_loops2_vs_for_uni_unconditional_break`, 20): `ICMP_OR.s32.ge.m1 r2, 0x0,
  u5.w1` and three `MUX.v2i16`, as the fragment's.
- **Returns**: `deqp_loops2_vs_ret_static` / `vs_fn_return_in_loop` (4) fold to
  `FADD.f32 r0, r0^.neg, 0x0.neg` and the store: the return makes the rest of
  `main` dead; `vs_ret_static_dyn` (15) a `CSEL.u32.eq` ladder of the run-time
  return condition; `vs_ret_infinite` (11, 20 cases with these words) the plain
  store. `vs_fn_after_seq` keeps one loop with constant bounds (14
  instructions, 1 loop: a function-local loop over a counter whose bounds are
  constants **and** a `BRANCHZ.eq` on `ICMP_OR.u32.ne` against 6): an outlier,
  listed under S8.
- **Invariance** (`vs_inv_*`, 6): the position program holds the whole
  unrolled FMA chain in fp16 (`FMA.v2f16 r2, r2^, u3.w0, 0x0.neg` x12, 43-71
  instructions); that is the `invariant gl_Position` family's words, not the loop's.

### 1.8 S8-S11: vertex kept loops

Section 1.1: the words are the fragment's with the stores after them. Details
of the shapes that differ:

- **S10** (vs 36: `conditional_break` etc.): the same mask forms as S5
  (`deqp_loops2_vs_for_uni_conditional_break`, 26: `ICMP_OR.s32.ge.m1 r1, r2,
  u6.w0`, `ICMP_OR.u32.eq.m1 r4, r2, u5.w1`, `LSHIFT_OR.i32`); the
  `conditional_body` `if` is `MUX.v2i16` (`vs_for_uni_conditional_body`, 28).
- **S9** (24): `vs_for_uni_nested` (30) and `vs_for_uni_nested_sequence` (39) are
  the fragment's nested words with the vertex tail.
- **S11** (18, `vs_idx_vec_*`): the same ladder as S7 in a kept loop
  (`vs_idx_vec_w_direct`, 36; `vs_idx_vec_dldl`, 46, two loops).
  `vs_idx_vec_w_dynsub`'s **position** program carries the loop (29 words: `IADD_IMM.i32.reconverge
  r0`, `ICMP_OR.s32.ge.m1 r1, r0, u3.w0`, `BRANCHZ.reconverge ... offset:2`, `IADD_IMM`,
  `BRANCHZ.eq ... offset:-4`, then the usual 24): a counter-only loop Mesa does
  not delete there (**read**: `node_is_dead` fails on the body's `load_output`/
  array access of the output copy; not proven).
- **S8's outliers**: `vs_fn_after_seq`, `vs_fn_mixed`, `vs_ret_dyn`, `vs_ret_dynamic_loop_never`
  (each a function with a loop and a return, 14-27 instructions).

## 2. Mesa's unroll decision, with the thresholds

`nir_opt_loop_unroll` (src/compiler/nir/nir_opt_loop_unroll.c) is run twice:
by the GL front end's optimisation loop and by bifrost's
(`bifrost_nir.c`, `bi_optimize_loop`); both before the loop's constants are
lowered to a FAU word. Options (`bifrost_compile.h:140-144`):
`max_unroll_iterations = 32`, `force_indirect_unrolling = shader_in | shader_out
| function_temp`, `force_indirect_unrolling_sampler = true`. The decision is
`process_loops` (`:990-1164`), per loop, innermost first:

| step | condition (source line) | result |
|---|---|---|
| 1 | no limiting terminator and not `complex_loop`, the last block ends in `break` (`:1067-1073`) | `wrapper_unroll` (`:546`): the `break` goes, the rest nests in the terminators' continue side (S1, S4) |
| 2 | same, no nested loop, one terminator, not yet partially unrolled, `guessed_trip_count` (an array indexed by the counter, `:1083-1086`) and `check_unrolling_restrictions` | `partial_unroll(n)` by the array's length (S2) |
| 3 | no limiting terminator (the bound is a uniform, a varying, a select of a uniform), or `has_nested_loop` and the loop does not break after its first iteration (`:1112-1114`) | **kept** (S3, S5-S11; also an outer loop around a kept inner loop, S6/S9) |
| 4 | `check_unrolling_restrictions` fails (`:899-931`): `trip_count <= max_iter` (32) and `instr_cost * trip_count <= 32 * LOOP_UNROLL_LIMIT (26)` = 832 | **kept** (the 101-pass constant loops) |
| 5 | exact trip count known (`:1119`) | `simple_unroll` (S1 constants, the walk's `walk_loop`) |
| 6 | two terminators, one limiting (`:1124-1149`) | `complex_unroll` (`conditional_break` with constants) |
| 7 | one terminator whose count is bounded but unknown (`:1151-1155`) | `complex_unroll_single_terminator` (the `merged_bound` form, mediump float counters) |

`instr_cost` (`nir_loop_analyze.c:79-130`): an intrinsic or texture is 1, an ALU
instruction 1, except what is constant after unrolling, a `mov`/`vec`, a
selection whose condition is constant after unrolling and an `fmul` read only by
`fadd`s (fused), which cost 0. The compiler's twin is `unroll_cost`
(`nir_fs.rs:4899`), limits `MAX_UNROLL_ITERATIONS = 32` and `LOOP_UNROLL_LIMIT = 26`
(`nir_fs.rs:76-77`). `trip_count` is `max_trip_count` when known, else the guess.
A dead loop (`nir_opt_dead_cf`, `node_is_dead`) is removed whatever the above says:
no side-effecting intrinsic or jump out of the loop, no phi after it, nothing
defined inside used outside (S4 empty bodies and `only_continue`).

## 3. The refusal sites at the tip, and why each refuses

`src/shaderc/online/mali/`, tip `5b3fb8c2`. "Cases" are 9.2's.

### 3.1 Fragment

| site | refusal | why it refuses |
|---|---|---|
| `nir_fs.rs:5115` `walk_kept_loop` | `LoopMediump` (281) | `!prec.highp`: the kept-loop code (header phis, `LoopCap`, exit edges) is stage-neutral, but a mediump shader's carried `vec4` is two 16-bit pairs and the mediump selection had no blocks. The route exists for other shapes (`kept16`, `guarded16`, `mod.rs:1548`: `select_highp` with the `fs16` layout) but not for a loop |
| `nir_fs.rs:4604` `walk_if16` | `GuardMediump` (12) | `exit` is not `IfExit::Plain`: `kept_loop` (`:4923`) said "unroll" for a loop it does not read (a `do { ... if (c) break; ... } while (...)`, whose first control node is the body's `if`; or `for (;;) { if (c) break; ... }`, whose head is `if (c) break;`, not `if (c) {} else break;`), so `walk_loop` unrolled it with a run-time guard, which the mediump `walk_if16` refuses. The loop is **kept** in Mesa |
| `nir_fs.rs:4733` `walk_passes` | `LoopForm` (3) | the 33rd pass: the 101-pass constant loop is folded pass by pass because `kept_loop`'s test (`ir_folds`) folds at pass 1; Mesa keeps it (rule 4) |
| `nir_fs.rs:4873`, `:5122` | `LoopForm` | a body with no instruction (the S4 empty body; `walk_loop` and `walk_kept_loop`): nobody decides "delete" |
| `nir_fs.rs:4883` | `LoopForm` | an unrolled body over `26 * 32` nodes (the cost rule applied after the fact) |
| `nir_fs.rs:4777`, `:4483`, `:5398`, `:5403`, `:5412` | `LoopForm` | a second run-time `break` in an unrolled loop; a side in a flow neither guard nor kept; `if (c) { step; continue; }` not at the top of a kept body or with more than the step: the forms S5 needs |
| `nir_fs.rs:5129`, `:5169`, `:5200`, `:5203`, `:5210`, `:5247`, `:5253`, `:5266` | `LoopForm` | the kept loop's own forms: the colour written inside, a carried lane the body never defines, a body that always breaks, a head test that took no counter, a lane left as found, more than two or no `break` edges, two edges with different colours |
| `nir_fs.rs:5429` | `ControlFlow` | a bare `continue` or `break` outside a loop the walk is in |
| `nir_fs.rs:11759` `select_highp` | `GuardMediump` | a `guarded16` ladder with a lone fp16 product (`deadloop_*`; **not** a dEQP shape) |

Which text a case first meets is `kept_loop` (`:4923-4933`): the body's first
non-instruction node must be `if (c) {} else break;` with `c` not foldable
(`ir_folds`, `:4938`). It is a decision on one node; every deviation of S3-S7
is a different node form or a count past 32.

### 3.2 Vertex

| site | refusal | why |
|---|---|---|
| `nir.rs:3819` `vs_steps` | `LoopVertex` (the bulk of 241) | a top-level loop that is not `vs_loop_shape` (`:3742`): nested loop, `if`, `continue`, second `break`, no head test, or the `unconditional_break`/`only_continue` forms Mesa unrolls |
| `nir.rs:3778` `vs_steps` (`side`) | `LoopVertex` | a loop inside an `if` side |
| `nir.rs:3813` | `LoopVertex` | unreachable from `vs_loop_shape` (a defensive arm) |
| `nir.rs:4128` `vs_kept_begin` | `LoopVertex` | an output stored in the body (`return_in_dynamic_loop_*`, S8) |
| `nir.rs:4174` | `LoopVertex` | a carried lane the body does not define (`declared_in`) |
| `nir.rs:4184` | `LoopVertex` | **a 16-bit carried lane**: every `mediump vec4` loop (S8 L1, S9, S10) |
| `nir.rs:4205`, `:4213` `vs_kept_head` | `LoopVertex` | the head test is not an integer `Icmp` `Lt/Ge/Eq/Ne` of one lane: a float counter (`basic_*_float`), an unsigned compare |
| `nir.rs:4255` `vs_kept_end`, `:4269` | `LoopVertex` | no head test taken; a lane left as found (`nir_opt_remove_phis` makes no phi) |
| `nir.rs:4261` | `LoopVertex` | an attribute, texture or uniform-array load made first inside the loop (`vs.block[id] != s.pre`) |
| `nir.rs:4362`, `:4371` | `LoopVertex` | the unrolled cost over 832, or the 33rd pass: **the right answer for a 101-pass loop is not a refusal but a kept loop** (S8), and the cost case is a kept loop too |
| `nir.rs:4382` | `LoopVertex` | a head test that does not fold in a pass of an unrolled loop (`complex_unroll`) |
| `nir.rs:6090` `variant_dce` | `LoopVertex` | the kept loops are not all needed by the position variant, or all by the varying one (S11's `vs_idx_vec_w_dynsub`) |

The vertex walk is one flat list of `Step`s with a single `pre`/`then` pair
for flattened `if`s (`walk`, `Step::If/Else/EndIf`, `vs_join`), so a nested
`if`, a `break`/`continue`, or two loops at once are not representable in it:
that, with the mediump lanes, is why nothing but plain loops reaches the
walker.

## 4. Design: fragment (item loops-lower-1)

Order: **A0 (the decision), S4, S3, S5, S6, S2-fs, S7**. S4+S3 first
(150 cases: the decision plus the first kept loop, which brings the
16-bit phi rows, the dynamic bound and the `LoopForm` fixes S5-S7 reuse).
A step is one commit with its tests un-ignored and its references byte-compared
(kept loops with `CapEdit`, section 7).

### A0. One decision function for both stages (`loop_plan.rs`, new)

Today `kept_loop(f, m, body, locals) -> bool` (`nir_fs.rs:4923`) is the only
decision, and the vertex walk has its own (`ir_folds` on the head,
`nir.rs:4340`). Replace both by one pure function over the IR in a new file
`src/shaderc/online/mali/loop_plan.rs`:

```text
pub(super) enum LoopPlan {
    Delete,                         // nir_opt_dead_cf
    Unroll,                         // simple/complex/wrapper: the walk's passes
    Partial { n: u32 },             // partial_unroll by the guessed trip count
    Keep(KeptForm),                 // emit_loop
}
pub(super) struct KeptForm { head: HeadAt, rotated: bool, ... }
pub(super) fn plan(m: &Module<'_>, body: ir::Range, after: &[Node],
                   locals: &[[Val; 4]; MAX_LOCALS], folds: &dyn Fn(ValueId) -> bool) -> LoopPlan
```

It reads Mesa's order (section 2): (1) `Delete` when every local lane stored in
the body is dead after the loop (the loop's own counter, declared in the
`for` scope: a scan of `after` for `LoadLocal`s of those locals) and the body
has no `Discard`, `Tex`, `StoreOutput`, colour store or `Return`; (2) the
terminators: the `if (c) {} else break;` head (`for`/`while`), the tail
test of a `do`/`while` (`Lower::break_unless` at the tail, `lower.rs:2287-2298`),
and every body-level `if (c) break;`; (3) the limiting one is the first whose
`c` folds from constants and the walk's constant locals (`ir_folds`), its exact
trip count from `loop_count::constant_loop_entries` (already used by
`loop_cap_limit`, `nir_fs.rs:5084`); (4) the order of section 2: `Unroll` for
a last `break` (wrapper), `Partial` for a guessed count (S2) when exactly one
terminator and no nested loop, `Keep` when no limiting terminator, else the
cost rule with `unroll_cost` generalised from node counts to IR ALU counts
(`instr_cost`) times the count against 832 and the count against 32.
`KeptForm.rotated` records the do-while / 101 form (counter before the test,
third `MKVEC.v2i16` after the loop).

Callers: `walk_nodes` (`nir_fs.rs:5419-5426`, the two `Node::Loop` arms) calls `plan`
once; `Keep` goes to `walk_kept_loop`, `Unroll` to `walk_loop`, `Delete` skips
the node, `Partial` to the S2 path. The vertex walker (`nir.rs:4340`) calls the same
function with its own `folds` closure (`vs` walks have `w.locals`).
Why one function: the vertex steps S1/S8-S10 (241 cases) are the same
decisions on the same IR forms; `kept_loop` and `vs_loop_shape` otherwise drift.

**Risk of A0**: it changes decisions that highp loops already take byte-equal
(`intloop_*`, `divnest`, `loop_exit`, `builtin_98`, `builtin_45`, `alu_divloop`,
`fvg_filter_image`). `src/shaderc/online/mali/tests.rs:1303` pins `for (int i =
0; i < 40; i++)` highp as `LoopForm`; rule 4 makes Mesa keep it, so after A0 it
compiles and is **not** Mesa's until a reference is drawn (`drm_shim_ref.sh run`
on that shader: add it as `deqp_loops3_*` or as the pin's reference, with
`CapEdit`). Do not leave a pin as "compiles".

### S4 (22): `Delete` and the `unconditional_break` unroll

- `plan`: `Delete` as in A0 (1); wrapper unroll: a body whose **last node is
  `Break`** (the `unconditional_break` for/while/do_while, and the infinite `for (;;)
  { ...; break; }` last forms) is `Unroll`. `walk_passes` ends at pass 1 on
  `Flow::Break` (`:4739`), **but the head test is not handled by the existing
  arms** (read at the tip, to confirm when written): the head `if (c) {} else
  break;` with a `c` the walk cannot fold, in an unrolled loop, matches none of
  `walk_nodes`' arms (`:5340-:5384`: `walk_guard` wants `then_ == [Break]`
  and an empty else, the kept-head arm wants `f.kept`), so it reaches the last
  `walk_if(.., IfExit::Plain, ..)` with a then side that falls through and an
  else side that breaks, which `walk_if`'s flow match answers `LoopForm`
  (`:4483`). S4 therefore needs **one new arm** in `walk_nodes`: the head
  test of a wrapper-unrolled loop is `if (c) { REST }`, the rest of the pass
  as the then side, no else; and `walk_if16`'s `sel` closure (`:4630-:4660`)
  must take an **integer-mask** condition (`f.mask_lane(cond)`, 32-bit
  `ICMP_OR` mask) with two different 16-bit lanes, a `Bcsel` of 16 bits
  emitted as `MUX.v2i16 rD, a.h00, b.h11, mask.h00` (today it takes two
  defined lanes only under a float compare, `cmp16`, and answers
  `ControlFlow` otherwise). The words are then the ladder of
  `deqp_loops_fs_uncond_break`: one `ICMP_OR.s32.ge.m1 r2, 0x0, u2.w0`, three
  `MUX.v2i16 ... r2.h00`, no loop. So S4 is **decision plus these two
  changes**, not "no new code" as 9.5 says; the same two changes are what S5's
  `continue` selects and S3's `conditional_body` need, so S4 comes first.
- `walk_loop` (`:4873`) and `walk_kept_loop` (`:5122`) refuse a body with no
  instruction; after A0 an empty loop is `Delete` and never gets there.
- The remaining S4 cases (`scoping.valid.while_*`, `struct.local`, `discard`,
  `return` forms with no loop) are the array rule (S2) or the dead loop;
  none needs new words.
- **Words to reproduce**: `deqp_loops2_fs_for_uni_empty_body` (8 words, 13 cases
  with these words), `deqp_loops_fs_uncond_break` (13, 0 loops), `fs_scope_while`
  (14), `fs_struct_local` (22).
- **Un-ignores** (`tests/mali_deqp_loops.rs`): `fs_uncond_break`.
  `tests/mali_deqp_loops2.rs`: `fs_for_uni_empty_body`, `fs_scope_while`,
  `fs_struct_local`, and the cases that share these words. Each: drop its
  `REFUSALS` entry, add `mali_ref::assert_matches`.
- **Pins at risk**: the `Tests.rs` pin of `while (true) { gl_Position = p; break; }`
  (`:1230`, vertex: moves in S1); `cflow_*` loops that end in `break`
  (`gl_test` databreak, case 87) must stay byte-equal.
- **Expected**: 22 leave `LoopMediump`/`LoopForm`; the next refusal is unknown
  (the two `struct` ones need the struct-array constant forms of unif-ref).

### S3 (128): the first mediump kept loop

Composition: 61 with a uniform bound, 61 with a dynamic bound (`one * ui_n`),
6 constant (3 `select_iteration_count`, 3 `101_iterations`); L1 106 +
`conditional_body` 4 + `select_iteration_count` 9 + `101_iterations` 9.

1. **Lift the `!prec.highp` return** (`walk_kept_loop`, `nir_fs.rs:5115`).
   The header-phi code already gives a mediump float local `bits: 16`
   (`:5170-5176`); an int or bool local stays 32. The rest (exit edges, `LoopCap`,
   `f.nir.loops[...]`) is stage-neutral.
2. **Route the mediump shader to `select_highp`** (`mod.rs:1548`): add a
   `kept_loops16(&f)` sibling of `guarded16`/`kept16` (`nir_fs.rs:10854, 10874`):
   `!f.prec.highp && f.nir.nloops > 0`. The `fs16` UBO layout is what the
   other routes use. This is the route's one change; `select_highp` already
   builds blocks and jumps for highp loops (`end_block`, the `LoopRegion`s).
   Two traps in `mod.rs:1500-1560`'s chain (read at the tip): `kept16`
   (`nir_fs.rs:10874`) is true for **any live `IfRegion` that is neither
   `single` nor `inot`**, and a kept loop's head `if` is such a region
   (`brk: Some(lp)`, `walk_if` :4470-:4480), so in a mediump kept loop the
   chain would take the `kept16` arm and `branch_forms` (`:10887`) would
   answer `ControlFlow` to the phis, `Icmp`, `Vec2`. Put `kept_loops16`
   **before** `kept16` and make `kept16` skip regions with `brk.is_some()`.
   The loop route needs its own forms list (`Phi` of width 1 and 2, `Vec2`,
   32-bit `Icmp/Iadd/Imul/Ior/Csel/F2i32`, 16-bit `Fadd/Fcmp/Bcsel`, the
   varying's `F2f16`), as the item below says.
3. **Pair the carried lanes** (`vectorize16`, `nir_fs.rs:11068`): it pairs ALU
   nodes only (`pairable`). Add `Phi`: two 16-bit one-lane phis of one header
   whose lanes `2k`, `2k+1` of the same local (`registration_order` makes them
   adjacent) become one `width: 2` phi, each reader's `Val::Lane(b, 0)` rewritten to
   `Val::Lane(a, 1)` as for ALU pairs; the back-edge source is the `Vec2`
   (`MKVEC.v2i16`) that rebuilds the pair. `res.yzwx` is **not** a node of
   the walk (a swizzle of lanes): the two `Vec2` nodes are made where the
   stored local's lanes are rebuilt (`Alu::Vec2` is excluded from `pairable`
   and already exists for the selects of a ladder, `walk_if16`'s `sel`, and
   `Fs::vec2` use in `nir_fs.rs`).
4. **Emit rows** (`select_highp`, `emit.rs`): a `(Phi, 16, width 2)` copy as
   `MOV.i32` of the whole word; a `(Phi, 16, width 1)` copy as `U16_TO_U32` (a
   lone lane; `intloop_contloops` words 51-54); `Vec2` of two halves as `MKVEC.v2i16
   a.h1, b.h0` (exists: `nir.rs:2273-2277`). `branch_forms` (`:10887`) and
   `ladder_forms` (`:10988`) are the gates of the `kept16`/`guarded16` routes and
   stay unchanged: the kept-loop route has its own forms list (add `Phi`
   pairs, `Icmp`/`Iadd`/`Imul` at 32 bits, `Fcmp` at 16 for the float counter, `Vec2`).
5. **`kept_loop` decisions** (via A0): the `for` whose constant count is over 32
   (the 101 cases) is `Keep` with `rotated = true`; a `do`/`while` is `Keep` whatever
   the body holds (S5); `select_iteration_count`'s bound is
   `Bcsel(bool uniform, N, 0)` and not foldable: `Keep`.
6. **The rotated form** (`deqp_loops_fs_dowhile_uni`, `fs_101`): the counter's
   `IADD_IMM` is emitted before the test; the exit leaves the pair as it was
   *before* the rotation: `walk_nodes`' head-test arm (`:5365-5383`) gets a
   `KeptForm.rotated` flag: the `cap`'s `iadd(phi, -1)` and the counter step precede
   the compare, and the `break` edge's locals are the pre-body ones, so the exit's
   third `MKVEC.v2i16` is the rebuild the exit edge needs (the walk's
   `edges[0].slot`, `:5240`).
7. **`select_iteration_count`**: `CSEL.u32.eq r1, u2.w0, 0x0, 0x0, r1^` needs the
   bool uniform word as a source (`Fs::int_lane`'s `bool_dword`, `nir_fs.rs:2275`)
   and the `MOV.i32 r1, u2.w1` of the then arm; take the intval implementation,
   do not duplicate it.
8. **The float counter** (`fs_float_uni`): the phi is a one-lane 16-bit node of
   the counter local (`IADD_IMM.i32.reconverge r2, 0x0, #0x2A66`: a 16-bit constant
   in a 32-bit move, duplicated halves are **not** used: the low half only), the
   step `FADD.v2f16 r2, r2^.h00, u2.w0.h00`, the test `FCMP_OR.v2f16.ge.m1 r1, r2.h00,
   u2.w1.h00`. The hidden cap's compare is an integer compare, so the cap's
   OR is a separate `LSHIFT_OR` (a 4-word cap, mali-loop-board-failures.md).
9. **The dynamic bound**: `int(v_one + 0.5)` is `FADD.f32` narrow, `FADD.v2f16 +0.5`,
   `F16_TO_S32.rtz` before the loop (four words, `fs_for_dyn_basic_mediump_int`
   words 1-4) and `IMUL.i32` in the header: let `walk_nodes` emit the multiply where
   the head test's operand is computed (inside the loop, not hoisted: `f.op32`
   at the head, `:5375`). This is the part that needs the intval items.

- **Words to reproduce**: `deqp_loops_fs_for_uni`, `fs_while_uni` (17),
  `fs_dowhile_uni` (18), `fs_101` (18), `fs_float_uni` (17), `fs_select` (19),
  `deqp_loops2_fs_for_dyn_basic_mediump_int` (23), `..._float`, `fs_do_*`,
  `fs_for_uni_conditional_body` (21), `fs_wh_*_conditional_body`, `fs_*_select_iteration_count`,
  `fs_*_101_iterations`, `fs_*_basic_highp_float`.
- **Un-ignores** (`mali_deqp_loops.rs`): `fs_for_uni`, `fs_while_uni`, `fs_dowhile_uni`,
  `fs_101`, `fs_float_uni`, `fs_select`, `fs_dowhile_if` (after A0(2): the do-while whose
  body holds an `if` is `Keep`, the `if` flattens: its pin moves from
  `ControlFlow`, **+2 cases beyond the 537**, the `do_while` forms of
  `conditional_body` first stop at `ControlFlow`); the corresponding
  `deqp_loops2_*`, compared with `CapEdit`.
- **Pins at risk**: `tests.rs:1240` (`intloop_loop_nested.frag`, `LoopMediump`; becomes
  S6's compare), `mali_int_loops.rs`, every highp kept-loop reference
  (`builtin_98`, `builtin_45`, `intloop_loop_exit`, `intloop_divnest`,
  `alu_divloop`: the loop code is shared), `tools/mali/corpus-verdicts.txt` rows for
  `loop_uniform.frag`, `loop_nested*.frag`, `discard_dynloop.frag`,
  `ret_dynloop_never.frag`, the glmark2 verdict row `fs 3678`.
- **Expected**: 128 leave the loop text. 61 of them also need the dynamic-bound
  words (intval's `IntValue` chain); 6 need the bool-uniform select; a case
  that stops at the next refusal is not counted here.

### S5 (28): `break`/`continue` in a kept loop

- `plan` (A0) makes the three loop types **kept** for every body whose limiting
  terminator does not fold: the two forms `kept_loop` misreads today
  (`do`/`while` with an `if`; `for (;;) { if (c) break; }`) and the S5 bodies.
  After this, the `GuardMediump` site `:4604` is not reached by dEQP shapes; keep it
  for `deadloop_*` (non-dEQP).
- **`break` under a run-time condition** (`walk_if`, the `brk` arm `:4350-4482`):
  Mesa's exit is **one** mask. Where today's walk builds a second exit edge
  (`BreakEdge`, `:5236-5266`: two edges, an exit phi), a `break` whose condition is
  a compare of the counter and a run-time value becomes `or(head_exit, c)`:
  `Alu::Ior` of the two masks as in `LSHIFT_OR.i32 r1, r4^, 0x0.b00, r1^` (the OR
  the cap already builds, `:5380`). `walk_nodes`' head arm (`:5365`) gets the guard's
  mask `c` as an extra operand: `walk_guard`'s `rest` is the rest of the
  body, run in the same block (no else block); only the `Break`-side
  flows as `Flow::Break` are discarded. `LoopForm` `:4483` and `:4777` become dead
  for these bodies.
  **When the merge applies is Mesa's, not "always"** (source,
  `opt_loop_merge_terminators`, nir_opt_loop.c:554-712): the `if (c) break;`
  and the loop's head test are merged into one `ior` only when (1) both are
  *basic terminators* (an `if` whose only content is the `break`, on the same
  side: `is_basic_terminator_if`, :604-632), (2) they are **consecutive**,
  with nothing but ALU, `load_const`, deref and `load_deref` instructions
  between them (:670-681), and (3) **no phi follows the loop and none the
  second `if`** (:655-659, :686-692): every value live out of the loop is the
  same at both exits. dEQP's shapes meet all three (the break comes before the
  rotation, or both exits read the same rotated pair), and so do the
  head-after-body `do while` forms. The highp references that keep **two
  real exit edges** (`intloop_loop_exit`, `intloop_divnest`: a different
  `last`/`outer` at each exit, an exit phi) do not, and must keep
  `walk_kept_loop`'s two-edge code (`:5236-:5290`, `MAX_BREAKS`). So the new
  arm decides by comparing the lane tables at the two exits as `walk_kept_loop`
  does for its exit phis (`f.table(e0)[l] != f.table(e1)[l]`): equal → OR into
  the head's mask (before the cap's `ior`), different → the old edge. Without
  that test the five cap tests and `intloop_divnest` change. The `infinite_with_conditional_break` head `if (c) break;`
  (`for (;;)`) is the one exit test, **no inverse compare** (`kept_loop`'s head
  polarity `then = Break`, else empty).
- **`continue`** (`if (c) { step; continue; }` form, the front end copies the
  step before the `continue`, `Lower::loop_body`): flatten as `walk_if16`'s select
  ladder over the rest of the body: today's `walk_nodes` arm (`:5396-5416`) takes
  `if (c) { step; continue; }` only at the kept body's **top** (highp `divnest`'s
  branch form) and `LoopForm`s otherwise (`:5398/5403/5412`). For a **mediump**
  body whose rest is ALU only, `nir_opt_peephole_select` flattens the whole
  rest (rule: `PEEPHOLE_LIMIT`, 8 ops per side, the GL front end's 64 for
  vertex); implement as `walk_if16` on the then side `[step; continue]` with
  `Flow::Continue` allowed (`:4611-4641` today demands both sides
  `Flow::Next`): `IfExit::Continue` into `walk_if16`, join at the latch, the
  moved `rest` in the else side. The second `continue` of `double_continue` joins
  the same selects (same words as `conditional_continue`).
  A `continue` whose rest holds a load or a call keeps the branch form (highp
  `divnest`); not a dEQP shape.
- **`mixed_break_continue`**: `if (i == 0) continue; else if (i == 3) break;`:
  the `continue` is selects on `i != 0` and the `break`'s mask is gated by `i != 0`
  (`ICMP_AND.u32.eq.m1 r4, r2, 0x3020100.b3, r3`): so the break's condition is
  `and(eq(i, 3), ne(i, 0))` which `int_lane` produces from the front end's else-if
  nest; the gate is Mesa's `nir_opt_algebraic` (the third source of an `ICMP_AND`
  is the other mask).
- **Words**: `fs_cond_break` 19, `fs_cond_continue` 21, `fs_double_continue` 21,
  `fs_mixed` 23, `fs_inf_break` 18, `fs_do_uni_conditional_break` 20, `..._continue`
  22, `fs_wh_uni_*`, `fs_do_dyn_conditional_break`.
- **Un-ignores**: `mali_deqp_loops.rs` `fs_cond_break`, `fs_cond_continue`,
  `fs_double_continue`, `fs_mixed`, `fs_inf_break` (**all of the 12 `GuardMediump`
  cases**); `tests.rs:1241` `intloop_contloops.frag` (`GuardMediump`): after this step
  it must **keep its words** (its `MUX.v2i16` at 40-55 are the same shape).
- **Pins at risk**: `intloop_contloops`, `intloop_divnest` (the branch form of
  `continue`), `cflow_71_deadloop` (the `guarded16` route).
- **Expected**: 28 leave their text (all `GuardMediump` 12 and 16
  `LoopMediump`). The `CapEdit` helper may need a new form: the cap's `ICMP_OR` fuses
  into the **first** compare of an OR chain, and `capped` "refuses a word that is
  not one of the forms above" (`tests.rs`): extend it with the three-compare
  chain of `fs_mixed`.

### S6 (24): nested and sequenced kept loops

- `walk_kept_loop` is already recursive for highp (`divnest`: `f.kept`, `f.loops`,
  `f.cap` saved and restored `:5180-5188`); lifting the mediump gate (S3) makes it
  take a nested mediump loop. What is new: the pairs the **inner** loop only
  reads must be copied before its header (`MOV.i32 r4, r1^` in `fs_for_uni_nested`):
  a `declared_in`-free local whose phi the outer loop owns is read by the inner
  loop through a copy that the out-of-SSA step places at the inner pre-header
  (`ra.rs`); and `nested_tricky_dataflow_*`'s inner loop rewrites `res`/`coords`
  so the outer loop's phi sources are the inner exit's phis (`f.phi(a, b, last)`
  at the exit: today's two-edge exit code `:5264-5276`). The inner counter is
  re-initialised in the outer body (the pre-header is in the outer's body).
  `nloops` bound: `nir::MAX_LOOPS`.
- **Words**: `fs_for_uni_nested` 23 (2 loops), `nested_sequence` 32 (3),
  `nested_tricky_dataflow_1` 30 / `_2` 24, `fs_for_uni_sequence` 26,
  `fs_do_dyn_nested` 31, `fs_wh_dyn_nested` 30.
- **Un-ignores**: `tests.rs:1240` `intloop_loop_nested.frag` (`LoopMediump`) moves
  to a `CapEdit` compare against its reference; `deqp_loops2_fs_*_nested*`.
- **Risk**: the register assignment of the copies (`fs_for_uni_sequence` words
  12-14: a three-copy swap) comes from `bi_out_of_ssa`, which is the one place where
  Mesa's order is an artefact; compare words 1..n in order and look at the copy
  sequence first. A mismatch here is `ra.rs`'s copy ordering, not the walk.
- **Expected**: 24.

### S2-fs (50): partial unroll of the array subscripts

- `plan` returns `Partial { n }` when: no limiting terminator (bound a uniform),
  one terminator, no nested loop, and an **indexed local/array access by the
  counter** (`IrOp::LoadLocal`/`StoreLocal` with `index: Some`, `LoadUniformIdx`,
  varying arrays) gives `n` = the array's length (a vector's element count, a matrix's
  column count: `find_array_access_via_induction`, nir_loop_analyze.c:400-427).
  `guess_loop_limit` is also a `check_unrolling_restrictions` input: `n <= 32`,
  `instr_cost * n <= 832` (`mat4` 60 words, fine).
- **Walker**: `walk_loop` with `Partial`: walk `n` passes (as `walk_passes`, the counter
  a constant per pass), each pass's head test `i < ui_four` being the **run-time**
  guard (`ui_four` an int uniform): through `walk_guard`/`walk_if16`, which in a
  mediump shader flattens to `ICMP_OR.s32.lt.m1 ... MUX.v2i16` as the S5 flattening
  does. Then the rule of section 1.3:
  - a guarded **store** into the array (`arr[k] = ...` under `k < ui_four`):
    the select's other arm is `Val::Undef`, so `walk_if16`'s `sel` keeps the
    stored value alone (`(t, Val::Undef) => Ok(t)`, `:4650`). No change;
  - a guarded **accumulation** (`res += arr[k]`): the select against the old
    `res` is kept: it is `walk_if16`'s select (the ladder of `fs_idx_tmp_float_dldl`);
  - a guarded **load** (a uniform array read, `LoadUniformIdx` with a constant
    index after folding): the `if` keeps its branch (`BRANCHZ.eq.reconverge` x4,
    `deqp_loops2_fs_idx_uni_vec4`): the `holds_look_up` rule of `walk_if`
    (`nir_fs.rs:4795`) extends to a uniform-array load; select the branch form
    when the side holds a `LoadUniformIdx`. That is the 8 cases of the uniform
    array (fs 4) and one vertex-free group.
  - the dynamic **read** of the array by uniforms (`arr[ui_n]`, `dd` cases): the
    existing `IndexedLocal`/`dynidx` ladder (`ICMP_OR.s32.lt ... MUX.v2i16`, three
    compares for four elements, `fs_idx_tmp_float_dd` words 6-17) is
    study-uniarray's; `stored_lanes` `:5042-5050` and `:5630` refuse
    `IndexedLocal` today, so an indexed store with a **constant** index (after
    the pass folds the counter) must fold to the array element
    (`ir::Op::LoadUniformIdx` fold at `:5444-5452` is the model for uniform
    arrays; local arrays need the same fold in the `StoreLocal`/`LoadLocal`
    with `index`).
- **Words**: `fs_idx_dyn` (old: 11, no `ui_four`), `fs_idx_tmp_float_dd` (38),
  `fs_idx_tmp_float_dldl` (22), `fs_idx_tmp_vec4_sd` (35), `fs_idx_mat4_dd` (60),
  `fs_idx_mat4_sd` (43), `fs_idx_uni_float` (30), `fs_idx_uni_vec4` (34),
  `fs_idx_var_vec4_sd` (44).
- **Un-ignores**: `fs_idx_dyn` (old); the nine `deqp_loops2_fs_idx_*` above. These
  have **no back edge**, so they are plain `mali_ref::assert_matches` (no
  `CapEdit`).
- **Pins at risk**: the walker's `holds_look_up` rule; `cflow_148_texbranch`,
  `dynidx_loop.frag` (`tools/shaders/es100`, a kept loop that must wait for S7).
- **Expected**: 50 (matrix 18 + tmp_array 24 + uniform_array 4 + varying_array
  4). The 4 `tmp_vec4_cd` scratch-array cases are counted in S7's 44, not here.
  Vertex twin: section 5, S2-vs.

### S7 (44): the kept loop with an index ladder, `discard`, `return`, structs

- `vector_subscript` (30): S3's loop plus `stored_lanes`' `IndexedLocal` accepting
  an indexed store whose index is the counter: the write is a ladder of `eq`
  masks into `MUX.v2i16` (`deqp_loops2_fs_idx_vec_w_direct` words 9-16: the masks are
  packed with `MKVEC.v2i16` for the pair-wide select), the read an `lt` ladder.
  `registration_order` decides which of `r2`/`r3` (the written vector's pairs) is
  the first phi.
- `discard`/`return` in a kept loop (6): the loop plus `Fs::discard` (`:1832`,
  `demote_if` as `DISCARD.f32.ne`) and a `return`, which the front end lowers to
  a flag (`Lower` return flag: an exit mask `or`ed into the head test).
  `walk_kept_loop`'s `stored_lanes -> LoopForm` (`:5129`, "the colour written inside")
  must accept the flag.
- `struct` (4): the struct-array locals of `fs_struct_local`; the same rule as S2.
- `tmp_array ... const_write_dynamic_loop_read` (4, `fs_idx_tmp_vec4_cd`): thread-local
  scratch; **leave refused** and name it in the report (a separate item: local arrays
  in scratch, `STORE.i64.force`/`LOAD.i64.force`).
- **Words**: `fs_vec_dyn` (old, 44, 2 loops), `fs_idx_vec_*` (25-58),
  `fs_disc_*` (16-25), `fs_ret_dyn` (21), `fs_ret_dynamic_loop_never` (17).
- **Un-ignores**: `fs_vec_dyn` (old); the `deqp_loops2_fs_idx_vec_*`, `fs_disc_*`,
  `fs_ret_*`.
- **Expected**: 40 of 44 (the 4 scratch arrays remain `LoopMediump` until a scratch
  item exists).

## 5. Design: vertex (item loops-lower-2)

What the vertex step reuses from the fragment steps:

| from lower-1 | used by | how |
|---|---|---|
| `loop_plan.rs::plan` (A0) | S1, S8-S11 | the decision is the same function; the vertex walker passes its `ir_folds` closure over `w.locals` |
| the 16-bit pair phis (`vectorize16` phi pairing, S3 item 3) and the `MOV.i32`/`U16_TO_U32`/`MKVEC.v2i16` rows | S8, S9, S10, S11 | the vertex `Nir` has `Val::Pair`/`vs_pair16` (`nir.rs:2273-2277`); the carried `vec4` is built with `V2F32_TO_V2F16` first: `vs_kept_begin`'s `bits != 32` refusal (`:4184`) is replaced by the same pairing |
| the mask forms (`Ior` of the exit masks, `LSHIFT_OR`, `MUX` flatten) | S10 | `vs_kept_head` (`:4204`) takes a mask, not only an `Icmp` lane |
| the partial-unroll rule and the array fold (S2-fs) | S2-vs | the same `Partial { n }` plan |
| the nested-loop phi copies (S6) | S9 | `KeptLoop` becomes a stack |

What is the vertex's own: the walker (a flat `Step` list), the `variant_dce`, the
tail `F16_TO_F32` stores (existing), and the hidden `UniformType` dependency.

### S1 (87): the counted loops Mesa unrolls, no kept loop

Composition: constant-bound bodies with `break`/`continue`/`if`/nested/sequenced
loops (L1 12 incl. `only_continue`, L2 12, L3 3, L4 42), `return` and
`functions` forms (10), `invariance` (6), struct (2).

- `vs_steps` (`nir.rs:3762`): replace the single `vs_loop_shape` check by
  `plan`. For an `Unroll` plan emit **nested** loops and the body's control
  flow into the step list: `Step::LoopBegin`, `Step::Head`, `Step::Break`,
  `Step::Continue`, `Step::If(c, else_idx, end_idx)`, `Step::LoopEnd`. `n` (the
  step count, `:3786-3792`) is computed from the same plan.
- `walk` (`nir.rs:4285`): `lp: VsLoop` becomes a stack `[VsLoop; MAX_VS_LOOPS]`.
  An unrolled loop is re-walked per pass (as today: `k = begin + 1`), with the
  counter's constants folded by `fold_int` (`nir.rs:865`); a folded `If`
  jumps to the taken side (`k = else_idx` or `end_idx`) instead of walking both
  and discarding one in `EndIf` (`:4400-4416`), so a `break`/`continue` in the
  taken side is a jump to the loop's end/next pass (`k = end + 1`,
  `k = latch`). A **run-time** condition inside an unrolled loop (not
  constant folded) is one level only (the S1 shapes: `unconditional_break`
  with a uniform bound = `wrapper_unroll`, whose single guard is the head
  test): it is the existing flat `Step::If`/`vs_join` select. A run-time
  `break` under a nested `if` (`complex_unroll`) has no vertex dEQP shape that
  reaches it with a uniform bound (those are S10, kept); leave `LoopVertex`.
- **Per-pass rebinding of IR values**: `build`'s values are keyed by IR value
  id and each pass overwrites its own (`w.vals`); the fragment walk does the
  same with `vals`/`locals`. No new rebinding is needed for the shapes above
  (the loop body's values are single-assignment per walk), except **nested
  loops**, whose inner values are re-walked per outer pass: that works for the
  same reason.
- `return` in the loop (`Node::Return`): when its path folds (`return_in_static_loop_*`,
  `return_in_nested_loop`, `functions.control_flow`) the walker stops the pass and
  every enclosing loop (`k = steps.len()`, as `IrNode::Return if j + 1 ==
  nodes.len()` does for the last node at `:3800`); a return under a run-time `if`
  is S8's.
- **`invariance.*.loop_*`** (6): their words are the `invariant gl_Position` family's
  (an fp16 chain in `vs.position`); lifting the loop text lands them on the
  invariant-position refusal, not on words. Count them as leaving `LoopVertex` only.
- **Words**: `vs_for_const` (11, old), `vs_while_const`, `vs_dowhile_const`,
  `vs_nested_const`, `deqp_loops2_vs_do_con_nested` (11), `vs_for_uni_unconditional_break`
  (20), `vs_ret_static` (4), `vs_ret_static_dyn` (15), `vs_ret_static_loop_always/never`,
  `vs_ret_infinite` (11), `vs_fn_return_in_loop` (4). 42 of the 87 have the words of
  `vs_for_const` already (loops note 9.3).
- **Un-ignores**: `mali_deqp_loops.rs` `vs_nested_const`; `tests.rs:1230`
  (`while (true) { gl_Position = p; break; }` pins `LoopVertex`: `wrapper_unroll`
  unrolls it: move the pin to the unrolled words); the `deqp_loops2_vs_*` above;
  `vs_idx_static` is S2-vs's.
- **Pins at risk**: `gm2_intloop_const_vs`, `gm2_cf_loop_uni_vs`
  (position-program loop), glmark2 rows 30/31 `vs 3cb0` (constant loop: leaves the
  refusal), `intloop_loop_nested_vs.vert` (S9).
- **Expected**: 87. `vs_fn_after_seq` (a kept loop in a function) is S8's.

### S2-vs (44): arrays by the counter, vertex

`Partial { n }` plus the same guard rule as S2-fs, with the vertex's flattening
(`PEEPHOLE_LIMIT_GL`, 64 operations per side, `vs_steps` `side`). The vertex words
are fp32 stores of the same ladders (`vs_idx_tmp_vec4_dd` 73, `vs_idx_mat4_dldl`
58, `vs_idx_var_vec4_dd` 83 at 114 fragment); `vs_idx_static` (constant
subscripts of the unrolled static loops, `deqp_loops_vs_idx_static`) is the same
constant array fold. Needs the vertex `IndexedLocal` with a folded index; today
`vs_kept_begin` returns `IndexedLocal` (`:4126`). **Expected**: 44 (12 `matrix`,
16 `tmp`, 16 `varying`).

### S8 (32): one kept vertex loop, 16-bit lanes

- `vs_kept_begin`: accept 16-bit carried lanes (`:4184`): phis of `bits: 16`
  and the pair pairing of S3; `vs.nodes[n].bits` of the lanes after
  `V2F32_TO_V2F16` is 16. The existing `LoopCap` phi (`:4137`) stays.
- `vs_kept_head` (`:4204`): accept a float counter (`Fcmp(Ge/Lt)` at fp32: `vs_float_uni`,
  `FCMP_OR.f32.ge.m1`) and an integer `Icmp` in either signedness; the inverse
  compare table gains `Fcmp`.
- **The 101-pass constant loop** (`vs_do_con_101_iterations`, `vs_wh_con_101`): the
  `:4371` pass-33 refusal goes: `plan` says `Keep` (rule 4). Its limit `101`
  (`constant_loop_entries`) makes the cap's immediate; the bound is a FAU word.
- **An output stored in the body** (`:4128`, the `return_in_dynamic_loop_*`
  forms) stays `LoopVertex` unless the store is after the loop; the 3 cases
  with a store inside lower to the return's exit mask (the fragment's S7).
- `do`/`while` (`vs_dowhile_uni`): the rotated form of S3 (the counter
  before the test, the exit takes the pre-rotation pair, one more
  `MKVEC.v2i16`); today `vs_loop_shape` accepts a tail head and
  `vs_kept_head` would build the wrong block order. Take `KeptForm.rotated`.
- **Words**: `deqp_loops_vs_for_uni` (24), `vs_dowhile_uni` (24), `vs_float_uni` (24),
  `vs_select` (26), `vs_vector` (24: same words as `vs_for_uni`), and
  `deqp_loops2_vs_for_dyn_basic_mediump_float` (27), `vs_for_uni_basic_lowp_float` (24),
  `vs_do_*_basic_*_float` (24-26), `vs_*_101_iterations` (24), `vs_ret_dyn` (27),
  `vs_ret_dynamic_loop_never` (24), `vs_fn_after_seq` (14), `vs_fn_mixed` (20).
- **Un-ignores**: `mali_deqp_loops.rs` `vs_float_uni`, `vs_for_uni`, `vs_dowhile_uni`,
  `vs_select`, `vs_vector` (last two also need the unitype/intval-vs16 families:
  their pins say `UniformType`); the `deqp_loops2_vs_*` above (`CapEdit`).
- **Dependency**: the 97 hidden cases (loops note 9.1) and `vs_for_uni`,
  `vs_dowhile_uni` stop first at `UniformType`/`mediump int` in the vertex stage; item
  intval-vs16 must have landed to see them leave the loop text. The 32 here are the
  cases already on `LoopVertex`.
- **Expected**: 32 (24 float-counter and 101 forms counted as in 9.5).

### S9 (24): nested and sequenced kept loops

`KeptLoop` (`nir.rs:4076`) becomes a stack (`kept: [KeptLoop; MAX_LOOPS]`, depth
`vs.nloops`); `Step::LoopBegin/LoopEnd` already carry the nesting indices;
`vs_kept_head`'s `then_first`/`else_first` blocks nest. `VsLoop::Kept` gets the
depth. `variant_dce` keeps all or none of the loops of a variant (`:6090`):
unchanged for these (the loop feeds only varyings; position is 24 words).
**Words**: `vs_for_uni_nested` (30), `nested_sequence` (39), `vs_*_dyn_nested*`.
**Un-ignores**: `tests.rs:1242` `intloop_loop_nested_vs.vert` (becomes a `CapEdit`
compare), the `deqp_loops2_vs_*nested*`. **Expected**: 24.

### S10 (36): kept vertex loops with `break`/`continue`/`if`

The vertex `if`-flatten (`vs_join`) takes the body `if`s of `conditional_body`
(L3, 6); the `break` forms (L4 30) become the single exit mask as in S5:
`vs_kept_head` takes `Ior` of the head compare and each body `break`'s mask;
a `continue` is the `MUX` flatten of the rest (S5), with the flat walker's `Step::If`
nesting capped at one level: `conditional_continue`/`double_continue` have
two sequential `if (c) { step; continue; }`, each a single-level flatten
that moves the rest into its else side. **Words**: `vs_*_conditional_*`,
`vs_for_uni_mixed_break_continue` (30), `vs_do_uni_infinite_with_conditional_break`.
**Expected**: 36 (6 + 30).

### S11 (18): vertex vector-subscript loops

S8's loop plus S7's index ladder, **and** the position-program loop of
`vs_idx_vec_w_dynsub`: `variant_dce`'s `kept != vs.nloops` (`:6090`) must
admit a variant with a counter-only loop. The reference (29-word position
program) is the only one in the 153 with a loop in `vs.position` besides
`gm2_cf_loop_uni_vs`. **Expected**: 17 safe (`w_direct`, `dldl`, ...) + 1 for the
position loop, which needs `gm2_cf_loop_uni_vs`'s position-program plumbing.

## 6. Case counts

| step | fs | vs | cases | 9.2 rows that make it |
|---|---:|---:|---:|---|
| S3 fragment kept loop, plain | 128 | | 128 | L1 106 + L3 4 + L5 9 + L6 9 |
| S4 deleted / unrolled | 22 | | 22 | L1 12 (`empty_body`, `only_continue`) + L4 4 (`unconditional_break`) + scoping 2 + struct 2 + discard 1 + return 1 |
| S5 `break`/`continue` kept | 28 | | 28 | L4 28 |
| S6 nested/sequenced kept | 24 | | 24 | L2 24 |
| S2 array subscripts | 50 | 44 | 94 | fs: matrix 18, tmp_array 24, uniform_array 4, varying_array 4; vs: matrix 12, tmp 16, varying 16 |
| S7 index ladder, `discard`, `return`, struct | 44 | | 44 | vector_subscript 30, tmp_array 4 (scratch), discard 4, return 2, struct 4 |
| S1 vertex counted loops Mesa unrolls | | 87 | 87 | L1 12, L2 12, L3 3, L4 42, functions 5, invariance 6, return 5, struct 2 |
| S8 one kept vertex loop | | 32 | 32 | L1 24, L6 3, functions 2, return 2, struct 1 |
| S9 nested kept vertex loops | | 24 | 24 | L2 24 |
| S10 vertex kept with jumps/`if` | | 36 | 36 | L3 6, L4 30 |
| S11 vertex vector subscript | | 18 | 18 | vector_subscript 18 |
| **sum** | **296** | **241** | **537** | |

By text: `LoopMediump` 281 + `GuardMediump` 12 (S5) + `LoopForm` 3 (S3's 101 constant) = 296 fragment,
`LoopVertex` 241: **537**, the total of 9.2. Fragment sessions: lower-1 does A0 + S4 + S3 (150) first,
then S5, S6, S2-fs, S7 (146). Vertex sessions: lower-2 does S1 (87),
S8 (32), then S9, S10, S11, S2-vs (122); it needs lower-1's A0, the phi pairing
and S5's mask forms, so it follows lower-1's S3 and S5.

"Expected" per step is the cases that **leave the loop text**; cases whose
next refusal is unknown stay in the count (section 9.5 states the same), and
the board rerun is the only pass count.

## 7. Cross-cutting rules for both items

- **The cap.** Every kept loop carries the hidden pass counter
  (`loop_cap_limit`, `LOOP_CAP` 4096: a deliberate deviation); kept-loop references
  are compared with `CapEdit` in `src/shaderc/online/mali/tests.rs`, never through
  `mali_ref::assert_matches` (section 9.6 of the loops note). A new word form
  (the `CSEL` bound, the `ICMP_AND` chain of S5) needs the `capped()` helper to
  know it, or the cap word lands in the wrong place.
- **References.** Every shape the lowering compiles needs a Mesa reference
  of its own words. 127 exist (`deqp_loops2_*`) plus the 26 older. A
  decision change that moves a highp loop (A0(4), the 40-pass constant) needs a
  reference drawn with `tools/mali/drm_shim_ref.sh` (build into
  `$CONNECTED_DEPS_DIR`), not a pin that says "compiles".
- **Regenerate** `tests/reference/*.rs` and `tools/mali/corpus-verdicts.txt`
  (`corpus-verdicts.sh --write`) in the same commit as a verdict moves; the glmark2
  fixture `tests/fixtures/mali_glmark2_verdicts.txt` rows 30-32.
- **Local checks** (CLAUDE.md): `cargo test --no-default-features --features
  host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_loops
  --test mali_deqp_loops2 --test mali_int_loops` and the touched `tests.rs`
  tests by name; never `tools/check.sh`.
- **Tests and pins most at risk**, by step: `intloop_loop_nested*`, `intloop_contloops`,
  `intloop_divnest`, `intloop_loop_exit`, `builtin_98`, `builtin_45`, `alu_divloop`,
  `gm2_cf_loop_uni_vs`, `gm2_intloop_*`, `cflow_71_deadloop`, `cflow_148_texbranch`,
  `fvg_filter_image`, `tests.rs:1230, 1240-1242, 1302-1303, 8990, 9013`,
  `tests/mali_int_loops.rs`.

## 8. Open points

- **Why Mesa does not delete `vs_idx_vec_w_dynsub`'s position-program loop**
  is read, not proven (`node_is_dead` in nir_opt_dead_cf.c:156-260 has the
  conditions; the body's `load_output` is the suspect). If S11 finds a
  second loop of that kind, derive the rule from the NIR, not from one
  reference.
- **The three-copy swap of `fs_for_uni_sequence`** and the `MOV.i32`
  placements of the nested loops are `bi_out_of_ssa` order. If a compare differs
  only in these copies, fix `ra.rs`, not the loop code.
- **`fs_idx_tmp_vec4_cd`** (4 cases) needs thread-local scratch arrays: out
  of this plan's items.
- **`vs_fn_after_seq`/`vs_fn_mixed`** (S8) are functions with a return after a loop with
  constant-bound counters Mesa keeps (14 and 20 instructions); their loop form is
  not one of the eleven shapes and is the first thing S8 should draw the NIR of.
- **S2, S7 and S11 (156 cases: 94 + 44 + 18) read local arrays at a run-time
  index** (`arr[ui_zero]`, `tmp[ui_one]`): both walks still raise
  `IndexedLocal` for an indexed `LoadLocal`/`StoreLocal` outside the forms
  case 80 (`mali-indexed-locals.md`) made byte-equal (`nir_fs.rs:5630`,
  `nir.rs:4127`, `:4546`). The loop text leaving does not make those cases
  compile; their next refusal is not measured (loops note 9.8). Run one
  `fs_idx_tmp_float_dd` through the walk with the loop lowered before
  counting S2.
- The vertex `UniformType` dependency (97 hidden cases) and the dynamic bound
  (61 fragment cases) are other items' lowering; every number above counts
  only the loop text.
