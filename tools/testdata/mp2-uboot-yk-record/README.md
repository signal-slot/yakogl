<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# DK U-Boot consoles with yk-record lines (P10 gap G3)

Synthetic STM32MP257F-DK consoles for `tools/board_stm32mp2_uboot_selftest.sh`
(section 4): what the typed U-Boot one-shot runner
(`tools/board-stm32mp2-uboot.sh`, its header's verdict line) makes of the
image's `yk-record:` lines (`driver/zephyr/lib/board/common/record.c`,
`YK_RECORD_LINE_FMT`): `result=record:<KIND>`, `resets=`, `record=` and the
record's own status (bad_record). Each file is the console the selftest's fake
console plays: the two boots with `@LINE@` where the line the runner typed is
echoed back. They are written by hand from the line formats. No byte comes
from a board, and they hold no host, address or console path.

| file | record line | the verdict line |
|---|---|---|
| `pass.console.txt` | `594b0101 00010101 00000001 00000000` (CASE, status pass) and a legacy `PASS:` line | `rc=0 verdict=done ... result=record:CASE resets=absent record=594b0101,00010101,00000001,00000000 env_diff=empty boot_diff=empty back_after=<n>s` — the whole line, the record fields between `result=` and `back_after=` |
| `resets.console.txt` | `594b0904 00000096 00000000 a1e01001` (DEQP_CASES, GPU resets 1, no status) | `result=record:DEQP_CASES resets=1 record=<its words>`: `resets=` a number, and status n/a is not bad_record (rc 0) |
| `invalid.console.txt` | `594b0103 00010209 ff0000ff ff00ff00` (CASE, status invalid) | `rc=4 verdict=bad_record ... result=record:CASE resets=absent record=594b0103,00010209,ff0000ff,ff00ff00`: the line order unchanged, the bad record still named |

The words are decoded with `yakogl-tools record --log` / `--line`
(`tools/testdata/record-decode.txt` has the same kinds). The cases without a
record line and the garbled ones are built inline in the selftest.
