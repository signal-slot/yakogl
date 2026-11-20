#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# fbcon565: driver/zephyr/lib/board/rpi5_scanout/fbcon565.h (the 16 bpp RGB565
# text renderer of the Pi 5 stage 1 scanout) against
# driver/zephyr/lib/board/test/fbcon565_mock.c, with -Wall -Wextra -Werror and
# AddressSanitizer + UBSan; the header is also compiled as C99.
#
# Run from anywhere. Builds into build/v3d71/fbcon565/. The last line is
# "fbcon565: N of N checks, 0 failed" and the exit 0 when it passes; exit 1
# otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=fbcon565
out=build/v3d71/fbcon565
t=driver/zephyr/lib/board/test
b=driver/zephyr/lib/board
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/fbcon565_mock.c $b/rpi5_scanout/fbcon565.h $b/common/gc_font8x16.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
flags="-std=gnu11 -g -O1 -Wall -Wextra -Werror $san -I$b/rpi5_scanout"

# The header alone must be C99 clean (no GNU extensions, no Zephyr includes).
printf '#include "fbcon565.h"\nint main(void) { return (int)yd5_fc565_pack(0, 0, 0); }\n' >"$out/c99.c"
if ! "$cc" -std=c99 -pedantic -Wall -Wextra -Werror -I"$b/rpi5_scanout" "$out/c99.c" -o "$out/c99"; then
  echo "$name: C99 compile of the header failed, 1 failed"
  exit 1
fi

# shellcheck disable=SC2086
if ! "$cc" $flags "$t/fbcon565_mock.c" -o "$out/fbcon565_mock"; then
  echo "$name: build failed, 1 failed"
  exit 1
fi
o=$("$out/fbcon565_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
n=$(printf '%s\n' "$o" | tail -n 1 | sed -nE "s/^fbcon565_mock: ([0-9]+) of \\1 checks, 0 failed\$/\\1/p")
if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
  echo "$name: exited $rc or without its result line, 1 failed"
  exit 1
fi
echo "$name: $n of $n checks, 0 failed"
exit 0
