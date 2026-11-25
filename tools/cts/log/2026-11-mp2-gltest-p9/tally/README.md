<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK P9: the one-case gl_test image prints its tally

Plan #405, item gltest-board (issues #1281, #7). This is the board check of
the fix for the bring-up note's "Found defect" (the one-case image printed no
`gl_test: all:` line). The fix is `one_case_tally` in
`driver/samples/zephyr/gl_test/src/main_stm32mp2.c`, landed on main as
`b7236e03`.

## What ran, against what

- Image built from **origin/main `b7236e03`**
  (`b7236e0368a96784b134a6fece76dd02d2026235`). Its banner reads
  `sample: build tree=b7236e03 table=6c5b8a70`, Zephyr `af17c0c8aa2a`.
- Image: `driver/samples/zephyr/gl_test` for `stm32mp257f_dk/stm32mp257fxx/a35`,
  built as `tools/cts/board-stage.sh`'s `dk_build` does, with
  `-DYAKOGL_WERROR=ON -DCONFIG_GL_TEST_MP2_SET_HEAD=y`. Its `.config` has
  `GL_TEST_MP2_SET_HEAD=y`, `GL_TEST_MP2_SET_HEAD_CASES=1`,
  `GL_TEST_MP2_CASES=""`, `GL_TEST_MP2_SKIP=""`,
  `YAKOGL_STM32MP2_LIFETIME_S=100`, and the OP-TEE watchdog arm
  (`gc_wdog_arm: armed`). `zephyr.bin` is 2,844,624 bytes, sha256
  `91d7274a81cd469a8b804dfc4f48154f82deb2ec5c2584fa00de6f4650446c13`.

## The run

`tools/board-stm32mp2.sh` at `b7236e03`, with `BOARD_HOST` from the lease and
`MP2_NO_CONSOLE=1`. The steps were `install <image>`, then `run <image> <logdir>
gl_test_case1_tally`, then `judge <rundir> <console file>`, then `uninstall`
and the restore below. The A35 console (USART2) was read through connected's
serial tools.

| run | cases | `gl_test: the suite took` | `PSCI SYSTEM_RESET at` | lifetime | back on Linux after |
| --- | --- | --- | --- | --- | --- |
| case 1 alone | 1 | 410 ms | 3,312 ms of uptime | 100,000 ms (3 % used) | 59 s |

The run ended on the image's own PSCI reset. The next boot's BL2 printed
`Reset reason: System reset (SYSRST) (0x2074). Warm boot.`

Verdict lines, verbatim:

```
board-stm32mp2.sh stm32mp257f-dk gl_test_case1_tally rc=0 verdict=returned boot=uboot-sd blind boot_verdict=no_console result=none back_after=59s
board-stm32mp2.sh stm32mp257f-dk gl_test_case1_tally rc=0 verdict=done boot=uboot-sd row boot_verdict=complete result=pass:gl_test back_after=59s console=external
gl_test: case 1 uniform: PASS
gl_test: all: 0 of 1 cases failed (1 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 6
yk-record: 594b0103 0001ff01 0000000a 00000000
verdict gl-case1 case1=PASS tally=PASS:0/1 final=PASS record=PASS:kind=1
```

The first line is the blind run's own verdict. The second is `judge` on the
console text. The last is
`tools/probes/stm32mp257f_dk/verdict/dk_verdict.py --expect gl-case1,supply`
on the same text, which also printed `verdict supply boot1=PASS:Warm:...` and
`4 checks, 0 failed`. The tally has the runner's `all_line` shape. The
kind-CASE single-case record is unchanged (case 1, no bad check, status 1,
10 checks), so the fix did not change the record. `yakogl: draws refused 0`.

## The console file

`gl_test_case1_tally.console.txt` is an excerpt of the USART2 text, from ST
U-Boot's banner before the one-shot to the next boot's BL2 reset reason.
The lines are verbatim. Left out:

- BL2, BL31 and OP-TEE `INFO`/`I/TC` lines, and U-Boot's `Net:` line and
  terminal-query escape bytes;
- the `optee msg`/`optee smc`/`scmi` hex traffic, the `rif`, `rcc`, `pins`,
  `clock` and `adv7535` lines of `display_power`, and the `gc_power` and `I:`
  lines between power-up and identity;
- the `gl_test: px` picture rows.

Kept: the hook and one-shot lines, the handover, the watchdog arm, the
banner, the identity, every check line, the case verdict, the tally, the
record, the final line and the reset. Redaction: no host names, addresses,
console paths or serials appear.

## Board state

Taken before `install` (read with dd and sha256 over ssh, the partition
resolved from `/boot`'s disk, type SD): the card's `u-boot-env` was sha256
`12061c13…48d5c06`, and `/boot` had 25 files with listing digest
`dc6c9b99…44f5430` (the same as on 2026-11-23) and no `/boot/yakogl`.
`uninstall` removed the hook and `/boot/yakogl`. U-Boot's own `saveenv` had
rewritten a copy, so the partition read `43a041e9…` after it. The 16 KiB of
the two copies were then written back from `install`'s pristine image. The
partition was resolved again in the same ssh command from the disk `/boot`
is mounted from, and checked as type SD, label `u-boot-env`, 512 KiB. It
read back as `12061c13…48d5c06`. `/boot` had the same 25 files and digest,
and `status` printed `install: absent` and `left behind: nothing recorded`.
The card was `mmcblk2` before the one-shot and `mmcblk0` after it. No path
named before the reboot was reused. Nothing was written to the eMMC.

Full record (gitignored):
`records/stm32mp257f-dk/20261125-2325-gl_test_case1_tally-91d7274a/`
(`INDEX.txt`, `meta.txt`, `console.mcp.txt`).
