<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali Valhall: why case 98 uniloop FAILs and case 104 divloop FAULTs on the T6

Host-only study (no board) of the two cases the loops-rt work made compile.
Both fail on the T6 in `tools/cts/log/2026-11-01-final3`:

    panthor-gltest: verdict case 98 uniloop: FAIL rc -1, first bad check 4 got 0xfff00010 want 0xfff00000
    panthor-gltest: verdict case 104 divloop: FAULT work lost 2, group state/fatal queues 0x1/0x0, vm usable true

(104 says `work lost 1` in the first case run and `work lost 2` in the
second case run and in both sweeps, with one kernel `job timeout` per run.)

## Result

- **The program words are byte-equal to the drm-shim references** for both
  cases: every region (`vs.position`, `vs.varying` where there is one,
  `fs.fragment`), the facts and the padding, when the pair is compiled
  linked, the way the driver compiles it.
- **The cause is a missing loop cap, not a wrong program and not the frame.**
  Mesa 26.2.1 keeps both loops as real loops whose only exit is the GLSL
  condition. Our Mali compiler copies that exactly. gl_test, though, expects
  the project's **deliberate deviation**: a loop the compiler cannot count
  stops after `UNIFORM_LOOP_CAP` = 4096 passes
  (`src/shaderc/online/powervr/unroll.rs:94`). The PowerVR and V3D back ends
  implement this cap (`src/shaderc/online/select.rs:2958-2987`,
  `src/shaderc/online/v3d/chain.rs:2693`). The Mali back end does not.
  - 98: the fourth draw sets `count = 4097`. The T6 runs 4097 passes, which
    is the correct GL result, and gl_test wants the capped 4096.
  - 104: band 7's bound is `1.0e9`. The T6 runs it for about 1e9 passes per
    fragment until the kernel's job timeout. Mesa's own program does the
    same: this draw cannot finish on any Mesa-equal program.
- So the fix is in the compiler (#33): give the Mali back end's kept loops the
  hidden pass counter that select.rs gives the other back ends.

## Step 1: the shaders and the values the sweep sets

| Case | gl_test.c | Shaders | Values |
|---|---|---|---|
| 98 uniloop | `case_uniloop`, `driver/tools/pvrlab/gl_test.c:21719` | `tools/shaders/es300/vid.vert` + `uniloop.frag` | `uniform float count`: one draw per quadrant with `counts[4] = {0, 1, 7, 4097}`. Wanted AABBGGRR: `0xff000000, 0xff000010, 0xff000070, 0xfff00000` (tolerance 3) |
| 104 divloop | `case_divloop`, `gl_test.c:22029` | top half `es300/bands.vert` + `divloop.frag`; bottom half `bands.vert` + `divnest.frag` | no uniform. `v_s = pos.x*0.5+0.5`, band `b = floor(v_s*8)`, `bound = b > 6.5 ? 1e9 : b`. Wanted: band b<7 loops b times, band 7 loops 4096 (`divloop_colour(4096,255)` = `0xfff00000`) |

Both comments in gl_test.c say the 4096 is a deviation: "4097 is past the cap:
the loop stops after 4096, a DELIBERATE deviation (unroll.rs
UNIFORM_LOOP_CAP ...)" and "band 7's bound (1e9) stops at the 4096 cap (a
DELIBERATE deviation; GL would never end)".

The testdata sources are byte copies of the sweep's:

    cmp tools/mali/testdata/builtin_98.frag  tools/shaders/es300/uniloop.frag   # equal
    cmp tools/mali/testdata/builtin_98.vert  tools/shaders/es300/vid.vert       # equal
    cmp tools/mali/testdata/alu_divloop.frag tools/shaders/es300/divloop.frag   # equal
    cmp tools/mali/testdata/alu_divloop.vert tools/shaders/es300/bands.vert     # equal

**The uniform values of the reference differ from the sweep's, and that does
not matter.** `builtin_98.expect.json` was drawn with gl_ref.py's default
value, `count = 16.0` (FAU word `u2.w0` = `41800000`). The sweep uses
0/1/7/4097. `count` is a pushed FAU word that the program reads
(`FCMP_OR.f32.ge.m1 r2, r2^, u2.w0`), so its value does not change any program
word. The only value-dependent word in 104 is the constant 1e9
(`IADD_IMM.i32 r2, 0x0, #0x4E6E6B28`, fs word 4), and it is in the program,
the same in the reference and in ours. No fresh drm-shim reference was
needed.

## Step 2: the compiled words against the references

`builtin_98_is_byte_equal_to_mesa_26_2_1` (tests.rs) already covers case 98,
with the vertex program compiled for a program that reads no varying.
Case 104 has a reference (`alu_divloop.*`) but **no byte-equality test**.
For this study I appended a throw-away test to
`src/shaderc/online/mali/tests.rs`, ran it and reverted it; it is not in the
tree:

    #[test]
    fn zz_study_loopfail_alu_divloop() {
        check_linked(td!("alu_divloop.vert"), td!("alu_divloop.frag"), td!("alu_divloop.expect.json"));
        check_linked(td!("builtin_98.vert"), td!("builtin_98.frag"), td!("builtin_98.expect.json"));
    }

    cargo test --no-default-features \
      --features host-test,gpu-mali,shaderc-online,no-shader-table --lib -- \
      mali::tests::zz_study_loopfail mali::tests::builtin_98
    # test ...::builtin_98_is_byte_equal_to_mesa_26_2_1 ... ok
    # test ...::zz_study_loopfail_alu_divloop ... ok

`check_linked` (tests.rs:440) compiles the fragment shader first, then the
vertex shader for the fragment's reads and moved expressions
(`fragment_reads` / `compile_vs_for_reads`). It compares every region of the
expect.json word for word, plus the facts (work registers, preload, FAU) and
the padding. A plain `check_program` on divloop fails on `vs.varying`, 4 words
against 5, because an unlinked vertex compile does not get Mesa's
`floor(v_s * 8.0)` multiply moved into it (`FMA.f32 r0, r0^, 0x48000000.h1,
0x0.neg`). That is a limitation of the test, not of the program the driver
links. The fragment program alone (`check_fs_stage`) is byte-equal too.

### The loops in the reference (equal to ours)

`builtin_98.expect.json`, fs.fragment (37 words):

    2  5110c200000000c0  IADD_IMM.i32.reconverge r2, 0x0, #0x0        i = 0 (pre-header)
    3  0091c30000000002  MOV.i32 r3, r2                               header
    4  0090c20000090042  S32_TO_F32 r2, r2^
    5  00f4c20280c08442  FCMP_OR.f32.ge.m1 r2, r2^, u2.w0, 0x0        float(i) >= count
    6  501fc02000000442  BRANCHZ.reconverge r2^.h0, offset:4         -> exit
    7  0114c03f80000040  FADD_IMM.f32 r0, r0^, #0x3F800000           n += 1
    8  0110c20000000103  IADD_IMM.i32 r2, r3, #0x1                   i++
    9  0091c10000000043  MOV.i32 r1, r3^                             last = i
    10 501fc017fffff8c0  BRANCHZ.eq.reconverge 0x0, offset:-8       back edge

`alu_divloop.expect.json`, fs.fragment (44 words). Words 8-16 are the same loop.
The compare is against a register (`FCMP_OR.f32.ge.m1 r3, r3^, r0, 0x0`, word
11), and that register holds `b > 6.5 ? 1e9 : b` (words 1-5, with
`0x4E6E6B28` = 1.0e9).

Neither program has a pass counter. The only exit is the GLSL condition.

## Step 3: what the board values say

The frame and the uniform push are correct:

- 98 checks 1-3 (`count` 0, 1, 7) pass, and the first bad check is 4. So the
  FAU push of `count` (`u2.w0`), the varyings and the epilogue are right
  for three different values.
- Check 4 got `0xfff00010`: red 0x10, green 0, blue 0xf0. Decode with the
  shader's colour: `n mod 16 = 1` (red 16), `floor(n/16) mod 16 = 0` (green
  0), `min(floor(n/256), 15) = 15` (blue 240). That is **n = 4097** (and
  `last + 1 == int(n)`, or the pixel would be white): exactly `count`
  passes, which is GL's answer. The wanted `0xfff00000` is n = 4096, the cap.
  An iteration-count bug (an off-by-one back edge) would also be wrong for
  counts 1 and 7. A branch-offset bug would not give the exact GL count. A
  uniform-value bug would be wrong for the first three checks too.
- 104: band 7 runs `float(i) < 1e9`. With the f32 rounding of `float(i)`,
  the loop leaves at i = 999,999,968, so about 1e9 passes of an 8-word loop
  per fragment. Band 7 is 8 columns by 32 rows of the 64x64 target (256
  fragments, 16 warps). That is many seconds of shader time, past the
  panthor job timeout. The kernel log matches: one `job timeout ...
  comm=panthor-gltest` per run, no fatal queue (`0x1/0x0`) and `vm usable
  true`. That reads as a timeout, not an MMU fault or an invalid
  instruction; this reading of the group-state bits is from the verdict
  text, not checked against panthor's source. The loop does end in the end, so it is
  not a branch that loops forever. It is a correct program that runs too
  long, and Mesa's program runs just as long.

## Proposed fix (later implementation item, compiler, #33)

Add the structural termination guarantee to the Mali back end's kept loops,
with the same rule as `select.rs` (2958-2987):

- **Where.** `src/shaderc/online/mali/nir_fs.rs` `walk_kept_loop` (:2840),
  which records the loop region (`f.nir.loops[lp]`: `pre`, `header`, `latch`,
  `exit`), and the header/latch emission that follows it. Also the vertex
  walk's kept loop, if one is ever kept there (today `MaliError::LoopVertex`
  refuses it).
- **The limit.** `crate::shaderc::online::loop_count::constant_loop_entries`
  for the loop, or `LOOP_CAP` (`select.rs:110`, 4096) when it cannot count
  the loop. A constant `for` over 32 passes keeps its own count, so the guard
  never fires and the loop stays exact.
- **The words.** Count the head entries, so that a `continue` is counted too:
  - pre-header: `IADD_IMM.i32 rc, 0x0, #0x0`
  - header: `IADD_IMM.i32 rc, rc, #0x1`, then
    `ICMP_OR.u32.gt.m1 rs, rc, <limit>, 0x0`, where the limit comes from an
    `IADD_IMM` or a constant FAU slot, whichever the scheduler and the FAU
    budget allow. Fold `rs` into the existing exit
    compare through Valhall's OR operand: `FCMP_OR.f32.ge.m1 r2, r2^, u2.w0,
    rs` in place of `..., 0x0`.

  That is 3 words more and one register live across the loop. The back
  edge's `offset:-8` and the exit's `offset:4` grow by the inserted words.
- **Byte-equality.** Mesa has no such counter, so every kept-loop reference
  (the testdata with a back edge, `grep -l 'offset:-'
  tools/mali/testdata/*.expect.json`: `builtin_98`, `builtin_45`,
  `intloop_loop_exit`, `intloop_divnest`, `alu_divloop`, and the refused
  `intloop_contloops`, `intloop_loop_nested`, `intloop_loop_nested_vs`)
  stops being byte-equal as is. The tests need one documented
  transform: the reference words, plus the cap words at named positions,
  plus the two branch offsets moved. Then every other word still compares
  against Mesa. This is a decision the issue has to record (a deliberate
  deviation from Mesa, the same one PowerVR and V3D already make). The
  alternative, a Mali-only gl_test expectation (want `0xfff00010` for 98), is
  rejected: it cannot help 104, whose band 7 runs too long for any program
  equal to Mesa's, and it would drop the termination guarantee that 104
  exists to test.
- **Add a byte test for 104**, `check_linked` on `alu_divloop.*`, which the
  tree lacks today.
- **Expected after the fix.** 98 check 4 gives `0xfff00000` (PASS). 104's top
  band 7 gives `0xfff00000`, and the case reaches its bottom half
  (`divnest.frag`, already byte-equal as `intloop_divnest`, small bounds),
  which the FAULT has hidden so far. Both need a board run to confirm.
