#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# wdog_mock: driver/zephyr/lib/board/rpi4/rpi_wdog.c, unchanged, against a
# mocked PM register block (driver/zephyr/lib/board/test/rpi_wdog_mock.c with
# Zephyr stubs), built twice: for the BCM2711 (Pi 4) and for the BCM2712
# (Pi 5, CONFIG_SOC_BCM2712 + CONFIG_YAKOGL_RPI5_WDOG), with -Wall -Werror and
# AddressSanitizer + UBSan.
#
# Run from anywhere. Builds into build/v3d71/wdog_mock/. The last line is
# "rpi_wdog_mock: N of N checks, 0 failed" and the exit 0 when both builds
# pass; exit 1 otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=rpi_wdog_mock
out=build/v3d71/wdog_mock
t=driver/zephyr/lib/board/test
b=driver/zephyr/lib/board
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/rpi_wdog_mock.c $b/rpi4/rpi_wdog.c; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
flags="-std=gnu11 -g -O1 -Wall -Wextra -Werror $san -I$t/stub -I$b/rpi4 -I$b/common"

fail=0
total=0
for soc in bcm2711 bcm2712; do
  case $soc in
  bcm2712) defs="-DCONFIG_SOC_BCM2712 -DCONFIG_YAKOGL_RPI5_WDOG=1 -DCONFIG_YAKOGL_RPI5_WDOG_HOLD_S=20" ;;
  *) defs="" ;;
  esac
  # shellcheck disable=SC2086
  if ! "$cc" $flags $defs "$t/rpi_wdog_mock.c" -o "$out/$soc"; then
    echo "$name: $soc build failed"
    fail=$((fail + 1))
    continue
  fi
  o=$("$out/$soc" 2>&1)
  rc=$?
  printf '%s\n' "$o"
  n=$(printf '%s\n' "$o" | sed -nE "s/^rpi_wdog_mock $soc: ([0-9]+) of \\1 checks, 0 failed\$/\\1/p")
  if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
    echo "$name: $soc exited $rc or without its result line"
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
