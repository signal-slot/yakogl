<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 board session, PR #133 baseline: v3d_gl_test on main, 12 / 12

Date: 2026-10-12. Plan #141, item base-pi4: the whole default `v3d_gl_test`
set on origin/main, without PR #133. This is the Pi 4 baseline the later
board-pi4 item compares work/int-pr49-board against.

Result: **12 of 12 passed, 0 recoveries, one tryboot, no extra reboot.** No
case failed, so no case was run alone.

## Commit

The baseline is origin/main `2fe4b8798b5f` ("Merge pull request #141 from
signal-slot/work/issue-127"), fetched in this session. The plan named
`87ce4a4c`; main has since moved by three commits (`a52f9155`, `86239c24`,
`2fe4b879`), none of them under `driver/samples/zephyr/v3d_gl_test/`. The
sample's default list is `CONFIG_V3D_GL_TEST_CASES` =
`5,58,59,66,67,102,3,9,11,12,13,4` (12 cases).

## Build

From a clean detached worktree of `2fe4b879` (`$SW`), with `ZEPHYR_WS`,
`WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR` and `RUSTUP_TOOLCHAIN=nightly` from the
environment:

```sh
$SW/tools/cts/common-images.sh pi4 $SW/tools/cts/common-tree.txt <img> GT
```

```
common-images: build 2fe4b879 (2fe4b8798b5faf2807f31dd486296f21045f4abf) board pi4
GT rc=0 sha256=a429543dc4c803224e186229586152a96209089c403fd6d804727a7eb42fd1b5 build=V3D_BUILD=2fe4b879
common-images: 0 row(s) not built
```

The image is 2,128,664 bytes on Zephyr `af17c0c8aa2a`.

## Run

`BOARD_HOST` (the lease's ssh login) and `BOARD_UART` (`host:port` of the
serial bridge in front of the Pi 4's Debug Probe) come from the environment;
`BOARD_HDMI_TIMINGS` is the lab's. The dry run came first, with the board
idle, and this session was the only GL client:

```sh
$SW/tools/board.sh --dry-run pi4 run <img>/GT/zephyr/zephyr.bin <rec> gt-2fe4b879
$SW/tools/board.sh pi4 run <img>/GT/zephyr/zephyr.bin <rec> gt-2fe4b879
```

```
board.sh pi4: gt-2fe4b879: build 2fe4b879 boot tryboot (row) image sha256 a429543dc4c80322 footprint 91635712
board.sh pi4: gt-2fe4b879: image build: V3D_BUILD=2fe4b879; zephyr: af17c0c8aa2a
  v3d_gl_test: build 2fe4b879 (the record at the end or at 75 s)
  gl_test: all: 0 of 12 cases failed (12 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 18
  v3d_gl_test RECORD, build 2fe4b879, 2 s after boot: DONE
  slow jobs: 0 (past 1000 ms, ended; the slowest 0 ms); MMU faults: 0; recoveries: 0; hung: none
  v3d_gl_test: 12 of 12 cases passed (2699 ms)
  v3d_gl_test RECORD end, build 2fe4b879
board.sh pi4 gt-2fe4b879 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:gl_test_record_end resets=0 record=absent back_after=262s
```

The per-case lines, all `PASS`: 5 scissor, 58 clearscis, 59 clearmask,
66 clearvp, 67 fbods, 102 maskclear, 3 depth, 9 stencil, 11 persist,
12 stencil2, 13 zls, 4 cull. No case failed, so there are no failing-case
lines.

The run dir is `20261012-1717-gt-2fe4b879-a429543d/`, in
`rec-pr133base/` of the worker's dependency cache (nuc). It is not committed,
like the other records/ dirs; copy it to `records/pi4/` with that name. Its files: `INDEX.txt`, `meta.txt`, and the console
(`console.raw`, `console.ts`, and `console.txt`, 742 lines). The run started
17:17:36 and ended 17:22:09. Clean-up: "staged image removed, tryboot.txt put
back (read back)".

| row | image | verdict | gl_test | recoveries | reboots |
|---|---|---|---|---|---|
| GT `v3d_gl_test`, default set | `a429543dc4c80322...`, `V3D_BUILD=2fe4b879` | done, rc 0 | 12 of 12 | 0 (console count; MMU faults 0) | 1 (the tryboot), 0 extra |

## Comparison with the last Pi 4 sets

| set | commit | result |
|---|---|---|
| docs/HANDOFF.md "State of main" | `afa12e9b` | 12 / 12 |
| tools/cts/RESULTS.md, common round | `36bdc26f` | 12 of 12, resets 0 (`2689 ms`) |
| main checkout, untracked `records/pi4/gt-1cf946a9/20261011-0152-gt-1cf946a9-4555048a` | `1cf946a9` | 12 of 12, resets 0, back after 262 s |
| this session | `2fe4b879` | 12 of 12, resets 0, back after 262 s, `2699 ms` |

The baseline matches: the same 12 cases pass, with the same compiler-run
count (18) as on `1cf946a9`, and no board drift shows in the set. The default
list holds no 3D or 2D-array mip-level case, so PR #133's own case is not in
it; board-pi4 runs that case on its own.

## Notes

- Earlier in this session the build and the run were refused by the
  unattended session's permissions (the checkout, `ask_farm`, `board.sh` with
  its environment). They ran once those were granted; nothing ran before then,
  and the board was not rebooted for any of it.
