#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# hvs6_dlist_swap: driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h,
# the firmware's list found from LPTRS.HEADE and walked to its END (bounded),
# our HVS6 display list placed in zero words clear of it (from word 32), the
# channel's LPTRS swapped as the last single write, the read-back and FRCNT
# results, and the byte-equal restore of exactly the firmware's words and
# LPTRS, against a model of one D0 channel, including the board's layout
# (HEADE=672), and the declared hardware-written words (SKIP/EXPECT by
# plane-entry offset, every differing word named, at the restore and at our
# slot's read-back)
# (driver/zephyr/lib/board/test/hvs6_dlist_swap_mock.c), with -std=c11
# -Wall -Wextra -Werror and AddressSanitizer + UBSan; the header also
# compiles alone under -std=c11 -pedantic.
#
# Run from anywhere. Builds into build/v3d71/hvs6_dlist_swap/. The last line
# is "hvs6_dlist_swap: N checks, 0 failed" and the exit 0 when the mock
# passes; exit 1 otherwise; exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=hvs6_dlist_swap
out=build/v3d71/$name
t=driver/zephyr/lib/board/test
h=driver/zephyr/lib/board/rpi5_scanout
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/hvs6_dlist_swap_mock.c $h/hvs6_dlist_swap.h $h/hvs6_dlist_write.h $h/dlist_alloc.h \
  $h/bus_addr.h $h/fb_flush.h $h/hvs6_chan.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
san="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
flags="-std=c11 -g -O1 -Wall -Wextra -Werror $san -I$h"

# The header alone: C11, pedantic, no Zephyr includes.
printf '#include "hvs6_dlist_swap.h"\nint main(void) { return (int)hvs6s_frdelta(0, 0); }\n' >"$out/alone.c"
if ! "$cc" -std=c11 -pedantic -Wall -Wextra -Werror -I"$h" "$out/alone.c" -o "$out/alone"; then
  echo "$name: the header alone does not compile, 1 failed"
  exit 1
fi
if grep -q '#include <zephyr' "$h/hvs6_dlist_swap.h"; then
  echo "$name: the header includes Zephyr, 1 failed"
  exit 1
fi

# shellcheck disable=SC2086
if ! "$cc" $flags "$t/hvs6_dlist_swap_mock.c" -o "$out/hvs6_dlist_swap_mock"; then
  echo "$name: build failed, 1 failed"
  exit 1
fi
o=$("$out/hvs6_dlist_swap_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
n=$(printf '%s\n' "$o" | tail -n 1 | sed -nE 's/^hvs6_dlist_swap_mock: ([0-9]+) of \1 checks, 0 failed$/\1/p')
if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
  echo "$name: the mock exited $rc or without its result line, 1 failed"
  exit 1
fi
echo "$name: $n checks, 0 failed"
exit 0
