#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# adopt: driver/zephyr/lib/board/rpi5_scanout/adopt.h (route 1 of the Pi 5
# display: adopt the firmware's scanout plane or not) against
# driver/zephyr/lib/board/test/adopt_mock.c: every verdict and reason and the
# boundary cases, compiled -std=c11 -Wall -Wextra -Werror with AddressSanitizer
# + UBSan.
#
# Run from anywhere. Builds into build/v3d71/adopt/. The last line is
# "adopt: N of N checks, 0 failed" and the exit 0 when it passes; exit 1
# otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=adopt
out=build/v3d71/adopt
t=driver/zephyr/lib/board/test
h=driver/zephyr/lib/board/rpi5_scanout/adopt.h
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/adopt_mock.c $h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
# shellcheck disable=SC2086
if ! "$cc" -std=c11 -g -O1 -Wall -Wextra -Werror $san "$t/adopt_mock.c" -o "$out/adopt_mock"; then
  echo "$name: build failed, 1 failed"
  exit 1
fi
o=$("$out/adopt_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
n=$(printf '%s\n' "$o" | sed -nE "s/^adopt_mock: ([0-9]+) of \\1 checks, 0 failed\$/\\1/p")
if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
  echo "$name: exited $rc or without its result line, 1 failed"
  exit 1
fi
echo "$name: $n of $n checks, 0 failed"
exit 0
