#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# fb_query_mock: driver/zephyr/lib/board/rpi5_scanout/fb_query.h against a
# model of the firmware's framebuffer tags (driver/zephyr/lib/board/test/
# fb_query_mock.c), with -Wall -Wextra -Werror and AddressSanitizer + UBSan.
#
# Run from anywhere. Builds into build/v3d71/fb_query/. The last line is
# "fb_query_mock: N of N checks, 0 failed" and the exit 0 on pass; exit 1
# otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=fb_query_mock
out=build/v3d71/fb_query
t=driver/zephyr/lib/board/test
h=driver/zephyr/lib/board/rpi5_scanout
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/fb_query_mock.c $h/fb_query.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
# shellcheck disable=SC2086
if ! "$cc" -std=gnu11 -g -O1 -Wall -Wextra -Werror $san -I"$h" \
  "$t/fb_query_mock.c" -o "$out/fb_query_mock"; then
  echo "$name: build failed, 1 failed"
  exit 1
fi
o=$("$out/fb_query_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
n=$(printf '%s\n' "$o" | tail -n 1 | sed -nE "s/^$name: ([0-9]+) of \\1 checks, 0 failed\$/\\1/p")
if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
  echo "$name: exited $rc or without its result line, 1 failed"
  exit 1
fi
exit 0
