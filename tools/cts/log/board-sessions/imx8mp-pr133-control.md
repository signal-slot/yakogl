<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP control for PR #133: case 206 without the PR, and repeat runs of the PR image

Date: 2026-10-12. Plan item imx8mp-control. Board: Verdin iMX8M Plus (GC7000), `BOARD_BOOT=uboot-mmc`.
The board has no UART, so each result is the SNVS record that `tools/board.sh` decodes.

Result: **the control failed, as expected. Case 206 on `2fe4b879` plus the case only gives status 2
(check failed). The PR image passed case 206 in both repeat runs (status 1).**
Three boots, one per run, all `boot_verdict=cycled`, `resets=absent`. Recoveries: 0.

## Commits

| ref | commit |
|---|---|
| control base (`main` of the PR's merge) | `2fe4b8798b5faf2807f31dd486296f21045f4abf` |
| case source | `066a2db81d40e519efc05dcbdcdd4e359468fe9a` (`gl_test: case 206 reads back 3D and 2D-array mip levels 1 and 2 (PR #133)`) |
| PR #133 merged with main | `81f4c089` (PR head `af9ecc53`, main `2fe4b879`) |

The case commit changes three files, all under `driver/tools/pvrlab/`: `gl_test.c`,
`gen_gl_test_shaders.py` and `gl_test_shaders_es100.h`. `git diff 2fe4b879 81f4c089 --stat -- driver/tools/pvrlab/`
is empty, so those three files at `066a2db8` are `2fe4b879` plus the case only.

## Control build: `2fe4b879` plus the case, without the PR

A detached worktree at `2fe4b879`, outside the repository tree. The three files above were written from
`066a2db8` (`git show 066a2db8:<path>`). The shaders header comes from the case commit, so it was not
regenerated. One uncommitted line was added to `driver/samples/zephyr/gc_gl_test/prj.conf`:
`CONFIG_GC_GL_TEST_CASE=206`.

Evidence that `src/` is `2fe4b879`'s, in that worktree:

```
$ git diff 2fe4b879 --stat -- src/
$ git diff 2fe4b879 --stat
 driver/samples/zephyr/gc_gl_test/prj.conf   |   1 +
 driver/tools/pvrlab/gen_gl_test_shaders.py  |   2 +-
 driver/tools/pvrlab/gl_test.c               | 144 +++++++++++++++++++++++++++-
 driver/tools/pvrlab/gl_test_shaders_es100.h |   6 +-
 4 files changed, 147 insertions(+), 6 deletions(-)
```

The first diff prints nothing. `src/gl/texture.rs`, the PR's GL-core change, is not in it.

```sh
tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt $CCFARM_DEPS_DIR/imx8mp-out/pr133-control-2fe4b879-r2 GT
```
```
common-images: build 2fe4b87-dirty (2fe4b8798b5faf2807f31dd486296f21045f4abf) board imx8mp
GT rc=0 sha256=829977582ddba5d93e4ffe6c58f46ced9cd96276445161f1ff2536fb3c54864c build=GC_BUILD_STAMP=2026-10-12T13:53:01Z
common-images: 0 row(s) not built
```

## Repeat image: `066a2db8` case 206 (PR #133 merged with main)

The prebuilt image (sha256 `0f082a54...`, `imx8mp-pr133.md`) is not obtainable. `fetch_from_farm` for its
directory answered `does not exist on this node`, and no other node has it. So I rebuilt it as
`imx8mp-pr133.md` describes: a detached worktree at `066a2db8`, one uncommitted line
`CONFIG_GC_GL_TEST_CASE=206` in the same `prj.conf`. The build stamp is new, so the sha256 differs from
`0f082a54`. The new sha256 is:

```sh
tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt $CCFARM_DEPS_DIR/imx8mp-out/pr133-066a2db8-c206-r2 GT
```
```
common-images: build 066a2db-dirty (066a2db81d40e519efc05dcbdcdd4e359468fe9a) board imx8mp
GT rc=0 sha256=62208aa67e82eaca1f88f6dd0d9e5f994588627c1ec887b325db4063418cc7ec build=GC_BUILD_STAMP=2026-10-12T13:53:34Z
common-images: 0 row(s) not built
```

## Runs

`tools/board.sh imx8mp run` with `BOARD_HOST` (the leased login, not written here) and
`BOARD_BOOT=uboot-mmc`, set by a three-line wrapper outside the repository that `exec`s
`tools/board.sh imx8mp run`. The first attempts, without the wrapper's variables, were refused
before `board.sh` started; they touched no board. After the permission was granted, the three runs went one
after the other (one GL client at a time). The control ran from the `2fe4b879` worktree and the repeats from
the `066a2db8` worktree. `records/` is gitignored, so the record directories stay on that machine.

Control (`2fe4b879` plus the case, no PR), image sha256 `829977582ddba5d93e4ffe6c58f46ced9cd96276445161f1ff2536fb3c54864c`:

```
board.sh imx8mp: ctl-2fe4b879-1: build 2fe4b87-dirty boot uboot-mmc (row) image sha256 829977582ddba5d9 footprint 50843648
board.sh imx8mp: ctl-2fe4b879-1: image build: GC_BUILD_STAMP=2026-10-12T13:53:01Z; zephyr: af17c0c8aa2a
board.sh imx8mp: ctl-2fe4b879-1: route env source mmc token ctl-2fe4b879-1-829977582ddba5d9-1791813448
records:
  CASE (1) seq 14: case 206 check 1 status 2 (check failed) got 0x04000000 want 0x04602020
board.sh imx8mp ctl-2fe4b879-1 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,00ce0102,04000000,04602020 back_after=39s uptime=24s
```

Record directory: `records/imx8mp/pr133-control/20261012-2257-ctl-2fe4b879-1-82997758/`.

Repeat run 1 (`066a2db8`, image sha256 `62208aa67e82eaca...`, full value above):

```
board.sh imx8mp: c206-066a2db8-r1: build 066a2db-dirty boot uboot-mmc (row) image sha256 62208aa67e82eaca footprint 50851840
board.sh imx8mp: c206-066a2db8-r1: image build: GC_BUILD_STAMP=2026-10-12T13:53:34Z; zephyr: af17c0c8aa2a
board.sh imx8mp: c206-066a2db8-r1: route env source mmc token c206-066a2db8-r1-62208aa67e82eaca-1791813537
records:
  CASE (1) seq 14: case 206 check 255 status 1 (pass) got 0x00000006 want 0x00000000
board.sh imx8mp c206-066a2db8-r1 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,00ceff01,00000006,00000000 back_after=63s uptime=27s
```

Record directory: `records/imx8mp/pr133-066a2db8/20261012-2258-c206-066a2db8-r1-62208aa6/`.

Repeat run 2:

```
board.sh imx8mp: c206-066a2db8-r2: build 066a2db-dirty boot uboot-mmc (row) image sha256 62208aa67e82eaca footprint 50851840
board.sh imx8mp: c206-066a2db8-r2: image build: GC_BUILD_STAMP=2026-10-12T13:53:34Z; zephyr: af17c0c8aa2a
board.sh imx8mp: c206-066a2db8-r2: route env source mmc token c206-066a2db8-r2-62208aa67e82eaca-1791813661
records:
  CASE (1) seq 14: case 206 check 255 status 1 (pass) got 0x00000006 want 0x00000000
board.sh imx8mp c206-066a2db8-r2 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,00ceff01,00000006,00000000 back_after=52s uptime=29s
```

Record directory: `records/imx8mp/pr133-066a2db8/20261012-2301-c206-066a2db8-r2-62208aa6/`.

## Outcome

| item | value |
|---|---|
| control (case 206, no PR) | **fail**: `status 2 (check failed)`, check 1, `verdict=failed`, record `594b010e,00ce0102,04000000,04602020` |
| repeat run 1 of the PR image | pass: `status 1`, `verdict=done`, record `594b010e,00ceff01,00000006,00000000` |
| repeat run 2 of the PR image | pass: same verdict and record words |
| earlier PR run (`imx8mp-pr133.md`, image `0f082a54`) | pass, same record words |
| boots | 3 (`boot_verdict=cycled`, `resets=absent` each) |
| recoveries | 0 |

## What this shows

The same case, on the same board, fails without the PR's change to `src/gl/texture.rs` and passes with it.
The PR image passed three times in all (the earlier run, and these two), with identical record words.
The control's check 1 read `0x04000000` where the case wants `0x04602020`. The record holds only that
word, not a per-level colour, so I do not read more into it than: the first check failed.

The whole gl_test set was not run in this item (it was not asked for here; `imx8mp-pr133.md` holds it).
The prebuilt `0f082a54` image was not obtainable, so the repeats used the rebuilt image with the new
sha256. Both are the same source and build; the build stamps differ.
