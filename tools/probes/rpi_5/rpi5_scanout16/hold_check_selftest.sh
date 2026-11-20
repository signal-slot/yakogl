#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# hold_check_selftest: host test of src/hold_check.h (the long hold's decision
# logic of rpi5_scanout16). Compiles test/hold_check_test.c with the host cc
# under -std=c11 -Wall -Wextra -Werror into build/rpi5_scanout16/ and runs it.
# No C compiler: "gap: CC", exit 77. Last line:
# "hold_check_selftest: N checks, 0 failed", exit 0.
set -u
cd "$(dirname "$0")/../../../.." || exit 1

out=build/rpi5_scanout16
dir=tools/probes/rpi_5/rpi5_scanout16
hdr=driver/zephyr/lib/board/rpi5_scanout
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
mkdir -p "$out" || exit 1

if ! "$cc" -std=c11 -Wall -Wextra -Werror -I"$hdr" -o "$out/hold_check_test" \
  "$dir/test/hold_check_test.c"; then
  echo "hold_check_selftest: compile failed"
  exit 1
fi
res=$("$out/hold_check_test") || { echo "$res"; echo "hold_check_selftest: failed"; exit 1; }
echo "$res" | sed 's/^hold_check_test:/hold_check_selftest:/'
exit 0
