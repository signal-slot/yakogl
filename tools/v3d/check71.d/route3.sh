#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# route3: driver/zephyr/lib/board/rpi5_scanout/route3.h (the route-3
# bring-up of the Pi 5 screen as one ordered plan: firmware clocks, HVS6
# list and channel, pixel valve, HDMI0; and its teardown) against
# driver/zephyr/lib/board/test/route3_mock.c, host cc, -std=c99 -Wall
# -Wextra -Werror.
#
# Run from anywhere. Builds into build/v3d71/route3/. The last line is
# "route3_mock: N of N checks, 0 failed" and the exit 0 when the build and
# the run pass; exit 1 otherwise; exit 77 with "gap: CC" when there is no C
# compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=route3
out=build/v3d71/route3
t=driver/zephyr/lib/board/test
h=driver/zephyr/lib/board/rpi5_scanout
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/route3_mock.c $h/route3.h $h/hvs6_dlist_write.h $h/hvs6_chan.h \
  $h/pv_timing.h $h/hdmi_ctrl.h $h/fw_clk.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
if ! "$cc" -std=c99 -g -O1 -Wall -Wextra -Werror -I"$h" "$t/route3_mock.c" -o "$out/route3_mock"; then
  echo "$name: build failed"
  exit 1
fi
o=$("$out/route3_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
if [ "$rc" -ne 0 ] || ! printf '%s\n' "$o" | tail -n 1 | grep -Eq '^route3_mock: ([0-9]+) of \1 checks, 0 failed$'; then
  echo "$name: exited $rc or without its result line"
  exit 1
fi
exit 0
