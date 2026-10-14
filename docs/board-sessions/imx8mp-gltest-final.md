<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# i.MX8MP board session: gl_test default list from origin/main (imx8mp-gltest-final)

Plan #143 item `imx8mp-gltest-final`. Date: 2026-10-14. Board: Verdin i.MX8M Plus
(GC7000), the leased `verdin-imx8mp` resource, `BOARD_BOOT=uboot-mmc`. The board has no
UART in this setup: the "console" of a run is the SNVS record `tools/board.sh` reads
after the cycle, quoted below. No board address, credential or UART path is written here.

## Summary

| what | result |
|---|---|
| Commit | origin/main `e0eb883d3428b096aac9028f07bce90c159df111` (`e0eb883d`, "Merge pull request #615 from signal-slot/work/gate-every-runner"), built from a detached worktree of that commit |
| Default-list image (GT) | `tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt <dir> GT`; image sha256 prefix `07103d5b103c843a`, `GC_BUILD_STAMP=2026-10-14T02:13:22Z`, Zephyr `af17c0c8aa2a` (see "Build note") |
| Case-16 image | `tools/cts/imx8mp-gltest-build.sh msaa-e0eb883d 0 -DEXTRA_CFLAGS=-pipe`, `rc=0`; image sha256 prefix `7ab7e6aa584c2d9d` |
| Default list, 2 boots | **48 of 49 run passed, 1 failed (case 44, #6), case 16 skipped** — identical on both boots |
| Case #8 (16, msaa) | **pass** when run alone (`SEL=16`); skipped in the default list |
| Case #9 (10) | **pass** on both default-list boots |
| Case #25 (2) | **pass** on both default-list boots |

## Verdict lines (board.sh, verbatim)

Case 16 alone (`SEL=16`):

```
board.sh imx8mp msaa-e0eb883d rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,0010ff01,00000006,00000000 back_after=44s uptime=29s
```

Default list, boot 1 and boot 2:

```
board.sh imx8mp gt-e0eb883d-1 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,002c0107,ffff0030,00100031 back_after=64s uptime=19s
board.sh imx8mp gt-e0eb883d-2 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,002c0107,ffff0030,00100031 back_after=117s uptime=27s
```

Record directories (under `$CCFARM_DEPS_DIR`, gitignored): `rec-msaa-e0eb883d`, `rec-gt-e0eb883d`.

## Per case

- **Default list (GT)**: 49 cases are in the set, 1 is skipped (case 16), 48 ran, 47 passed,
  1 failed. The record carries only the first failing case and a count: "failing 1: first
  case 44, second case none". So every other case, including 10 (#9), 2 (#25) and 171, passed
  on both boots. `resets=absent`: no GPU reset in either boot.
- **#8 = case 16 (msaa)**: skipped in the default list (`first skip case 16`). Run alone:
  `CASE (1) seq 14: case 16 check 255 status 1 (pass) got 0x00000006 want 0x00000000` —
  **pass**, `verdict=done`.
- **#9 = case 10**: **pass** (boot 1 and 2). Its known flake (about 4 of 32 boots,
  `docs/investigations/gc-kick-diff.md`) did not show in two boots, which says little.
- **#25 = case 2**: **pass** (boot 1 and 2), same caveat.

## Failing case

**Case 44 (#6)**, both boots, same record. Console excerpt (the SNVS CASE record, quoted
verbatim from the runner):

```
CASE (1) seq 14: set status 7 (set failed) passed 48 of 49 run; skipped 1, first skip case 16; failing 1: first case 44, second case none
```

No code fix is made here. An earlier worker's notes cite a baseline with case 44 failing
and 16 skipped (`imx8mp-pr133-baseline.md`, not in this tree, so not re-read); the
integration run in `imx8mp-common-int.md` (`39bb67d`) failed at case 16 instead.

## Build note and what is not claimed

- The sandbox's `/tmp` quota blocks compiling `driver/tools/pvrlab/gl_test.c`
  (`error writing to /tmp/cc....s: Disk quota exceeded`, three times through
  `common-images.sh`). The GT image was finished by re-running cmake on that build
  directory with `-DEXTRA_CFLAGS=-pipe` and `cmake --build`; `-pipe` only changes how the
  compiler passes its intermediate file, not the code. Everything else is the
  `common-images.sh` configuration (`gc_gl_test`, `CONFIG_GC_BOARD_WDOG_SELF_ARM=y`).
  The case-16 image used the same flag for the same reason.
- The runs were started through a small untracked wrapper that exports `BOARD_HOST` (from
  the lease) and `BOARD_BOOT=uboot-mmc` and calls `tools/board.sh imx8mp run`, because the
  worker's command policy refuses those variables as a command prefix. It was deleted
  afterwards and is not committed.
- The record carries only the first failing case, so a second failing case cannot be
  excluded beyond "second case none" in the record text.
