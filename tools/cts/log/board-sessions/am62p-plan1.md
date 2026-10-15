<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P board session: plan #1 on the integration image

**Status (2026-10-11): finished.** The 9 invariance failures it found were
fixed in `e17fb6a7`; image 1 on that commit passes 1,379 / 0 on the board
(`tools/cts/RESULTS.md`, 2026-10-10). The `am62p-deqp-images.sh` exit-1
quirk it reported is fixed (the script ends with `BUILD-EXIT`). This record
stays because `website/data.js` cites it as the source of the
images 1 to 7 figure.

## Re-run at main `4d744e17` / `291f3cb9` (2026-10-15): images built, board run not started

A re-run of images 1 to 7 (the fragdata row is in image 1) at main
`4d744e17b5612bb47b92b1f6a7f7ecd4164ed75e` was tried on workers `system76`
and `hx90` earlier on 2026-10-15, and again on `system76` at main tip
`291f3cb9`. The integration results below keep their `b83216d4` label.
**No image ran on the board, so there are no new verdict lines.** The one
thing that stopped the run is a **sandbox permission refusal**, not an ssh
or serial-console failure: ssh to the board worked on every attempt, and
`tools/board.sh am62p tree ...` was refused by the worker's command sandbox
with `This command requires approval` (twice on the last attempt, the
second as a separate simple retry, then not retried again; on the earlier
`hx90` attempt it was refused four times, and the `--dry-run` form the same
way). The command never started, so `board.sh` did not stage, arm or reboot
anything.

### What was established

- **ssh works.** On the last attempt `ssh torizon@<board> -- cat
  /proc/uptime` (the lease's ssh destination, the farm key) printed
  `379.81 1446.50`, and five seconds later `384.78 1465.53`: the board was
  up and not rebooting at that moment. The one-minute-apart second read was
  not taken, because the run could not start. On the earlier `hx90` attempt
  the board rebooted twice (about 07:47 and 07:50 UTC) while the lease was
  held, and the console bridge answered `busy` during the same minutes; no
  other driver was named by anything on the board.
- **The console bridge could not be probed on the last attempt.** The
  sandbox refuses any command that expands a shell variable
  (`A variable in this command can't be checked before it runs`), so
  `python3 tools/am62p_netboot_uart.py probe "$BOARD_UART"` could not be
  issued; `ask_farm` was also denied (a ccfarm permission request). On the
  earlier `hx90` attempt the probe printed `probe: busy (another client
  holds the bridge)` (exit 4) while the board was rebooting and exited 0 at
  07:57 and 07:58 UTC. The bridge value is environment only
  (`$BOARD_UART`, resource `usb-serial-1`) and is not recorded here.
- **Main's driver and samples did not change.** `git diff --stat 4d744e17
  291f3cb9 -- driver src tools/cts/am62p-deqp-images.sh
  tools/cts/am62p-full-tree.txt` is empty, so images built at `4d744e17`
  are valid for the current tip, and the tip's `HEAD8` for the run is
  `4d744e17` as `TREE_TAG`.
- **Firmware blob.** The board's `/lib/firmware/rgx.fw.36.53.104.796` is
  229,376 bytes, sha256
  `32c8b3d7730a523bdf2e392c62a42bcb8c0bef725f6e89b05c06a7cc4ea9a406`; the
  bytes decoded from the dependency cache's `pvr_fw_blob.h` hash to the
  same value. The header is copied to `driver/src/fw/pvr_fw_blob.h` in the
  build worktree (ignored by `driver/.gitignore`, never committed).
- **Images 1 to 7 built at `4d744e17`.** In a detached worktree with
  `ZEPHYR_WS=<deps>/zephyr-ws`, `WEST_VENV=<deps>/zephyr-ws/.venv`,
  `ZEPHYR_SDK_INSTALL_DIR=<deps>/zephyr-sdk-1.0.1`,
  `DEQP_WORK=<deps>/deqp-stage1-am62p`, `TREE_ROWS=0-6`:
  `tools/cts/am62p-deqp-images.sh tools/cts/am62p-full-tree.txt
  <deps>/plan1-am62p-4d744e17/img 4d744e17 > build.out`. The `system76` set
  (the one the run would use on that host), `build.out` as printed:

```
img1 rc=0 bin=6069e873959e22ee
img2 rc=0 bin=46d3de696a19a81c
img3 rc=0 bin=6b53183faa109835
img4 rc=0 bin=022244ea005f92c8
img5 rc=0 bin=d3ac5835f5e69883
img6 rc=0 bin=5b6732c46b28ebd6
img7 rc=0 bin=c1bff17aa7ce23e6
BUILD-EXIT
```

  Full sha256 of `<img>/img<N>-4d744e17/zephyr/zephyr.bin` on `system76`:
  1 `6069e873959e22eec1bdba77668ae33ea8acbc6b5a9540939a43bc115b37b465`, 2
  `46d3de696a19a81c2ba68b5cb6f34c01a012b7a493cb27c99601959b1c7249b1`, 3
  `6b53183faa1098353ec5c7d1e45e9f7e65ccb9ee87b5021b7649828c9f28cbb5`, 4
  `022244ea005f92c8487d062ad64333873d5aadf2e15b46d93efd1c4e08f54203`, 5
  `d3ac5835f5e69883c0ece110da2fd852b242f670551eba35f2dfa965da5a4fe9`, 6
  `5b6732c46b28ebd6419487018f96730c7c1170fcd8444f7bf737bdf85ca95d7e`, 7
  `c1bff17aa7ce23e66e4327c2cf51168c19cb96fde420cae3e76c89a6e048c769`.
  The same commit built on `hx90` gave different hashes (`68c0302f…`,
  `a06e352d…`, `fc181356…`, `9408dcee…`, `c667c00e…`, `e86b0932…`,
  `ab792fe7…`): the image build is not byte-reproducible across hosts, so
  a verdict line's `<sha8>` must be read against the host that built it.

### The blocker: the run command was refused by the worker sandbox

With the board up and the images built, this command was issued (the
bridge value is in the worker's environment and was not written on the
command line):

```sh
BOARD_HOST=<lease ssh destination> BOARD_BOOT=uboot-mmc \
TREE_IMAGE_DIR=<deps>/plan1-am62p-4d744e17/img TREE_TAG=4d744e17 TREE_ROWS=0-6 \
    tools/board.sh am62p tree tools/cts/am62p-full-tree.txt <deps>/plan1-am62p-4d744e17/rec/full
```

The sandbox answered `This command requires approval` both times; no
approval came within the turn. Nothing was staged, armed or rebooted, so
the board's `yako_once`, `/var/yakogl/` and U-Boot env are as found. This
is a permission blocker on the worker (`tools/board.sh` is not on its
allowed command list), not evidence about the board, the bridge or the
code. A first attempt on `system76` (07:30 UTC) had no `BOARD_UART` in its
environment and the lease held only `verdin-am62p` (capability ssh), so
`board.sh` would have stopped at its `BOARD_UART` check there regardless.

### Commands for the next attempt

The next attempt needs a worker whose allowed commands include
`tools/board.sh` and shell variable expansion, a lease of `verdin-am62p`
and `usb-serial-1` (capability serial-console) with `BOARD_UART` set, and
nobody else driving the board (the probe must exit 0, and the uptime must
keep growing across two reads a minute apart). `$SW` is the checkout and
`<deps>` the dependency cache.

```sh
python3 $SW/tools/am62p_netboot_uart.py probe "$BOARD_UART"   # exit 0; 4 = someone else holds it: stop
ssh $BOARD_HOST -- 'cat /proc/uptime; sudo -n fw_printenv -n yako_once; ls /var/yakogl/'   # growing; empty; empty
# only for a new tip: rebuild
mkdir -p $SW/driver/src/fw && cp <deps>/pvr_fw_blob.h $SW/driver/src/fw/pvr_fw_blob.h
export ZEPHYR_WS=<deps>/zephyr-ws WEST_VENV=<deps>/zephyr-ws/.venv \
       ZEPHYR_SDK_INSTALL_DIR=<deps>/zephyr-sdk-1.0.1 DEQP_WORK=<deps>/deqp-stage1-am62p
TREE_ROWS=0-6 $SW/tools/cts/am62p-deqp-images.sh $SW/tools/cts/am62p-full-tree.txt <img> $HEAD8 > <img>/build.out
# the run (about 15 to 20 min for seven images, from the b83216d4 back_after times)
BOARD_HOST=<lease ssh destination> BOARD_UART="$BOARD_UART" BOARD_BOOT=uboot-mmc \
TREE_IMAGE_DIR=<img> TREE_TAG=$HEAD8 TREE_ROWS=0-6 \
    $SW/tools/board.sh am62p tree $SW/tools/cts/am62p-full-tree.txt <rec>/full
$SW/tools/board.sh am62p tally <rec>/full
```

Expected, from the 2026-10-10 run of `e17fb6a7` (`tools/cts/RESULTS.md`):
image 1 at 1,379 pass / 0 fail with every `fragdata.*` case passing, and
images 2 to 7 equal to the table below (994, 1172, 1100, 2525, 1318, 1631,
all 0 fail).

**Integration image, not a landing proof.** Source: the integration
snapshot, HEAD `b83216d4003b3e0b4764adc067dbbc843083c8f0` (`b83216d4`, the
integration tip this session ran; it replaces the earlier label
`198e8a5de6da1b4452f8cc7c9c502b6cd757f1bf`, at which nothing ran — the
session's shell was restricted and no command outside the workspace was
approved). `4e9397c` and `c6aa6ba` are both ancestors of HEAD (checked with
`git merge-base --is-ancestor`). A later origin/main session repeats this
before landing.

## Result: images 1 to 7 ran on the AM62P board; 10,092 pass, 9 fail

All seven shaders images (table rows 1 to 7, `TREE_ROWS=0-6`) built at
`b83216d4` and ran to `verdict=done` through `tools/board.sh am62p tree`
(boot `uboot-mmc`, serial bridge `$BOARD_UART`). Every case in every
image got a result. Six images match the `d60b9d76` baseline exactly. Image 1
has **9 new Fail cases** (`shaders.invariance.*.common_subexpression_1/2/3`
in all three precisions) and one expected flip: `fragdata.invalid_assign_to_1`
moved from NotSupported to **Pass**, confirming the prediction in
`docs/es2-cts-failures.md` ("expected pass, awaiting board"). The 9 failures
are a regression against `d60b9d76` (they passed there) and are not in
`docs/es2-cts-failures.md`.

## How it was run

Inputs on this node (host B), all under the dependency cache
`<deps>`. Board address, UART bridge and workspace paths are
environment only; they are
named here by their variables, never by their values:

- `ZEPHYR_WS=$ZEPHYR_WS`,
  `WEST_VENV=$WEST_VENV`,
  `ZEPHYR_SDK_INSTALL_DIR=$ZEPHYR_SDK_INSTALL_DIR`,
  `CTS=<deps>/deqp-stage1-rpi_4b/cts-src` (shared with the Pi 4 stage 1).
- Stage 1 for this board: `WORK=<deps>/deqp-stage1-am62p
  BOARD=verdin_am62p/am62p54/a53 tools/cts/cts-build-zephyr.sh` (rc=0).
  It needs a real `cargo`/`rustc` first on PATH (`$CARGO_HOME/bin`);
  the kache cargo shim has no rustc next to it (same finding as baseline2).
- `driver/src/fw/pvr_fw_blob.h` regenerated from the board's
  `/lib/firmware/rgx.fw.36.53.104.796` (fetched with
  `ssh $BOARD_HOST cat ... > .inputs/rgx.fw`, then the
  `driver/tools/mkfwblob.sh` format). Never committed.
- Images: `TABLE=<deps>/plan1-am62p-images/am62p-shaders7.txt
  tools/cts/am62p-deqp-images.sh <that table> b83216d4` — the table is rows
  1 to 7 of `tools/cts/am62p-full-tree.txt`. All seven `img<N> rc=0`
  (`build.out`). The script still exits 1 after the last unlabelled row (a
  `set -e` quirk in its final labelled-table test); every image built.
- Board: `BOARD_HOST` and `BOARD_UART` from the environment (values never
  written here), `BOARD_BOOT=uboot-mmc
  TREE_IMAGE_DIR=<the images dir above> TREE_TAG=b83216d4
  TREE_ROWS=0-6 tools/board.sh am62p tree tools/cts/am62p-full-tree.txt
  records/plan1-am62p/full`, then `tools/board.sh am62p tally
  records/plan1-am62p/full`. The tree is resumable: rows with an `INDEX.txt`
  are skipped, so an interrupted row re-runs alone.

## Records and tallies

| image | record | pass | fail | NotSupported | warnings |
|---|---|---|---|---|---|
| 1 | `20261008-0634-img1-b83216d4-f4c169af` | 1503 | **9** | 0 | 0 |
| 2 | `20261008-0636-img2-b83216d4-b7f3c40b` | 994 | 0 | 0 | 0 |
| 3 | `20261008-0638-img3-b83216d4-a09e9b60` | 1172 | 0 | 0 | 0 |
| 4 | `20261008-0641-img4-b83216d4-891bc4bd` | 1100 | 0 | 0 | 0 |
| 5 | `20261008-0650-img5-b83216d4-f442972e` | 2525 | 0 | 0 | 0 |
| 6 | `20261008-0653-img6-b83216d4-6e9f93f0` | 1318 | 0 | 0 | 0 |
| 7 | `20261008-0656-img7-b83216d4-fd54245d` | 1631 | 0 | 0 | 0 |
| 8 to 17, ms4 | none | not run | not run | not run | not run |

Image sha256 prefixes (from `build.out` and the board lines): 1
`f4c169aff5c1bbb4`, 2 `b7f3c40bd8bdc0f9`, 3 `a09e9b609e4529bf`, 4
`891bc4bdc9c3092c`, 5 `f442972e497857e8`, 6 `6e9f93f0fe482aff`, 7
`fd54245d5b6f21ce`. Every console shows the provenance line
`PVR_SAMPLE_TABLE=c95547a1 PVR_SAMPLE_TREE=b83216d4; zephyr: b8df2f8d3d4e`.

Image 5 ran twice: the first capture (`20261008-0643-img5-...`) was cut when
the worker's command limit ended the capture mid-run (the tally lists it as
`rc=4 verdict=no_reset`); the re-run above holds the result. Same situation
RESULTS.md §1z2 notes for `d60b9d76` image 5.

Verdict lines, as printed by `tools/board.sh`:

```
board.sh am62p img1-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=136s
board.sh am62p img2-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=117s
board.sh am62p img3-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=110s
board.sh am62p img4-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=123s
board.sh am62p img5-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=179s
board.sh am62p img6-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=137s
board.sh am62p img7-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=140s
```

The `tally` mustpass section: 10,101 mustpass cases fall in these seven
images' filters (7,064 mustpass cases are in rows 8 to 17, not run); of them
**10,092 pass / 9 fail / 0 NotSupported / 0 warnings**, no case without a
result. No GPU reset lines in any record.

## Comparison with d60b9d76 (17,141 pass / 0 fail / 19 NotSupported / 5 warnings)

Rows 2 to 7 are identical to the §1z2 per-image counts (994, 1172, 1100,
2525, 1318, 1631, all 0 fail). Row 1 changed in two ways:

- `dEQP-GLES2.functional.shaders.fragdata.invalid_assign_to_1`: NotSupported
  at `d60b9d76`, **Pass** here (`deqp: Pass`, console line 2957). All seven
  `fragdata.*` cases pass. The `4e9397c` ES 2.0 reporting fix works on the
  board.
- 9 cases moved from Pass to Fail — a regression of the compiler changes
  since `d60b9d76`, exactly the case HANDOFF warned about:

```
dEQP-GLES2.functional.shaders.invariance.highp.common_subexpression_1   Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.highp.common_subexpression_2   Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.highp.common_subexpression_3   Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.lowp.common_subexpression_1    Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.lowp.common_subexpression_2    Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.lowp.common_subexpression_3    Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.mediump.common_subexpression_1 Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.mediump.common_subexpression_2 Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.mediump.common_subexpression_3 Fail (Detected variance between two invariant values)
```

`common_subexpression_0` passes in all three precisions; the failure message
carries no numeric detail. These 9 names are not in `docs/es2-cts-failures.md`
(no `invariance` entry anywhere in it): a new AM62P failure. The test computes
one common subexpression twice and requires bit-identical results, so a
compiler that fuses or reassociates one occurrence and not the other produces
exactly this. The prime suspects are the AM62P fusion/predication commits
listed in `docs/es2-cts-failures.md` section D with risk H — `6f96104` +
`18972dc` (fmad fusion), `39bd698` (varying product fused with its sum),
`81c53c6`, `afa12e9`, `ecd341d`, `f4c41d0`, `97ba03d`/`5f4d367` — plus
`c6aa6ba` (vertex attribute registers). A bisect of image 1 is the next step;
the host `compiler-regression-host` corpus did not catch this.

## Not run

Rows 8 to 17 (fbo, texture, vertex attribute, …) and the `ms4` multisample
image were not built or run: this item covers images 1 to 7 and the fragdata
row only. The whole-tree comparison against 17,141 / 0 / 19 / 5 needs those
rows too.

## Handoff:

- The 9 `invariance.common_subexpression_1/2/3` failures (all precisions) are
  the item's main finding: a board-proven regression of the pvr-online
  compiler since `d60b9d76`, first error line "Detected variance between two
  invariant values", records under
  `records/plan1-am62p/full/20261008-0634-img1-b83216d4-f4c169af/`. Bisect
  image 1 across the section-D H-risk commits; a dEQP-shaped host repro of
  "same subexpression, two occurrences, one fused" would be cheaper than
  board bisection.
- `fragdata.invalid_assign_to_1` Pass is board-confirmed; `es2-cts-failures.md`
  (owned by bench-recipes) can move that row from "awaiting board" to done.
- Stage 1, the seven images, and the records are on this node:
  `DEQP_WORK=<deps>/deqp-stage1-am62p`,
  images in `<deps>/plan1-am62p-images/img1..7-b83216d4` (labels
  `img<N>-b83216d4`, `build.out` present). A re-run of rows 8 to 17 needs no
  rebuild of rows 1 to 7: the tree skips rows with `INDEX.txt`.
- `am62p-deqp-images.sh` exits 1 after the last unlabelled row even when all
  images built (`set -e` on its final `[ "$labelled" = 1 ]` test) — worth a
  one-line fix by the tooling owner.
