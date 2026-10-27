<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali Valhall: Mesa's lowering of a loop with a run-time break (gl_test case 87)

The reference is `tools/mali/testdata/case87_databreak.{vert,frag,expect.json}`:
Mesa 26.2.1's panfrost GL driver on drm-shim, one draw of the pair with
`tools/mali/gl_ref.py`, `BIFROST_MESA_DEBUG=shaders,shaderdb`
`PAN_MESA_DEBUG=trace`, and `pan_ref_diff.py extract <trace> --draw` for the
JSON, the same as `s6_discloop.*`. `drm_shim_ref.sh run` is that invocation
inside a wrapper; see Caveats for why the draw here was made without the
wrapper's `stdbuf`. The NIR below is the driver's own print of the fragment
shader after the optimisation loop, quoted as dumped; the mechanism (which
pass does what) is read from Mesa's source and agrees with it. Case 87 is
`case_databreak` in `driver/tools/pvrlab/gl_test.c` (`gl_test_databreak_fs_src`,
passthrough VS):

```glsl
for (int i = 0; i < 8; i++) {
    if (float(i) >= s * 8.0) break;       // s = v_color.w (a varying)
    if (top > 0.5 && i == 2) continue;    // top = v_color.y
    count += 1.0;
}
gl_FragColor = vec4(count / 8.0, 0.0, top > 0.5 ? 1.0 : 0.0, 1.0);
```

## Answer: neither a loop nor branches

**Mesa unrolls the loop with its exit guarded, then flattens every guard into
a select. The fragment program is one basic block: no `BRANCHZ`, no `JUMP`,
no back edge** (`0 loops`, one `b0`, 40 instructions, 384 bytes, 8
registers, 6 uniforms). The break point is not a control-flow change at all:
the number of iterations a pixel runs is computed as a ladder of compares and
`CSEL`s over compile-time constants.

NIR after the optimisation loop (`%3` = the varying pair: `.x` = `v_color.y`,
`.y` = `v_color.w * 8.0`, the VS computes it; see pass 2). Quoted from
`b0` line for line; the `load_const`s (`%1`, `%2`, `%4`, `%5`, `%8`, `%10`,
`%12`, `%14`, `%16`, `%30`, `%35`), the barycentric load, `%38`-`%41` and the
epilogue (`%42`-`%45`, blend) are left out:

```
32     %6 = flt_pan %5 (0.500000), %3.x // preserve:inf
32     %7 = bcsel_pan %6, %1 (0x40000000), %4 (0x40400000)
32     %9 = bcsel_pan %6, %4 (0x40400000), %8 (0x40800000)
32    %11 = bcsel_pan %6, %8 (0x40800000), %10 (0x40a00000)
32    %13 = bcsel_pan %6, %10 (0x40a00000), %12 (0x40c00000)
32    %15 = bcsel_pan %6, %12 (0x40c00000), %14 (0x40e00000)
32    %17 = bcsel_pan %6, %14 (0x40e00000), %16 (0x41000000)
32    %18 = fge_pan %14 (7.000000), %3.y // preserve:inf
32    %19 = bcsel_pan %18, %15, %17
32    %20 = fge_pan %12 (6.000000), %3.y // preserve:inf
32    %21 = bcsel_pan %20, %13, %19
32    %22 = flt_pan %10 (5.000000), %3.y // preserve:inf
32    %23 = bcsel_pan %22, %21, %11
32    %24 = flt_pan %8 (4.000000), %3.y // preserve:inf
32    %25 = bcsel_pan %24, %23, %9
32    %26 = flt_pan %4 (3.000000), %3.y // preserve:inf
32    %27 = bcsel_pan %26, %25, %7
32    %28 = flt_pan %1 (2.000000), %3.y // preserve:inf
32    %29 = bcsel_pan %28, %27, %1 (0x40000000)
32    %31 = flt_pan %30 (1.000000), %3.y // preserve:inf
32    %32 = bcsel_pan %31, %29, %30 (0x3f800000)
32    %33 = flt_pan %2 (0.000000), %3.y // preserve:inf
32    %34 = bcsel_pan %33, %32, %2 (0x0)
32    %36 = fmul %34, %35 (0.125000)
32    %37 = f2f16 %36
```

`%6` is the `continue` condition (`top > 0.5`); `%7..%17` are the counts at
each exit as constants (a pixel that leaves at pass k has count k, or k-1 once
the skipped iteration 2 is behind it, which `%6` selects); `%18..%34` are the
eight passes' break tests, `s*8` against the constants 7 down to 0, one
`bcsel_pan` each, innermost (last) pass first.

The passes, in the order they produce this (`bifrost_optimize_nir`,
`src/panfrost/compiler/bifrost/bifrost_nir.c:225-275`, repeated until no
progress):

1. **`nir_opt_loop_unroll`** (`src/compiler/nir/nir_opt_loop_unroll.c`,
   `process_loops`:990). Loop analysis gives two terminators: `i < 8` (trip
   count 8, the *limiting* terminator) and the run-time `break`
   (`exact_trip_count_unknown`). `check_unrolling_restrictions`:899-931 passes
   (`max_unroll_iterations` is 32, `st_program.c:651`; cost
   `instr_cost * 8 <= 32 * LOOP_UNROLL_LIMIT`, :36). Two terminators with the
   limiting one known: **`complex_unroll`**:338 (not `simple_unroll`:105, the
   arm yakogl's `walk_loop` transcribes). It clones the body once per pass and
   nests each clone in the continue side of the previous clone's run-time
   `break` `if` (`complex_unroll_loop_body`:237,
   `get_complex_unroll_insert_location`:222), the loop deleted. What it
   leaves is nested `if`s, not selects.
2. **`nir_opt_constant_folding` / `nir_opt_algebraic` / `nir_opt_cse`**: `i`
   is a constant in every clone, so `float(i)` and `i == 2` fold: the compare
   of pass `i` is `fge/flt_pan <i>.0, x` with a constant on one side, and the
   `continue` is live only in pass 2. The eight copies of `s * 8.0` are one
   value after CSE.
   **That `s * 8.0` is not in the fragment program at all**: the linker's
   `nir_opt_varyings` moves a one-varying ALU op (`fmul` by a constant) to the
   previous stage. The vertex shader's varying program computes
   `FMA.f32 r0, r3^, 0x48000000.h1, 0x0.neg` (`w * 8.0`) and stores it as the
   varying's second lane; the first lane is `v_color.y` as is. The fragment
   program therefore loads a `vec2` (`top`, `s*8`) and compares the constants
   with `%3.y` directly.
3. **`nir_opt_dead_cf`**: the `continue` of every pass but 2 is dead.
4. **`nir_opt_peephole_select`** (`bifrost_nir.c:234`, `limit = 64`,
   `expensive_alu_ok = true`; the GL front end's loop runs it too): an `if`
   whose sides hold only a few ALU ops and no side effects becomes `bcsel` of
   the two sides' values. The innermost guard `if` is such an `if` (its sides
   are constants and a phi), then the one around it is, up the nest: **every
   nested guard flattens**, innermost first, one `bcsel` per pass. The
   `continue` of pass 2 is a `bcsel` of `count + 1` and `count` on
   `top > 0.5`, which constant-folds into the `%7..%17` pairs above.
5. `nir_opt_if`, `nir_opt_loop`, `nir_opt_phi_to_bool` find nothing left to do.

The backend then emits no control flow: `emit_if`
(`bifrost_compile.c:3697`, `BRANCHZ.i16` + `JUMP`) and `emit_loop`:3738
(`loop_header`, back `JUMP`) are not reached, and `insert_flow_control_nops`
has one block.

(If the unroll did not happen, because of the cost or the trip count, Mesa
would keep a loop: `loop_header` block, a back `JUMP`, the `break` a `JUMP`
to `break_block` inside a `BRANCHZ` guard. Case 87 is far inside the bounds.)

## The words

The fragment program is the 40 instructions of `case87_databreak.expect.json`
(`fs.fragment`), ending in the `BLEND` (`.end`) and the
`IADD_IMM r48 / BRANCHZI.eq.absolute blend_descriptor_0.w1` tail every
fragment program has. Its FAU table (`fau`) is 3.0, 4.0, 5.0, 6.0, 7.0, 8.0 as
words 4-9, 0.125 as word 10 and `3c003c00` (1.0h, 1.0h) as word 11; 0.5 is the
immediate `0x38000000.h1` and 2.0, 1.0 are the immediates `0x40000000`,
`0x3F800000`. Abridged from the disassembly in that file:

```
LD_VAR_BUF_IMM.f32.slot0.v2.src_f32.center.store.wait0 @r0:r1, r61^, index:0x0
FCMP_OR.f32.lt.m1.discard r0, 0x38000000.h1, r0^, 0x0      // top > 0.5 -> r0 (a mask)
MOV.i32 r2, u3.w1                       // CSEL takes at most one FAU operand: the
CSEL.u32.eq r2, r0, 0x0, u4.w0, r2^     //   other constant is MOVed to a register
...  (FCMP_OR.f32.{ge,lt}.m1 r4, <const>, r1, 0x0 ; CSEL.u32.eq r2, r4^, 0x0, a, b) per pass
FCMP_OR.f32.lt.m1 r1, 0x0, r1^, 0x0 ; CSEL.u32.eq r1, r1^, 0x0, 0x0, r2^
FMA.f32 r1, r1^, u5.w0, 0x0.neg        // count * 0.125 as FMA(count, 0.125, -0.0)
MUX.v2i16 r0, 0x0, 0x3C000000.h11, r0^.h00      // top > 0.5 ? 1.0h : 0
FADD.f32 r1.h0, r1^, 0x0.neg           // the fp32 -> fp16 convert
MKVEC.v2i16 r2, r0^.h0, u5.w1.h0 ; U16_TO_U32.wait0126 r0, r1^.h0
ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
MOV.i32.wait r1, r2^
BLEND.slot1.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
```

Facts the byte-equal lowering needs from it: the compare is
`FCMP_OR.f32.<cond>.m1` against a constant (`ge` for passes 7 and 6, `lt`
below, as the NIR's `fge_pan`/`flt_pan`), the select is
`CSEL.u32.eq dst, mask, 0x0, a, b`, and the ladder runs from the highest pass
(`ge 7.0`) down.

## What yakogl would need to change

Everything is in the fragment compiler. None of it is new control flow: the
outcome is straight-line code, so the existing `if`-to-select machinery does
the flattening once the walk can produce the nested `if`s Mesa's
`complex_unroll` leaves.

0. **The vertex-side motion exists; check that it takes this one.**
   `src/shaderc/online/mali/motion.rs` (`move_into_producer`, called from
   `mod.rs:1126` for a highp shader, `apply` on the VS side) is the
   transcription of `nir_opt_varyings`' backward motion, and `flag` there
   accepts `fmul`/`ffma` by a constant on an interpolated lane. Two things
   can stop `s * 8.0` here, and neither is known until `loopbreak` runs the
   pair: `flag` returns `Unmovable` for an operation whose block is not 0
   (`motion.rs:366`: "an operation inside an `if`"), and the motion runs
   before `select_highp` flattens any `if` (`nir_fs.rs:2644`), where Mesa
   flattens first. The pass-0 `s * 8.0` stands in block 0 (before the first
   guard) and the later passes' copies are `nir_opt_cse`'d into it
   (`nir.rs:1175`, an equal instruction in a dominating block), so the motion
   should find it; if the later copies are not CSE'd (a different
   `Val::Const` form for `float(i)`'s product), `motion.rs::flag` is where to
   look, and the motion would have to run after the peephole as in Mesa.
1. **`walk_if` and `walk_nodes` (`nir_fs.rs:1220`, `:1399`): the guard that
   ends in `Break` or `Continue`.** `walk_if` returns `MaliError::LoopForm`
   when a side's flow is not `Flow::Next` (`nir_fs.rs:1271`). That is the
   refusal to remove, for a run-time condition. `complex_unroll` gives each
   such guard the rest of the loop as its other side: for `if (c) break;` the
   sides are "leave now" (the locals at the break, the join being the loop's
   exit) and "everything after, then the next pass" (nested in the else
   side); for `if (c) continue;` the sides are "this pass ends" and "the rest
   of this pass". So `walk_nodes` must, on an `if` with a `Break`/`Continue`
   side, walk `nodes[k + 1..]` (and, for `Break`, the passes after it, driven
   from `walk_loop`) as the other side's body inside `walk_if`, then join
   with the existing phi code (`Fs::phi`, one phi per local the sides left
   different, in `Fs::rank` order). The join's place (`join`) for the
   loop-exit phi is `walk_loop`'s `last`. This produces nested `IfRegion`s
   (`nir.rs` `IfRegion`, `Nir::ifs`) exactly where Mesa has nested `if`s. Do
   not add a second select lowering here.
2. **`walk_loop` (`nir_fs.rs:1351`).** Today the loop is walked pass by pass
   until `walk_nodes` returns `Flow::Break` on a folded condition. For
   `complex_unroll` the walk must also accept a pass whose `break` condition
   is a run-time value, nesting the next pass in that guard's continue side
   (item 1); the limiting terminator (the folded `i < 8`) ends the chain at
   `MAX_UNROLL_ITERATIONS` (`:57`); the `LOOP_UNROLL_LIMIT *
   MAX_UNROLL_ITERATIONS` size check (`:1384`) stays. Mesa's gate is exactly
   two terminators with one of them known (`nir_opt_loop_unroll.c:1129`); a
   loop with a store or discard in a guarded side, or with more terminators,
   stays `LoopForm`. Update `walk_loop`'s doc and `MaliError::LoopForm`'s
   (`mod.rs:196`, its message at `mod.rs:399`), whose texts name the run-time
   `break` as the refusal.
3. **`Nir::peephole_select` (`nir.rs:821`): confirm it flattens an 8-deep
   nest.** It is Mesa's `nir_opt_peephole_select` to a fixed point and takes
   an `IfRegion` only when each side is one block (`then_first ==
   then_last`, `else_first == else_last`). An `if` whose side holds another
   `if` qualifies only after the inner one has merged its blocks, which the
   `while progress` loop provides, innermost first, matching Mesa. The point
   to check is the budget: `a + b > limit` against `PEEPHOLE_LIMIT_GL` and
   `PEEPHOLE_LIMIT` (`nir.rs:62`; Mesa's `limit = 64`) counts the ALU ops of
   both sides, and the inner `bcsel`s from earlier merges are inside the
   outer side, so the count grows up the nest. With case 87 each side is a
   constant or a phi, so it stays far below. A body with more work per pass
   (the femtovg `filter_image*` loops with 24 copies) is where the limit, and
   `nir_opt_collapse_if` (not transcribed, `nir.rs:820`), start to matter.
   The call sites are `nir_fs.rs:2644-2645` in `select_highp`.
4. **The constants and the order of the selects** (`Fs::lane32` and the
   Alu builder in `nir.rs` at `:1175`): `count + 1.0` per pass, `float(i)`
   and `i == 2` fold as the counter's arithmetic already folds in
   `walk_loop`, so each select's two sides are `Val::Const`s and only the
   compares and selects remain nodes; a count that stayed a node would not
   match the reference. The byte-equal order is the NIR's: `%6`/`%7..%17` (the
   `continue` pairs) before the `%18..%34` ladder, innermost pass first.
5. **`emit.rs`: expected no change.** `FCMP_OR.f32.<cond>.m1` against a
   constant and `CSEL.u32.eq` are what `if`-flattening phis already emit
   (case 212, `ifsel` in `driver/tools/pvrlab/gl_test.c`, is the existing
   select case); the detail to check is the FAU-operand rule: a `CSEL` takes
   one uniform/constant source and the other is first `MOV.i32`ed to a
   register (`MOV.i32 r2, u3.w1` before the `CSEL` above).
6. **`flow.rs`: no change expected.** The program is one block, so
   `analyze_helper_requirements`, `assign_slots`, `insert_flow_control_nops`
   and `merge_flow` see no branch. (A discard in the same loop is another
   matter, and stays a refusal.)
7. **Tests and gate.** A byte-equal case `case87_databreak` against
   `tools/mali/testdata/case87_databreak.expect.json`, beside `s6_discloop`
   (so `MALI_TESTS` in `tools/mali/gate.env` moves with it). Do not count the
   femtovg `filter_image*` programs in `tools/mali/corpus-verdicts.txt` until
   their own references are checked: they have the same loop shape with 24
   guarded copies and refuse today at the same place (`walk_loop`'s doc);
   whether they unlock depends on their guarded copies being select-only too.

## Status: lowered (plan #153 loopbreak)

`case87_databreak_is_byte_equal_to_mesa_26_2_1` (tests.rs) pins both
stages byte-equal. What the code does:

- `walk_guard` / `walk_passes` (nir_fs.rs): an `if (c) break;` in a pass is
  an `IfRegion` with `guard` set, its else side the rest of the pass and then
  the later passes; `if (c) { ...; continue; }` is an `if` whose else side is
  the rest of the pass. A second run-time `break` stays `LoopForm`.
- `Nir::inverted_guards` / `invert_guards`: **the polarity is fitted to this
  one reference.** Mesa's `opt_if_simplification` inverts every guard
  (empty then side); the front end's peephole (limit 8) flattens the inner
  ones in the same round, so `bcsel(inot c, ..)` turns back into `fge`; the
  rest become `flt` with swapped sides. The repository's count (the folded
  side plus one per nested guard's `inot`) gives 1, 5, 9 for passes 7, 6, 5,
  which matches the reference's split (`fge` for 7 and 6, `flt` for 5..0).
  Whether Mesa's state at that call really is that folded was not checked:
  the per-pass NIR (`NIR_DEBUG=print_fs`) could not be drawn in that session.
  A second reference with a different body size would confirm or correct it.
- `Nir::fold_const_selects` (`nir_opt_constant_folding` through a `bcsel` of
  two constants), `Nir::cse_compares` (the compares equal once the blocks
  merged), `bcsel(a, 1.0, 0.0)` as `b2f(a)`, and `iand` with a constant.

## Caveats

- The `peephole_select` limit (64) is why the whole nest flattens here. A
  body with more work per pass leaves real `if`s inside the unrolled chain
  (then `emit_if` makes `BRANCHZ.i16`/`JUMP` blocks as for any `if`, and
  `flow.rs` matters). Case 87 does not need that.
- `tools/mali/drm_shim_ref.sh run` fails on this host at `eglInitialize
  failed` for this pair and for the existing `s6_discloop` pair alike, so the
  failure is the host's, not the pair's. The only difference between the
  script's command and the one that produced `case87_databreak.expect.json`
  is the script's `stdbuf -o0` in front of `python3`; the draw was made with
  the same environment (`LD_PRELOAD` of the shim, `PAN_GPU_ID=a867`,
  `MESA_LOADER_DRIVER_OVERRIDE=panthor`, `BIFROST_MESA_DEBUG=shaders,shaderdb`,
  `PAN_MESA_DEBUG=trace`) and without it. Why `stdbuf` breaks the shim here
  was not investigated. A second draw without `stdbuf` gave an `extract
  --draw` output identical to the committed file but for the `trace` label,
  so the bytes are reproducible and do not depend on stdout buffering.
