<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK P9: the gl_test regression set at `dd6cfa02`

Plan #405, item gltest-board (issues #1281, #7). The 50-case regression set
(`SET_ORDER`, `driver/tools/pvrlab/runner/src/lib.rs`) ran on the DK and was
diffed against the i.MX8MP's gl_test column. main had no set log under
`tools/cts/log/` before this one, so the shard ran here.

## The plan and the images

`python3 -I tools/cts/mp2-gltest-shards.py --out <dir> --skip "" --budget-ms 90000`
printed `mp2-gltest-shards: 50 cases, 1 shards, budget 90000 ms`
(`manifest.tsv`, `shard-0000.conf`). Every image was built from **origin/main
`dd6cfa02`** (banner `sample: build tree=dd6cfa02 table=6c5b8a70`) as
`tools/cts/board-stage.sh`'s `dk_build` does, with `-DYAKOGL_WERROR=ON
-DCONFIG_GL_TEST_MP2_SKIP='""'`. They differ in one setting:

- the shard image (`-DEXTRA_CONF_FILE=shard-0000.conf`: `GL_TEST_MP2_CASES`
  is the 50-case list, walked through `gl_test_set_list`), `zephyr.bin`
  sha256 `46b5cec318607b104f4e54ea11d3eb2106a293b9ac7831d06062e51d83c6ee40`;
- the default walk (`GL_TEST_MP2_CASES=""`, the image of the `670913e1`
  rerun), sha256 `f0b0e9691f8ea56d7eb2168742ee69a6d0be501e51e5dded9f2720b196ded610`.

Both have `YAKOGL_STM32MP2_LIFETIME_S=100` and arm the OP-TEE watchdog.

## The runs

The runs were one-shots from the DK's U-Boot off the microSD with
`tools/board-stm32mp2.sh run` (`MP2_NO_CONSOLE=1`), judged with `judge` on
the A35 console read through connected's serial tools. Each run ended on
the image's own PSCI reset, and every next BL2 printed `Reset reason:
System reset (SYSRST) (0x2074). Warm boot.` No console has an `mp2:
lifetime` line.

| run | image | `gl_test: the suite took` | `PSCI SYSTEM_RESET at` | of `CONFIG_YAKOGL_STM32MP2_LIFETIME_S=100` | back on Linux after | failed | console |
| --- | --- | --- | --- | --- | --- | --- | --- |
| shard 0 of 1 | shard | 11,415 ms | 14,404 ms | 14.4 s of 100 s (14 %) | 57 s | 3 of 49 | `shard-0000.console.txt` |
| shard 0 of 1, rerun | shard | 11,419 ms | 14,408 ms | 14.4 s of 100 s (14 %) | 70 s | 5 of 49 | `shard-0000-rerun.console.txt` |
| default walk | default | 11,414 ms | 14,387 ms | 14.4 s of 100 s (14 %) | 57 s | 3 of 49 | `set-default.console.txt` |

Records (gitignored): `records/stm32mp257f-dk/20261126-0119-gltest_set-46b5cec3/`,
`20261126-0123-gltest_set_rerun-46b5cec3/`, `20261126-0124-gltest_set_default-f0b0e969/`.

### Shard 0 of 1 (the P9 result at `dd6cfa02`)

```
board-stm32mp2.sh stm32mp257f-dk gltest_set rc=0 verdict=returned boot=uboot-sd blind boot_verdict=no_console result=none back_after=57s
board-stm32mp2.sh stm32mp257f-dk gltest_set rc=4 verdict=failed boot=uboot-sd row boot_verdict=complete result=fail:gl_test back_after=57s console=external
gl_test: all: 3 of 49 cases failed (46 passed; 0 timed out; first failure case 47 at index 30; second failure case 182; 1 skipped, first skip case 16); precompiled hits 0, compiler runs 190
```

`tools/cts/mp2-gltest-diff.py shard-0000.console.txt` (defaults) gives new 47,
182 and 189, with flaky 0, expected 0, stale 0, skip differences 0 and no
verdict 0. Its last line:

`mp2-gltest-diff: new 3, flaky 0, expected 0, missing 0` (shard 0 of 1 at `dd6cfa02`: 14.4 s of 100 s, the suite 11.4 s of it)

### The rerun, and the default walk

The rerun of the same image failed 5 of 49, the five cases of every earlier
set run (`a5f1dc8e`, `670913e1`). The default walk failed 3, like the first
run.

```
gl_test: all: 5 of 49 cases failed (44 passed; 0 timed out; first failure case 6 at index 5; second failure case 11; 1 skipped, first skip case 16); precompiled hits 0, compiler runs 190
gl_test: all: 3 of 49 cases failed (46 passed; 0 timed out; first failure case 47 at index 30; second failure case 182; 1 skipped, first skip case 16); precompiled hits 0, compiler runs 190
```

`mp2-gltest-diff: new 5, flaky 0, expected 0, missing 0` (rerun: 14.4 s of 100 s)

`mp2-gltest-diff: new 3, flaky 0, expected 0, missing 0` (default walk: 14.4 s of 100 s)

## The failing checks: two states

The wrong pixels fall into two sets, and each set repeats exactly:

| case, check | state A (run 1, default walk) | state B (rerun; `a5f1dc8e`, `670913e1`) | want |
| --- | --- | --- | --- |
| 6 `fbo`, checks 1 and 6, `v0 red apex (31,12)` | ok (`0xff090fe7`) | `0xff120fe7` | `0xff090fe7` |
| 11 `persist`, check 2, `earlier frame kept (31,12)` | ok | `0xff120fe7` | `0xff090fe7` |
| 47 `es3inst`, check 4, `arrays: instance 3 half white (48,16)` | `0x80938080` | `0x99b48080` (`0x80b48080`, `0x80b58080` before) | `0x80808080` |
| 47 `es3inst`, check 9, `elements: instance 3 half white` | `0x80a98080` | `0xb5f18080` (`0x80f18080` before) | `0x80808080` |
| 182 `fsat`, check 2, `folded clamps, vertex (48,16)` | `0x73d57340` | `0x7efa7340` (`0x73fa7340` before) | `0x73bf7340` |
| 189 `mulone`, check 2, `M * vec3(0.5), no lane 1.0` | `0xff3f3050` | `0xff5a3050` | `0xff303050` |
| 189 `mulone`, check 3, `abs, negation, uniform, each * 1.0` | `0xff542040` | `0xff782040` | `0xff402040` |

Every wrong channel is too large. In state A each error is smaller than in
state B, and cases 6 and 11 pass. Nothing in the image changed between the
first run and the rerun (the same `zephyr.bin`). Between `670913e1` and
`dd6cfa02`, the only commits under `src/vivante/` are licence headers. So
6 and 11 were not fixed: their verdict depends on the run. Outside the
check lines, the consoles of run 1 and the rerun differ only in the one-shot
token, the load time, U-Boot's device-tree address in `mp2-handover`
(`x1`), the system counter at `PRE_KERNEL_1`, and the suite time. `x1` does
not follow the state (state A's default walk and state B's rerun both have
`0xf8099c68`). The cause is open and is not fixed here.

In all three runs, nothing hung and nothing was refused in the Vivante back
end: `yakogl: draws refused 0`, fences 173 of 173 completed,
`irq errors 0`. The stall dump after `gl_test: FAIL rc=-1` shows the front
end idle with nothing pending.

## The console files

Each `*.console.txt` is an excerpt of the USART2 text of one run, the same
cut as `../tally/` (README there): from ST U-Boot's banner to the next BL2
reset reason, verbatim lines. It leaves out the `INFO`/`I/TC` lines, the
OP-TEE and SCMI hex traffic, the `display_power` detail and the
`gl_test: px` rows. `tools/cts/mp2-log-redact-check.sh` on this directory:
clean.

## Board state

- Before `install`: the card was `/dev/mmcblk1` (type SD) and `/boot` was
  `/dev/mmcblk1p8`. The card's `u-boot-env` had sha256 `d1fa271c…2924398`
  (both copies valid, left that way by an earlier session). `/boot` had
  25 files with listing digest `dc6c9b99…44f5430` (the same as on
  2026-11-23) and no `/boot/yakogl`.
- Across the four runs the card was numbered `mmcblk1`, `mmcblk2` and
  `mmcblk0` (`meta.txt`: `card_before`/`card_after`), so no device path
  written down before a reboot was reused. That includes the
  `dd of=/dev/mmcblk…p7` undo line that `install` prints.
- `uninstall` removed the hook and `/boot/yakogl`, and `status` printed
  `install: absent` and `left behind: nothing recorded`. U-Boot's own
  `saveenv` had rewritten a copy (`cfdd621a…`), so the 16 KiB of the two
  copies were written back from `install`'s pristine image. The partition
  was resolved in the same ssh command from the disk `/boot` is mounted on,
  and checked as a block device, type SD, label `u-boot-env`, 524,288 bytes:
  `/dev/mmcblk0p7`.
- The partition read back as `d1fa271c…2924398`, and `/boot` had the same
  25 files and digest. Nothing was written to the eMMC.
