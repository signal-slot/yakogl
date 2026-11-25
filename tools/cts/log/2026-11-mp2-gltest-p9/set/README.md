<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK P9: the gl_test regression set, timed against the lifetime

Plan #405, item gltest-set (issues #1281, #7). Procedure:
`docs/notes/stm32mp257f-dk-p9.md`, sections 2 and 3.

## What ran, against what

- Image built from **origin/main `2f020ffb`**
  (`2f020ffbe442bf2cb5c175c6c6a66e6276ada492`). Its banner reads
  `sample: build tree=2f020ffb table=6c5b8a70`, Zephyr `af17c0c8aa2a`.
- Image: `driver/samples/zephyr/gl_test` for `stm32mp257f_dk/stm32mp257fxx/a35`,
  built with `tools/cts/board-stage.sh`'s `dk_build` command minus
  `-DCONFIG_GL_TEST_MP2_SET_HEAD=y`: `-DYAKOGL_WERROR=ON
  -DCONFIG_GL_TEST_MP2_SKIP='""'`. Its `.config` has `GL_TEST_MP2_CASE=7`
  (the set), `GL_TEST_MP2_SKIP=""` (all 50 cases of `SET_ORDER`, case 10
  included), no `GL_TEST_MP2_SET_HEAD`, `YAKOGL_STM32MP2_LIFETIME_S=100`,
  and the OP-TEE watchdog arm (`gc_wdog_arm: armed`).
  `zephyr.bin` is 2,844,624 bytes, sha256
  `dfb21555855450150290a3fe62835f4228604bc0e10e02939d2d5d5ffa172dfe`.
- Shard plan: `tools/cts/mp2-gltest-shards.py --skip "" --budget-ms 90000`
  (default 500 ms a case). Result: **1 shard of 50 cases**, estimate 25,000 ms.
  The whole set is one run. No lifetime or `RUN_BOUND` change was needed, as
  predicted.

## The run

`tools/board-stm32mp2.sh` at `2f020ffb`, with `BOARD_HOST` from the lease and
`MP2_NO_CONSOLE=1`. The steps were `install`, then `run <image> <logdir>
gltest_set`, then `judge <rundir> <console file>`, then the restore below.
The A35 console (USART2) was read through connected's serial tools and judged
with `judge`. The RAM ring was not used: it does not survive the warm reset
(c465a879).

| shard | cases | `gl_test: the suite took` | `PSCI SYSTEM_RESET at` | lifetime | back on Linux after |
| --- | --- | --- | --- | --- | --- |
| 0 | 50 (49 counted + case 16 SKIP) | **11,436 ms** | **14,412 ms** of uptime | 100,000 ms (14 % used) | 70 s |

There was no `mp2: lifetime` line: the run ended on the image's own PSCI
reset, not at the lifetime. The next boot's BL2 printed `Reset reason: System
reset (SYSRST) (0x2074). Warm boot.`

Verdict lines, verbatim:

```
board-stm32mp2.sh stm32mp257f-dk gltest_set rc=0 verdict=returned boot=uboot-sd blind boot_verdict=no_console result=none back_after=70s
board-stm32mp2.sh stm32mp257f-dk gltest_set rc=4 verdict=failed boot=uboot-sd row boot_verdict=complete result=fail:gl_test back_after=70s console=external
gl_test: all: 5 of 49 cases failed (44 passed; 0 timed out; first failure case 6 at index 5; second failure case 11; 1 skipped, first skip case 16); precompiled hits 0, compiler runs 190
yk-record: 594b0103 00060107 000b002c 00100030
```

The first line is the blind run's own verdict. The second is `judge` on the
console text.

## Against the i.MX8MP

`tools/cts/mp2-gltest-diff.py shard-0000.console.txt` (defaults:
`tools/cts/RESULTS.md`'s i.MX8MP gl_test cell, 49 of 49, and
`tools/cts/mp2-gltest-expected.txt`):

```
== new failures (DK only): 5
6
11
47
182
189
== failures flaky on the i.MX8MP: 0
== expected differences failing: 0
== expected-list entries that pass (stale): 1
10
== skip differences: 0
== regression-set cases with no verdict: 0
mp2-gltest-diff: new 5, flaky 0, expected 0, missing 0
```

(exit 1: `new` > 0). This is the same five cases as the 2026-11-24 run at
`a5f1dc8e`. The failing checks, from the console:

- 6 `fbo`, checks 1 and 6: `v0 red apex (31,12) = 0xff120fe7 want 0xff090fe7`.
- 11 `persist`, check 2: the same apex pixel, `0xff120fe7` against `0xff090fe7`.
- 47 `es3inst`, checks 4 and 9: `instance 3 half white (48,16)`,
  `0x80b58080` / `0x80f18080` against `0x80808080`.
- 182 `fsat`, check 2: `folded clamps, vertex (48,16) = 0x73fa7340 want 0x73bf7340`.
- 189 `mulone`, checks 2 and 3: `0xff5a3050` against `0xff303050`, and
  `0xff782040` against `0xff402040`.

No hang, no refusal (`yakogl: draws refused 0`), fences 173 of 173 completed.
Case 10 (`prims`) passes, check 11 `loop: 3 px wide` included, so its
expected-list entry is stale.

## The console file

`shard-0000.console.txt` is an excerpt of the USART2 text, from ST U-Boot's
banner before the one-shot to the next boot's BL2 reset reason. The lines are
verbatim. Left out:

- BL2, BL31 and OP-TEE `INFO`/`I/TC` lines;
- the `optee msg`/`optee smc 0x32000039`/`scmi` hex traffic and the `rif`
  lines of `display_power` and `gc_power`;
- `gl_test: px` picture rows, the gradient row dump and the point picture;
- every check line that ends in `ok  [check N]`, except case 10's check 11.

Kept: every case verdict line, every `MISMATCH` check, the tally, the timing
lines, the records and the stall dump. The Linux shutdown before the one-shot
is not in the file. Redaction: no host names, addresses, console paths or
serials appear.

## Board state

Taken before `install` (read with dd and sha256 over ssh): the card's
`u-boot-env` was sha256 `979256b1…f6a2f`, and `/boot` had 31 entries with
listing digest `146da183…95b9` and no `/boot/yakogl`. After the run, the
card's two environment copies were written back from `install`'s pristine
image. The partition then read `979256b1…f6a2f` again, `/boot` had the same
31 entries and digest, and `status` said `install: absent`. One fault
happened during the restore and is named in the commit message: a 16 KiB
write landed on the eMMC.

Full record (gitignored):
`records/stm32mp257f-dk/20261125-1130-gltest-set-2f020ffb/` (run directory
`20261125-1131-gltest_set-dfb21555`).
