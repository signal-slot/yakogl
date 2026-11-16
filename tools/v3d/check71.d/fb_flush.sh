#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# fb_flush: driver/zephyr/lib/board/rpi5_scanout/fb_flush.h, the header-only
# cache-clean range planner for the Pi 5 scanout buffer
# (driver/zephyr/lib/board/test/fb_flush_mock.c), with -Wall -Wextra -Werror and
# AddressSanitizer + UBSan. The header is also compiled for aarch64 with
# -Werror -fsyntax-only when a cross compiler is present ("gap: aarch64 cc"
# otherwise, not a failure).
#
# Run from anywhere. Builds into build/v3d71/fb_flush/. The last line is
# "fb_flush: N of N checks, 0 failed" and the exit 0 when the mock passes;
# exit 1 otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=fb_flush
out=build/v3d71/fb_flush
t=driver/zephyr/lib/board/test
h=driver/zephyr/lib/board/rpi5_scanout
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/fb_flush_mock.c $h/fb_flush.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
flags="-std=gnu11 -g -O1 -Wall -Wextra -Werror $san -I$h"

# shellcheck disable=SC2086
if ! "$cc" $flags "$t/fb_flush_mock.c" -o "$out/fb_flush_mock"; then
  echo "$name: build failed"
  exit 1
fi

# the header alone for aarch64, as the Zephyr image compiles it
a64=""
for c in aarch64-linux-gnu-gcc aarch64-none-elf-gcc aarch64-zephyr-elf-gcc; do
  if command -v "$c" >/dev/null 2>&1; then
    a64=$c
    break
  fi
done
if [ -n "$a64" ]; then
  printf '#include "fb_flush.h"\n' >"$out/a64.c"
  if ! "$a64" -std=c11 -Wall -Wextra -Werror -fsyntax-only -I"$h" "$out/a64.c"; then
    echo "$name: aarch64 syntax check failed"
    exit 1
  fi
  echo "aarch64 syntax: ok ($a64)"
else
  echo "gap: aarch64 cc"
fi

o=$("$out/fb_flush_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
n=$(printf '%s\n' "$o" | sed -nE 's/^fb_flush_mock: ([0-9]+) of \1 checks, 0 failed$/\1/p')
if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
  echo "$name: the mock exited $rc or without its result line"
  exit 1
fi
echo "$name: $n of $n checks, 0 failed"
exit 0
