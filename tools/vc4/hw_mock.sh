#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# hw_mock: the VC4 V3D driver core (driver/src/vc4) against its register
# model (tools/vc4/hw_mock_test.c), with the firmware path through
# driver/zephyr/lib/board/rpi4/rpi_mbox.c built for the BCM2837 and the host
# Zephyr stubs of driver/zephyr/lib/board/test/stub. Built with `cc -Wall
# -Wextra -Werror`, as the gate's other host mocks are (tools/v3d's
# hw_mock_test row); VC4_MOCK_SAN=1 adds AddressSanitizer + UBSan for a local
# run. The gate row builds without them: no other gate row needs the
# sanitizer runtimes, and a runner without them (or whose kernel ASan does
# not start on) would fail the row for a reason that is not the code.
#
# Run from anywhere. Builds into build/vc4-check/. The last line is
# "vc4 hw_mock_test: N checks run (planned N), 0 failed" and the exit 0 when
# it passed; exit 1 otherwise; exit 77 with "gap: cc" when there is no C
# compiler.
set -u
cd "$(dirname "$0")/../.." || exit 1

out=build/vc4-check
if ! command -v cc >/dev/null 2>&1; then
  echo "gap: cc (no C compiler)"
  exit 77
fi
mkdir -p "$out" || exit 1
san=
[ "${VC4_MOCK_SAN:-0}" = 1 ] &&
  san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
# shellcheck disable=SC2086  # the flags are one word each
cc -std=gnu11 -g -O1 -Wall -Wextra -Werror $san -DCONFIG_SOC_BCM2837 \
  -Idriver/src/vc4 -Idriver/zephyr/lib/board/test/stub -Idriver/zephyr/lib/board/rpi4 \
  tools/vc4/hw_mock_test.c driver/src/vc4/vc4_hw.c driver/src/vc4/vc4_power.c \
  driver/zephyr/lib/board/rpi4/rpi_mbox.c -o "$out/hw_mock_test" || {
  echo "vc4 hw_mock_test: build failed"
  exit 1
}
"$out/hw_mock_test"
