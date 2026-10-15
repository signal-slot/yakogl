<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The V3D 4.2 back end (Raspberry Pi 4): design record

Status 2026-10-11: the instruction set (`src/shaderc/online/v3d/`: model,
encoder, decoder, round trip), the back end (`prepare`, `select`, `sched`,
`alloc`, `verify`, `chain`) and the GL core's V3D back end (`src/v3d/`, "M6"
below) are implemented, and the Pi 4 reads 0 dEQP-GLES2 failures (17,126 pass /
34 NotSupported, `tools/cts/RESULTS.md`). The sections from "M6" on were written
as the work went (dated plans and board readings); their status lines are those
of their day unless a later line says otherwise, and code cites them by heading.
Status 2026-09-24, as written then: 43 corpus pairs compile (40 in a
`report-es2` build, which refuses GLSL ES 3.00). On the board, our programs drew
the `v3d_triangle` sample three ways: plain, with uniforms, and with an `if`
(`docs/notes/pi4-v3d-bringup.md` has the result rows). The
product path for this GPU is this compiler only (`driver/docs/rpi4-v3d.md`,
"Shader compiler"); Mesa's compiler, built as `tools/v3d/v3d_shc`, is the
reference we are compared with, never a path into the product. Two dated
sections, the M9 Bevy survey and the slow-job readings of the E runs, are the
record `ab43a20c5:tools/cts/log/v3d-backend-diary-2026-09.md`.

References: Mesa 26.2.1 `src/broadcom/compiler/` and `src/broadcom/qpu/` (MIT,
transcribed with attribution where we follow them). No GPL source is read for this
work.

## What is different from the other two back ends

- **A program, not a stage.** The VS writes its varyings in the order the FS reads
  them (Mesa keys the VS by the FS's `input_slots`), so a GL program's code exists
  only once both stages are known: `glLinkProgram`, not `glCompileShader`.
- **Three QPU programs per GL program**: the FS, the VS, and the coordinate shader
  (CS) the binner runs. The CS is the VS writing only what binning needs (clip-space
  position, scaled xy, and the point size when the VS writes gl_PointSize).
- **Variants by primitive**: the FS for points and lines reads the point/line
  coordinate as implicit varyings (`is_points`, `is_lines`). Mesa's VS keeps the
  point size only for points; ours keeps it for every primitive when the shader
  writes gl_PointSize (Mesa's per_vertex_point_size layout: VS row 4, CS row 6,
  the varyings one row later), and reports `writes_psiz`. The record bit only
  describes the layout: with the word kept and the bit set, a triangle and lines
  drew 0 of 4096 pixels differently on the Pi 4 (gl_test case 110, 37264ce2), so
  the VS needs no points variant.
- **The machine is scalar with a 16-wide SIMD per QPU**, like the USC and unlike the
  GC7000's vec4. The IR's vectors are scalarised first, as `powervr/prepare.rs` does.
- **Dual issue**: one 64-bit word holds an add-unit op and a mul-unit op plus a
  signal. Pairing is a scheduling question, not a selection one.
- **Registers**: accumulators r0..r5 beside the register file rf0..rf63; r5 is
  written by `ldunif`/`ldvary`'s C term; the file is split between threads, so the
  thread count is chosen from register pressure: 4 (rf0..rf31 a thread), else 2
  (rf0..rf63), as Mesa's thread loop does; never 1, which on 4.x has no more
  registers than 2 and only helps with spilling (`chain.rs` `build`). The
  register-file bound is checked on every program (`verify.rs`
  `registers_fit_threads`), because the interpreter gives every thread 64.
- **I/O is through special paths**, each with its own ordering rules: uniforms from a
  per-draw stream (`ldunif`), varyings from `ldvary` (plus the W and C terms), vertex
  attributes from the VPM (`ldvpmv_in`), vertex outputs to the VPM (`stvpmv`, ending
  in `vpmwt`, GFXH-1684), fragment colour to the TLB (`tlb`/`tlbu`, the first write
  carrying a config uniform), textures through the TMU.
- **Thread switches** (`thrsw`) with two delay slots, and the program end is a
  `thrsw` with its own rules.

## I/O conventions: Mesa's, exactly

The driver builds the shader state record and the uniform stream
(`driver/src/v3d/v3d_frame.c`); it must serve Mesa's code (the M5 vehicle) and ours
alike. So the back end reproduces Mesa's conventions, not its code:
- the uniform stream is a list of `(contents, data)` with Mesa's
  `enum quniform_contents` values (`compiler/v3d_compiler.h`) for the kinds we emit;
- the VPM output layout of the VS and CS (`v3d_nir_lower_io.c` for v42: CS writes clip
  xyzw, scaled xy, then what binning needs; VS writes scaled xy, Zs, 1/Wc, varyings);
- the reported prog_data fields the record reads (threads, single_seg, VPM sizes,
  `vcm_cache_size`, input slots and flat/noperspective/centroid flags).

## Pipeline

1. **prepare**: IR to scalar virtual operations; the IoSlots to the I/O paths above;
   the FS's input order decides the VS's output order.
2. **select**: scalar operations to QPU operations on virtual registers (add/mul unit
   choice, signals, magic write addresses). Every IR op in the kernel (`ir.md`) is
   either selected or refused by name.
3. **schedule**: pairing into words, `ldvary`/`ldunif` and r5 hazards, `thrsw`
   placement and delay slots, the program end.
4. **allocate**: accumulators and the register file; thread count; no spilling to
   memory. A program that does not fit 2 threads is selected again with shorter
   live ranges (`select.rs` `Pressure`, tried in order by `chain.rs`
   `under_pressure`): each vertex attribute read where it is first needed, then
   every uniform and constant that is not a small immediate loaded again at each
   read, as Mesa rematerialises a uniform it would otherwise spill
   (`v3d_spill_reg`). A program that fits keeps Mesa's layout; one that still does
   not fit is refused, by name. In every program a uniform or a constant is loaded
   at its first read, not where the IR first names it (Mesa's `nir_opt_move` with
   `nir_move_load_uniform | nir_move_const_undef`), and integer arithmetic on
   constants is folded in prepare (the index an unrolled loop leaves behind), so
   an array read at a constant index reads one element.
   After allocation, `order.rs` reorders each region (between thread switches,
   loop heads, back edges, branches and labels) the way a list scheduler would
   (Mesa's `choose_instruction_to_schedule`: the longest path first, no uniform
   load while a varying is still to be loaded), with every dependency between
   physical registers, r5, the flags, rtop and the outside effects kept. The
   packer schedules both orders and keeps the shorter program: its word model is
   not the pre-pass's, and on some programs (femtovg's fill_color) the selected
   order packs shorter. On the femtovg programs: 3506 -> 3428 words, and 14836 ->
   14494 executed per the 58 pinned runs (`tests/v3d_sim_mesa.rs`), none longer.
5. **encode**: `v3d::encode_program`.
6. **verify**: the rules between instructions Mesa checks (`compiler/qpu_validate.c`),
   run on every program we emit.

## How it is checked without a board

The repository's pattern (`eval.rs`, `tests/common/mod.rs`,
`tests/backend_differential.rs`), for this GPU:

1. **A QPU interpreter** of encoded words (test-only): one QPU, 16 lanes, the add and
   mul units, accumulators and register file, the signals we emit, the uniform
   stream, varyings (W and C), VPM in and out, and TLB writes. Anything it does not
   model stops the run as "unmodelled", never a value.
2. **The interpreter is validated on Mesa's code first**: for corpus pairs `eval.rs`
   can run, Mesa's FS/VS/CS from `v3d_shc`, run on the interpreter, must give what
   `eval.rs` gives for the same IR and inputs. A mismatch there is an interpreter
   defect (or an `eval.rs` one), found before our code is judged with it.
3. **Our code, three ways**: for every program we compile, the interpreter on our code
   must agree with `eval.rs` and with the interpreter on Mesa's code for the same
   source. Transcendentals are compared with a stated tolerance, everything else
   bit for bit.
4. **The structural verifier** passes on everything we emit.
5. **On the board**: gl_test cases per shape, pixel read-back, as for the other GPUs.

The interpreter vouches only for what it models; a rule it does not model (a real
hazard the hardware has and the interpreter does not) is the verifier's job, and the
board's.

## SFU precision: what is modelled, what is not measured

Status 2026-09-24. **Board precision is unmeasured for all five SFU operations.**

What we emit (`select.rs`), the same operations Mesa emits:

| IR | QPU | Mesa 26.2.1 |
|---|---|---|
| `Frcp` | `recip` | `nir_to_vir.c:1742-1744` |
| `Frsq` | `rsqrt` | `:1745-1747` |
| `Fexp2` | `exp` | `:1748-1750` |
| `Flog2` | `log` | `:1751-1753` |
| `Fsqrt` | `recip(rsqrt(x))` | `lower_fsqrt` (`v3d_screen.c:585`), `nir_opt_algebraic.py:1908` |
| `Fsin`, `Fcos` | `x * (1/pi)` (+ 0.5 for cos), `fround`, `fsub`, `sin`, sign from the period's low bit | `ntq_fsincos`, `nir_to_vir.c:991-1001` |

Mesa uses the SFU result as it comes: no Newton step, no fix-up. Its v3d compiler and
gallium driver make no claim about SFU accuracy (a search of
`src/broadcom/compiler/` and `src/gallium/drivers/v3d/` for "accura", "precision",
"ulp" and "approximat" finds only texture and image notes). So our code and Mesa's
have the same error on silicon by construction. That is an equivalence, not a bound.

What each instrument computes:

- **The interpreter** (`sim.rs`, module doc on the SFU and its `Recip | Rsqrt | Exp |
  Log | Sin` arm): the exact f32 function of the operand, `1/x`, `1/sqrt(x)`,
  `exp2`, `log2`, and `sin(pi * x)`. A SIN operand outside `[-0.5, 0.5]` stops the
  run as unmodelled. Every SFU op a run executes is listed in `Outcome::approximate`.
  This is an **exact-f32 model, not an error model**.
- **`eval.rs`** (the IR's meaning): the exact f32 function on the unreduced argument
  (`1/x`, `1/sqrt(x)`, `sqrt`, `exp2`, `log2`, `sin`, `cos`).
- **The comparison** (`tests/v3d_sim_mesa.rs`): a run that executed an SFU op is
  compared with a tolerance. f32 within `1e-5` relative (`close_f32`), f16 within one
  unit in the last place with the same sign (`close_f16`), scaled integers within 1
  (`close_int`). Otherwise the comparison is bit for bit.

So the three-way check shows that our selection computes the IR's maths **under the
exact-f32 SFU model**, within 1e-5 relative. That includes the error our own lowering
adds: `sqrt` rounds twice, and `sin`/`cos` reduce their argument in f32 (`x / pi` loses
low bits as `|x|` grows). It shows **nothing** about the silicon's SFU error.

What the language asks (GLSL ES 1.00 revision 17, section 4.5.1 and the table in
section 4.5.2): highp has a relative precision of 2^-16, mediump 2^-10. Section 4.5.1
adds that "a small number of computationally intensive built-in functions, e.g. atan(),
... may return results at less than the declared precisions".

What a board run could tell apart: the M5 path writes the colour through `vfpack`
(f16) and an RGBA8 target, so a read-back resolves about 2^-8 per channel (2^-11 at
best in the f16). No error smaller than that can be seen. It could show a gross SFU
defect (a wrong function, a wrong range reduction). It cannot check the highp
requirement of 2^-16. That needs a 32-bit float render target or a TMU write, and M5
has neither. Until one exists, "board precision unmeasured" stays the verdict, and a
passing picture is not evidence about it.

## Control flow, step 1: `if` by if-conversion

Status: implemented 2026-09-24 (`prepare.rs`, `Walk::if_convert`). Loops, `break`,
`continue`, a `return` that is not the end of `main`, and `discard` stay refused by
name. Evidence, as planned below: the `prepare.rs` tests (two negative controls, the
arms swapped in the select and the else walk not restored, each failing them), the
three-way check (40 of 40 of our programs agree with the IR, 27 with Mesa's, 0
disagree, 67 of 67 mutants caught), and the board sample `-DTRI_IF=ON` (built, not
run yet).

**What Mesa does.** v3d flattens every small `if` into selects before instruction
selection: `nir_opt_peephole_select` with `limit = 24`, `expensive_alu_ok` and
`indirect_load_ok`, and again with `limit = 0, discard_ok`
(`nir_to_vir.c:2256-2284`). Only a larger `if` reaches the back end as control flow,
where Mesa runs it with a per-lane execute mask and conditional writes, and branches
around a block no lane runs. The PowerVR half made the same choice for the same
reason (`backend.md`, "the reference does not branch, it if-converts").

**What we do.** Every `if` is if-converted, with no size limit, in `prepare.rs`. That
is the one place that already holds the program's state as values: each local lane,
`gl_Position`, `gl_FragColor` and each stored varying lane is an `S` (a scalar value
id), and a store replaces it. So an `if` needs no branch, no execute mask and no
change below `prepare`:

1. Prepare the condition: lane 0 of `cond`, a bool (~0/0, the `select.rs` convention).
2. Take a snapshot of the state (locals, position, colour, stored varyings).
3. Walk `then_` from the snapshot, giving state T. Restore the snapshot, walk `else_`,
   giving state E.
4. Merge, slot by slot and lane by lane:
   - T and E hold the same value: keep it, with no select.
   - They differ: `Bcsel(cond, T, E)`, which `select.rs` already emits (a flag push
     and a conditional move).
   - **One arm wrote and the other did not**: the other arm's value is the
     snapshot's, the value from before the `if`, never "undefined". This is the
     case that miscompiled 30 of 137 sources on the PowerVR (the one-arm-writes
     defect, 2026-09-22), and it gets its own test, with distinct values on both
     sides.
   - Neither the snapshot nor the arm that did not write has a value (a local first
     written inside one arm): GLSL leaves the other path's value undefined, so any
     value is correct. We take the written arm's value, say so in the code, and pin
     it with a test. `eval.rs` reads such a slot as `UNSET` (0x7fc0dead, a quiet
     NaN), so its output on that path is a NaN where ours is a number. The
     three-way check cannot vouch for that lane, and it must count it as not
     compared, never as agreeing or disagreeing.
5. Nested `if`s recurse. Inside an arm only `Instr` and `If` nodes are allowed.
   `Loop`, `Break`, `Continue` and `Return` in an arm are refused by name.

**Why running both arms is safe here.** Every operation this cut selects has no side
effect: the ALU and SFU ops, the uniform, attribute and varying reads. The QPU has no
floating-point traps, so an untaken arm's `rsqrt(-1.0)` or `1/0` makes a NaN or an
infinity that the conditional move never picks. The varyings are still read once each
in input order, because the result is straight-line code and `fragment_inputs` orders
the loads as before. A texture read inside an arm (not selected yet) and `discard`
(refused) are the two operations that change this. They are decided when they come
(Mesa speculates small texture arms: `indirect_load_ok`), not now.

**What it costs.** Both arms always run. Mesa runs an `if` with more than 24
instructions an arm as real control flow. We do not, so a large `if` costs more
instructions than Mesa's version, never a wrong value. Real control flow (the execute
mask, the uniform branch that skips an empty block, the branch's delay slots and the
verifier's branch rules) is a later step. It will be justified by an instruction-count
comparison with Mesa on the corpus, not added in advance.

**Flags.** Each merged lane is one `Bcsel`. When the condition is a comparison, the
select fuses its flag push as it does now. Otherwise it tests the bool against zero
(`MOV.pushz`, `ifna`). Reusing one push for many conditional moves is an optimisation
for later. The scheduler already refuses a conditional move with no push before it
(`SchedError::CondWithoutFlags`).

**Evidence, landing with the change** (the CLAUDE.md rule for code that changes what
instructions compute):

- `prepare.rs` tests:
  - both arms write;
  - one arm writes, then with the arms swapped (the value from before the `if` must
    survive, with distinct constants so a wrong merge shows);
  - a nested `if`;
  - `gl_FragColor` written in the arms;
  - a VS varying stored in one arm;
  - a local first written inside one arm;
  - each refusal inside an arm.
- The three-way check on every corpus pair that the `if` refusal now lets through
  (13 at the last count). Each must agree with the IR, and the lanes that read an
  undefined value are counted separately.
- A board sample with pixel read-back on **both sides of the condition**: the
  triangle with an FS `if` on an interpolated value whose threshold crosses the
  triangle, one arm writing the colour and the other not, so both sides and the
  one-arm case are drawn in one picture. The check expects each pixel's side from
  the barycentric value, and pixels within one step of the threshold are an edge
  class, as the triangle's edges are.

## Control flow, step 2: bounded loops (implemented)

Status (2026-09-24): both halves are implemented, on the host; no board run yet.
The V3D chain calls the unroller (`chain.rs` `unrolled`), so every loop that
leaves only by its Appendix A test is expanded (dEQP-GLES2 light_amount's light
loops among them). A loop the unroller keeps runs as a counted loop:
- prepare.rs `counted_loop`: every lane of a local the body writes becomes a
  `Phi` before the loop and a `PhiNext` at its end; `LoopHead` and
  `LoopEnd { count, exit }` bracket the body. Refused by name: a kept loop
  inside a kept loop, and an output or a discard written inside one.
- select.rs: the counter is a CONSTANT uniform. At `LoopEnd` it is decremented
  and copied back, MOV.pushz sets the flags, and a `BackEdge` branches (AnyNA
  on the counter; AnyA on "no lane has stopped" when the unroller gave an exit
  flag). msfign P in a fragment shader.
- alloc.rs: a value defined before the head and read in the body lives to
  the back edge.
- sched.rs: the head is a word of its own; the branch is a word of its own
  with three delay slots (the words it moves back over, else NOPs: "Branch
  delay slots (step (g))"), outside a THRSW's delay slots; its REL offset
  and its uniform-stream offset are set once every word is placed.
- verify.rs `branches_are_counted_loops`: every branch in a program must be a
  recorded back edge, conditional, REL with the uniform stream moved, and
  backward to its loop's head. build() and the tests' check_all apply it.

What the unroller gives decides the exit. For a loop with a constant head
test, `count` includes the entry that only tests the bound and breaks (count
9 for `i < 8`). When the body can also `break` on a condition the compiler
cannot decide and every exit is uniform (filter_image's `i >= sampleCount`,
from a uniform), `Trip.exit` names the break flag and the loop leaves at the
break, as Mesa's does (unroll.rs, 2026-09-26: filter_image 2289 -> 156
executed, Mesa 65; before, the counter alone ended it and the passes after
the break ran under the flag, doing nothing). With a break that may differ
between lanes, `exit` is `None`: the counter alone ends the loop, and the
passes after a lane's break change nothing because the unroller guards every
store after it. For a loop whose count is read at run time (capped at 4096),
`exit` names the break flag, and the loop leaves once every lane has set it.
Which corpus loops the early exit reaches is pinned by unroll.rs
`the_corpus_loops_the_early_exit_reaches_are_pinned`: gl_test 70 loopcarry
and 71 deadloop, and femtovg's two filter_image variants.

How it is checked (tests/v3d_sim_mesa.rs; interpreter branches, delay slots
and uniform-stream moves as the design said):
- femtovg filter_image (both variants), es100/databreak, deadloop, loopcarry,
  and the EXTRA_PAIRS es100/loop_full (the whole count on most lanes) and
  es300/loop_exit (a run-time bound, 0..5): all agree with the IR.
  es300/divloop compiles but is not tried: its bound of 1e9 is past the cap,
  and the IR evaluator stops it as non-terminating.
- Controls, one hand mutant each: counter - 2 is caught (databreak,
  loop_full); counter - 1 is an equivalent mutant (the last pass only
  tests and breaks) and counter + 1 is one too (the guards); phi copies
  dropped: 2 disagree and 3 fail to compile; the uniform-stream offset one
  word off: the interpreter stops on all 5 loop pairs (an impossible
  uniform read); the branch target one word late: verify.rs refuses it ("a
  back edge whose target is not its loop's head"); the exit flag ignored:
  no value can show it, so the harness pins es300/loop_exit's instructions
  run (276 with the exit, 118,862 without; the pin is 1,000).

**Target.** femtovg's `filter_image` (its two variants are the only femtovg
shaders that do not compile). One loop: `for (float i = 1.0; i <= 24.0; i += 1.)`
with `if (i >= sampleCount) break;`, where `sampleCount = ceil(3.0 * sigma)` comes
from a uniform. Two texture lookups per pass.

**Where the trip count comes from: the existing unroller, used as it is.**
`powervr/unroll.rs` works IR to IR and is not PowerVR-specific. A loop that leaves
only by its GLSL ES 1.00 Appendix A test is expanded into its iterations, so
selection never meets it (once the V3D chain calls it, that case costs nothing
more). A loop that can leave on
data is KEPT: its body once, every exit a flag local, and a `Trip { first, count,
exit, divergent }`. `count` is the Appendix A bound (24 here). `divergent` says
whether the flag can differ between fragments. Correction, from reading
`roll()`: for a loop whose head test is constant (filter_image's `i <= 24`),
`count` is the static schedule's and `exit` is `None`; `exit` names the break
flag only for a loop whose count is read at run time (capped at
`UNIFORM_LOOP_CAP`). The break flag is still a local of the module. The V3D
chain calls `unroll()` without changing it. That file belongs to usc-backend: the V3D becomes a second
user, and moving it out of `powervr/` is their call.

**Two ways to run a kept loop.**

1. *Expand it anyway* (24 copies, each pass after the break guarded). No branch
   at all, but every pixel always pays 48 lookups, whatever sigma is. On the
   PowerVR the expanded filter_image was 4,094 dwords against 636 kept
   (unroll.rs). Not the plan; a fallback only.
2. *A counted loop* (the plan). The body appears once. A counter register starts
   at `count`. The back edge is one REL branch, taken while the counter is not
   zero AND at least one lane still has its break flag clear. The counter bounds
   the loop by construction; the flag only lets it leave early. Leaving when every
   lane has broken is safe even when the flag differs between fragments, because
   the unroller guards every store after a break, so a lane that broke changes
   nothing in the passes it still runs.

**The V3D mechanics** (Mesa, `qpu_schedule.c:2916-2983`; our interpreter already
models them, `sim.rs` "Branches"):
- A branch has 3 delay slots. No THRSW, no branch and no uniform read goes in
  them.
- A branch also takes one CONSTANT uniform: the byte offset that moves the
  uniform stream to the target (`ub`, REL). So each pass re-reads the body's
  uniform words (P0/P1 of each lookup, constants). Without it, the second pass
  would read the words after the loop.
- The condition is a flag pushed on the combined test (counter != 0 and flag
  clear), branched on with AnyA: taken while any lane still has work. msfign P,
  so discarded lanes do not vote. No uniformity is needed, so a divergent `Trip`
  does not have to be refused.
- Every lookup in the body retires (ldtmu) before the back edge, so no TMU
  request is outstanding across passes.

**What changes in our back end.**
- `select.rs`: a loop head label, the counter's init/decrement, the flag push
  and the branch.
- `alloc.rs`: a value live at the loop head, or defined in the body and read
  after it or in the next pass, is live over the whole loop (liveness over one
  cycle, not only a line).
- `sched.rs`: no instruction moves across the loop head or the back edge; the
  delay slots are NOPs at first.
- **The structural guarantee** (CLAUDE.md: the compiler must never emit a
  non-terminating program). The assembler refuses any backward branch that is
  not a loop node's back edge, any branch target outside the program, and any
  back edge whose condition does not include its own counter. `check_all`
  asserts it on every emitted program, as it does for the TMU rule. Its known
  positive: a hand-made program with a data-only backward branch is refused.

**How it is checked.**
- Three-way on filter_image. The harness draws sigma so that the trip count
  varies from pair to pair, including 0 passes and the full 24.
- Mutants that each must be caught: counter off by one, the exit flag ignored,
  the uniform-stream offset wrong by one word, the guard on stores after the
  break removed.
- The IR interpreter runs the original module with its loops as loops, so the
  IR side does not share the unroller's view.
- The board test is femtovg's filter_image through `v3d_gl_femtovg` (v3d-glue's
  sample).

**Decided by the lead (2026-09-24):** (a) the V3D chain calls `powervr::unroll`
unchanged (use is not modification; moving the file is usc-backend's call); (b)
the counted loop first, full expansion only as the fallback.

## Uniform branches (implemented)

Status: implemented 2026-09-26 (v3d-m5/vid). Evidence:
`chain.rs a_large_uniform_if_is_a_branch_round_each_arm` and
`tests/v3d_sim_mesa.rs executed_instruction_counts_are_pinned`. Look-ups in
an arm: `chain.rs a_look_up_in_a_uniform_branch_switches_again_after_it`
(gl_test 148's shader in the interpreter), `prepare.rs
a_uniform_if_with_a_look_up_is_a_branch`, `alloc.rs
a_merge_register_is_lent_inside_the_else_arm_it_is_not_live_in`.

**Why.** If-conversion (step 1) runs both arms for every fragment. Mesa keeps
an `if` as control flow when its arms hold more than 24 ALU operations
(`nir_opt_peephole_select` with v3d's `.limit = 24`, nir_to_vir.c:2261-2266).
When the condition is uniform, it branches round the untaken arm
(`ntq_emit_uniform_if`, :4293-4390). femtovg's `scissorMask` is such an `if`
(`scissorRadius > 0.0`), and it sits in every fill shader. The Slint cluster
draws with `scissorRadius` 0 (records/pi4/glcount_c510dbae_trace_frame300.txt),
so the rounded-rectangle arm is skipped. Executed instructions of
fill_image.nodiscard on that path: 104 before, 85 now; Mesa's 68.

**When** (prepare.rs `Walk::if_convert`): all of these must hold:
- a fragment shader;
- the condition is computed from uniforms and constants only (`is_uniform`),
  so every fragment of a draw takes the same arm;
- neither arm holds a discard, a loop or a jump (`branchable`). Look-ups
  and nested `if`s are allowed;
- the arms cost more than 24 scalar ALU operations, or an arm holds a
  look-up (`arms_cost`). Moves, negations and absolute values are not
  counted, as Mesa does not count its move-like operations. The cost is
  measured by walking both arms (each in a region of its own) and taking the
  operations back out. Mesa never flattens an `if` with a look-up in an arm
  (`block_check_for_allowed_instrs`, nir_opt_peephole_select.c: a tex
  instruction is refused unless the limit is ~0), so the draw that does not
  take it requests nothing from the TMU.

A kept loop's break on a uniform bound is uniform too (`uniform_locals`: a
local whose every store is of a uniform value, not indexed and not under a
non-uniform `if`; `is_uniform` accepts a phi of such a local), so the
unroller's guard round the rest of a pass is a branch. Its other guard,
round the whole body on the flag as the pass found it, is clear in every
pass that runs and is dropped (`clear_at_pass_start`). femtovg's
filter_image (a 24-pass blur, two look-ups a pass, a break on
`sampleCount`) executes 110 instructions on the cluster path (Mesa 65;
2289 when every pass ran, 143 before look-ups could sit in an arm).

**The last thread switch.** A look-up's switch inside an arm may be skipped,
so it is never the program's last one: select.rs `last_thrsw` adds the pair
after the `if`, as it does after a loop (Mesa's `last_thrsw_at_top_level`,
nir_to_vir.c:5007, false inside control flow). verify.rs refuses a
last-switch pair inside a loop body or between a forward branch and its
target.

**Registers across an arm** (alloc.rs `holes`). A merge register is written
at the end of the then arm, which then jumps over the else arm, and again by
the Copy at the end of the else arm. On the else path it holds nothing
until that Copy, so it is dead across the whole else arm; a linear scan
still counts it busy there. The allocator finds that hole (and the same
hole in a loop's carried value between its last read and its Copy) and
lends the register to a value that lives inside it, only when no register
is free otherwise, so a program that fits without lending keeps its
registers. The unroller's guards (`if (flag) {} else { rest }`) have two
merge registers per carried value; without the holes filter_image needed
2 threads, Mesa's 4.

**The shape.** prepare brackets the arms with `SOp::IfBranch { cond }`,
`SOp::IfElse` and `SOp::IfEnd`. A lane the arms leave different becomes an
`SOp::IfMerge { then, else }` after `IfEnd`, not a `Bcsel`. select.rs then
emits:
- the condition's flag;
- `bu.anyna` (or `anya`) with msfign P to the else label. "P" means lanes that
  were not dispatched, or are discarded, do not vote; this is Mesa's choice;
- the then arm, then one MOV per merge (the merge register's definition);
- `bu` (always) to the end label, then the else label;
- the else arm, then one Copy per merge (a rewrite, as a loop's carried
  value), then the end label.

sched.rs places each branch after its flag push and outside a THRSW's
delay slots. It gives the branch three NOP delay slots and patches its REL
offset and its uniform-stream offset, with the back edge's rule. A label
starts a word of its own. A THRSW merged back, and the thrend, never reach
across a label. verify.rs `branches_are_structured` accepts a forward branch
only when it is recorded, REL with the stream moved, strictly forward past
its slots, inside the program, never into a loop body, and out of one only
to that loop's exit (a break, "Control flow, step 3"). So every program
still ends: loops by their counters, the rest by moving forward.

**What must not cross an arm** (the rules that keep a value from being read
on a path that did not compute it):
- prepare's common-subexpression reuse treats each arm as a region, as it
  treats a loop body. A value from the then arm is not reused in the else
  arm, nor after the `if`.
- gl_FragCoord (x, y, w) and gl_FrontFacing are computed where they stand
  (FXCD, FYCD, RECIP, REVF), so they are reused under the same region rule.
  Inputs, uniforms and constants stay shared: select.rs reads them at the top
  or at each use.
- select.rs forgets, at the end of each arm, the uniform and constant loads it
  cached inside it. The flag reuse is dropped at every branch and label.
- prepare's dead-code removal keeps a branch only while an operation computed
  inside it, or one of its merges, is live. femtovg's stencil and
  filter_image call `scissorMask` and never read the result; the empty
  skeleton cost 6 to 10 instructions until this was done.

**Not done yet.** Hoisting the work
both arms repeat (`scissorMat * vec3(p, 1.0)`, which Mesa computes once
before its branch): it costs size, not executed instructions.

## Branch delay slots (step (g), implemented)

Status: implemented 2026-09-29 (v3d-m5/stepg), on the host; no board run yet.

Mesa's `emit_branch` (qpu_schedule.c:2445-2502): a branch moves back over up to
three of the words before it, which then run in its delay slots instead of
NOPs. They ran before the branch and still do, on both of its paths.
sched.rs `branch_back` does this for back edges and forward branches. A word
may move there when it has no branch, THRSW, uniform, signal, peripheral
access, magic write (but an accumulator), MULTOP or UMUL24, and, for a
conditional branch, no flag push (the branch reads the flags as it issues).
The branch never lands in a THRSW's or another branch's slots, on the word a
loop head or a label names, or within three words after an `ldvary`. Mesa's
own rule is wider (it allows TMU writes and LDTMU in the slots); this one is
narrower so that no other placement bookkeeping sees a word move.

Two changes let a counted loop's back edge use it:
- select.rs `SOp::LoopEnd`: the flag push reads the counter after its Copy
  (`t = c - 1; c <- t; pushz c`). The Copy is then t's last read, so the
  allocator writes t into c's register: one `sub c, c, 1`, and the Copy
  (`mov c, c`) is dropped. Before, `sub r0, c, 1; mov c, r0; mov.pushz r0`
  took three words at the end of every pass.
- sched.rs `hoist`: a flag push moves back like a plain operation when no
  word from its new place on pushes or reads the flags (Mesa orders the flags
  like a register, qpu_schedule.c:539-542). The words after it can then be
  the back edge's slots.

**What moved.** filter_image: 61 + 41 executed per pass (was 61 + 45; Mesa's
loop is 41). Program length: every femtovg program 3 or 4 words fewer (the
uniform branches' slots), fs total 6048 -> 5928 (ES2 4892 -> 4794), no
program longer. Tests: sched.rs
`a_back_edge_takes_the_work_after_its_flag_push_as_a_delay_slot`,
`a_branch_never_moves_over_its_flags_a_uniform_or_its_loops_head`,
`a_flag_push_is_not_hoisted_over_a_flag_read_or_push`; chain.rs
`a_uniform_break_leaves_filter_images_loop_by_a_branch`.

## Control flow, step 3: a uniform `break` as a branch (implemented)

Status: implemented 2026-09-29 (v3d-m5/stepc), on the host; no board run yet.
Evidence: `chain.rs a_uniform_break_leaves_filter_images_loop_by_a_branch`
(filter_image in the interpreter at 0, 2, 23 and 24 passes, the colours
against the host's blur), `prepare.rs a_kept_loops_uniform_break_guard_is_a_branch`,
the `verify.rs` tests (below), and the pins in `tests/v3d_sim_mesa.rs`.

**Why.** Before this, a kept loop's uniform break set its flag, the
unroller's guard `if (flag) {} else { rest }` was a uniform branch round the
rest of the pass (with its merges), and the back edge tested the counter OR
the flag. filter_image ran 71 instructions a pass (Mesa ~41) and 99 with no
pass. Mesa leaves at the break itself (nir_jump_break, ntq_emit_jump: a
branch to the loop's exit block).

**When** (prepare.rs `Walk::uniform_break`): all of these must hold:
- a fragment shader, in a counted loop the unroller gave a uniform exit flag
  (`Trip.exit`, not `divergent`) that starts clear, not nested in another
  counted loop, and not inside an arm of an `if` (a branch arm may be
  skipped; a select arm runs on every path);
- an `if` one arm of which is the unroller's break (constants and one store
  of `true` to the loop's flag, `Walk::break_arm`), the other empty, on a
  uniform condition (`is_uniform`);
- every lane the loop carries, the flag and the discard flag still hold
  their values from the start of the pass: then the phis are the values the
  loop leaves with on that path too, as when the back edge falls through.

Then prepare emits `SOp::LoopBreak { cond, when }` (`when`: whether the then
arm is the break) and leaves the flag clear, so the guards after it are the
code itself (`clear_at_pass_start`); a flag no store in the body set is clear
at the back edge, which then tests the counter alone. Anything else keeps
the flag, as before.

**The code.** select.rs gives a loop with a break an exit label after its
back edge; `LoopBreak` is the condition's flag and a `FwdBranch` to it
(AnyA or AnyNA, msfign P), the same REL branch with the uniform stream moved
as a uniform `if`'s. A load cached inside such a loop is forgotten after it,
as after an arm (a pass that breaks may not have made it). alloc.rs needs no
change: a forward branch to a label past a carried value's Copy already ends
that value's hole (`holes`, "skips"). No TMU request is open at the branch:
each look-up's `ldtmu` follows it directly (select.rs `SOp::Tex`).

**The guarantee** (verify.rs `branches_are_structured`): a forward branch
enters no loop body ("a forward branch into a loop body") and leaves one
only to the word after that loop's back edge's delay slots ("a forward
branch out of a loop body, not to its exit"); with loops nested, a break of
the inner loop aimed at the outer loop's exit is refused. Pinned directly
by `a_backward_target_not_made_by_a_loop_node_is_refused`,
`a_target_outside_the_enclosing_loop_is_refused` and
`a_target_past_the_programs_end_is_refused`; each fails when its rule is
switched off.

**What moved.** filter_image: 63 + 56 executed per pass (was 99 + 71); on
the pinned cluster path 110 -> 74 (nodiscard 99 -> 63); the program 160 ->
119 words. es100 deadloop 45 -> 36, loopcarry 46 -> 37, es300 dynidx 134 ->
112, uniloop 48 -> 41. No other corpus program moves.

The two look-ups of a pass each had a switch of their own; step (d) below
puts them under one.

## Look-ups under one thread switch (step (d), implemented)

Mesa queues a look-up's thread switch and `ldtmu`s instead of emitting them
with the request (`ntq_add_pending_tmu_flush`, `nir_to_vir.c:301-328`). The
queue is flushed, one THRSW and then every queued look-up's `ldtmu`s oldest
first, which is the order the TMU returns words in (`ntq_flush_tmu`,
`:262-293`), when:

- a queued result is read (`ntq_get_src`, `:856-868`);
- the block ends, or a jump or a discard comes (`:4595-4626`, `:3753-3775`);
- the next look-up's result words would overflow the TMU output FIFO, which
  cannot stall: 16 / threads words, at most 8 look-ups
  (`ntq_tmu_fifo_overflow`, `:251-258`; `MAX_TMU_QUEUE_SIZE`,
  `v3d_compiler.h:62`).

Before that, `nir_schedule` starts texture fetches early, as long as fewer
than 24 channels (4 threads) are live (`vir.c:2096-2118`,
`nir_schedule.c:1115-1120`); a shader that still does not fit turns
pipelining off in a late strategy (`vir.c:2197`).

Ours (select.rs):

- `Sel::flush_tmu` and the queue follow Mesa's triggers: a `TexLane` of a
  queued look-up, any loop or branch structure (`ends_stretch`), the end of
  the body (before the discard, the outputs and the last switch), and the
  FIFO bound. The bound is `TMU_OUTPUT_WORDS` = 16 / 4 = 4 words for every
  program: the thread count is chosen after selection (4, else 2), so the
  4-thread bound is the one that holds for both.
- `issue_order` is our `nir_schedule` for this one purpose. In each
  straight-line stretch it moves a later look-up, with the operations its
  coordinates need, up next to an earlier one, while the group's words fit
  the bound. A look-up whose coordinates read a result of the group stays
  where it is and gets its own switch. Only pure operations move, each
  after its operands. The register cost is bounded by the FIFO: at most 4
  result words wait in registers.
- `Pressure::Serial` (the chain's last level, after `Remat`) turns
  pipelining off, as Mesa's late strategy does.
- sched.rs: a `wrtmuc` goes in a word after the last sequence-ending TMU
  write (TMUS, TMUSCM, TMUSLOD, ...). Mesa's scheduler never merges the two
  (`qpu_schedule.c:249-250`, `:511-516`). With a switch between every two
  look-ups this never bound; without the rule the next P0 rode in the S
  write's word and the interpreter read it as the first look-up's P2.

**What moved.** filter_image: 63 + 54 executed per pass (was 63 + 56). The
pass's two look-ups have one THRSW, but after it the pass is bound by the
mul unit: 8 f16-unpack `fmov`s, 8 `fmul`s and the tail's 5 mul-unit
operations. Folding the unpack into the `fmul`'s input unpack is the next
lever there. Tests: chain.rs
`independent_look_ups_share_one_switch_in_request_order` (2 look-ups of 2
words, 3 of 1 word, 3 of 2 words past the FIFO; colours in the
interpreter) and `a_dependent_look_up_has_its_own_switch`.

## A look-up lane read through the reader's unpack (step (f), implemented)

A look-up's result word holds two f16 channels. Before step (f), each lane
was an FMOV with the L or H input unpack, and the reader read the FMOV's
register. Mesa makes the same FMOV, then folds it into every reader whose
operation unpacks f32 and which has no unpack of its own on that operand
(`vir_opt_copy_propagate.c:167-216`, the list in `qpu_instr.c:1153-1192`,
Mesa 26.2.1). Dead-code elimination then drops the FMOV if no reader is
left.

- Step (f) did this in select.rs `folded_unpacks`, before selection, for
  a `TexLane` whose every reader was an FADD, FSUB, FMIN, FMAX or FMUL.
  Step (h) replaced it; see below.
- The encoder already wrote the unpack on these operations (encode.rs
  `add_pack` / `mul_pack` then; Mesa's `v3d42_qpu_add_pack` /
  `v3d42_qpu_mul_pack` through FFI since 2026-10-01, docs/source-form.md F1). Its decode-back reference is Mesa's own word
  `0x5de24398bbdc6218`, `fmul rf14, -8, rf8.h` (golden.rs).

**What moved.** filter_image: 61 + 45 executed per pass (was 63 + 54).
Program length: filter_image 117 -> 106, the femtovg glyph programs 1 to 5
words fewer. Four fill_image_gradient programs are 1 word longer: a
register choice in the gradient arm puts a NOP in an SFU gap. None of
their executed counts moves. Test: chain.rs
`a_lane_read_only_through_an_unpack_has_no_fmov`. With the FMOVs gone, the
first LDTMU's result is often read only after the second LDTMU lands, so
the LDTMU-destination mutant applies to many more pairs (ES 3: 3 -> 33
caught). The control texture's red channel now names the row too
(tests/v3d_sim_mesa.rs `texture`), so qt/outlinedtext's two look-ups one
row apart differ in `.x` and that mutant is caught.

## Every reader the encoder's table allows (step (h), implemented)

Step (h) moves the fold to where Mesa does it: after selection, on the
selected operations. select.rs `fold_unpacks` (it was `fold_abs`, which did
the same for fabs's ABS move) takes `fmov d, s.u` (u = ABS, L or H) and
rewrites each read of `d` to a read of `s.u` where that input has the
field. The table is `takes_unpack`, read from encode.rs `add_pack` /
`mul_pack` (decode.rs reads the same fields); since F1 those are Mesa's
`v3d42_qpu_add_pack` / `v3d42_qpu_mul_pack`, called through FFI, and the
table in select.rs is still a copy of what they accept:

| Operation | ABS | L, H |
|---|---|---|
| FADD, FADDNF, FSUB, FMIN, FMAX, FCMP, FMUL (both inputs) | yes | yes |
| FMOV (its one input; also a flag-pushing FMOV) | yes | yes |
| VFPACK (both inputs) | no | yes |
| FFLOOR, FROUND, FTRUNC, FCEIL, FDX, FDY (input a) | no | yes |
| FTOIN, FTOIZ, FTOUZ, FTOC | no | no (Mesa: yes; our encoder refuses L/H there as unmeasured on silicon) |
| anything else (integer ops, MOV, CondMov, the SFU, TMU and VPM writes) | no | no |

Mesa's list is `v3d_qpu_unpacks_f32` (`qpu_instr.c:1153-1192`) and its
ABS exceptions `vir_opt_copy_propagate.c:196-212` (Mesa 26.2.1, MIT). As
in Mesa, an input with an unpack of its own is never rewritten ("No
composing the unpacks"), so `abs(lane)` keeps the lane's FMOV.

One difference from Mesa: an L or H move goes only when **every** read of
it takes the unpack; otherwise no reader is rewritten. Folding into some
readers while the move stays saves no instruction. Here it kept the
look-up's words live longer, and femtovg's fill_image and
texture_copy_unclipped grew 1 to 4 words (their texType selects then kept
real MOVs; measured 2026-09-29). An ABS move still folds into each reader
that takes it, as before.

The count of reads left now counts a read through an unpack too. Before,
`abs(abs(x))` dropped the inner move while the outer one still read it,
and the program was refused (`Alloc(UseBeforeDef)`, measured on 8472118d).

**What moved.** Programs that write a look-up straight to the colour:
gl_test_es100 texture and compact 13 -> 11 words, es300 cube 27 -> 25,
qt3d skyboxcube_300 22 -> 20, skybox_300 29 -> 28, cubeMapToHemisphere_300
31 -> 30. No femtovg program moves: each of their remaining lane FMOVs
feeds a select. The LDTMU-destination mutant applies to seven more pairs
(ES 3: 33 -> 40 caught). Tests: chain.rs
`a_lane_read_only_through_an_unpack_has_no_fmov` (VFPACK, FCMP),
`a_lane_read_by_ffloor_fceil_or_a_flag_fmov_has_no_fmov`,
`a_lane_another_reader_cannot_unpack_keeps_its_fmov`,
`abs_of_abs_keeps_the_inner_move`; select.rs
`fold_unpacks_moves_l_and_h_into_readers_that_take_them`.

## A register the hoist can use (step (l), implemented)

The scheduler keeps program order and moves one operation back into an
earlier word when it can (sched.rs `hoist`). It moves a value back to
word p only when no word from p on reads or writes the value's register.
So the register the allocator gives a value decides how far that value
can move. Before step (l) the allocator did not look at this: lowest-first
for most values, round-robin for loads. Two losses came from it:

- (i) fill_image_gradient's late FMIN uniform load took rf6 round-robin,
  which the RSQRT -> RECIP chain had just read; the load could not move
  above that read.
- (k) giving a MOV its source's register (the dropped `mov rX, rX`) kept
  low accumulators busy longer, so idiv's short `big` chain got r0, which
  the division reads inside the hoist window: 43 -> 31 operations hoisted,
  +4 words.

The rule (alloc.rs, "Hoist-aware choice"): for a value hoist may move
(sched.rs `hoistable`, now shared by both), the allocator knows

- `earliest`: the barrier (last thread switch, branch, loop head or label)
  and one after each operand's own earliest point. The chain matters: a
  uniform load can move to the barrier, so a RECIP of it can go right after
  that, not only after the load's place in program order;
- per register, the last operation before this one that read or wrote it.

Its stop in a register is the later of `earliest` and that last touch. The
free register with the earliest stop is taken; the old order (lowest-first,
round-robin for loads) breaks ties. A touch that is the definition of this
operation's own operand never stops it (the value cannot go above its
operand anyway), so the operand's register (an in-place update) is as good
as an untouched one. A value hoist cannot move keeps the old order. The
class (accumulator or register file) is not changed.

Mesa has no such rule. Its list scheduler orders by the dependency graph,
and its allocator spreads reuse round-robin "to give post-RA instruction
selection more options" (vir_register_allocate.c:936-947, :990-1003, Mesa
26.2.1). It keeps accumulators for short values (:881-917), as we do.

**What moved** (2026-09-29, against step (g) 5fff6d69, both builds): fs
5928 -> 5723 (ES 2: 4794 -> 4632), vs 3522 -> 3287 (2637 -> 2458), cs
2804 -> 2623 (2204 -> 2055); femtovg executed 5486 -> 5311. No pair and no
executed run is longer. idiv 210 -> 201 (with (k); 214 with (k) alone);
fill_image_gradient 113 -> 108; filter_image 61 + 41 -> 59 + 41 a pass (the
pass is unchanged, = Mesa's). Found on the way, each was a rise until fixed:
treating an op that reads r5 as movable (hoist refuses it: the refusal now
lives in `hoistable`), counting a register's touch as its last holder's
last read instead of the last operation (55 programs or runs longer, ES 2:
46),
and taking `earliest` from the operands' places in program order instead of
their earliest points (main.vert's three `recip rf7` stayed serial). Tests:
alloc.rs `a_late_load_takes_a_register_nothing_reads_inside_its_hoist_window`
(a word fewer than the round-robin register),
`a_move_that_ends_its_source_takes_the_source_register` ((k)).

## filter_image against Mesa, word by word (step (m), implemented)

femtovg filter_image (the blur) at 435e6a94 against Mesa's fixture
program: both passes were 41 words; our prologue was 25 against Mesa's
21, so 0 passes ran 59 against 57. Three changes, each measured per pair
in both builds with no program longer:

- alloc.rs: a plain MOV that ends its source, into a value that must be in
  the register file (a loop's carried value lives across the thread
  switch), puts its source in the register file too, so the MOV takes the
  source's register and is dropped. filter_image's prologue lost five
  `mov rfN, rX` (the loop-carried sums set from the centre tap). Mesa does
  this before allocation: out-of-SSA coalesces a copy with its source
  (nir_from_ssa.c :385-440). Not for a uniform load: as `ldunifrf` it
  takes the condition field (es300 idxtex was one word longer with it).
- select.rs: a deferred load (uniform, texture-size word, constant) that a
  counted loop reads and that is defined before the loop is loaded before
  the loop's head, not on every pass. This is Mesa's nir_opt_move ("just
  before their first use inside the same basic block", nir_opt_move.c:30-31).
  Loads defined in the body stay there: hoisting them held es300 dynidx's
  array across the loop (109 -> 120 words).
- order.rs: before a conditional branch, the flag push it reads ranks as a
  path of 1 + 3 (the branch's delay slots), so the push goes before work
  the slots can take. Without it es100 loopcarry's body chain outranked the
  counter once its uniform left the body, and the back edge's slots held
  NOPs.

Result: filter_image 53 + 40 a pass (Mesa 57 + 41); fs 5723 -> 5653 (ES 2:
4632 -> 4599); femtovg executed 5311 -> 5271. What is left in the pass: the
loop-test constant's `ldunif` (defined inside the loop, so it stays), and
two branches with NOP slots where Mesa ORs its test and break into one
flag. Tests: alloc.rs `a_move_into_the_register_file_puts_its_source_there`,
chain.rs `a_uniform_read_in_a_loop_is_loaded_before_its_head`, order.rs
`a_back_edges_flag_push_goes_before_work_its_slots_can_take`.

## One exit branch per loop pass (step (p), implemented)

A counted loop with a uniform `break` had two exits in a row: the loop's
own test, then the break, each a compare, a branch and three delay slots
(10 words). Mesa merges two such terminators into one if on the OR of their
conditions (nir_opt_loop.c `opt_loop_merge_terminators` :649-712,
`merge_terminators` :554-604) and builds the OR as booleans: 8 words
before its branch in filter_image, 12 with the slots. Ours (select.rs
`merge_exits`) does it on the flags: the second compare updates flag A
instead of pushing (ANDxx / NORxx, which Mesa uses for divergent control
flow, nir_to_vir.c:3783-3785; modelled in sim.rs `apply_flags`), so A is
"no exit" and one `bu.anynap` leaves: `fcmp.pushc; fcmp.andnc; bu.anynap`,
6 words. As in Mesa only operations without an effect outside the registers
may sit between the two exits (they run on the path that leaves too).

The loop's bound is not in these branches: it is the counted back edge,
which counts the constant bound, or UNIFORM_LOOP_CAP for a bound read at run
time (even `while (true)`), and is never merged. Labels are resolved by the
scheduler after this, so removing a branch moves no other target.

**What moved** (both builds, nothing else): femtovg filter_image and its
nodiscard form, es100 deadloop and loopcarry, 4 words each (an instrument,
v3d_sim_mesa `back_to_back_loop_exits_are_counted`, found these four and now
counts 0); filter_image executed 49 + 36 a pass (was 53 + 40; Mesa 57 + 41),
the cluster's filter_image 62 -> 58. Tests: select.rs
`merged_exits_leave_when_either_would` (every branch sense, pushed bit and
flag value), chain.rs `a_merged_exit_leaves_the_loop_bound_to_the_back_edge`.

## Textures: the P0/P1 uniform words

Status: agreed before lookups were selected (2026-09-24); 2D lookups
(`texture2D`, `texture2DProj`, bias) are selected now. Every emitted program is
checked to touch the TMU only through such a lookup (`chain.rs`
`first_tmu_memory_access`: TMUS/TMUT/TMUB writes and one `wrtmuc` per P0/P1 word).

Mesa 26.2.1's convention, which ours follows so that the driver serves both
compilers:

- **Contents codes:** `QUNIFORM_TMU_CONFIG_P0` = 40 and `QUNIFORM_TMU_CONFIG_P1` =
  41 (their positions in `enum quniform_contents`, `v3d_compiler.h`). The word is
  written through `wrtmuc`.
- **The compiler's word:** `data = index << 24 | packed` (`v3d_unit_data_create`,
  `v3d_compiler.h:371-375`; the P0 word at `v3d_tex.c:298-300`, the P1 word at
  `:343-345`). The low 24 bits are the compiler's own packed P0/P1 fields (for
  example the return-word count); bits 24..31 name the texture (P0) or the sampler
  (P1).
- **What the driver fills in at draw time** (`v3d_uniforms.c`): for P0, the
  texture-state record's address OR'd with the low 24 bits (`write_tmu_p0`,
  :151-156). With no texture bound, it writes the low 4 bits only (:147-148). For
  P1, the sampler-state record's address OR'd with the low 24 bits
  (`write_tmu_p1`, :208-215).
- A P1 that needs no API sampler state is a plain `QUNIFORM_CONSTANT`
  (`v3d_tex.c:350`). Ours never emits that case: Mesa takes it only for a lookup
  that needs no sampler (`nir_tex_instr_need_sampler`, `:337`), and a regular
  `texture2D` / `texture2DProj` always needs one. So ours always writes contents 41
  with `binding << 24 | 4`, the per-pixel mask enable, as Mesa's fragment lookups
  carry it (its es100/textured has 41:0x4).

**Ours:** bits 24..31 hold the sampler's **reflection binding**. That is its index
among the stage's samplers, in declaration order, 0..15 (`reflect.rs`: "Samplers take
bindings in declaration order"; an array element k is `binding + k`). It is not a GL
texture unit, because the unit is not known until draw time: `glUniform1i` on the
sampler stores it per binding (`gl/program.rs`: `fs_sampler_unit[binding]`,
`vs_sampler_unit[binding]`). The CS reads the VS's samplers, as it reads the VS's
uniform storage. At draw time the driver's `write_uniforms` resolves binding → GL
texture unit → the bound texture's state record (P0) or sampler state (P1), and ORs
in the low 24 bits, as Mesa's driver does. A binding with no texture bound is Mesa's
case at :147-148.

The harness (`tests/v3d_sim_mesa.rs`) maps ours as it maps Mesa's: bits 24..31 name
the binding, and that binding's texture feeds the lookup.

## gl_FragCoord: Mesa's four reads, and one window-transform word

Mesa (`emit_fragcoord_input`, `nir_to_vir.c:1017-1024`) reads gl_FragCoord as
x = `FXCD`, y = `FYCD`, z = the payload's Z (rf2), w = `RECIP` of the payload's W
(rf0). `FXCD`/`FYCD` are already the pixel centre: their fraction is the sample
offset (`:3161`). Ours emits the same four.

What Mesa leaves to gallium is the y flip. The V3D's origin is upper left and an
ES shader's is lower left, so st/mesa's `nir_lower_wpos_ytransform` makes
y' = y * T[0] + T[1] from the state uniform `FB_WPOS_Y_TRANSFORM`
(`nir_lower_wpos_ytransform.c:102-110`). The V3D compiler never sees that uniform,
so ours asks the driver for the two numbers directly:

- **Contents codes** (the lead's choice (A), 2026-09-24): `QUNIFORM_FB_WPOS_Y_SCALE`
  = 0x10000 and `QUNIFORM_FB_WPOS_Y_OFFSET` = 0x10001 (`select.rs`; the driver's
  `V3D_QUNIFORM_FB_WPOS_Y_*` in `v3d_frame.h`, `v3d_job.h`, `ffi.rs`, whose
  description is v3d-glue's M6 section). They are above Mesa's
  `enum quniform_contents`, so they can never mean a Mesa value. One code per
  quantity, as Mesa does with `VIEWPORT_X/Y_SCALE`.
- **`data` is 0 in both**; any other value is refused by the driver and by the
  harness. y' = `FYCD` * scale + offset (an FMUL then an FADD).
- **The driver writes** scale -1.0 and offset the target's height for a window,
  +1.0 and 0.0 for a framebuffer object (the driver side is v3d-glue's).
- `uses_center_w` is set when any instruction reads rf0 (Mesa's
  `vir_check_payload_w`, `:5054-5069`), so a shader with no varyings that reads
  gl_FragCoord.w still sets it.

**How it is checked:** the harness drives both compilers with one non-identity
transform (scale -1.0, offset 64.0; `tests/v3d_sim_mesa.rs` `WPOS_SCALE`,
`WPOS_OFFSET`): Mesa's `FB_WPOS_Y_TRANSFORM` and our two words get the same
numbers, and the IR's gl_FragCoord.y is 64 - y. With the identity, a program that
dropped the transform or swapped its two codes would agree. Three hand mutants of
`select.rs` (on e2e2ac36's fixture): the two codes swapped, 8 programs disagree;
the transform dropped, 7 (all but fragcentre); x replaced by Z, 8. They are es100/fragcoord,
es100/fragcentre and six femtovg gradients (fill_gradient,
fill_gradient_conic.nodiscard, fill_image_gradient and its nodiscard,
fill_image_gradient_conic.glyph and .nodiscard). The other ten compiled femtovg
gradient variants agree but are **not shown** to exercise gl_FragCoord, since no
mutant moved them. The cause is established for two: under the harness's inputs,
fill_gradient_conic discards 15 of 16 lanes and fill_image_gradient_conic 10 of
16. For the other eight it is not established. The femtovg variants that do not
select a gradient read gl_FragCoord only in dead code. The board check is
v3d_triangle's -DTRI_FRAGCOORD variant, where the flip is visible in the picture,
then femtovg's gradients.

## gl_PointCoord: the same flip, two more words

Decided with v3d-m5 and the lead (2026-09-24): points only. The FS reads the
two implicit point varyings and flips y as Mesa's GL does:
y = y_raw * scale + offset (`nir_lower_pntc_ytransform.c:76-81`). The words
are our own contents codes `V3D_QUNIFORM_FB_PNTC_Y_SCALE` 0x10002 and
`_OFFSET` 0x10003 (`v3d_frame.h`, `ffi.rs`; `data` 0). The job builder writes
them from the job's `struct v3d_wpos`: (-1.0, 1.0) for a window, (1.0, 0.0)
for an FBO. These are Mesa's `STATE_FB_PNTC_Y_TRANSFORM`
(`prog_statevars.c:754-763`), with the sprite origin always upper left
because ES has no `GL_POINT_SPRITE_COORD_ORIGIN`. The V3D has no sprite-origin
bit (`v3d_packet.xml`), so the two words are the whole flip. job_check part 6
pins both targets and refuses data 1 and a zeroed transform. A program
reading gl_PointCoord carries the compiler's `uses_implicit_point_line_varyings`
(v3d-m5's `FsProg` flag). Its record keeps the implicit varyings, and any
primitive other than points is refused: by name in the backend ("a program
reading gl_PointCoord drawn as GL_TRIANGLES", one string per primitive,
counted in `v3d_refused_draws`) and as REFUSED in the job builder
(job_check part 11). Another primitive would read every declared varying two
slots off. Unmeasured on the board: the compiler does not set the flag yet.

## gl_VertexID: a VPM row ahead of the attributes

v3d-m5, 2026-09-26. The vertex fetch writes the vertex ID into the VPM input
segment when the shader record's "reads vertex ID" bit is set. It goes ahead
of the attributes. Mesa 26.2.1 reads it once at the top (`ntq_setup_vs_inputs`,
`nir_to_vir.c:2402-2421`: instance ID, base instance, vertex ID, each only if
read) and moves every attribute index past those rows (`:2890-2922`). Ours:

- prepare: a vertex-stage load of `IoSlot::VertexId` is `SOp::VertexId`, one
  int lane, merged like a uniform (one read per program).
- select: the input rows are the ID (row 0, if read), then each attribute
  record's rows. `Selected::uses_vid` carries the bit to `VsProg::uses_vid`,
  and from there through `shc_v3d_vs` to `v3d_job.c`, which sets
  `vs/cs_reads_vertex_id`. The coordinate shader sets it only when its position
  depends on the ID. In es300/vid.vert only the colour does, so the
  coordinate shader reads `pos` alone.
- GFXH-1602 (`:2907-2915`): if a coordinate shader reads a builtin and none
  of attribute 0, the driver still loads one row of attribute 0
  (`v3d_job.c:486-501`), so the later records start one row further.
- chain `vpm_fields`: the shared segment counts the ID row and that driver
  row. Mesa leaves the driver row out. Counting it can only make the segment
  larger.
- gl_InstanceID (added the same day, second commit) is `SOp::InstanceId`,
  row 0 ahead of the vertex ID (which moves to row 1), as in Mesa. The driver
  draws one instance (`SET_INSTANCEID 0`, no base vertex), so it reads 0.
  The ID of an array draw is `first + i`, and that of an indexed draw is the
  index. A draw of more than one instance is refused by name and counted
  (`state::refusal`, "instanced draws are not implemented"; test
  `a_draw_of_more_than_one_instance_is_refused_and_one_is_drawn`), so gl_test
  47 stops there on the Pi 4. Real instancing (Mesa `v3dx_draw.c`: the
  instanced primitive packets, SET_INSTANCEID per instance) is a separate item.
- The interpreter harness (`tests/v3d_sim_mesa.rs`) came with three fixes:
  - uint uniforms are drawn 1..4;
  - the evaluator judges an undefined division or shift only on the lanes
    the result defines (`eval.rs`);
  - a vertex shader that indexes an array by gl_VertexID gets IDs inside it.
  Each fix made pairs measurable that had not been.

Checked on the host: `chain::tests::gl_vertex_id_is_vpm_row_0_ahead_of_the_attributes`
(IDs 0..15, colour `gl_VertexID % 4` per lane, an attribute read after the ID
comes from its own rows) and `a_coordinate_shader_reading_the_id_skips_the_gfxh_1602_row`
(the dummy row holds a poison value that no output shows; with the row left out
of the index, the test fails). `tests/v3d_sim_mesa.rs` feeds the ID row (and
now the GFXH-1602 row) and runs the four es300/vid.vert pairs: they agree with
the IR and with Mesa's own programs. On the board: gl_test 43 (array draw),
57 (indexed, reversed) and 98. Instance ID, host only: the chain tests
`gl_instance_id_is_vpm_row_0_ahead_of_the_vertex_id` and
`a_coordinate_shader_reading_both_ids_skips_the_gfxh_1602_row`. The nine pairs
that read it (es300/inst and eight Qt Quick 3D particle pairs) agree with the
IR, and every mutant of them is caught.

## Flat varyings

v3d-m5, 2026-09-26. A flat input is read as Mesa reads it (`emit_flat_varying`,
`nir_to_vir.c`): the `ldvary` is issued in its place in the input order, its own
result is not read, and the value is r5, the C coefficient. The chain sets bit i
of `flat_shade_flags` for each flat input i (in `ldvary` order, not counting the
implicit point varyings, as `c->num_inputs` does not), and the job builder already
emits `FLAT_SHADE_FLAGS` from it (`v3d_job.c`), which makes the hardware put the
provoking vertex's value in C. More than 24 inputs are refused by the job builder
as before. Checked: `chain::tests::a_flat_varying_is_the_c_coefficient_and_sets_its_flag`
(flags 0b1100 for a smooth vec2 then a flat vec2; the flat inputs carry noise in
the `ldvary` result, so a wrong read shows), and 9 more corpus pairs in
`tests/v3d_sim_mesa.rs`, es300/flatlast agreeing with Mesa's own program too. Board
case: gl_test 89 flatlast (the provoking vertex is the last one, ES 3.0 2.18).

The interpreter harness also classifies an output that depends on sin/cos past the
comparison's tolerance as unmeasured (`TRIG_UNDETERMINED`): GLSL ES 3.00 section
4.5.1 leaves the precision of the trigonometric functions undefined, and Qt Quick
3D's `fract(sin(dot(co, k)) * 43758.5)` hash turns the f32 argument reduction's
error (Mesa's sequence, ours too; 1e-7..5e-6 for arguments up to 91) into a
different texel. The check runs only at a disagreement, and it re-runs the IR with
every sin/cos result moved by 2^-20 * max(1, |x|). A disagreement that does not move
stays a disagreement.

## Derivatives

v3d-m5, 2026-09-26. `dFdx(p)` is `fdx p` and `dFdy(p)` is `fdy (p * y_scale)`, as
Mesa emits them (`nir_to_vir.c:4052-4065`, FDX / FDY of the value) after st/mesa's
`nir_lower_wpos_ytransform` has rewritten `ddy(p)` as `ddy(p * transform.x)`
(`lower_ddy`, `nir_lower_wpos_ytransform.c:209-225`, Mesa b739817b). `y_scale` is
the word `QUNIFORM_FB_WPOS_Y_SCALE` that gl_FragCoord.y also reads: -1 for a window
(stored top-down), +1 for an FBO. The interpreter stops at FDX / FDY as unmodelled:
no public source states which lanes form a quad on the V3D, and Mesa's v3d has no
quad operation that would show it. So the values are measured on the board only,
by gl_test 138 deriv (window and FBO, 0xffbfbfbf expected on both). The chain test
pins the sequence through select's output (FDY's operand is an FMUL of the
y-scale word).

## Multiple render targets

**Built (steps 1-3, 2026-09-26; the board run, step 4, pending):** the compiler
(838b59e9), the job builder (7952287f) and the backend (the step-3 commit). The
backend takes up to four colour attachments. Each one past the first must be RGBA8
at RT 0's size; anything else is refused by name. The job's RT count is the
draw's `num_targets`. A program that writes a location with no attachment slot is
refused ("a colour output with no attachment slot") and counted. So is a colour
clear the quad would do on several attachments, because the quad writes RT 0 only.
The tile pools are sized for four RTs (32x32 tiles, about 1.1 MB more than before).
GL_MAX_DRAW_BUFFERS answers 3 on the V3D (every colour attachment, as on the
PowerVR) since the step-4 commit. It answered 1 before, and the first board run
(records/pi4/record_VIDMRT_gt_VIDMRT_227e5dec.txt) skipped gl_test 106 for it: 55
PASS, 106 "SKIP (GL_MAX_DRAW_BUFFERS 1 < 3)".
**Not built:** GL's real semantics for an output with no draw buffer (the write is
dropped). That needs a program variant keyed by the framebuffer's RT count, as
Mesa keys `fs_key.cbufs`. Two findings go to the lead:
- The GL core stops listing targets at the first GL_NONE draw buffer
  (`resolve_targets` breaks at the first `fb_target_n` that is None). So (ATT0,
  NONE, ATT2) reaches every backend as one target.
- gl_test 106 part 3 cannot see attachment 2, or attachment 0: the colours it
  checks are the ones part 2 left. On the V3D, part 3 is then a named refusal.

### The design (as noted before building)

v3d-m5, 2026-09-26. `gl_FragData[n > 0]` (and an ES 3.00 output at location
n > 0) is refused today: "gl_FragData[n > 0]", 18 corpus pairs after flat varyings
(qt3d/clear_mrt_300, lightmapuvraster_300 and 16 particle shaders). The backend also
refuses `num_targets > 1` ("more than one colour attachment is not written yet").
Board cases that exist: gl_test 55 es3mrt (two attachments) and 106 mrt3 (three,
then `glDrawBuffers(ATT0, NONE, ATT2)`). Sources: Mesa b739817b
`src/gallium/drivers/v3d/v3dx_rcl.c`, `src/broadcom/common/v3d_util.c`, and
`nir_to_vir.c` (26.2.1).

What Mesa does on V3D 4.2:

- **Shader** (`vir_emit_tlb_color_write`, `nir_to_vir.c:1953-2037`): one TLB write
  per render target `rt` the key's `cbufs` mask holds. The first word goes through
  TLBU with a config uniform `0xffffff00 | per-pixel (1 << 2) | (7 - rt) << 3 |
  F16 (3 << 6) | swap (1 << 1) | vec4 (1 << 0)` ("Reversed! 7 = RT 0",
  `:1932`). For RT 0 with an f16 vec4 that is `~0`, the default, which is why
  our RT 0 write needs no config today. f32 targets (`f32_color_rb`) write each
  channel with `TLB_TYPE_F32_COLOR`; not in scope here.
- **RCL** (`v3dx_rcl.c:761-913`): `number_of_render_targets` and the maximum
  internal bpp of all RTs in the common config; `TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1`
  (and PART2/3 at 64/128 bpp) per RT with its `render_target_number`; one
  `TILE_RENDERING_MODE_CFG_COLOR` that holds bpp, type and clamp for RT 0..3.
- **Tile list** (`v3d_rcl_emit_loads` / `_stores`, `:188-290`): a load and a
  store per RT, buffer `RENDER_TARGET_0 + i`.
- **Tile size** (`v3d_choose_tile_size`, `v3d_util.c:122-200`, the 4.x branch):
  idx += 1 for 2 RTs, += 2 for 3-4, += 3 above 4, plus the max bpp. So 2 RGBA8 RTs
  bin at 64x32 and 3-4 at 32x32.

The constraint that decides our design: the tile size is fixed when the job
starts (`v3d_frame.c` computes it, and binning uses it), before any draw is
known. So the RT count must come from the **framebuffer** (its colour attachments),
as Mesa's `nr_cbufs` does, not from the programs.

Proposed steps, each with its evidence:

1. **Compiler**: a TLB write per written output location n <= 3, in RT order,
   the first word of RT n > 0 through TLBU with Mesa's config uniform (RT 0
   stays as today, byte for byte). `FsProg` gets `rt_mask` (the RTs the program
   writes). Host: a chain test with two outputs checks the config words and the
   interpreter's TLB (sim.rs must decode the RT index from the config; today
   `tests/v3d_sim_mesa.rs` calls any specifier but 0xff unmodelled).
2. **Job builder**: `v3d_job_target` carries up to 4 colour buffers (address,
   format, stride; 0 for a `GL_NONE` draw buffer). Job begin takes the RT count
   (highest attached index + 1) and chooses the tile size by the 4.x rule. The
   RCL emits the per-RT clear colours and the COLOR packet, and the tile list
   loads and stores each RT **that has a buffer**. An RT with none (NONE) is
   configured, so the program's write has TLB space, but never loaded or
   stored, so it is dropped as GL requires (gl_test 106 part 3). `job_check`
   and `cl_golden` get the Mesa packets for 2 and 3 RTs.
3. **Backend**: `num_targets > 1` passes when every RT the program writes is
   below the job's RT count. A draw whose program writes an RT the framebuffer
   has no slot for is refused by name, "a colour output with no attachment
   slot", and counted: GL discards such a write, but the TLB would need the RT
   configured; a later step could add it. A job's RT count changes only with
   the framebuffer, which already starts a new job.
4. **Board**: gl_test 55 and 106 plus the default 12, one image.

Open for the lead: the refusal in step 3 versus configuring the extra RTs at job
begin when the default framebuffer (one attachment) is drawn with a program
that writes more. That costs every such job the smaller tile, and nothing in the
corpus needs it.

## Integer and ES 3.0 colour render targets (int-rt-v3d, 2026-10-14; host only)

**Built:** the nineteen integer colour formats of ES 3.0 table 3.13 (R/RG/RGBA
8/16/32 I and UI, RGB10_A2UI) and the float ones (RGBA16F, R16F, R32F) are
render targets on the V3D, alone or among other attachments, and an integer
fragment output (`out ivec4`, `uvec2`, `int`, ...) reaches the TLB as 32-bit
words. Nothing is measured on a board yet: the host checks are
`tests/es3_int_rt_v3d.rs` (the rows against Mesa's table, the clear words,
the compiler's TLB writes on the interpreter) and `src/v3d/backend_tests.rs`
(the typed job, its strides, clears and refusals, against the fake job
builder); the job builder's packets for a typed job have no `cl_golden` /
`job_check` row yet (see "Left").

What Mesa does on V3D 4.2, and what was transcribed:

- **Formats** (`v3dx_format_table.c`, the `rt` column of `FORMAT(...)`: the
  integer rows :118-137, R16F :90, R32F :91, RGBA16F :100, RGB10_A2UI :75):
  each format is its own OUTPUT_IMAGE_FORMAT for the colour load and store,
  raster, at its own bytes a pixel (`src/v3d/state.rs` `rt_format`).
- **Tile buffer** (`v3dX(get_internal_type_bpp_for_output_format)`,
  :220-341): the internal type is the channel width and kind (8I, 16UI,
  32F, ...), the bpp the channels' total width: 32 for the 8-bit rows and
  the one- and two-channel 16-bit ones, 64 for RGBA16*, RG32*, RGB10_A2UI
  (held as 16UI) and BOTH 16F rows (R16F at 64 "to make sure the TLB doesn't
  throw away the alpha channel before alpha test happens", :266-268), 128
  for RGBA32*. The clamp is `clamp_for_format_and_type`'s
  (`v3dx_rcl.c:396-407`): INT for a pure integer format, NONE otherwise
  (NORM for sRGB, whose render side is not built, `docs/es3/srgb.md` 5.1).
- **Tile size** (`v3d_util.c` `v3d_choose_tile_size`): one step down the
  table per bpp step on top of the RT-count and MSAA steps
  (`v3d_frame_tiling_bpp`; the table now runs to 8x8). The pools are sized
  at init for 2048x2048 at four 32-bpp RTs (4096 tiles); a typed job with
  more tiles is refused by name ("more tiles than the job pools hold").
- **Clear** (`v3d_tlb_clear`, `v3dx_draw.c:1706-1769`; the packets
  `v3dx_rcl.c:833-860`): the clear colour is packed per internal type (a byte
  a channel for 8I/8UI, a half for 16F, 16 bits for 16I/16UI, the words as
  they are at 32) into up to four words, CLEAR_COLORS PART1 for every RT,
  PART2 from 64 bpp, PART3 at 128 (`state::clear_words`,
  `v3d_job_clear.clear_rt`). glClearBuffer(GL_COLOR, n) (`clear_target_mask`)
  clears draw buffer n alone; the other RTs load.
- **Shader** (`vir_emit_tlb_color_write`, `nir_to_vir.c:1953-2036`, the
  `f32_color_rb` branch `:1974-1977` and `:2005-2021`, which the key takes
  for every pure integer format as for 32F, `v3d_program.c:701-704`): an
  output declared `int`/`uint` (any width) is MOVed to the TLB one 32-bit
  word per declared component, the first through TLBU with `0xffffff00 |
  per-pixel | (7 - rt) << 3 | TLB_TYPE_F32_COLOR (0 << 6) | (components -
  1)`, the rest through TLB; the hardware narrows to the target's type under
  the INT clamp (`prepare.rs` `rt_int` / `rt_comps`, `select.rs`
  `tlb_config_32b`). V3D 4.2 has one 32-bit TLB type for integer and float
  words alike: Mesa 26.2.1 defines `TLB_TYPE_I32_COLOR (1 << 6)`
  (`nir_to_vir.c:1930`) but never writes it, the tile buffer's internal type
  says what the words are. A float output keeps its f16 pairs.

**The job builder's interface** (`driver/src/v3d/v3d_job.h`): `v3d_job_target`
gained `typed` and `rt_format[4]` (RT 0..3's output image formats; with
`typed` false every ES 2.0 job is built byte for byte as before: RGBA8 or
BGR565 by `rgb565`), `v3d_job_clear` gained `clear_rt[4][4]`, and
`v3d_frame_tiling_bpp` takes the largest bpp. `rt_internal` in `v3d_job.c`
holds the same rows as `state::rt_format`, keyed by the output format; a
typed format outside them, or 4x with `typed`, breaks the job at begin.
`src/v3d/ffi.rs` and `ffi_layout_check.c` pin the new layouts (target 88
bytes, clear 132, job 856) and the enum values.

**Left:**

- A board run: gl_test 200 intrt (and 201 isamp once formats-v3d samples
  integer textures) and the dEQP-GLES3 `fragment_out`, `fbo.color` and
  `shaders.builtin_functions` buckets on the Pi 4.
- `tools/v3d/job_check.c` and `cl_golden.c` rows for a typed job (the
  PART2/PART3 clear packets, the COLOR packet with 16I/64 bpp, a 16x16
  tile), against Mesa's CLE.
- The clear quad's program writes halves: a scissored or masked colour
  clear of an integer target is refused by name. An integer clear program
  (`out ivec4 o = u_color`) would lift it.
- A float output to an R32F target writes f16 pairs (the program is not
  keyed by the target's class, so Mesa's F32 TLB write cannot be chosen);
  the 32F tile buffer takes them at half precision. EXT_color_buffer_float
  is not claimed, so no dEQP case reaches it.
- 4x on an ES 3.0 colour format is refused by name (the kept samples and
  the resolve are RGBA8). Nothing in the GL core makes such a surface.
- `GL_MAX_DRAW_BUFFERS` stays 3 (`backend::DRAW_TARGETS`); a fourth
  attachment is the MRT item's.

## Multisampling (4x; steps 1-3 of 4 built: the job builder, the backend, the EGL config behind a switch)

The target: dEQP-GLES2 functional.multisample, 16 cases "NotSupported (No
multisample buffers)" on the Pi 4, run with the rgba8888d24s8ms4 config. The
plan (user decision 2026-09-26, after the fbo work), each step measured on
the board before the next: (1) the job builder, (2) the backend: 4x buffers,
the sample mask (glSampleCoverage reaches no backend today: GlesDraw needs a
sample mask, made as Mesa's state tracker does, st_atom_msaa.c), gl_test 145
(sample coverage, read back on both sides of the mask), (3) the 4x EGL config
behind a switch, (4) alpha-to-coverage (Mesa: SETMSF(MSF AND FTOC(alpha)),
nir_to_vir.c:2055-2064), from one uniform word rather than a program variant.

Step 1, the job builder (driver/src/v3d/v3d_job.c), following Mesa 26.2.1's
v3d:
- `v3d_job_target.msaa`: Multisample Mode (4x) in TILE_BINNING_MODE_CFG and
  the rendering mode's COMMON (v3dx_draw.c:92, v3dx_rcl.c:763-764); the tile
  size two steps down (v3d_util.c 4.x: `if (msaa) idx += 2`, 32x32 for one
  RT); CFG_BITS rasterizer oversample 1 (v3dx_emit.c:326-334); SAMPLE_STATE
  mask = the draw's `sample_mask` (0xf in a single-sampled job,
  v3dx_emit.c:728-736).
- RT 0 is stored resolved into the target (store general, decimate 4X,
  v3dx_rcl.c:159-166) at the end of every job, so the window, glReadPixels
  and texture reads see the resolved image with no separate pass. Its
  samples are kept in `ms_address` (UIF RGBA8 at twice the width and height,
  Mesa's layout for a multisampled resource, v3d_resource.c:612-642), stored
  and loaded with decimate ALL_SAMPLES (v3dx_rcl.c:104-107, :157-158); Z/S
  buffers of a 4x job are loaded and stored with ALL_SAMPLES too.
- Refused: 4x with more than one RT (at begin), and a 4x job that loads RT 0
  with no kept samples (at end: a resolved image loaded back is not what a
  4x surface holds).
- Checked by tools/v3d/job_check.c part 14 through Mesa's CLE decoder (14
  checks; revert checks: without the rendering config's bit, and with the
  resolve stored as sample 0, the part fails).

Step 2, the backend (src/v3d/backend.rs, state.rs) and the GL core:
- GlesDraw.sample_mask and .alpha_to_coverage (draw.rs set_sample_state,
  Mesa's st_update_sample_state rule), 0xf and false on every
  single-sampled draw.
- A draw with samples 4 is a 4x job (`Target.msaa`, part of the job's
  identity). Its colour samples live in an `MsBuf` per target (two slots,
  LRU), its depth/stencil in a 4x `ZsBuf` (never found or lent across 1x
  and 4x). Both are laid out by `v3d_tex_layout_msaa` (driver/src/v3d/
  v3d_tex.c: one level at 2w x 2h, UIF at the top as Mesa's uif_top; checked
  by tools/v3d/tex_check.c against a hand derivation from v3d_setup_slices).
- The samples are valid after a job stored them, and undefined after the
  surface's eglSwapBuffers (EGL 1.5 3.10.1, as for depth/stencil): a job
  that would load undefined samples starts from the clear colour instead.
- Refused by name: a sample count other than 1 and 4, alpha-to-coverage
  (step 4), 4x with several colour attachments.
- Nothing asks for a 4x surface until step 3 offers the EGL config.

Step 3: the V3D build offers the 4x EGL config while `v3d_debug_msaa` is on
(src/v3d/backend.rs v3d_debug_set_msaa, egl.rs offered_configs; off by
default, as the GC7000's gc_debug_msaa). The Pi 4 image turns it on with
CONFIG_V3D_GL_TEST_MSAA. gl_test 16 (msaa: a 4x pbuffer, an edge half
covered) and 145 (samplecov: glSampleCoverage, each half of the mask read
back) are the board's first measurement; then dEQP-GLES2 multisample with
the ms4 config. Prediction: 12 of the 16 dEQP cases pass (the 4 with
alpha-to-coverage wait for step 4).

## textureSize

The compiler side is built (2026-09-26); the job builder fills the words
(2026-10-10, for the ES 2.0 non-seamless cube look-up's level of detail, which
reads the face size the same way: docs/v3d-noseamless-cube.md). Each entry of
the job builder's sampler table (`struct v3d_job_tmu`) carries the binding's
base-level width and height, filled by the backend (`texture_units`: the
binding's size, 1 x 1 for the black stand-in), and v3d_job.c writes
`V3D_QUNIFORM_TEXTURE_WIDTH` / `_HEIGHT` from the entry the word's data names;
a binding the draw did not give, or one without its size, is a bad field
(tools/v3d/job_check.c part 5).

Mesa's `ntq_emit_txs` (nir_to_vir.c:915-956): each lane of the result is a
uniform word, QUNIFORM_TEXTURE_WIDTH (44) or _HEIGHT (45), whose data is the
texture unit. The driver fills it with the base level's size (`get_texture_size`,
v3d_uniforms.c:50-90: `u_minify(width0, first_level)`, one face for a cube). The
shader minifies it, `ntq_minify`: `max(size >> lod, 1)`, a logical shift. The
values 44 and 45 are Mesa's, printed by a program compiled against Mesa 26.2.1's
v3d_compiler.h (DEPTH 46, ARRAY_SIZE 47, LEVELS 48 follow).

Ours (prepare.rs `Op::TexSize`, select.rs `SOp::TexSizeWord`): the same words,
with data = the sampler's reflection binding (the number P0/P1 carry in bits
24..31), and the same `Ushr` and `Imax`. Only 2D and cube samplers; 3D and array
samplers are refused by name ("textureSize of a 3D or array sampler", 2 corpus
pairs). Evidence: `chain.rs texture_size_is_the_base_size_minified` (a 64x16
texture at levels 0, 1, 5 in the interpreter; it fails without the clamp), and
qt3d/lightmapdilate_300 now compiles and agrees with the IR and with Mesa in
tests/v3d_sim_mesa.rs.

## Open reports to the front end

Found while writing this back end, in shared front-end code. They are not fixed
here: a change in `builtin_call.rs` or `lower.rs` changes what the PowerVR and
GC7000 back ends compute, so it needs their board regressions.

**1. A built-in's width is unrecorded (a gap, not a wrong value).** A built-in
emitted through `builtin_call.rs` (`Componentwise` `abs` at :164, `max` at :177)
leaves its entry in `Module::comps` at 0, "unrecorded", so `comps_of` answers its
fallback 4 for `abs()` or `max()` of a `vec2`. A plain `u + u` records 2.

- Repro:
  `cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --lib -- a_builtin_on_a_vec2_records_two_lanes --ignored --nocapture`
  prints `[(Fabs, Some(0), 4), (Fmax, Some(0), 4), (Fadd, Some(2), 2), ...]`
  (op, recorded width, `comps_of`).
- Why it cannot draw wrong: `comps_of`'s contract is "an over-estimate, never a
  wrong narrow answer" (`ir.rs:656-658`), and every store bounds its own lanes.
  `StoreLocal` has `write_mask`. A varying store takes its lane count from the
  declaration's `components` (`v3d/prepare.rs:386-394`). The V3D leaves a lane
  whose source lanes do not exist uncomputed, and the PowerVR's `result_lanes`
  (`powervr/select.rs:2465-2466`) computes lanes nothing reads.
- The cost: wasted lanes (instructions and registers), not pixels.
- The fix belongs in the built-in emitter: record the argument's width, which the
  checker already knows.

**2. The `Bool` comment in `ir.rs` is stale.** It says "0 or 1 in an integer
lane", but every producer writes all ones for true: `eval.rs` (`bits_of_bool`),
the PowerVR's constants, the GC7000's comparisons (its `B2f` is an AND mask that
relies on it), and the V3D. Only `B2f`/`B2i` give a bool a numeric value. `ir.rs`
is frontend3's file: reported to frontend3
(`.claude/worktrees/report-frontend3-from-v3d-m5.md`), not changed here.

## Order

1. The interpreter, validated on Mesa's code (step 2 above).
2. prepare + select + schedule + allocate for the smallest programs: a pass-through
   VS/CS, a constant FS, an FS that reads varyings (`color_uv`, `interp_color`).
3. Arithmetic across the kernel, then uniforms (plain and indexed), then control
   flow (Mesa uses branches with per-lane conditions; this is where the design needs
   its own record), then textures.
4. A `gpu-v3d` crate feature and its `codegen::compile_linked` path, when the chain
   compiles something worth running.

## M6: the GL core on the V3D

Status (2026-09-24, v3d-glue): written and host-checked, not yet run on the board:
the `gpu-v3d` feature and its gate rows, the V3D shader ABI (`ShcV3d`, compile per
stage, the program at link), the job builder (`driver/src/v3d/v3d_job.c`, held to
Mesa by `tools/v3d/cl_golden.c` and `job_check.c`), the backend (`src/v3d/`), the
Zephyr OSAL (`src/osal/zephyr_v3d.c`) and the sample
`driver/samples/zephyr/v3d_gl_triangle`. The aim: the GL core draws
through the V3D, so that gl_test and later dEQP-GLES2 run on the Pi 4 as on the other
two GPUs. Nothing in the GL core's structure changes; the V3D fits the seams the GC7000
already uses (a `gpu-*` feature, a `ShcShader` extension, a backend table, an OSAL).

### Compile and link

The V3D compiles a program, not a stage (`chain::compile_program(vs, fs, prim,
Target)`). The GL core's shader ABI is per stage, but its `link` entry point already
takes `fs` by `*mut` and may replace it: `link_online` recompiles the fragment shader
for varyings that are declared and never written, and hands back the new record.
M6 uses the same move for the whole program.

- **glCompileShader**: the front end and the reflection only (`lower::compile_to_ir`,
  `reflect`). Errors are reported here, as on the other GPUs (dEQP reads the compile
  status). The stage's `ShcShader` carries the reflection rows (attributes, varyings,
  uniforms, `uniform_dwords`) and no code (`usc_dw` 0). The record keeps the **source**
  (it already does, for blend variants); the IR is not kept: it lives in the per-compile
  scratch, and lowering again at link costs time, not memory in `keep`.
  **A stage with no code** must not read as a failed compile or an empty upload.
  Every reader of a compiled stage's `usc`/`usc_dw`, found by
  `grep -rln usc_dw` over the tree (the search finds `upload_prog_gpu`, the known
  reader), outside the per-GPU tools and tables:
  - `gl/shader.rs:323`: the compile status is `rc == 0`, never the code size.
  - `gl/draw.rs:1038-1055` (`upload_prog_gpu`): uploads a stage only when
    `usc_dw != 0`. The VS copy uploads nothing and `vs_usc_gpu` stays 0; the V3D
    backend never reads it. The linked FS uploads the three programs.
  - `gl/draw.rs:996` (a variant's upload): only a points variant would get here,
    and it carries code.
  - `gl/draw.rs:2099,1091` (`dump_small_fs_block`): a debug print gated on
    `usc_dw > 900`; it only reads the number.
  - `testutil.rs:538-541`: reads `usc_dw * 4` bytes at `vs_usc_gpu` in tests, which
    is 0 bytes for a V3D VS.
  - `gl/shader.rs:1214`: a test asserting a table shader has code. It runs on the
    PowerVR table build only, never on `gpu-v3d` (no table).
  - C: `include/gles_shaderc.h` declares the field; no C file in `driver/` or
    `tools/` reads `usc_dw` (the same grep, 0 hits there). `pvr/` and `vivante/`
    read it and are not built on `gpu-v3d`.
- **glLinkProgram** (`shc_backend.link(vs, &mut fs)`): after the existing source-level
  rules, lower both sources again in scratch, run `compile_program(.., Triangles,
  Target { vpm_size: 65536 })`, and publish a **program record** in `keep`, cached by
  sha256(vs source's hash, fs source's hash, zero-varying list, prim, vpm_size)
  (`online/backend.rs` `link_program`). The compiler's own version is NOT in the key,
  and that is deliberate (asked for in the review, answered here): the cache lives in
  RAM, starts empty in every process and is never persisted, so every record it holds
  was built by the compiler of the running binary; a stale record cannot exist to
  match. A version in the key would guard a persistent cache, which this is not.
  `*fs` becomes the
  record's shader and the old copy is released (the `link_online` pattern). A back-end
  refusal (`V3dError`) fails the link with the chain's error in the program log; it is
  never a draw-time surprise.
- **Freeing program records**: a program record is a `Compiled` record like any
  other: one extent of `keep`, a holder count, freed by `store::free_keep` when the
  last holder releases it, and removed from the cache at that moment. The holders are
  the GL core's copies: `glLinkProgram` takes the new record and releases the
  program's previous `rfs` (`gl/program.rs:652-657`); `glDeleteProgram` releases it
  (`destroy_program`); two programs that link the same pair get the same record
  from the cache, which retains once per hit, so it lives until both let go. The
  per-stage records are released as they are today. So there is a free path, and no
  bump arena with a reset: the keep region is a granule map (`store.rs`). Bound: the
  region is `KEEP_BYTES` (512 KiB) for every live shader and program together. A
  record that does not fit fails the compile or the link with the log line that names
  the constant (`-ENOMEM`); nothing is overwritten. What one program costs in `keep`
  (both sources, the record, three code blocks, three uniform streams) is printed by
  the first host test that links one, and is not measured yet.
- **What the linked FS `ShcShader` carries** (`ShcShader::v3d: ShcV3d`, under
  `gpu-v3d`, mirrored in `include/gles_shaderc.h` and checked like `shc_viv`):
  - `usc`/`usc_dw`: the three QPU programs back to back, FS, VS, CS, each starting at
    a multiple of 8 bytes. The GL core uploads `usc` unchanged (`upload_prog_gpu`), so
    `fs_usc_gpu` + the word offsets in `ShcV3d` give the three code addresses. Mesa
    uploads exactly `qpu_size` bytes at 8-byte alignment (v3d_program.c:507): no padding.
  - the prog_data of each program (the fields of `v3d_fs_prog` / `v3d_vs_prog` except
    the code address and the uniform stream);
  - the three uniform streams, `(contents, data)` pairs, as pointers into `keep`;
  - the attribute map: for each GL attribute location 0..15, the attribute record it
    is fetched through, or none (`V3dProgram::attributes`).
- **The VS copy** stays per stage: its attribute rows give `vi_reg = location` (a
  `matN` is one row, split per column by `draw.rs::vertex_inputs` as on the GC7000:
  that `cfg!` widens to `gpu-v3d`). The backend maps each `GlesAttrib::vi_reg`
  (a location) to its record through the linked FS's map; a location the program does
  not read is skipped, a record with no `GlesAttrib` refuses the draw (-EINVAL).
- **Primitive variants**: M6 links the triangles program only. Lines use it too.
  The evidence, from Mesa 26.2.1, for what the primitive changes in a program:
  - the FS key's `is_points`, `is_lines`, `line_smoothing` (v3d_program.c:604-608).
    `is_lines` changes the code only when the FS reads `SYSTEM_VALUE_LINE_COORD`
    (nir_to_vir.c:4873-4878), and `line_smoothing` only with GL_LINE_SMOOTH
    (vir.c:1282). GLSL ES 1.00 has no line coordinate and ES 2.0 no line smoothing,
    so neither can be set by an ES 2.0 program. `is_points` matters only when the FS
    reads `gl_PointCoord` (nir_to_vir.c:4869), and `point_sprite_mask` only for
    points (v3d_program.c:707).
  - the VS key's `per_vertex_point_size`, set only for points
    (v3d_program.c:783-785), and read only there (v3d_nir_lower_io.c:427,
    vir.c:1111).

  So for lines Mesa builds the same three programs as for triangles. Our chain has no
  input that could make them differ either: `compile_program` refuses `Prim::Lines`
  before it looks at the modules, and nothing after that check reads `prim`. The
  shader record fields that depend on the primitive
  (`point_size_in_shaded_vertex_data`, `disable_implicit_point_line_varyings`) are
  the same for lines and triangles too. Points need no variant after all (the
  plan above this line was superseded): the board measured that the record's
  point-size bit only describes the VPM layout, for every primitive (gl_test
  110, image 37264ce2). So a VS that writes gl_PointSize keeps the word for
  every draw (v3d-m5, ac6045b6), the record's bit follows the program's
  writes_psiz (a3d3fb0c), and points are drawn with the triangles program
  (9e95fe76). See "M6 points: one program for every primitive".

### Uniforms

`QUNIFORM_UNIFORM`'s `data` is a dword index into the stage's uniform block
(`prepare.rs`: `LoadUniform`'s `dword_off + c`, bounded by `uniform_dwords`). That
block is exactly what `glUniform*` fills through the reflection rows
(`ShcUniform::dword_off`) and what `draw.rs` hands the backend as `vs_uniform_cpu` /
`fs_uniform_cpu` with their dword counts. So no GL-core change: at each draw the
backend writes the three streams into the frame's arena, the VS and the CS from the
VS block, the FS from the FS block, with the viewport contents from the draw's
viewport. An index outside the block, or a null block, refuses the draw; it is never
read as 0. Texture contents (`QUNIFORM_TEXTURE_CONFIG_P0_*`, `P1`) come with textures.

### The frame: from `GlesDraw` to control lists

`src/v3d/` calls the C frame builder in `driver/src/v3d/` through FFI, as `src/pvr/`
calls the PowerVR generators. Not a Rust port, because `v3d_cl.h` is checked byte for
byte against Mesa's packers (`cl_golden`, 720,024 cases) and whole frames are decoded
by Mesa's CLE (`frame_check`); a port would lose both checks. The builder becomes a
job builder with three steps (Mesa: `v3d_get_job`, `v3d_draw_vbo`, `v3d_job_submit`):

- `v3d_job_begin(target, clear)`: the BCL prologue. One job is one frame of one
  target; the clear flags and values go to the RCL (`CLEAR_COLORS`, `ZS_CLEAR_VALUES`,
  `CLEAR_TILE_BUFFERS`), so a clear at the start of a frame is a TLB clear.
- `v3d_job_draw(job, draw)`: the state packets, then the uniform streams, the shader
  record and the attribute records in the indirect buffer, then the primitive.
  The first cut writes the whole state for every draw (what Mesa writes for the first
  draw of a job, every dirty bit set); dirty tracking is an optimisation for later.
- `v3d_job_end(job)`: `FLUSH`, the generic tile list (with a `LOAD` of RT0 when the
  frame did not clear it: the target keeps what earlier frames drew), and the RCL.

| `GlesDraw` | V3D (Mesa v3dx_emit.c / v3dx_draw.c) |
|---|---|
| `viewport`, `depth_range`, `flip_y` | `v3d_viewport_gl` -> `CLIPPER_XY_SCALING`, `CLIPPER_Z_*`, `VIEWPORT_OFFSET`, the viewport uniforms |
| `scissor`, `scissor_rect` | `CLIP_WINDOW`, intersected with the viewport |
| `cull`, `front_cw` | `CFG_BITS` forward/reverse facing, clockwise |
| `depth_test`, `depth_func`, `depth_write` | `CFG_BITS` depth function and Z updates; early Z per `v3d_update_job_ez` |
| `polygon_offset*` | `DEPTH_OFFSET`, `CFG_BITS` enable |
| `stencil*` | `STENCIL_CFG`, front and back |
| `blend*`, `blend_color` | `BLEND_ENABLES`, `BLEND_CFG`, `BLEND_CONSTANT_COLOR` (the `shc_blend_factor` codes are `pipe_blendfactor`, so `v3d_factor()` is transcribed as is) |
| `color_mask` | `COLOR_WRITE_MASKS` (inverted) |
| `line_width`, `prim` | `LINE_WIDTH`; the V3D primitive codes |
| arrays / elements | `VERTEX_ARRAY_PRIMS`; `INDEX_BUFFER_SETUP` + `INDEXED_PRIM_LIST` (v3dx_draw.c:1305-1333) |
| `attribs` | attribute records; float32 only at first (`native_attrib_formats` = float32, the GL core converts the rest); staged `cpu` data copied into the frame arena |
| `do_clear*`, `clear_*` | the RCL clear of the job |
| `clear_quad` | refused (-EINVAL) in the first cut; later a draw with a clear program the backend compiles once at init (see below for what this blocks) |
| `textures` | later: TMU uniforms and texture shader state; the images must be UIF or LT tiled |

- **Several draws per frame**: all go into one BCL; the job is submitted on `flush`,
  `finish`, a target change, or when an arena is full (then the next job loads RT0).
  The tile allocation size grows with the draw count (`v3d_frame_alloc_sizes`); the
  binner's out-of-memory interrupt gets overflow blocks (`PTB_BPOA`/`BPOS`, "M6 job
  arenas" below, step c).
- **Depth and stencil** live in the tile buffer only (D24S8 internal, cleared every
  tile); nothing stores or loads them yet. So a job cannot see what an earlier job
  left in them. The rule is refuse, not draw wrong: the backend keeps, per target
  (`src/v3d/backend.rs`, `ZsKnown`, depth and stencil apart), what the last job left:
  `Untouched` (GL leaves it undefined: any start value is right), `Cleared(v)` (the job
  cleared it and drew nothing into it: the next job starts from the same clear value,
  which is exactly what memory would hold) or `Written` (a draw wrote it: no later job
  can reproduce it). A draw that reads depth (the depth test on) or stencil (the
  stencil test on) in a job that did not clear it, on a `Written` target, is refused:
  -EINVAL, one log line naming the cause, `GL_OUT_OF_MEMORY` in the GL core. A job
  that must split in the middle of such a scene (an arena full, `glFlush`, a
  read-back) therefore refuses the next depth or stencil draw instead of drawing it
  against the wrong depth. Colour-only scenes split freely and load RT0. The history
  holds 8 targets; a target not in it once it is full counts as `Written`, never as
  `Untouched`. Storing and loading Z/S (a UIF buffer, `ds_persist`) lifts the refusal
  later.
- **Target**: raster RGBA8888 at the GL core's row pitch (`tex_pitch`: whole pixel
  pairs, so an odd-width target has a pixel of padding per row; step B corrected
  the backend's `w * 4`, which only even widths had used). A window surface and an
  RGBA8 FBO texture are both such targets; RGB565 and the other formats later.
- **A draw that does not go in leaves nothing** (`v3d_job_draw`): the job builder
  rolls its lists back to where they stood and answers FULL (the backend submits the
  job and records the draw in a new one; FULL again is `-E2BIG`) or REFUSED
  (`-EINVAL`). The job's earlier draws are never lost to a later draw's overflow.
- **Submit**: the backend hands the OSAL a `struct v3d_kick` ("V3DK" magic: BCL and
  RCL start/end, tile alloc and tile state with their sizes) and gets a fence back, as
  the GC7000 does with `gc_kick`. The Zephyr OSAL (`src/osal/zephyr_v3d.c`) zeroes the
  tile state, runs bin then render and polls (`v3d_submit_bin`, `v3d_wait(FLDONE)`,
  `v3d_submit_render`, `v3d_wait(FRDONE)`, `v3d_caches_clean`), so `submit` returns
  after the job; the backend waits for every job before it reuses the job memory, so
  one job is in flight at most. GPU addresses are 32-bit V3D MMU addresses; a `u64`
  that does not fit refuses the draw. The OSAL owns the carve-out (a `__noinit` static
  mapped Normal non-cacheable), the page table and the power-on; the board's display
  (the firmware framebuffer) and its watchdog stay board support in the sample, which
  hands the OSAL the one thing of the board's it needs, the V3D clock switch
  (`struct osal_zephyr_v3d_init`), as the EGL native display. Binning waits with
  `v3d_wait_bin`, which hands the binner overflow blocks (step c below). Not in M6:
  hang recovery (the job fails with the registers printed; the watchdog resets the
  board).
- **VPM**: the link uses 65536 (the Pi 4, measured under Linux); the backend's `init`
  compares it with the identified `vpm_size` and refuses to start on a mismatch.

### What the host gate does not cover

`tools/check.sh`'s `gpu-v3d` test row runs `--lib` with `gl::` and `testutil::`
skipped, because those tests take their shaders from the offline table, and this
GPU has none (303 failed for "not in the offline shader table" when run). So on a
`gpu-v3d` build the GL core itself is compiled and linted but NOT tested: that is a
coverage gap, not a pass. What the row does test is the V3D backend (on fakes of the
job builder), the state translation, the compiler with its V3D chain, and the
compile/link/free path of the V3D program. The gap closes with the first gl_test
run on the Pi 4.

### What the first board run can show

The `clear_quad` refusal blocks the gl_test cases that clear through a scissor
smaller than the target, or clear stencil with a partial write mask (the GL core's
rule, `draw.rs:1217-1233`). A static scan of `gl_test.c` (a `glClear` after
`glEnable(GL_SCISSOR_TEST)`, or a stencil clear after a `glStencilMask` other than
0xff; it finds `clearscis`, the known case) lists: 5 `scissor` (a 32x32 box on the
64x64 target), 58 `clearscis`, 59 `clearmask`, 66 `clearvp`, 67 `fbods`,
88 `rsflip`. The scan does not follow state set in helpers, so the list is a
lower bound until the first run prints each refusal. Cases that need textures, points
or RGB565 are blocked by those refusals in the same way. The first run names each
blocked case by its refusal line, not by a pixel failure. (An earlier version of this
paragraph and of the list below named FBO targets as refused. Nothing refuses them:
an RGBA8 texture target is drawn like a window. Corrected in step B.)

### The common back-end entry (planned)

The online compiler's GPU back ends have no common entry today. The GC7000's
selection (`online/select.rs`), ABI assembly (`online/abi.rs`) and its
`codegen::compile`/`compile_linked` sit in the shared layer under
`cfg(gpu-vivante)`; the PowerVR and the V3D each have their own chain
(`powervr/chain.rs`, `v3d/chain.rs`). M6 does not move the first two (their
owners' sessions hold them, and a move needs their regression runs). It routes the
V3D's link through ONE shape that can later serve every GPU:

```text
codegen::compile_program(vs: &Lowered, fs: &Lowered, key: ProgramKey,
                         scratch: &Arena, keep: &'static Arena)
    -> Result<ShcShader, CompileError>
ProgramKey { prim: ProgramPrim (Triangles | Lines | Points), blend_key: u32 }
```

- IN: each stage's IR, as `lower::Lowered` (the `ir::Module` and the reflection
  the ABI record carries), with the fragment shader's zero-varying list already
  applied (`lower::zero_varyings`, as on every GPU); the key; the two arenas.
- OUT: the program's code and its ABI record, as the `ShcShader` the program
  holds. On the V3D that is the fragment shader carrying all three programs
  (`ShcV3d`). A per-stage GPU calls its stage compiler twice inside it; its OUT
  then has to be two records (the vertex one and the fragment one), which is the
  one open point of the shape, settled when the first per-stage GPU moves.
- The key is what selects a binary on any GPU: the V3D's primitive variants, the
  PowerVR's blend variants (`blend_key`, 0 where the hardware blends).
- The source-level wrapper `codegen::link_sources` (lower both sources again,
  since a record keeps sources, not IR, then `compile_program` with
  `ProgramKey::LINK`) is what `online/backend.rs` `link_program` calls.

The V3D arm of `compile_program` is the call sequence v3d-m5 gave (2026-09-24):

```text
let fs = lower::zero_varyings(a, fs_lowered, zero)?;          // in link_sources
let prog = v3d::chain::compile_program(&vs.module, &fs.module,
                                       Prim::Triangles, Target { vpm_size }, a)?;
compile_program<'a>(vs: &ir::Module, fs: &ir::Module, prim: Prim, target: Target,
                    a: &Arena<'a>) -> Result<V3dProgram<'a>, V3dError>
```

no_std, allocating only in the arena, checked for aarch64-unknown-none; only
`Prim::Triangles` compiles, Lines and Points return `NotYet` by name. That
QUNIFORM_UNIFORM's datum is the dword index into the stage's block is pinned on
v3d-m5's side by `quniform_uniform_data_is_the_dword_index_into_the_block`, with a
negative control.

What the later move needs (after M6's first board PASS, coordinated with the GC7000
and PowerVR owners):

- **GC7000 items that leave the shared layer**, into `online/vivante/`:
  `online/select.rs`, `online/abi.rs`, and in `codegen.rs` the `gpu-vivante` arms:
  the `BackEndError` variants `Select`/`Alloc`/`Encode`/`Abi` and their `From`
  impls, `assemble_from_ir`, `abi_outputs`, `compile`/`compile_linked`, and the
  `chain_tests` and other `cfg(all(test, feature = "gpu-vivante"))` modules there.
- **PowerVR**: the `gpu-powervr` arm of `compile`/
  `compile_linked` and `BackEndError::Usc` in `codegen.rs`, and its tests, into
  `online/powervr/`.
- **cfg branches that go away**: in `codegen.rs` every per-GPU arm of
  `BackEndError`, `compile`, `compile_linked` and the V3D's `compile_program` (one
  module per GPU, chosen once); in `online/backend.rs` the three-way tail of
  `online_link` (`link_shaders` / `link_shaders_viv` / `link_shaders_v3d`), the
  `gpu-v3d` arms of `online_link` and `link_online` (`link_program` against
  `link_substituted`), and `each_input`'s two arms; in `online/mod.rs` the `cfg` on
  `abi`, `select` and `v3d_abi`.
- **The evidence each GPU needs before the move is kept.** The move is meant to
  change no instruction, so the strongest check is also the cheapest: every corpus
  source compiles to the same words, the same reflection and the same refusal
  before and after (per source, not totals), on each GPU's table. Beside that:
  - GC7000: `tools/check.sh` (the `gpu-vivante` rows), the online oracle
    (`tests/online_oracle.rs`, `tools/online_oracle.py`) with no source changing
    outcome, `tests/backend_differential.rs`, and gl_test on the i.MX8MP with the
    online compiler giving the same PASS set as before.
  - PowerVR: `tools/check.sh` (the `gpu-powervr` rows), the online oracle on the PowerVR
    table, the USC tests (`tests/usc_limits.rs`, `tests/usc1_triangle.rs`), and
    gl_test on the AM62P with the online compiler giving the same PASS set.
  - V3D: its gate rows, and the M6 board run again (`v3d_gl_triangle`, and gl_test
    once it runs there).

### What M6 does not cover yet

Points, textures, RGB565 and the other non-RGBA8 targets, persistent depth/stencil,
non-float attributes, the binner overflow pool, dirty-state tracking. Each is refused
by name where it would be needed, not drawn wrong. (`clear_quad` and the masked colour
clear were on this list until step B, below.) RGBA8 FBO textures are drawn, not
refused, and have not been drawn on a board yet.

## M6 step B: the clear quad, and gl_test on the Pi 4

Status 2026-09-24: the clear quad is implemented and host-tested
(`src/v3d/state.rs` `clear_quad_draw`, `src/v3d/backend.rs` `draw_clear_quad`, the
tests below, `tools/v3d/job_check.c` part 4: 156 of 156 checks); no board has drawn
it yet. Step A (f6630ec1) refused a partially masked colour clear; step B draws it,
together with every other `clear_quad`, and builds gl_test for `rpi_4b` so the board
can show it.

### The clear quad

The TLB clear (the RCL's CLEAR_TILE_BUFFERS) clears whole tiles and every channel.
A clear that is scissored, stencil-masked or colour-masked is drawn instead, as Mesa's
st does for a screen without `clear_scissored` / `clear_masked` (st_cb_clear.c:440-471
choose the quad, `clear_with_quad` at :194 draws it; mesa-26.2.1). Our quad takes
Mesa's state:

| state | the quad's value | Mesa |
|---|---|---|
| colour write mask | `clear_color_mask` when `do_clear`, else 0 (COLOR_WRITE_MASKS) | :239 |
| depth | when `do_clear_depth`: test on, ALWAYS, write on; else off | :254-255 |
| stencil | when `do_clear_stencil`: ALWAYS, REPLACE on fail, zfail and zpass, value mask 0xff, write mask `stencil_write_mask`, ref `clear_stencil`, both faces; else off | :261-268 |
| z | every vertex at `2 * clear_depth - 1`, viewport the whole target, depth range (0, 1) | :309 |
| extent | the whole target with the scissor test on the GL core's `scissor_rect` | :300-308 draws the rect itself |
| blend, cull, dither | off | the st's clear rasterizer, :89 |

The one difference: Mesa's quad covers the scissor rectangle, ours covers the target
and is scissored. Our scissor translation (y flip included) is already tested and
drawn by M6; a second way to place the rectangle would be a second thing to check.

**The program.** A vertex shader that passes `attribute vec4 a_pos` to `gl_Position`,
and a fragment shader that writes `uniform vec4 u_color`: GLSL ES 1.00 text in
`src/v3d/`. The GC7000's `tools/shaders/internal/clear.*` are the same two shaders,
but in GLSL 450 with a push constant, which our compiler does not take. `v3d_init`
compiles and links them with the product compiler, in the image, once. The program
record holds its keep extent for the backend's life. Init prints the result once:
"clear program built: N bytes of code", or "clear program did not build: <the step>:
<the compiler's own error>". Init still succeeds then, and every quad is refused with
its own reason ("the clear program did not build, so a clear the TLB cannot do is not
drawn"); the TLB clears still work. The uniform stream of the program reads dwords 0..3 of the
fragment block, which is `clear_rgba`.

**The draw.** `v3d_draw` turns a `clear_quad` `GlesDraw` into an ordinary draw and
sends it down the same path as an application's (`state::job_state`, `record`): six
vertices (two triangles) from a CPU array staged like any client array, the state in
the table above. So the quad goes through the same depth/stencil history: with depth
test ALWAYS it counts as a depth read (`zs_use`), so a scissored depth clear on a
target whose depth an earlier job wrote is refused, which is right, because the
pixels outside the box keep a value no job loads. The same holds for a partial
stencil write mask.

**A masked background clear.** A colour clear with a partial mask that arrives as a
background clear (not `clear_quad`), alone or fused into a draw, is split as the
PowerVR splits it: depth and stencil stay in the TLB clear, the job loads colour,
the quad draws the colour with the mask, then the fused draw. Step A's refusal goes,
`backend::masked_clear_applied()` answers true on `gpu-v3d`, and the `draw.rs` skip
of the GL core's log line goes with it (with the answer true the line is never
printed).

**Host evidence, landing with the change** (the rule for a change that alters what
is drawn):

- The clear program builds on the host, and its uniform stream reads dwords 0..3 of
  the fragment block.
- Backend tests on the strict mocks: a scissored clear is one draw with the scissor,
  the clear program and the colour mask; each of colour / depth / stencil alone and
  together gives the state in the table; a masked background clear gives a TLB clear
  for depth and stencil plus one colour quad; a masked clear fused into a draw gives
  the quad, then the draw, in that order; a scissored depth clear after a job split
  is refused.
- The packets: `tools/v3d/job_check.c` checks the quad's COLOR_WRITE_MASKS (a
  partial mask), STENCIL_CFG (REPLACE, write mask) and CFG_BITS (depth ALWAYS, write
  on) through Mesa's CLE decoder, as it does for M6's draws (run by hand, with a
  Mesa tree).
- The GL suite does not run on this build's host row (the coverage gap named in
  `check.sh`), so the GL-level cases are the board's.

### gl_test for `rpi_4b`

`driver/samples/zephyr/v3d_gl_test` compiles the same `driver/tools/pvrlab/gl_test.c`
that the AM62P and the i.MX8MP run. Built for `rpi_4b` with 0 warnings; the QEMU smoke
run (`V3D_RPI_WDOG=OFF`) reaches the expected stop: the list is parsed ("5,58,59,66,67,102
(6 cases)"), the OSAL's power-on fails with -19 (QEMU has no V3D), and the sample prints
FAIL ("no case ran"), not an empty pass.

- **The board glue** is `v3d_clear`'s sources (mailbox, HDMI console, watchdog), which
  `v3d_gl_triangle` already compiles directly. The planned move into a
  `v3d_common/` directory turned out to be unneeded: only the V3D clock callback
  (ten lines) is repeated, and `v3d_gl_triangle` is not touched.
- **gl_test.c** has three arms now: `GL_TEST_VIVANTE`, `GL_TEST_V3D` (defined from
  `CONFIG_YAKOGL_GPU_V3D`) and `GL_TEST_PVR`. The V3D arm starts the OSAL with the
  sample's `gl_test_v3d_osal_arg()` (the clock switch) and sets no backend options.
  The sites that call the PowerVR's knobs or counters (MRT stride, the clip sweep,
  the wait-timeout counter, the pretransform option, the performance counters)
  became `#ifdef GL_TEST_PVR`; the maskclear case's PowerVR message is
  `#elif defined(GL_TEST_PVR)`. Case 67's GL_RGB attachment frame stays
  `#ifndef GL_TEST_VIVANTE`: it tests a GL feature, not a knob, so the V3D runs it
  (and refuses the RGB target by name until that format is written).
  **Equivalence:** `gcc -E -P` of the old and the new file is byte-identical for six
  PowerVR and Vivante configurations (Linux user, host, Zephyr with both shader
  sets; Zephyr headers stubbed), and differs (176 lines) for the V3D one, the
  positive control.
- **Shaders:** `GL_TEST_ES100=1`, always.
- **Target:** the suite's own 64x64 default surface. No EGL window.
- **What runs:** `CONFIG_V3D_GL_TEST_CASES`, default "5,58,59,66,67,102", walked on
  one context through `gl_test_set_list`; `CONFIG_V3D_GL_TEST_CASE` runs one case
  alone. The console scrolls, so the sample repeats every case's verdict, the pass
  count and the build at the end (through gl_test's `gl_test_case_result` hook).
- **Case 88 is not in the list:** `case_rsflip` is the GC7000's resolve engine and
  returns -2 (no such case) on any other GPU, so it cannot be evidence here.
- **What each case can show:**
  - 58 `clearscis`, 66 `clearvp`: scissored colour clears.
  - 59 `clearmask`: colour, depth and stencil write masks on clears, then draws
    that read them.
  - 102 `maskclear`: a full-screen R+A clear, a scissored G clear, and B fused into
    a draw, every pixel checked.
  - 5 `scissor` and 67 `fbods` also render into an RGBA8 FBO texture, which nothing
    refuses and no board run has drawn on the V3D yet. 67's third frame is a GL_RGB
    attachment, which the backend refused by name ("fewer than 8 alpha bits") until
    "M6 colour targets with fewer than 8 alpha bits" below; it is drawn now. **How these two are
    counted:** a PASS with no refusal line is evidence for the quad and the FBO
    target together; any other result is not evidence about the quad until its first
    failing check and its refusal lines have been read and attributed.

### What the board run is

The lead runs it: the list above from the commit's image, each case's line and the
summary; then `v3d_gl_triangle` again as the regression, because `v3d_draw`'s clear
path changed (its full-mask TLB clear takes the same path as before; the quad is built
only when a clear needs it).

## M6 step C: depth and stencil kept between jobs

Status 2026-09-24: implemented and host-tested (see "What was built" at the end of
this section); not yet on the board. The lead moved it onto the critical path: dEQP
batch I ended its session on the refusal below (a refused draw is GL_OUT_OF_MEMORY,
the proposal `proposal-refused-draw-error.md` is with the user), and
depth_stencil_clear, polygon_offset, multisample and femtovg's stencil fills need
it. What follows was the plan; "What was built" says where the code differs. Today depth and stencil live in
the tile buffer only (the frame section above). A draw that reads depth or stencil
an earlier job wrote is refused by name. gl_test 59 `clearmask` stops there on
the board: "the draw reads stencil an earlier job wrote, and stencil is not kept
between jobs yet". Step C stores Z/S to memory at the end of a job, and loads it
at the start of the next job that needs it, as Mesa's v3d does for a zsbuf
(`v3dx_rcl.c`, `load_general` and `store_general`, mesa-26.2.1).

### The buffer

- **One buffer per `GlesDraw::ds_id`**, the key the GL core already gives every
  draw (`draw.rs` `resolve_ds_storage`: the window surface, or the FBO's depth and
  stencil renderbuffers), sized for the target. This is the PowerVR's scheme
  (`pvr/backend.rs` `ds_get`): a fixed table, a buffer re-allocated when its id
  comes back at another size, and the least recently used slot given up when the
  table is full (round robin until the fix below).
  A target with `ds_persist` false (an FBO with no depth or stencil attachment)
  gets no buffer; GL gives it no depth or stencil either.
- **Format D24S8**, one 32-bit word per pixel: the tile buffer's internal type is
  already `depth_24` (M6), and Mesa's zsbuf for `S8_UINT_Z24_UNORM` has the
  load/store format D24S8 (`v3dx_format_table.c:149`). Z and S are always loaded
  and stored together (ZSTENCIL, buffer 11): the V3D 4.2 loses the clear of one
  when only the other is loaded (GFXH-1461, `v3dx_draw.c:1676-1689`).
- **Layout UIF (no XOR)**, Mesa's default for a tiled resource
  (`v3d_resource.c:680-696`). For 4 bytes a pixel the utile is 4x4 and the UIF
  block 8x8 (`v3d_tiling.c:38-69`). The width is aligned to four UIF blocks (32
  pixels), the height to one (8 pixels), and "Height in UB" is the padded height
  / 8 (`v3dx_rcl.c:63-66`). Mesa's `ub_pad` and its XOR choice are a page-cache
  optimisation; NO_XOR without the pad is a valid layout, and the TLB is the only
  thing that reads or writes the buffer, so its internal order never has to be
  known on the CPU. (Raster Z/S would avoid UIF, but Mesa never takes that path
  for a zsbuf, so it is a path with no reference behind it.)

### The job

- **Load** (`LOAD_TILE_BUFFER_GENERAL`, ZSTENCIL, UIF no XOR, D24S8, per tile,
  after the RT0 load) when the job starts from what an earlier job stored: the
  history says `Written` (renamed `Stored`, since there is now memory behind it)
  and the job does not clear both Z and S.
- **Store** (`STORE_TILE_BUFFER_GENERAL`, the same fields, after the RT0 store)
  when the job wrote or cleared Z or S, so the next job can load it. Afterwards
  the history is `Stored`.
- **A partial Z/S clear with a load** (only depth, or only stencil, while the
  other must be loaded): the cleared half is drawn by the clear quad (step B),
  depth-only or stencil-only over the whole target, after the load, instead of
  by the TLB. This is Mesa's rule (`v3d_tlb_clear` drops such a clear from the
  TLB, `v3dx_draw.c:1676-1689`, and `v3d_draw_clear` draws it).
- The refusals "the draw reads depth/stencil an earlier job wrote" remain only
  for a target with no buffer: `ds_persist` false, or an allocation that failed
  (which is logged once).
- `TILE_RENDERING_MODE_CFG_COMMON` keeps early Z off (M6). Mesa's
  `early_depth_stencil_clear` is an optimisation for jobs that neither load nor
  store Z/S; it stays off.

### Evidence, landing with the change

- `tools/v3d/cl_golden.c`: the ZSTENCIL load and store packets, byte for byte
  against Mesa's packers.
- `tools/v3d/job_check.c`: a job that loads and stores Z/S through Mesa's CLE
  decoder (buffer 11, memory format UIF no XOR, D24S8, the height in UB, the
  address), and a job without a buffer that emits neither.
- Backend tests on the strict mocks:
  - a depth draw after a split loads, and the job before it stored;
  - a job that only clears still stores;
  - a partial Z/S clear with a load goes to the quad;
  - a new size under the same id gets a new buffer;
  - a full table gives up the oldest slot;
  - a target with `ds_persist` false is still refused as before.
- On the board: gl_test 3 `depth`, 9 `stencil`, 11 `persist`, 12 `stencil2`,
  13 `zls` (depth and stencil kept across kicks), and 59, from the commit's image;
  `v3d_gl_test`'s default list gains them. Then `v3d_gl_triangle` again as the
  regression (it has no depth buffer in use, so its RCL must not change: checked
  on the host first by job_check's identity part).

### What was built

- `driver/src/v3d/v3d_job.h/.c`:
  - `struct v3d_job_clear` gains `zs_address` (0: none), `zs_memory_format`,
    `zs_height_in_ub`, `zs_load` and `zs_store`.
  - The per-tile list loads Z+Stencil (buffer 11, D24S8 43) after RT0's load, and
    stores it after RT0's store, before the per-tile clear, as `v3dx_rcl.c` does.
  - The packets are unchanged in `v3d_cl.h`, only named constants were added.
- `src/v3d/backend.rs`:
  - A job's target is `Target { gpu, w, h, ds }`, where `ds` is `GlesDraw::ds_id`
    when `ds_persist`. Another buffer on the same colour target is another job.
  - `zs_buf`: up to `ZS_BUFS` (8) buffers by id. A new size under the same id is a
    new buffer. A full table gives up the least recently used slot (each `zs_buf`
    call stamps the buffer it returns), with a log line; that id then starts
    Untouched. The GL core does not say when a renderbuffer is deleted, so a run
    of dEQP cases leaves the table full of buffers of objects that are gone, and
    those are the least recently used. Round robin gave up buffers the running
    case still used: shared_depthbuffer.rbo_bgra_depth_component16_stencil_index8
    on 8f2c90a3 logged "the depth/stencil buffer of id 5 is given up for id 3",
    then the window's "for id 5" (fbo B's stencil, cleared to 1, came back
    empty, so the stencil test passed everywhere). Host test:
    a_full_table_gives_up_the_least_recently_used_buffer.
  - The layout is `v3d_tex_layout` (Mesa's zsbuf: one level, 4 bytes a pixel), with
    UIF XOR turned into NO_XOR (`zs_layout`). The height is the padded height in UIF
    blocks (8 rows). Level 0 is 4 KiB aligned.
  - `ZsKnown::Stored`: a job that wrote depth or stencil stores both, and the next
    job loads them unless it clears them.
  - A partial Z/S clear whose other half is Stored goes to the clear quad
    (GFXH-1461), decided on the job the clear goes into.
  - A target with no buffer (`ds_persist` false, or no memory) keeps the old
    per-target history and refusals. Their wording is now "... and the target has no
    depth/stencil buffer to keep it in".
- **One difference from the plan:** a job that only CLEARS stores nothing. The next
  job starts from the same clear values (`ZsKnown::Cleared`), as a job with no
  buffer does. Storing a clear it can reproduce would cost a full write for nothing.
- `src/osal/zephyr_v3d.c`: prints UIFCFG once at init (Linux v6.17 `v3d_regs.h`,
  `V3D_HUB_UIFCFG` 0x004).
- Tests:
  - `backend_tests.rs`, 7 new tests:
    - stored by the writer and loaded by the next job;
    - a clear-only job stores nothing;
    - the partial clear goes to the quad;
    - a new size gets a new buffer;
    - a full table gives up the oldest;
    - a new buffer on the same colour target is a new job;
    - `zs_layout`.
    Each has a negative control that fails it.
  - `job_check.c` part 7 (20 checks) decodes a Z/S job through Mesa's CLE decoder:
    buffer, format, D24S8, height, address, RT0 stored before Z/S, a store-only job
    with no load, and no buffer with neither. Controls: no Z/S store, a load of Z
    only, the Z/S store first. Each is caught.
  - `cl_golden.c`: our four Z/S constants against Mesa's.
- `v3d_gl_test`'s default list gains 3, 9, 11, 12 and 13.

### On the board

- c2edfb2d (the step C image, 11 cases): 8 of 11. 59 passes now, and 3, 11 and 13 (zls:
  depth and stencil kept across kicks, the partial clear through the quad) pass,
  0 timeouts.
  - 67 failed at its RGB frame (the colour-format gap; its record, 8bacb437, shows
    checks 1-2 ok and then the RGB clear refused).
  - 12 (two-sided stencil) had front and back swapped on the window and the FBO.
  - 9 wrote through a stencil write mask (check 9).
- b019c041 (the same list plus 4), after c359306a (the front face follows st/mesa: no
  toggle for a window, toggled for an FBO): 10 of 12 in 416 ms, 0 timed out. 12 and 4
  (cull) PASS; case 4's checks 1-6 are ok on the window and the FBO for both
  windings. gl_test 4 had never been in a V3D list, so the inverted front face was
  invisible to every board run before it. 67 (format) and 9 still FAIL.
- 9's cause (from its record, 8bacb437): the clear quad took the fused draw's stencil
  write mask instead of the glClear's. Fixed in 66a1b5d2, measured on the host; its
  board run is pending.
- dEQP (run J, 1ef6ce30 = 3c193ccd, step C on main): depth_stencil_clear 11 of 11 pass,
  0 refusals, the session complete (complete=1), 1226 ms. Before step C the same group
  passed 4, was refused on the 5th ("stencil is not kept between jobs yet", which
  GL_OUT_OF_MEMORY turned into the end of the session), and never ran the rest.
- dEQP fragment_ops (run F1, re-run on 094c4fe1, with c359306a's front face and
  66a1b5d2's clear-quad stencil mask): 1,855 pass of 1,894 begun. The run before
  those fixes (1ef6ce30) passed 1,110 of 1,923.
  - Now passing in full: depth_stencil.stencil_ops (512), stencil_depth_funcs (72),
    depth_stencil.random (25), write_mask (3) and random (99). blend is still at 100 %.
  - What remains: 38 F2b (fixed on main since batch 12), and 1 R in scissor, "points
    are not drawn: the points variant of the program is not built". That refusal's
    GL_OUT_OF_MEMORY ended the session, so 29 cases never began.
  - The numbers are the lead's reading of the board run.
- UIFCFG reads 0x00000045 on this Pi 4 (the record's repeat). The loads and stores
  round-trip (13 zls passes), so the UIF layout without XOR is right on it.

**Unmeasured until the board:** whether the V3D's UIF configuration matches what
Mesa's layout assumes. UIFCFG is a hub register (`V3D_HUB_CTL_UIFCFG`), which
Linux exposes read-only as `DRM_V3D_PARAM_V3D_UIFCFG`. Mesa's gallium and Vulkan
drivers neither read nor write it; their layout code assumes 4 KiB pages and 8
banks (`v3dv_limits.h:96-99`). With NO_XOR and no padding, those numbers only
matter for speed. Step C reads the register once at init and prints it (the
offset from the Linux driver's register list, a fact cited by file:line), so the
board states its value; the first run of 13 `zls` shows whether the loads and
stores round-trip.

## M6 textures (plan)

Status 2026-10-11: built (`src/v3d/texture.rs`, the texture state records, the
tiled copies); the Pi 4 passes dEQP-GLES2's texture groups. The plan below is
kept as written, with the board measurements code cites.

Status 2026-09-24: planned, nothing implemented. The goal behind it is Slint on the Pi 4
(femtovg draws text and images from textures). Step C (depth and stencil between
jobs) waits until femtovg's frames show they need it.

### T0. A defect first: kick_id and kick_pending

The GL core writes a texture or a buffer only after every kick that reads it has run
(`texture.rs` `settle_texture`: `kick_pending(t.last_use)`, then flush and finish).
The V3D backend's `kick_id()` returns the number of jobs already submitted, and its
`kick_pending()` always answers false. So a draw recorded in the open job gets an id
the backend calls finished. A glTexSubImage2D or glBufferSubData between two draws of
one job then writes the memory the earlier draw still reads, and that draw renders
with the new data. The fix: `kick_id()` names the open job (submitted + 1 while one
is open), and `kick_pending(id)` is true for the open job's id. The GL core's
existing flush then submits the job before the write. A backend test pins it; femtovg
(which updates its glyph atlas between draws) is the board evidence.

### T1. The tiled image

(Owned by v3d-tex from 2026-09-24: `driver/src/v3d/v3d_tex.c/.h` and
`tools/v3d/tex_check.c`. T0 and T2 stay with v3d-glue.)

The TMU does not sample a raster 2D image: Mesa samples a linear resource through a
tiled shadow. Every level is LT, UB-linear or UIF, chosen by its size, and the TMU
works out each level's address from level 0's base and the sizes. So the chain must
be laid out exactly as Mesa lays it out. New C files next to the job builder,
`driver/src/v3d/v3d_tex.c/.h`, transcribed from Mesa (MIT, with attribution):

- the level layout: `v3d_setup_slices` (`v3d_resource.c:580-770`, mesa-26.2.1), for
  2D RGBA8: levels from the smallest up, the power-of-two padding from level 1, the
  page alignment of level 1 when it could be UIF XOR, `ub_pad`, and level 0 aligned
  to 4 KiB;
- the texel tiling: `v3d_store_tiled_image` (`broadcom/common/v3d_tiling.c`), from
  the GL core's linear level to the level's tiling;
- the texture shader state (`v3d_setup_texture_shader_state`, `v3dx_state.c:904-989`:
  width, height, base and max level, the base pointer at level 0,
  `level_0_is_strictly_uif`, `level_0_xor_enable`, `level_0_ub_pad`, `extended`,
  the swizzle, and the type RGBA8) and the sampler state
  (`v3d_upload_sampler_state_variant`, `v3dx_state.c:586`: filters, wrap modes, the
  LOD range), as packers in `v3d_cl.h`.

#### T1 as built (v3d-tex; `driver/src/v3d/v3d_tex.h`)

One helper for all three users: the backend (T2), v3d-m5's `TRI_TEX` sample,
and tex_check. The packers are in `v3d_tex.h`, not `v3d_cl.h`.

- `v3d_tex_layout` and `v3d_tex_store_level` as planned. Also
  `v3d_tex_store_box(image, l, level, x, y, w, h, src, src_stride)`, which is
  `glTexSubImage2D` (femtovg fills every new image with one full
  `glTexSubImage2D` after a NULL `glTexImage2D`, and updates a gradient
  texture of more than two stops this way). It refuses a box outside the level.
- `v3d_tex_format_get`: RGBA8 and BGRA8, as Mesa's format table rows
  (type RGBA8, swizzle XYZW / ZYXW, 16-bit return). Every other ES 2.0 format is
  refused by name (`v3d_tex_format_name`). The BGRA8 row is for memory that
  really holds B,G,R,A; a `GL_BGRA_EXT` upload never reaches it, because the GL
  core swaps it to R,G,B,A as it stores (below).
- `v3d_tex_pack_state` and `v3d_tex_pack_sampler` return `bool`. They refuse,
  and write nothing, for a type no supported format has, or for any field that
  does not fit. Nothing is masked.
- `v3d_tex_p0` / `v3d_tex_p1`: the record's address ORed with the low 24 bits
  of the uniform data (QUNIFORM_TMU_CONFIG_P0/P1), as Mesa's
  `write_tmu_p0/p1` do. They refuse an address that is not 32-byte aligned,
  and payload bits outside the parameter's fields.
- `tools/v3d/tex_check.c` checks all of it against Mesa 26.2.1, and
  `tools/check.sh` runs it (a missing Mesa tree is a `gap` row, not ok).

What femtovg/Slint ask of textures, and where T2 (v3d-glue) handles it
(agreed with v3d-glue, 2026-09-24):

- The sources are femtovg 0.26 and Slint patched for no_std
  (`driver/samples/zephyr/slint/nostd/`), not the crates.io releases; an
  earlier version of this list read stock femtovg 0.25.1 and was wrong about
  text. Text: each Text item is composed on the CPU into its own premultiplied
  RGBA8 texture and drawn with `ImageRendering::Smooth` (LINEAR, CLAMP, one
  level; `slint-nostd.patch:6872-6905`). So text is sampled with LINEAR.
- Formats. YakoGL reports ES 3.0, so femtovg takes its non-ES2 path: alpha
  images are `R8`/`RED`, and an image can also be `RGB8`. Slint's cluster
  creates only RGBA8 (the text items and the logo). The GL core widens every ES 2.0 upload format to RGBA8 storage in
  its upload converters (`src/gl/texture.rs`, `convert_row`), except
  GL_LUMINANCE, GL_ALPHA and GL_LUMINANCE_ALPHA, which the V3D keeps narrow
  (L8, A8, LA8; "Narrow L8, A8 and LA8" below). That includes `GL_BGRA_EXT`: the converter
  swaps red and blue as it stores, and the bind's `bgra` flag is 0 for such a
  texture (v3d-glue, from the code). The TMU has no 3-byte type, and Mesa's
  V3D 4.2 table has no R8G8B8 row, so the widening is needed, not only
  convenient. `v3d_tex_format_get`'s refusals by name are a guard.
- Render to texture. femtovg draws into FBO textures for layers and blurs
  ("M7" above: the cluster has neither). In this design the texture's storage is the
  GL core's linear RGBA8. The V3D renders an FBO texture target in raster into
  it (gl_test case 5, passed on the Pi), and sampling always goes through a
  per-job tiled shadow. So no tiled RCL store is needed. It would be needed
  only for persistent tiled textures, which is a later measurement.
- `glTexSubImage2D` (femtovg fills every new image with one). It writes the linear storage,
  and the shadow is re-tiled whole the next time a job samples it.
  `v3d_tex_store_box` is for a shadow kept across jobs that updates only the
  changed box. It has no caller until the per-job tiling cost is measured on
  femtovg's frames.
- No mipmaps on the Slint path: Slint never sets `GENERATE_MIPMAPS`. Every
  femtovg sampler has mip filter NONE, so the LOD is clamped to 1/256.

#### LINEAR on the board (plan, v3d-tex; to agree with v3d-glue and v3d-m5)

Why. Slint text and smooth images are sampled with LINEAR (above). Nothing on the
host can say what the TMU returns for LINEAR: tex_check checks the sampler bytes,
v3d-isa's interpreter models only NEAREST, and the TMU's filter weights and
rounding are not in Mesa (there is no Mesa code that computes them). Only the
board can answer.

The reference is dEQP's, because dEQP is what will judge us:
`tcuTexture.cpp` `sampleLinear2D` (VK-GL-CTS 93bca01, `framework/common/`):
x0 = floor(u - 0.5), a = frac(u - 0.5), and the same for v, in texel units; the
four texels wrapped by the sampler; then (1-a)(1-b)p00 + a(1-b)p10 + (1-a)b p01
+ ab p11. dEQP's GLES2 filtering cases (`es2fTextureFilteringTests.cpp:316-324`)
accept 9 coordinate bits, 5 weight bits, and a colour threshold of
`computeFixedPointThreshold(colour bits - 2)` for LINEAR. So the board test
reports its error against that bound, not only "matches".

The test: a textured variant of `v3d_gl_triangle` (a textured quad through the
public GL, so the whole path is tested: T2's shadow and records, the compiler's
TMU code, and the P0/P1 words):

1. A 4x4 RGBA8 texture whose channels are separable ramps (R 0, 85, 170, 255
   along x; G the same along y; B a 0/255 checker; A 255). Every bilinear blend
   is then a different value, and a swapped s/t or a wrong channel shows.
2. A 32x32 window region, the quad covering it exactly, texture coordinates 0..1.
   Pixel x samples at u = (x + 0.5) / 8 texels, so u - 0.5 = (2x - 7) / 16 and
   every weight is a multiple of 1/16: exact at dEQP's 5 weight bits and at any finer hardware. Any error
   left is rounding (the f16 return, the TLB's unorm conversion), and the test
   measures it: max |error| per channel and a histogram, over all 1024 pixels.
3. Four draws, one sampler each: (a) LINEAR/LINEAR, CLAMP; (b) the same, REPEAT
   (the edge texels blend with the other side); (c) NEAREST/NEAREST (must be
   exact: it separates the filter from the return and write path);
   (d) min NEAREST + mag LINEAR, REPEAT, on a 16x16 quad with texture
   coordinates 0..8 (two texels a pixel, LOD 1: minified, so it must come out
   NEAREST; LINEAR there is always a 50/50 blend, so the two differ on every
   pixel): the 1/256 LOD clamp Mesa relies on for "no mipmaps" picks the right
   filter only if the hardware decides min vs mag as Mesa expects.
4. Controls, computed on the CPU before the draw, printed, and required: the
   LINEAR and NEAREST references differ on at least N pixels by more than the
   tolerance (so the probe can tell LINEAR from NEAREST); a reference shifted by
   half a texel fails the tolerance (the texel-centre convention is measured);
   the CLAMP and REPEAT references differ at the edges.
5. Verdict: PASS if (c) is exact and (a), (b), (d) are within dEQP's bound; the
   measured max error is printed either way. The first run's numbers become the
   tolerance, stated with the run that produced them.

Who: the CPU reference is `tools/v3d/tex_filter_ref.h` (dEQP's `wrap`,
`sampleNearest2D` and `sampleLinear2D`, header only, no OS calls, no libm).
`tools/v3d/tex_linear_case.h` is the whole test as a sample includes it: the
texture, the four draws with their samplers and tolerances, the per-pixel check
(the largest error per channel and a histogram), the verdict line, and the
controls. It is freestanding and prints through the caller's function (printk).
`tools/v3d/tex_filter_ref_check.c` checks both headers: 12 hand-computed pixels;
the k/16 claim for every draw; the controls (measured: LINEAR vs NEAREST 960 of
1024 pixels over dEQP's threshold, the half-texel shift 1008, REPEAT vs CLAMP
448, the minified scene 256 of 256; and a flat texture is refused); and the
verdict on host-made read-backs (the ideal one passes every draw; NEAREST for
LINEAR, a half-texel shift, LINEAR on the minified draw, a short read-back and
one step off in an exact draw each fail; 3 steps off in a LINEAR draw passes).
26 checks. Twelve mutants of the two headers each fail it. check.sh runs it.
The board carrier is gl_test case 109 "texlinear" (`driver/tools/pvrlab/gl_test.c`,
`GL_TEST_V3D` only, run by `v3d_gl_test`; 27f7382c): through the public GL, so
T2's shadow and records, the compiler's TMU code and the P0/P1 words are all in
the path.

**The board result: PASS** (the lead's record of v3d-glue's image, build
27f7382c, cases 61, 109, 18; read from the screen). The controls line printed
exactly the host's counts ("LINEAR vs NEAREST 960/1024, half-texel shift
1008/1024, REPEAT vs CLAMP 448/1024, minified NEAREST vs LINEAR 256/256"), so
the probe could tell the cases apart on silicon too. Then, per draw:

| draw | pixels | over the tolerance | max error R / G / B / A (1/255 steps) | histogram |
|---|---|---|---|---|
| (a) LINEAR, CLAMP_TO_EDGE | 1024 of 1024 | 0 over 3.98 | 0.56 / 0.56 / 0.49 / 0.00 | 64 exact, 960 under 1 |
| (b) LINEAR, REPEAT | 1024 of 1024 | 0 over 3.98 | 0.56 / 0.56 / 0.49 / 0.00 | 0 exact, 1024 under 1 |
| (c) NEAREST, CLAMP_TO_EDGE | 1024 of 1024 | 0 over 0 | 0 | all exact |
| (d) min NEAREST + mag LINEAR, REPEAT, minified | 256 of 256 | 0 over 0 | 0 | all exact |

"case 109 texlinear: PASS". What it says:
- The TMU's LINEAR filter on RGBA8 is within one 1/255 step of dEQP's
  reference on every pixel, with the weights exact (k/16) by construction. The
  largest error, 0.56, is above the 0.5 an exact rounding would give, so some
  results round to the other neighbour; that is the TMU's own rounding (the
  f16 return and the TLB's conversion), seven times inside dEQP's 3.98.
  Slint's text and smooth images are sampled this way.
- NEAREST through the f16 return and back to 8 bits is exact, as predicted.
- The minified draw came out NEAREST on every pixel: the hardware chooses
  between the min and mag filters as Mesa's 1/256 LOD clamp for "no mipmaps"
  assumes (`v3d_upload_sampler_state_variant`).
The tolerances in `tex_linear_case.h` stay at dEQP's bound (the pass criterion
the CTS applies); the measured 0.56 is the number to compare a later run with.
An earlier run of case 109 failed with "glDrawArrays was not recorded (-22)":
collateral of case 18's leaked context in the same boot, not of this case
(v3d-glue's harness fix, ce89f96a).

#### Re-tiling cost per job (plan, v3d-tex with v3d-glue)

T2 re-tiles every sampled texture into a per-job shadow. On the Slint cluster that
means every Text texture (one per Text item) and the logo, every frame, whether
they changed or not. The question is
whether that is a visible share of the frame. Measure, when T2 samples textures:
per job, the number of textures tiled, the bytes, and the time
(`k_cycle_get_64` around the tiling), printed with the frame time, on the
`v3d_gl_slint` cluster frame (and on `v3d_gl_femtovg` once it has textures).
Rule: if the tiling takes more than 10 % of the frame time, or more than 2 ms at
60 Hz, keep the shadow across jobs (freed or refreshed on a CPU write), with
`v3d_tex_store_box` updating only the changed box. That needs the GL core to
tell the backend which box changed, which is a proposal to the lead. Below
that, keep the per-job shadow: it is simpler and already correct with T0.

How it is measured (agreed with v3d-glue). The backend has no clock (layers 1-4
call no OS API), but it counts the tiling: `v3d_texture_counters(&tilings,
&bytes)`, cumulative, bytes being each tiled image's size. The tiling is
`v3d_tex_store_level` itself (`src/v3d/texture.rs` `tile()`, through the FFI).
It reads the GL core's storage and writes the shadow, and both are in the V3D
OSAL's carve-out, CPU-mapped Normal non-cacheable (`src/osal/zephyr_v3d.c`). So
the cost per frame is the counters' bytes for that frame divided by the rate
that `tools/v3d/tex_tiling_bench.h` measures on the board. The rate must be
measured on buffers from the same `gpu_alloc`: a cached buffer would give a
rate far too good. The harness checks every tiled texel, so a run that wrote
nothing cannot report a rate.

A first finding, on the host (cached memory, x86, so it says nothing about the
Pi's numbers but a lot about the code): `v3d_tex_store_level` as written first
computed a tiled address and copied 4 bytes for every texel. That is 190 KiB
per ms for 256x256 RGBA8, 1.3 ms, on a desktop CPU. It now copies whole utiles,
as Mesa's `v3d_move_pixels_general_percpp` does (64 contiguous bytes of
destination per address, texel by texel only at the box's partial edges): 2600
to 3500 KiB per ms on the same host, 72 us for 256x256. The bytes are the same
(tex_check: 339 of 339 against Mesa), and five mutants of the new path each fail
tex_check.

**The board rate** (measured 2026-09-24, the lead's run; image built by v3d-glue from
6c31ab9c, v3d_gl_triangle with the bench after its verdict; sha 3641192c...,
booted 13:57:34, read from the screen). The triangle itself: "v3d_gl_triangle:
build 6c31ab9c", PASS (inside wrong 0, largest channel diff 1, guard row 0 of
1280 changed). Then "v3d_gl_triangle: tiling bench, build 6c31ab9c", RGBA8, 5
runs each, the source and destination both from the OSAL's `gpu_alloc`
(Normal non-cacheable), every texel checked:

| size | tiled bytes | best | worst | KiB/ms | texels checked, wrong |
|---|---|---|---|---|---|
| 256x1 | 4096 | 37 us | 38 us | 106 | 256, 0 |
| 64x16 | 4096 | 118 us | 119 us | 33 | 1024, 0 |
| 128x32 | 16384 | 431 us | 433 us | 37 | 4096, 0 |
| 256x64 | 65536 | 1698 us | 1701 us | 37 | 16384, 0 |
| 256x256 | 262144 | 6774 us | 6802 us | 37 | 65536, 0 |
| 512x128 | 262144 | 6781 us | 6811 us | 37 | 65536, 0 |
| 1024x512 | 2097152 | 54542 us | 54583 us | 37 | 524288, 0 |

"tex_tiling: 7 of 7 sizes measured and checked (clock 54000000 Hz)". (256x1 is
LT, padded to 4 rows, so only a quarter of its tiled bytes are texels: per
texel byte it is 1024 B in 37 us, 27 KiB/ms, the slowest row, as the per-texel
path it takes (no whole utile fits in one row) would predict.)

What the numbers say:
- The rate is flat, 37 KiB/ms, from 16 KiB to 2 MiB. So the time is per byte
  of memory traffic, not per texel or per call. The whole-utile path made the
  code 15x faster on cached memory (above), but on this memory the CPU waits
  for the bus. The host's cached rate (2600 to 3500 KiB/ms) is 75 to 95 times
  this.
- Against the rule (more than 10 % of the frame or more than 2 ms at 60 Hz):
  2 ms buys about 74 KiB of tiled texture per frame. One 256x256 RGBA8
  texture costs 6.8 ms. The Slint cluster samples its Text textures (one per
  Text item, each sized to its item) and the logo every frame, so re-tiling all
  of them per job is over the rule. The count is for
  `v3d_gl_slint` to print with `v3d_texture_counters`; it only has to exceed
  74 KiB, and a single 256x80 text texture (80 KiB tiled) already does.

Decision: **the per-job shadow is not kept.** A texture's tiled shadow is kept
across jobs and re-tiled only when its content changed. Who can see a change
(v3d-glue, from the code): the V3D backend sees no texel write and no delete.
`glTexImage2D`, `glTexSubImage2D`, `glCopyTex*`, `glGenerateMipmap` are CPU
stores inside the GL core's storage (`src/gl/texture.rs`), and
`GlesTextureBind` carries `base_gpu` and the layout, with no object id and no
generation. Freed storage can come back at the same address for a new
texture. So the key must be fed by the GL core. v3d-glue's proposal, which the
lead is deciding: a global write stamp taken where every write path ends
(`settle_texture`), passed as `GlesTextureBind.content_gen`, and the key
(`base_gpu`, `content_gen`, level). The stamp is global and only grows, so a
new texture at a recycled address can never hit an old shadow (the ABA case),
and a shadow nobody asks for again is simply evicted (LRU), with no delete
hook needed. A draw that renders into the texture is a write too: the
re-tile must wait for that job's fence. Open: an EGLImage sibling writes the
same memory without touching this texture's stamp.

What the cache cannot fix: the Slint path's uploads, **measured** by v3d-slint
(a host recorder driving gl_slint's own Rust archive, 1295 frames of the scripted
drive at 58 fps, `slint/host/slint_gl_rec.c`; data bytes, each counted
once; it replaced an earlier estimate that was 4 times too low). Every new
texture is `glTexImage2D` with NULL, then one full `glTexSubImage2D` (one
tile, not two, when the tile is taken at the first sample).
- First frame: 54 new textures, 1,677,236 bytes (the scale numbers, the texts
  from 400x21 to 400x147, the 330x132 logo).
- Idle: 177,182 bytes a frame, 10.3 MB/s. New every frame: two 256x1 conic
  gradient textures (the arcs' spans move), the two frame counters (320x21
  and about 77x21), and rpm (400x65).
- Accelerating: 417,411 bytes a frame, 24.2 MB/s: the same, plus speed
  (400x147) every frame and four 256x1 gradients.
- Cruise: 139,315 bytes a frame. The worst single frame: 759,684 bytes.

These textures are NEW in every frame, so a shadow cache saves nothing for
them; only the per-byte rate matters. Tiled sizes are larger than the data
(`v3d_tex_layout`): 1.08 to 1.42 times for the texts (400x147: 252,928 tiled
for 235,200), and 4 times for a 256x1 gradient (LT, padded to 4 rows: 4096 for
1024). At about 1.15 times the data, the tiling at 37 KiB/ms costs about
5.4 ms a frame idle, 12.8 ms accelerating, and 23 ms in the worst frame,
against 16.7 ms. To bring the accelerating frame under the rule's 2 ms, the
rate must reach about 236 KiB/ms, 6.4 times today's. The same code does
2600 to 3500 KiB/ms on cached memory (the host). So the cache is necessary
(the static texts and the logo tile once), but the deciding number is the
rate.

**The second board run: the source read is the whole cost** (the lead's run of
v3d-glue's image 10e1d641, sha 303099e1..., booted about 15:00, read from the
screen; "tex_tiling: A 7 of 7, B 6 of 6, C 6 of 6 sizes measured and checked",
clock 54000000 Hz, 0 wrong texels in every mode). Best / worst of 5 runs:

| size | A: src NC, dst NC | B: src cacheable, dst NC | C: both cacheable |
|---|---|---|---|
| 256x1 | as in the first run | 8 / 8 us, 466 KiB/ms | 8 / 8 us, 465 KiB/ms |
| 64x16 | as in the first run | 17 / 17 us, 235 | 17 / 17 us, 226 |
| 128x32 | as in the first run | 70 / 73 us, 228 | 70 / 70 us, 228 |
| 256x64 | as in the first run | 279 / 289 us, 229 | 279 / 279 us, 229 |
| 256x256 | 6825 / 6853 us, 37 | 1121 / 1139 us, 228 | 1121 / 1121 us, 228 |
| 512x128 | 6850 / 6866 us, 37 | 1115 / 1117 us, 229 | 1116 / 1122 us, 229 |
| 1024x512 | 54893 / 54937 us, 37 | (not run: the cacheable buffers hold 256 KiB) | (not run) |

B equals C: the destination's memory type does not matter, and reading the
source from Normal non-cacheable memory is the whole cost. With a cacheable
source the tiling runs at 228 KiB/ms, 6.2 times mode A. For the measured Slint
frames (above, tiled bytes) that is about 0.9 ms idle, 2.1 ms accelerating and
3.8 ms in the worst frame, against 5.4, 12.8 and 23 ms today. (The 256x1 row's
466 KiB/ms counts its padded 4096 tiled bytes; per texel it is about 117.)

**Decision (the lead's): tile at upload, from the caller's pixels.** It is the
second touch of the GL core, under the same rules as the first: one additive
backend hook, `texture_uploaded(bind, level, box, src pixels, stride)`, called
from the GL core's `glTexImage2D` / `glTexSubImage2D` path after its own store,
and a no-op on the PowerVR and the GC7000. The V3D backend tiles straight from
`src` into the texture's tiled copy (`v3d_tex_store_box` with the box, or
`v3d_tex_store_level` for a whole level) and stamps the copy with the write's
content generation, so the first sample re-tiles nothing. `glCopyTex*`,
`glGenerateMipmap` and a draw into the texture keep the lazy re-tile from the
GL core's storage. Slint needs only the simplest case (v3d-slint, measured):
every texture is written exactly once, whole, with one full `glTexSubImage2D`
right after a NULL `glTexImage2D` (0 sub-boxes in 9,503), from its cacheable
UI heap, rows packed. The NULL allocation carries no data and tiles nothing.
The design is v3d-glue's with v3d-tex (the C layer needs no new function).

#### Cube maps (v3d-tex, with v3d-glue and v3d-m5)

- Layout, as Mesa lays out a cube (`v3d_resource.c:757-779`, checked by
  tex_check part 9):
  - each face is a whole mip tree of the 2D layout;
  - face f starts at `f * v3d_tex_layer_stride(l)`, where the stride is
    `align(slices[0].offset + slices[0].size, 64)`;
  - `v3d_tex_layers_size` gives the image size, and `v3d_tex_layer_offset`
    gives where face f of level L starts.
  - To tile face f, pass `image + f * stride` to `v3d_tex_store_level`.
- Record: `image_depth` is 6 and `array_stride_64` is the stride / 64
  (`v3dx_state.c:936-961`). There is no cube bit. The compiler's lookup
  writes TMUR and TMUSCM (v3d-m5).
- Board test: `tools/v3d/tex_cube_case.h`, gl_test "texcube".
  - Each face is a 4x4 image, sampled by textureCube in the fragment shader
    with NEAREST and with LINEAR.
  - Every pixel is checked against dEQP's selectCubeFace, projectToFace and
    2D sample.
  - The LINEAR pixels at a face edge are measured, not judged, because GL ES
    2.0 does not require seamless filtering.
  - gl_test 113 "cubemap" (another team's) checks the placement of every face
    at every level; texcube checks the sampling.

#### Mipmaps on the board (v3d-tex)

- dEQP's `texture.mipmap.2d` cases (basic, affine, generate) pass on the Pi 4
  (T3, a3f3c39f). gl_test "texmip" (`tools/v3d/tex_mip_case.h`) is the
  regression pin that keeps them true.
- Texture U: 64x64 with 7 levels, one colour each, uploaded level by level
  (as dEQP's basic cases do).
- Texture G: level 0 is a one-texel checker; the other levels come from
  glGenerateMipmap and are exactly its mean.
- Draws: quads of 42 and 14 pixels, so the LOD is 0.61 and 2.19, under all
  four mipmap min filters: 12 draws.
- The verdict is dEQP's own: its LOD bounds (`computeLodBoundsFromDerivates`
  with derivateBits 10, lodBits 8), its choice of levels, and
  `isLinearRangeValid`.
  - At LOD 0.61 a nearest-mipmap filter must read level 1, so rounding down
    fails.
  - At LOD 2.19 a linear-mipmap filter must blend levels 2 and 3, so reading
    one level alone fails.
- Host: tex_filter_ref_check part 6 (15 checks). 8 of 8 mutants of the
  header are caught.

#### Cube mipmaps on the board (v3d-tex)

- Why: dEQP's `texture.mipmap.cube.{basic,bias,projected}` fail on the Pi 4
  (batch 23, 8f2c90a3), `nearest_nearest` included.
  - For basic and bias, dEQP's reference differs from an ES3 context's only
    in `seamlessCubeMap` (es2fTextureMipmapTests.cpp:658). `lodBits` is 6
    either way (:669); the ES2 value 4 applies to projected look-ups only.
  - A NEAREST sample never reads across a face, so seamless filtering alone
    cannot explain those failures.
- gl_test 133 "texcubemip" (`tools/v3d/tex_cubemip_case.h`) asks what is left,
  on face interiors only: is each face's level L where the TMU reads it, and
  does the TMU pick the level dEQP allows?
  - The scene is dEQP's own: a 64x64 cube with 7 levels, one colour per face
    and level.
  - Per face, NEAREST_MIPMAP_NEAREST at LODs where dEQP allows exactly one
    level (0 to 4), and LINEAR_MIPMAP_LINEAR blending levels 1-2 and 2-3 with
    every footprint inside the face. That is 42 draws.
  - The verdict is dEQP's `computeCubeLodBoundsFromDerivates` (derivateBits
    10, lodBits 6) and its 8 - 2 bit colour threshold.
  - Each draw's line names the (face, level) its centre pixel reads, so a
    level found at the wrong place names itself.
- Host: tex_filter_ref_check part 7 (10 checks). 6 of 6 mutants of the
  header are caught.
- Board (8e360630): 133 PASS, all 42 draws, 0 faults. So level placement
  and choice inside the faces are right.
- dEQP's failures (qpa8 masks) are two full-height columns, the last column
  of a 17x128 cell and the first of the 7x128 cell after it. Those pixels
  read their face one level finer. 133's odd-sized quads also have a 2x2
  quad cut by the viewport edge, and they pass; 133 has no thin cell.
- gl_test 134 "texcubethin" (`tools/v3d/tex_cubethin_case.h`) draws the thin
  cells: 17x64 at x 40 and 7x64 at x 57 (dEQP's thin sides and pixel
  parity), and the same two turned, on every face, NEAREST_MIPMAP_NEAREST.
  - Two arms: the direction as interpolated, and divided in the shader by
    its largest component (Mesa's `nir_normalize_cubemap_coords`).
  - Each line counts the invalid pixels whose 2x2 quad the cell cuts apart
    from the rest.
  - Host: tex_filter_ref_check part 8 (6 checks). 5 of 5 mutants of the
    header are caught.
- Board (image d4fb2127, same case code as 7f2cce78; record
  `record_gltest_134_d4fb2127.txt`): 134 FAIL, as the as-is arm predicts.
  0 MMU faults, 0 recoveries.
  - As-is arm: 24 of 24 draws FAIL, every one with "split-quad pixels 64 of
    64 invalid, whole-quad pixels 0 invalid". The invalid pixels read their
    face one level finer (level 1 where dEQP allows 2, level 2 where it
    allows 3..4). This holds on all six faces, for both the s and t cells.
  - Normalized arm: 24 of 24 draws PASS, 0 of 64 split-quad pixels invalid.
- What this measures: when a cell's edge cuts a 2x2 quad, and the direction
  is not divided by its major axis, the TMU reads one level too fine for the
  pixels of that quad. Dividing the direction by its largest absolute
  component before the TMU (as Mesa's `nir_normalize_cubemap_coords` does)
  fixes it. v3d-m5's compiler change (a1faa59d / 27beed81) does that.
- An earlier note from this team said "133 passes, so normalization is not
  the fix". That was wrong. 133 has only square cells, and they do not show
  the defect. 133 still proves what it was built for: inside a face, each
  level is where the TMU reads it, and the TMU picks a level dEQP allows.
- Caveat: on a build with m5's normalization, the compiler divides the
  as-is arm's direction too. Both arms are then normalized and should both
  pass, so the case can no longer tell the two apart. Only a build without
  that change (such as d4fb2127) shows the as-is failure.
- The same fix closes dEQP `shaders.random.texture`: 152/200 on 8e360630
  (`record_RT_B25_8e360630.txt`), 200/200 on 57d6467d
  (`record_RT_B.txt`), 0 MMU faults and 0 recoveries in both runs. The
  lead's triage puts all 48 failures on cube sampling. The records do not
  show shader sources, so that was not re-checked here. The B arm also
  carries 012b6fe8 (constant folding), so this pair alone does not isolate
  the normalization. With 134, the cube-random bucket (these 48 and the 8
  in S5f) and the mipmap.cube face-edge failures are read as one defect
  with one fix.

#### Decision: seamless cube filtering, a reference-selection difference (v3d-tex)

The user decided on 2026-09-25 (16:52): the conformance configuration keeps
reporting ES 2.0 (`YAKOGL_REPORT_ES2`). The 25 cases below are recorded as
a reference-selection difference, not as a hardware constraint: they pass
under an ES 3-reporting context; they fail only against the ES 2.0
reference. No shader-side emulation will be written. (An earlier decision
the same day, 14:50, recorded 19 of them as a hardware constraint; the two
experiments below replaced it.)

- The mechanism:
  - The V3D 4.2 always filters cube maps seamlessly: a LINEAR sample at a
    face edge reads texels of the next face (gl_test 115, measured; see
    "On the board with v3d-m5's cube look-up" below). The records have no
    bit to turn it off: Mesa's `v3d_packet.xml` has no such field, and
    Mesa's `v3dx_state.c` `translate_wrap` passes the wrap modes through.
    ES 3.0 requires exactly this (ES 3.0.6 Appendix F.2, in the sources).
  - dEQP picks its reference by the context: `seamlessCubeMap =
    glu::IsES3Compatible(gl)` in texture.mipmap, texture.vertex and
    shaders.texture_functions (es2fShaderTextureFunctionTests.cpp:633 and
    the lines cited in that section). A context that reports ES 2.0 only
    gets the non-seamless ES 2.0 reference.
  - Mesa's context on this GPU is ES 3 (`main/version.c:694`), so Mesa is
    judged against the seamless reference and passes.
- The measured proof: the same cases on an image with `YAKOGL_REPORT_ES2`
  off (the context reports ES 3, so dEQP uses its seamless reference), both
  on build 758a3123, which has v3d-m5's cube normalization:
  - `record_SEAM19_noes2_758a3123.txt`: the 19 cases of the first decision,
    19 of 19 pass, complete=1, 0 MMU faults, 0 recoveries. Image
    `v3d_deqp-758a3123-SEAM19-noES2`, sha256
    e08c15a84a61ee78955ef0b4143f14fa27762e7c878346b8b3c866d9b4d62261,
    9,923,432 bytes (the size the record loaded, 0x976b68).
  - `record_CUBELIN6_noes2_758a3123.txt`: the six LINEAR mipmap.cube cases,
    6 of 6 pass, complete=1, 0 MMU faults, 0 recoveries. Image
    `v3d_deqp-758a3123-CUBELIN6-noES2`, sha256
    b8bf81f3cb437e02b5345b4b4b8b0be35dc3db769569e94c4978c906823e246a,
    9,923,432 bytes (loaded 0x976b68).
  - Both images' `.config`: "# CONFIG_YAKOGL_REPORT_ES2 is not set" and
    "# CONFIG_DEQP_REPORT_ES2 is not set". Neither record prints the
    context version itself; the configuration is the evidence.
- The 25 cases:
  - shaders.texture_functions.vertex.texturecubelod: a 256x256 cube, 9
    levels, LINEAR_MIPMAP_NEAREST, LOD -1..9
    (es2fShaderTextureFunctionTests.cpp:642, :666). At levels 7 and 8 a
    face is 2x2 or 1x1, so every LINEAR sample touches a face edge.
  - 18 texture.vertex.cube cases (record `record_TV_B26_76e63ccd.txt`):
    10 filtering cases that combine mipmaps with LINEAR, and 8 wrap cases
    (every pair except repeat_repeat).
  - 6 texture.mipmap.cube cases: {basic, bias, projected} x {linear_linear,
    linear_nearest}, the ones that filter LINEAR within a level (record
    `record_B27_CUBETF_758a3123.txt`; their six NEAREST-within-level twins
    pass). Their error masks (`qpa_cubelin/`, 758a3123) lie on face edges
    only: with dEQP's own cell grid (computeGridLayout, seed 7, compiled
    against its deRandom.c; each cell is one whole face), every invalid
    pixel of the basic and bias cases is within half a texel of a face
    edge at the level LINEAR reads there (basic.linear_linear 1267 of 1267,
    in a band that is 15 % of the image; bias 3293 of 3293 and 3445 of
    3445, band 30 %), and 922 of 925 and 997 of 1005 for projected, the
    rest one texel out. Scripts: v3d-tex's scratchpad `cubelin/`.
- Caveats that still hold:
  - Not explained: texture.vertex.cube.wrap.repeat_repeat passes against the
    ES 2.0 reference, although a non-seamless REPEAT reference and a
    seamless sample should differ at face edges. No public source was
    found on how the V3D applies wrap modes to a seamless cube.
  - Forcing CLAMP_TO_EDGE on cube samplers (what Mesa's state tracker does
    when seamless is on, `st_atom_sampler.c:84-95`) would not change the
    ES 2.0 result: it does not make the GPU non-seamless, and
    wrap.clamp_clamp fails there although it already uses CLAMP_TO_EDGE.
  - No gl_test case pins the difference yet (a face-edge sample judged
    against both a seamless and a non-seamless reference would); the two
    experiments above are the board evidence.
  - The count's presentation (how the 25 appear next to the ES 2.0 score)
    is the user's; this section records the classification only.
- Sources, checked on the internet on 2026-09-25 (the user's rule of
  15:45: when the spec and the hardware disagree, confirm with public
  information). Mesa at b739817b (main that day), dEQP at 93bca018.
  - The Raspberry Pi graphics maintainers say the hardware has no
    non-seamless mode. FOSDEM 2024, "Graphics stack updates for Raspberry Pi
    devices" (J. M. Casanova and J. A. Suárez, Igalia), slide "From OpenGL
    2.1 to 3.1": "Missing HW features, not fully conformant: ... Non-seamless
    cubemap filtering"
    (https://archive.fosdem.org/2024/events/attachments/fosdem-2024-2841-graphics-stack-updates-for-raspberry-pi-devices/slides/22765/FOSDEM-2024-Raspberry-Pi-Graphics-Stack-Update_pX9XpmS.pdf).
  - Mesa's own Pi 4 CI expects failures for the same reason:
    `src/broadcom/ci/broadcom-rpi4-fails.txt:233-236`, "# OpenGL 3.x applies
    non-seamless cubemap texturing, while our driver/GLES uses seamless
    cubemap texturing.", over two piglit cube tests marked Fail
    (https://gitlab.freedesktop.org/mesa/mesa/-/blob/b739817b7e816d9728be1aad9616127c2c84e926/src/broadcom/ci/broadcom-rpi4-fails.txt#L233). The lines came with
    b346c019d529 "v3d: add support for ARB_texture_cube_map_array"
    (2023-04-13) and 728e31686471 "v3d/ci: annotate failures" (2023-05-11).
  - Mesa's Vulkan driver for the Pi has no non-seamless mode either:
    `docs/features.txt:676` lists VK_EXT_non_seamless_cube_map for anv,
    hasvk, hk, lvp, nvk, panvk, pvr, radv, tu and vn, not v3dv.
  - The register description: Mesa's `src/broadcom/cle/v3d_packet.xml` has
    no seamless field in "Texture Shader State" (max_ver 42, line 1592) or
    "Sampler State" (line 1709); the word does not occur in the file.
    Broadcom's own V3D 4.x documentation is not public; this file is the
    public description.
  - Why Mesa passes: an ES 2.0 request gets Mesa's highest ES version
    (`src/mesa/main/version.c:482` compute_version_es2, used by
    `_mesa_compute_version` :599-606 whatever minor version was asked for),
    ES 3.1 on the Pi 4 (the FOSDEM slides: "Conformant OpenGL ES 3.1");
    `_mesa_is_gles3` is then true (`src/mesa/main/context.h:335-338`) and
    `version.c:694` sets `ctx->Texture.CubeMapSeamless` for it. dEQP then
    asks for a seamless reference, since the context is ES3-compatible.
  - The specs: ES 2.0.25 section 3.7.5, "This new (s t) is used to find a
    texture value in the determined face's two-dimensional texture image
    using the rules given in sections 3.7.6 through 3.7.8" (one face, its
    wrap modes); ES 3.0.6 Appendix F.2 (p. 333), "OpenGL ES 3.0 requires that
    all cube map filtering be seamless. OpenGL ES 2.0 specified that a single
    cube map face be selected and used for filtering. See section 3.8.9.1."
    (https://registry.khronos.org/OpenGL/specs/es/2.0/es_full_spec_2.0.pdf,
    https://registry.khronos.org/OpenGL/specs/es/3.0/es_spec_3.0.pdf).
  - dEQP's reference: `seamlessCubeMap = isES3Compatible` in
    es2fTextureMipmapTests.cpp:658, `m_isES3Capable` in
    es2fVertexTextureTests.cpp:1154, `glu::IsES3Compatible(gl)` in
    es2fShaderTextureFunctionTests.cpp:633
    (https://github.com/KhronosGroup/VK-GL-CTS/blob/93bca01861b0e3ef3c387027a9791e6d065f900c/modules/gles2/functional/es2fTextureMipmapTests.cpp#L658).
  - Not found: any public statement of how the V3D applies wrap modes to a
    seamless cube (which would explain wrap.repeat_repeat passing), and any
    Khronos issue about ES 2.0-only implementations on seamless-only
    hardware.

#### Narrow L8, A8 and LA8 (v3d-tex)

- Why: dEQP texture.size.cube.512x512_l8_mipmap was refused ("larger than
  the tiled-copy budget and the texture arena", record
  `record_SW_T4_c9eb1078.txt`). The V3D tiled every texture at 4 bytes a
  texel, so that L8 chain was 8,404,992 bytes (6 x 1,400,832, from
  `v3d_tex_layout` itself), 16 KiB over the 8 MiB budget that image had.
  The GL core also kept L8 expanded into RGBA8, 4 times its size.
- What changed: the V3D samples them as Mesa does
  (`v3dx_format_table.c:114-117`, return size 16 as RGBA8's):
  - L8 as R8 with swizzle XXX1: (L, L, L, 1);
  - A8 as R8 with 000X: (0, 0, 0, A);
  - LA8 as RG8 with XXXY: (L, L, L, A)
  (ES 2.0 table 3.12; `v3d_packet.xml`: R8 is type 0, RG8 is 2).
  `backend::narrow_la_texels()` is true for the V3D, so the GL core stores
  them at 1, 1 and 2 bytes a texel. `src/v3d/texture.rs` lays out and tiles
  each level at the format's size (`texel_bytes`), and reads the GL core's
  levels at that size (`gl_level_layered`'s `bpp`). The record's type and
  swizzle come from `tmu_format`. `driver/src/v3d/v3d_tex.c`'s format
  table has the three rows now, so `v3d_tex_pack_state` accepts R8 and RG8.
  The tiled-copy key includes the format.
- Effect on that chain: 2,113,536 bytes tiled (2.0 MiB), a quarter, and a
  quarter of the GL core's store. The dEQP group itself is settled by the
  larger tiled-copy budget on the dEQP image (deqp's change: texture.size
  69/69 on 578304ef with 16 MiB, the lead's T4 run), which the RGB888 and
  RGBA8888 mipmap cubes need anyway (8,404,992 bytes at 4 bytes a texel,
  Mesa's size too). This change is for the memory: every L8, A8 and LA8
  texture (femtovg's alpha masks among them) costs a quarter or half.
- Host evidence:
  - `tools/v3d/tex_check.c` against Mesa 26.2.1: the three rows are Mesa's
    own (type, cpp, swizzle, return size), and records of L8 64x64 x7, A8
    512x512 x10 (UIF) and L8A8 13x7 x3 pack to Mesa's bytes; 354 checks.
    A mutant with L8's alpha swizzle ALPHA and one with L8A8's type RGBA8
    each fail two checks.
  - Backend tests: narrow textures are tiled at their own size with R8/RG8
    and the swizzles (every byte of an 8x4 chain and a 13x7 level); the GL
    core's narrow store (2D chain, odd level, LA8 cube) is where the backend
    reads it; femtovg's GL_LUMINANCE is stored narrow. A mutant that tiles at
    4 bytes a texel (the old rule) fails the tiling test and both format
    tests of `texture.rs`.
- Board test: gl_test 135 "texnarrow" (`tools/v3d/tex_narrow_case.h`). Per
  format, a 64x64 chain drawn level by level at one texel a pixel (every
  tiling mode at 1 and 2 bytes), a 13x7 level, and a 16x16 cube of 5 levels
  drawn per face at levels 0 and 2: 60 draws, every pixel against the
  hashed bytes of its texel.
  - Controls, run before the draws: 21,552 reference pixels accepted; each
    wrong read (next texel, next row, transposed, next level, next face,
    4-byte rows, LA8 bytes swapped) accepted on at most 1.2 % of its pixels.
  - Host: tex_filter_ref_check part 9 (16 checks). 5 of 5 mutants of the
    header are caught.
  - Board (image 6f463f58, record `record_GT135.txt`, 2026-09-25): case 135
    PASS. The controls line equals the host's (21,552 of 21,552 reference
    pixels accepted; each wrong read accepted on at most 1.2 %); 60 draw
    lines, all PASS; "texnarrow draws judged = 60"; 0 MMU faults, 0
    recoveries.
- dEQP on the same build (record `record_TEX135_DEQP.txt`): texture.format.*
  and the texture.size list (2D up to 1024, every cube size but the three
  512 cube mipmaps of 4-byte formats, which need the larger budget) at the
  default 8 MiB tiled budget: 100 of 100 pass, complete=1, 0 MMU faults,
  0 recoveries. texture.size 2d and cube 512x512_l8 and _l8_mipmap pass;
  the tiled-copy high-water was 4,198,400 of 8,388,608 bytes.
- Sources, checked on the internet on 2026-09-25 (Mesa at b739817b):
  - Mesa's V3D format table, `src/gallium/drivers/v3d/v3dx_format_table.c`
    lines 114-117: `FORMAT(A8_UNORM, NO, R8, SWIZ_000X, 16, 0, true)`,
    `FORMAT(L8_UNORM, NO, R8, SWIZ_XXX1, 16, 0, true)`,
    `FORMAT(L8A8_UNORM, NO, RG8, SWIZ_XXXY, 16, 0, true)`
    (https://gitlab.freedesktop.org/mesa/mesa/-/blob/b739817b7e816d9728be1aad9616127c2c84e926/src/gallium/drivers/v3d/v3dx_format_table.c#L114). tex_check links
    this same table (Mesa 26.2.1) and finds our rows equal to it.
  - The type numbers: `src/broadcom/cle/v3d_packet.xml:333` "Texture Data
    Format R8" value 0, `:335` "Texture Data Format RG8" value 2.
  - What each format samples as: ES 2.0.25 table 3.12, ALPHA (0, 0, 0, At),
    LUMINANCE (Lt, Lt, Lt, 1), LUMINANCE_ALPHA (Lt, Lt, Lt, At). ES 3.0.6
    Appendix F.1 keeps the three formats as legacy features.
  - Not found: any public V3D statement about these formats beyond Mesa's
    table; none was needed, since the board case checks the pixels.

### T2. The draw

- Per draw, for every `GlesDraw::textures[i]`, the backend makes (or finds, once per
  job) a tiled shadow of the texture in GPU memory it owns. It reads the GL core's
  linear levels at `base_gpu` (the packed chain at `tex_pitch` rows; the offsets
  pinned by a test against the GL core's own computation, as `target_pitch` is),
  tiles them, and writes one texture state and one sampler state record into the
  indirect buffer.
- A shadow lives for one job. That is correct because, with T0, any CPU write to the
  texture first flushes the job that reads it, and a texture the GPU rendered into
  was rendered by an earlier, finished job. The price is one tiling per texture per
  job, read from non-cacheable memory; it is counted and printed. A cache across jobs
  needs to know when the GL core wrote the texels (a change counter in
  `GlesTextureBind`, which is the GL core's): measure first, then propose it if the
  count shows it matters.
- The uniforms: Mesa's convention (`v3d_uniforms.c:129-232`): a
  `QUNIFORM_TMU_CONFIG_P0`'s data is `unit << 24 | p0 bits`, and the driver writes
  the texture state's address ORed with the low bits; `P1` the sampler state's
  address ORed with the p1 bits (plus the unnormalised-coordinates bit). The unit is
  the GL sampler binding (`GlesTextureBind::unit`). The job builder writes both from a
  per-draw table of (texture state, sampler state) addresses. It refuses a unit with
  no texture by name, never writes a null.
- GL's incomplete texture (`GlesTextureBind::incomplete`): the backend's own 1x1
  (0, 0, 0, 1), made once at init.
- Formats: RGBA8. The GL core stores GL_BGRA_EXT uploads as R,G,B,A (src/gl/texture.rs
  convert_row), so they are plain RGBA8 here. The one binding with `bgra` = 1 is an
  EGLImage of an XRGB8888 native buffer (texture_attach_external: B,G,R,X in memory),
  which state() samples with a swizzle. Float, compressed and layered textures are
  refused by name for now.
- Caches: the TMU and L2T caches must not hold an earlier job's texels. What the
  driver invalidates before a job is checked against the Linux driver's sequence
  (facts cited by file:line), before the board run.

### Evidence, landing with it

- `tools/v3d/tex_check.c`: our tiling byte for byte against Mesa's
  `v3d_store_tiled_image` (linked from `libv3d_neon.a`) for every tiling mode
  and a set of sizes (1x1 to 512x512, non-power-of-two, every mip level); our level
  offsets against numbers derived by hand from Mesa's `v3d_setup_slices` for those
  sizes. The function is static in Mesa, so it cannot be linked.
- `tools/v3d/cl_golden.c`: the texture and sampler state packets against Mesa's
  packers; `tools/v3d/job_check.c`: a textured draw's P0/P1 uniforms through Mesa's
  CLE decoder.
- Backend tests: T0; a shadow per texture per job; the P0/P1 addresses; the
  incomplete texture; the refusals.
- Board: a textured variant of `v3d_gl_triangle` with pixel read-back (nearest and
  linear, a mip chain, wrap modes), then gl_test's texture cases (2 texture, 6 fbo,
  60 wrapmode, 61 texincomp, and the mipmap cases femtovg's path uses), then
  `gl_femtovg` for `rpi_4b`.

The compiler side (the TMU lookups, `QUNIFORM_TMU_CONFIG_P0/P1`) is v3d-m5's. This
section's convention is Mesa's; it is shared with them before any code depends on it.
## M7: femtovg and Slint on the Pi 4 (plan)

**Status 2026-09-24 20:23 (v3d-slint): Zephyr + Slint + YakoGL run on the Raspberry
Pi 4.** `v3d_gl_slint` (build 1b045956: m5's discard fix aa391d2f plus the samples'
fringe checks) draws the SafeUI cluster through Slint's FemtoVG renderer, YakoGL's
GL core, its in-process compiler and its V3D backend, and reads its first frame
back: "v3d_gl_slint: PASS (the AM62P's cluster frame through YakoGL on the V3D)".
All five AM62P reference points are within 1 of the AM62P's own readings, and
`v3d_gl_femtovg` PASSes with the anti-aliased fringes included. The record is in
"The milestone" below; the rest of this section is how it was reached.

The goal is the Slint cluster on the Pi 4. Slint draws through its FemtoVG renderer,
so femtovg comes first: a smaller frame with a scene we can check pixel by pixel.
The AM62P samples `gl_femtovg` and `gl_slint` are not edited. The V3D samples reuse
their Rust crates and UI by path, and have their own board files and checks.

### What femtovg asks of the GL

Read from femtovg 0.26 as patched for no_std (`slint/nostd/`,
`renderer/opengl.rs`, `program.rs`, `main-vs.glsl`, `main-fs.glsl`).

**At canvas creation.** femtovg builds all its programs at once, whether a frame
uses them or not: 4 sets (glyph texture yes/no, times discard yes/no) of up to 9
shader types = **30 programs, 29 distinct fragment shaders** (TextureCopyUnclipped
is built twice). If one fails, `Canvas::new` fails and nothing is drawn. So every
one of the 29 must compile, not only the ones a frame uses. Each program binds
`vertex` = 0 and `tcoord` = 1 before the link. The uniforms are `viewSize`
(vec2, both stages), `frag[14]` (vec4 array, fragment) and two samplers, `tex`
(unit 0) and `glyphtex` (unit 1). YakoGL reports ES 3.0 (no `YAKOGL_REPORT_ES2`, as
on the AM62P), so femtovg takes its non-ES2 path: one vertex array object,
`R8`/`RED` for alpha images, `UNPACK_ROW_LENGTH`.

**Per frame** (`render`): the state once (cull BACK/CCW on, blend on, depth test
off, scissor off, colour mask all, stencil mask all, op KEEP, func ALWAYS, units 0
and 1 unbound); **one** `glBufferData(GL_ARRAY_BUFFER, <all vertices>,
GL_STREAM_DRAW)`; two float32 vec2 attributes, stride 16. Then per command:
`glBlendFuncSeparate`, `glUniform4fv(frag, 14)`, and a texture (or 0) on each of
the two units. The commands:

| command | GL | stencil |
|---|---|---|
| ClearRect | scissor on, `glClear(COLOR \| STENCIL)`, scissor off | cleared |
| ConvexFill | `TRIANGLES` (fill) + `TRIANGLE_STRIP` (AA fringe) | none |
| ConcaveFill | colour off, ALWAYS, INCR_WRAP front / DECR_WRAP back, cull off, `TRIANGLES`; the fringes with EQUAL 0; the cover strip with NOTEQUAL and op ZERO | written, back to 0 at the end |
| StencilStroke (the default: `Paint` has `stencil_strokes` true) | the strip 3 times: EQUAL 0 / INCR with the **discard** program (threshold 1 - 0.5/255); EQUAL 0 / KEEP (AA); ALWAYS / ZERO with colour off | written, back to 0 at the end |
| Stroke | `TRIANGLE_STRIP` | none |
| Triangles (image quads) | `TRIANGLES` | none |
| SetRenderTarget, RenderFilteredImage | an FBO over an RGBA texture; the blur uses an intermediate texture | none |

**Stencil and job splits.** Every stencil use starts and ends inside one command,
and the stencil is 0 again at the end of it. The frame starts with a stencil clear.
So the stencil never has to survive a job, unless a job splits in the middle of one
command (a full arena). The backend then refuses the next stencil read (`ZsKnown`,
"stencil is not kept between jobs yet"). That is correct but pessimistic: the
cover pass leaves the stencil at 0, but the backend counts it as `Written`. Whether
a femtovg or Slint frame ever splits is a board measurement (a refusal line would
say so).

**Textures femtovg creates.** A gradient with more than two stops: a 256x1 RGBA8
texture (`gradient_store.rs`), updated with `glTexSubImage2D`. An image: RGBA8
(`Rgba8`), RGB8 or alpha (`R8`), with `NEAREST`/`LINEAR`, `REPEAT`/`CLAMP_TO_EDGE`
and, when flagged, `glGenerateMipmap`. femtovg's own glyph atlas is not used here:
the no_std build has no font stack, and Slint draws text its own way (below).

### The two frames

**fvg_demo** (gl_femtovg's scene, `v3d_gl_femtovg`): ClearRect to rgb(24,24,32);
ConvexFill with FillColor (a rounded rect); ConvexFill with FillGradient (a
two-stop linear gradient: no texture); StencilStroke with FillColor (a 5-point
polyline, width 6). It uses three programs (fill_color without discard,
fill_gradient without discard, fill_color with discard), the stencil inside one
command, blending, and no texture.

**The Slint cluster** (`slint/ui/*.slint`, read from the source, not measured):
54 Rectangles (16 with a border radius: ConvexFill with FillColor, with strokes for
borders), a two-stop linear gradient background (FillGradient), 6 Paths (stroked
arcs and icons: StencilStroke), conic-gradient stroke brushes (FillGradientConic,
or FillImageGradientConic with a 256x1 texture above two stops), 12 Text elements
and 1 Image. The no_std Slint patch draws each Text item as an RGBA texture
composed on the CPU from bitmap glyphs (`bitmapfont.rs`), sized to the item, drawn
as an image fill (FillImage). The image (`slint-logo.png`) is decoded at build time
into an RGBA texture. The exact command counts and texture sizes of one cluster
frame are for `v3d_gl_slint` to print, not guessed here.

### What the V3D path refuses today

**The compiler.** The host measurement: the 29 fragment shaders of
`tools/shaders/femtovg/` with `main.vert`, through `lower::compile_to_ir` and
`v3d::chain::compile_program` (the chain `glLinkProgram` runs), by a local test
that is not committed. The first refusal of each shader, at two commits:

| first refusal | at 84bfd80a | at 3dd1e3a5 (Fsign landed, 4e94e5d4) | why femtovg has it |
|---|---|---|---|
| compiled | 2 | 2 | fill_color.nodiscard, stencil.nodiscard |
| `Discard` | 14 | 14 | every shader without NO_DISCARD but the texture copy (the stroke threshold test) |
| `Tex` | 7 | 9 | image, image-gradient (linear and conic), filter and texture copy without the discard, and FillColor with the glyph texture |
| `Fsign` | 4 | 0 | the conic gradients (`atan(y, x)`) |
| `gl_FragCoord` | 2 | 4 | the gradient dither (`ditherGradient`), linear and conic |

v3d-m5's count with its two local commits (discard a6510bf6, 2D texture look-ups
9cc937c3; not pushed, no board run yet), from its pinned test
`femtovg_outcomes_are_pinned`: 11 compile (every fill_color, fill_image and
stencil shader, and texture_copy_unclipped), 16 stop at `gl_FragCoord` (every
linear and conic gradient, plain and image), 2 at a loop (filter_image's blur).
Discard and Tex are then gone as first stops; femtovg's first program
(FillGradient) then stops at gl_FragCoord.

With Fsign in, the 4 conic shaders moved to their next stop: the two
FillGradientConic ones to `gl_FragCoord`, the two FillImageGradientConic ones to
`Tex`. Counted over femtovg's 30 programs in its order (TextureCopyUnclipped is
built twice), at 3dd1e3a5: 2 link, 14 stop at Discard, 10 at Tex, 4 at
gl_FragCoord. The first program femtovg builds (FillGradient with the glyph texture
and the discard) stops at `Discard`, so `fvg_draw_frame` returns 1 before any draw.
These are **first stops only**: what stands behind each one is unknown until it is
lifted.

**The board**, two runs by the lead, both on 2026-09-24. 8b450093 (the sample on
84bfd80a; image sha256 2283921d...): "femtovg programs: 2 of 30 link (120 ms)",
14 x Discard, 8 x Tex, 2 x gl_FragCoord, 4 x Fsign, "femtovg stops at FillGradient
glyph: ... NotYet("Discard")", "v3d_gl_femtovg: FAIL (fvg_draw_frame 1: femtovg's
programs do not all link (above))", the window left magenta. Count for count the
host numbers at 84bfd80a: the image's compiler and the host's agree on this
corpus. 97ec61f2 (the same sample on 3dd1e3a5, Fsign in; image sha256
bbfa6105...): "femtovg programs: 2 of 30 link (121 ms)", 14 x Discard, 10 x Tex,
4 x gl_FragCoord, "femtovg stops at FillGradient glyph: ... NotYet("Discard")",
"v3d_gl_femtovg: FAIL (fvg_draw_frame 1: ...)", build 97ec61f2, the window magenta:
the host prediction for 3dd1e3a5, count for count.

**Batch 7 (0f6ac998: discard, textures and gl_FragCoord in the compiler).** Host,
`femtovg_outcomes_are_pinned` (rc 0): 27 of 29 fragment shaders compile;
filter_image and filter_image.nodiscard stop at `NotYet loop` (the blur's `for`
with `break`). Over femtovg's 30 programs: 28 link, 2 do not; FilterImage is built
once per discard set, so it is 2 programs, and the one duplicate femtovg builds
(TextureCopyUnclipped) links. The board, `v3d_gl_femtovg` from 0f6ac998 (image
sha256 0444d4e2..., booted 14:33:57, read from the screen by the lead): "femtovg
programs: 28 of 30 link (126 ms)", "2 x link: online shaderc: the code generator
refused this fragment shader: Program(NotYet("loop"))" (FilterImage and FilterImage
nodiscard), "femtovg stops at FilterImage: ...", "v3d_gl_femtovg: FAIL
(fvg_draw_frame 1: femtovg's programs do not all link (above))", GL error 0. The
host prediction (28), count for count. The loop is now the only compiler need
between femtovg and its first frame.

The same run printed four lines "yakogl: program 14 has no uniform 'glyphtex'"
(and 16, 18, 20). They are harmless. femtovg looks up `tex`, `glyphtex` and `frag`
in every program it builds (program.rs, MainProgram::new). Before the FilterImage
failure it had built the 6 glyph programs and 4 programs without the glyph texture
(FillGradient, FillImage, Stencil, FillImageGradient, with discard). In those four,
`glyphtex` is declared and never read: its only read (main-fs.glsl:237) is inside
`#ifdef ENABLE_GLYPH_TEXTURE`. The reflection keeps a sampler only when the source
mentions it (`reflect.rs`, `mentions`; `uniform_rows` drops the rest), so
`glyphtex` is not an active uniform there, glGetUniformLocation answers -1, and
femtovg's glUniform1i(-1, 1) is ignored, as the spec says. `tex` never misses,
because every variant mentions it in functions it does not call (renderImage and
the others). The miss log's budget is 200 lines, so four lines are all the misses
there were.

The first textured draws on silicon (`v3d_gl_test`, 454a2971) hang the GPU (job
timeouts; glue, tex and m5 are on it). So when the loop lands, the first femtovg
frame can still stop at that hang, not at a wrong pixel.  The fvg_demo scene
itself samples no texture, but Slint's does. (The hang was closed on the board by
e1165634, batch 12.)

**Batch 14 (ba703983: the counted loop, and the survey holding its programs).** Host,
`femtovg_outcomes_are_pinned` (rc 0): all 29 fragment shaders compile. But femtovg
keeps all 30 programs alive, and the compiler's keep region cannot hold them: a
freed fragment-stage record leaves a 10,624-byte hole behind every program, and new
records always go to the largest free run, so each program costs about 24.5 KB of
the 512 KiB region while holding about 13.9 KB. Measured on the host: 21 programs
link and the 22nd (fill_gradient.nodiscard) fails, with 294,080 of 524,288 bytes
in use and the largest free run 10,624 bytes. The report for the store's owner is
`.claude/worktrees/report-online-store-keep-femtovg.md`. The survey (f13da183) now
holds its programs as femtovg does, so it names this in step 1: before, it freed
each program and would have shown 30 of 30. The board, `v3d_gl_femtovg` from
ba703983 (captured 17:59, read from the screen by the lead): "femtovg programs: 22
of 30 link (138 ms)", "8 x link: online shaderc: out of compiler memory on this
fragment shader (10624 bytes left in the keep arena); raise
shaderc::online::store::KEEP_BYTES", "femtovg stops at FillGradient nodiscard",
"v3d_gl_femtovg: FAIL (fvg_draw_frame 1: femtovg's programs do not all link
(above))", the window magenta. The first failure is the host's, program for program
and byte for byte. The total is 22, not 21, because the survey goes on after the
first failure (the host test stopped there), and one of the 8 later programs still
fits. No compiler refusal is left: the keep region is the one thing between
femtovg and its first frame (v3d-isa's proposal: placement by lifetime, and
KEEP_BYTES from a Kconfig).

The same run printed nine `glyphtex` lines (programs 14..30, even). They are the
nine programs without the glyph texture that femtovg built before it stopped, and
they are harmless, for the same reason as the four lines above.

**The keep region placed by lifetime (10bcfe1a, v3d-isa's proposal, measured
before it lands; the user's decision, 2026-09-24).** Built on 10bcfe1a with the
default `CONFIG_YAKOGL_SHADERC_KEEP_BYTES` = 524288, so only the placement is
measured; the Zephyr side (the region lent at SYS_INIT) compiles and links for an
image. Host, femtovg's 30 programs held at once: 30 of 30, keep in use 388,544
bytes. The board, `v3d_gl_femtovg` from 7953742b (10bcfe1a plus the samples' "keep
used" line; image sha256 9849b2ca..., captured 18:26:31, read by the lead):

- "femtovg programs: 30 of 30 link (128 ms)".
- **"v3d_gl_femtovg: PASS (a FemtoVG frame through YakoGL, every pixel read
  back)"**: the first femtovg frame on the V3D. "window 460800 px: rrect 47354
  grad 50232 stroke 2944 bg 334394 edge 25876"; rrect, stroke and bg wrong 0
  (largest channel difference 0), grad wrong 0 (largest 1: the dither); guard
  row 0 of 1280 changed; the four glReadPixels equal memory. The screen shows
  the scene.
- "frame: 142 ms, GL error 0x0 after the draw, 0x0 after the swap"; "job arenas
  high-water at frame 1: BCL 819 of 65536, RCL 463 of 4096, indirect 3008 of
  262144, data 0, textures 0, 7 draws".
- "keep used 2432 (peak 401536) of 524288 bytes". The line was labelled "with
  femtovg's canvas alive", which was false: fvg_demo's canvas and renderer are
  locals of `fvg_draw_frame`, so its 30 programs are deleted when the line prints
  (fixed in 9698f967). The 2,432 bytes are the backend's clear program (measured
  on the host: its program record alone is 2,432 bytes once its vertex stage is
  released). The peak is 388,544 (the 30 programs) + 2,432 (the clear program) +
  10,560 (the last fragment-stage record, alive while its program links) =
  401,536, to the byte.

**What the PASS does not cover.** The check left out every anti-aliased edge
("edge", 25,876 pixels). The first Slint frame (below) lost a stroke's fringe, so
since 01913b2d the check also judges two fringes: `sfringe`, the stroke's (2.7-3.3
px from its centre line, femtovg's alpha 0.8..0.2), and `rfringe`, the rounded
rect's (a plain convex fill: the control). Each must be drawn partly, strictly
between the background and its colour.

**The backend** (`src/v3d/state.rs` `refusal`, `backend.rs` `record`): "textures
are not sampled yet" for any draw with a texture bound; the stencil or depth read
after a job split (above). The window is RGBA8888, so the target-format refusals do
not apply. FBO targets over RGBA8 textures are drawn, not refused. Slint uses them
only for layers and blurs, and the cluster has neither.

### What each frame needs, and who is building it

| need | femtovg demo | Slint cluster | owner |
|---|---|---|---|
| Discard in the compiler | yes (the stroke, and 14 programs at init) | yes | v3d-m5 (its next commit) |
| Texture sampling (Tex, TMU state, tiled images) | 10 of the 30 programs, at init only | yes: every Text item, the image, gradients above two stops | v3d-tex, v3d-glue, v3d-m5 (in progress) |
| `Fsign` (atan) | 4 programs at init only | yes: the conic arcs | landed (4e94e5d4) |
| `gl_FragCoord` (with Mesa's `FB_WPOS_Y_TRANSFORM` state uniform, see tools/v3d/README.md) | yes: the gradient rect, and 16 programs at init once discard and textures are in | yes: the background, the conic arcs | v3d-m5, next after discard and textures |
| A loop (filter_image's blur, `for` with `break`) | 2 programs at init only | 2 programs at init only (the cluster has no blur) | v3d-m5, after gl_FragCoord |
| Stencil inside one job | yes | yes | drawn today (M6) |
| No job split inside a stencil command | measure | measure | board |

Because femtovg builds every program at canvas creation, the femtovg frame needs
all of these compiler parts, even the ones its scene does not draw with.

### The sample `v3d_gl_femtovg`

`driver/samples/zephyr/v3d_gl_femtovg`: the board support of `v3d_gl_triangle`
(the mailbox framebuffer, the HDMI console in the top half, the watchdog), the EGL
window in the bottom half (1280x360, RGBA8888, stencil 8), `gl_femtovg/rust` built
here by path, and femtovg's own shader text embedded from `FVG_NOSTD_DIR` by CMake.

1. **The programs.** Before the frame, the sample builds femtovg's 30 programs
   through GL in femtovg's order and with its source layout, and prints each one
   that does not link, with its info log. It also prints a histogram keyed on the
   whole log, and the program femtovg stops at. (`fvg_draw_frame` only returns 1.)
   The programs are deleted again before femtovg builds its own.
2. **The frame.** `fvg_draw_frame`, `eglSwapBuffers`, `glGetError`, then every
   pixel of the window in one of five classes: inside the rounded rect, inside
   the gradient (the mixed colour within 2: femtovg dithers by half a step),
   on the stroke's centre line away from joins and ends, the background, and the
   edge band (counted, not checked). Alpha is 255 in every checked class. Then the
   guard row above the window, and four `glReadPixels` reads against memory.
   Every class must be non-empty, and the classes must add up to the window.

Build (the no_std workspace recreated from `slint/nostd/README.md`, steps 2
and 3, under the worktree's gitignored `build/`):

```sh
cmake -GNinja -S driver/samples/zephyr/v3d_gl_femtovg -B build/v3d_gl_femtovg \
      -DBOARD=rpi_4b -DPython3_EXECUTABLE=$(which python3) -DFVG_NOSTD_DIR=$PWD/build/nostd
ninja -C build/v3d_gl_femtovg
```

0 C warnings (the Rust warnings come from the femtovg and glow forks, as in
`gl_femtovg`). QEMU smoke run (`-DV3D_RPI_WDOG=OFF -DCONFIG_AARCH64_IMAGE_HEADER=y`,
`-M raspi4b`): the image boots, prints its build, and stops where a machine with no
V3D must: "osal(v3d): power-on failed (-19)", `eglInitialize: 0x3001`,
"v3d_gl_femtovg: FAIL (EGL did not initialise ...)". On the board, step 1's
histogram equals the host's (above), and the frame stops at `fvg_draw_frame 1`
until all 30 programs link.

### Order

1. Done: the board run of `v3d_gl_femtovg` as it is (step 1's numbers, above).
2. As Discard, Tex and gl_FragCoord land, re-run step 1 until 30 of 30 link, then
   the frame: the first draw refusal or wrong pixel is the next measurement.
3. `v3d_gl_femtovg` PASS, then `v3d_gl_slint` (below; built and waiting). Its
   step 1 is the same 30 programs, so it cannot pass before femtovg's programs link.

### The sample `v3d_gl_slint`

`driver/samples/zephyr/v3d_gl_slint`: `gl_slint`'s Rust crate and UI by path, the
no_std Slint workspace (`SLINT_NOSTD_DIR`) and the femtovg/glow forks
(`NOSTD_PATCH_DIR`) recreated from `slint/nostd/README.md`, and the board support
of `v3d_gl_triangle`. **Layout** (the lead, 2026-09-24): the SafeUI repository has
no Pi 4 panel directory, so the Pi 4 uses the AM62P's `ui/1024x600`, drawn into a
1024x600 EGL window. The frame is then the AM62P's, and its reference pixels apply.
The window is a buffer of its own in the image (tightly packed, followed by 8 guard
rows), as `gl_slint`'s is on the AM62P, and is copied to the top left of the 1280x720
firmware framebuffer after the check, for the eye; the console is the 256-pixel
column right of it.

Why a buffer of its own: YakoGL's native window must have tightly packed rows
(`src/gl/egl.rs`, `GlesNativeWindow` and `native_buffer_format`: `stride_bytes ==
tex_pitch(width) * bpp`), on every GPU. The first version drew straight into the
framebuffer, a 1024-pixel window with the framebuffer's 5120-byte pitch, and the
board run of 95d0cc6c stopped at `eglCreateWindowSurface` (EGL_BAD_NATIVE_WINDOW)
before step 1. The sample printed "EGL surface/context: 0x3000" because it read
`eglGetError` after `eglCreateContext` had succeeded: the error of the last call,
not of the failed one. Both samples now check every EGL call on its own.

**The board runs** (the lead, 2026-09-24):

| build | what it showed |
|---|---|
| 95d0cc6c | stopped at `eglCreateWindowSurface` (the pitch, above) before step 1 |
| ef8f4211 | the EGL fix holds: step 1 as `v3d_gl_femtovg` prints it ("2 of 30 link (121 ms)", 14 x Discard, 10 x Tex, 4 x gl_FragCoord, stops at FillGradient glyph); then `slint_zephyr: rust panic: .../internal/renderers/femtovg/opengl.rs:118:14: called Result::unwrap() on an Err value: ShaderLinkError("online shaderc: the code generator refused this fragment shader: Program(NotYet("Discard"))")`; no verdict line; the watchdog returned the board |
| 0f6ac998 (batch 7) | "GL_RENDERER Broadcom V3D 4.2 (YakoGL), GL_VERSION OpenGL ES 3.0 YakoGL"; "femtovg programs: 28 of 30 link (127 ms)", 2 x NotYet("loop") (FilterImage and FilterImage nodiscard), "femtovg stops at FilterImage"; "v3d_gl_slint: FAIL (femtovg's programs do not all link (above): the frame is not attempted, because Slint's renderer panics on a link error)" (sha256 6fd831e9..., booted 14:40:56, read from the screen by the lead) |
| ba703983 (batch 14) | "v3d_gl_slint: build ba703983"; "femtovg programs: 22 of 30 link (135 ms)", "8 x link: online shaderc: out of compiler memory on this fragment shader (10624 bytes left in the keep arena)" (FillGradientConic nodiscard and FillImageGradientConic nodiscard among them), "femtovg stops at FillGradient nodiscard"; "v3d_gl_slint: FAIL (femtovg's programs do not all link (above); the frame is not attempted, because Slint's renderer panics on a link error)"; magenta window, no panic (captured 20 s after the 18:05:46 boot). An earlier capture of the same image at 60 s showed no HDMI signal: the watchdog reset window (it is fed for RPI_WDOG_HOLD_S = 50 s, then fires within one period, at most ~16 s), and Linux was back within a minute |
| 7953742b (10bcfe1a, the keep region placed by lifetime) | **the SafeUI cluster on the screen** (gauges, the Slint logo, drive info, the power bar, fuel): the first Slint frame on the V3D. "femtovg programs: 30 of 30 link (127 ms)"; "keep used 390976 (peak 401536) of 524288 bytes with Slint's renderer alive" (388,544 + the clear program's 2,432); "UI heap: 329160 bytes now, 686824 at most, of 50331648"; "job arenas high-water at frame 1: BCL 55395 of 65536, RCL 583 of 4096, indirect 188557 of 262144, data 0, textures 12664 of 8388608 bytes, 493 draws"; "swap: 32 ms, GL error 0x0"; "window 1024x600: 0 sentinel pixels left, 0 with alpha != 255, crc32 0x7ea0e569"; "guard rows 0 of 8192 pixels changed"; four of the five AM62P points within 1 (bg-top-left 11,15,23; face-left 9,12,18; face-right 8,11,17; bg-bottom 4,6,10); **"track (47,330) = 8,11,17: AM62P 13,17,24 (diff 7), host 12,16,23 (diff 6) WRONG"**; "v3d_gl_slint: FAIL (the AM62P reference points)" (sha256 47bc7c06..., captured 18:28:05, read by the lead) |

Every femtovg program must link, or Slint panics; the panic is bounded by the
watchdog, and since 07b3acc7 the sample does not reach it: it stops after step 1
with a named FAIL. The 0f6ac998 run shows that FAIL working, with the same survey as
`v3d_gl_femtovg` on the same build, count for count. Since batch 14 every
femtovg shader compiles (the loop landed, and the textured-draw hang was closed in
batch 12), and the ba703983 run shows the one thing left: the compiler's keep region
holds only 22 of the 30 programs femtovg keeps alive (above, "Batch 14").
With the keep region placed by lifetime (10bcfe1a), all 30 link and the first Slint
frame is drawn (7953742b).

**The track point.** 8,11,17 is exactly the dial face's colour (face-left and
face-right read it), so nothing was drawn over the face at (47,330). The host
recorder's coverage probe (`host/slint_gl_rec.c`, 998e5188) lists the draws of
the reference frame that cover that pixel centre: the background gradient, the
dial face (FillColor 8,11,17), femtovg's three-pass stroke of the track in
20,26,34 (draw 144: the discard program, threshold 0.998, stencil EQUAL 0 / INCR;
145: the anti-aliased pass, stencil EQUAL 0; 146: the stencil clear, colour mask
off), and a transparent text texture. The AM62P's 13,17,24 is about 42 % of the
track over the face: the stroke's fringe, which only draw 145 writes. So the point
lies on the stroke's edge by design, and on the V3D the fringe pass wrote nothing
there.

The cause, from Mesa (mesa-26.2.1), measured on the board below: a shader that discards keeps early
Z/stencil only when the depth-stencil state writes neither depth nor stencil
(`v3d_program.c:627-631`, `can_earlyz_with_discard`); otherwise it gets no early
fragment tests, `writes_z` (the shader record's "fragment shader does Z writes")
and `disable_ez` (`nir_to_vir.c:2076-2078, 2097-2098`; `v3dx_draw.c:560-575`:
"Must be set if the shader modifies Z, discards, ..."). Our compiler always keeps
early Z with discard (`src/shaderc/online/v3d/chain.rs:138-140`, "as v3d_shc's key
has it": the host tool's key has no stencil writes), pinned at chain.rs:1769-1783.
Then draw 144's stencil INCR lands on the fringe fragments the shader discards,
and draw 145's EQUAL 0 finds no fringe to draw: strokes keep their cores and lose
their anti-aliased edges.

**Measured** (`v3d_gl_femtovg` 998e5188, with 01913b2d's fringe classes, on the
board at 18:44, read by the lead): "sfringe not drawn partly 1188 of 1188 (largest
distance from bg 0)": every stroke fringe pixel is exactly the background.
"rfringe not drawn partly 0 of 136 (largest distance from bg 159)": the convex
fill's fringe, the control, is drawn. "first wrong (sfringe) at 370,198:
0xff201818" (the background). The rest as before: rrect, grad, stroke and bg wrong
0, 30 of 30 link, frame 142 ms, GL error 0; "v3d_gl_femtovg: FAIL (the stroke's
anti-aliased fringe is not drawn partly)". The whole fringe pass of a stencil
stroke is missing, and nothing else is, as the mechanism predicts.

The mechanism, refined by v3d-glue: it is not early Z, which our jobs already turn
off for the whole job (`driver/src/v3d/v3d_job.c:647`, the RCL's
`early_z_disable`). It is the implicit depth/stencil write. Without "fragment
shader does Z writes" in the shader record, the tile buffer updates Z and S from
the rasteriser's value even for fragments the shader discards; Mesa's comment at
`nir_to_vir.c:2081-2086`: "By default, Z buffer writes are implicit using the Z
values produced from FEP ... When this is not desirable (shader writes Z
explicitly, has discards, etc) we need to let the hardware know by setting
c->writes_z to true, in which case we always need to write a Z value from the
QPU, even if it is just the passthrough Z value produced from FEP." The fix v3d-glue
proposed to v3d-m5: a discarding fragment shader always writes the pass-through Z
through the TLB (`writes_z` and `disable_ez` true), for every draw, with no
variant. The backend already copies both bits into the shader record
(`v3d_job.c:429-431`). v3d-m5 implements it and v3d-glue pins the record bits; the
same edge image re-ran on the fix, and the Slint frame with it (below).

**The fix, measured** (v3d-m5's aa391d2f: a discarding fragment shader runs with
late tests and writes the pass-through Z through the TLB, `writes_z` = 1,
`disable_ez` = 1; with the samples' fringe checks, build 1b045956). Host:
`femtovg_outcomes_are_pinned` rc 0, 29 of 29 compile; femtovg's 30 programs held
at once keep 386,304 bytes, 2,240 fewer than on 10bcfe1a (the discard programs
changed). The board, `v3d_gl_femtovg` at 20:21 (read by the lead): "sfringe not
drawn partly 0 of 1188 (largest distance from bg 173)", "rfringe not drawn partly
0 of 136 (largest distance from bg 159)", rrect, grad, stroke and bg wrong 0,
"30 of 30 link (127 ms)", frame 142 ms, GL error 0, "keep used 2432 (peak 399296)
of 524288 bytes after fvg_draw_frame (its canvas already dropped)" (the peak is
2,240 lower, as the host number is), **"v3d_gl_femtovg: PASS"**.

**The arenas at 493 draws.** The job holds two Slint frames (the build frame and
the reference frame, drawn before one swap; the recorder has 237 + 250): 112 B of
BCL and 382 B of indirect per draw. A steady-state frame (250-265 draws) is about
30 KB of BCL and 101 KB of indirect, and a job would split at about 585 draws
(BCL) or 686 (indirect): correct since step C, only slower. The 1280x800 layout
has the same items, so about the same draws.

### What the Slint frames ask of the backend (host-measured)

`driver/samples/zephyr/slint/host/slint_gl_rec.c` drives `gl_slint`'s own Rust
archive, built for x86_64 from the same crate, UI and trees as the rpi_4b image, as
`v3d_gl_slint` drives it. Its GL entry points are a recorder that keeps femtovg's GL
state and logs every draw. Nothing is rendered, and every compile and link reports
success, so this measures the GL call stream the backend receives, not pixels. The
run (505e90d3's tree): the first frame (the component build, then `demo_vehicle` at
t = 0), then 1295 frames of `gl_slint`'s scripted drive at 58 fps, all rc 0. glow
resolves 512 more names than the recorder has; none was called (a call to an
unresolved name panics in glow, and the run finished).

| | frame 1 + 2 (one job in `v3d_gl_slint`) | the drive, per frame |
|---|---|---|
| draws | 237, then 250 (4658 / 4907 vertices) | 250..265 |
| programs | FillColor 136-141, FillImage 108, FillGradient 1 (the background), FillColor with discard 5 | the same, plus FillImageGradientConic 4-12 (the moving arcs; 1/3 of them with discard) |
| textures sampled | 54 distinct (108 draws: each image fill is a fill and an AA strip) | 56-58 distinct |
| stencil | 6 / 15 draws with the test on, 4 / 10 write it (StencilStroke) | the same pattern |
| clears | **none**: the background is a FillGradient fill | none |
| job ends | only the swap: no target switch, no flush, no read-back | only the swap |
| uploads | 54 new textures, 1,677,236 bytes of data (60x30 scale numbers, 400x21..400x147 texts, a 330x132 logo) | below |
| vertex buffer | one `glBufferData`, 66,976 / 70,192 bytes | one, ~70 KB |

New textures per frame in the drive, by size (data bytes; each is `glTexImage2D` with
no data, then one full `glTexSubImage2D`):

| phase | new textures per frame | data per frame | per second at 58 fps |
|---|---|---|---|
| idle (0-3 s) | 256x1 x2 (the arcs' gradients), 320x21 (a frame counter), 400x65 (rpm), 68-77x21 (the other counter), ... | 177,182 bytes | 10.3 MB |
| accelerate (3-15 s) | the above, 256x1 x4, and 400x147 (speed) every frame | 417,411 bytes | 24.2 MB |
| cruise (15-22 s) | 256x1 x2, the counters, ... | 139,315 bytes | 8.1 MB |

The largest frame is 759,684 bytes (frame 356, accelerating). The estimate I sent
v3d-tex from the source (about 6 MB/s) was 4x low: the arcs' gradient textures and
the frame counters are new in every frame, and rpm and speed change every frame
while they animate. v3d-tex has the measured numbers.

**What this asks of the backend:**

1. **Stencil across eglSwapBuffers.** Slint never clears the stencil. femtovg returns
   it to 0 at the end of every command that writes it, and relies on it starting
   at 0. Today `flush_job` records the stencil as `Written` after a job whose
   strokes wrote it. The next job then refuses the first draw that reads the
   stencil ("the draw reads stencil an earlier job wrote ..."): that is the first
   stroke of every frame after the first swap. EGL 1.5, section 3.10.1 ("Posting to
   a Window"): "The contents of ancillary buffers are always undefined after calling
   eglSwapBuffers." So after a swap the window's depth and stencil may be taken as
   `Untouched`: the next job starts them from the tile buffer's clear (0), which is
   what femtovg needs, with no store or load. Step C (keeping depth/stencil between
   jobs) is needed only for a split INSIDE a frame, which the measured stream never
   asks for (no target switch, flush or read-back mid-frame). The one remaining way
   it can split is a full arena.
2. **The arenas for ~265 draws.** BCL is 64 KiB, IND 256 KiB, DATA 1 MiB and TEX
   8 MiB per job, and the first cut writes the whole state for every draw. Whether
   265 draws with 58 sampled textures fit in one job, without a mid-frame split
   (which then refuses the next stencil read), is for the backend's owner to answer
   from its bytes per draw. It is not measured here.
3. **Textures:** 56-58 RGBA8 textures sampled per frame (one sampler, unit 0; the
   glyph unit is never bound: Slint's no_std text has no glyph atlas), sizes 256x1
   to 400x147 plus the 330x132 logo, all NPOT, `CLAMP_TO_EDGE`, `LINEAR` or `NEAREST`,
   no mipmaps; 5-9 created and about as many deleted per frame.
4. **Programs:** FillColor (with and without discard), FillImage, FillGradient and
   FillImageGradientConic (with and without discard) are the ones drawn. femtovg
   still builds all 30 at canvas creation.

**The answers** (v3d-glue, 2026-09-24; their commits, not verified here beyond
reading their note):

1. Stencil across the swap: 315a0b47. `eglSwapBuffers` calls a new backend hook,
   `surface_swapped`, before its flush. The V3D drops the window's depth/stencil
   history, so the next frame starts from the tile clear (stencil 0). The negative
   control: without a swap, the read is still refused. Step C (c2edfb2d) also keeps
   depth/stencil in memory across a split inside a frame, so a mid-frame split is
   now correct too, at the cost of a store and a load.
2. Arenas, estimated from job_check's three-draw job (about 125 B of BCL and
   220 B of indirect per draw, plus about 240 B for femtovg's 14-vec4 uniform
   block): 265 draws is about 33 KB of BCL (limit 64 KiB) and about 125 KB of
   indirect (limit 256 KiB), so one job. The tiled textures live across jobs (the
   (B) cache, 1f0061e3: 64 entries, 8 MiB). v3d-glue will print each arena's
   high-water mark on the board, which replaces the estimate once a frame runs.
3. The vertex buffer: `glBufferData` goes to a buffer object in V3D memory, read
   in place; only client-side arrays are staged into DATA. The 70 KB per frame is
   not copied per draw.
4. Texture creation: the lead decided to tile at upload. The bench: the
   non-cacheable read is the whole cost; a cacheable source gives 228 KiB/ms. The
   Slint text pixels are in the cacheable UI heap, and every upload is the whole
   texture (the recorder: 0 sub-boxes in 9503 `glTexSubImage2D`, 963c909a).
   v3d-glue and v3d-tex implement it after the textured-draw hang is closed.

1. **The programs**: the same 30 as `v3d_gl_femtovg`, built from the femtovg tree
   the archive builds (`v3d_gl_femtovg/fvg_programs.cmake` and
   `src/fvg_programs.c`, shared by both samples). Slint's renderer creates a
   femtovg canvas and **unwraps** the result
   (`internal/renderers/femtovg/opengl.rs:118` of the no_std Slint tree), so on the
   V3D a program our compiler refuses is a Rust panic inside `slint_draw_frame`,
   not a return code: the board ends at the watchdog with no verdict. So the
   sample stops after step 1 when any program fails, with a named FAIL ("femtovg's
   programs do not all link (above); the frame is not attempted ...").
2. **The frame**: `gl_slint`'s first frame. `slint_draw_frame` builds the
   component and draws its default state; `slint_set_vehicle` with
   `demo_vehicle` at t = 0; `slint_draw_frame` again; `eglSwapBuffers`. The GL calls
   Slint makes are counted per call through the get-proc hook: draws and vertices,
   clears, stencil enables, program binds, FBO binds, `glTexImage2D` (count, bytes,
   largest size), `glTexSubImage2D`, mipmaps, buffer uploads. They are printed, not
   judged: this is the cluster frame's list of needs, measured, which the plan
   above only read from the source. The sample also prints how many of the 11
   counted names Slint resolved, because a zero from a name it never asked for
   says nothing, and the UI heap's high-water mark.
3. **The check**, against the AM62P: the five points `gl_slint` reads back, each
   within 1 of the AM62P's own reading (tools/cts/RESULTS.md, 2026-09-23, where the
   AM62P is within 1 of a host render at all five); no pixel of the window left at
   the CPU's sentinel; alpha 255 everywhere; the guard band unchanged; and
   `glReadPixels` equal to memory at the five points. The window's CRC-32 is printed
   as a fingerprint, for a later comparison with an AM62P frame; it is not a
   verdict. The check reads the window's own buffer through a Normal
   non-cacheable mapping (as the OSAL maps its carve-out), so no cache
   maintenance stands between the GPU's writes and the CPU's reads.

Build: add `-DSLINT_NOSTD_DIR=<slint-nostd/slint> -DNOSTD_PATCH_DIR=<nostd>
-DSAFEUI_DIR=<SafeUI checkout>` to the `v3d_gl_femtovg` line. A worktree has to name
`SAFEUI_DIR`: the sibling default does not resolve from inside `.claude/worktrees/`.
RAM 124,924 KB of 144 MB (sram0 is 144 MiB here: the 48 MiB UI heap and the
2.4 MiB window are added to `v3d_gl_triangle`'s 96 MiB). 0 C warnings. QEMU smoke run: the window stays magenta,
the guard band red and the console column readable, and the run stops where a
machine with no V3D must ("osal(v3d): power-on failed (-19)", then "v3d_gl_slint:
FAIL (EGL did not initialise ...)").

### The milestone: the SafeUI cluster through YakoGL on the Pi 4

`v3d_gl_slint`, build 1b045956 (aa391d2f on a60104c2, plus 8e942583 and 1b045956;
all in batch 18), image sha256 0a211b60d51a3d15fc83e4a6637daf859da8106107d334e3486b8668e3940308
(3,817,344 bytes), on the Raspberry Pi 4 at 20:23 on 2026-09-24, read from the
HDMI screen by the lead:

| check | result |
|---|---|
| verdict | **"v3d_gl_slint: PASS (the AM62P's cluster frame through YakoGL on the V3D)"** |
| femtovg's programs | 30 of 30 link (128 ms) |
| track (47,330) | 13,17,24: AM62P 13,17,24 (diff 0), host 12,16,23 (diff 1) |
| bg-top-left (40,20) | 11,15,23: AM62P 11,16,23 (diff 1) |
| face-left (214,330) | 9,12,18: AM62P 9,12,18 (diff 0) |
| face-right (810,330) | 8,11,17: AM62P 8,11,17 (diff 0) |
| bg-bottom (300,590) | 4,6,10: AM62P 4,6,10 (diff 0) |
| window 1024x600 | 0 sentinel pixels left, 0 with alpha != 255, **crc32 0x76cf3606** |
| guard rows | 0 of 8192 pixels changed |
| GL | swap 31 ms, GL error 0x0 |
| job arenas (high-water, frame 1) | BCL 55,395 of 65,536, RCL 583 of 4,096, indirect 188,781 of 262,144, data 0, textures 12,664 of 8,388,608 bytes, 493 draws (two Slint frames in one job) |
| UI heap | 329,160 bytes now, 686,824 at most, of 50,331,648 |

The CRC-32 is the window's fingerprint at this build: a later run of the same
frame on the same stack can be compared with it. It was 0x7ea0e569 before the
discard fix, when the strokes had no fringes. It is not a verdict, and a change
anywhere in the stack that moves a pixel moves it.

**The CRC moved with no pixel-level change: Slint's clock.** The same frame on
main 5320acfc gave crc32 0x9969bcd4, with the same verdict and all five points
within 1. The cause is the clock the sample gave Slint, not the GPU. Slint starts
the dashboard's property animations at the clock it reads while drawing frame 1.
Frame 2 shows the state 150 ms or more after that clock as settled, and anything
under 150 ms as a point inside the animation. Measured with the host recorder
(`host/slint_gl_rec.c`: frame 1 at `SLINT_REC_T1`, the new state set at
`SLINT_REC_TSET`, frame 2 at `SLINT_REC_T2`; digest of frame 2):

| frame 1 | state set | frame 2 | frame 2 digest |
|---|---|---|---|
| 0 | 0 | 0 | 0x12b0751406fe2ac3 (the animations' start) |
| 1000 | 1000 | 1000 | 0x12b0751406fe2ac3 |
| 0 | 0 | 22 | 0x39bc0a9d1c80343b (inside the animation) |
| 165 | 165 | 187 | 0x39bc0a9d1c80343b |
| 165 | 187 | 187 | 0x39bc0a9d1c80343b |
| 0 | 0 | 165 | 0x5b1546a09a86b205 (settled) |
| 165 | 165 | 315 | 0x5b1546a09a86b205 |
| 165 | 165 | 1000 | 0x5b1546a09a86b205 |

Only the gap from frame 1's clock to frame 2 counts. When the state is set does
not (165/187/187 equals 165/165/187). On the board the clock was the uptime.
Frame 1 takes about 165 ms (canvas and program build), and Slint reads the
clock late in it. Frame 2 then follows about 22 ms later, inside the
animation, so a few milliseconds of jitter move the picture and the CRC.

Since 64a7e9f5 (in this document's history: c68472ef, 60ee3280, 5c432977) the
sample gives Slint its own clock: frame 1 at 0 ms and frame 2 at 1000 ms
(`UI_CLOCK_FRAME2_MS`), so frame 2 is the settled state on every boot. Two boots
of 64a7e9f5 on the board, read by the lead, gave the same lines:
"v3d_gl_slint: PASS", all five points within 1, "frames: 1 165 ms, 2 23 ms"
(22 ms on the second boot), "swap 31 ms (Slint clock 0 / 1000 ms)", 493 draws,
binner tile pool 282,624 of 663,552 bytes, and **crc32 0x7b079d2c** on both.
**0x7b079d2c is the reference CRC from 64a7e9f5 on**; 0x76cf3606 and 0x9969bcd4
were two frames from inside the animation.

**The per-batch sample regression (from batch 22 on).** After each landed batch
both samples are built from origin/main (v3d-slint) and run on the board (the
lead). The host now logs the Pi 4's UART. The checks are the same as above: the
verdict, crc32 0x7b079d2c, 30 of 30 programs, and femtovg's pixel classes.

- **The UART carries the samples' lines since 19644131.** fbcon_attach()
  installs its own printk hook in place of the UART's. So up to batch 22, the
  UART got only the lines before it: the Zephyr banner and "shader compiler
  scratch". The samples now keep the earlier hook and write each character to
  both, as v3d_deqp does. cb85a1ba adds progress lines, so a hang stops after a
  named step: each program's link, "frame N, draw M of the frame" every 64 draws,
  and "swap start (N draws in the job)" / "swap done".
- **Times with the UART are not the old times.** The UART is polled at 115200 8N1
  (86.8 us a character). Inside eglSwapBuffers, YakoGL prints 4 lines (623
  characters: the BASELINE, the binner pool, the job arenas and the tile pool)
  after the job. Inside Slint's frame 1 it prints 22 lines (1,073 characters).
  The first run with the UART (41719f83) read swap 82 ms and frame 1 237 ms,
  against 31 and 165 ms. That looked like a regression but was the UART: 54 ms
  and 93 ms of it. Since e5d26b5b, the samples add up the UART's cycles and print
  each timed step without them.
- **Measured, batch 23** (8f2c90a3 + the four sample commits, d12dda3e): both
  PASS. Slint: crc32 0x7b079d2c, 493 draws in the job, "frames: 1 254 ms, 2 41 ms,
  swap 83 ms" and "without the UART: survey 138 ms, frame 1 169 ms, frame 2
  26 ms, swap 33 ms (the UART took 99, 85, 15, 50 ms)". Those match 64a7e9f5's
  129 / 165 / 23 / 32 ms, re-measured on the same board the same night.
  femtovg: every class wrong 0; "without the UART: survey 147 ms, frame 149 ms".
  Compare these "without the UART" times from batch to batch.

What it took, in order: the EGL window with packed rows (ef8f4211); discard,
textures, gl_FragCoord and the counted loop in the compiler (batches 5-14); the
keep region placed by lifetime (v3d-isa, 10bcfe1a, batch 16); the stencil reset at
eglSwapBuffers (v3d-glue, 315a0b47); and a discarding shader's pass-through Z
write (v3d-m5, aa391d2f, batch 18). Main reproduces it once batch 18 is pushed.

What it does not cover yet: one frame, not the animated drive (the host recorder
measured 250-265 draws and 139-417 KB of new textures per drive frame; texture
upload speed is v3d-tex's and v3d-glue's next item); and five reference points plus
a CRC, not a full-frame comparison with an AM62P frame.

## M8: dEQP-GLES2 on the Pi 4 (plan)

Goal: numbers for the "Raspberry Pi 4 (V3D)" column of `tools/cts/STATUS.md`, from
dEQP-GLES2 run on the board, on Zephyr, through YakoGL's V3D backend and the
compiler in the image. Status 2026-09-24: on the board (images built at eddf6ab7),
`info` 6/6, `functional.prerequisite` 3/3 and `functional.color_clear` 19/19, with no
refusal; each record was read from one photo (tools/cts/RESULTS.md).

### The sample: `driver/samples/zephyr/v3d_deqp`

- **dEQP itself is the deqp sample's**, compiled from its directory by path and
  never edited: `main.cpp` (the `tcu::App` loop, `CONFIG_DEQP_CASE`, a
  `CONFIG_DEQP_SURFACE_SIZE` pbuffer), `deqp_archive.cpp` (the embedded test data),
  `deqp_log.c` (the QPA log turned into one `deqp: <StatusCode> <case>` line per
  case) and `deqp_stubs.c`. The Kconfig sources the deqp sample's Kconfig for
  those options. The CMake list of the stage-1 archives is the one duplicated part.
- **Stage 1** is `tools/cts/cts-build-zephyr.sh` with `BOARD=rpi_4b`, unchanged.
  It takes the compile flags from the deqp sample configured for `rpi_4b`
  (`-mcpu=cortex-a72`, that configuration's `autoconf.h`); the POSIX and C++ lines
  of our `prj.conf` are the deqp sample's, line for line, so the headers match.
  Built once: 418 of 418 steps.
- **The board side** is `v3d_clear`'s sources (mailbox framebuffer, HDMI console,
  watchdog), as `v3d_gl_triangle` and `v3d_gl_test` use them.
- **The EGL default display.** dEQP's YakoGL platform asks for
  `EGL_DEFAULT_DISPLAY`. YakoGL hands the native display to the OSAL's init as is,
  and the V3D OSAL needs the board's clock switch there
  (`struct osal_zephyr_v3d_init`; with NULL it refuses to start). The sample links
  with `--wrap=eglGetDisplay` and `--wrap=eglGetPlatformDisplay`: on this board
  the default display is the V3D with this clock switch. Nothing in YakoGL or
  dEQP changes.
- `CONFIG_YAKOGL_REPORT_ES2=y`: dEQP-GLES2 applies the ES 2.0 rules only to a
  context that reports ES 2.0.
- Memory: 256 MiB from 0x200000 (the overlay). The image uses 145 MB: the 48 MiB
  carve-out, the 64 MiB C heap, dEQP's code and its 1.1 MB of test data.

### The record: one photo

The console is the HDMI screen (1280x720, 44 usable rows of 160 columns). During
the run the lines scroll. At the end the screen is cleared and a record is printed
once, then frozen (nothing else is drawn) until the watchdog reset:

```
v3d_deqp DONE: build <commit>, filter 'dEQP-GLES2.functional.color_clear.*'
  19 cases in 7600 ms, record at 37 s after boot, reset at ~100 s
  totals: P 10 F 6 R 1 N 1 Q 1  (R: not passed, logged a refusal; passed but logged one: 1)
  done: ... / summary: ... / dEQP: FATAL ERROR: ... (when there is one)
case order: P pass F fail R refused N not supported Q quality C compat E resource ...
    0 FPPRPPFQPFPNFPPFPPF
refusals: 2 draws not recorded
  r0 (first #3) a colour format with fewer than 8 alpha bits (RGB, RGBA4, RGB5_A1) is not written yet
    3 R masked_rgba                          r0 r?      6 F ...
v3d_deqp: build <commit> DONE: 19 cases, P 10 F 6 R 1 N 1 other 1
```

- One letter per case, in the order dEQP ran them (128 to a row, the index at the
  left). The order is the case list's, so a letter maps back to a name.
- **A refusal is its own outcome.** A case is R when it did not pass and the V3D
  backend refused a draw during it. That is read from the backend's own count,
  `v3d_refused_draws()` (`src/v3d/backend.rs`), at every verdict: the count grew across
  the case. The log's `yakogl/v3d: draw refused: <why>` lines (one per cause per boot)
  only name the causes. They are listed once, numbered, with the first case that hit
  each one. A refused case whose cause was logged earlier is tagged `r?`.
- **A refused draw can end the run.** It is reported as `GL_OUT_OF_MEMORY`, and dEQP
  treats that as fatal (a `tcu::ResourceError`) when a test checks `glGetError()`. The
  cases after it never begin, and the record says `STOPPED EARLY` (`complete=0`).
  tools/cts/RESULTS.md, batch I, has the details.
- Then the cases that did not pass by name, two to a row, for as many rows as
  are left. A name drops the filter's own prefix, and a name still too long keeps
  its tail (`~...`), which is the part that tells cases apart. Every line also
  goes to the mini UART.
- Zephyr LOG errors and warnings, and other driver lines, are counted, and the
  first 3 are shown.

### The watchdog bound

`CONFIG_V3D_DEQP_LIFETIME_S` (default 240) is the image's whole life: the PM
watchdog (`rpi_wdog.c`, `RPI_WDOG_HOLD_S`) is fed until then and resets the board
at most 10 s later, back into Linux. At `LIFETIME - CONFIG_V3D_DEQP_PHOTO_S`
(default 30 s) a run that has not finished prints the record it has, marked
INCOMPLETE, and the screen freezes. So there is always a record for at least 30 s.
A fault prints the record marked FATAL. The kernel may then halt (it did in the
QEMU run, for a fault in `main`), and a halt stops the watchdog feeding: the reset
then comes within 10 s, earlier than the record's "reset at" says. Keeping the
record up longer after a fault needs a feed call from `rpi_wdog.c`; not done.

A group must finish inside `LIFETIME - PHOTO`. The compiler takes about 0.2 s per
shader on the A53 of the AM62P; the Pi 4's A72 is not measured yet. So the first
runs measure ms per case (the record's second line). A group that does not fit is
split into several images, each with a comma-separated `CONFIG_DEQP_CASE` list.

### Order

1. `dEQP-GLES2.info.*` (6 cases) and `dEQP-GLES2.functional.prerequisite.*` (3):
   no textures, no depth.
2. `dEQP-GLES2.functional.color_clear.*` (19): the clear quad is on the V3D since
   3c94ad9d. The RGB and RGBA4-type targets were refused by name; they are drawn
   now ("M6 colour targets with fewer than 8 alpha bits").
3. Then groups that need no texture sampling, smallest first, sized by the
   measured time per case: `implementation_limits` (16), `flush_finish` (5),
   `read_pixels` (8), `depth_range` (28), `dither` (20), `light_amount` (19),
   `default_vertex_attrib` (40), `attribute_location` (57), `buffer` (49),
   `shader_api` (38), `negative_api` (246), `state_query` (396), then
   `vertex_arrays`, `rasterization`, `clipping`, `fragment_ops`. Groups that
   depend on depth or stencil surviving a job split will show R where the backend
   refuses that.
4. `texture`, `fbo`, and the `shaders` cases that sample wait for textures on the
   V3D.

### The QEMU smoke run

With `-DV3D_RPI_WDOG=OFF` (QEMU only: its raspi4b resets as soon as the watchdog is
armed). QEMU also starts all four cores at the entry point of an ELF, which the
Pi's firmware never does; CPUs 1-3 are parked on a two-instruction loop with
`-device loader,addr=...,cpu-num=N`. Result: the wrap works (the OSAL gets the
init argument; power-on then fails with -19, because QEMU has no V3D). dEQP stops
with `FATAL ERROR: Got EGL_NOT_INITIALIZED`. The record shows that line, the
driver line, and "FATAL (reason 0)". A host harness (Zephyr headers stubbed) fed
the tally 19 and 700 synthetic cases with refusals: every record fits the screen,
and no line is wider than 160 columns.

## M6 gl_FragCoord

The compiler and the job builder agree on two uniform contents codes of our own
(v3d-m5, v3d-isa, v3d-glue and the lead, 2026-09-24). They sit far outside Mesa's
`enum quniform_contents`, so neither side can mistake them for a Mesa code:

| Code | Name (C `v3d_frame.h`, Rust `src/v3d/ffi.rs`; compiler `select.rs`) | Value written | `data` |
|---|---|---|---|
| `0x10000` | `V3D_QUNIFORM_FB_WPOS_Y_SCALE` / `QUNIFORM_FB_WPOS_Y_SCALE` | f32 `-1.0` for a window, `1.0` for an FBO | 0 |
| `0x10001` | `V3D_QUNIFORM_FB_WPOS_Y_OFFSET` / `QUNIFORM_FB_WPOS_Y_OFFSET` | f32 the target's height for a window, `0.0` for an FBO | 0 |

The fragment shader computes `y = FYCD * scale + offset`. x, z and w are as
Mesa does them (nir_to_vir.c:1018-1024).

Why two uniforms: the V3D reports an upper-left origin with half-integer centres
(v3d_screen.c:313-316). Mesa corrects y with `STATE_FB_WPOS_Y_TRANSFORM`, a GL
state uniform that st/mesa fills from the draw buffer's `FlipY` and height
(prog_statevars.c:736-751). We have no state uniforms, so the driver fills the
two words itself when it writes the uniform stream.

- `src/v3d/state.rs` `wpos()` takes the same `flip_y` and `target_h` as the viewport,
  so the viewport and the transform cannot disagree. A window gets `(-1, height)`,
  an FBO `(1, 0)`. The result goes into `v3d_job_state.wpos`.
- `v3d_wpos_uniform()` (v3d_frame.c) writes the word for both uniform writers,
  the job builder's and the M5 frame builder's. It refuses, and the draw leaves
  nothing, when:
  - `data` is not 0;
  - the scale is not exactly 1 or -1 (so a zeroed, absent transform is never
    drawn as 0);
  - the offset is not the one that scale means for the job's target (0, or the
    target's height).
- Checked by `tools/v3d/job_check.c` part 6 (15 checks). Negative controls: a
  helper without the offset check, without the data check, with the two words
  swapped, or accepting a zeroed transform, each fails 1 to 3 of them. Also pinned
  by `state.rs` `a_flipped_target_gets_minus_one_and_its_height_and_an_fbo_gets_one_and_zero`.
- The test harness (tests/v3d_sim_mesa.rs) maps our codes as the FBO case, `(1, 0)`,
  where gl_FragCoord.y is FYCD itself.

On the board: nothing yet. It needs the compiler's side (v3d-m5), and then a
gl_test case with a pixel read-back on a window and on an FBO.

## M6 flush and finish: the synchronous submit, and what an asynchronous one needs (design, no code)

dEQP-GLES2.functional.flush_finish failed 5 of 5 on the Pi 4 (the v3d_deqp sample,
eddf6ab7). First, what that group measures, read from
`modules/gles2/functional/es2fFlushFinishTests.cpp` (VK-GL-CTS):

- Every case draws a 128x128 quad N times with blending on (`render`, :239-248),
  then calls `waitForGL()`, then reads one pixel. It times the wait and the read,
  fits a line to each against N (Theil-Sen, 25 samples, :358-367), and normalises
  the slope to the calibrated maximum N (the N that takes about 1 s, :259-321).
- The verdicts (wait / read):
  - `wait` (a 1200 ms busy loop): wait slope < 0.1; read > -1000, so anything passes.
  - `flush`: wait < 0.2; read > 0.5.
  - `flush_wait`: wait < 0.2; read < 0.1.
  - `finish` and `finish_wait`: wait > 0.5; read < 0.1.
- A slope outside its bound, or a failed calibration, gives
  **CompatibilityWarning**, not Fail (:320-321, :433, :457-460). The only Fail in
  the file is `TCU_FAIL("Compile failed")` in `init()` (:195-201), plus the
  `TCU_CHECK` and `GLU_EXPECT_NO_ERROR` in `setupRenderState` (:227, :236).
- The fragment shader has `for (int i = 0; i < 10; i++) color = sin(color);`
  (:184-194). Our V3D back end refuses loops (`prepare.rs` `Node::Loop =>
  NotYet("loop")`), unless the front end unrolls this one first.

So 5 of 5 **Fail** is the program failing to link, not the submit: under any submit
design `wait` cannot fail on timing. Confirmed on the board (the lead, 2026-09-24): all
five cases print "Compile failed at es2fFlushFinishTests.cpp:201". Once the program
links, the synchronous submit below predicts:
- `wait`, `finish` and `finish_wait` pass;
- `flush`: wait slope about 1 (glFlush does the whole job), read about 0;
- `flush_wait`: wait slope about 1.
That is two CompatibilityWarnings, which are not failures. The submit design matters
for frame pacing (Slint), not for a mustpass Fail.

### Today: the submit is synchronous

- `src/osal/zephyr_v3d.c` `z_submit` runs the whole job before it returns:
  - start the binner (`v3d_submit_bin`), poll for FLDONE;
  - start the renderer, poll for FRDONE;
  - write back the TMU caches (`v3d_caches_clean`).
  `v3d_wait` polls the core's interrupt status. `v3d_irq_quiesce` masks every
  interrupt at init, so no interrupt is used. `z_wait` only returns the result of
  that finished job.
- `src/v3d/backend.rs` `flush_job` submits and then waits at once. The reason: every
  job reuses ONE set of memory, and the next job may only record into it after the
  GPU is done:
  - BCL 64 KiB, RCL 4 KiB, indirect 256 KiB, data 1 MiB, texture shadows 8 MiB;
  - the tile alloc pool (up to 512 KiB) and the tile state.
  That is about 9.9 MiB of the 48 MiB carve-out (`YAKOGL_V3D_CARVEOUT_BYTES`).
- `kick_id` / `kick_pending` (T0) are already the PowerVR's contract: the kick being
  recorded is `kicks + 1`, and it is pending until its job has run. So the GL core
  already waits before writing memory a pending draw reads.

So glFlush costs as much as glFinish, and the CPU never records frame N+1 while the GPU
renders frame N.

### What an asynchronous submit needs

1. **Completion from the interrupt, not from a poll inside submit.**
   - Unmask FLDONE, FRDONE and OUTOMEM (binner out of memory) for the V3D core, and
     connect the core's interrupt line in Zephyr (`IRQ_CONNECT`).
   - The handler:
     - on FLDONE: start the render list;
     - on FRDONE: hand the job to a thread;
     - on OUTOMEM or an MMU fault: record the error and complete the job with it.
   - The thread (not the handler) writes back the TMU caches, which is itself a
     polled wait, then advances `completed` and gives a semaphore.
   - This is the same order as today's `z_submit`, cut at each wait.
   - Unread so far, to be read before any code:
     - the interrupt line's number, from the Pi 4's devicetree;
     - how the Linux driver splits this work (`drivers/gpu/drm/v3d/v3d_irq.c`,
       `v3d_sched.c`). It is GPL, so facts only, cited by file:line. The local
       kernel tree here does not hold those files.
2. **The OSAL's wait and completed become real.**
   - `z_submit` returns at once with the fence.
   - `z_completed` reads `completed`, which the thread advances.
   - `z_wait(fence, timeout)` blocks on the semaphore until `completed >= fence`, or
     the timeout. The job timeout (`JOB_TIMEOUT_US`) moves from submit to wait.
   - A hung job needs a recovery path: a reset, counted and reported. Today the
     board's watchdog is the only one.
3. **Two sets of job memory in the backend**, used in turn.
   - Without a second set, `flush_job` must still wait before the next draw records.
     Only the glFlush return is saved; there is no overlap.
   - With two sets, `flush_job` submits and returns. The next job records into the
     other set, and `begin_job` waits only for the job that last used that set.
   - Cost: about +9.9 MiB of carve-out. The texture arena is most of it; it could
     shrink, or be shared with a per-set high-water mark.
4. **kick_pending from the OSAL**: `pending(id) = id != 0 && id > completed`, with
   `completed` read through `osal.completed()` instead of assuming every submitted
   job has run. `kick_id` does not change.
5. **Errors arrive later.**
   - A job that fails is known only at a later wait. flush then returns 0 for a job
     that will fail.
   - The failure has to reach the GL core at the next flush, finish or read: the
     PowerVR backend reports a failed fence the same way.
6. **The depth/stencil history (`zs_record`) does not change.** It is what the CPU
   recorded, known at flush.
7. **Host tests.** The fake OSAL today completes a job at submit. It needs a pending
   state, completed by an explicit call from the test, so the tests can check that:
   - a draw is pending across a flush;
   - the second set is used;
   - `begin_job` waits for the set's previous job;
   - a failed job is reported at the next wait.

### What it buys, and when

- glFlush returns without doing the job (`flush` and `flush_wait` would pass).
- The CPU records frame N+1 while the GPU renders frame N. The gain is at most the
  smaller of the two times per frame. It is worth it only if both are a large part of
  the frame.
- Measure first, on v3d_gl_slint's cluster frame: the CPU time to record (draw calls
  up to the flush), and the GPU time from submit to FRDONE. Both can be read with
  `k_cycle_get_64` in the OSAL today, with no design change.

The lead decides when. Textures and gl_FragCoord come first.

## M6 attribute_location: 32 dEQP cases, two causes (findings and a proposal, no code)

dEQP-GLES2.functional.attribute_location scored 25 of 57 on the Pi 4 (eddf6ab7). The
host reproduces it exactly: the test `deqp_attribute_location_survey` (src/v3d/backend_tests.rs,
not committed yet). It builds dEQP's own sources and bindings (glsAttributeLocationTests.cpp:111-247,
iterate() :829-998, :1286-1316) and runs its two query checks (:491-640). Through the
GL core with gpu-v3d it fails the same 32 cases the board failed, and no other case.

These cases never draw. runTest only compiles, links and queries (:683-795), so the GL
core's attribute mapping (`vi_reg`, one record per column) is not reached. Where a
program links, both query checks pass.

### Cause 2: 18 cases, `Program(Alloc(DoesNotFit))` at link

The cases:
- `bind_hole.*` (7) and `bind_relink_hole.*` (7);
- `bind_max_attributes.{vec2,vec3,vec4,mat2}`.

Each is 15-16 bound locations of mostly vec4 inputs, all summed into one vec4. About 64
live `ldvpmv_in` components do not fit rf3..rf31. The allocator has no spilling and no
fallback to fewer threads (src/shaderc/online/v3d/alloc.rs:31, :58). The passing
neighbours read fewer components: float x16, mat4 x4 (`vec4(mat4)` reads column 0
only), mat3 x5. This is v3d-m5's allocator, and handed to them with the sources
(`.claude/worktrees/deqp-attrib-repro/`). Mesa's answer is fewer threads, then spilling.

### Cause 1: 14 cases, `Abi(AttribLocation { location: 16 })` at compile

The cases: `bind_aliasing.max_cond_*` (7) and `bind_aliasing.max_inactive_*` (7). Both
declare 2 x 16 attributes and bind them in pairs to the 16 locations:
- In `max_cond`, the two of a pair are read on the two sides of `if (u_A != 0.0)` /
  `if (u_A == 0.0)`, so all 32 are read.
- In `max_inactive`, the second of each pair is under `if (0 != 0)` and never read.

The compiler numbers attributes in declaration order (src/shaderc/online/reflect.rs:20),
so the 17th declared gets location 16. The V3D program assembly refuses it
(src/shaderc/online/v3d_abi.rs, `stage_shader`), at compile, before any binding is known.

How the GL core handles bindings today:
- `build_attrib_indices` (src/gl/program.rs) maps each compiler location to the bound
  index (`attrib_app_index`, 16 entries).
- It allows aliasing, and `draw.rs` gives each aliased input its own staging from the
  shared array.
- So aliasing works as long as the compiler's numbering fits 16. For these shaders it
  does not: 32 names, 32 compiler locations.

On the V3D, the program has at most 16 attribute records. So `max_cond`'s 32 inputs can
only be drawn the way Mesa draws them: the linker gives every attribute its BOUND
location, and the two names of a pair become one input read from one record. Our V3D
program is compiled at glLinkProgram anyway (`link_sources` recompiles the VS source).
What it lacks is the binding table: `shc_backend.link` (include/gles_shaderc.h)
carries none.

Options:
- **(a) V3D only, no API change: number only the attributes the program reads.**
  - Would fix `max_inactive` (7): 16 are read.
  - Only if the reads under `if (0 != 0)` are removed first. Today they stay in the
    IR: a probe with 16 read floats and a 17th read only under `if (0 != 0)` is
    refused with `AttribLocation { location: 16 }` whether the dead one is declared
    first or last (host, 2026-09-24).
  - Leaves `max_cond` refused by name.
  - Cost: v3d_abi.rs, the reflection's location for the V3D, and the constant-branch
    removal (mine and v3d-m5's).
- **(b) Bound locations into the link** (the root fix, Mesa's order):
  - `link` gains the program's binding table (name -> index). It is ignored by the
    back ends that do not need it.
  - At the V3D link the front end numbers each attribute: its bound index plus its
    column, and the lowest free locations for the unbound ones (ES 2.0 2.10.4).
  - Aliased names then read one input and one record. `attrib_app_index` becomes the
    identity for the V3D.
  - Fixes all 14. It also removes the reason the PowerVR needed 65 vertex-input
    registers for the same cases (tools/cts/RESULTS.md, `OutOfRegisters { file:
    VtxIn, needed: 65 }`), if that back end takes the table too.
  - Cost: a shared header and the GL core's link call (the lead's), the front end's
    location assignment (v3d-m5's), and v3d_abi.rs (mine).
  - Tests: the survey above as a pin, and gl_test's aliasing case with a pixel
    read-back on the V3D.

The lead decides between them. The survey becomes a committed pin either way: it asserts
these 32 names with these two messages, and fails on the day a case moves.

**Resolved with (b), through the shared link-time layout (2026-09-25).** The GL core
and the other two back ends built (b) as `layout_at_link` / `compile_vs_linked`
(src/shaderc/SHADERC_NOTES.md, "Attribute registers decided at link"). The V3D now
takes part:
- `codegen::compile_linked_blocks`: a vertex shader that declares more than
  `SHC_V3D_MAX_ATTRIB_LOCATIONS` (16) locations (a matrix counts its columns) gets
  `deferred_vertex`'s record: no code, `layout_at_link`, every declared attribute.
- `compile_vs_linked` is registered on the V3D. Its record is the stage's reflection
  with `with_attrib_map`'s locations, and lists each location once
  (`v3d_abi::stage_shader`).
- The program is built at link from the sources, so the relinked record keeps the map
  (`Compiled::attrib_map`) and `link_sources` applies it to the vertex shader. The map
  is in that record's cache key, so it is in the program's too.
- The chain already fetched per location (`attributes`, `vattr_sizes` the widest read),
  so two names on one location read one record. No change to link.rs or the draw.

Host evidence: `v3d_abi::tests::deqp_bind_aliasing_links_with_the_links_layout` builds
dEQP's 14 vertex shaders (their lengths are the board record's), binds them as dEQP
does, and links all 14. The board run is the measurement.

## M6 tiled-copy cache: one tiling per texture write, not per job

The board measured the CPU tiling at 37 KiB/ms on the carve-out's non-cacheable memory
(v3d_gl_triangle's bench, 6c31ab9c): 256x256 RGBA8 costs 6.8 ms. The per-job copy of
M6 textures tiles every sampled texture again in every job, so it fails the 2 ms per
frame rule for Slint. The lead's decision (B), 2026-09-24: keep one tiled copy across
jobs, made again only when the texels change.

**The GL core's part.** It is additive; the PowerVR and the GC7000 do not use it.
- **The stamp** (`GlTexture::content_gen`, passed as `GlesTextureBind::content_gen`):
  - A new value from ONE global counter (`texture.rs` `next_content_gen`) is taken at
    every CPU write of the texels.
  - Every write entry point calls `settle_texture` first, so the stamp is taken there.
  - It is also taken when the storage is laid out again (`texture_alloc_storage_ex`,
    the compressed path), which is where a fresh glTexImage, the one write that skips
    `settle_texture`, gets its stamp.
  - Being global, a stamp is never reused for other texels at an address that was
    freed and handed out again.
- **Absence has its own value.** 0 is "no stamp": storage an EGLImage shares is 0,
  because a sibling can write it without passing here. A backend never caches a
  binding with 0. The cube-map probe's own copy also passes 0.
- **The hook.** `YakoglBackend::texture_released(base_gpu, bytes)` is called from
  `release_storage_with`, BEFORE the OSAL's free is scheduled. It is `None` on the
  PowerVR and GC7000 tables.

**The V3D backend's part** (`src/v3d/backend.rs`, `tiled_copy`):
- A copy is keyed on (base_gpu, content_gen, width, height, levels sampled, bgra,
  format, layers), in memory of its own: `gpu_alloc`, level 0 aligned to 4 KiB inside
  it.
- At most `TILED_ENTRIES` (64) copies and `TILED_BUDGET` (8 MiB by default; the
  platform's `OSAL_QUERY_V3D_TILED_BUDGET`). The victim (`tiled_victim`) is the least
  recently sampled, and of those sampled equally recently, the one with the fewest
  bytes. The OSAL defers every free past the job being recorded, so a copy the open
  job samples stays valid.
  - Why the byte order: every copy one job samples carries that job's kick, so when
    the whole table is the last frame's working set they all tie. The first slot went
    before, which is the texture tiled first. On the cluster (v3d_gl_cluster DEMO3N,
    f5f28256, 2026-09-25) that was the 1920x720 backdrop and the two 600x600 rings: the
    worst frames re-tiled 28-41 textures, 8.5 MB, inside the draws (GL draws 72 ms
    against 14-17), with the table at 64 entries and 12 of 16 MiB. The count limit
    bound, not the bytes. Now a tie costs the smallest copy.
  - That was not enough (DEMO4, 5dfc7954 = dd70ff1e plus glue's counters,
    2026-09-25): 4.1-4.7 fps against 5.7-5.9, with 19-22 tilings (3.8-5.6 MB) every
    frame and 700-937 evictions per 10 s, all hot and all for the entry count. The
    t=10 s dump: 64 of 64 live, every one sampled by the last job, 6.5 MB, the
    1920x720 backdrop not among them (just evicted). One frame needs about 64 + 20 =
    86 entries and about 12.3 MB. `TILED_ENTRIES` is 256 since; the tie order stays
    (with age first it only matters when a tie remains).
  - The same run's second cause: 634 lines "no GPU memory for a tiled copy of 5533696
    bytes" (the backdrop), with 11.6 MB of the carve-out free and often no frees held.
    Once evicted, the backdrop found no contiguous room and was tiled into every job's
    arena (the 3.8-5.6 MB of tilings a frame). So the working set is never evicted
    for a newcomer: when the least recently sampled copy was sampled by the open or the
    previous job, the newcomer is not kept (`KeptMiss::WorkingSetFull`) and is tiled
    into the job's arena, as a failed copy always was. Only a newcomer the arena can
    take: one larger than the arena (dEQP texture.size 2048x2048, 8-12 MiB tiled) has
    no other way to be drawn and evicts as before (the gate's
    `a_kept_copy_that_fits_only_after_the_open_job_runs_is_made_after_it` caught the
    rule without this).
  - `v3d_tiled_misses(no_memory, kept_refused)` counts the kept copies not made: no
    OSAL memory, and refused for the working set.
  - The OSAL reports its free space as runs (`osal_v3d_mem_free_runs(longest, runs,
    free_total)`, `src/osal/zephyr_v3d.c`, the walker `v3d_free_runs` in
    `src/osal/v3d_frees.h`, checked by `tools/v3d/osal_frees_check.c`). A large copy
    that fails with the free total above its size and the longest run below it is
    fragmentation. Placing large copies at the carve-out's other end waits for those
    numbers from the board.
  - `v3d_tiled_evictions(count[2], bytes[2], hot[2])` counts the evictions since init
    by cause ([0] the entry count, [1] the byte budget), their bytes, and the "hot"
    ones: sampled by the previous or the open job, the working set, which the next job
    tiles again. A drop by a new stamp, a render or a release is not an eviction.
  - `v3d_tiled_dump()` logs one line per live copy (slot, size, layers, levels,
    format, bytes, age in jobs) and returns how many, for a one-time look at what
    fills the table.
- A new stamp of the same storage drops the old copy.
- A job that renders into memory drops the copies made from it, in `begin_job`: the
  stamp moves only on the CPU's writes. The next job that samples it tiles it again,
  after the render's job has run (the submit is synchronous).
- Tiling happens at the first sample, not at the write. So femtovg's glTexImage2D(NULL)
  followed by a full glTexSubImage2D costs one tiling, not two.
- Refused, or too large for the budget, or no memory: the copy goes into the job's
  arena as before, per job.
- The texture state and sampler records stay per job (24 bytes each).
- `v3d_tiled_copies(live, bytes, high_water)` reads the cache. A log line is printed
  each time the high-water doubles, and at shutdown with the tiling totals.

**Not now:** a partial re-tile from the glTexSubImage2D box. There is one stamp, and the
whole level is tiled again.

**Tests.**
- Backend (`src/v3d/backend_tests.rs`):
  - a stamped texture is tiled once across two jobs, and once more after a new stamp;
  - an unstamped one is tiled per job and never kept;
  - a render into the texture drops the copy, and the next sample tiles again;
  - `texture_released` frees the copy;
  - LRU within 64 entries;
  - a tie (64 copies all sampled by the last job, and one more) gives up the smallest,
    counted as hot and for the entry count, and the big copy is not tiled again next
    job (the old first-of-the-tied victim fails it);
  - a copy the last job did not sample goes first however large, and is not hot;
    once only the working set is left, the next eviction is hot;
  - an eviction the byte budget forces is counted under that cause; the dump lists the
    live copies;
  - the working set is never evicted for a newcomer: the newcomer is refused, counted
    and tiled into the arena, and kept once the working set has aged past two jobs;
  - a 200-entry working set is found whole in the next job, with no tiling;
  - through the GL core, glDeleteTextures frees the copy BEFORE the storage (the
    OSAL's free order) and leaves no copy.
- GL core: every CPU write entry point gives a new non-zero stamp, an EGLImage's
  storage stays 0 (`texture.rs`), and the draw carries the stamp (`draw.rs`).
- Negative controls, each failing exactly its test:
  - the hook moved after the free;
  - no drop on a render;
  - caching an unstamped binding;
  - keeping a stale stamp;
  - evicting the newest;
  - no stamp in `settle_texture`;
  - no stamp at layout;
  - no stamp in the bind.

**On the board:** not yet. The measurement is the bench's rate times
`v3d_texture_counters`' bytes per frame, on v3d_gl_slint's cluster frame.

## M6 colour targets with fewer than 8 alpha bits

The GL core stores every colour target as RGBA8; only
`GlesDraw::target_alpha_bits` says what the format asked for (8, 4 for RGBA4,
1 for RGB5_A1, 0 for RGB and RGB565). The backend refused anything below 8.
That blocked gl_test 67's RGB attachment, dEQP lifetime (an RGBA4
renderbuffer, glsLifetimeTests.cpp:474-490) and polygon_offset's fixed16
cases (an RGBA4 colour renderbuffer, es2fPolygonOffsetTests.cpp:317).

- **1 or 4 alpha bits:** rendered as RGBA8, as the PowerVR does. The GL core
  quantises the clear's alpha (`framebuffer::quantise_unorm`).
- **0 alpha bits:** the format reads alpha 1 (ES 2.0 4.3.1). The GL core
  samples and reads the storage as RGBA8, so the stored alpha must BE 1.
  - The job loads the colour with Force alpha 1 (`V3dJobClear::force_alpha_1`,
    LOAD_TILE_BUFFER_GENERAL bit 18), as Mesa's v3d does for a format whose
    alpha swizzle is 1 (v3dx_rcl.c:99).
  - No draw writes alpha: `job_state` clears bit 3 of the colour mask. Mesa
    need not do this, because its X8 formats are always read through a
    SWIZ_XYZ1 view.
  - The TLB clear's alpha is the GL core's quantised 1.0 (0xff).
  - Blending already uses Mesa's dst_alpha_one factors
    (v3dx_emit.c:34-122, `state::blend_factor`).
  - The alpha-less bit is part of the job's `Target`: draws into targets that
    differ in it never share a job.

Pinned:
- `state::tests::a_format_with_fewer_alpha_bits_is_drawn_and_one_with_none_keeps_alpha`;
- `backend_tests::a_target_with_fewer_alpha_bits_is_drawn_and_one_with_none_keeps_alpha_one`
  (RGB565 cleared and drawn, RGBA4 loaded and drawn, then RGB565 again);
- job_check part 8: Mesa's decoder reads Force alpha 1 on the colour LOAD of an
  alpha-less job, and not on the other.

Each was checked against its revert. No board run yet: gl_test 67 and the
lifetime and polygon_offset groups are the evidence to collect.

## M6 tile at upload: the V3D side of texture_uploaded

The GL core's `texture_uploaded` hook (approved by the user as proposal (a),
.claude/worktrees/proposal-texture-uploaded-hook.md) hands the backend the
caller's rows of an RGBA / UNSIGNED_BYTE upload. `v3d_texture_uploaded` makes
the tiled copy from them at once, under the key a draw looks for (the stamp
after the write), when all of these hold:
- the box is the whole of level 0;
- one level is sampled (no mipmap min filter);
- the texture is stamped (not an EGLImage), not XRGB (`bgra`), and the
  sampler takes it (`texture::refusal`).

Every other upload makes nothing. Its new stamp makes the next draw tile the
storage, as before. This is v3d-tex's rules 3 to 5, minus one optimisation:
a sub-box applied to a still-valid copy. Slint uploads no sub-boxes (0 in
9,503), so that is left for when something needs it.

- The copy is tiled with `v3d_tex_store_box`. It refuses a stride shorter
  than a row; a refused copy is freed and nothing is kept.
- The copy shares the cache and its rules with the draw's copies: budget,
  LRU, drop on a render into the storage, and texture_released.
- `eager_tilings` counts the tilings made at an upload; the shutdown line
  prints it.

Pinned in the rig (backend_tests):
- a whole upload is tiled once, at the upload, and the draw tiles nothing;
  the copy is byte for byte the one a draw tiles from the storage (with the
  fake layer; the C's store_box against Mesa is tex_check's);
- eight uploads that must make nothing make nothing; a sub-box of the next
  stamp leads to the lazy tile at the draw;
- a one-level copy is not found once the chain is sampled.

Each was checked against its revert. No board run yet; the Slint frame times
are the evidence (5.4 / 12.8 / 23 ms predicted to become 0.9 / 2.1 / 3.8 ms).

## M6 points: one program for every primitive

Mesa's V3D compiles a points variant: its VS writes the point size into the
VPM only for a points draw (vs_key per_vertex_point_size), and the shader
record's "Point size in shaded vertex data" is set only then
(v3dx_draw.c:556-558). A second program per GL program would have been
shared code (include/gles_shaderc.h, src/shaderc, src/gl). So the board was
asked first whether the bit only describes the VPM layout.

- gl_test case 110 psizvpm (image 37264ce2, the lead's board run): the same
  triangle and three lines, drawn by a VS that keeps the gl_PointSize word
  and by one without it. The triangle lit 1588 pixels and the lines 166; each
  had 1 draw with the word; 0 of 4096 pixels differ; 0 timeouts; PASS.
- So the triangles program serves points:
  - v3d-m5's VS keeps the word whenever the shader writes gl_PointSize
    (their commit);
  - the record's bit is the program's writes_psiz (a3d3fb0c);
  - the job builder and the state no longer refuse points.
- A VS without gl_PointSize leaves the bit off, and the POINT_SIZE packet's
  1.0 is the size. That is Mesa's path without a per-vertex size
  (v3dx_emit.c:434-435); ES 2.0 leaves the size undefined.
- Mesa's RCL uses LIST_TRIANGLES for every primitive (v3dx_rcl.c:364-365),
  as ours does.
- Not yet: gl_PointCoord. Its implicit point varyings need the FS's
  uses_implicit_point_line_varyings, which the compiler refuses by name for
  now.
- Pinned: the rig (a points draw is recorded with the program's bit) and
  job_check part 9 (a points job decodes as VERTEX_ARRAY_PRIMS mode 0, with
  the bit).
- gl_test case 111 points draws sizes 1, 3 and 5 and a VS without
  gl_PointSize, on the window and an FBO, and checks every pixel. It says NOT
  EXERCISED until the compiler keeps the word.
- Board evidence to collect when both halves are on main: case 111, dEQP
  clipping.point (11), rasterization.limits.points and
  rasterization.primitives.points, scissor's point draw, and the 29
  fragment_ops cases that point refusal left unbegun.

## M6 XRGB8888 targets and unbound sampler units

Two causes from the diag record (27f7382c, gl_test 61/109/18; the lead's
board run):

- **gl_test 18** drew into an XRGB8888 target, which the backend refused
  ("a colour target other than RGBA8888").
  - An XRGB8888 target's bytes are B,G,R,X. Mesa 26.2.1 renders B8G8R8X8 with
    the TLB swapping R and B at the load and the store (`r_b_swap` for a
    format whose first swizzle is Z; v3d_formats.c:171-177, v3dx_rcl.c:98
    and :150). Its shader no longer swaps (`swap_color_rb` is reset and never
    set, v3dx_state.c:547).
  - The alpha is X, so the target takes the no-alpha path of "M6 colour
    targets with fewer than 8 alpha bits": Force alpha 1 at the load, alpha
    masked, dst_alpha_one.
  - `V3dJobClear::r_b_swap` carries the swap. It is part of the job's
    Target, so an XRGB and an RGBA target never share a job.
- **gl_test 61**'s "nothing bound" draw was refused by the job builder.
  - The GL core leaves a sampler with texture object 0 bound out of the draw,
    so the FS's TMU P0/P1 words named a unit with no records.
  - Texture object 0 is incomplete and samples (0, 0, 0, 1) (GL ES 2.0
    3.7.10). Every unit a stage's uniform streams sample that no binding
    names now gets the incomplete-texture stand-in and a NEAREST sampler.
- **gl_test 109** passed on its own. Its earlier failure was case 18's
  leaked EGL context, which the harness now releases (gl_test.c's walker).

Pinned: state and rig tests, and job_check part 8 (R/B swap read at the
colour LOAD and the RT0 STORE, 224 of 224). Each was checked against its
revert. Board evidence to collect: gl_test 18 and 61.

### RGB565 targets (gl_test 18, second cause)

The verify record (613dcddd) showed case 18 still refused, now with "a
colour target other than RGBA8888 or XRGB8888 (RGB565, float)". So case 18's
target was RGB565, not XRGB8888, and my earlier reading that its pbuffer
config is the RGBA8888 one was wrong: I read the config table's order, not
the sort.
- Case 18 asks eglChooseConfig for EGL_SURFACE_TYPE = EGL_PBUFFER_BIT alone.
  With no colour size requested, EGL 1.5 3.4.1's colour-bits rule compares
  nothing, and the next rule (EGL_BUFFER_SIZE, smaller first) puts the 16-bit
  config first.
- Pinned on the host: a rig test calls eglChooseConfig with case 18's list
  and gets BUFFER_SIZE 16, RED 5, ALPHA 0.

RGB565 targets are now drawn as Mesa draws B5G6R5_UNORM:
- output image format bgr565 (7) at the colour load and the RT0 store
  (v3dx_format_table.c:81), no R/B swap (v3d_formats.c:176);
- the same 8-bit tile buffer (v3dx_format_table.c:225-234);
- the GL core's pitch at 2 bytes a pixel;
- the no-alpha path: Force alpha 1 and alpha masked.
`V3dJobTarget::rgb565` carries it (the size is now 24; v3d_job's offsets move
by 4, all pinned).

Each refused float format now names itself ("an RGBA16F colour target ...",
R16F, R32F, and an unknown format): the log says the value the backend saw.

Pinned, each against its revert: a rig test (16-bit target, pitch x 2,
alpha 1, no swap); job_check part 8 (bgr565 at the LOAD and the RT0 STORE at
2 bytes a pixel, 227 of 227); cl_golden (our BGR565 constant against Mesa's
enum).

### gl_test 18 on the board: both causes fixed

- a60104c2 (the lead's run, 20:13): checks 1-7 ok. The RGB565 pbuffer draw
  is right: top-left blue, bottom-right green. Then the native-buffer image
  draw was refused: "the texture's storage is not in the V3D OSAL's memory".
  - Cause: map_phys places a native buffer at a window VA past the
    carve-out, and cpu_of answered only for the carve-out, so the backend
    could not read the texels to tile them.
  - Fix, 0f002cfc: map_phys records each window (VA, span, CPU view) in a
    32-entry table, and cpu_of answers inside it.
- eb5685fe (a60104c2 + 0f002cfc, 20:18): checks 8-12 ok. The native
  image's bottom row is red, its top row white and its second row green;
  the image is kept after the source is deleted; eglDestroyImage succeeds.
  "case 18 image: PASS", 0 timeouts.

## M6 cube maps: the backend side

317 dEQP-GLES2 texture cases and 45 vertex.cube cases are behind the cube
refusal (v3d-tex's survey). The work is split three ways:
- v3d-tex, the C layer (2e6068cf): a cube is six mip trees
  `v3d_tex_layer_stride` apart (Mesa's cube_map_stride,
  v3d_resource.c:757-779), and the record has depth 6 and array_stride_64.
- v3d-glue, the backend (this section).
- v3d-m5, the lookup (TMUT/TMUR, TMUSCM).

The backend:
- The V3D table sets `native_layered_textures`. The GL core then binds a cube
  as one face's size with layers 6 and stacks the faces plainly. It only
  changes the binding and 3D storage (3D is still refused).
- `texture::gl_level_layered` is the GL core's cube storage: faces stacked
  inside each level, a chain's levels rounded as `chain_layout` rounds them
  (a 16 cube's level 0 is 128 rows). Pinned against the GL core's
  `level_off` and each face's texel (cube step 1, 2d404bab).
- The refusal accepts a whole (6-face), square cube. 2D arrays and 3D stay
  refused by name.
- `tile` puts face f's level L from its rows in the GL core's level into
  tree f. `image_bytes` sizes the layered image; `state` sets depth 6. The
  tiled-copy key carries the layer count.

Pinned in the rig, each against its revert (every face tiled from face 0's
rows; every face tiled into tree 0): an 8 cube with 4 levels, every texel of
every face and level found in its own tree. Board evidence waits for m5's
lookup.

The incomplete-texture stand-in has a cube twin: a 1x1 cube, six black
faces, depth 6. A samplerCube with nothing bound, or bound to an incomplete
cube, samples it; a 2D sampler samples the 2D one. A cube look-up steps faces
by the record's array stride, so the 2D record (depth 1) would send faces
1..5 past its image. Mesa, too, samples a fallback of the sampler's own
target (main/texobj.c _mesa_get_fallback_texture). For a unit no binding
names, the type comes from the linked program's uniform table; VS units are
2D until the compiler takes VS look-ups.

**On the board with v3d-m5's cube look-up** (84b2cf93, TMUR + TMUSCM; image
f7bf2f44 = batch-21 pre-merge 105e9590 + 84b2cf93, 22:50, the lead's
record): gl_test 113 cubemap (the other team's fingerprint, textureCubeLod
in the VS), 115 texcube, 118 texmip and 61 (an unbound samplerCube) all
PASS, 4 of 4, 0 timeouts. The six-layer record and the cube stand-in work
with the real look-up. 115 run alone measured the LINEAR edge error across
faces at 112 to 174 steps: the V3D filters seamlessly across cube faces.
dEQP's ES2 reference sets that per test group (read in the source, not
measured): seamless in texture.filtering (es2fTextureFilteringTests.cpp:594);
not seamless in texture.size and texture.format (es2fTextureSizeTests.cpp:320,
es2fTextureFormatTests.cpp:352); and only on an ES3-capable context in
texture.mipmap, texture.vertex and shaders.texture_functions
(es2fTextureMipmapTests.cpp:658, es2fVertexTextureTests.cpp:1154,
es2fShaderTextureFunctionTests.cpp:633), so not under YAKOGL_REPORT_ES2.
LINEAR cube cases in the non-seamless groups can differ at face edges;
NEAREST is exact.

## M6 job arenas: what an overflow does, and the binner's tile pool

**An arena overflow splits the frame. It is not refused and not truncated.**
- When a job's BCL, indirect, data or texture arena is full (v3d_job_draw
  answers V3D_JOB_FULL, v3d_job.c:540; or the backend's own arena answers
  Full), draw_in_job flushes the open job and records the draw into a new job
  on the same target (backend.rs `draw_in_job`).
- Only a draw that does not fit even an empty job is refused: -E2BIG, "a draw
  does not fit an empty job".
- The split frame is correct. A window surface always keeps its depth and
  stencil in a D24S8 buffer (ds_persist, draw.rs `resolve_ds_storage`;
  step C), so they survive the split, and so do an FBO's when it has a
  depth/stencil renderbuffer.
- The split costs one more job: a synchronous submit and wait, plus a colour
  load/store and a Z/S load/store of the whole target. Unmeasured.

Slint's dashboard (first frame, the lead's board record): 493 draws, BCL
55,395 of 65,536 (112 B a draw), indirect 188,557 of 262,144 (382 B a draw).
At today's sizes the BCL splits a job at about 580 draws.

**The binner's tile pool.** The tile lists the binner writes live in the
pool the job gives it (CT0QMA/CT0QMS). The pool is sized at init for 1024
draws at 2048x2048 (v3d_frame_alloc_sizes, Mesa's rule), and the OSAL gives
CT0QMS all of it: 663,552 bytes. Of that, 139,264 is fixed (32 x 32 tiles x
128 B initial blocks = 131,072, rounded to 4 KiB, + 8 KiB) and 524,288 is
the draw part (131,072 x 1024 / 2, capped at 512 KiB). Only the draw part is
capped. When the pool runs out, the binner asks for more (step c).
- After every job's binning the OSAL reads V3D_PTB_BPCA and _BPCS (Linux
  v6.17 v3d_regs.h:339-340, facts only), with CT0QMA/QMS and the pool it
  gave (OSAL_QUERY_V3D_BIN_POOL).
- The backend takes BPCA less the pool's start as the pool's use, only when
  BPCA lies inside the pool; otherwise "not read", never 0.
- The arena line prints the high-water ("binner tile pool N of S bytes").
  The first answer is printed raw once, so the reading "BPCA is the next free
  address" can be checked on the board: it must lie inside the pool and grow
  with the draws. (Board-confirmed: BPCA - start == size - BPCS. Slint's
  frame used 282,624 of 663,552.)
- The high-water has its own line ("binner tile pool high-water at frame N:
  ..."). glog formats into 256 bytes, and the arena line with Slint's
  numbers leaves no room for the overflow form after it.

**The draw budget (step b).** CONFIG_YAKOGL_V3D_DRAWS_PER_JOB (default
1024, 64..8192) reaches the backend through OSAL_QUERY_V3D_DRAWS_PER_JOB;
without an answer the backend uses 1024. From it, rounded up to 4 KiB:
- BCL = 4 KiB + 129 B a draw;
- indirect = 8 KiB + 440 B a draw (15 % over the measured 112 and 382);
- the binner's tile pool by Mesa's rule, still capped at 512 KiB.
At 1024 draws: BCL 139,264 and indirect 458,752 bytes (from 65,536 and
262,144), about 264 KiB more of the 48 MiB carve-out. The init prints the
sizes and whether the budget was the platform's or the default. The note
after the 493-draw record: those were two Slint frames in one job (237 +
250, v3d-slint), so one frame is about 250 draws.

**The tiled copies' budget.** CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES (default
8 MiB, the old constant) reaches the backend through
OSAL_QUERY_V3D_TILED_BUDGET. The texture arena's size check (8 MiB tiled)
applies only when a texture goes to the per-job arena: an unstamped one (an
EGLImage's), or one the kept copies cannot take. A 2048x2048 RGBA8 texture
(about 16.8 MiB tiled) is therefore sampled from its kept copy when the
budget holds it. When the kept copy cannot be made, the refusal names why:
unstamped (an EGLImage's), over the tiled-copy budget, no GPU memory for the
copy, no free slot, the tiling refused the image, or an address past 32
bits. For no memory, a log line also gives the bytes, the copies' live
bytes and whether a job is open; the OSAL's own line gives the frees it
holds for the open job. T5 on eaf98dc8 read "the texture arena (8 MiB tiled)" for the
2048x2048 RGBA8 mip chain: 22,376,448 bytes tiled, inside the 24 MiB
budget, so the kept copy's allocation is the likely cause (the OSAL's "out
of GPU memory" line confirms it). The dEQP image sets 24 MiB (with the 64 MiB carve-out, T5 on
651755c7: the seven size.2d 2048 cases were refused by the arena check with
the budget at 8 MiB).

**Freed memory comes back at once when no job is open.** The OSAL submits
synchronously, so a free only has to wait for the job the backend is
recording (it may render into, or sample, that memory). The backend now
says when it opens a job and when none is open (OSAL_QUERY_V3D_JOB_OPEN,
sent after a job has run). The OSAL frees at once without an open job and
holds a free for the job being recorded otherwise (src/osal/v3d_frees.h,
host-checked by tools/v3d/osal_frees_check.c). Before this every free
waited for the next job: in T4 (the Pi 4's texture set on 5320acfc) 27 cube
cases whose draws were refused left 25.3 MB held, and cube.512x512_l8's
11.2 MB upload ran out of memory. An allocation that fails drains what
completed jobs no longer hold and tries once more. Running the open job to
free its memory on an allocation failure would need the GL core, which is
the allocator's caller: not done for the GL core's allocations.

**A kept copy that fits only after the open job runs.** For the backend's
own allocation, the kept tiled copy, the backend is the caller and does
it. T5 on 68b6df6a (texture.size 2048x2048_rgba8888_mipmap, 64 MiB
carve-out) showed the case. The previous case's draw, still in the open
job (dEQP does not flush between cases), samples a 16,781,312-byte kept
copy. The new 22,376,448-byte copy evicts it (the 24 MiB budget), and the
OSAL holds that free for the open job. The allocation then finds 15,380,480
bytes in the largest extent, and 32,161,792 with the held free given back.
Now a draw whose kept copy finds no memory, and which the texture arena
cannot take either, ends the open job and tries once in a new one
(`Attempt::NoMemory`). Ending the job drains the held frees
(OSAL_QUERY_V3D_JOB_OPEN 0). In the new job the draw is drawn, or refused
with the no-memory reason. The log says "the job runs first, and the draw
is tried in a new one (retry N)" for the first 16 retries, and the
shutdown line counts the retries and how many were drawn. A texture the
arena can take is still tiled into the arena, with no extra job. Host test:
a_kept_copy_that_fits_only_after_the_open_job_runs_is_made_after_it (its
mock OSAL holds frees as v3d_frees.h does).

**The binner's overflow blocks (step c).** When the pool runs out, the
binner raises OUTOMEM and waits. It does not report FLDONE until it gets
more memory (Linux v6.17 drivers/gpu/drm/v3d/v3d_irq.c:39-77, 94-101: the
overflow work gives a 256 KiB block through V3D_PTB_BPOA, then BPOS;
OUTOMEM is edge-signalled; facts only). The OSAL does the same:
- CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS blocks of 256 KiB (default 2,
  0..64) are taken from the carve-out once, at init, page-aligned, and never
  freed. So a job that runs out needs no allocation and no page-table
  change while the binner waits. Every job uses them again; the bin writes
  BPOS 0 first, as the kernel does (v3d_sched.c:245-248).
- `v3d_wait_bin` (driver/src/v3d/v3d_hw.c) waits for FLDONE. On each
  OUTOMEM it writes the next block to BPOA, then its size to BPOS, and waits
  on. With no block left it fails by name ("V3D binner out of memory (N
  overflow blocks given)", -ENOMEM). The job's frame is lost, and the binner
  is left waiting for memory; there is no GPU reset, so later jobs time out.
  The OSAL counts these apart from timeouts (osal_v3d_diag_print: "binner
  out of memory: N jobs").
- The default: Slint's 493 draws (two frames) used 282,624 bytes, about
  570 B a draw. 1,024 draws then need about 590 KB; the pool (663,552) plus
  two blocks (524,288) cover that twice over.
- OSAL_QUERY_V3D_BIN_POOL carries the blocks the job took (overflow_used),
  the blocks there are and their size; the struct is 40 bytes. A job that
  took any has BPCA in the last block, so no pool use is read. The line then
  says "ran out of S bytes: U of N overflow blocks of B bytes taken".
- Host checks: tools/v3d/hw_mock_test.c t_overflow (one and two OOMs served
  in order, BPOA before BPOS; a dry pool and no pool fail with -ENOMEM) and
  the backend test the_arena_line_carries_the_binners_tile_pool_use.
  Unmeasured on the board: no workload has run the pool out yet. gl_test
  case 124 binovf makes one (frames of N checkerboard draws on a 1024x1024
  FBO, N growing by half until a job takes a block; every pixel checked),
  and its control image (CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS=0, the case
  last) must fail by name and count "binner out of memory: 1 job".
  Board (Pi 4, 99ae2d63, the lead's record): 8, 12 and 18 draws took 0
  blocks, and 27 draws took "overflow blocks 1 of 2, picture right" in one
  job: PASS. The OSAL line read "binner out of memory: 0 jobs". Step (c)
  is measured on silicon.
  The control (CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS=0, same build,
  22:21): 8, 12 and 18 draws each went in 1 job, 0 of 0 blocks, picture
  right. At 27 draws: "V3D binner out of memory (0 overflow blocks
  given)", FIRST FAILURE bin rc -12 (job 4), 1048576 of 1048576 pixels
  wrong, "counted 1": PASS. The OSAL read "timeouts 0, binner out of
  memory: 1 jobs". Both arms differ where predicted: the 27-draw frame
  needs a block, gets one with two configured, and fails by name with
  none.

## Z and stencil apart (FB2, behind v3d_split_zs)

FB2 on the Pi 4 (651755c7) drew 14 wrong pictures from one cause, found on
the host rig (dEQP's GL sequences through the GL core into this backend;
backend_tests.rs):
- The GL core names an FBO's depth/stencil storage
  `(depth rbo << 16) | stencil rbo` (draw.rs `resolve_ds_storage`), and
  the backend kept ONE D24S8 buffer per name.
- shared_depthbuffer.*_stencil_index8 (7): two FBOs share a depth
  renderbuffer and each has its own stencil, so the names differ. B's job
  used its own buffer and never loaded the depth A stored. The control
  without stencils (one name) loads it.
- recreate_stencilbuffer.no_rebind_*_depth_component16_stencil_index8 (7):
  the stencil renderbuffer is deleted and made again while the FBO is not
  bound. The GL core's orphan rule then names the FBO's stencil
  differently, and the depth written before is lost with the old name.

The fix keeps each half under its own renderbuffer (`zs_keys`). When an
FBO's depth and stencil names differ, the depth lives in the buffer keyed
`depth << 16` and the stencil in the buffer keyed `stencil`. These are the
keys an FBO with only that depth, or only that stencil, already uses.
Windows, packed depth-stencil renderbuffers and FBOs with only one half keep
one buffer, as before.
- The job builder loads and stores the halves apart. `v3d_job_clear`
  gains `s_address` and friends; with it set, `zs_*` is the depth's buffer
  as buffer Z and `s_*` the stencil's as buffer STENCIL. Both are D24S8 in
  memory: Mesa loads or stores one half of a packed zsbuf this way
  (v3dx_rcl.c:173-184 zs_buffer_from_pipe_bits). The order is Mesa's:
  stencil then Z on load (:220-234), Z then stencil on store (:288-304).
  job_check part 7 pins the packets.
- GFXH-1461 (a TLB clear of one half is lost when the other is loaded,
  v3dx_draw.c:1676-1689). A clear inside a job already goes to the quad
  when the other half is loaded. With the switch on, a job also stores a
  half it only cleared, so a kept buffer's history is only Stored or
  Untouched, never "cleared, not stored". A job therefore never loads one
  half while the other starts from a TLB clear.
- An image chooses it at start with CONFIG_YAKOGL_V3D_SPLIT_ZS (default
  n), read at init through OSAL_QUERY_V3D_SPLIT_ZS (8); an FB2 image sets it.
- `v3d_split_zs(bool)` switches at run time. It flushes the open job and
  gives back every kept buffer, so neither way reads what the other stored.
  It is OFF by default until the board has measured it: gl_test case 123
  zsapart replays both dEQP sequences, off (the FB2 picture) then on (the
  right one), every pixel off the depth-tie diagonal judged; then an FB2
  re-run with it on.
- On this main the GL core's orphan rule keeps the orphan attached under
  another name. With the halves apart the depth is found either way; in
  one buffer it is not (the rig's pinned symptom).

Board (99ae2d63, gl_test 123): the shared half proves the fix on silicon.
In one buffer, 3906 of 3906 judged pixels read "as if the depth were lost";
apart, 3906 of 3906 were right. The recreate half drew right in one buffer
too, for a reason the host rig replays: the scenes ran back to back, and
the recreate scene got the shared scene's renderbuffer names again. After
the orphan rule its FBO's id ((1 << 16) | 3) was the shared scene's fbo B
id, and that kept buffer's sloped depth, 0.5 + (x + y) / 4, happens to
give the right picture. Loading a new renderbuffer's undefined contents is
legal, so the case was blind. ec33ba1a starts each scene with no kept
buffer, and the recreate half in one buffer is printed, not judged. A dEQP
run reuses names across cases the same way, so what
recreate_stencilbuffer.no_rebind reads depends on the case before it.
Board, 4f0a735d (22:57, switch off, each scene fresh): case 123 PASS.
- In one buffer, both scenes lose the depth: shared 3906 of 3906 "as if
  the depth were lost"; recreate the same ("not judged (the depth lost, as
  the host rig predicts from a fresh start)").
- Apart, both are right: 3906 of 3906 each.
The fix holds for both halves on silicon.

texsubimage (4, and the i.MX8MP's same 4) does not reproduce on the rig.
The render into the texture is submitted before glTexSubImage2D writes, and
the texture is sampled from a copy of its current stamp. gl_test case 125
texsubfbo's first replica passed on the board: 0 of 16384 pixels differed,
both as rendered and as sampled. It was an RGBA source with NEAREST, a
solid sub-image and a 1:1 FBO, so it could not show a flipped or shifted
sub-image, or LINEAR on the window. ec33ba1a follows dEQP line by line:
- an RGB source, LINEAR with REPEAT;
- rgb and rgba textures;
- a gradient sub-image;
- after_render and between_render;
- the texture drawn LINEAR on the window.
Both the texture and the window are judged against a C reference, with
the mean difference per channel and the box of differing pixels.
Board, 4f0a735d: all four variants had 0 of 16384 texture pixels and 0 of
4096 window pixels differing (PASS). So the backend draws dEQP's sequence
right as far as the replica reaches, and the four FB2 failures lie
elsewhere: in what the replica still does not share with dEQP (its
metaballs content, the dEQP window's size and format, the reference's
own filtering), or outside this path.

**The texsubimage cause: a stamp for the job the draw ended.** The QPA of
the four cases on 5ff3076c (record_QPA_TSUB, decoded pictures) shows the
render into the texture and no sub-image, with GL_NO_ERROR. dEQP's
FboRenderCase::iterate clears the window before the case. That clear became
a job of its own and was still open when the render into the texture came.
The GL core stamped the texture as rendered by `kick_id()` = that clear
job, then the backend ended the clear job (another target) and recorded the
render into the next job. At glTexSubImage2D, `kick_pending(stamp)` said
the stamped job had run, so nothing was flushed: the CPU wrote the
sub-image, and the render ran over it at the next flush. gl_test 125 and
the host test had no window clear first, so the render opened a job with
the stamped id and the order was right. The same stamp covers everything a
draw uses (textures sampled, vertex and index buffers, staging), so any
draw that ends the open job itself (another target, a full job, no memory
for a kept copy) left its stamps naming a job that had run.

Fix (backend only): `v3d_draw` notes `kicks + 1` at entry, the id the GL
core stamped the draw with. If the draw is recorded into a later job, the
open job also answers to that id (`open_alias`), and `v3d_kick_pending`
says pending for it while that job is open; `flush_job` clears it. A write
to something only the ended job used, while the new job is open, now
flushes the new job too: an extra job, not a wrong picture. Host test:
a_sub_image_after_a_render_that_switched_jobs_waits_for_that_render (dEQP's
exact call sequence from the QPA). gl_test 125 texsubfbo now clears the
window first, as dEQP does.

## Render targets sampled from their render image, and their TFU-made mip chain (v3d-rt-sample)

**The problem** (tools/cts/RESULTS.md "Pi 4: the OSAL looks at the job in
flight", option (a)). The TMU does not sample a raster image, and the GL core
keeps every texture raster, so a texture a draw samples is tiled into a copy
(`tiled_copy`, "M6 tiled-copy cache"). A render target is the worst case of
it: every job that renders into the texture drops its copy (the stamp only
moves on CPU writes, so the copy cannot be keyed), and the next sample tiles
the whole target again, 7,684,096 bytes for glmark2 refract's 1600x1200
colour target, once a frame, on the CPU, from non-cacheable memory; and
while the copy stands it is a second 7.68 MB next to the storage. Refract
and terrain add a glGenerateMipmap of that target every frame: the GL core
box-filters 7.68 MB of non-cacheable storage on the CPU, and the copy is
then the whole 11-level chain. Mesa keeps every render target UIF, samples
it as it is, and has the TFU make its mipmaps.

**The design.** A texture the application renders into and samples gets a
*render image* (`RtImage`, `src/v3d/backend.rs`): Mesa's tiled layout of
the texture (`texture::rt_layout`: `v3d_tex_layout(w, h, levels, 4)`, with
level 0's UIF XOR dropped as the depth/stencil and 4x buffers drop it, so
the TLB, the TMU and the TFU read one layout and the CPU never tiles into
it; `levels` is 1, or the chain the texture is sampled with). A job
rendering into the target stores RT 0 twice (`v3d_job_target.uif_*`,
`emit_tile_list`): the raster store into the GL core's storage, as before,
and a second STORE_TILE_BUFFER_GENERAL of RT 0 into level 0 of the image,
UIF, sample 0, RGBA8, as Mesa stores one buffer to two surfaces when it
resolves a blit in the same tile list (v3dx_rcl.c:242-283) and as this
builder's 4x path stores the samples and then the resolve. The image is
never loaded: the raster target is, so no job reads an image before one has
written it. A draw that samples the texture then points its texture state
at the image (`rt_sample`): no copy, no tiling, no wait for the rendering
job (the GPU runs the jobs in order).

Why the raster store stays: every CPU path of the GL core -- glReadPixels,
glCopyTex*, glTexSubImage2D, the depth resolve -- reads and writes the
storage as raster rows, and so does the window system. Keeping the storage
current costs one more TLB store per tile (the GPU writing 7.68 MB a frame
for refract's target, against the CPU tiling 7.68 MB from non-cacheable
memory), and changes nothing the GL core does for level 0. The memory
saving of option (a) is the copy's budget share and its churn, not the
storage: the image is the copy's size.

**The chain.** glGenerateMipmap on a 2D RGBA8 texture is offered to the
backend first (`YakoglBackend::generate_mipmap`, `src/gl/texture.rs`).
The V3D answers 1 when the texture has a render image that holds its level
0 under the texture's current stamp: it submits the job rendering into the
target if that one is still open (glmark2 calls glGenerateMipmap right
after `glBindFramebuffer(0)`), waits for it, and runs one TFU job that
makes levels 1.. of the image from its level 0 (`v3d_tex_tfu_pack_mipmap`,
Mesa's v3d_generate_mipmap through v3dX(tfu) with for_mipmap: the source
in its own tiling, NUMMM the levels after the first, IOA with DIMTW so level
0 is not written over itself, the texture's real type so the filter averages
channels). The GL core then filters nothing, moves no stamp, records the
chain as present (`GlTexture::chain_in_backend`), and before any CPU read
or write of levels 1.. (every `settle_texture`, a level above 0 attached
to a framebuffer) asks the backend to write the chain back into the
storage (`return_mipmap_chain`, `rt_chain_back`: v3d_tex_load_box level
by level into the GL core's rows) -- so the storage is GL's chain again
whenever the CPU looks. The image's levels 1.. are GL's chain (`chain`)
from the TFU job until the GL core takes it back, the storage is written
under the image (a render into another view, a level above 0 attached), or
the image is laid out again; a render into level 0 keeps them, as GL keeps
the old mips until the next glGenerateMipmap. A mip-filtered sample reads
the image only while `chain` holds; a level-0 sample reads it whenever
level 0 is stored. The hook answers 0 (the CPU filters as before) for a
texture without an image, a stamp the image was not stored under, an image
laid out with fewer levels (laid out again for the next render), an OSAL
without the TFU, a level above 0 attached, or a TFU job that failed.

**When the image is what the storage holds.** The backend cannot see the
GL core's writes directly, but it sees their stamp: every CPU write of the
texels moves `content_gen` (texture.rs `stamp_texels`, through
`settle_texture`, `resolve_depth` and the layout). So:
- a texture is registered the first time a draw samples it after a render
  into it (`target_kick` not 0), with the stamp of that moment
  (`seen_gen`) and the levels it is sampled with; that first sample is a
  copy, as before;
- every job that renders into the target from then on stores the image, and
  a submitted job sets `stored` (`flush_job`);
- a sample whose stamp is `seen_gen` with `stored` set reads the image: the
  storage was last written by the jobs that wrote the image, and by nothing
  else, so the two agree;
- a sample whose stamp moved reads a copy, records the new stamp, and clears
  `stored` and `chain`: the next render stores the image under the new stamp
  (a render loads the raster target, which has the CPU's write) and the
  sample after it reads the image again;
- a sample with more levels than the image has lays it out again (the next
  render makes the new one); a render into another view of the same storage
  clears `stored` after the chain went back; `texture_released` drops the
  image; the image's memory is allocated by the first render after the
  registration (a texture sampled once after a render costs nothing), and
  the least recently used of `RT_IMAGES` (8) slots goes when the table is
  full, its chain written back first.
So a render-to-texture scene is copy-free from its second frame on
(refract: frame 1 registers the target with its 11 levels, frame 2's render
makes the chain image and its glGenerateMipmap is the TFU's, frame 2's
sample reads the image), and a scene that uploads into its render target
between frames is a copy for one frame after each upload: never a stale
image.

**What keeps the copy, by name** (`rt_texture`, `rt_eligible`):
- a depth texture: R32F storage the GL core resolves from the D24S8 buffer
  every frame (RESULTS option (b), sampling the buffer with the TMU's
  DEPTH24_X8 type: it needs the texture's object id in the binding, which
  `GlesTextureBind` does not carry); refract's and shadow's depth maps.
- a cube map or an array (one face is rendered; every layer is sampled);
  L8, A8, LA8 (stored narrow; a target is RGBA8); the sRGB class (never a
  target: a draw into it is refused by name); an EGLImage's texture
  (`bgra`, or no stamp); render targets 1..3 of a multi-target framebuffer
  (RT 0 alone has an image; `v3d_job_target` carries one); bgr565 and 4x
  targets (`v3d_job_begin` refuses an image with them).
- a texture a draw never rendered into (nothing stores an image from), and
  the first sample after a texture's registration.

**Evidence.**
- `tools/v3d/tex_check.c` part 1: the one-level layouts of 1600x1200 and
  800x600 against numbers derived by hand from Mesa's v3d_setup_slices (a
  1600x1200 image is exactly the raster storage's 7,680,000 bytes: the
  7,684,096 of the RESULTS lines less the backend's 4 KiB of alignment);
  part 10: the TFU mipmap job's fields against the layout for five sizes
  (UIF, UIF XOR as Mesa lays 1024x1024 out, UB-linear, LT), and its
  refusals.
- `tests/v3d_rt_sample.rs`, on the real v3d_tex.c (built with cc as the
  glmark2 memory replay builds its shim): refract's 11-level chain image
  derived by hand (11,001,856 bytes, level 0 at 3,321,856; a first
  derivation padded the 256 and 512 levels, which v3d_get_ub_pad does not at
  a multiple of 32 rows), the levels below 0 at the same distances whatever
  the count the layout was asked for (so a chain image serves a sample of
  fewer levels), the chain's TFU job register by register from Mesa's
  v3d_tfu.h constants (ICFG 14 << 18 | 4 << 9 | 10 << 5, IIS 150, IOA level
  0 | 6 << 3 | DIMTW), the boards' target sizes (800x600, 1024x600,
  1280x800 with Mesa's two rows of ub_pad, 1920x720, 1024x1024 where Mesa
  picks the XOR) and the small UB-linear and LT ones, the TLB's memory
  format and height in UIF blocks against the TMU state's UIF bits for one
  image, and that dropping the XOR changes the tiling word alone.
- `tools/v3d/job_check.c` part 20, through Mesa's CLE decoder: the second RT 0
  store right after the raster one (UIF no XOR, the padded height in UIF
  blocks, RGBA8, sample 0), no load from the image, and the refusals (4x,
  bgr565, no colour buffer).
- Backend tests (`src/v3d/backend_tests.rs`, "render images"): the first
  render stores no image and the first sample is a copy; the second render
  stores the image (its address, format and height) and the second sample's
  texture state points at it with no tiling and no copy; a CPU write sends
  the sample back to a copy until the next render; a mip-filtered target is
  a copy until the delegated glGenerateMipmap submits the open render, waits
  and runs one TFU job (DIMTW, two levels after the first, RGBA8) whose
  fake makes the box filter of level 0, after which the sample reads the
  image's level 0 with max level 2 and nothing is tiled, a render keeps the
  chain, and the chain asked back is in the storage's level 1 at the GL
  core's pitch; the hook answers 0 without an image, with a moved stamp,
  with fewer levels (the image laid out again), without the TFU, and after
  a failed TFU job (logged once); it waits for the render in flight; the
  unstamped and the never-rendered texture keep the copy; `texture_released`
  frees the image; no memory keeps the copy path (logged once) and the next
  render makes the image; the LRU slot goes when the table is full, its
  chain written back; a render into another view of the storage writes the
  chain back and invalidates the image.
- GL core tests (`src/gl/texture.rs`): a delegated glGenerateMipmap filters
  nothing, moves no stamp, records the chain as present and asks it back
  once before the next CPU write; a refused one filters on the CPU and asks
  nothing back; a new level 0 forgets the backend's chain; a level above 0
  attached asks the chain back and keeps later glGenerateMipmaps on the CPU.

**What was not run here.** The mipmap job's words were read from Mesa
26.2.1's v3dx_tfu.c (lines 97-163, the V3D 4.2 arm) and v3d_tfu.h in the
dependency layout (`mesa-26.2.1-full`, not vendored): NUMMM at ICFG bit 5,
TTYPE at bit 9, the source tiling in ICFG's format field at bit 18, OPAD at
bit 22 for every UIF destination (DIMTW or not: Mesa's comment says the
hardware infers the tiling of levels 1+, its code sets the field anyway, and
the packer follows the code), DIMTW at IOA bit 0 and the destination tiling
at IOA bit 3. The job has not run on the board. If the board's first refract
frame shows wrong minified levels, that is where to look; the TFU job's
failure path (an MMU fault, a timeout) already falls back to the CPU filter.

**Board expectation (Pi 4, not run here).** glmark2 33 ran / 0 failed / 0
unsupported as before; no "tiled copies high-water" line names a colour
render target: refract's 7,684,096-byte copy lines are gone (its target is
sampled from the image from its second frame on, its chain the TFU's) and
its fps rises from 4 (the CPU no longer filters 7.68 MB of non-cacheable
storage nor tiles 10 MB of chain each frame; what remains per frame is the
wait for the render job at glGenerateMipmap, which the CPU filter had too,
and one TFU job); terrain's render targets and desktop's windows likewise;
Slint's and the cluster's layer render targets are sampled from their
images. The shutdown line "render images: N registered, M draw bindings
sampled from one, ..., K mip chains made by the TFU" says whether the path
ran; a non-zero "failed" count there, or "the TFU did not make the N-level
chain" in the log, says the TFU refused the job and the CPU filtered.
