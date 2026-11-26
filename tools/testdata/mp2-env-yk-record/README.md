<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# DK consoles with yk-record lines (P10 gap G3)

Synthetic STM32MP257F-DK consoles for `tools/board_stm32mp2_selftest.sh`
(section 4, "G3"): what `tools/board-stm32mp2.sh tally` and `judge` make of
the image's `yk-record:` lines (`driver/zephyr/lib/board/common/record.c`,
`YK_RECORD_LINE_FMT`). Each file is a `console.txt` as the runner writes it
(`HH:MM:SS` and the line): U-Boot's hook lines, the layer's banner, one or more
record lines and the layer's end line. They are written by hand from the line
formats. No byte comes from a board, and they hold no host, address or
console path.

| file | record line(s) | the verdict line's result |
|---|---|---|
| `pass.console.txt` | `594b0101 00010101 00000001 00000000` (CASE, status pass) and gl_test's one-case tally | `verdict=done result=record:CASE ... resets=absent record=594b0101,00010101,00000001,00000000` |
| `fail.console.txt` | `594b0102 00010203 ff0000ff ff00ff00` (CASE, status fail), no FAIL line | `verdict=failed result=record:CASE` (rc 4) |
| `fault.console.txt` | `594b0305 96000045 40001234 00000010` (FAULT) | `verdict=failed result=record:FAULT` (rc 4) |
| `invalid.console.txt` | `594b0103 00010209 ff0000ff ff00ff00` (CASE, status invalid) | `verdict=bad_record result=record:CASE` (rc 4) |
| `garbled.console.txt` | three cut or garbled lines, then gl_test's tally | no record: `result=pass:gl_test`, the line without `resets=` or `record=` |

The words are decoded with `yakogl-tools record --log` / `--line`
(`tools/testdata/record-decode.txt` has the same kinds).
