#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# hold_check_selftest: compiles test/hold_check_test.c (host test of
# src/hold_check.h) with the host cc under -std=c11 -Wall -Wextra -Werror
# into build/rpi5_scanout16/ and runs it. Last line:
# "hold_check_selftest: N checks, 0 failed"; exit 0. No cc: a gap, exit 77.
cd "$(dirname "$0")/../../../.." || exit 1
cc=${CC:-cc}
out=build/rpi5_scanout16
if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
mkdir -p "$out" || exit 1
if ! "$cc" -std=c11 -Wall -Wextra -Werror -o "$out/hold_check_test" \
    tools/probes/rpi_5/rpi5_scanout16/test/hold_check_test.c; then
  echo "hold_check_selftest: compile failed"
  exit 1
fi
res=$("$out/hold_check_test")
rc=$?
printf '%s\n' "$res"
if [ $rc -ne 0 ]; then
  echo "hold_check_selftest: FAILED"
  exit 1
fi
echo "hold_check_selftest: $(printf '%s\n' "$res" | tail -n 1 | sed 's/^hold_check_test: //')"
exit 0
