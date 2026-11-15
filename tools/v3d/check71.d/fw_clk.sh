#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# fw_clk: driver/zephyr/lib/board/rpi5_scanout/fw_clk.h (firmware clock
# queries for the Pi 5 display clocks, HDMI clock rules, formatter) against a
# model of the firmware's property interface
# (driver/zephyr/lib/board/test/fw_clk_mock.c), host cc, C99,
# -Wall -Wextra -Werror.
#
# Run from anywhere. Builds into build/v3d71/fw_clk/. The last line is
# "fw_clk_mock: N of N checks, 0 failed" and the exit 0 when every check
# passed; exit 1 otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=fw_clk_mock
out=build/v3d71/fw_clk
t=driver/zephyr/lib/board/test
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/fw_clk_mock.c driver/zephyr/lib/board/rpi5_scanout/fw_clk.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out" || exit 1
"$cc" -std=c99 -g -O1 -Wall -Wextra -Werror "$t/fw_clk_mock.c" -o "$out/$name" || {
  echo "$name: build failed"
  exit 1
}
o=$("$out/$name" 2>&1)
rc=$?
printf '%s\n' "$o"
if [ "$rc" -ne 0 ] || ! printf '%s\n' "$o" | tail -n 1 | grep -Eq "^$name: ([0-9]+) of \1 checks, 0 failed\$"; then
  echo "$name: exit $rc or no passing result line"
  exit 1
fi
exit 0
