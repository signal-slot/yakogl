<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali Valhall: Mesa's output for the control-flow refusals (gl_test cases 68, 71, 74, 142, 148)

The compiler (`src/shaderc/online/mali`) refuses `Valhall: control flow (if,
a loop, a return before the end) is not lowered yet` in gl_test case 68
`armselect`, 71 `deadloop`, 74 `ifoutput`, 142 `unibranch` and 148
`texbranch`, and in the corpus sources `tools/shaders/es100/{cond_sequence,
frontfacing,ifoutput,ifoutput2,tri_if,texbranch,unibranch,deadloop}.frag` and
`tools/shaders/gl_test_es100/armselect.frag`
(`tools/mali/corpus-verdicts.txt`). This note decodes what Mesa 26.2.1's
panfrost makes of the five cases and sorts them into two lowerings. The
corpus sources `cond_sequence`, `frontfacing` and `tri_if` were not drawn
here; they go with the part their shape belongs to once an item measures them.

## The references

Made again (the earlier `cflow_*` files were in the non-public repository):
Mesa 26.2.1 built with the drm-shim from the extracted tarball
(`tools/mali/drm_shim_ref.sh build`; the build here was a `meson setup` of the
same tree with the script's options and `-Dlibdir=lib64`), then for each pair

```sh
tools/mali/drm_shim_ref.sh run <shim dir> cflow_<n>_<name>.vert cflow_<n>_<name>.frag <out> --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract <out>.trace --draw > cflow_<n>_<name>.expect.json
```

in `tools/mali/testdata/` (`<out>.stderr` is the `BIFROST_MESA_DEBUG=shaders,
shaderdb` dump quoted below). `--uniform-values` makes every uniform word
name itself (uniform k, component c = 16 * (k + 1) + c), as for `builtin_98`.
The sources are the gl_test ones verbatim: `armselect.frag` from
`tools/shaders/gl_test_es100/`, the others from `tools/shaders/es100/`; the
vertex shader is gl_test's passthrough (`case87_databreak.vert`) for 68, 71,
74 and `deriv.vert` for 142 and 148 (`link_program(gl_test_deriv_vs_src, ..)`).
Case 74 draws two fragment shaders, so it has two references:

| reference | case | FS instrs / bytes / regs | branches |
| --- | --- | --- | --- |
| `cflow_68_armselect` | 68 | 11 / 128 / 5 | none |
| `cflow_71_deadloop` | 71 | 29 / 256 / 6 | none |
| `cflow_74_ifoutput` | 74 (if/else) | 10 / 128 / 4 | none |
| `cflow_74_ifoutput2` | 74 (if only) | 10 / 128 / 4 | none |
| `cflow_142_unibranch` | 142 | 61 / 512 / 12 | none |
| `cflow_148_texbranch` | 148 | 542 / 4352 / 10 | 26 `BRANCHZ` on a condition, 4 unconditional |

Every vertex shader is the 24-instruction, 256-byte position program of
`case87_databreak` (no varyings: the varying program is empty).

## Classification

**Mesa flattens every `if` of 68, 74 and 142, and unrolls and flattens the
loop of 71; only 148 keeps branches.** `nir_opt_peephole_select` (limit 64,
`expensive_alu_ok`, the pass `Nir::peephole_select` transcribes) turns an `if`
whose two sides hold only ALU work into `bcsel_pan` at the join; a look-up
under the `if` is not allowed there, so it stays a branch.

| case | Mesa's output | part |
| --- | --- | --- |
| 68 armselect | two `bcsel_pan` pairs, `MUX.v2i16`, 11 instructions, 1 block | flatten |
| 74 ifoutput, ifoutput2 | the output written in both arms: two `bcsel_pan`, 10 instructions | flatten |
| 142 unibranch | a uniform `if` with 24+ operations per arm: still two `bcsel_pan` at the end (the V3D keeps this one as a branch, Mesa's Bifrost limit is 64), 61 instructions | flatten |
| 71 deadloop | the loop unrolled by `complex_unroll`, its guards flattened, the unused work deleted: 29 instructions, `0 loops` | flatten |
| 148 texbranch | the two `if`s and 24 guarded passes keep their blocks: 51 `TEX_SINGLE`, `BRANCHZ.eq.reconverge` | branch |

## Case 68, armselect (flatten)

```glsl
vec4 v = u_a;  vec4 w = u_b;
if (u_flag.x > 0.5) { v = u_b; } else { w = u_a; }
gl_FragColor = v * 0.5 + w * 0.5;
```

NIR after the optimisation loop (`%1` the first UBO row's two 64-bit halves,
`%5` = `u_flag.x`; `%2`, `%3` unpack `u_a`, `u_b`; the `*0.5` sum is gone,
see below):

```
16     %7 = flt_pan %6 (0.500000), %5 // preserve:inf
16x2   %8 = bcsel_pan %7.xx, %3.xy, %2.xy
16x2   %9 = bcsel_pan %7.xx, %3.zw, %2.zw
32    %10 = @load_cumulative_coverage_pan
16    %11 = mov %9.y
32    %12 = @atest_pan (%10, %11)
16x4  %14 = vec4 %8.x, %8.y, %9.x, %9.y
              @blend_pan (%12, %13, %14)
```

Disassembly (one block, 11 instructions, 5 uniforms):

```
MOV.i32.discard r0, 0x0
FCMP_OR.v2f16.lt.m1 r0, 0x38000000.h11, u4.w0.h00, r0^
MUX.v2i16 r1, u2.w0, u2.w1, r0.h00
MUX.v2i16.wait0126 r0, u3.w0, u3.w1, r0^.h00
ATEST.wait0 @r60, r60^, r0.h1, atest_datum.w0
MOV.i32 r2, r0^ ; MOV.i32 r0, r1^ ; MOV.i32.wait r1, r2^
BLEND.slot0.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
```

The selects work on a half pair at once (`MUX.v2i16 ... r0.h00`: the compare
result's low half is the mask of both lanes). Two locals each written by one
arm are two selects, whose `then` and `else` values are the locals' values on
that side (the unwritten side keeps the value it had before the `if`:
`v = u_a`, `w = u_b`). The colour is `v*0.5 + w*0.5` in the source; Mesa has
folded the mix away (the two selected pairs are the colour: with the flag set
`v = u_b`, `w = u_b`, else `v = u_a`, `w = u_a`, so the colour is `u_b` or
`u_a`), which is the `0.5 + 0.5` of equal operands. Our `walk_if16`
(`nir_fs.rs`) refuses it because its join accepts only the bool-lane cases
(`(Const(u32::MAX), Const(0)) => cond` and the like) and a local with two
different defined values returns `MaliError::ControlFlow`.

## Case 74, ifoutput and ifoutput2 (flatten)

```glsl
if (u_c.x > 0.5) gl_FragColor = vec4(1,0,0,1); else gl_FragColor = vec4(0,1,0,1);   // ifoutput
gl_FragColor = vec4(0,1,0,1); if (u_c.x > 0.5) gl_FragColor = vec4(1,0,0,1);        // ifoutput2
```

Both give the same NIR and disassembly (the two `.stderr` dumps show the same
instruction text), so one lowering covers both:

```
16     %4 = flt_pan %3 (0.500000), %2 // preserve:inf
16     %6 = bcsel_pan %4, %5 (0x3c00), %0 (0x0)      // red   = c ? 1.0 : 0.0
16     %7 = bcsel_pan %4, %0 (0x0), %5 (0x3c00)      // green = c ? 0.0 : 1.0
16x4  %11 = vec4 %6, %7, %0 (0x0), %5 (0x3c00)
```

```
MOV.i32.discard r0, 0x0
FCMP_OR.v2f16.lt.m1 r0, 0x38000000.h11, u2.w0.h00, r0^
MUX.v2i16 r1, 0x0, 0x3C000000.h11, r0.h00
MUX.v2i16.wait0126 r0, 0x3C000000.h11, 0x0, r0^.h00
ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
MKVEC.v2i16 r0, r1^.h0, r0^.h0
MOV.i32.wait r1, 0x3C000000
BLEND.slot0.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
```

The written-only output is not special for Mesa: the `if` is flattened with
the colour's lanes as the join's phis. The two selects are `MUX.v2i16
.int_zero` with a constant pair as one source; `MKVEC.v2i16` packs the two
results for the blend staging pair (`r0:r1`). Our `walk_if16` refuses it at
`then_colour != f.colour` (the colour differs per side).

## Case 142, unibranch (flatten)

Each arm is about 25 half-precision operations on `u.yzw`. Mesa's peephole
limit (64) is not reached, so the branch is not kept: both arms' arithmetic
is computed and the three colour lanes are two selects on `flt(0.5, u.x)`:

```
16    %57 = flt_pan %5 (0.500000), %1.x // preserve:inf
16x2  %58 = bcsel_pan %57.xx, %34, %55          // c.rg
16    %59 = bcsel_pan %57, %35, %56             // c.b
16x4  %64 = vec4 %58.x, %58.y, %59, %61 (0x3c00)
```

61 instructions, one block, 12 registers, no branch. The tail:

```
FCMP_OR.v2f16.lt.m1 r0, 0x38000000.h11, u2.w0.h00, r0^
MUX.v2i16 r2, r3^, r2^, r0.h00
MUX.v2i16.wait0126 r0, r4^.h00, r1^.h00, r0^.h00
ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
...  IADD_IMM.i32 r2, 0x0, #0x3C003C00 ; MKVEC.v2i16 r2, r0^.h0, r2^.h0
BLEND.slot0.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
```

The arms' arithmetic is a plain ALU chain on fp16 (`FMA.v2f16`, `MKVEC`,
`FADD.v2f16.clamp_0_1` for `clamp`): the work item `cflow-flatten` shares with
the existing mediump arithmetic. Mesa pairs the two arms' lanes into
`v2f16` operations (x and y of `c` together, z alone), so the instruction
count is the pairing's, not the source's. That is the part to match byte for
byte; a plain "both arms, then selects" with one operation per lane will
differ.

## Case 71, deadloop: what the dead loop becomes (flatten)

```glsl
float unused(vec2 p) { if (u_k.z > 0.0) return p.x*2.0 + p.y; return p.y*3.0; }
float dead = unused(u_k.xy);               // never read
float n = ceil(u_k.x * 3.0);  float acc = 0.0, waste = 0.0;
for (float i = 0.0; i < 8.0; i += 1.0) {
    if (i >= n) break;
    waste = waste * 3.0 + i;  acc += u_k.y;
}
gl_FragColor = vec4(acc, 0.0, 0.0, 1.0);
```

Mesa deletes `dead` and `waste` (nothing reads them), and the loop, whose
bound 8 is constant and whose one `break` is behind a run-time condition,
goes through `complex_unroll` (the mechanism of `mali-loop-break.md`) with
every guard flattened: `0 loops`, one block, 29 instructions. The NIR is
the ladder of `i >= n` compares and selects, eight guarded passes (the first
compares are `fge_pan 7.0, n`, then `flt_pan` pairs for 6, 5, 4, 3, 2, 1, 0):

```
16     %4 = fceil %3.x
16     %6 = fmin %4, %5 (8.000000) // preserve:inf          // n, clamped to the 8 passes
16     %9 = fge_pan %8 (7.000000), %6 // preserve:inf
16    %10 = bcsel_pan %9, %3.y, %7
16    %11 = fadd %1.y, %10                                   // acc += u_k.y
16    %13 = flt_pan %12 (6.000000), %6 // preserve:inf
16    %14 = bcsel_pan %13, %11, %3.y
...
16    %31 = bcsel_pan %28.x, %29, %30 (0x0)
16x4  %36 = vec4 %31, %30 (0x0), %30 (0x0), %33 (0x3c00)
```

```
FMA.v2f16 r0, u2.w0, u2.w1, r0^.neg                  ; u_k * (3,2)
FROUND.v2f16.rtp r1, r0.h00                          ; ceil
FMIN.v2f16 r1, r1^.h00, 0x48000000.h11
FCMP_OR.v2f16.ge.m1 r3, u3.w0, r1.h00, 0x0
MUX.v2i16 r2, r2^.h00, r0.h11, r3^.h00
FADD.v2f16 r2, u2.w0.h11, r2^.h00                    ; and so on, 8 passes
...
MUX.v2i16.wait0126 r0, 0x0, r0^.h00, r1^.h00
ATEST ... ; U16_TO_U32 r0, r0^.h0 ; BLEND ... .end
```

The `return` inside `unused` is gone before this (inlining plus
`nir_opt_dead_cf`/DCE); nothing here is a branch. The refusal at our side is
two things: the `unused()` call (a function with an early `return`, which
`walk_nodes` refuses as `Node::Return` not at the end) and the loop's
`break`, which the mediump path rejects (`IfExit` other than `Plain` is
`MaliError::GuardMediump`, in `walk_if16`). Both go away if the mediump path
reaches the same unrolled ladder as `walk_guard` builds for the highp one.
The compare constants come in `v2f16` pairs (`flt_pan (4.0, 5.0), %6.xx`):
the pairing again fixes the instruction count.

## Case 148, texbranch (branch)

```glsl
if (u.x > 0.5) c.r = texture2D(ta, vec2(0.5)).r; else c.r = texture2D(tb, vec2(0.5)).r;
if (u.y > 0.5) c.g = texture2D(ta, vec2(0.5)).g;
for (float i = 1.0; i <= 24.0; i += 1.0) {
    if (i >= u.z) break;
    s += texture2D(tb, vec2(0.5) - i*vec2(u.w)).b;
    s += texture2D(tb, vec2(0.5) + i*vec2(u.w)).b;
}
c.b = s / 48.0;
```

Mesa's NIR keeps both single `if`s as `nir_if`s with the `tex` in each arm
(peephole's `block_check_for_allowed_instrs` refuses a `tex`) and phis at
the joins:

```
16x4    %2 = @load_ubo (...)                                  // u
16x2    %3 = flt_pan %0 (0.500000).xx, %2.xy
16      %4 = mov %3.x
if %4 {
    16x4    %7 = (float16)tex %6 (0x20001800, 0x0) (texture_handle), %5 (0.5, 0.5) ..., 2D
    16      %8 = mov %7.x
} else {
    16x4   %11 = (float16)tex %10 (0x20011801, 0x0) ..., %9 (0.5, 0.5) ..., 2D
    16     %12 = mov %11.x
}
16     %13 = phi b1: %8, b2: %12
...
```

The loop is unrolled (`complex_unroll`, 24 passes) but its guards are not
flattened: each pass is an `if` holding two look-ups and the next pass's
`if`, so the NIR is 24 `if`s nested in each other. The guard conditions are
`flt_pan` compares of the pass number with `u.z` in `v2f16` pairs (`1.0, 2.0`,
`3.0, 4.0`, ... `%22 = flt_pan %21 (1.0, 2.0), %2.zz`); a pair's x lane tests
its first pass and its y lane the second. The look-up's coordinates are
`f2f32` of `fadd`/`ffma` of 0.5, the pass number and `u.w`; the 24th pass
ends in a ladder of 24 phis, a chain `phi b72: %424, b73: %51`
(`%427` at the last join), then `s * (1/48)` (`fmul %427, 0.020828`).

```
51 `tex` in all: 1 + 1 (the first `if`), 1 (the second), 2 x 24 (the passes)
```

Disassembly (542 instructions, 4352 bytes, 10 registers, 26 conditional
`BRANCHZ.eq.reconverge`, 4 unconditional; no `JUMP` instruction: a jump is
`BRANCHZ.eq.reconverge 0x0`, zero as the tested value):

```
FCMP_OR.v2f16.lt.m1 r0, 0x38000000.h11, u2.w0, r0^             ; both u.x and u.y tests
BRANCHZ.eq.reconverge r0.h0, offset:8                          ; u.x branch
   IADD_IMM.i32 r1, 0x0, #0x20001800 ; ... TEX_SINGLE.slot0 ... .r.2d.computed.wait0 @r1, @r2:r3, [r4^:r5^]
   U16_TO_U32 r1, r1^.h0
   BRANCHZ.eq.reconverge 0x0, offset:6                         ; jump over the else arm
   (else arm: its own TEX_SINGLE.slot1 ... , U16_TO_U32.reconverge r1, r1^.h0)
BRANCHZ.eq.reconverge r0^.h1, offset:8                         ; u.y branch (one arm)
   TEX_SINGLE.slot2 ... .g.2d ... ; U16_TO_U32 r0, r0^.h0
   BRANCHZ.eq.reconverge 0x0, offset:1
   IADD_IMM.i32.reconverge r0, 0x0, #0x0                       ; the empty arm: the join's 0.0
IADD_IMM.i32 r2, 0x0, #0x40003C00
FCMP_OR.v2f16.lt.m1 r2, r2^, u2.w1.h00, 0x0                    ; passes 1 and 2 guard
BRANCHZ.eq.reconverge r2.h0, offset:501                        ; pass 1: skip to the end
   <fadd/f16 coordinates, F16_TO_F32 x2, TEX_SINGLE.slot0/1/2 .b.2d.computed x2, FADD.v2f16>
   BRANCHZ.eq.reconverge r2^.h1, offset:482                    ; pass 2
   ...                                                        ; (the same shape, 24 times)
...
NOP.discard ; FADD.v2f16.reconverge r2, r2^.h00, r3^.h00      ; the last pass's join
BRANCHZ.eq.reconverge 0x0, offset:2 ; ... MOV.i32.reconverge r2, r3^  ; the joins
IADD_IMM.i32.reconverge r2, 0x0, #0x0                          ; s = 0.0 when no pass ran
FMA.v2f16 r2, r2^.h00, r3^, 0x0.neg                            ; * 1/48 (0x2555)
```

A branch is `BRANCHZ.eq.reconverge <cond>, offset:<words>`: it skips when the
tested half is zero, the offset in 64-bit instruction words to the join. The
nested exits all land on the single join at the end (the offsets fall by about
20 each pass: 501, 482, 460, 439, ...). Each look-up is `TEX_SINGLE.slotN.
skip.reserved.16.<comp>.2d.computed` with its descriptor pair: `IADD_IMM.i32
rX, 0x0, #0x20001800` (texture 0, sampler 0) or `#0x20011801` (texture 1,
sampler 1) as the low word and a zero high word, the slot cycling 0, 1, 2.
`.reconverge` is `reconverge_branches` (`flow.rs`: a block whose branches
may diverge); the `NOP.discard` words before the last joins are
`insert_flow_control_nops`' (`flow.rs`), which a later item must match from
the reference.

## Lowering plan

Two items, the first before the second; both extend the mediump path of
`select` (`nir_fs.rs`, `walk_if16`, 2477) because all five shaders are
`precision mediump float`.

### 1. flatten: 68, 71, 74 (both references), 142

Mesa's output has no branch; the NIR is `bcsel_pan` of the condition. The
instruction shapes, in the order they appear:

- `flt_pan 0.5, x` is `FCMP_OR.v2f16.lt.m1 c, x, 0` (or against the previous
  node's result, `FCMP_OR ... r0^`); the mask is a half-lane mask of the
  compare, so a pair of selects shares one compare (`.h00` / `.h11` select
  which half of the mask).
- `bcsel_pan(c, a, b)` over two `f16` lanes is `MUX.v2i16 dst, b, a, mask`
  (`int_zero` mode, operands `else, then`); when one lane is 0 the zero
  operand is a constant, a pair `0x3C000000.h11` (1.0 in the upper half) the
  other. Two lanes sharing the condition are `MUX.v2i16` on a half pair; a
  lone lane takes `.h00` of the pair.
- the packing for the blend staging: `MKVEC.v2i16` of two select results, a
  `MOV.i32` of the third pair, `IADD_IMM.i32 rX, 0x0, #0x3C003C00` for an
  `(1.0, 1.0)` constant (cases 74, 142).
- 71 adds `FROUND.v2f16.rtp` (ceil), `FMIN.v2f16` with the constant `8.0`, `FCMP_OR`
  with `.ge` for the first pass and `.lt` for the others, and the eight-pass
  `FADD.v2f16` / `MUX.v2i16` ladder.

Existing code to extend:

- `walk_if16` (`nir_fs.rs:2477`): its join accepts only the boolean lanes
  and refuses a different defined value on each side, a side that sets the
  colour (`then_colour != f.colour`), and any non-`Plain` `IfExit`. The
  flatten generalises its join: a local with two defined values becomes a
  `bcsel` node of the condition (`Nir::push_node` of an `Alu::Bcsel`,
  already built for `Nir::peephole_select`), and a colour set on both sides
  becomes the lane-wise select instead of an error. The "limit 64" budget
  of `Nir::peephole_select` (`nir.rs:1287`, `PEEPHOLE_LIMIT`) is what decides
  flat against kept; 142's arms stay under it, so the flatten is right for
  it, but the check must be the same count (see "Caveats").
- `nir::fuse`, `nir::late_rules`, `nir::bcsel_fadd` and `fold_const_selects`
  (called from `select_highp`) already fold the selects of constants; the
  mediump path needs the same calls after the join for 74's constant arms.
- 71 needs the loop reached through `walk_loop` (the unrolled `complex_unroll`
  of case 87, `mali-loop-break.md`) in the mediump path with the
  `GuardMediump` refusal lifted for a `break` whose guard Mesa flattens, and
  the call of `unused()` (an early `return` in a called function) inlined
  before the walk: `Node::Return` not at the end of `main` is today a
  `ControlFlow`. Nothing of the call survives in the output, so the function
  can be dropped by liveness (its result is never read) before the walk.

### 2. branch: 148 (and the sources that keep blocks)

Mesa's output keeps `nir_if`s with look-ups. The instruction shapes: an `if`
is `BRANCHZ.eq.reconverge <half of the compare mask>, offset` at the end of
its block (skips the then side); a then side that has an else side ends with
`BRANCHZ.eq.reconverge 0x0, offset` (the unconditional jump, `0x0` is the
tested value); `.reconverge` on the first word of every join block and on
the last of every block that ends in a branch; the look-up is the same
`TEX_SINGLE` as in straight-line code, with its `skip` flag and slot rotated
(`flow.rs: assign_slots`); the join's value is a `phi` (a `MOV.i32` to the
phi's register on each side, `IADD_IMM.i32 r, 0x0, #0` for the empty arm).

Existing code to extend:

- the blocks and the jumps: `end_block` (`nir_fs.rs:6770`) writes the `Branchz`
  for each `IfRegion` (`before` the `if`, `then_last` the jump to `after`;
  `brk` the kept loop's exit) and for each `LoopRegion`; `live_ifs` /
  `lay_out_inverted_guards` (`nir.rs`) choose which regions stay; both exist
  from loops-rt (`3e6d9bdf`, `kept_loop` / `walk_kept_loop` / `walk_guard`).
  What 148 adds is a *two-sided* kept `if` and a one-sided one that
  produces a value, so the phi at the join is two `MOV`s instead of a
  select: `IfRegion` has `else_first`/`else_last`/`after`; the phi writer
  is missing.
- the 24 nested guards are the `guard` form of `IfRegion`
  (`Nir::invert_guards`): each pass's guard is a kept `if` whose else side is
  the rest of the loop, as the femtovg `filter_image` comment in `walk_loop`
  already says ("Guarded passes too large for the peephole stay branches").
  148 is the reference for it: the first time Mesa's bytes for such a
  shader are in the tree.
- `flow.rs`: `insert_flow_control_nops` (`.reconverge` and the `NOP.discard`),
  `reconverge_branches`, `helper_terminate`, `assign_slots`: transcribed for
  the loops, they should apply unchanged; the offsets of `BRANCHZ` are
  resolved after the layout, as for the kept loops.
- the textures under `if`: every `TEX_SINGLE` here is `.skip.reserved.16.
  <comp>.2d.computed`; `analyze_helper_requirements` (`flow.rs`) decides
  `skip`, and 148's words are what to compare against.

### Caveats

- The peephole count: whether 142's arms are flattened or kept depends on the
  `block_check_for_allowed_instrs` budget (`nir_opt_peephole_select`, 64 in
  Bifrost's call; 8 in the GL front end's earlier call, `PEEPHOLE_LIMIT_GL`).
  Mesa flattened 142 here at fp16 pairing, with ~25 operations per arm; a
  change to the pairing could move it across the limit, so the item should
  keep `Nir::peephole_select`'s count honest rather than hard-code a "this
  is a flat shader".
- The five references are drawn with `--uniform-values`: a shader the
  item compares should be drawn the same way (the words named by value). The
  expect.json `trace` label is the output prefix (`cflow_<n>_<name>.trace`),
  not a file in the tree.
- Measured here: shaderdb numbers, the NIR and the disassembly above, one
  draw per pair. Not measured here: a board run (the roadmap's board work
  comes after #154's final record); the plan above is read from the dumps
  and the compiler's source, not run.
- The corpus sources `cond_sequence`, `frontfacing` and `tri_if`, and the
  `ifoutput2` NIR equal to `ifoutput`'s, are classified by shape only: `tri_if`
  and `cond_sequence` were not drawn, and `frontfacing` is the
  `gl_FrontFacing` of `mali-builtin-inputs.md` plus a select.
