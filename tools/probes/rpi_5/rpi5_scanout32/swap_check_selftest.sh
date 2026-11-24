#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# swap_check_selftest: compiles test/swap_check_test.c (host test of
# src/swap_check.h) with the host cc under -std=c11 -Wall -Wextra -Werror
# into build/rpi5_scanout32/ and runs it. Last line:
# "swap_check_selftest: N checks, 0 failed"; exit 0. No cc: a gap, exit 77.
# Covers the DL.LACT pickup helpers too (lact-probe-a): swap_named,
# swap_seq_judge_lact, swap_restore_lact (the swap-1-only path),
# swap_restore_pass and swap_clear_ok; and the vblank landing (swap-lact-probe):
# swap_vblank_judge and swap_vblank_first_miss (in vblank, a missed frame, a
# pickup timeout, no vblank found, a write outside it).
cd "$(dirname "$0")/../../../.." || exit 1
cc=${CC:-cc}
out=build/rpi5_scanout32
if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
mkdir -p "$out" || exit 1
if ! "$cc" -std=c11 -Wall -Wextra -Werror -o "$out/swap_check_test" \
    tools/probes/rpi_5/rpi5_scanout32/test/swap_check_test.c; then
  echo "swap_check_selftest: compile failed"
  exit 1
fi
res=$("$out/swap_check_test")
rc=$?
printf '%s\n' "$res"
if [ $rc -ne 0 ]; then
  echo "swap_check_selftest: FAILED"
  exit 1
fi
echo "swap_check_selftest: $(printf '%s\n' "$res" | tail -n 1 | sed 's/^swap_check_test: //')"
exit 0
