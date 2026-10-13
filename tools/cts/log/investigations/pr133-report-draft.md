<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PR #133 / issue #133: draft board report (plan #141, item report-draft)

Written 2026-10-12, host only. **Nothing has been posted to GitHub.** Both
drafts quote the records named in them and nothing else. The item `report`
fills the Raspberry Pi 4 rows (marked `PI4 PLACEHOLDER`), re-reads the
records on the merged branch, and posts.

Sources, read with `git show <branch>:<path>` (all on `ccfarm/<id>`):

| record | branch id |
|---|---|
| `tools/cts/log/board-sessions/imx8mp-pr133.md` | `01a12f7b-c9dc-700a-a443-4f0a9a7b10a9` |
| `tools/cts/log/board-sessions/imx8mp-pr133-baseline.md` | `01a12f50-f444-7252-9c16-ee795ae499cf` |
| `tools/cts/log/board-sessions/am62p-pr133.md` | `01a12fce-5b70-77f6-9ae6-6584bb466270` |
| `tools/cts/log/board-sessions/am62p-pr133-baseline.md` | `01a12f3f-1361-739e-913e-8e22a2f8b289` |
| `tools/cts/log/board-sessions/pi4-pr133-baseline.md` | `01a12f0e-1bfc-77a9-8fbb-3ea9d98e7835` |
| `tools/cts/log/investigations/pr133-board-images.md` | `01a12f98-6600-709e-817f-5e5fdac87c3c` |
| `tools/cts/log/investigations/pr133-case-check.md` | `01a12f98-6634-7184-825f-099e63025a37` |
| `tools/cts/log/investigations/pr133-texel-levels.md` | `01a12f21-fb69-7583-8552-0d5308c379ca` |
| case commit `066a2db8` (message = the control) | `01a12f31-af26-7505-a35a-153b04454dae` |

Notes for whoever posts (not part of either comment):

- The record paths below are the paths the files will have once ccfarm merges
  those branches. Check they exist on `main` before posting.
- The i.MX8MP `records/...` directories are gitignored and stay on the worker
  that ran them; the committed record is the `.md` file.
- The control has two statements in the sources. The case commit message says
  a throwaway host-mock replay of case 206's GL calls fails every 3D
  sub-check on main and passes all 12 with the PR. `pr133-case-check.md` says
  case 206 itself cannot be executed on the host (the fake GPU never executes a
  kick), and its control is the PR's storage test. The drafts give both.
  `tools/check.sh` could not be started on the case-check worker, so there is
  no check.sh summary line. Re-read that record if the item `case-check` was
  re-run after this draft.
- The i.MX8MP session built its own images at `066a2db8` (the case image
  with an uncommitted `CONFIG_GC_GL_TEST_CASE=206` in `prj.conf`, `-dirty`).
  Its sha256s differ from the ones in `pr133-board-images.md`. The drafts say so.

---

## PR comment draft (post on PR #133)

<!-- ===== begin PR comment ===== -->

### Board measurement of PR #133: gl_test case 206 (`es3levels`)

**Status: two of the three boards are in. i.MX8MP ran and passed. AM62P did
not run. Raspberry Pi 4 is pending.**

**Commit.** `work/int-pr49-board` was never pushed to origin. The images come
from commit `066a2db81d40e519efc05dcbdcdd4e359468fe9a` (`066a2db8`,
"gl_test: case 206 reads back 3D and 2D-array mip levels 1 and 2 (PR #133)"),
which is PR #133's head `af9ecc53` merged with origin/main `2fe4b879`, plus the
case. It is on `origin/ccfarm/01a12f31-af26-7505-a35a-153b04454dae`.

**The case.** `driver/tools/pvrlab/gl_test.c` case **206**, `es3levels`. It
uploads a 16x16x4 3D texture and a 16x16x2 2D array. Levels 0, 1 and 2 each
have their own colour. `MAX_LEVEL` is 2 and the min filter is
`NEAREST_MIPMAP_NEAREST`. Case 48's and case 42's programs
(`es300/vol.frag`, `es300/array.frag`) draw them into a 16x16, an 8x8 and a
4x4 viewport, so the implicit LOD is exactly 0, 1 and 2. Each level is its own
check ("3D L0..L2", "array L0..L2"); the pixels are read back and compared.
Per GPU: the GC7000 wants each level's own colour. The PowerVR's 3D checks want
level 0's colour (its grid has no mip chain, as the PR documents). The V3D
skips the case by name (it refuses 3D and array bindings), as does an ES 2.0
library. Case 206 is not in the regression set; it runs only when selected.
It is compiled into all three GPUs' images (`pr133-board-images.md`).

**Control (host).**

- The case commit's message: a throwaway test replays case 206's GL calls on
  the `es2_npot.rs` scaffold, on a PowerVR-shaped (grid) and a GC7000-shaped
  (layered) mock. On origin/main `2fe4b879` every 3D sub-check fails on both
  shapes: the bind is incomplete (0,0,0,1), because `glTexImage3D` returns at
  once for a level > 0 (`src/gl/texture.rs:4075-4076` there). The array
  sub-checks pass on main. With PR #133 (`af9ecc53` merged with `2fe4b879`)
  all 12 sub-checks pass. The GC7000 shape samples `90 40 c0` at L1 and
  `f0 80 10` at L2. The throwaway test is not committed.
- `tools/cts/log/investigations/pr133-case-check.md`: case 206's own pixel
  assertions cannot run on the host ("the fake GPU never executes a kick"). Its
  control is the PR's storage test. On `066a2db8`, `es3_tex_levels` gives
  9 passed; 0 failed. On `2fe4b879` without the PR,
  `stacked_3d_levels_read_back_through_a_framebuffer` fails at level 1 slice 0:
  left all zero, right the uploaded colours (`[16, 0, 0, 161, ...]`). The
  record says: "This is a control of the PR's storage change, not of case
  206's pixel assertions: nothing here renders a pixel."
- On `066a2db8`, `cargo fmt --check`, the `host-test,gpu-powervr`,
  `host-test,gpu-powervr,report-es2` and `host-test,gpu-vivante` test rows,
  the three `cc -fsyntax-only -Wall -Wextra -Werror gl_test.c` rows and
  `gen_gl_test_shaders.py --check` are ok. The `shaderc-online` test rows,
  clippy and most other rows are recorded as gaps, and `tools/check.sh` was
  refused on that worker, so there is no check.sh summary line.

**Boards.**

| board | image commit | verdict | recoveries | record |
|---|---|---|---|---|
| i.MX8MP (Verdin, GC7000), `uboot-mmc` | `066a2db8` | **case 206: pass** (`status 1`, `verdict=done`, record `594b010e,00ceff01,00000006,00000000`). **Set: passed 48 of 49 run; skipped 1 (case 16); failing 1: case 44** (`rc=4 verdict=failed`, record `594b010e,002c0107,ffff0030,00100031`). Two boots, one per image | 0 | `tools/cts/log/board-sessions/imx8mp-pr133.md` |
| AM62P (Verdin, PowerVR) | `066a2db8` | **not run.** The `YK-PROBE-S2` probe failed (0 of 2): the leased console `usb-serial-1` (port 35932) did not carry the marker written to `/dev/ttyS2`, so no image was fetched, flashed or booted. No case 206 line, no `gl_test: all:` line | 0 (no reboot, RTI reset or power cycle) | `tools/cts/log/board-sessions/am62p-pr133.md` |
| Raspberry Pi 4 (V3D), `tryboot` | `066a2db8` | **PI4 PLACEHOLDER: pending. Fill from `tools/cts/log/board-sessions/pi4-pr133.md` once item board-pi4 is merged. Do not post this row as it is.** The case prints SKIP on the V3D (`gl_test.c:7215`), so the case-206 image is a boot proof, not a pixel check | PI4 PLACEHOLDER | PI4 PLACEHOLDER (`tools/cts/log/board-sessions/pi4-pr133.md`) |

**Baselines (origin/main, without PR #133).**

| board | main commit | result | record |
|---|---|---|---|
| i.MX8MP | `63efed8e` | passed 48 of 49 run; skipped 1 (case 16); failing 1: case 44, record `594b010e,002c0107,ffff0030,00100031`; 1 boot, 0 recoveries | `tools/cts/log/board-sessions/imx8mp-pr133-baseline.md` |
| AM62P | `2fe4b879` | **not run**, the same console reason; the image built (rc=0), nothing booted | `tools/cts/log/board-sessions/am62p-pr133-baseline.md` |
| Raspberry Pi 4 | `2fe4b879` | **12 of 12 passed**, 0 recoveries, one tryboot (`verdict=done`, `resets=0`) | `tools/cts/log/board-sessions/pi4-pr133-baseline.md` |

**What this says.**

- i.MX8MP: the set's verdict line and record words are the same as on the main
  baseline (`63efed8e`): passed 48 of 49 run, case 16 skipped, case 44 failing.
  The two main commits differ (`2fe4b879` in the branch, `63efed8e` in the
  baseline). Case 44 is the known failure (#6). Case 206 passed, which means
  the GC7000 sampled levels 1 and 2 of the 3D texture and the 2D array. The
  record holds only the pass, not the per-check colours. There is no on-board
  control on main, because case 206 does not exist there; the control is the
  host one above.
- AM62P: **there is no board measurement.** Neither case 206 nor the regression
  set ran, with or without the PR. The records name the cause as the console
  lease: the leased port reaches mcu_uart0, and main_uart0 is port 35933 in the
  farm's notes (unverified in these sessions). A `board.sh` run on the leased
  port would reboot the board to capture an empty console, so none was started.
  The case-206 and whole-set images for it are built and unrun.
- The images on i.MX8MP were built by the board session from `066a2db8` (case
  image sha256 `0f082a546bd2698a…`, with `CONFIG_GC_GL_TEST_CASE=206` set in an
  uncommitted `prj.conf` line, hence `-dirty`; set image
  `c30d8bd64de663c44b6d9bfdc543296767420fa48578e651c57e657275f9731a`). The prebuilt images in `pr133-board-images.md`
  were not the ones run there.
- Nothing is claimed for the PowerVR or the V3D from the i.MX8MP result.

<!-- ===== end PR comment ===== -->

---

## Issue comment draft (post on issue #133)

<!-- ===== begin issue comment ===== -->

Board results for PR #133 (plan #141). The case is **gl_test case 206**
(`es3levels`): a 3D texture and a 2D array with a distinct colour per mip
level, drawn so that levels 1 and 2 are sampled, read back and compared.

**`work/int-pr49-board` was never pushed to origin.** The images come from
commit `066a2db81d40e519efc05dcbdcdd4e359468fe9a` (`066a2db8`: PR #133's head
`af9ecc53` merged with origin/main `2fe4b879`, plus the case), on
`origin/ccfarm/01a12f31-af26-7505-a35a-153b04454dae`.

**Control (host):** per the case commit's message, on origin/main `2fe4b879`
every 3D sub-check of a host-mock replay of the case fails (the level > 0
upload is dropped), and with the PR all 12 sub-checks pass. The host cannot
execute case 206's own pixel reads (`pr133-case-check.md`).

| board | image commit | verdict | recoveries | record |
|---|---|---|---|---|
| i.MX8MP | `066a2db8` | case 206 **pass**; set passed 48 of 49 run, skipped 1 (case 16), failing 1: case 44, the same as the main baseline | 0 | `tools/cts/log/board-sessions/imx8mp-pr133.md` (baseline: `imx8mp-pr133-baseline.md`) |
| AM62P | `066a2db8` | **not run** (console probe failed: the leased `usb-serial-1` does not reach main_uart0); no baseline either | 0 | `tools/cts/log/board-sessions/am62p-pr133.md` (baseline: `am62p-pr133-baseline.md`) |
| Raspberry Pi 4 | `066a2db8` | **PI4 PLACEHOLDER: pending.** Main baseline: 12 of 12 passed | PI4 PLACEHOLDER | PI4 PLACEHOLDER (`tools/cts/log/board-sessions/pi4-pr133.md`; baseline: `pi4-pr133-baseline.md`) |

Not measured: case 206 on the PowerVR (AM62P). The V3D skips the case by name.
Details are in the PR #133 comment.

<!-- ===== end issue comment ===== -->
