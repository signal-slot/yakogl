<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali loops: the references A0 and S8 need (plan #387 item loops3-ref)

`docs/notes/mali-loops-lowering.md` section 4 (A0) and section 8 (S8) ask for
Mesa 26.2.1 references before the lowering is written. They are
`tools/mali/testdata/deqp_loops3_*`, drawn with
`tools/mali/deqp_ref.sh <vert> <frag> <prefix> --instrument shim` and pinned
by `tests/mali_deqp_loops3.rs` (all three refused at the tip, none compared
yet).

| reference | shader | refusal at the tip | step |
| --- | --- | --- | --- |
| `fs_for_highp_40` | `precision highp float; varying vec4 v; ... vec4 c = v; for (int i = 0; i < 40; i++) c = c * v; gl_FragColor = c;` (partner: `a_position`, `a_coords` to a highp `v`) | `LoopForm` | A0 |
| `vs_fn_after_seq` | dEQP `functions.control_flow.return_after_loop_sequence_vertex` | `LoopVertex` | S8 |
| `vs_fn_mixed` | dEQP `functions.control_flow.mixed_return_break_continue_vertex` | `LoopVertex` | S8 |

The two `vs_fn_*` pairs have the sources of `deqp_loops2_vs_fn_*` (the loops2
references stay as they are). The NIR dumps below are Mesa's, from
`tools/mali/drm_shim_ref.sh run <work> vs fs <prefix> --uniform-values`
(`.stderr`, the second `shader: MESA_SHADER_VERTEX` / `FRAGMENT` dump, the one
after the link; the first vertex dump is the position half of the IDVS split).

## 1. `fs_for_highp_40` (A0)

Mesa keeps the loop: 40 iterations of a vec4 product exceed its unroll limit,
so the fragment NIR is one `loop` with a counter and no other control flow.

```text
loop {
  b1:  phi x4 (the vec4 c, init = v.xyzw), phi counter (init 0)
       c' = fmul c, v           (four scalar fmul, each with the varying lane)
       i' = iadd counter, 1
       %19 = ige_pan i', 40
       if %19 { break } else { }
}
b5: f2f16 x4 (the colour, FRAG_RESULT_DATA0 is mediump), atest, blend
```

Valhall words (one block `b1` per pass, no counter in the partner's words):

```text
block0: LD_VAR_BUF_IMM.f32.v4 r0:r1:r2:r3 ; r4..r7 = MOV r0..r3 ; r8 = IADD_IMM u256 (0)
block1: r8 = IADD_IMM r8, 1 ; r7 = FMA r7, r3, -0 ; r6 = FMA r6, r2, -0 ;
        r5 = FMA r5, r1, -0 ; r4 = FMA r4, r0, -0 ;
        r9 = ICMP_OR.s32.ge.m1 r8, u2 (= 40), 0 ; BRANCHZ.i16.eq r9.h00, 0 -> block3
block2: BRANCHZ 0 -> block5            (the break)
block3: (empty) -> block4 ; block4: BRANCHZ 0 -> block1   (the back edge)
block5: four FADD.f32 .h00 (f32 to f16) ; ATEST ; MKVEC.v2i16 x2 ; BLEND.f16
```

Facts for A0: the counter bound 40 is a uniform word (`u2`), the product is
`FMA(acc, v, -0.0)` (not `FMUL`), the loop test is `ICMP_OR.s32.ge.m1` with the
branch at the end of the body and an empty fallthrough block, and the final
`f2f16` of the highp value is a `FADD.f32 .h00 x, -0.0`. `0 loops` in the
shaderdb line is Mesa's own counter, not a statement about the code. The pin
at `tests.rs` stays `LoopForm` until A0 lands; when it does it moves to this
reference (`deqp_loops3_fs_for_highp_40`) as the byte compare.

## 2. The loop form of the S8 shapes

Both functions are inlined by Mesa's GLSL front end (no call at run time), and
what is left in NIR is **one kept `loop` in the vertex shader** whose bound is
a constant, the counter a phi, and the early exits `break`s to one merge block
with phis for the function's result. This is not one of the eleven forms of
`mali-loops-lowering.md` (they are the fragment `for`/`while`/`do` with the
colour or a conditional in the body).

### `vs_fn_after_seq`

```glsl
float func(float a) {
  int i;
  for (i = 0; i < 6; i++) { a = -a; if (i == 4) a = -a; }   // negate a
  for (; i < 10; i++) { if (i == 8) continue; else if (i == 9) break; a = -a; }
  return a;
}
```

Mesa fully unrolls the first loop (6 iterations, `-a` five times folded to
one negate by the `i == 4` double negation: `%2 = fneg in0`), and keeps the
second as the only loop, with the counter's initial value **6** in the phi
(the first loop's exit value, constant-folded):

```text
%2 = fneg in0
loop {
  b1:  %8 = phi b0: 6, b10: %14          (counter)
       %9 = phi b0: %2, b10: %13         (a)
       %10 = ine_pan %8, 8
       if %10 {                          (i != 8: not the `continue` pass)
         b2: %11 = ieq_pan %8, 9
             if %11 { break }            (i == 9)
         b5: %12 = fneg %9
       } else { }
  b7:  %13 = phi b5: %12, b6: %9
       %14 = iadd %8, 1
       %15 = ige_pan %14, 10
       if %15 { break } else { }
}
b11: %16 = phi b3: %9, b8: %13           (the two breaks)
     store %16
```

So: a kept loop, **two breaks** (the `i == 9` break inside the `i != 8`
branch, the step's break at the bottom), a `continue` turned by Mesa into the
`i != 8` guard, the counter starting at a folded constant, three bounds as
uniform words (`u261.b0..b2` = 8, 9, 10), and the exit through a phi block.
The words: `ICMP_OR.u32.ne.m1` / `eq.m1` / `ICMP_OR.s32.ge.m1` for the three
tests, `BRANCHZ.i16.eq` to the merge, `FADD.f32 x.neg, -0.0` for the negate
(Mesa has no `FNEG` word here), `IADD_IMM.i32 r, 1` for the step. After
the scheduler pass Mesa also rewrites block 2's branch to a `.ne` branch to
the merge block and drops its empty fallthrough block (block 3), so the final
CFG has the `break`s as direct conditional branches into block 11.

### `vs_fn_mixed`

```glsl
float func(float a) {
  for (int i = 0; i < 6; i++) {
    if (i == 0) continue;
    else if (i == 1) { }
    else if (i == 3) break;
    else return a;
    a = -a;
  }
  return 1.0;
}
```

Mesa keeps the loop with counter init **0** and bound 6 and compiles the
`return a` as a break whose merge block selects the result:

```text
loop {
  b1:  %5 = phi 0, %12 (counter) ; %6 = phi in0, %11 (a)
       %7 = ine_pan %5, 0
       if %7 {                           (i != 0)
         b2: %8 = ine_pan %5, 1
             if %8 {                     (i != 1)
               b3: %9 = ine_pan %5, 3    (the flag the merge reads)
                   break                 (`i == 3` and `else return a`: one break)
             } else { }
         b5: %10 = fneg %6
       } else { }
  b7:  %11 = phi b5: %10, b6: %6
       %12 = iadd %5, 1
       %13 = ige_pan %12, 6
       if %13 { break } else { }
}
b11: %14 = phi b3: %9, b8: 0              (return flag: i != 3, i.e. the `return a`)
     %15 = phi b3: %6, b8: %11            (a)
     %17 = bcsel_pan %14, %15, 1.0        (return a if the flag, else 1.0)
     store %17
```

So: one kept loop, **two breaks** as in after_seq, the `i == 3` break and the
`return a` collapsed by Mesa into the same exit (the flag is the compare
itself, `i != 3`), the loop's *fall-through* exit supplying flag 0, and the
result selected after the loop by `CSEL.u32.eq flag, 0, 1.0, a`
(`r0 = CSEL.u32.eq r1, u256, u264, r0`). Both `else if` bodies with no code
(`i == 1`) become the bare `if (i != 1)` skip.

### What the two share, for S8

- one kept loop with a **constant bound and counter phi** (the walk's pass
  counter form), `IADD_IMM` step, `ICMP_OR.<s|u>32.<ge|ne|eq>.m1` tests;
- the body's `if`/`else if` chain becomes **nested `if (cond) { ... } else { }`
  with empty else blocks**, each leaving empty blocks in the CFG (Mesa's
  `BRANCHZ ... -> block` to an empty block and a `BRANCHZ 0 -> next`);
- every early exit (`break`, `return` in the loop) is a `break` to one merge
  block `b11` whose phis carry the function's value (`%16`, or `%14`/`%15` with
  the select);
- the float negate is `FADD.f32 x.neg, -0.0`, **not** the source's `-a`;
- the first loop of `vs_fn_after_seq` is unrolled and folded before the
  second is compared: the lowering needs the constant fold of the counter's
  exit value (6) as the phi initial value.

Neither shape has a `continue` after Mesa's lowering (it is the guard) and
neither a loop-carried value other than `a` and the counter, so the
`LoopVertex` lowering of `loops-lower-2` can reuse the fragment walk's
single-loop form once the vertex path takes the break-to-merge-with-phis
shape; the `bcsel` after the loop in `vs_fn_mixed` is the extra piece.

## 3. Caveats

- The vertex words of these shapes are in the references (the `.expect.json`
  regions); the position half (IDVS) is the same as every other
  `deqp_*` vertex reference.
- `vs_fn_*` fragment partners are dEQP's comparing shader, no loop.
- The `deqp_loops2_vs_fn_*` pairs are unchanged; `deqp_loops3_vs_fn_*` are the
  same sources drawn again so that the loops3 test is self-contained.
