#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# hdmi_ctrl: driver/zephyr/lib/board/rpi5_scanout/hdmi_ctrl.h (the BCM2712
# HDMI0 controller in DVI mode as a register-write list) against
# driver/zephyr/lib/board/test/hdmi_ctrl_mock.c: 640x480, 1280x720 and
# 1920x1080 at 60 Hz, the polarity bits, the refused clock and the disable
# sequence. Built with -std=c99 -Wall -Wextra -Werror.
#
# Run from anywhere. Builds into build/v3d71/hdmi_ctrl/. The last line is
# "hdmi_ctrl_mock: N of N checks, 0 failed" and the exit 0 when it passes;
# exit 1 otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=hdmi_ctrl_mock
out=build/v3d71/hdmi_ctrl
t=driver/zephyr/lib/board/test
h=driver/zephyr/lib/board/rpi5_scanout/hdmi_ctrl.h
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/hdmi_ctrl_mock.c $h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
if ! "$cc" -std=c99 -g -O1 -Wall -Wextra -Werror "$t/hdmi_ctrl_mock.c" -o "$out/hdmi_ctrl_mock"; then
  echo "$name: build failed"
  exit 1
fi

o=$("$out/hdmi_ctrl_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
if [ "$rc" -ne 0 ] || ! printf '%s\n' "$o" | grep -Eq "^hdmi_ctrl_mock: ([0-9]+) of \1 checks, 0 failed\$"; then
  echo "$name: exited $rc or without its result line"
  exit 1
fi
exit 0
