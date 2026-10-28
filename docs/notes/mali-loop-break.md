<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali Valhall: Mesa's lowering of a loop with a run-time break (gl_test case 87)

Two parts. The first (to "Caveats") is case 87, a loop whose constant bound
lets Mesa unroll it with its run-time `break` guarded and flattened: lowered
in plan #153. The second ("Part 2", at the end) is the loops Mesa **keeps**
because no constant bounds them, with conditional `break` and `continue`:
the decoding of the five `intloop_*` references and the loop parts of
`builtin_45`/`builtin_98`, and the plan for the item that lowers them
(plan #154, loops-rt).

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

# Part 2: loops a run-time bound keeps (plan #154, study-loops)

The references are the committed `tools/mali/testdata/intloop_{loop_exit,
loop_nested,loop_nested_vs,contloops,divnest}.{vert,frag,expect.json}`
(recipe and shaderdb counts in `docs/notes/mali-int-loops.md`) and
`builtin_45`/`builtin_98` (`docs/notes/mali-builtin-inputs.md`;
`builtin_98` drawn with `--uniform-values`, so its `count` word is 16.0).
Everything below is read from their words; no new reference was made. The
refusals were measured at c5b55ee2 with a scratch build (a test compiling
each source alone, as `the_corpus_verdicts_are_pinned` does, with every
refusal site printing its place; not committed). Function names are given,
not lines.

## Why Mesa keeps these loops

`nir_opt_loop_unroll` unrolls only a loop with a *limiting terminator*, a
`break` whose trip count loop analysis knows (`nir_loop_analyze.c`); a
loop without one is left as it is (`process_loops`,
`src/compiler/nir/nir_opt_loop_unroll.c:1112-1114`, `goto exit`), and so is
an outer loop holding a nested one unless it runs at most once (same lines,
`has_nested_loop`). `i < n` with `n` a varying, `i < ui_two` with a uniform,
`float(i) < count`, `float(i) < floor(v_s * 8.0)` have no known trip count:
every loop here is kept. (`contloops` has both kinds: its `for (i < 8)` and
its `do .. while` with `m == 4` have a constant terminator and are unrolled
as in Part 1; only its `while (k < n)` is kept.)

The back end then emits, for each kept loop (`emit_loop`,
`src/panfrost/compiler/bifrost/bifrost_compile.c:3738-3770`): a header block
(the `continue_block`, `loop_header = true`), the body, a `JUMP` back to the
header at the body's end, and a `break_block` after it. A `break` or
`continue` is a `JUMP` to those blocks (`bi_emit_jump`, :85-100); an `if`
is `BRANCHZ.i16.eq cond.h0` to the else side plus a `JUMP` from the end of
the then side to the join (`emit_if`, :3697-3735). Two clean-ups shape the
words:

- `bi_lower_branch` (:4218-4245) drops a second branch in one block (the
  `then_exit` jump after a `break`'s jump).
- `bi_opt_control_flow` (`bi_opt_control_flow.c:186`) removes the "simple"
  blocks (one predecessor, one successor, nothing but a branch,
  `is_simple_block` :15). For `if (c) break;` the then side is such a block:
  the `BRANCHZ.eq` of the `if` that skipped it is **inverted** and retargeted
  at the break block (`try_remove_simple_block` :98-147, `inverse_cmpf`
  :68). That is why a loop's exit test is a plain `BRANCHZ` (the disassembly
  omits `.ne`: branch when the mask is non-zero) while an `if` around work is
  `BRANCHZ.eq`. A break edge that carries phi copies is not simple and stays
  `BRANCHZ.eq` over a block of `MOV`s and a `JUMP` (divnest). A backward
  `JUMP` is never merged this way (`succ_seen`, :124-131).

In the words a `JUMP` is `BRANCHZ.eq.reconverge 0x0` (`va_lower_isel`);
`offset` counts instructions from the one after the branch. `.reconverge`
ends every block with two successors, or with one successor that has more
than one predecessor (`bi_reconverge_branches`, `bir.c:229-236`,
`va_insert_flow.c:482-494`): the preheader (its successor, the header, has
the preheader and the latch as predecessors), the header's exit test, the
latch, a join. Phis leave SSA as a `MOV` at the end of each predecessor and
one where the phi stood, propagated into the writer when it can be
(`bi_out_of_ssa`, `bi_ra.c:999`): a loop-carried value is written in place
by its update (`IADD_IMM.i32 r5, r5^, #0x1`), and a header `MOV rX, rY`
appears when the value is read after the register is overwritten
(`builtin_98` word 3, `loop_nested` word 9).

**Header phis are in first-store order** (the order of the locals'
initialisers): `loop_exit` `s` r4, `i` r5; `builtin_98` `n` r0, `last` r1,
`i` r2; `builtin_45` `c` r0..r3, `i` r4. An `if`'s join phis are in the
reverse of first reference (`Fs::rank`, `walk_if`); the loop header is not
the same rule. Measured on these three only.

## The refusals today (c5b55ee2)

| source | stage | refusal | where |
|---|---|---|---|
| `intloop_loop_exit.frag` | fs | `LoopForm` | `nir_fs.rs` `walk_if`, its closing `match exit` (`_ =>`) |
| `intloop_divnest.frag` | fs | `LoopForm` | the same |
| `intloop_loop_nested.frag` | fs | `IntValue` | `nir_fs.rs` `walk_nodes`, the `ConstI32` arm (`int i = 0`; only `prec.highp` takes one) |
| `intloop_loop_nested_vs.vert` | vs | `IntValue` | `nir.rs` `build`, the `ConstI32` arm |
| `intloop_contloops.frag` | fs | `IntValue` | `nir_fs.rs` `walk_nodes`, the `F2i` arm (a mediump `int(v_color.w * 4.0 + 3.5)`) |
| `builtin_45.frag` | fs | `UniformType` | `nir_fs.rs` `walk_nodes`, the highp `LoadUniform` arm (`n`, an `int` uniform that is not only an index, `nir::index_only`) |
| `builtin_98.frag` | fs | `Arithmetic` | `nir_fs.rs` `Fs::lane32`, `_ =>` (the `Ineg` of `int last = -1`) |
| `builtin_45.vert`, `builtin_98.vert` | vs | `SpecialInput` | `nir.rs` `build`, the `LoadInput` arm (`gl_VertexID`) |

`loop_exit` and `divnest` reach the loop: their int counter and its `Ilt` /
`I2f` / `Flt` are taken by the highp walk (`Fs::lane32`), `walk_loop` starts
`walk_passes`, and pass 1's test, the front end's `if (c) {} else { break; }`
(`Lower::break_unless`, `src/shaderc/online/lower.rs`), is a run-time
condition. `walk_nodes` sends to `walk_guard` only `if (c) break;` with an
empty else side, so the test goes to `walk_if` as `IfExit::Plain`, whose
else side ends in `Flow::Break`: `LoopForm`. The others stop before the
loop at a value-level refusal; the next one each would meet is the loop:
`walk_loop`'s `!prec.highp` (`ControlFlow`) for the mediump `loop_nested`
and `contloops`, the first non-instruction node in `nir.rs` `build`
(`ControlFlow`) for `loop_nested_vs`, and `walk_if`'s `LoopForm` for the
two highp builtins. (`alu_deadloop`/`alu_loopcarry`, the mediump twins of
Part 1's guard ladder, stop at `walk_if`'s `!prec.highp` and at the
mediump `Fceil` in `walk_nodes`: they are the control-flow item's (cases
70/71), not this one's.)

## loop_exit (`es300/loop_exit.frag`, highp, 23 words)

```
b0  0  LD_VAR_BUF_IMM.f32.v3 @r0:r1:r2               ; r, g, b
    1  FADD.f32.clamp_0_1.discard r0, r0^, 0x0.neg  ; clamp(r, 0, 1)
    2  FMA.f32 r0, r0^, u2.w0, 0x0.neg              ; * 5.0 (kept: float(n) reads it)
    3  F32_TO_S32.rtz r3, r0                         ; n
    4  IADD_IMM.i32 r4, 0x0, #0x0                    ; s = 0.0
    5  IADD_IMM.i32.reconverge r5, 0x0, #0x0         ; i = 0
b1  6  ICMP_OR.s32.ge.m1 r6, r5, r3, 0x0             ; header: i >= n
    7  BRANCHZ.reconverge r6^.h0, offset:5           ; -> 13 (exit)
b2  8  S32_TO_F32 r6, r5                             ; float(i)
    9  FADD.f32 r6, r1, r6^                          ; g + float(i)
   10  FADD.f32 r4, r4^, r6^                         ; s +=
   11  IADD_IMM.i32 r5, r5^, #0x1                    ; i++
   12  BRANCHZ.eq.reconverge 0x0, offset:-7          ; -> 6
b3 13  FMA.f32 r1, r4^, u2.w1, 0x0.neg               ; s / 16
   14  FROUND.f32.rtz r0, r0^                        ; float(n) = ftrunc(x)
   15..22 FMA * 0.125, ATEST, MOVs, BLEND, the return
```

FAU: `u2.w0` 5.0, `u2.w1` 1/16, `u3.w0` 0.125. One loop, two carried
values (`s` r4, `i` r5), no exit copies (the values leave in the registers
they were carried in). `float(n)` is not `S32_TO_F32` of the counter bound:
`i2f(f2i(a))` is `ftrunc(a)` (`nir_opt_algebraic.py:1250`), so the float
product of word 2 stays live across the loop.

## builtin_98 (`es300/uniloop.frag`, highp, the loop words 0-10)

```
b0  0  IADD_IMM.i32.discard r0, 0x0, #0x0            ; n = 0.0
    1  IADD_IMM.i32 r1, 0x0, #0xFFFFFFFF             ; last = -1
    2  IADD_IMM.i32.reconverge r2, 0x0, #0x0         ; i = 0
b1  3  MOV.i32 r3, r2                                ; header: i's phi copy
    4  S32_TO_F32 r2, r2^
    5  FCMP_OR.f32.ge.m1 r2, r2^, u2.w0, 0x0         ; float(i) >= count
    6  BRANCHZ.reconverge r2^.h0, offset:4           ; -> 11 (exit)
b2  7  FADD_IMM.f32 r0, r0^, #0x3F800000             ; n += 1.0
    8  IADD_IMM.i32 r2, r3, #0x1                     ; i + 1
    9  MOV.i32 r1, r3^                               ; last = i
   10  BRANCHZ.eq.reconverge 0x0, offset:-8          ; -> 3
```

Words 11-36 are the epilogue: `mod(n, 16.0)` etc. as `FMA` by 1/16,
`FROUND.f32.rtn` (floor), `FMA` by -16 plus `n` (`lower_fmod`,
`bifrost_compile.h:87`, rule `nir_opt_algebraic.py:2492`), `FMIN` with
`15.0` (an `IADD_IMM` immediate, as 1/256 is), `F32_TO_S32.rtz n`, `IADD_IMM
last + 1`, `ICMP_OR.u32.eq.m1`, three `CSEL.u32.eq ..., 0x3F800000, x`.
FAU: `u2.w0` the uniform `count`, `u2.w1` 1/16, `u3.w0` -16.0, `u3.w1`
16/255. Three carried values; `last` is a phi whose latch source is `i`.

## builtin_45 (`es300/dynidx.frag`, highp, 21 words)

```
b0  0..3  IADD_IMM.i32 r0..r3, 0x0, #0x0 (word 0 .discard)  ; c = vec4(0)
    4  IADD_IMM.i32.reconverge r4, 0x0, #0x0         ; i = 0
b1  5  ICMP_OR.s32.ge.m1 r5, r4, u2.w1, 0x0          ; header: i >= n
    6  BRANCHZ.reconverge r5^.h0, offset:9           ; -> 16 (exit)
b2  7  IADD.s32 r5, u2.w0, r4                        ; idx + i
    8  LSHIFT_OR.i32 r5, r5^, 0x7060504.b00, 0x0     ; * 16 bytes
    9  LD_PKA.i128.unsigned.slot0.wait0 @r6:r7:r8:r9, r5^, 0x0   ; cols[idx + i]
   10..13 FADD.f32 r0..r3, rK^, r6..r9^              ; c +=
   14  IADD_IMM.i32 r4, r4^, #0x1
   15  BRANCHZ.eq.reconverge 0x0, offset:-11         ; -> 5
b3 16  NOP.wait0126 ; ATEST.wait @r60, r60^, r3, atest_datum.w0 ; BLEND ...
```

The int uniforms are FAU words read by the compare and the `IADD` directly
(`idx` `u2.w0`, `n` `u2.w1`); `cols[8]` is not pushed (it is read with a
run-time index): the `LD_PKA` of the push table at byte `(idx + i) << 4`.
Four carried lanes plus the counter. The `LD_PKA` waits in place
(`.wait0` on itself, the load's slot 0) because its result is read by the
next instruction.

## divnest (`es300/divnest.frag`, highp, 57 words)

The vertex shader computes `v_s * 8.0` (the motion of Part 1's step 0:
`FMA.f32 r0, r0^, 0x48000000.h1, 0x0.neg` in `vs.varying`), so `b` is
`FROUND.f32.rtn` of the loaded lane. FAU: `u2.w0` 1/16, `u2.w1` -16.0,
`u3.w0` 16/255; 6.0, -2.0, 1.0, 1/256, 15.0 and 32/255 are immediates.

```
B0   0-3   LD_VAR; FROUND.rtn.discard r0 (b); n r1 = 0; i r2 = 0 (.reconverge)
B1   4-6   outer header: S32_TO_F32 r3, r2; FCMP_OR.f32.ge.m1 r3 (float(i) >= b);
           BRANCHZ.eq r3 -> 10                       ; not done: into the body
B2   7-9   break edge: MOV r0, r1^ (n); MOV r1, r2^ (outer); JUMP -> 32
B3  10-12  ICMP_OR.u32.ne.m1 r3, r2, 0x3020100.b2 (i != 2); IADD_IMM r2, r2^, #1;
           BRANCHZ.eq r3 -> 31                       ; i == 2: continue
B4  13     j r3 = 0 (.reconverge)
B5  14-16  inner header: S32_TO_F32 r4, r3; FCMP_OR.f32.ge.m1 r5, r4, r0;
           BRANCHZ r5 -> 25                          ; inner exit
B6  17-18  ICMP_OR.u32.ne.m1 r5, r3, 0x1000000.b3 (j != 1); BRANCHZ.eq r5 -> 23
B7  19-21  FADD_IMM r5, r0, #-2.0; FCMP_OR.f32.ge.m1 r4, r4^, r5^; BRANCHZ r4 -> 25
B8  22     FADD_IMM.f32.reconverge r1, r1^, #1.0     ; n += 1
B9  23-24  inner latch: IADD_IMM r3, r3^, #1; JUMP -> 14
B10 25-27  IADD_IMM r3, 0x0, #6.0; FCMP_OR.f32.ge.m1 r3, r1, r3^; BRANCHZ.eq r3 -> 31
B11 28-30  break edge: MOV r0, r1^ (n); MOV r1, r2^ (outer); JUMP -> 32
B12 31     outer latch: JUMP -> 4
B13 32-56  epilogue: mod/floor/min (as builtin_98), S32_TO_F32 outer, FMA by 32/255, BLEND
```

What the words show that the others do not:

- **Two break edges to one exit with copies.** The exit's phis (`n` in r0,
  `outer` in r1) take a `MOV` pair on each edge (B2, B11), so neither edge
  is a simple block and both stay `BRANCHZ.eq` over the copies.
- **`outer` is `i`.** `outer = i + 1` is computed before the `i == 2` test,
  so the step's copies of `i + 1` (the front end puts one before each
  `continue`, `Lower::loop_body`) are CSE'd into it (a dominating equal
  instruction, `nir_opt_cse`), and the header phis of `outer` and `i` have
  equal sources and are one: at the header exit `outer` is `r2` itself (B2).
  The increment stands in B3, before the `continue`, which therefore jumps
  straight to the latch (B12).
- **The inner `continue` is a branch to the latch**, the `j++` there once
  (B9: the `continue`'s copy and the end's merged by `opt_loop_last_block`,
  `nir_opt_loop.c:209-299`); the inner `break` is the simple-block inversion
  to the inner exit (B7 -> B10).
- `j != 1` and `i != 2` are `ICMP_OR.u32.ne.m1` (`Ine`, which `Fs::lane32`
  refuses today as `CompareForm`).

## loop_nested (`es100/loop_nested.frag`, mediump, 90 words)

Nine loops, each the shape of builtin_45's without the body's load: the
preheader ends in `IADD_IMM.i32.reconverge rC, 0x0, #0x0`, the header is
(sometimes a phi `MOV`, then) `ICMP_OR.s32.ge.m1 rT, rC, <uniform word>,
0x0` and `BRANCHZ rT -> exit`, the body is the `yzwx` swizzle as two
`MKVEC.v2i16` of the f16 pairs (`MKVEC.v2i16 r3, r6.h1, r2.h0 ; MKVEC.v2i16
r2, r2^.h1, r6^.h0`), `IADD_IMM rC, rC^, #1` and the back `JUMP`. Bounds are
the FAU words `ui_one` `u2.w0`, `ui_two` `u2.w1`, `ui_three` `u3.w0`;
`u3.w1` is the 1.0 pair of the epilogue.

| words | nest | loops (header word -> exit word) |
|---|---|---|
| 0-4 | load, `V2F32_TO_V2F16` x2 (`coords` r0/r1), `res` = copies r2/r3 | |
| 5-18 | nested | outer 6 -> 19 (`ui_two`), inner 9 -> 16 (`ui_three`); the outer latch 16-18 copies the inner's result back (`MOV r3, r6^`) |
| 19-43 | nested_sequence | outer 22 -> 44 (`ui_three`), inner A 25 -> 32 (`ui_two`), inner B 34 -> 41 (`ui_one`) |
| 44-59 | tricky_dataflow_1 | outer 45 -> 60, its body resets `res` (`MOV r4, r0 ; MOV r6, r1`), inner 50 -> 57 |
| 60-77 | tricky_dataflow_2 | outer 62 -> 78, inner 65 -> 73; the body's tail 73-76 rotates once more and copies (`coords = res`) |
| 78-89 | epilogue: `FADD.v2f16`/`FMA.v2f16` of the four results, `MKVEC`, `BLEND.v4.f16` | |

Carried values are 16-bit pairs in 32-bit registers, copied whole by
`MOV.i32` at headers and latches; nothing is unrolled, nothing hoisted.

## loop_nested_vs (`es100/loop_nested.vert`, the varying program, 53 words)

The same two nests in the **vertex** stage: words 1-2 `V2F32_TO_V2F16`
(`coords`, no `.discard`), 3-18 the `nested` nest (outer header 6 -> 19 on
`u5.w1` = `ui_two`, inner 9 -> 16 on `u6.w0` = `ui_three`), 19-34
`tricky_dataflow_2` (outer 20 -> 35, inner 23 -> 30, tail 30-33), then
35-52 the varying stores with `F16_TO_F32` per lane (`v_color = vec4(a.xy,
res.zw)`). The uniform words follow the vertex program's own FAU words
(`u0`..`u5.w0`: viewport and varying offsets). The position program (24
words) and the fragment (`va.frag`, 7 words) are the usual ones and compile
today.

## contloops (`es100/contloops.frag`, mediump, 72 words)

Three loop forms in one program:

- words 0-39: `for (i < 8)` with `if (i >= n) break; if (i == 1)
  continue;`: **unrolled** (`complex_unroll`, Part 1), as a ladder of
  `ICMP_OR.s32.{ge,lt}.m1` of byte constants against `n` (r1) and
  `MUX.v2i16` of the f16 pairs, the mediump form of case 87.
- words 40-55: `while (k < n) { k++; if (k == 2) continue; res.x += 0.125;
  }`, **kept**:

  ```
  40  IADD_IMM.i32.reconverge r3, 0x0, #0x0        ; k = 0
  41  ICMP_OR.s32.ge.m1 r7, r3, r1, 0x0            ; header: k >= n
  42  BRANCHZ.reconverge r7^.h0, offset:13         ; -> 56
  43  FADD.v2f16 r7, r0.h00, u2.w1                 ; res.x + 0.125
  44-46 MKVEC.v2i16 (the candidate pairs)
  47  IADD_IMM.i32 r3, r3^, #0x1                   ; k + 1
  48  ICMP_OR.u32.eq.m1 r6, r3, 0x3020100.b2, 0x0  ; k + 1 == 2
  49  MUX.v2i16 r0, r2^, r0^, r6.h00               ; the continue as a select
  50  MUX.v2i16 r4, r4^, r4^, r6^.h00              ; (degenerate, as dumped)
  51-54 U16_TO_U32 r2, r0, r6, r4                  ; the 16-bit phis' copies
  55  BRANCHZ.eq.reconverge 0x0, offset:-15        ; -> 41
  ```

  The `continue` is gone: `opt_loop_last_block` moves the rest of the body
  into the `if`'s else side, and `nir_opt_peephole_select` makes the `if` a
  select. The carried `vec4` is four 16-bit phis whose copies are
  `U16_TO_U32` (not `MOV.i32`).
- words 56-65: `do { if (m == 4) break; if (m == 1) continue; res.y +=
  0.0625; } while (++m < n);`: unrolled (`m == 4` is the limiting
  terminator), selects over `n` against 3 and the earlier masks; `do {
  continue; } while (++j < 3)` folds to `j = 3`, the alpha 0.75 (`u4.w0`).

## Plan for loops-rt, in order

Each step ends with its references byte-equal and the corpus moved forward
(`tools/mali/corpus-verdicts.sh --write` in the same commit, `MALI_TESTS` in
`tools/mali/gate.env` counted again). The tests are byte-equal tests named
as the existing ones (`<ref>_is_byte_equal_to_mesa_26_2_1` in `tests.rs`,
`check_program` / `check_linked` against the `expect.json`, every region
and the FAU words); the unit tests named are the ones each step needs
beyond that.

**Prerequisites from other items** (loops-rt starts after them):

- intbool (session 1): int values as IR values in both precisions (a
  `ConstI32` in a mediump shader, `walk_nodes` and `nir.rs` `build`), `F2i`
  of a run-time value in mediump, `Iadd`, `Icmp` (`s32.ge`, `u32.eq`), int
  uniforms as FAU words read by the compare (`ui_*`, `n`, `idx`; today
  `UniformType` in `walk_nodes`), and the folding of `Ineg` of a constant
  (`int last = -1` -> `#0xFFFFFFFF`, today `Arithmetic` in `Fs::lane32`).
  Every reference here needs these.
- intbool-2: the read of a vec4 array element at a computed index
  (`LD_PKA.i128` at `(idx + i) << 4`; today `nir_fs.rs` takes one int
  uniform as the whole index, two words): builtin_45.
- builtin: `gl_VertexID` in `nir.rs` `build` (`SpecialInput`), for
  `vid.vert`, the vertex shader of builtin_45 and builtin_98. Their
  fragments read no varying, so the linked vertex program is the plain
  24-word position program; whether the linked compile
  (`compile_vs_for_reads` with no reads) still refuses the unused
  `gl_VertexID` was not measured: if it does, builtin_45/98's byte-equal
  tests wait for builtin, otherwise only their single-stage verdicts do.
- Not owned elsewhere, so loops-rt's own: `Ine` (`ICMP_OR.u32.ne.m1`,
  `CompareForm` today; divnest) and `mod()` (`Fmod`, `lower_fmod`:
  `fsub(a, fmul(b, ffloor(fdiv(a, b))))`, the divide by a constant a
  multiply, the subtract fused: `FMA`, `FROUND.rtn`, `FMA` by `-b` plus `a`;
  divnest and builtin_98's epilogues).

**Step 1, the kept loop in the NIR walk (loop_exit).** In `nir_fs.rs`
`walk_loop`, decide before walking whether Mesa unrolls: a loop with a
limiting terminator (a constant bound within `MAX_UNROLL_ITERATIONS`, the
current `walk_passes` path) or not (`nir_opt_loop_unroll.c:1112-1114`; the
front end's trip-count analysis, `src/shaderc/online/loop_count.rs`, is the
place to ask). A kept loop gets, in `nir.rs` beside `IfRegion`, a
`LoopRegion { preheader, header, latch, exit }` and blocks in `emit_loop`'s
order; at the header one phi per local the body stores
(the front end's `subtree_stores_local` in `src/shaderc/online/ir.rs` is
private; the walk needs such a scan), in first-store order, its latch source
filled after the body is walked; the front end's `if (c) {} else { break; }`
at the head is the exit test; an exit block whose phis are the values at
each break edge. The walk must not fold the counter (it is a node, not
`Val::Const`). Emission (`nir_fs.rs` `end_block` and `select_highp`): emit
Mesa's unoptimised blocks (the `if`'s `BRANCHZ.eq`, the `break` `JUMP`, the
back `JUMP`) and let `opt.rs`'s `try_remove_simple_block` transcription do
the inversion, so `BRANCHZ` vs `BRANCHZ.eq` and the offsets come out as
Mesa's. Back end with a back edge: `sched.rs` `liveness_ssa` and `ra.rs`
`liveness_ra` already iterate to a fixed point; `flow.rs`
`insert_flow_control_nops` runs the scoreboard **once in layout order**,
which is wrong with a back edge: make it the worklist of
`va_assign_scoreboard` (`va_insert_flow.c:281-302`), and check `flow.rs`
`mark_last` across the back edge; `ra.rs` `out_of_ssa` must put the latch's
phi moves before its `JUMP` (`logical_end`). Tests:
`intloop_loop_exit_is_byte_equal_to_mesa_26_2_1`; a unit test of the
scoreboard over a back edge (a load issued in the body and read at the
header); `MaliError::LoopForm`'s text (`mod.rs`) and `walk_loop`'s doc
rewritten, since a run-time loop is then not the refusal.

**Step 2, carried copies and a second carried int (builtin_98).** The
header `MOV r3, r2` and the latch's `MOV r1, r3^` come from `out_of_ssa` as
it is; the step checks them, the `-1` constant phi source
(`IADD_IMM #0xFFFFFFFF`), and the `Fmod` epilogue. Test:
`builtin_98_is_byte_equal_to_mesa_26_2_1` (with the `--uniform-values`
FAU word for `count`).

**Step 3, nesting, `continue` and break edges (divnest).** A loop inside a
kept loop; `continue` as a `JUMP` to the latch; `if (c) { ...; continue; }`
reshaped as `opt_loop_last_block` does (the walk's `IfExit::Continue`
already models it for unrolled passes); a `break` behind a run-time
condition anywhere in the body (not only at the head) as an edge to the
exit block with its phi sources; the exit phis' copies on each edge (B2,
B11); CSE of equal header phis (`outer` and `i`) and of the step's `i + 1`
copies into the dominating `outer = i + 1`; `Ine`. `MAX_IFS` (`nir.rs`, 16)
and the blocks (`bir.rs` `MAX_BLOCKS`, 32) are counted here: divnest is 14
blocks after the clean-ups, more before. Test:
`intloop_divnest_is_byte_equal_to_mesa_26_2_1`, with its vertex program
(the `* 8.0` motion).

**Step 4, a load in the body (builtin_45).** After intbool-2's indexed
vec4 read: the index a run-time `IADD` of a uniform word and the counter,
the `LD_PKA` inside the loop (its slot and wait in a block that is entered
twice: the step-1 scoreboard), four carried lanes. Test:
`builtin_45_is_byte_equal_to_mesa_26_2_1`.

**Step 5, mediump kept loops (loop_nested).** The pair path (`select`,
mediump fragment) has no blocks today (`walk_if` and `walk_loop` refuse
`!prec.highp`, and only `select_highp` ends blocks): give it the loop
blocks of step 1 with carried f16 pairs (`MKVEC.v2i16` swizzles, whole-word
`MOV.i32` copies), nine loops in sequence and nested. Count the blocks
first: nine loops with Mesa's unoptimised `if` blocks may pass `MAX_BLOCKS`
(32) before the clean-ups; sizelimit owns those caps now, so raise it here
only after sizelimit has merged. Test:
`intloop_loop_nested_is_byte_equal_to_mesa_26_2_1`.

**Step 6, loops in the vertex stage (loop_nested_vs).** `nir.rs` `build`
walks only straight code (`ControlFlow` on any other node): give the
vertex walk the same loop regions and its selection the blocks, then the
`F16_TO_F32` stores after the loop. Test:
`intloop_loop_nested_vs_is_byte_equal_to_mesa_26_2_1` (the varying program
is the subject; the position and fragment programs as usual).

**Step 7, contloops.** The largest: the mediump form of Part 1's guarded
ladder (`MUX.v2i16` of pairs; the control-flow item's cases 70/71 need the
same and may land it first), a kept `while` whose `continue` becomes a
select, the 16-bit phis' `U16_TO_U32` copies, the degenerate `MUX` kept, and
the folded `do { continue; }`. If the mediump ladder has not landed by
then, split this step off rather than grow loops-rt. Test:
`intloop_contloops_is_byte_equal_to_mesa_26_2_1`.

**Corpus lines expected to move** (each to `ok` or to a later gap, never
back): `es300/loop_exit.frag`, `es300/divnest.frag`, `es300/uniloop.frag`,
`es300/dynidx.frag`, `es100/loop_nested.frag`, `es100/loop_nested.vert`,
`es100/contloops.frag`, and with them the same-shape sources
`es100/loop_nested.{while,dowhile}.frag`, `loop_uniform*`, `loop_full.frag`
(`mali-int-loops.md`, "Not recorded"), whose verdicts are to be read, not
assumed.

## Open points (measure before relying on them)

- The header phi order (first store) rests on three references; a loop
  whose locals are first stored in another order than declared would tell.
- Where Mesa puts the step's `i + 1` (before the `continue` in divnest's
  outer loop, in the latch in its inner loop) is explained above by CSE into
  `outer = i + 1`; the per-pass NIR (`NIR_DEBUG=print_fs`) was not drawn.
- The linked compile of `vid.vert` for a fragment that reads nothing (see
  the builtin prerequisite). Measured since: it compiles (the position
  program alone), `cases_45_and_98_vertex_programs_are_byte_equal_to_mesa_26_2_1`.

## Status of Part 2 (plan #154 loops-rt)

Byte-equal (tests in `tests.rs`): `intloop_loop_exit` (step 1),
`builtin_98`'s fragment program (step 2, with its `mod` epilogue),
`intloop_divnest` (step 3), `builtin_45`'s fragment program (step 4), and
femtovg's `filter_image` and `filter_image.nodiscard` (Part 1's guarded
unroll with the guards left as branches).

Not lowered: the other three references. Each needs a path the back end
does not have, not a missing case in the paths above, so each is refused
with a gap of its own (pinned in `tests.rs` `gap_cases` and in
`tools/mali/corpus-verdicts.txt`, the same-shape corpus sources with them).
Measured at plan #154 (d)4 by making each `ControlFlow` site return a
distinct gap and reading the corpus verdicts:

| reference | stops at | gap | what lowering it needs |
|---|---|---|---|
| `intloop_loop_nested` (step 5) | `nir_fs.rs` `walk_kept_loop`, `!prec.highp` | `LoopMediump` | Blocks in the mediump fragment path. `select` rebuilds the colour's expression trees into lane pairs and emits one block; it has no phis, no edges and no per-block selection. The words need the header phis as 16-bit vectors (`MOV.i32` of whole pairs, words 3-4, 9, 17), the `yzwx` swizzle as two `MKVEC.v2i16` in the body, int uniforms read by `ICMP_OR` in a mediump shader, and the epilogue's scalar fp16 sums (`FADD.v2f16 .h00/.h11`) over the loops' results. |
| `intloop_loop_nested_vs` (step 6) | `nir.rs` `build`, the first `Loop` node | `LoopVertex` | Three things the vertex stage lacks. (1) Control flow: `build` walks straight code, and the vertex selection has no blocks. (2) fp16 values: mediump temporaries are `V2F32_TO_V2F16` pairs, then `F16_TO_F32` per lane at the stores. No vertex reference had a mediump temporary before. (3) `int` uniforms read by value (`u5.w1`, `u6.w0`): `build` takes an `int` uniform only as an index (`UniformType`). |
| `intloop_contloops` (step 7) | `nir_fs.rs` `walk_if16`, a pass's guard (`IfExit::Guard`) | `GuardMediump` | Before its kept `while` comes the mediump form of Part 1's `complex_unroll` ladder (words 0-39: `ICMP_OR` against byte constants, `MUX.v2i16` of fp16 pairs, `MKVEC` re-pairing). This is the shape the control-flow item's cases 70/71 also need. After the ladder it needs `LoopMediump`'s blocks, plus scalar 16-bit phis copied by `U16_TO_U32` (words 51-54). |

The work splits into three steps, each a block emitter of its own (as
`select_highp` is for the highp path), so each is its own item, in this
order: mediump blocks (loop_nested), then the mediump guard ladder on top
of them (contloops), then the vertex stage (loop_nested_vs).

Where the implementation differs from the plan above:

- A kept loop (`nir_fs.rs` `kept_loop`, `walk_kept_loop`) is decided from
  the body's head test alone: a test the walk cannot fold at the first pass
  keeps the loop. `nir::LoopRegion` holds `pre`, `header`, `latch`, `exit`;
  edges are added in the order the regions close (`IfRegion::seq`).
- Mesa 26.2.1's GLSL front end puts a `for` step in the loop's continue
  construct (glsl_to_nir.cpp:794-797), and `nir_lower_continue_constructs`
  moves the code after each `continue`'s `if` into its other side, then
  inlines the step once before the back edge
  (nir_lower_continue_constructs.c:91-140). yakogl's front end copies the
  step before each `continue` instead (`Lower::loop_body`), so the walk
  takes `if (c) { step; continue; }` at the top of a kept body as
  `if (!c) { rest } else {}` followed by the step, the copy matched against
  the body's tail (`same_instrs`). That is why divnest's inner `j++` is in
  the latch (B9) and the outer `i + 1` is the one of `outer = i + 1`.
- Two `break` edges: the exit's phis are made for the lanes the edges
  leave different, in rank order (`n` before `outer`: the copies `MOV r0,
  r1^; MOV r1, r2^` on both edges; the other order puts `n` in r1 and
  copies only `outer`). Equal header phis are one (`outer` and `i`), the
  first kept. A local the body stores before any read (`j` in the outer
  loop) has no header phi.
- The scoreboard (`flow.rs`) and `mark_last` iterate to a fixed point over
  the back edge; `opt.rs` `dce`, `cse` and `single_use` see a header phi
  that reads a later value.
- `i2f(f2i(a))` is `ftrunc(a)` (`Alu::Ftrunc`, `FROUND.f32.rtz`).
- filter_image: Mesa unrolls it (`complex_unroll`) but its 24 guards are
  too large for the peephole, so they stay `BRANCHZ.eq` over each pass, the
  break's five copies after the pass (`Nir::lay_out_inverted_guards`
  renumbers the two sides' blocks). The unroll's size test counts
  `instr_cost` as Mesa does (`unroll_cost`: phis and an `fmul` only `fadd`s
  read are free), not every node. A texel read with a highp non-constant
  value makes the look-up fp32 (`.32`), as GLSL's lower_precision leaves
  it (hdrsample's `* 0.25` stays fp16); `fneg` of a constant in the late
  fusion folds (`-3.0` an `IADD_IMM`). The only words that differ are the
  `skip` bits of the first 18 (19 without the discard) look-ups, a
  value-number collision in `bi_add_branch_compare_values`, pinned in
  `FEMTOVG_SKIP_PINNED`.
- Caps raised for it: `bir::MAX_BLOCKS` 128 (82 used), `bir::MAX_INSTRS`
  and `MAX_NODES` 1536 (1419 instructions), `nir::MAX_NODES` 1536 (1153),
  `nir::MAX_IFS` 32 (27); the scheduler refuses a block over 512
  instructions (`sched::MAX_BLOCK_INSTRS`: its edge rows are the block's
  size squared), and `liveness_ssa`'s sets live in the scratch arena.
