#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# hw_mock: driver/src/v3d/v3d_hw.c's V3D 7.1 (BCM2712) path against the
# register model of tools/v3d/hw_mock_test.c (--v71: the 7.x map, the SMS and
# pm's 2712 reset register, no bridge), and the same binary's 4.2 run, which
# must stay as tools/check.sh's v3d row pins it.
#
# Run from the repository root. Builds into build/v3d71/hw_mock/. The last
# line is "hw_mock: <summary>, 0 failed" and the exit 0 when both runs pass;
# exit 1 otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u

name=hw_mock
out=build/v3d71/hw_mock
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in tools/v3d/hw_mock_test.c driver/src/v3d/v3d_hw.c driver/src/v3d/v3d.h driver/src/v3d/v3d_regs.h; do
  [ -f "$f" ] || { echo "$name: $f missing (run from the repository root), 1 failed"; exit 1; }
done

mkdir -p "$out"
if ! "$cc" -std=c11 -Wall -Wextra -Werror -Itools/v3d/hostshim -Idriver/src/v3d \
    tools/v3d/hw_mock_test.c driver/src/v3d/v3d_hw.c -o "$out/hw_mock_test"; then
  echo "$name: build failed, 1 failed"
  exit 1
fi

fail=0
# the 7.1 run
o71=$("$out/hw_mock_test" --v71)
rc71=$?
printf '%s\n' "$o71" | grep -v '^  '
s71=$(printf '%s\n' "$o71" | sed -nE 's/^hw_mock_test 7\.1: ([0-9]+) checks run \(planned \1\), 0 failed$/\1/p')
if [ "$rc71" -ne 0 ] || [ -z "$s71" ]; then
  echo "hw_mock_test --v71 exited $rc71 or without its result line"
  fail=$((fail + 1))
fi

# the 4.2 run: its result line exactly as tools/check.sh's v3d row reads it
o42=$("$out/hw_mock_test")
rc42=$?
s42=$(printf '%s\n' "$o42" | sed -nE 's/^hw_mock_test: ([0-9]+) checks run \(planned \1\), 0 failed$/\1/p')
if [ "$rc42" -ne 0 ] || [ -z "$s42" ]; then
  printf '%s\n' "$o42" | tail -20
  echo "hw_mock_test (4.2) exited $rc42 or without its result line"
  fail=$((fail + 1))
fi

echo "$name: 7.1 ${s71:-?} of ${s71:-?} checks, 4.2 ${s42:-?} of ${s42:-?} checks, $fail failed"
[ "$fail" -eq 0 ]
