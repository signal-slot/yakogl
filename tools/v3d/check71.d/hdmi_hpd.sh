#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# hdmi_hpd: driver/zephyr/lib/board/rpi5_scanout/hdmi_hpd.h against a register
# model of the HDMI0 hotplug bit and the DDC controller
# (driver/zephyr/lib/board/test/hdmi_hpd_mock.c), built with -std=c11 -Wall
# -Wextra -Werror and AddressSanitizer + UBSan.
#
# Run from anywhere. Builds into build/v3d71/hdmi_hpd/. The last line is
# "hdmi_hpd: N of N checks, 0 failed" and the exit 0 when it passes; exit 1
# otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=hdmi_hpd
out=build/v3d71/hdmi_hpd
t=driver/zephyr/lib/board/test
b=driver/zephyr/lib/board
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/hdmi_hpd_mock.c $b/rpi5_scanout/hdmi_hpd.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
# shellcheck disable=SC2086
if ! "$cc" -std=c11 -g -O1 -Wall -Wextra -Werror $san "$t/hdmi_hpd_mock.c" -o "$out/mock"; then
  echo "$name: build failed"
  exit 1
fi
o=$("$out/mock" 2>&1)
rc=$?
printf '%s\n' "$o"
n=$(printf '%s\n' "$o" | sed -nE 's/^hdmi_hpd_mock: ([0-9]+) of \1 checks, 0 failed$/\1/p')
if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
  echo "$name: mock exited $rc or without its result line"
  exit 1
fi
echo "$name: $n of $n checks, 0 failed"
exit 0
