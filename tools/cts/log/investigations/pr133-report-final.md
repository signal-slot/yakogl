<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PR #133 / issue #133: final board report (plan #141, item report-update)

Written 2026-10-13. This file holds the exact text of the two comments and,
at the end, where each was posted. Every cell below is copied from a record in
`tools/cts/log/`; nothing else is quoted. No board address, login or bridge
port appears here. "Not run" means not run; it is never a pass.

Records read: `board-sessions/imx8mp-pr133.md`, `imx8mp-pr133-control.md`,
`imx8mp-pr133-baseline.md`, `pi4-pr133.md` (section "Third attempt"),
`pi4-pr133-baseline.md`, `am62p-pr133-main.md`, `am62p-pr133.md`,
`am62p-pr133-baseline.md`; `investigations/pr133-case-check.md`,
`pr133-board-images.md`, `am62p-console-map.md`.

---

## PR comment (post on PR #133)

<!-- ===== begin PR comment ===== -->

### Board measurement of PR #133: gl_test case 206 (`es3levels`)

**Summary.** The case is gl_test case **206**. On the host, the PR's storage
test fails without the PR and passes with it. On the boards: **i.MX8MP ran:
case 206 passed, and the control without the PR failed.** **Raspberry Pi 4
ran, but case 206 is skipped there by design, so it says nothing about this PR.
AM62P did not run.**

**Commit.** `work/int-pr49-board` was **not pushed** to origin. The images come
from commit `066a2db81d40e519efc05dcbdcdd4e359468fe9a` (`066a2db8`,
"gl_test: case 206 reads back 3D and 2D-array mip levels 1 and 2 (PR #133)"),
which is PR #133's head `af9ecc53` merged with origin/main `2fe4b879`, plus the
case. It is on `origin/ccfarm/01a12f31-af26-7505-a35a-153b04454dae`.

**The case.** `driver/tools/pvrlab/gl_test.c` case **206**, `es3levels`. It
uploads a 16x16x4 3D texture and a 16x16x2 2D array. Levels 0, 1 and 2 each
have their own colour. `MAX_LEVEL` is 2 and the min filter is
`NEAREST_MIPMAP_NEAREST`. The programs of case 48 and case 42
(`es300/vol.frag`, `es300/array.frag`) draw them into a 16x16, an 8x8 and a
4x4 viewport, so the implicit LOD is exactly 0, 1 and 2. Each level is its own
check ("3D L0..L2", "array L0..L2"); the pixels are read back and compared.
Per GPU: the GC7000 wants each level's own colour. The PowerVR's 3D checks want
level 0's colour (its grid has no mip chain, as the PR documents). The V3D
skips the case by name (it does not sample 3D or 2D-array textures), as does an
ES 2.0 library. Case 206 is not in the regression set; it runs only when
selected. It is compiled into all three GPUs' images (`pr133-board-images.md`).

**Control on the host.** Case 206's own pixel reads cannot run on the host:
"the fake GPU never executes a kick" (`pr133-case-check.md`). Two host controls
exist:

- The case commit's message: a throwaway test replays case 206's GL calls on
  a PowerVR-shaped (grid) and a GC7000-shaped (layered) mock. On origin/main
  `2fe4b879` every 3D sub-check fails on both shapes, because `glTexImage3D`
  returns at once for a level > 0. The array sub-checks pass on main. With
  PR #133 (`066a2db8`) all 12 sub-checks pass. The throwaway test is not
  committed.
- `tools/cts/log/investigations/pr133-case-check.md`, the PR's own storage test
  `tests/es3_tex_levels.rs`: on `066a2db8`, 9 passed, 0 failed, on both
  `gpu-powervr` and `gpu-vivante`. On `2fe4b879` (the test file copied over
  main, with a labelled stub for the PR's helper `level_texel_offset`), 9
  fail on each GPU. 6 of the 9 fail through the stub and say nothing about the
  PR; 3 do not touch the stub: `stacked_3d_levels_read_back_through_a_framebuffer`
  (level 1 of a stacked 3D texture reads back all zero on main),
  `a_3d_level_of_another_size_or_format_is_not_written`, and
  `copy_tex_sub_image_3d_writes_a_slice_of_a_level_above_0`. This is a control
  of the PR's storage change, not of case 206's pixel assertions.
- `tools/check.sh --changed 2fe4b879` on `066a2db8`: 140 ok, 7 failed, 0 gaps.
  The three `cc gl_test.c` rows (host, GC7000, GC7000 with test knobs), the
  `zephyr imx8mp gc_gl_test` and `zephyr rpi4 v3d_gl_test` image rows (`-Werror`)
  and the shader-header generation check are ok. The 7 red rows are the
  board.sh dry-run selftest, the two `zephyr am62p` image rows (no PowerVR
  firmware blob on that worker) and four `gen precompiled table` rows (a
  metadata-only `yakogl` rlib on that worker). They are red on `2fe4b879` as
  well. So the AM62P image row has no check.sh pass; the AM62P images were
  built separately (`pr133-board-images.md`).

**Boards.**

| board | image commit | verdict | recoveries | record |
|---|---|---|---|---|
| i.MX8MP (Verdin, GC7000), `uboot-mmc` | `066a2db8` | **case 206: pass** (`status 1`, `verdict=done`, record `594b010e,00ceff01,00000006,00000000`); it passed three times with identical record words (one run, two repeats). **Control, case 206 on `2fe4b879` plus the case, without the PR: fail** (`status 2 (check failed)`, check 1, `verdict=failed`, record `594b010e,00ce0102,04000000,04602020`). **Set: passed 48 of 49 run; skipped 1 (case 16); failing 1: case 44** (`rc=4 verdict=failed`, record `594b010e,002c0107,ffff0030,00100031`) | 0 (5 boots in all: set + case, plus the control and two repeats; each `boot_verdict=cycled`, `resets=absent`) | `tools/cts/log/board-sessions/imx8mp-pr133.md`; control and repeats: `tools/cts/log/board-sessions/imx8mp-pr133-control.md` |
| AM62P (Verdin, PowerVR) | `066a2db8` (images only) | **not run.** The probe passed (a marker written to the board's `ttyS2` arrived on the main_uart0 console path, 13 bytes, once), and both images were in hand with the recorded sha256. `tools/board.sh` was then refused by the agent's permission layer on every attempt (one try and 3 retries of the same plain command), so nothing was booted. No case 206 line, no `gl_test: all:` line, no verdict | 0 (nothing booted, no reboot) | `tools/cts/log/board-sessions/am62p-pr133-main.md`; the earlier session and the console cause: `am62p-pr133.md`, `tools/cts/log/investigations/am62p-console-map.md` |
| Raspberry Pi 4 (V3D), `tryboot` | `066a2db8` | **case 206: SKIP, no verdict on PR #133.** Console: `gl_test: case 206 es3levels: SKIP (the V3D does not sample 3D or 2D-array textures)`; `v3d_gl_test: 0 of 0 cases passed`. The skip is by design (`gl_test.c`), so this board does not exercise the PR's change. **Set: 12 of 12 passed** (`rc=0 verdict=done`, `slow jobs: 0; MMU faults: 0; recoveries: 0`). Same tally as the main baseline: no regression on the default set | 0 (`resets=0` in both runs; two tryboots by `board.sh`) | `tools/cts/log/board-sessions/pi4-pr133.md`, section "Third attempt" |

**Baselines (origin/main, without PR #133).**

| board | main commit | result | record |
|---|---|---|---|
| i.MX8MP | `63efed8e` | passed 48 of 49 run; skipped 1 (case 16); failing 1: case 44, record `594b010e,002c0107,ffff0030,00100031`; 1 boot, 0 recoveries | `tools/cts/log/board-sessions/imx8mp-pr133-baseline.md` |
| AM62P | `2fe4b879` | **not run**; the image built (rc=0), nothing booted | `tools/cts/log/board-sessions/am62p-pr133-baseline.md` |
| Raspberry Pi 4 | `2fe4b879` | **12 of 12 passed**, 0 recoveries, one tryboot (`verdict=done`, `resets=0`) | `tools/cts/log/board-sessions/pi4-pr133-baseline.md` |

**What this says.**

- i.MX8MP (GC7000): the same case 206 on the same board fails without the PR's
  change to `src/gl/texture.rs` and passes with it. Without the PR, check 1
  read `0x04000000` where the case wants `0x04602020`. The record holds only
  that word, not a per-level colour, so no more is read into it than "the first
  check failed". The whole set has the same tally as the main baseline (48 of
  49 run, case 16 skipped, case 44 failing); case 44 is the known failure
  (#6). The baseline ran on main `63efed8e` and the PR images on `2fe4b879`,
  so the two main commits differ.
- Images on i.MX8MP: the board session built its own images from `066a2db8`
  (case image sha256 `0f082a546bd2698a…`, with `CONFIG_GC_GL_TEST_CASE=206` in
  an uncommitted `prj.conf` line, hence `-dirty`; set image sha256
  `c30d8bd64de663c44b6d9bfdc543296767420fa48578e651c57e657275f9731a`). The
  repeats used a rebuild (sha256 `62208aa67e82eaca…`); the sha256 differs
  because the image holds a build timestamp. The prebuilt images in
  `pr133-board-images.md` were not the ones run there.
- Raspberry Pi 4 (V3D): the whole set shows no regression from the PR (12 of
  12, 0 recoveries). Case 206 is a skip on this GPU, so the Pi 4 neither
  confirms nor refutes the PR. The rebuilt Pi 4 images (case image sha256
  `5d440bc943c78b57…`, `066a2db-dirty`; set image sha256
  `b0bd62c82a71b42e…`) are not byte-equal to the ones in
  `pr133-board-images.md`; the builds are not reproducible.
- AM62P (PowerVR): **there is no board measurement.** Neither case 206 nor the
  regression set ran, with or without the PR. The images are built and the
  console path is settled (`am62p-console-map.md`: the leased default port
  reaches mcu_uart0, and the main_uart0 port is the one that carries
  `gl_test`). What is left is only that the two `tools/board.sh am62p run`
  commands run in a session that is allowed to run them: case 206 first, then
  the set. The PR's PowerVR checks stay unmeasured on hardware.
- Nothing is claimed for the PowerVR from the i.MX8MP or the Pi 4 results.

<!-- ===== end PR comment ===== -->

---

## Issue comment (post on issue #133)

<!-- ===== begin issue comment ===== -->

Board results for PR #133 (plan #141). The case is **gl_test case 206**
(`es3levels`): a 3D texture and a 2D array with a distinct colour per mip
level, drawn so that levels 1 and 2 are sampled, read back and compared.

**`work/int-pr49-board` was not pushed to origin.** The images come from
commit `066a2db81d40e519efc05dcbdcdd4e359468fe9a` (`066a2db8`: PR #133's head
`af9ecc53` merged with origin/main `2fe4b879`, plus the case), on
`origin/ccfarm/01a12f31-af26-7505-a35a-153b04454dae`.

**Control on the host:** per the case commit's message, on origin/main `2fe4b879`
every 3D sub-check of a host-mock replay of the case fails (the level > 0
upload is dropped), and with the PR all 12 sub-checks pass. The PR's own storage
test (`tests/es3_tex_levels.rs`) passes 9 of 9 on `066a2db8` and fails 9 of 9
on `2fe4b879` (3 of the 9 without depending on a stub). The host cannot execute
case 206's own pixel reads (`tools/cts/log/investigations/pr133-case-check.md`).

| board | image commit | verdict | recoveries | record |
|---|---|---|---|---|
| i.MX8MP | `066a2db8` | case 206 **pass** (3 runs, same record words). **Control without the PR: fail** (`status 2`, check 1). Set: passed 48 of 49 run, skipped 1 (case 16), failing 1: case 44, the same as the main baseline | 0 | `tools/cts/log/board-sessions/imx8mp-pr133.md`, `imx8mp-pr133-control.md` (baseline: `imx8mp-pr133-baseline.md`) |
| AM62P | `066a2db8` (images only) | **not run.** The console probe passed and the images were ready, but `tools/board.sh` was refused for approval on every attempt, so nothing booted; no baseline either | 0 (nothing booted) | `tools/cts/log/board-sessions/am62p-pr133-main.md` (also `am62p-pr133.md`, `tools/cts/log/investigations/am62p-console-map.md`; baseline: `am62p-pr133-baseline.md`) |
| Raspberry Pi 4 | `066a2db8` | case 206 **SKIP** (the V3D does not sample 3D or 2D-array textures; no verdict on the PR). Set: 12 of 12 passed, the same as the main baseline | 0 | `tools/cts/log/board-sessions/pi4-pr133.md`, section "Third attempt" (baseline: `pi4-pr133-baseline.md`) |

Not measured: case 206 on the PowerVR (AM62P); that still needs a session that
can run `tools/board.sh am62p run`. The V3D skips the case by name, so it has
no measurement of the PR either. Details are in the PR #133 comment.

<!-- ===== end issue comment ===== -->

---

## Where the comments were posted

- PR #133 comment: **not posted: approval refused.** `gh pr comment 133
  --repo signal-slot/yakogl --body-file <copy of the PR comment above>` was
  refused with "This command requires approval" on 3 tries (1 run + 2 retries
  of the same plain command).
- Issue #133 comment: **not posted: approval refused.** `gh issue comment 133
  --repo signal-slot/yakogl --body-file <copy of the issue comment above>`
  was refused the same way on 3 tries.

Nothing was posted, so there are no comment URLs. To post, run those two
commands in a session that is allowed to run `gh`, with each section above
(between its begin/end markers) saved as the body file.
