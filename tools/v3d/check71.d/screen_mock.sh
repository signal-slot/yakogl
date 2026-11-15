#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# screen_mock: driver/zephyr/lib/board/rpi5/board_rpi5.c, unchanged, over
# rpi4/rpi_mbox.c built for the BCM2712 (-DCONFIG_SOC_BCM2712), against a
# mocked mailbox and a model of the firmware's framebuffer tags
# (driver/zephyr/lib/board/test/rpi5_screen_mock.c with the Zephyr stubs under
# test/stub), with -std=gnu11 -Wall -Wextra -Werror and AddressSanitizer +
# UBSan.
#
# The Rust decisions (yd_rpi_*, yd_rpi4_* of yakogl_drivers.h) are NOT linked:
# like the other host mocks of the board layer, the test carries a C model of
# them, checked in its first case against the values of the Rust tests
# (driver/rust/src/board/rpi_fb.rs and rpi4.rs). The prototypes come from the
# real yakogl_drivers.h, so a changed signature fails this build.
#
# Run from anywhere. Builds into build/v3d71/screen_mock/. The last line is
# "screen_mock: N of N checks, 0 failed" and the exit 0 when it passes; exit 1
# otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=screen_mock
out=build/v3d71/screen_mock
t=driver/zephyr/lib/board/test
b=driver/zephyr/lib/board
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/rpi5_screen_mock.c $b/rpi5/board_rpi5.c $b/rpi5/board_rpi5.h $b/rpi4/rpi_mbox.c \
  $b/rpi4/rpi_mbox.h $b/rpi4/fbcon.h driver/rust/include/yakogl_drivers.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
flags="-std=gnu11 -g -O1 -Wall -Wextra -Werror $san -DCONFIG_SOC_BCM2712"
inc="-I$t/stub -I$b/rpi5 -I$b/rpi4 -I$b/include -Idriver/rust/include -Iinclude"

# shellcheck disable=SC2086
if ! "$cc" $flags $inc "$t/rpi5_screen_mock.c" -o "$out/rpi5_screen_mock"; then
  echo "$name: build failed, 1 failed"
  exit 1
fi
o=$("$out/rpi5_screen_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
n=$(printf '%s\n' "$o" | sed -nE 's/^rpi5_screen_mock: ([0-9]+) of \1 checks, 0 failed$/\1/p')
if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
  echo "$name: exited $rc or without its result line, 1 failed"
  exit 1
fi
echo "$name: $n of $n checks, 0 failed"
exit 0
