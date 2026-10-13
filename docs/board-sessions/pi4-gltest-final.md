<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-gltest-final: Pi 4 gl_test default list from the origin/main tip

Date: 2026-10-14. Image built from `origin/main` tip `a63fcc5b`
(`a63fcc5b48c58849267dc3f3299c8514bdf861ba`, "Merge pull request #439 from
signal-slot/work/stage-after-note"). The image reports `V3D_BUILD=a63fcc5b`,
zephyr `af17c0c8aa2a`. It was built in the ccfarm workspace checkout, which
was clean and at exactly this commit (a detached worktree was refused at the
time). `board.sh` printed `build b1004e35` for itself because by then the
workspace held this record's own commit on top of `a63fcc5b`; the image is
the one built from `a63fcc5b` (sha256 `25548c99d50e4c2a...`, as printed).

**Result: 12 of 12 default-list cases passed on the Pi 4 (V3D). 0 failed,
0 timed out, 0 skipped.** 0 MMU faults, 0 GPU recoveries, 0 resets.

## Build

```
tools/cts/common-images.sh pi4 tools/cts/common-tree.txt build/gltest-final GT
common-images: build a63fcc5b (a63fcc5b48c58849267dc3f3299c8514bdf861ba) board pi4
GT rc=0 sha256=25548c99d50e4c2aacd3f77e01c987be3ea489affc6ee929ab176a3cad5f3d1c build=V3D_BUILD=a63fcc5b
common-images: 0 row(s) not built
```

## Verdict line printed by the device run

Run: `tools/board.sh pi4 tree tools/cts/common-tree.txt <outdir>` with
`TREE_IMAGE_DIR=build/gltest-final`, `TREE_ROWS=4` (row GT), `BOARD_BOOT=tryboot`,
the lease's ssh login and the serial bridge as the console. The verdict line,
printed twice (once after the row, once in the tree's per-run list):

```
board.sh pi4 GT rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:gl_test_record_end resets=0 record=absent back_after=261s
```

`record=absent`: the image prints no `yk-record:` line; the verdict comes from
the console's own end line (`result=legacy:gl_test_record_end`).

## Per case

Console line `v3d_gl_test: the list 5,58,59,66,67,102,3,9,11,12,13,4 (12 cases)
on one context`, then:

| Case | Name | Result |
|---|---|---|
| 5 | scissor | PASS |
| 58 | clearscis | PASS |
| 59 | clearmask | PASS |
| 66 | clearvp | PASS |
| 67 | fbods | PASS |
| 102 | maskclear | PASS |
| 3 | depth | PASS |
| 9 | stencil | PASS |
| 11 | persist | PASS |
| 12 | stencil2 | PASS |
| 13 | zls | PASS |
| 4 | cull | PASS |

Console summary lines:

```
gl_test: all: 0 of 12 cases failed (12 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 18
slow jobs: 0 (past 1000 ms, ended; the slowest 0 ms); MMU faults: 0; recoveries: 0; hung: none
v3d_gl_test: 12 of 12 cases passed (2700 ms)
```

## Failing cases

None. The console printed `FIRST FAILURE: none (no timeout)`.

## How the run went

- Dependencies: `tools/cts/deps-env.sh --check rpi_4b` printed `ok` for every
  entry; `MESA_V3D_TREE` was not needed.
- An earlier attempt of this session did not run the board: the serial bridge
  address was not yet known and the runner was refused by the session's
  permission layer. After that was granted the run above was the first and
  only run; the board returned to Linux by itself (`back_after=261s`).
- The board's records stay under the gitignored `records/` of the workspace
  (directory `records/pi4-gltest-final/20261014-0000-GT-25548c99`); they are
  not committed.
