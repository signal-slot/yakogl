<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P board session: gl_test baseline on origin/main for PR #133 (not run)

**Baseline half of the PR #133 measurement** (plan #141, item base-am62p).
Source: origin/main `2fe4b8798b5faf2807f31dd486296f21045f4abf` (`2fe4b879`,
"Merge pull request #141 from signal-slot/work/issue-127"), the tip after
`git fetch origin` on 2026-10-12. The plan named `87ce4a4c`; main has moved
two commits on (`a52f9155`, `86239c24`, both check.sh tooling). PR #133's
commits (`origin/work/int-pr49`, `3e90710d` and `af9ecc53`) are not ancestors
of `2fe4b879` (checked with `git merge-base --is-ancestor`).

## Result: the image built; the board run did not run

The default gl_test image (`CONFIG_GL_TEST_CASE=7`, the regression set)
built at `2fe4b879`, rc=0. **It did not run on the board.** The console
port this session leased carries mcu_uart0, not main_uart0, so no gl_test
output could reach the capture (section "Why it did not run"). A run
would have rebooted the board to capture nothing (`verdict=empty`, as in
the 2026-10-08 record quoted in
`tools/cts/log/investigations/am62p-console-empty.md`). The board was not
rebooted.

- `gl_test: all: ...` line: **none** (no run).
- Failing cases and their re-runs alone: **none** (no run).
- Record dirs: **none**. No `records/...` dir was made; only `--dry-run`
  ran, and it writes nothing.
- Recoveries: **0**. No reboot, no RTI reset, no power cycle. The board's
  Linux uptime ran on through the session (15:44 at the start, 15:49 at the
  end), and `yako_once` and `yako_default` were empty at the end.

## How it was built

`BOARD_HOST` and `BOARD_UART` are named here by their variables only.
Inputs, all from the dependency cache `<deps>`
(`tools/cts/deps-env.sh --check am62p`: every entry `ok`):
`ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR` (1.0.1) from the
environment, and a real `cargo`/`rustc` first on PATH (`$HOME/.cargo/bin`;
the kache cargo shim has no rustc beside it).

- Source tree: `git archive 2fe4b879` unpacked to
  `<deps>/base-am62p-2fe4b879/src`. This session could not run
  `git worktree add` (not approved in the session). A check against
  `git ls-tree -r 2fe4b879` matched all 2,330 blobs (0 mismatches). Because
  the tree has no `.git`, the image's build line reads `tree=unknown`
  (`sample.cmake` takes it from `git describe`). The commit is the one named
  above.
- `driver/src/fw/pvr_fw_blob.h`: `<deps>/pvr_fw_blob.h`. The bytes it holds
  have sha256 `32c8b3d7730a523b...`, the same as the board's
  `/lib/firmware/rgx.fw.36.53.104.796` (read over ssh today). Not committed.
- Build: `tools/cts/common-images.sh am62p tools/cts/common-tree.txt
  <deps>/base-am62p-2fe4b879/img GT` (the common set's GT row: the `gl_test`
  sample with its default case list, `-DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y
  -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n`).

```
common-images: build unknown (unknown) board am62p
GT rc=0 sha256=fead30c56d38458f9faf413779fe988732031e1c3a59cba966914c1ce2eabd6a build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=unknown
common-images: 0 row(s) not built
```

The image is `<deps>/base-am62p-2fe4b879/img/GT/zephyr/zephyr.bin`
(2,599,552 bytes, `.config`: `CONFIG_GL_TEST_CASE=7`,
`CONFIG_GL_TEST_CASES=""`). It is kept, so a later session can run it as
built.

## The run line (dry run only)

```
BOARD_HOST=$BOARD_HOST BOARD_UART=$BOARD_UART BOARD_BOOT=uboot-mmc \
TREE_IMAGE_DIR=<deps>/base-am62p-2fe4b879/img TREE_TAG=2fe4b879 \
  tools/board.sh --dry-run am62p tree tools/cts/common-tree.txt records/am62p/pr133-baseline-2fe4b879
```

GT's dry run printed every step with no error: `image sha256
fead30c56d38458f footprint 40812544`, `image build: PVR_SAMPLE_TABLE=c95547a1
PVR_SAMPLE_TREE=unknown; zephyr: af17c0c8aa2a`, `route env source mmc`, then
the lock, stage, `fw_setenv`, reboot, wait, return and clean-up commands.
`=== GT rc=0`. The other four rows (GM, SO, SV, CL) are not this item's.
They were not built and print `no image`. To run GT alone without the dry
run, add `TREE_ROWS=4`.

Before the run, `ps` on the board showed no gl_test, deqp or labrun
process: the board was idle.

## Why it did not run

`tools/board.sh` reads the AM62P's gl_test result only from the serial
console: `gl_test_set_verdict` and the `gl_test: all:` line come from
`console.txt`, and the gl_test image writes no record. The console must be
the bridge port wired to main_uart0. U-Boot, Zephyr's `zephyr,console` and
the Linux getty (ttyS2) all print there.

The probe this session ran (the H2 check of
`tools/cts/log/investigations/am62p-console-empty.md`), reading the leased
serial console (`usb-serial-1`, port 35932):

1. Two CRs typed into the console: 0 bytes back in 4 s (a getty on that UART
   would answer with `login:`).
2. From the board's Linux, `stty -F /dev/ttyS2 115200; echo YK-PROBE-S2 >
   /dev/ttyS2; stty -F /dev/ttyS3 115200; echo YK-PROBE-S3 > /dev/ttyS3`
   (rc=0).
3. The console then held exactly `YK-PROBE-S3\r\n` (13 bytes). The ttyS2
   marker did not arrive.

So port 35932 is mcu_uart0 (ttyS3), as `tools/cts/log/investigations/console-map.md`
records ("35932 AM62P mcu_uart0 ... never carries U-Boot, getty or Zephyr
output"). The console is port 35933 (main_uart0, verified by the owner on
2026-10-09). This session's lease did not include 35933, and a task may not
use a farm resource it was not leased. The project's stored `BOARD_UART`
value names yet another port, 35931, which console-map lists as "not
recorded".

## Comparison with the last AM62P set

There is no number to compare: nothing ran. The last AM62P gl_test sets on
main are:

- `32778b57` (2026-10-10, RESULTS.md "AM62P: gl_test default set on
  `32778b57`: 48 / 48"): `gl_test: all: 0 of 48 cases failed (48 passed; 0
  timed out; first failure case -1 at index -1)`, 0 resets.
- `c4ccc966`: 49 / 49, 0 resets, with 197 `vtxgrid` in the set (HANDOFF,
  "ES 2.0, per board"; no RESULTS.md entry).
- Cases 198 to 201 passed alone on `a27d69c0`. Case 198 hung the fragment
  DM on `452d6c31` before `a80b297f5`. This is the known case to watch.

A baseline run of this image would compare against those numbers.

## Handoff:

- **Fix the lease first.** The ccfarm resource `usb-serial-1` must be
  leased at port 35933 (main_uart0) for any AM62P run. At port 35932 every
  board.sh run captures nothing. Also correct the stored `BOARD_UART`
  (35931). After that, re-run this item. The image needs no rebuild:
  `<deps>/base-am62p-2fe4b879/img/GT` (sha256 `fead30c5...`), with the run
  line above and `TREE_ROWS=4`. Before the reboot, check the port: the
  `YK-PROBE-S2` marker written to ttyS2 must arrive on the leased console.
- The board-am62p item (work/int-pr49-board) has **no AM62P baseline from
  this session**. Until one runs, its regression set can only be compared
  with the `32778b57` 48 / 48 row above, which mixes in drift since
  `32778b57`.
- A worktree build would print `tree=2fe4b879` instead of `tree=unknown`.
  A session that is allowed to run `git worktree add` should prefer one.
