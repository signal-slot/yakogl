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
  hidden pass counter that select.rs gives the other back ends. Done by plan
  #185 loop-cap, a deliberate deviation from Mesa's words
  ([the fix](#the-fix-a-pass-counter-on-every-kept-loop-plan-185-loop-cap-33)).
  Steps 1-3 below are the study as it was made, before the fix.

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

`builtin_98_is_byte_equal_to_mesa_26_2_1` (tests.rs; since the fix
`builtin_98_is_mesa_26_2_1_with_the_loop_cap`) already covers case 98,
with the vertex program compiled for a program that reads no varying.
Case 104 has a reference (`alu_divloop.*`) but **no byte-equality test**
(since the fix: `alu_divloop_is_mesa_26_2_1_with_the_loop_cap`).
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

## The fix: a pass counter on every kept loop (plan #185 loop-cap, #33)

**A deliberate deviation from Mesa 26.2.1.** Mesa keeps these loops with the
GLSL test as their only exit. Our Mali back end now also gives every loop it
keeps a hidden counter of its head entries, with the same rule the GC7000
selection (`src/shaderc/online/select.rs` `loop_convert`) and the shared
unroller of the PowerVR and the V3D (`UNIFORM_LOOP_CAP`) use:

- **The limit** (`nir_fs.rs` `loop_cap_limit`):
  `loop_count::constant_loop_entries` for the loop, with the index's value
  before the loop read from the walk's constant locals. When it cannot count
  the loop, the limit is `LOOP_CAP` (`mali/select.rs`, the PowerVR's
  `UNIFORM_LOOP_CAP`, 4096). A counted loop's limit is its own number of
  head entries, so the guard never fires and the loop's results stay exact.
  A loop Mesa keeps has a run-time bound, so in practice every kept loop
  gets 4096. A constant `for` of 32 passes or fewer is unrolled and has no
  counter. Its words are unchanged.
- **Where** (`walk_kept_loop`, `walk_nodes`): a header phi after the locals'
  phis, its first source `limit`. At the loop's head test (the body's first
  `if`, `Lower::break_unless`), `iadd(phi, -1)` becomes the phi's latch
  source, `ilt(count, 0)` is the stop mask, and `ior(test, stop)` is the
  condition the `BRANCHZ` to the exit reads. `walk_if` takes that `ior` as a
  condition only in this form. A nested kept loop's counter restarts at
  every entry of the inner loop (its pre-header is in the outer body), as in
  `loop_convert`.
- **The words.** The counter counts down, not up as the study proposed (an
  `IADD_IMM rc, rc, #0x1`, then `ICMP_OR.u32.gt.m1` against the limit).
  That puts the limit in the pre-header's 32-bit immediate, and the compare
  reads the inline zero. Counting up needs `limit + 1` as a compare operand,
  which became an `IADD_IMM` in the loop (case 98) or a new FAU constant. The
  FAU constant moved every later uniform word (`u2.w1` became `u3.w0` in
  loop_exit's epilogue). Counting down adds no FAU word:

      pre-header  IADD_IMM.i32.reconverge rc, 0x0, #0x1000   (the phi's limit; takes
                                                              the pre-header's .reconverge)
      header      IADD_IMM.i32 rc, rc^, #0xFFFFFFFF
                  ICMP_OR.s32.lt.m1 rs, rc, 0x0, 0x0
      integer test: rs is its third source (va_fuse_cmp, opt.rs fuse_cmp)
                  ICMP_OR.s32.ge.m1 r7, r5, r3, r7^           (loop_exit)
      float test: one more word (va_fuse_cmp fuses only one compare family)
                  LSHIFT_OR.i32 r2, r2^, 0x0.b00, r5^         (builtin_98)

  So 3 words for a loop with an integer test and 4 for one with a float test.
  The counter's register is live across the loop. Entry `k` stops when
  `limit - k < 0`, so the body runs at most `limit` times: 4096 passes for 98's
  `count = 4097` and for 104's band 7.
- **The FAU tables, the preloads and the work register counts** of all five
  references are unchanged.

### The byte-equality tests (tests.rs `CapEdit`)

A test of a kept loop compares against the drm-shim reference with one
documented transform, `capped(region, edits)`:

- `CapEdit::Add(at, word)`: a word of the cap, inserted. `capped` refuses a
  word that is not one of the forms above.
- `CapEdit::Fix(at, word)`: a reference word rewritten. The rewrites are a
  register the counter moved, a branch offset over the cap's words, the
  pre-header word whose `.reconverge` the counter's start takes, and the
  integer test's third source `rs^` in place of `0x0`.

Every word that no edit names still compares against Mesa, as do the facts
and the padding. With the cap's words taken out, our program lines up one to
one with the reference in every test:

| Test (tests.rs) | Reference | Words (ours / Mesa) | Add | Fix |
|---|---|---|---|---|
| `builtin_98_is_mesa_26_2_1_with_the_loop_cap` | builtin_98 | 41 / 37 | 4 | 5 |
| `builtin_45_is_mesa_26_2_1_with_the_loop_cap` | builtin_45 | 24 / 21 | 3 | 7 |
| `intloop_loop_exit_is_mesa_26_2_1_with_the_loop_cap` | intloop_loop_exit | 26 / 23 | 3 | 7 |
| `intloop_divnest_is_mesa_26_2_1_with_the_loop_cap` (two kept loops) | intloop_divnest | 65 / 57 | 8 | 22 |
| `alu_divloop_is_mesa_26_2_1_with_the_loop_cap` (new, case 104's top half, `check_linked`) | alu_divloop | 48 / 44 | 4 | 5 |

The four existing tests were renamed from `*_is_byte_equal_to_mesa_26_2_1`.
The vertex programs of all five pairs are still byte-equal. No other
byte-equality test changed: every other population compiles to the same
words, because only `walk_kept_loop` makes the counter. The other kept-loop
references (`intloop_contloops`, `intloop_loop_nested`,
`intloop_loop_nested_vs`) are refused as before (`GuardMediump`,
`LoopMediump`, `LoopVertex`). A mediump kept loop is refused before the
counter is made. The vertex walk keeps no loop (`MaliError::LoopVertex`), so
it has no counter. If it ever keeps one, it needs the same counter.

The alternative was rejected: a Mali-only gl_test expectation (want
`0xfff00010` for 98). It cannot help 104, whose band 7 runs too long for
any program equal to Mesa's, and it would drop the termination guarantee
that 104 exists to test.

### Expected on the board (predictions, not yet run)

- 98 uniloop: check 4 (`count = 4097`) gives `0xfff00000` (n = 4096 passes,
  `last = 4095`), checks 1-3 unchanged: **PASS**.
- 104 divloop: the top half's band 7 stops at 4096 passes,
  `divloop_colour(4096, 255)` = `0xfff00000`, with no job timeout. The bottom
  half (`divnest.frag`, bounds of 7 or less, its counters never fire) is
  reached for the first time: **PASS** expected. A FAIL there would be a
  divnest frame or value issue that the FAULT has hidden so far.
- The board session must confirm both (twice each), then the sweep.
