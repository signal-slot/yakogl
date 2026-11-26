<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK P9: the one-case gl_test image prints its tally

Plan #405, item gltest-board (issues #1281, #7). This is the board check of
the fix for the bring-up note's "Found defect": the one-case image printed no
`gl_test: all:` line. The fix is `one_case_tally` in
`driver/samples/zephyr/gl_test/src/main_stm32mp2.c` (`b7236e03`, README
`decaf45b`).

## What ran, against what

- The image was built from **origin/main `dd6cfa02`**
  (`dd6cfa02bce8d91fe438029d37cbf0aad6cefd16`), which contains `b7236e03`.
  Its banner reads `sample: build tree=dd6cfa02 table=6c5b8a70`, Zephyr
  `af17c0c8aa2a`.
- Image: `driver/samples/zephyr/gl_test` for `stm32mp257f_dk/stm32mp257fxx/a35`,
  built as `tools/cts/board-stage.sh`'s `dk_build` does, with
  `-DYAKOGL_WERROR=ON -DCONFIG_GL_TEST_MP2_SET_HEAD=y`. Its `.config` has
  `GL_TEST_MP2_SET_HEAD=y`, `GL_TEST_MP2_SET_HEAD_CASES=1`,
  `GL_TEST_MP2_CASES=""`, `GL_TEST_MP2_SKIP=""` and
  `YAKOGL_STM32MP2_LIFETIME_S=100`, and it arms the OP-TEE watchdog
  (`gc_wdog_arm: armed`). `zephyr.bin` is 2,844,624 bytes, sha256
  `d3a5c59e9e34f3d16e7cbe68140a9c7e166f14e22d006c3a947870abfe1bafc5`.

## The run

`tools/board-stm32mp2.sh` at `dd6cfa02` ran with `BOARD_HOST` from the lease
and `MP2_NO_CONSOLE=1`, in this order: `install <image>`, `run <image>
<logdir> gl_test_case1_tally`, `judge <rundir> <console file>`. The A35
console (USART2) was read through connected's serial tools. The same
`install` also served the set runs in `../set/`, and `uninstall` and the
restore came after them (`../set/README.md`, "Board state").

| run | cases | `gl_test: the suite took` | `PSCI SYSTEM_RESET at` | of `CONFIG_YAKOGL_STM32MP2_LIFETIME_S=100` | back on Linux after |
| --- | --- | --- | --- | --- | --- |
| case 1 alone | 1 | 410 ms | 3,312 ms of uptime | 3.3 s of 100 s (3 %), no `mp2: lifetime` line | 60 s |

The run ended on the image's own PSCI reset. The next boot's BL2 printed
`Reset reason: System reset (SYSRST) (0x2074). Warm boot.`

The verdict lines, verbatim:

```
board-stm32mp2.sh stm32mp257f-dk gl_test_case1_tally rc=0 verdict=returned boot=uboot-sd blind boot_verdict=no_console result=none back_after=60s
board-stm32mp2.sh stm32mp257f-dk gl_test_case1_tally rc=0 verdict=done boot=uboot-sd row boot_verdict=complete result=pass:gl_test back_after=60s console=external
gl_test: case 1 uniform: PASS
gl_test: all: 0 of 1 cases failed (1 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 6
yk-record: 594b0103 0001ff01 0000000a 00000000
verdict gl-case1 case1=PASS tally=PASS:0/1 final=PASS record=PASS:kind=1
```

- The first line is the blind run's own verdict, and the second is `judge`
  on the console text.
- The last line is
  `tools/probes/stm32mp257f_dk/verdict/dk_verdict.py --expect gl-case1,supply`
  on the same text. It also printed
  `verdict supply boot1=PASS:Warm:... boot2=PASS:Warm:...` and
  `5 checks, 0 failed`.
- The tally has the runner's `all_line` shape.
- The kind-CASE single-case record (case 1, no bad check, status 1,
  10 checks) is unchanged, so the fix did not change the record.
- `yakogl: draws refused 0`.

## The console file

`gl_test_case1_tally.console.txt` is an excerpt of the USART2 text. It runs
from ST U-Boot's banner before the one-shot to the next boot's BL2 reset
reason, and the lines it keeps are verbatim. It leaves out:

- BL2, BL31 and OP-TEE `INFO`/`I/TC` lines, U-Boot's `Net:` line and its
  terminal-query escape bytes;
- the `optee msg`/`optee smc`/`scmi` hex traffic of `display_power` and
  `gc_power`, and the other `display_power` lines except `powered:` and `up`;
- the `gl_test: px` picture rows;
- the next boot after its reset reason.

It keeps the hook and one-shot lines, the hand-over, the watchdog arm, the
banner, the identity, every check line, the case verdict, the tally, the
record, the final line and the reset. It has no host names, addresses,
console paths or serials (`tools/cts/mp2-log-redact-check.sh`: clean).

The full record is gitignored:
`records/stm32mp257f-dk/20261126-0118-gl_test_case1_tally-d3a5c59e/`
(`INDEX.txt`, `meta.txt`, `console.mcp.txt`).
