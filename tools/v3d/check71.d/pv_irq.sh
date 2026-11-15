#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# pv_irq: driver/zephyr/lib/board/rpi5_scanout/pv_irq.h, the header-only
# BCM2712 pixel valve vblank status decode, acknowledge list and bounded wait,
# against a fake register file (driver/zephyr/lib/board/test/pv_irq_mock.c),
# with -Wall -Wextra -Werror and AddressSanitizer + UBSan.
#
# Run from anywhere. Builds into build/v3d71/pv_irq/. The last line is
# "pv_irq: N of N checks, 0 failed" and the exit 0 when the mock passes; exit
# 1 otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=pv_irq
out=build/v3d71/pv_irq
t=driver/zephyr/lib/board/test
h=driver/zephyr/lib/board/rpi5_scanout
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/pv_irq_mock.c $h/pv_irq.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
flags="-std=c11 -g -O1 -Wall -Wextra -Werror $san -I$h"

# shellcheck disable=SC2086
if ! "$cc" $flags "$t/pv_irq_mock.c" -o "$out/pv_irq_mock"; then
  echo "$name: build failed"
  exit 1
fi
o=$("$out/pv_irq_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
n=$(printf '%s\n' "$o" | sed -nE 's/^pv_irq_mock: ([0-9]+) of \1 checks, 0 failed$/\1/p')
if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
  echo "$name: the mock exited $rc or without its result line"
  exit 1
fi
echo "$name: $n of $n checks, 0 failed"
exit 0
