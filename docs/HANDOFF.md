<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Handoff

The state of the work and what comes next. Update this file in the same
commit as any result that changes it. Last update: 2026-10-05.

Board addresses, UART paths and credentials never go in this file; they come
from the environment (`BOARD_HOST`, `BOARD_UART`, ...).

## Goal, in order

1. ES 2.0 finished on the three boards (AM62P PowerVR, i.MX8MP GC7000,
   Raspberry Pi 4 V3D), each booted from its own storage by its bootloader.
2. Minimum performance.
3. A public website (`website/`). It goes live when the repository is public.
4. ES 3.1 / 3.2.

## State of main

Results are from main `81c53c61` (gl_test) and `7dada1ff` (Slint, cluster, table image), except the Pi 4. All three boards boot standalone from eMMC (the two Verdins, U-Boot) or SD (Pi 4).
`docs/standalone-boot.md` has the procedure.

| Board | gl_test | Slint | Cluster |
|---|---|---|---|
| AM62P | 48 / 48, 0 resets (the set now has 192 `predlive`) | pass on `7dada1ff` (705 frames in 10007 ms) | 91.8 fps, 56.4 Mpx/s, 16.28 ms/Mpx on `7dada1ff` (`39bd6983`: 91.2 fps, 56.0 Mpx/s, 16.11 ms/Mpx) |
| i.MX8MP | fails: first failure case 16 check 255 (4x MSAA), the same as on `39bd6983`, where it was 45 / 47 | not run on `81c53c61` | not run on `81c53c61` |
| Pi 4 | 12 / 12 on `afa12e9b` (not run since) | not run since | not run since |

AM62P, the fill shader from femtovg (`fill_color`), 3D busy per frame:

| Measurement | Run-time compiler `81c53c61` | `39bd6983` | Mesa table (`afa12e9b`) |
|---|---|---|---|
| gl_test case 39, perf 2 (16 full-screen quads) | 77.19 ms | 81.1 ms | 83.4 ms |
| case 186 arm 0 (blend) | 76.98 ms | 80.9 ms | 83.4 ms |
| case 186 arm 1 (nodiscard, blend) | 54.56 ms | 55.3 ms | 54.9 ms |
| case 186 arm 2 (blend off) | 68.93 ms | 69.8 ms | 71.6 ms |
| case 186 arm 3 (nodiscard, blend off) | 44.00 ms | | |
| case 187 arms 0 / 1 / 2 / 3 | 81.02 / 59.05 / 72.52 / 47.94 ms | | |

The table column comes from an earlier image. No table image of `81c53c61`
was run.

Case 193 `offarm` (`7dada1ff`): 186 arm 1 with the never-run arm grown by N
groups, 3D busy per frame (record `20261005-2118-case193-7dada1ff-ad3d86cf`
online, `20261005-2119-case193-table-7dada1ff-ed2efe33` table; pixels equal
at every N):

| Added groups N | Run-time compiler | Table |
|---|---|---|
| 0 | 54564 us | 54851 us |
| 8 | 55415 us | 56008 us |
| 32 | 59323 us | 60633 us |
| 128 | 91142 us | 91745 us |

The offline PowerVR table (`src/shaderc/blobs.rs`) records, since `0fe778f6`,
the toolchain that reproduces it byte for byte: a `pco_shc` built by
`tools/pco_shc/build_pco_shc.sh` with the patched usclib
(`// toolchain: pco_shc=f537fc86b41f glslang=9e007f4bf514`). That commit also
adds `matattr`'s 20 rows and re-keys `texsize.frag` to its current text.
On the AM62P (`60ffeec8`, table image) the regression set passes 39 / 40,
0 resets; the one failure is case 48 `es3vol`, which was already failing in
table images. Case 46 `es3size` (it uses `texsize.frag`) passes now; it was
refused before the re-key. Case 63 `matattr` passes, served from the table
for the first time.
The GC7000 table still lags on `matattr`: issue #22, not fixed by `0fe778f6`.

## Open work, in order

1. **Branch or predicate: choose by the never-run arm's size.** A
   predicated-off group costs time (gl_test 193, above). Per added group per
   frame: run-time compiler +0.107 ms at N = 8, +0.149 ms at N = 32,
   +0.286 ms at N = 128; table +0.145, +0.181, +0.288 ms. An executed group
   costs about 0.8 ms a frame (184, 185), so 14 to 36 %. The cost per group
   rises with N; the cause is not measured (instruction fetch is a
   candidate). Both paths have the same slope at N = 128. Predicating both
   arms as pco does (`afa12e9b`) therefore loses to a branch once the
   never-run arm is large enough. Where the crossover lies is not known.
   - The finer N series, gl_test 194 `offarmn` (`e34ba09b`, 14 values of N,
     pixels equal at every N; records `20261006-0021-case194-e34ba09b-bf2afe0b`
     online, `20261006-0023-case194-table-e34ba09b-5c92d5c1` table), 3D busy
     per frame (online / table, ms): N = 0 54.56 / 54.85; 8 55.42 / 56.10;
     16 56.40 / 57.40; 32 59.33 / 60.52; 64 68.36 / 69.37; 128 91.15 / 91.71;
     256 139.87 / 140.31. Marginal cost per added off-arm group, online
     (difference / group step): 0 to 8 0.107 ms, 16 to 24 0.162, 32 to 48
     0.257, 64 to 96 0.346, 128 to 192 0.378, 192 to 256 0.384. A small
     never-run arm costs about 0.11 ms a group, the marginal cost rises with
     the arm's size and levels off near 0.38 ms a group beyond N = 128, about
     half an executed group (0.8 ms, 184 / 185). The table image has the
     same shape, 0.3 to 1.3 ms above the online image at every N. The cause
     of the rise is not measured; instruction fetch is a candidate.
   - The cost of a branch, gl_test 195 `ifbranch` (`97ba03d4`; record
     `20261006-0140-case195-97ba03d4-d4863a05`, 8 pixels equal): 193's
     programs both ways on one context, the branched form behind
     `PVR_BACKEND_OPT_IF_LOWERING` (pco's `lower_if_cond_exec`, six control
     groups; the default still predicates). 3D busy per frame, predicate /
     branch: N = 0 54562 / 55337 us; 8 55420 / 55338; 32 59331 / 55338;
     128 91144 / 55338. The branched form costs 55.34 ms at every N: the
     never-run arm is skipped. Against the predicated form it costs +0.78 ms
     at N = 0 (55337 - 54562 = 775 us: the six control groups and the
     branch, less the 31 base groups it no longer issues), and wins from
     N = 7 on (0.78 / 0.107 ms per group), that is, for this program, once
     the never-run arm exceeds about 38 groups (31 + 7). At N = 128 it saves
     35.8 ms (91144 - 55338 us). `fill_color`'s own scissor arm (31 groups)
     therefore stays predicated, which is the default.
   - The branched form with the arm taken (scissorRadius 4), gl_test 195
     again on `39a5dc39` (record `20261006-0209-case195-39a5dc39-b665ee9b`;
     pixels ff000099 with the arm off, ff000088 taken). 3D busy per frame,
     us, predicate off / on, branch off / on: N = 0 54563 / 59046, 55338 /
     61921; 8 55415 / 66907, 55338 / 69702; 32 59342 / 86005, 55338 / 88325;
     128 91133 / 163104, 55338 / 162452. With the arm taken the branch costs
     more than the predicated form by 2.88 ms at N = 0 (61921 - 59046), 2.80
     at N = 8, 2.32 at N = 32, and 0.65 ms less at N = 128 (the predicated
     form then pays the 23-group else arm masked at the large-arm rate). An
     executed group costs about 0.79 ms here ((66907 - 59046) / 10 = 786 us:
     N = 0 to 8 adds 10 groups to the then arm, the 8 fmad and two uniform
     copies, not 8; 184 / 185 measured about 0.8). The branch's fixed cost, the six control groups plus the taken
     branch, comes out at about 4 to 5 ms a frame either way: radius 0 gives
     0.78 ms net plus the 31 masked groups it no longer issues (about 3.3 ms
     at 0.107 ms each), radius 4 gives 2.88 ms net plus the 23 masked groups
     (about 2.5 ms); 6.8 executed groups (5.2 with the arm off). Branching saves the masked
     cost of the arm it skips (194's curve, 0.107 to 0.38 ms a group). The
     skipped arm is not known at compile time, so the rule that never loses
     is: branch when both arms' masked cost exceeds the fixed cost. For
     `fill_color` (31 and 23 groups, about 3.3 and 2.5 ms) predicating, the
     default, is right; for a symmetric `if` the crossover is about 38
     groups an arm by this estimate (49 in the rule below, which uses the
     larger fixed cost).
   - The rule as landed (`5f4d3677`, `select::branch_pays`): the default
     lowering branches an `if` iff the masked cost of EACH arm exceeds
     `BRANCH_FIXED_US` = 5336 us; otherwise it predicates (the options
     PREDICATE and BRANCH still force a form). The branch's fixed cost is
     the larger of 195's two readings: arm taken 2875 + 23 x 107 = 5336 us,
     arm off 775 + 31 x 107 = 4092 us. The masked cost of an arm is a table
     of 194's marginal costs by arm size (107 us a group up to 41 groups,
     122 to 49, 162 to 57, 204 to 65, 257 to 81, 307 to 97, 346 to 129, 367
     to 161, 378 to 225, 384 beyond). The rule is crossed at 49 groups an
     arm. On the corpus 0 of 168 `if`s branch (the largest smaller arm of
     any `if` is 36 groups); `fill_color` (31 / 23) and the offarm sources
     (else arm 23) stay predicated, and the one-armed `if`s have nothing to
     save when the arm runs. The 36 ms saved at N = 128 needs knowing which
     arm runs. Not modelled: 195 at N = 128 with the arm taken, where the
     branch is 652 us cheaper than the model's +2875.
     Board run of the rule: the corpus bytes are unchanged, and the gl_test
     full set on the AM62P passes 48 / 48, 0 resets (record
     20261006-0403-gt-5f4d3677-ecce9b75).
   - Next, proposed (not done): a per-draw variant chosen by the GL core from
     the uniform condition's value, like the blend and sampler-state
     variants: predicated when the long arm runs, branched when it is
     skipped. `fill_color` at N = 128: 91.1 to 55.3 ms with the arm off, no
     loss with it on.
2. **Compile shaders at build time with our own compiler.** Shaders known
   at build time are compiled then; the run-time compiler runs only on a
   miss. An image without the run-time compiler refuses a miss by name.
   Mesa's `pco_shc` / `etna_shc` then compile nothing.
   Done so far: the one source list every table generator reads
   (`tools/shaders/precompiled.list`) and `yakogl-precompile`, which
   compiles it with our compiler on the host into `build/precompiled/`
   (tools/README.md); next, an image carries that table, and the Mesa
   tables become host-test reference material only.
   Then rename the two paths to "precompiled shaders" and "shader compiler"
   (features `precompiled-shaders` and `shader-compiler`; the source moves
   from `src/shaderc/online` to `src/shaderc/compiler`). The words online,
   offline and remote go away.
3. **Design clean-up**:
   - `DESIGN.md` and `docs/` describe the current design, not its history.
   - About 50 debug and experiment globals in product code are removed or
     moved out.
   - The 26 samples are cut to the few that are needed.
   - The bare-metal claim is made true, or it is removed.
   - The `SAFEUI_*` and `powervr-bxs` names are removed.
   - Our remaining C (the OSAL, `gl_test.c`) moves to Rust.
4. **Known defects.** Each has a GitHub issue; what remains is the fixes:
   - i.MX8MP gl_test case 10 fails about 1 run in 3 (nondeterminism; the
     cause is not found) (#9);
   - i.MX8MP case 16 (4x MSAA) fails (#8);
   - the Pi 4 differs from the ES 2.0 reference in 25 cube cases (#12).

## How to work here

- The gate is `tools/check.sh`. Run it from a clean detached checkout of
  the exact commit, with `driver/src/fw/pvr_fw_blob.h` copied in, and with
  `ZEPHYR_WS=<west workspace>` and `HAL_NXP_DIR=<hal_nxp>` set. Without
  them the Zephyr and hal_nxp rows are gaps and the gate is red. The
  Mesa-table reproduction row also needs `PCO_SHC=<pco_shc from
  tools/pco_shc/build_pco_shc.sh, content id f537fc86b41f>`,
  `GLSLANG=<glslangValidator>` and, for speed, `GLSL2USC_CACHE=<dir>`;
  without them that row is a gap and the gate is red. It takes about 45
  minutes.
- Board runs use `tools/board.sh <board> run <zephyr.bin> <record dir>
  <label>`. Run them from a detached checkout of main, and only with images
  built from commits on origin/main. Use `BOARD_BOOT=uboot-mmc` on the
  Verdins.
- One GL client per board. Check that a board is idle before you use it.
