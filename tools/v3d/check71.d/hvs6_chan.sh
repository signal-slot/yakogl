#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# hvs6_chan: driver/zephyr/lib/board/rpi5_scanout/hvs6_chan.h (the HVS6
# channel setup as a register-write list) against
# driver/zephyr/lib/board/test/hvs6_chan_mock.c, host cc, -std=c99 -Wall
# -Wextra -Werror.
#
# Run from anywhere. Builds into build/v3d71/hvs6_chan/. The last line is
# "hvs6_chan_mock: N of N checks, 0 failed" and the exit 0 when the build and
# the run pass; exit 1 otherwise; exit 77 with "gap: CC" when there is no C
# compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=hvs6_chan
out=build/v3d71/hvs6_chan
t=driver/zephyr/lib/board/test
h=driver/zephyr/lib/board/rpi5_scanout
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/hvs6_chan_mock.c $h/hvs6_chan.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
if ! "$cc" -std=c99 -g -O1 -Wall -Wextra -Werror -I"$h" "$t/hvs6_chan_mock.c" -o "$out/hvs6_chan_mock"; then
  echo "$name: build failed"
  exit 1
fi
o=$("$out/hvs6_chan_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
if [ "$rc" -ne 0 ] || ! printf '%s\n' "$o" | tail -n 1 | grep -Eq '^hvs6_chan_mock: ([0-9]+) of \1 checks, 0 failed$'; then
  echo "$name: exited $rc or without its result line"
  exit 1
fi
exit 0
