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

- Source tree: `git worktree add --detach <deps>/base-am62p-2fe4b879/wt
  2fe4b879`. (A first build came from a `git archive` export, before
  `git worktree add` was approved. Its image says `tree=unknown`. It is
  superseded and listed at the end of this section only for completeness.)
- `driver/src/fw/pvr_fw_blob.h`: `<deps>/pvr_fw_blob.h`. The bytes it holds
  have sha256 `32c8b3d7730a523b...`, the same as the board's
  `/lib/firmware/rgx.fw.36.53.104.796` (read over ssh today). Not committed.
- Build: `tools/cts/common-images.sh am62p tools/cts/common-tree.txt
  <deps>/base-am62p-2fe4b879/img-wt GT`, run in the worktree (the common
  set's GT row: the `gl_test` sample with its default case list,
  `-DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y
  -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n`).

```
common-images: build 2fe4b879 (2fe4b8798b5faf2807f31dd486296f21045f4abf) board am62p
GT rc=0 sha256=c42e6f5820e7c576f02ea629395bb13fb219157cc3f1abc87cc3a67d753df5a1 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=2fe4b879
common-images: 0 row(s) not built
```

The image to run is `<deps>/base-am62p-2fe4b879/img-wt/GT/zephyr/zephyr.bin`
(the default set: `CONFIG_GL_TEST_CASE=7`, `CONFIG_GL_TEST_CASES=""`). It is
kept, so a later session can run it as built.

The superseded export build: `<deps>/base-am62p-2fe4b879/img/GT`, sha256
`fead30c56d38458f...`, `PVR_SAMPLE_TREE=unknown`. Its source matched
`git ls-tree -r 2fe4b879` on all 2,330 blobs. Do not run it; it does not
name its commit.

## The run line (dry run only)

```
BOARD_HOST=$BOARD_HOST BOARD_UART=$BOARD_UART BOARD_BOOT=uboot-mmc \
TREE_IMAGE_DIR=<deps>/base-am62p-2fe4b879/img-wt TREE_TAG=2fe4b879 TREE_ROWS=4 \
  tools/board.sh --dry-run am62p tree tools/cts/common-tree.txt records/am62p/pr133-baseline-2fe4b879
```

The dry run, run in the worktree, printed every step of the GT row with no
error:

```
board.sh am62p: tree: 5 rows from tools/cts/common-tree.txt, build 2fe4b879
board.sh am62p: GT: build 2fe4b879 boot uboot-mmc (row) image sha256 c42e6f5820e7c576 footprint 40812544
board.sh am62p: GT: image build: PVR_SAMPLE_TABLE=c95547a1 PVR_SAMPLE_TREE=2fe4b879; zephyr: af17c0c8aa2a
=== GT rc=0 2026-10-12 17:09:13
```

Between those lines it printed the lock, stage, `fw_setenv`, reboot, wait,
return and clean-up commands. `TREE_ROWS=4` selects GT alone. The other
four rows (GM, SO, SV, CL) are not this item's, and they were not built.
The real run is the same line without `--dry-run`.

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

## Re-run attempt (item base-am62p-run, 2026-10-12): console probe failed, nothing ran

**Outcome: nothing ran.** No boot, no `board.sh` run (not even `--dry-run`
again), no record directory, recoveries 0 (no reboot, RTI reset or power
cycle). The board was not touched except for the two probe writes below.

Console probe (before any boot):

- Port **35932** (the address the lease actually holds; the task text said
  35931, which is the stored `BOARD_UART` value): `YK-PROBE-S2` was written
  to the board's `/dev/ttyS2` over ssh (after `stty -F /dev/ttyS2 115200`,
  rc=0), twice. The console bridge showed **0 bytes** both times
  (`serial_read` next_offset 0, `bytes_buffered` 0). The marker did not
  arrive. Same result as the first session: 35932 is not main_uart0.
- Port **35933**: **not probed.** The only way to reach it from this task
  is a raw TCP connection (`nc`) to a port the lease does not list, and the
  sandbox refused that command ("requires approval"; no one is there to
  approve). The MCP serial tool is bound to 35932 and cannot be re-pointed.
  So whether 35933 carries the marker is still unverified in this session
  (the owner verified it on 2026-10-09, see `console-map.md`).

So no address reached the board's main_uart0 here, and the run was not
started: a `board.sh` run on 35932 would reboot the board to capture an
empty console (`verdict=empty`).

For the farm's resource entry: stored `BOARD_UART` port 35931 (wrong; no
bridge answer recorded), lease/MCP port 35932 (mcu_uart0, ttyS3), wanted
port 35933 (main_uart0, ttyS2).

To finish this baseline: lease the console at 35933 (or approve `nc`/ssh
access to it), repeat the probe, then run the "run line" above from the
worktree with the kept image `img-wt` (sha256 `c42e6f58...`, unchanged),
then the tally. The verdict lines, recovery counts and record directory go
in this section; there are none yet.

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
  `<deps>/base-am62p-2fe4b879/img-wt/GT` (sha256 `c42e6f58...`,
  `tree=2fe4b879`), with the run line above. Run it from the worktree
  `<deps>/base-am62p-2fe4b879/wt`. Before the reboot, check the port: the
  `YK-PROBE-S2` marker written to ttyS2 must arrive on the leased console.
  The farm's own notes agree that 35933 is main_uart0 and that no ccfarm
  resource names it (ask_farm, 2026-10-12).
- The board-am62p item (work/int-pr49-board) has **no AM62P baseline from
  this session**. Until one runs, its regression set can only be compared
  with the `32778b57` 48 / 48 row above, which mixes in drift since
  `32778b57`.

## retry 2026-10-13: nothing ran

- ssh to the leased verdin-am62p worked (`uname -a`: Linux aarch64,
  6.6.138-7.6.1 Torizon).
- The leased console is `usb-serial-1:35932` (connected, exclusive). The
  marker `YK-PROBE-S2` was written to ttyS2 over ssh twice (both writes
  returned 0). Nothing arrived on port 35932 (0 of 2, 0 bytes read).
- Port 35933 (the expected A53 main_uart0): a single `nc` probe was denied
  by the sandbox, so it was not tested. It was not retried.
- No console address is verified, so `tools/board.sh` was not run. Nothing
  ran; there is no verdict line, no recovery count and no record directory.
- Still needed: lease `usb-serial-1` at port 35933, as in the handoff above.
- Follow-up, same day: the approval requests for `ssh` and `nc` were
  denied (nobody decided in time). A second single `nc` probe to port 35933
  needed approval and was not run. No further retries.
