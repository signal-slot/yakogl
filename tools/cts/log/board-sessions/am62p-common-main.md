<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P common set from origin/main: images built, run not possible on the leased console

Session 2026-10-12, item `bench-recipes` of plan #18 (second attempt; the
first, task 01a12ebc, was closed in review). Outcome: **the five images built
again from `origin/main`; the common set did not run, because the console
this session was leased is the mcu_uart0 bridge port and the board's ssh
login was not leased.** No GM, SO, SV, CL or GT verdict exists from this
session. Nothing here confirms or contradicts the three glmark2 fixes on the
board.

## Commit tested

`origin/main` at `4a81162a2b8457e441803170cfd602664ebd5274`
(`Merge pull request #143 from signal-slot/work/treekey-inputs`).
`git merge-base --is-ancestor bd8df7a origin/main` exits 0, so the stale-depth
fix (bd8df7a) is in the tree. The build used a detached worktree of that
commit under the gitignored `build/`, with the PowerVR firmware blob copied to
`driver/src/fw/pvr_fw_blob.h` (not committed).

## Image build

`tools/cts/cts-common.sh am62p build build/common/am62p-4a81162a`, output
verbatim:

```
common-images: build 4a81162a (4a81162a2b8457e441803170cfd602664ebd5274) board am62p
GM rc=0 sha256=54ffab7be285480c00856e0ed59f8ca2b47169970513ce581b50319eb1570fed build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=4a81162a
SO rc=0 sha256=edfbc5c4b723234fc8a0bd5d737da06d91f89f104d8478edc2c7cf26d3a0d187 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=4a81162a
SV rc=0 sha256=edfbc5c4b723234fc8a0bd5d737da06d91f89f104d8478edc2c7cf26d3a0d187 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=4a81162a
CL rc=0 sha256=c0bb837c289b2469c73856b07991a48ef9cccc7e7c5b2519930053b3ca7160fa build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=4a81162a
GT rc=0 sha256=31b3b3db701c511c6afdebdc9fc2fad6266432db45a5842ba8f7e64b1b7b652b build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=4a81162a
common-images: 0 row(s) not built
```

SO and SV share one sha256 (one image, two run labels). The SO, SV and CL
hashes differ from the first attempt's: the Slint and cluster builds are not
byte-reproducible across sessions here, which does not affect the GM and GT
hashes (identical to the first attempt).

## Why the run did not start

Evidence, in the order it was obtained:

- The resource lease names a bridge console whose port number ends in 32.
  `tools/cts/log/investigations/am62p-console-empty.md` (summary and
  section 1) records that with the port ending in 32 the data of ttyS2 (main_uart0),
  ttyS0 and ttyS1 never arrived and only ttyS3 (mcu_uart0) did, and that
  every successful AM62P run used the port ending in 33.
- `serial_status` at the start of this session (tool result, not a file):
  `"connected":true,"current_offset":0,"bytes_buffered":0,"lease_mode":"exclusive"`.
  `serial_read` returned `"bytes":0,"from_offset":0,"next_offset":0,"output":""`.
  This is the "captured nothing" case of `am62p-console-empty.md`. Its
  section 6 says to probe and report rather than reboot, so the board was
  not rebooted and nothing was written to the console.
- `BOARD_HOST` and `BOARD_UART` were not in this session's environment and
  the lease lists only the serial adapter, not the board's ssh login.
  `ask_farm` answered that the board's ssh login and the port ending in 33
  exist (from a node's memory note, confidence 0.85; the other node could
  not confirm). The answer was not used: the lease says to use no farm
  resource other than the leased ones, and the port ending in 33 and the
  ssh login are not in it.
- The run and tally commands were therefore not issued, and no
  `records/am62p/common-4a81162a` directory exists.

Visual check: not done (no camera or display view was available).

## glmark2 results

None from this session. No `did not resolve its depth (-16)` line, memory
refusal line or `N ran / M failed` line exists here; shadow, refract and
terrain have no per-scene lines from this tree.

For context, an earlier committed record says the GM image at `f2a13877`
(before this tip) ran `33 ran / 0 failed`, score 465, with no `did not
resolve its depth` line (`tools/cts/STATUS.md`, AM62P glmark2 bullet; HANDOFF
"Known defects" says no capture shows the pictures). That is not evidence
for `4a81162a`: the NPOT and pool fixes (#1, #2) and bd8df7a postdate or are
unconfirmed in it.

## What a session needs to finish this

A session leased the port ending in 33 (main_uart0) plus the board's ssh
login, same images (rebuild from `4a81162a` or later), then:

```sh
BOARD_HOST=<user@host> BOARD_UART=<port-33 bridge> BOARD_BOOT=uboot-mmc \
  tools/cts/cts-common.sh am62p run build/common/am62p-4a81162a records/am62p/common-4a81162a
tools/cts/cts-common.sh am62p tally records/am62p/common-4a81162a
```

## Updates for the landing

Apply only after a run with the verdict lines. Wording assumes GM `33 ran /
0 failed`, no `-16` line, no refusal line; if shadow or refract still show a
`-16` line, write that instead and keep the row open. Line numbers are those
of `4a81162a`.

- `docs/HANDOFF.md`, "Known defects", the bullet at line 288 ("glmark2
  terrain black (#1) and the AM62P refract depth pass (#2) are fixed on the
  host ..."): replace "on the AM62P at `f2a13877` all 33 scenes ran, but no
  capture shows the pictures" with "on the AM62P at `<sha8>` all 33 scenes
  ran, 0 failed, with no `did not resolve its depth` line and no memory
  refusal in shadow, refract or terrain
  (`tools/cts/log/board-sessions/am62p-common-main.md`); no capture shows
  the pictures". The task text's "stale-depth row 102" and "#1/#2 rows" do
  not exist as separate rows at this tip: the stale-depth fix is covered by
  this bullet and by `STATUS.md`.
- `docs/HANDOFF.md`, the summary table at line 45 (glmark2 column, AM62P
  row): put the new score and `<sha8>`.
- `docs/HANDOFF.md`, "Next board session" (line 377): remove the AM62P
  glmark2 and common-set steps now run; keep Pi 4 refract and terrain
  (#4, #5, line 77) and the i.MX8MP steps.
- `tools/cts/RESULTS.md`, the glmark2 score table (line 49): the AM62P row
  becomes `<score> (<sha8>, <date>)`; the Slint, cluster and gl_test rows
  get the SO/SV/CL/GT verdict lines.
- `tools/cts/STATUS.md` (Japanese), the AM62P glmark2 bullet at line 19:
  「**AM62P: glmark2（GM 画像、<sha8>）は 33 シーン / 失敗 0 / スコア <score>、
  「did not resolve its depth」の行は 0、メモリ拒否の行は 0。** shadow、
  refract、terrain の 3 シーンとも走った（NPOT テクスチャ、refract の深度
  ターゲット、bd8df7a の確認）。絵の目視はまだ。」 Add a line for SO, SV, CL
  and GT with their verdicts.
- `README.md`, the Status table (line 49, AM62P score) and the paragraph at
  lines 53-56: change "on the AM62P the depth pass now resolves, but no
  picture of either scene has been checked yet" to the board result with
  `<sha8>`; keep the Pi 4 sentence.
