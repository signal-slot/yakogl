#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# bus_addr: driver/zephyr/lib/board/rpi5_scanout/bus_addr.h (BCM2712 bus/ARM
# address translation, dlist low/high words, DRAM range check) against
# driver/zephyr/lib/board/test/bus_addr_mock.c, built with the host cc as
# C11 with -Wall -Wextra -Werror, once plain and once with AddressSanitizer +
# UBSan; the header also compiles alone.
#
# Run from anywhere. Builds into build/v3d71/bus_addr/. The last line is
# "bus_addr: N of N checks, 0 failed" and the exit 0 when both builds pass;
# exit 1 otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=bus_addr
out=build/v3d71/bus_addr
t=driver/zephyr/lib/board/test
h=driver/zephyr/lib/board/rpi5_scanout/bus_addr.h
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/bus_addr_mock.c $h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
flags="-std=c11 -g -O1 -Wall -Wextra -Werror"
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"

fail=0
total=0

# The header alone, as a translation unit of its own.
printf '#include "%s"\nint main(void) { return 0; }\n' "$PWD/$h" >"$out/alone.c"
# shellcheck disable=SC2086
if ! "$cc" $flags "$out/alone.c" -o "$out/alone"; then
  echo "$name: header alone failed to build"
  fail=$((fail + 1))
fi

for variant in plain san; do
  case $variant in
  san) extra=$san ;;
  *) extra="" ;;
  esac
  # shellcheck disable=SC2086
  if ! "$cc" $flags $extra "$t/bus_addr_mock.c" -o "$out/$variant"; then
    echo "$name: $variant build failed"
    fail=$((fail + 1))
    continue
  fi
  o=$("$out/$variant" 2>&1)
  rc=$?
  printf '%s\n' "$o"
  n=$(printf '%s\n' "$o" | sed -nE 's/^bus_addr_mock: ([0-9]+) of \1 checks, 0 failed$/\1/p')
  if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
    echo "$name: $variant exited $rc or without its result line"
    fail=$((fail + 1))
  else
    total=$((total + n))
  fi
done

if [ "$fail" -eq 0 ]; then
  echo "$name: $total of $total checks, 0 failed"
  exit 0
fi
echo "$name: $fail build(s) failed"
exit 1
