<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): a loop in a vertex shader, the `LoopVertex` refusal (row 14 of `mali-corpus-families.md`)

**Status: design, nothing implemented** (plan #336, item ana-locals-loops;
issue #33). Row 14 names two corpus sources, `tools/shaders/es100/loop_nested.vert`
and `loop_uniform.dowhile.vert`; neither is a sweep case. The F6 row of
`mali-remaining-families.md` and the `loop_nested_vs` section of
`mali-loop-break.md` (Part 2, "Plan for loops-rt", step 6) name the item
`loop-vertex`; this note measures the two sources on Mesa 26.2.1, puts the
block structure and the branch words next to the tip (f5bc25f4), and orders the
steps. Related: `mali-int-loops.md` (the fragment twin, "loop_nested: kept
loops with uniform bounds"), `mali-loop-board-failures.md` (the pass counter
every kept loop gets: section "The fix"), `mali-vs-abs-if.md` (the vertex `if`
flattening that `vs_steps`/`vs_join` implement).

"Measured" is a run in this session of Mesa 26.2.1 on the drm-shim
(`tools/mali/drm_shim_ref.sh run`, `BIFROST_MESA_DEBUG=shaders,shaderdb`,
GPU id 0xa867), each vertex source paired with `es100/va.frag`. The
`loop_nested.vert` run matches the committed reference
`tools/mali/testdata/intloop_loop_nested_vs.*` (53 instructions in the
VARYING program, 4 loops, 10 registers, 7 uniforms; the committed testdata
is the same source). `loop_uniform.dowhile.vert` has no reference in the tree.
Everything marked **inferred** was not measured.

## 1. The refusal sites on the tip

| where | what |
|---|---|
| `nir.rs:2326-2389` `vs_steps` | the vertex body as `Step`s; **`:2367`** a top-level `IrNode::Loop` becomes `Step::Refuse(MaliError::LoopVertex)` ("intloop_loop_nested_vs: Mesa keeps the loops as blocks", `:2366`) |
| `nir.rs:2342` | the same error for a `Loop` inside an `if` side (`side`), where the `if` would be flattened |
| `nir.rs:2506` | `build` returns `Err(e)` of a `Refuse` step at the node's place: an earlier refusal (an unlowerable instruction before the loop) comes first |
| `mod.rs:229`, `:333`, `:454` | the variant, its list entry, "a loop in a vertex shader is not lowered yet" |
| `tests.rs:1115`, `:1127` | pins: `while (true) { gl_Position = p; break; }` and the `intloop_loop_nested_vs.vert` reference |
| `tools/mali/corpus-verdicts.txt:68`, `:70` | `loop_nested.vert`, `loop_uniform.dowhile.vert` |
| `tools/mali/testdata/intloop_loop_nested_vs.{vert,frag,expect.json}` | the committed reference (not walked by a byte test: the test is the refusal pin at `tests.rs:1127`) |

What the vertex walk lacks (the three points of `mali-loop-break.md`'s table,
re-checked on the tip):

1. **Blocks.** `build` (`nir.rs:2478`) is one straight sequence; the only
   control flow is the flattened `if` (`Step::If/Else/EndIf`, `vs_join`
   `:2412`), which turns into `bcsel`s. The vertex selection has no
   `IfRegion`/`LoopRegion` use (`nir.rs:471`, `:532`: those are filled by the
   fragment walk, `nir_fs.rs` `walk_loop` `:3271`, `kept_loop` `:3334`,
   `walk_kept_loop` `:3514`).
2. **fp16 temporaries.** `a_coords` is a `highp` attribute but `coords`, `res`
   are `mediump vec4`: Mesa converts once (`f2f16`) and carries the vector
   across the back edge as two f16 pairs. The vertex `build` has `Alu::F2f16`
   (`nir.rs:104`) and sized nodes (`bits: 16`, `:1803`, `:3036` for
   `gl_PointSize` only), but no f16 vector phi or `MKVEC.v2i16`
   swizzle (**inferred**: not grepped beyond the `F2f16`/`bits` sites above).
3. **`int` uniforms by value.** `LoadUniform` of a `GL_INT` uniform is
   `UniformType` unless `index_only` (`nir.rs:2561-2563`), and an integer
   value is `IntValue` (`:2545` a bool constant, `:2685`, `:2786`, `:2811` the integer ALU arms). The loop bound
   `ui_two`/`ui_three` is compared (`ige_pan`), not used as an index.

The first refusal after `LoopVertex` is removed would therefore be
`UniformType` (**inferred**: the bound's `LoadUniform` is the first
instruction in the loop body, and the counter's `iadd` then `IntValue`).

## 2. What Mesa does

### Source lines (Mesa 26.2.1)

- `bifrost_nir.c:268` `nir_opt_loop_unroll` in the optimisation loop;
  `bifrost_compile.h:140` `max_unroll_iterations = 32`. Both loops here
  have a run-time bound (a uniform), so there is no limiting terminator
  and Mesa leaves them (`nir_opt_loop_unroll.c:1112-1114`, cited by
  `mali-loop-break.md` Part 2; not re-read this session).
- `bifrost_compile.c:3738` `emit_loop`: the header block
  (`continue_block`), the body's blocks, the latch jumping back
  (`bi_emit_jump`, `:85`), and the exit (`break_block`); `emit_if` `:3697`.
  The structure is the same in the vertex and the fragment stage: the
  vertex specifics are the IDVS split (`MESA_SHADER_POSITION` and
  `MESA_SHADER_VARYING` programs) and the `f16` store epilogue.
- `ige_pan` (a Bifrost-only NIR ALU op) is the loop test in the dump
  below; the front end's `i >= n` break test is the `if { break }` at the head.

### Measured: `loop_nested.vert` (two nests, 4 loops)

The VARYING program is 53 instructions (`6 cycles`, 512 bytes code, 10
registers, 7 uniforms, `4 loops`); the POSITION program is the usual 24
instructions (the loops feed only `v_color`).

NIR of the VARYING variant, abbreviated (blocks as printed):

    32   %0 = load_const 0
    32x4 %1 = @load_input (a_coords)
    16x2 %2 = f2f16 %1.xy        ; coords, as two vec2 f16
    16x2 %3 = f2f16 %1.zw
    32   %4 = load_const 1
    loop {                                          ; outer, nest 1 (b1)
      b1: %5 = phi b0: %2, b9: %10      ; res.xy
          %6 = phi b0: %3, b9: %11      ; res.zw
          %7 = phi b0: 0,    b9: %19    ; i
          %8 = @load_ubo (0, 0)          ; ui_two (range 4)
          %9 = ige_pan %7, %8
          if %9 { break } else { }
      loop {                                         ; inner (b5)
        b5: %10 = phi b4: %5, b8: %18
            %11 = phi b4: %6, b8: %17
            %12 = phi b4: 0,  b8: %16   ; j
            %14 = @load_ubo (0, 4)       ; ui_three, offset 4
            %15 = ige_pan %12, %14
            if %15 { break } else { }
        b8: %16 = iadd %12, 1
            %17 = vec2 %11.y, %10.x       ; res = res.yzwx
            %18 = vec2 %10.y, %11.x
      }
      b9: %19 = iadd %7, 1
    }
    ... the second nest (b11-b19): the same, with `res = coords.wxyz` before
        the inner loop and `coords = res` after it;
        the inner bound `%8` (ui_two) is reused, not loaded again (the
        outer's `%25` is a fresh load of ui_three)
    b20: %37..%40 = f2f32 (%5.x, %5.y, %20.x, %20.y)     ; v_color = vec4(a.xy, res.zw)
         four store_global (estream) at the idvs output buffer offsets 0,4,8,12

Notes on the NIR: the loop bound is a `load_ubo` **inside** the loop (at
the header, loaded each iteration), the break test is the first thing in
the header, there is no `continue` construct (for `i++` is the latch's
`iadd`), and `res.yzwx` is two `vec2` rebuilds from the halves, not a
`vec4` shuffle.

The disassembly, VARYING program (block structure and branches only; `r6`
etc. as RA assigned; `.reconverge` is the flag on a loop header and an
exit):

    block0:  LD_ATTR_IMM.v4.f32 @r0:r1:r2:r3, r60, r61, index:1, table:1
             V2F32_TO_V2F16 r0, r0^, r1^      ; coords.xy
             V2F32_TO_V2F16 r1, r2^, r3^      ; coords.zw
             MOV.i32 r2, r0                   ; res copies (phi inputs)
             MOV.i32 r3, r1
             IADD_IMM.i32.reconverge r4, 0x0, #0x0           ; i = 0
    block1:  ICMP_OR.s32.ge.m1 r5, r4, u5.w1, 0x0            ; i >= ui_two  (u5.w1)
             BRANCHZ.reconverge r5^.h0, offset:11            ; -> exit if the compare is not zero  (see below)
      block4: IADD_IMM.i32.reconverge r5, 0x0, #0x0          ; j = 0
      block5: MOV.i32 r6, r3^
              ICMP_OR.s32.ge.m1 r3, r5, u6.w0, 0x0           ; j >= ui_three (u6.w0)
              BRANCHZ.reconverge r3^.h0, offset:4
      block8: MKVEC.v2i16 r3, r6.h1, r2.h0                   ; res = res.yzwx
              MKVEC.v2i16 r2, r2^.h1, r6^.h0
              IADD_IMM.i32 r5, r5^, #0x1                     ; j++
              BRANCHZ.eq.reconverge 0x0, offset:-7           ; back edge: always
      block9: IADD_IMM.i32 r4, r4^, #0x1                     ; i++
              MOV.i32 r3, r6^
              BRANCHZ.eq.reconverge 0x0, offset:-13          ; outer back edge
    block10 .. block19: the second nest, the same words (`offset:13`, `4`, `-7`, `-15`)
    block20: LEA_BUF_IMM.slot1 @r0:r1, r59^, table:0xD
             F16_TO_F32.wait1 r4, r2.h0 ; STORE.i32.estream.slot2 ... (four lanes,
             SHADDX.s64 offsets from u4.w0, u4.w1, u5.w0)  .end

- **Compare:** `ICMP_OR.s32.ge.m1 rD, counter, u<bound>, 0x0` -- the bound is
  an FAU word read directly (the uniform's slot: `ui_two` = `u5.w1`,
  `ui_three` = `u6.w0`: the first words after the vertex stage's own
  `u0..u5.w0`: viewport scale/offset and the varying offsets `u4.w0`,
  `u4.w1`, `u5.w0`).
- **Exit test:** the pre-RA print is `BRANCHZ.i16.eq %9^.h00, u256 -> block3
  block2` for the NIR `if (c) break` and, after the empty-block removal, the
  final print is `BRANCHZ.i16.flow10.ne r5^.h00, u256 -> block10`: a forward
  branch **taken when the compare is nonzero**, to the exit. The assembled
  word is `BRANCHZ.reconverge r5^.h0, offset:11`; the offset counts
  instructions forward (the outer test skips 11 words of the first nest).
- **Back edge:** `BRANCHZ.eq.reconverge 0x0, offset:-N` -- an always-taken
  `BRANCHZ.eq` of `0x0` (the final print is `BRANCHZ.i16.flow10.eq u256, u256 -> block5`), a negative
  offset to the header. There is no `JUMP`/`BRANCH` word. (This is the
  fragment loops' form: `mali-int-loops.md` "loop_nested", `offset:-7`.)
- **Counters:** `IADD_IMM.i32 rN, rN^, #0x1` in the latch; initial `IADD_IMM.i32 ... #0x0` in
  the block before the header. No cap or pass counter in Mesa's words
  (`mali-loop-board-failures.md`'s "pass counter on every kept loop" is
  yakogl's own addition for the fragment stage after board faults; **inferred**
  that a vertex loop needs the same until the board shows otherwise -- the
  vertex stage runs on the same cores).
- **No unroll, no hoist.** All four loops are in the program; the second
  nest's outer body is entered even for a bound of zero.
- **Registers:** the loop-carried `res` stays in `r0:r3` across the loop
  (`MOV` into `r2`, `r3` before the header and `MKVEC.v2i16` re-pairing).
- **Flow control:** `.flow10`/`.flow4` bits in the final dump come from
  the scoreboard pass over the back edge; the unlowered (pre-RA) text has
  them only on the first instruction of a header.

### Measured: `loop_uniform.dowhile.vert` (one do-while, 29 instructions)

    do { res = res.yzwx; } while (++i < ui_three);      ; v_color = vec4(res.rgb, 1.0)

VARYING program: **29 instructions, 11 registers, 6 uniforms, 1 loop**; POSITION
24 (the common one). NIR:

    loop {
      b1: %5 = phi b0: %2, b4: %12 ; %6 = phi b0: %3, b4: %11 ; %7 = phi b0: 0, b4: %8
          %8  = iadd %7, 1                      ; ++i first
          %9  = @load_ubo (0, 0)                ; ui_three (the only used uniform: offset 0)
          %10 = ige_pan %8, %9
          if %10 { break }
      b4: %11 = vec2 %6.y, %5.x ; %12 = vec2 %5.y, %6.x
    }
    b5: %13 = f2f32 %5.y ; %14 = f2f32 %6.x ; %15 = f2f32 %6.y    ; res.rgb after one more rotation

The do-while is the same loop with the test **moved before the body**: the
increment and the compare come first and the `yzwx` of the last pass is not in
the loop: it is read from the phi through a shifted swizzle in the exit
block (`%5.y, %6.x, %6.y` for `res.xyz`, so the stores use the rotation of the
phi, not a second `MKVEC`). That is a NIR rewrite (the body's last pass is
peeled into the exit by the optimiser, the mechanism **inferred**; the NIR
is measured). The final program:

    block0:  LD_ATTR_IMM.v4.f32 @r0:r1:r2:r3, r60, r61, index:1, table:1
             V2F32_TO_V2F16 r0, ..  ; V2F32_TO_V2F16 r1, ..
             IADD_IMM.i32.reconverge r2, 0x0, #0x0                   ; i = 0
    block1:  MOV.i32 r3, r1^
             IADD_IMM.i32 r2, r2^, #0x1                              ; ++i
             ICMP_OR.s32.ge.m1 r1, r2, u5.w1, 0x0                    ; i >= ui_three
             BRANCHZ.reconverge r1^.h0, offset:3                     ; -> block5 (exit)
    block4:  MKVEC.v2i16 r1, r3.h1, r0.h0 ; MKVEC.v2i16 r0, r0^.h1, r3^.h0
             BRANCHZ.eq.reconverge 0x0, offset:-7                    ; back edge
    block5:  F16_TO_F32 r0, r0^.h1 ; F16_TO_F32 r1, r3.h0 ; F16_TO_F32 r2, r3^.h1
             LEA_BUF_IMM ..., STORE.i32.estream x3, IADD_IMM.i32 r2, 0x0, #0x3F800000 ; STORE (w = 1.0)

`ui_three` is `u5.w1` here (the other six uniforms are unused and dropped,
not packed), `u6` unused: **seven uniforms declared, one FAU word**.

## 3. Lowering steps

Order matters: 1-2 are needed by both sources; 3-5 are the loop itself.
Everything goes through the existing kept-loop machinery of the fragment
walk where it applies (`nir.rs` `IfRegion`/`LoopRegion` are the stage-
independent structures, and the back-end passes -- `sched.rs`, `ra.rs`, `flow.rs`,
`opt.rs` -- already take back edges since the fragment's `loop_exit`, plan
#154).

1. **`int` uniforms by value and integer compares in `build`.** `nir.rs:2561-2563`:
   accept a `GL_INT` uniform read that is the right operand of an integer
   compare (the loop test), as an FAU word read raw (`mali-int-uniforms.md`
   section 2: "int and bool are one 32-bit word a component"); add
   `Alu::Icmp(ge, s32)` and `Alu::Iadd` for the counter to the vertex
   `build` (the fragment's `Fs::lane32` has them), and `ConstI32` already
   exists (`:2544`). The mediump int is a 32-bit word (`ICMP_OR.s32`,
   not 16-bit). Tests: unit tests in `tests.rs` for an unrolled-away
   counter must not change (`for (i < 4)` is still unrolled by the front end).
2. **fp16 vectors in the vertex walk.** `coords = a_coords` (a mediump
   assignment of a highp attribute) -> `V2F32_TO_V2F16` pairs (the word
   exists in `bir.rs`, used by the fragment's mediump path), `yzwx` -> two
   `MKVEC.v2i16` (swizzle `.h1/.h0` operands), and the stores
   `F16_TO_F32` per lane. Carry them as 16-bit two-lane nodes
   (`NirNode.bits = 16`, `width = 2`; `nir_fs.rs` `phi_src` `:1085`
   is the model for a vector phi). Reference without a loop first:
   an unrolled mediump vertex temporary (`for (i < 3)` rotating `res`) is
   byte-equal to Mesa -- needs its own reference (**not in the tree**).
3. **A vertex `Step::Loop`.** In `vs_steps` (`nir.rs:2326`) replace
   `Step::Refuse(LoopVertex)` at `:2367` by `Step::LoopBegin`/`LoopEnd`
   steps that wrap the body's steps (flattened `if`s stay), and in `build`
   (`:2506`) turn them into the `LoopRegion { pre, header, latch, exit }`
   blocks as `walk_kept_loop` (`nir_fs.rs:3514`) does: one phi per local the
   body stores, in `registration_order` (`nir.rs:845`), the break edge, the
   exit block. The vertex walk keeps `w.locals`; save/restore as `Step::If`
   does (`pre`/`then`). The loop test is the loop head's `if {break}`; the
   walk must not fold the counter.
   Delete the check at `:2342` (a loop in an `if` side) only if a reference
   exists; otherwise leave it refusing as `LoopVertex` (**decision**).
4. **Emission.** The fragment emitter's `end_block` (`nir_fs.rs:8315`) and
   `select_highp` build the blocks and call `emit_loop`'s order; the vertex
   selection (`select.rs`) needs the same block emission for the VARYING
   program and for the POSITION program (here empty: the loop feeds only
   varyings; a loop that feeds `gl_Position` appears in both: **inferred**,
   no vertex reference has one). `ICMP_OR.s32.ge.m1` against the FAU word,
   `BRANCHZ` forward on the compare, `BRANCHZ.eq 0, 0` back; the inversions
   and the offsets come out of `opt.rs`'s `try_remove_simple_block` as the
   fragment's do. The pass counter of `mali-loop-board-failures.md` must be
   added to the vertex loops or the board fault of case 98 repeats
   (**inferred**: the vertex stage has not been run on the board with a kept
   loop). A byte-equal test cannot contain the counter if Mesa has none:
   use `CapEdit` (`tests.rs`, `check_linked_capped`, `:499`) as
   `intloop_loop_exit_is_mesa_26_2_1_with_the_loop_cap` does.
5. **do-while.** The test-before-body form of `loop_uniform.dowhile.vert`
   (the `iadd` before the compare, the last body pass read by swizzle in
   the exit block) is how Mesa's optimiser leaves it. Our front end's
   `Loop` for a do-while has the test at the tail: match "body then
   `if (!c) break`" and walk it as Mesa's rotated form: the phi takes the body's
   values and the exit block reads them after one more rotation. This is a
   walk-level rewrite (like `same_instrs` for the `for` step,
   `mali-loop-break.md` "Where the implementation differs"). Do the
   nested for-loop reference (`loop_nested.vert`) first; this second.

## 4. Host tests to add

- `tests.rs`: `intloop_loop_nested_vs_is_byte_equal_to_mesa_26_2_1` over
  the committed `intloop_loop_nested_vs.{vert,frag,expect.json}` with
  `check_linked`/`check_program` (the committed JSON carries the 53
  words), plus `loop_exit`-style capped variant if the pass counter is
  added; remove the `LoopVertex` pin at `tests.rs:1127` (keep `:1115`,
  or move it to a different gap if `while (true)` takes a different
  path).
- A second reference for the do-while: `loop_uniform.dowhile.vert` + `va.frag`
  through `drm_shim_ref.sh run` and `pan_ref_diff.py extract --draw` into
  `tools/mali/testdata/intloop_dowhile_vs.*` (not committed by this item:
  testdata is out of its contract); its stderr above (29 instructions, 11
  registers) is the check until then.
- `corpus-verdicts.sh --write` moves lines 68 and 70 of
  `corpus-verdicts.txt`.

## 5. What it moves

- **Corpus sources:** `es100/loop_nested.vert` (step 1-4) and
  `es100/loop_uniform.dowhile.vert` (step 5). Their verdicts become the next
  refusal or a pass; the nested one is a pass if the byte test passes.
- **gl_test:** none; the sweep has no vertex loop (the sweep's `LoopVertex`
  count is 0, the 15 REFUSED of #309 are other families).
- **dEQP-GLES2** (from the sources' headers): `shaders.loops.for_uniform_iterations`
  `nested`/`nested_tricky_dataflow_2` (vertex) and
  `do_while_uniform_iterations.basic_mediump_int_vertex`. The generator that makes
  them (`es2fShaderLoopTests.cpp`) makes about as many `_vertex` variants of
  each loops test; which share this form is **inferred**, not tallied here
  (`corpus-tally` is the item for that).
- **Depends on:** the fragment kept-loop emitter (landed for highp; the mediump
  fragment `LoopMediump` is a sibling that shares step 2's fp16
  carry, `mali-loop-break.md` Part 2 step 5; do them together or fragment
  first). `nir.rs` is shared with vs-lookups: start after it merges.

## 6. Open points

- Whether the vertex POSITION program of a loop that feeds `gl_Position` is
  the same words (needs a reference with such a loop).
- Whether the vertex stage needs the pass counter (it is not in Mesa's words;
  the board's behaviour on a vertex loop is unmeasured).
- `ui_zero..ui_six` of the do-while source are unused and dropped by Mesa; the
  front end's uniform table must drop them the same way for the FAU word
  to be `u5.w1` (the existing "dead uniform" rule is `mali-uniform-*`:
  **inferred** to apply here).
