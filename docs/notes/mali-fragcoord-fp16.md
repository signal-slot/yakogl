<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: glmark2's highp copy of gl_FragCoord in a mediump ES 1.00 shader (FRAGCOORD_FP16)

**Design and implementation, lower-fragcoord16** (plan #367, issues #32,
#33). Mesa 26.2.1 is `MESA_PAN_TREE`; `G/` is `src/compiler/glsl/`, `N/`
`src/compiler/nir/`. Mesa's output is the drm-shim references in
`tools/mali/testdata/` (`gm2_fcoord16_1..6`, `gm2_cf_call_fs`,
`gm2_cf_loop_const_fs`, `gm2_cf_ifelse_fs`). Our side is
`src/shaderc/online/mali/nir_fs.rs` and `mod.rs` at this commit; the names
are functions, re-grep them before relying on a line. The ops and their
encodings are in mali-fragcoord-fp16-ops.md section 7.

This note follows mali-fragcoord-mediump.md (the ES 1.00 read narrowed at
the load) and mali-fragcoord-fp16-ops.md sections 1-6 (case 218: the
ES 3.00 highp read in a mediump shader, an fp32 island).

## 1. The shaders and the refusals

glmark2 22c527cb's conditionals (fs 6c61, a5b5), function (c8ce) and loop
(e016, c7ea) fragment shaders start the same way (`data/shaders/
conditionals.frag`, `function.frag`, `loop.frag`; GPL text, not copied):

```
HIGHP_OR_DEFAULT vec2 FragCoord = gl_FragCoord.xy;   // highp on an ES device
float d = fract(FragCoord.x * FragCoord.y * 0.0001); // d is mediump
<the scene's steps: d = fract(3.0 * d), an if on d >= 0.5, a call, a loop>
gl_FragColor = vec4(d, d, d, 1.0);
```

under `precision mediump float`, `#version 100`. At the tip the five were
refused at the colour (`ColourForm`, "a gl_FragColor lane other than the
varying's own lane or a constant"): the walk read gl_FragCoord as the
mediump ES 1.00 built-in, narrowed at the load (`Fs::frag_coord16`), stored
the narrowed halves into the highp local, and the pair graph had no leaf for
one narrowed lane alone. Given that leaf, the next refusal was
`FragCoordMediump` (FRAGCOORD_FP16) in `Fs::alu16`, which took no one-lane
operation of the island. The study's FS_OPERAND for a5b5 belonged to the
pair-graph route the shader no longer takes (section 3).

## 2. What Mesa does

* `G/builtin_variables.cpp:1333-1335`: gl_FragCoord is mediump in ES 1.00.
* `G/lower_precision.cpp:835-843` (`handle_rvalue`): a bare dereference is
  never a lowered root. `FragCoord = gl_FragCoord.xy` is an assignment of a
  dereference to a highp variable: no `f2fmp`, the local holds the fp32
  read (the `U16_TO_F32`, `+ 0.5`, y's `FMA` with `gl_FbWposYTransform` of
  mali-highp-fragcoord.md 2.1).
* `:295-312`: a highp operand makes the operation `CANT_LOWER`. The whole
  head `fract(x * y * 0.0001)` is fp32; the assignment to the mediump `d`
  narrows it once (`FADD.f32 r0.h0, r0^, 0x0.neg`).
* Every step after it reads `d` (mediump) and constants: fp16, one lane
  (`FMA.v2f16 r0, u3.w1, r0^.h00, 0x0.neg` of `3.0 * d`, the constant 3.0h in
  both halves of a FAU word; `fract` is `FROUND.v2f16.rtn` and `FADD.v2f16
  .., r1^.neg.h00`).
* `N/nir_opt_algebraic.py:2394`: `fmul(fmul(a, b), #c)`, the inner product
  used once and of no constant, is `fmul(fmul(a, c), b)`: `x * 0.0001` first,
  then `* y` (both in the fp32 head and in the all-mediump fp16 one).
* Without the highp local (gm2_fcoord16_*, written on the mediump read) the
  read is narrowed at each lane (`FADD.f32 r0.h0`, `FADD.f32 r1.h0`) and the
  head is fp16 of one lane, `FMA.v2f16 r0, r0^.h00, r1^.h00, 0x0.neg`.

## 3. What we lower

1. **The precision of the local** (mod.rs `FsPrecision::of`). A float local
   declared `highp` in a mediump ES 1.00 shader no longer sets
   `FsPrecision::mixed` at once: its name's FNV-1a hash goes into
   `FsPrecision::highp_locals` (four at most). After the walk
   (`nir_fs::walk`), `highp_locals_copy` marks the shader mixed unless every
   such local is only ever stored a gl_FragCoord read that `highp_copy`
   accepts; a read narrowed beside the copy (`frag16` set too) is mixed as
   well (Mesa reads it at fp32 inside the copy's highp tree; no reference).
2. **The read** (`walk_nodes`, `LoadInput { FragCoord }`). `highp_copy`: the
   read's every user is a `StoreLocal` to a local whose
   `Module::local_prec_of` is `High` (the front end's precision table,
   lower.rs / precision.rs). Then the read is `Fs::frag_coord`'s fp32 lanes
   and `Fs::fc_copy` is set; otherwise the mediump `Fs::frag_coord16` as
   before.
3. **The island** (`Fs::fc32`, `frag32`). `fc_copy` opens the fp32 island
   case 218 built for the ES 3.00 read: operations on the copy's lanes go
   through `Fs::alu32` / `lane32`, and the program takes `select_highp`'s
   route (mod.rs `compile_stage`, `frag32`) with the fp16 uniform block.
4. **The store honours the local's precision** (`StoreLocal` arm). The
   narrowing at a declared variable's store now skips a local whose
   `local_prec_of` is `High`: the copy keeps fp32; `d` (Medium) is narrowed
   once by `narrow_stored`.
5. **`:2394` on the island** (`lane32`, the `Fmul` arm with a constant).
   `fmul(fmul(p, q), #c)` with `p`, `q` no constants, the inner IR value read
   once and on the island (`fc32`) is `fmul(fmul(p, c), q)`. Kept to the
   island: no highp reference draws the rule yet.
6. **One-lane fp16 steps** (`Fs::alu16`, `w == 1`). `Fmul`/`Fadd` of a
   one-lane island value (`Fs::island16`: the `f2f16` of an island float,
   or a 16-bit one-lane operation on one) and a constant or another such
   value, in the order written; `Ffloor`, `Fneg` and `Ffract` (as
   `fadd(a, fneg(ffloor(a)))`) of one. `Fs::lone16` is the admission. The
   walk's arm that calls `alu16` for a mediump shader now also fires on an
   `island16` operand, so the steps after the first are fp16 nodes with
   their constants rounded to fp16 (before, they fell to the generic
   mediump path and kept the fp32 constant, which `select_highp` read as
   its low half: `0x0`).
7. **The colour** is case 218's leaf: `select_highp`'s `IADD.v2u16 r1,
   r0.h00, 0x0` and `MKVEC.v2i16 r2, r0^.h0, u4.w0.h0` of `(d, d, d, 1.0h)`.

The all-mediump shapes (gm2_fcoord16_*) take the pair graph (`select`):

8. **The narrowed lane alone** (`lane_tree`). A lane of `Fs::frag16` read
   by a one-lane operation is `Leaf::FragLane` (the leaf
   fsarith_138_deriv's derivatives used), each lane narrowed in its own
   register (`frag16_lanes`).
9. **`:2394` on the narrowed lanes** (`Fs::op`, `Fs::position_product`).
   `fmul(fmul(x16, y16), #c)` of the two narrowed lanes is
   `fmul(fmul(x16, c), y16)`. The walk cannot see the inner product's other
   readers (`is_used_once`); no reference reads it twice.
10. **A select beside a constant in the colour** (`vectorise`, the
    computed-lane-then-constant arm; `lone_select`). A one-lane 16-bit
    select of its own fp16 compare, computed from the narrowed position
    (`reaches_frag16`), is `Leaf::Lane { x, half }`: gm2_fcoord16_5's
    `(d, 1.0)` word. The two arms `d - 0.5`, `d + 0.25` are already paired
    by `mates` into one `FADD.v2f16 r1, r0.h00, u3.w1` (0x3400b800). The
    restriction keeps deqp_uniarray_struct (a select of uniform compares,
    owned by constop) refused as pinned.
11. **A store is no vector** (`walk`, `ir_comps`). An unrolled loop's
    nodes take the loop's last instruction for their origin (`walk_loop`),
    here the counter's `StoreLocal`, whose `width_of` is 4: `Fs::wide` took
    every pass's `fract` for a lane of a vector and the pair graph found no
    arm for `(Ffloor, Ffloor)`. A `StoreLocal`, `StoreOutput` or `Discard`
    now counts one lane. No test or reference changed with it.

## 4. Results

| Reference / shape | Before | Now |
|---|---|---|
| gm2_fcoord16_1..6 (mediump read) | `ColourForm` | byte-equal |
| gm2_cf_call_fs (c8ce shape) | `ColourForm` | byte-equal |
| gm2_cf_loop_const_fs (c7ea shape) | `ColourForm` | byte-equal |
| gm2_cf_ifelse_fs (a5b5 shape) | `ColourForm` | compiles, 58 words to Mesa's 45 (section 5) |
| R1 6c61, R2 a5b5 (glmark2 text shape) | `ColourForm` | compile |
| R3 c8ce, R4 e016, R5 c7ea | `ColourForm` | compile, fragment words = gm2_cf_call_fs / gm2_cf_loop_const_fs |

`tests/mali_fragcoord16.rs` pins all of it. cases.txt has the six
gm2_fcoord16_* lines: pan_shc (SPIR-V, no lower_precision) differs from
Mesa's GL driver in every fragment program (`mediump`). No source under
`tools/shaders` has the shape: `tools/mali/corpus-verdicts.txt` keeps every
verdict (`corpus-verdicts.sh --check`: 256 ok -> 256 ok).

## 5. What stays open

| Shape | Verdict | Why |
|---|---|---|
| gm2_cf_ifelse_fs / a5b5's words | compiles, differs | Mesa's `nir_opt_vectorize` pairs each `if`'s arms `2.0 * t` and `3.0 * t` into one `FMA.v2f16 r1, u3.w0, r0.h00` (`u3.w0` = 0x42004000), `FROUND`/`FADD` of the pair and `MUX.v2i16 r0, r1^.h11, r1^.h00, r0^.h00`; it also moves 0.0001 and the alpha's 1.0h pair into registers (`IADD_IMM`). The `select_highp` route has no `mates` pairing; the pair-graph route has it (gm2_fcoord16_5) but not the fp32 head. Pinned in the test |
| function medium (849c) | `FragCoordMediump` | `fract(sqrt(d))`: no one-lane `Fsqrt` arm in `alu16` (no reference) |
| loop fs 3678 (uniform bound) | `LoopMediump` | a kept loop in a mediump shader (LOOP_FP16; its references are another item's) |
| the copy beside a mediump read | `MixedPrecision` | section 3.1 |
| more than four highp locals, or one that copies anything else | `MixedPrecision` | as before |
| ideas df64 / gm2_texcoord_1 (`gl_FragCoord.xy / 32.0` as a coordinate) | `FragCoordMediump` | lower-texcoord-lane's |
