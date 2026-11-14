#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# pv_hdmi: driver/zephyr/lib/board/rpi5_scanout/pv_hdmi.h (the BCM2712 pixel
# valve and HDMI0 state decoder) against register vectors
# (driver/zephyr/lib/board/test/pv_hdmi_mock.c), with -Wall -Werror and
# AddressSanitizer + UBSan.
#
# Run from anywhere. Builds into build/v3d71/pv_hdmi/. The last line is
# "pv_hdmi: N of N checks, 0 failed" and the exit 0 when it passes; exit 1
# otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=pv_hdmi
out=build/v3d71/pv_hdmi
t=driver/zephyr/lib/board/test
b=driver/zephyr/lib/board
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/pv_hdmi_mock.c $b/rpi5_scanout/pv_hdmi.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
flags="-std=gnu11 -g -O1 -Wall -Wextra -Werror $san -I$b"

# shellcheck disable=SC2086
if ! "$cc" $flags "$t/pv_hdmi_mock.c" -o "$out/pv_hdmi_mock"; then
  echo "$name: build failed, 1 failed"
  exit 1
fi
o=$("$out/pv_hdmi_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
n=$(printf '%s\n' "$o" | sed -nE 's/^pv_hdmi_mock: ([0-9]+) of \1 checks, 0 failed$/\1/p')
if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
  echo "$name: exited $rc or without its result line, 1 failed"
  exit 1
fi
echo "$name: $n of $n checks, 0 failed"
exit 0
