#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# hdmi_rates: driver/zephyr/lib/board/rpi5_scanout/hdmi_rates.h, the
# header-only HDMI clock rate rules of the BCM2712 (TMDS, HSM, BVB, pixel
# limit), tested by driver/zephyr/lib/board/test/hdmi_rates_mock.c with
# -Wall -Wextra -Werror and AddressSanitizer + UBSan.
#
# Run from anywhere. Builds into build/v3d71/hdmi_rates/. The last line
# is "hdmi_rates: N of N checks, 0 failed" and the exit 0 when the mock
# passes; exit 1 otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=hdmi_rates
out=build/v3d71/hdmi_rates
t=driver/zephyr/lib/board/test
h=driver/zephyr/lib/board/rpi5_scanout
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/hdmi_rates_mock.c $h/hdmi_rates.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
flags="-std=gnu11 -g -O1 -Wall -Wextra -Werror $san -I$h"

# shellcheck disable=SC2086
if ! "$cc" $flags "$t/hdmi_rates_mock.c" -o "$out/hdmi_rates_mock"; then
  echo "$name: build failed"
  exit 1
fi
o=$("$out/hdmi_rates_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
n=$(printf '%s\n' "$o" | sed -nE 's/^hdmi_rates_mock: ([0-9]+) of \1 checks, 0 failed$/\1/p')
if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
  echo "$name: the mock exited $rc or without its result line"
  exit 1
fi
echo "$name: $n of $n checks, 0 failed"
exit 0
