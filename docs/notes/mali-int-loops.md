<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali Valhall: Mesa's lowering of integer loop counters, int comparisons and loops that end at a run-time condition

Plan #153, item acceptcheck (a study; nothing under `src/` changed). Two groups
of shaders in `tools/mali/corpus-verdicts.txt` are still refused:

1. `Valhall: an integer or boolean value is not lowered yet` (`MaliError` for
   an int/bool IR value): es100 `loop_nested*.frag`, `loop_nested.vert`,
   `loop_uniform*`, `loop_full.frag`, `contloops.frag`, `ifloop.frag`,
   `dynidx*.frag`, `dynvec4.frag`, `idiv.frag`, `int_compare.frag`,
   `intlit.frag`, `tri_int.frag`, `cond_complex.frag`, `cond_sequence.frag`,
   `affectand.frag`, `ret_nested.*`, `ret_dynloop_never.frag`,
   `uniform_idx.frag`, es300 `array.frag`, gltest `f1_fs.frag`, `f3_fs.frag`.
2. `Valhall: a loop other than one a constant bound ends within 32 passes`:
   es300 `divnest.frag`, `loop_exit.frag`.

## References

Mesa 26.2.1, panfrost on drm-shim (GPU_ID 0xa867), one draw per pair with
`tools/mali/drm_shim_ref.sh run <shim> V F <prefix>` (`mesa-shim` from
`$CONNECTED_DEPS_DIR`; no `--uniform-values`, so every uniform word of the
FAU tables is 0) and
`python3 -I tools/mali/pan_ref_diff.py extract <prefix>.trace --draw >
tools/mali/testdata/intloop_<name>.expect.json`. The sources are copied next to
the JSON as `intloop_<name>.{vert,frag}`. The NIR and disassembly quoted below
are the driver's own `BIFROST_MESA_DEBUG=shaders,shaderdb` print (the
`.stderr` of the run), the final NIR of the fragment (or varying) shader before
the Bifrost back end.

| name | vertex | fragment | stage of interest |
|---|---|---|---|
| `loop_nested` | `es100/dynidx.vert` | `es100/loop_nested.frag` | fragment |
| `loop_nested_vs` | `es100/loop_nested.vert` | `es100/va.frag` | vertex (varying program) |
| `contloops` | `es100/tri_int.vert` | `es100/contloops.frag` | fragment |
| `ifloop` | `es100/tri_int.vert` | `es100/ifloop.frag` | fragment |
| `dynidx` | `es100/dynidx.vert` | `es100/dynidx.frag` | fragment |
| `idiv` | `intloop_plain.vert` (position only) | `es100/idiv.frag` | fragment |
| `int_compare` | `es100/tri_int.vert` | `es100/int_compare.frag` | fragment |
| `tri_int` | `es100/tri_int.vert` | `es100/tri_int.frag` | fragment |
| `f1_fs` | `intloop_plain.vert` | `gltest/f1_fs.frag` | fragment |
| `divnest` | `es300/bands.vert` | `es300/divnest.frag` (ES 3 mode) | fragment |
| `loop_exit` | `es300/loop_exit.vert` | `es300/loop_exit.frag` (ES 3 mode) | fragment |

The vertex pairs are the ones the sweep links (or a plain pass-through where the
fragment shader reads no varying, `intloop_plain.vert`); the word-level
vertex programs in the JSON are those pairs', not the subject. Every program
is `work_reg_count` 32 with preload `0x3000...` (fragment) or `0x3800...`
(position/varying), the draw-time values the trace prints for any shader here.
`idiv` is also recorded by study-intbig (`intbig_case79`, with the sweep's
pass-through vertex shader); the fragment program is the subject of both.

Two measures of size are given. "shaderdb" is Mesa's own line in the
`.stderr` (`N instrs, ... L loops, ... R registers used, U uniforms used`);
its register count includes the preloaded registers and its uniform count is
the FAU slots it pushes. "rK" and "FAU words" are read from the committed JSON:
the highest register the program's text names (`r60`/`r61` aside) and the
entries of the `fau` table past the four reserved words of `u0`/`u1`.

## Summary

| shader | Mesa's loops after the loop passes | program size (shaderdb regs/uniforms; JSON rK, FAU words) | int/control ops it selects | refusal removed |
|---|---|---|---|---|
| loop_nested | **kept, 9** (2+3+2+2; every bound is a uniform) | 90 instrs, 12/4; r0..r8, 4 FAU words | `ICMP_OR.s32.ge.m1`, `IADD_IMM`, `MKVEC.v2i16`, `BRANCHZ` | int value (uniform-bound int loop counters) |
| loop_nested_vs | **kept, 4** (varying program) | 53 instrs, 10/7; r0..r6 (+7 fragment) | same, in the vertex stage | int value (`loop_nested.vert`) |
| contloops | for(i<8) and the two bounded do/while **unrolled**; `while (k < n)` **kept, 1** | 72 instrs, 11/5; r0..r7, 6 FAU words | `F16_TO_S32.rtz`, `ICMP_OR.{s32.lt,s32.ge,u32.eq}.m1`, `MUX.v2i16`, `U16_TO_U32` | int value |
| ifloop | **fully unrolled, 0**; folded to constants | 10 instrs, 7/3; r0..r3 | none (no int op survives) | int value (counter folds) |
| dynidx | none | 65 instrs, 10/7; r0..r6, 10 FAU words | `ICMP_OR.{u32.eq,s32.lt}.m1`, `MUX.v2i16`, `MKVEC.v2i16`, `IADD.v2u16` | int value (int uniform subscript) |
| idiv | none | 181 instrs, 13/6; r0..r10, 8 FAU words | `IMUL.i32`, `IABS.s32`, `U16_TO_U32`, `FRCP.f32`, `CSEL.u32.eq`... | int value (integer division) |
| int_compare | none | 20 instrs, 7/3; r0..r3 | `F16_TO_S32.rtz`, `ICMP_OR.{s32.lt,s32.ge}.m1`, `MUX.v2i16` | int value |
| tri_int | none | 32 instrs, 8/5; r0..r4, 6 FAU words | `IMUL.i32`, `ISUB.s32`, `S32_TO_F32`, `ICMP_OR.u32.{eq,ne}.m1`, `CSEL.u32.eq` | int and bool values |
| f1_fs | none | 9 instrs, 6/3; r0..r3 | none (a `TEX_SINGLE` at a constant coordinate) | int value (all of it constant-folds) |
| divnest | **kept, 2** (nested, with continue and a break out of both) | 57 instrs, 9/4; r0..r5, 4 FAU words | `S32_TO_F32`, `FCMP_OR.f32.ge.m1`, `ICMP_OR.u32.ne.m1`, `BRANCHZ` | the "loop other than ..." refusal |
| loop_exit | **kept, 1** (run-time bound) | 23 instrs, 10/4; r0..r6, 4 FAU words | `F32_TO_S32.rtz`, `ICMP_OR.s32.ge.m1`, `S32_TO_F32`, `BRANCHZ` | the "loop other than ..." refusal |

"Kept, N" is the number of back edges (`BRANCHZ.eq.reconverge 0x0,
offset:-M`) in the program's words, equal to shaderdb's `loops` for every
program here; "unrolled" means the loop left none.

The finding that matters for group 2: **the refusal's own text is wrong for
these two shaders**. It says Mesa "unrolls with guards" (true for
`case87_databreak`, whose loop has the constant bound `i < 8`: see
`mali-loop-break.md`). `divnest` and `loop_exit` have no constant cap (the
bound is `floor(v_s * 8.0)` or `int(clamp(r,0,1) * 5.0)`, a value read at run
time), so NIR cannot unroll them and Mesa emits a real loop. Group 1's loops
are the same: a bound that is a uniform or a varying keeps the loop.

## How to read the Valhall output

* The loop is `BRANCHZ` plus compare-into-a-register. A loop's header test is
  `ICMP_OR.s32.ge.m1 rC, rI, rBound, 0x0` and the exit is `BRANCHZ.reconverge
  rC.h0, offset:N` (branch when the compare is **non-zero**, i.e. the loop is
  done); the back edge is `BRANCHZ.eq.reconverge 0x0, offset:-M` (branch when
  `0x0 == 0`, always). `offset` is the signed 24-bit field at bits 8..31 of
  the word's low dword, counted in instructions from the one after the
  branch (loop_exit word 7: `...0546` = src `0x46` (r6 discard), offset 5, a
  jump from instruction 8 to 13; word 12: `...fffff9c0` = offset -7 back to
  the compare at 6). The `.reconverge` flag is carried by the instruction that
  opens the loop and on both branches (loop_exit words 5, 7 and 12); the
  words are in `intloop_loop_exit.expect.json` and should be matched
  byte-for-byte, the flag is not derived here.
* Loop-carried values are plain registers: the NIR phis are resolved by
  writing the next trip's value into the register the header reads
  (`r4 = FADD.f32 r4, r6`, `r5 = IADD_IMM.i32 r5, #0x1`, no `MOV` per trip).
  Phis whose value comes from different breaks (divnest's `%27`, `%28` at the
  loop exit) are resolved by `MOV`s on each edge before its branch.
* A zero constant for a loop init is `IADD_IMM.i32 rN, 0x0, #0x0` (also the
  way a 32-bit float constant that is not an FAU word is materialised:
  `IADD_IMM.i32 r4, 0x0, #0x40C00000` in divnest).
* Integer uniforms are **not** loaded: each is a push-uniform word of the FAU
  (`u2.w0`, `u2.w1`, `u3.w0`, ... in declaration order from the first
  non-reserved slot, `u0`/`u1` are zero here). loop_nested: `ui_one` = `u2.w0`,
  `ui_two` = `u2.w1`, `ui_three` = `u3.w0`, and `u3.w1` is the constant pair
  `0x3c003c00` (1.0 f16) the epilogue uses. The words the trace prints for
  them are 0 (state); the constants are in the JSON's `fau`.
* Small integer literals come as byte lanes of packed FAU constants:
  `0x3020100.b3` is 3, `.b2` is 2, `0x1000000.b3` is 1, `0x7060504.b2` is 6,
  `0xF0E0D0C.b0` is 12 (tri_int, contloops); `u256`/`0x0` is zero. A literal
  that fits no lane is a full constant or an `IADD_IMM` immediate.
* mediump `int` is a 32-bit value (`.i32`, `bit_sizes_int: 0x31`/`0x30` show 16
  as well because the f16 lanes exist); mediump `float` arithmetic around the
  ints is `v2f16`, so an int compare of a mediump value feeds a `MUX.v2i16`
  that picks the f16 pair. A float to int is `F16_TO_S32.rtz rD, rS.h0` (or
  `.h1`), highp is `F32_TO_S32.rtz`; int to float is `S32_TO_F32`.

## loop_nested: kept loops with uniform bounds (9)

`loop_nested.frag` is four dEQP nests: nested, nested_sequence, and the two
tricky-dataflow forms, each with bounds from `ui_one/two/three`. NIR keeps all
nine `loop {}` in order (2 + 3 + 2 + 2) and they are all
`ige_pan %counter, %bound` / `break` / `iadd %counter, 1` with the carried
`vec4` as two `vec2` f16 phis. The `yzwx` swizzle is two `vec2` rebuilds:

```
loop { %12 = phi b4: %6, b8: %20 ; %13 = phi b4: %7, b8: %19 ; %14 = phi 0, %18
       %15..%16 = load_ubo (ui_three)   ; %17 = ige_pan %14, %16 ; if %17 break
       %18 = iadd %14, 1 ; %19 = vec2 %13.y, %12.x ; %20 = vec2 %12.y, %13.x }
```

Selected (the middle nest, one inner pass of the body):

```
IADD_IMM.i32.reconverge r5, 0x0, #0x0      ; i = 0
ICMP_OR.s32.ge.m1 r6, r5, u3.w0, 0x0       ; i >= ui_three
BRANCHZ.reconverge r6^.h0, offset:20       ; exit
  IADD_IMM.i32.reconverge r6, 0x0, #0x0    ; inner j = 0
  MOV.i32 r7, r4^
  ICMP_OR.s32.ge.m1 r4, r6, u2.w1, 0x0
  BRANCHZ.reconverge r4^.h0, offset:4
    MKVEC.v2i16 r4, r7.h1, r3.h0           ; res = res.yzwx
    MKVEC.v2i16 r3, r3^.h1, r7^.h0
    IADD_IMM.i32 r6, r6^, #0x1
    BRANCHZ.eq.reconverge 0x0, offset:-7
```

The uniform bounds are FAU words read directly by the compare (no register
copy); three int uniforms use `u2.w0`, `u2.w1`, `u3.w0`. Nothing is unrolled,
duplicated or hoisted: the "tricky dataflow" outer loops, whose body
overwrites `res`, are still run (the compiler cannot know the trip count is
at least one). 90 instructions, r0..r8.

`loop_nested_vs` is the vertex twin (`loop_nested.vert`): 4 loops in the
**varying** program (the position program is the plain 24 instructions),
53 instructions; its uniform words are `u5.w1` (ui_two), `u6.w0` (ui_three),
the same compare and `MKVEC.v2i16` pairs, then `F16_TO_F32` and the usual
`STORE.i32.estream` sequence of a varying program. Its fragment (`va.frag`) is
a seven-instruction pass-through.

## contloops: a mix, decided per loop by whether its bound is a constant

`n = int(v_color.w * 4.0 + 3.5)` (`F16_TO_S32.rtz` of `FMA.v2f16 .h11 * 4 + 3.5`).

* `for (i < 8) { if (i >= n) break; if (i == 1) continue; res = res.yzwx; }`:
  the bound 8 is constant, so NIR unrolls it completely and the `break`
  becomes a ladder of `ilt_pan 7, n`, `ilt_pan 6, n`, ... `ilt_pan 0, n`
  selects, nothing left of `i`: seven `ICMP_OR.s32.{ge,lt}.m1` against byte
  constants (`0x7060504.b3` ... `0x3020100.b2`, then `0x0`; words 5-33), each
  followed by `MUX.v2i16`s over the f16 pairs. (The same mechanism as `mali-loop-break.md`, and the one our
  back end already has for a constant bound.)
* `while (k < n) { k++; if (k == 2) continue; res.x += 0.125; }`: the bound is
  `n`, so it stays a loop (the only one in the program, `1 loops`):

  ```
  IADD_IMM.i32.reconverge r3, 0x0, #0x0       ; k = 0 (the loop's own counter)
  ICMP_OR.s32.ge.m1 r7, r3, r1, 0x0
  BRANCHZ.reconverge r7^.h0, offset:13
    FADD.v2f16 r7, r0.h00, u2.w1                ; res.x += 0.125 (as a pair)
    MKVEC.v2i16 r0, r0^.h0, r2.h0 ...           ; the select for k == 2:
    IADD_IMM.i32 r3, r3^, #0x1
    ICMP_OR.u32.eq.m1 r6, r3, 0x3020100.b2, 0x0 ; (k+1) == 2
    MUX.v2i16 r0, r2^, r0^, r6.h00              ; continue = keep the old value
    MUX.v2i16 r4, r4^, r4^, r6^.h00             ; (a degenerate select, as dumped)
    U16_TO_U32 r2, r0.h1 ... U16_TO_U32 r4, r4^.h0   ; widen the carried pairs back
    BRANCHZ.eq.reconverge 0x0, offset:-15
  ```

  `continue` is a select on the loop-carried values (no branch). The widen
  `U16_TO_U32`s are the phi copies of the f16-pair carried values (NIR keeps
  the carried `vec4` as four 16-bit phis, `%84..%87`).
* `do { continue; } while (++j < 3);` folds to `j = 3`: `float(j) * 0.25` is
  the literal `0x3a00` in the blend source. No loop, no int.
* `do { if (m == 4) break; if (m == 1) continue; res.y += 0.0625; } while
  (++m < n);` is unrolled the same way as the first loop (m runs at most 5
  times, then the compare against `n`), selects over `ige_pan 3, n`, `ilt 2, n`
  of the pairs `%109`.. `%127`.

72 instructions, r0..r7, 6 FAU words.

## ifloop: a counted loop whose counter folds away

`do { if (i < 2) res = res.yzwx; } while (++i < 3);` has the constant trip count
3, so it unrolls and the compare folds: two rotations, i.e. the colour is
`(z, w, x, 1)`. The whole fragment program is
`LD_VAR_BUF_IMM`, `V2F32_TO_V2F16 r1, r2, r3` (z, w), a `FADD.f32 r0.h0, r0,
-0` (the x lane as f16), `ATEST`, `MKVEC.v2i16` and `BLEND`: 10 instructions,
no int op, no branch (the one `BRANCHZI` is the blend epilogue's return). The refusal this shader hits is a value-level one (an int
counter and its compare), and it disappears by constant-folding the loop
before lowering, which our "constant bound within 32 passes" path already
does for the loops that have no int *value* left. What is left to do for
ifloop is to unroll without ever materialising `i` as an IR value, so the
`i < 2` compare is not seen by the lowering as a value.

## dynidx: a dynamic vector subscript is a compare ladder

`tmp[ui_zero] = coords.x; ...; res += tmp[ui_one] * 10.0` ...: no loop. NIR turns
each dynamic write and read of `tmp[k]` into `ieq_pan k, 0/1/2/3` and
`ilt_pan k, 3/1/2` selects over `bcsel_pan` of the f16 lanes (`ffma` into the
sum), and that is what is selected:

```
ICMP_OR.u32.eq.m1 r4, u2.w1, 0x1000000.b3, r3   ; ui_one == 1 ...
ICMP_OR.u32.eq.m1 r5, u2.w1, 0x0, 0x0           ; ui_one == 0
MKVEC.v2i16 r4, r5^.h0, r4^.h0                  ; the two masks as an f16 pair
MUX.v2i16 r4, r0.h00, r2.h00, r4^               ; select between two f16 lanes
...
ICMP_OR.s32.lt.m1 r1, u2.w0, 0x3020100.b3, r3   ; read index < 3
MUX.v2i16 r1, r0.h11, r0.h00, r1^.h00
```

65 instructions, r0..r6, 10 FAU words (the four int uniforms are
`u2.w0..u3.w1`; the constants `0.25/0.125`, `10`, `100`, `1000`-as-f16 follow
as `0x30003400`, `0x49004900`, `0x56405640`, `0x63d063d0`, and the 1.0 pair).
Equality uses `.u32.eq`, the ordering `.s32.lt`, as the uniform's type
demands; `m1` is the mask-out-as-one form, the same as for the loop compares.
The vertex-side `dynidx.vert` is a pass-through. The same ladder is what
`dynvec4.frag` and the dynamic subscripts of `uniform_idx.frag` and
`dynidx_loop.frag` (`tmp[i]` with `i` a loop counter) select: the loop in
`dynidx_loop` has the uniform bound `ui_four` (kept as a loop, like
loop_nested) around a first loop with the constant bound 4 (unrolled).

## idiv: nir_lower_idiv, 55 instructions a division

No loop. `n / d` is Mesa's precise `nir_lower_idiv` sequence on `abs()` of the
operands: `IABS.s32`, `U32_TO_F32`, `FRCP.f32` times `4294967040.0`
(`0x4f7ffffe`, an FAU word), `F32_TO_U32.rtz`, one Newton-style refinement
with a 32x32 -> high-32 multiply emulated by four 16x16 `IMUL.i32` on
`U16_TO_U32` halves (about 20 instructions), a remainder `IMUL` + `ISUB.s32`,
one `ICMP_OR.u32.ge` + `CSEL.u32.eq` correction (twice, as in the NIR
`uge_pan`/`bcsel_pan` pair), the sign by `LSHIFT_XOR.i32` and an
`ICMP_OR.s32.lt.m1` on the xor, `ISUB.s32 0, q` and a final `CSEL.u32.eq`.
`idiv.frag` has three divisions (`q.x`, `q.y` and `100 / (...)`), so 181
instructions, r0..r10, 8 FAU words (the `u_d` words `u2.w0..u3.w1`, then
the 10.0 pair, `0x4f7ffffe`, the integers `1` and `100` as words, `0.0625`
and the 1.0 pair). Int literals here come as FAU words
(`u4.w0 = 1`, `u4.w1 = 100`) and the `u2.w0`/`u3.w0` f16 uniforms feed the
`F16_TO_S32`.

## int_compare and tri_int: compare-and-select, no branch

`int_compare`: `i = int(3.0 * r)` is `FMA.v2f16 ... ; F16_TO_S32.rtz .h00 / .h11`;
each ordering is one `ICMP_OR.s32.{lt,ge}.m1` and a `MUX.v2i16.int_zero 0,
1.0, mask` (the `?: 1.0 : 0.0` select). The last lane `i <= 1 ? 1.0 : 0.25`
is `ICMP_OR.s32.ge.m1 0x1000000.b3, i` (the constant on the left: `1 >= i`)
into a `MUX` between two f16 constants. 20 instructions, r0..r3: the
whole shader is the table above and the four selects.

`tri_int`: `i = int(r * 4.0)` is `FMA.v2f16` then `F16_TO_S32.rtz`; `j = -i*3 + 7`
is **`IMUL.i32` by the byte constant 3 and `ISUB.s32 12, x`** (NIR folds `-i*3 + 7
+ 5` into `12 - 3i`), `S32_TO_F32` and an `FMA.f32` by 1/12; `bool e = i == 1`,
`n = i != 2` are `ICMP_OR.u32.{eq,ne}.m1` and `CSEL.u32.eq rD, mask, 0, 0,
1.0f` (`B2F`, a 32-bit select to float), summed by `FMA.f32 e, 0.5, 0.125` and
`FMA.f32 n, 0.25, ...`. The `sign()` is two `FCMP_OR.v2f16.lt.f1` and a
`FADD.v2f16` of the pair. 32 instructions, r0..r4, 6 FAU words.
These two together show every int/bool op the others use besides division,
loops and the dynamic subscript.

## f1_fs: nothing remains

`gltest/f1_fs.frag` is dead integer code (global initialisers, constants, a
dead `if`) around `texture2D(l, inversesqrt(vec2(k, m)))`, where `k` is a
constant boolean and `m` a constant int. Mesa folds all of it, including the
coordinate: `%0 = load_const (1.0, 0.5)` and a single `tex`:

```
MOV.i32 r0, 0x3F800000 ; MOV.i32 r1, u2.w0 ; MOV.i32 r2, u2.w1 ; MOV.i32 r3, 0x0
TEX_SINGLE.slot0.skip.reserved.16.2d.computed.wait0126 @r0:r1, @r0:r1, [r2^:r3^]
ATEST ... r1.h1 ; BLEND.slot1.v4.f16.end ...
```

9 instructions, r0..r3. `f` (the `int` uniform) is read by nothing.
Removing the refusal needs constant folding of int/bool *values* only (the
shader never needs an int op at run time); `f3_fs.frag` is the same family
(not recorded). The texture words are not this item's subject (they follow
the texture-refusal notes).

## divnest and loop_exit: real loops (the second group)

`loop_exit.frag`, 23 instructions, r0..r6:

```
loop { %8 = phi 0, %13 ; %9 = phi 0, %14 ; %10 = ige_pan %9, %6 ; if %10 break
       %11 = i2f32 %9 ; %12 = fadd %2.y, %11 ; %13 = fadd %8, %12 ; %14 = iadd %9, 1 }
```

`%6 = f2i32 (fsat(r) * 5)` is `FADD.f32.clamp_0_1`, `FMA.f32`, `F32_TO_S32.rtz`;
the loop is exactly the five-instruction body of the listing in the section
"How to read": `S32_TO_F32`, `FADD`, `FADD`, `IADD_IMM +1` and the back
branch, with the header `ICMP_OR.s32.ge.m1` + exit `BRANCHZ`. After the loop:
`FMA.f32 s, 1/16`, `FROUND.f32.rtz` for `float(n)`'s `ftrunc`, the `BLEND`.
The whole program (words 0..22) is in `intloop_loop_exit.expect.json`.
There is no cap: n is 0..5 in the draw, but the program is the same for any
n, and no guard or flag appears (the "capped, with an exit flag" of the shader's comment is the v3d
back end's form, not Mesa's here).

`divnest.frag` (highp, ES 3): two nested loops whose bound `b = floor(v_s *
8)` is a run-time value, with `continue` and a `break` that leaves the inner
loop and a `break` that leaves the outer one. NIR:

```
loop { phi n, outer ; %10 = i2f32 i ; %11 = fge_pan %10, b ; if %11 break (b22)
       %12 = i + 1 ; if (i != 2) { loop { %16 = i2f32 j ; %17 = fge_pan %16, b ; if %17 break (b16)
                if (j != 1) { %21 = fge_pan %16, b - 2 ; if %21 break (b16) ; n += 1 }
                j += 1 }
              if (n >= 6.0) break (b22) } }
```

Selected (57 instructions, r0..r5, 4 FAU words): the float compare of the
converted counter is `S32_TO_F32` + `FCMP_OR.f32.ge.m1` (a counter is compared in
float because the source says `float(i) < b`), the `i != 2` / `j != 1` tests
are `ICMP_OR.u32.ne.m1` against `0x3020100.b2` and `0x1000000.b3`, `continue`
is a branch over the rest of the body (`BRANCHZ.eq.reconverge`, no select
here: there is no value to keep), and the two `break`s to the exit carry the
phi copies (`MOV.i32 r0, r1^ ; MOV.i32 r1, r2^` before the
`BRANCHZ.eq.reconverge 0x0, offset:22` of the first, words 7-9, and the same
pair before the one at the outer loop's `n >= 6.0` test, words 28-30). The constants `6.0` and `2.0`, `1.0` are
materialised by `IADD_IMM.i32 r3, 0x0, #0x40C00000` and `FADD_IMM.f32` (imm
`0xC0000000`, `0x3F800000`) because they are not FAU words. The epilogue
`mod(n,16)` etc. is `FMA.f32`, `FROUND.f32.rtn`, `FMIN` (`FROUND.f32.rtn` is
`floor`). Two loops (back edges at words 24 and 31), eight forward branches
(words 6, 9, 12, 16, 18, 21, 27, 30); the structure is
`if (i != 2) { inner loop; if (n >= 6) break; }` inside the outer loop.

## Not recorded (classified by source)

Their Mesa programs were not recorded here; each has the shape of a recorded
one, which is where their refusal is removed:

| shader | shape | like |
|---|---|---|
| `loop_nested.while/.dowhile.frag`, `loop_uniform.frag`, `loop_uniform.dowhile.vert` | uniform-bound int loops | loop_nested |
| `loop_full.frag` | bounded loop with a conditional break | contloops |
| `intlit.frag`, `cond_complex`, `cond_sequence`, `affectand` | int/bool literals and short-circuit logic, folded | int_compare + f1_fs |
| `dynvec4.frag`, `uniform_idx.frag`, `dynidx_loop.frag` | dynamic index of a vector / uniform array | dynidx (and `mali-uniform-index.md`) |
| `ret_nested.*`, `ret_dynloop_never.frag`, `discard_dynloop.frag` | early `return` / discard inside loops | contloops (return = exit with a flag) and `mali-discard.md` |
| `es300/array.frag`, `gltest/f3_fs.frag` | `textureSize`/`texelFetch` ints; constant ints | `mali-texture-lookups.md`, f1_fs |

## Recommended lowering order for our back end

Each step is complete on its own and removes the named refusals; no step needs
a later one.

1. **Int values as 32-bit registers, FAU words for int uniforms and byte-lane
   literals** (`.b0..b3` of packed constants, `IADD_IMM` immediates,
   `IADD_IMM rN, 0, #imm` for any 32-bit constant). Conversions
   `F16_TO_S32.rtz` / `F32_TO_S32.rtz` / `S32_TO_F32`, and `IADD`, `ISUB.s32`,
   `IMUL.i32`. Bit-compare against `tri_int`'s words. Removes: the
   value-level refusal for `tri_int`, `int_compare` (with 2), `cond_*`
   when folded.
2. **Compare and select**: `ICMP_OR.{s32,u32}.{lt,ge,eq,ne}.m1` into a register,
   `MUX.v2i16` (f16 pair) and `CSEL.u32.eq` (32-bit, `B2F`), the
   constant-on-the-left form. Removes: `int_compare`, `tri_int`, `affectand`,
   and the compare ladders of `dynidx`/`dynvec4`/`uniform_idx` (`dynidx` is
   then steps 1-2 plus `MKVEC.v2i16`).
3. **Fold int/bool loops with a constant trip count so no int value is lowered**
   (the existing 32-pass unroll, extended to loops whose counter never
   leaves the compile-time domain): `ifloop`, `f1_fs`, `f3_fs`, the constant
   loops of `contloops`/`loop_full`, with run-time breaks as guards
   (`mali-loop-break.md`, done).
4. **Integer division** (`idiv`): the fixed 55-instruction `nir_lower_idiv`
   sequence per division, no control flow; mechanical once 1 and 2 exist.
   Removes `idiv.frag`.
5. **Real loops, smallest first** — `loop_exit` (one loop, one carried float,
   one int counter, header compare + exit `BRANCHZ` + back `BRANCHZ.eq`), then
   `loop_nested` and `loop_uniform` (nesting; carried f16 pairs live in
   registers, `MKVEC.v2i16` for swizzle; FAU-word bounds), then `contloops`'s
   `while`, then `divnest` (a `continue` as a forward branch, a `break` out of
   a nested loop, phi copies on each break edge). The `.reconverge` flag and
   branch offsets are checked against the words of `intloop_loop_exit` first.
   Removes the "loop other than ..." refusal (`loop_exit`, `divnest`), the
   loop part of the value refusal (`loop_nested*`, `loop_uniform*`,
   `contloops`, `loop_full`, `ret_*`) and, with the vertex stage's
   `loop_nested_vs`, vertex loops.
6. **Early `return` and `discard` in loops** (`ret_nested`, `ret_dynloop_never`,
   `discard_dynloop`): after 5, following `mali-discard.md`; not recorded here,
   record `ret_nested.frag` first.

## Caveats

* `m1` and the `.int_zero` MUX form are the printed names; the words are the
  authority. The `ICMP_OR`'s third source `0x0` is the OR operand.
* The `contloops` second `MUX.v2i16 r4, r4^, r4^, r6^.h00` is a no-op select
  Mesa leaves in; a byte-equal back end has to emit it too.
* The three FAU words that hold int uniforms read as zero in the references
  (state); only the constants are compared.
* The two ES 3 pairs: gl_ref.py reports their attributes as `layout(location)`
  ones in declaration order; the position/varying programs are not the subject.
* No `.trace` or `.stderr` is committed; re-record with the recipe in
  "References" (`target/` is not tracked). The `<prefix>`'s basename is the
  JSON's `trace` field (`loop_exit`, not `intloop_loop_exit`): a prefix of
  another name gives a JSON that differs in that one line. The NIR fragments
  and the shaderdb figures quoted above come from those `.stderr` prints;
  words, rK and FAU words are read from the committed JSON.
* Regenerated on 2026-10-25 and again on 2026-10-26 from the committed
  `intloop_*.{vert,frag}` on the `mesa-shim` build of Mesa 26.2.1
  (`$CONNECTED_DEPS_DIR/mesa-shim`): all eleven `expect.json` byte-equal, and
  the shaderdb counts above (instructions, loops, registers, uniforms) are
  those runs'.
* The committed `.frag`/`.vert` are byte-equal to the `tools/shaders/` files of
  the table above (`intloop_divnest.vert` to `es300/bands.vert`); only
  `intloop_plain.vert` has no counterpart there.
