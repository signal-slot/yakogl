#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# hvs6_vblank: driver/zephyr/lib/board/rpi5_scanout/hvs6_vblank.h (the
# read-only FRCNT-step frame-edge detector for HVS6 channel 0) against
# driver/zephyr/lib/board/test/hvs6_vblank_mock.c, host cc, -std=c99 -Wall
# -Wextra -Werror. The header is also compiled for aarch64 when a cross
# compiler is present (CROSS_CC, aarch64-linux-gnu-gcc or aarch64-none-elf-gcc);
# otherwise that step is skipped by name.
#
# Run from anywhere. Builds into build/v3d71/hvs6_vblank/. The last line is
# "hvs6_vblank: N of N checks, 0 failed" and the exit 0 when the build and
# the run pass; exit 1 otherwise; exit 77 with "gap: CC" when there is no C
# compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=hvs6_vblank
out=build/v3d71/hvs6_vblank
t=driver/zephyr/lib/board/test
h=driver/zephyr/lib/board/rpi5_scanout
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $t/hvs6_vblank_mock.c $h/hvs6_vblank.h; do
  [ -f "$f" ] || { echo "$name: $f missing, 1 failed"; exit 1; }
done

mkdir -p "$out"
if ! "$cc" -std=c99 -g -O1 -Wall -Wextra -Werror -I"$h" "$t/hvs6_vblank_mock.c" -o "$out/hvs6_vblank_mock"; then
  echo "$name: build failed"
  exit 1
fi

cross=
for c in "${CROSS_CC:-}" aarch64-linux-gnu-gcc aarch64-none-elf-gcc; do
  [ -n "$c" ] && command -v "$c" >/dev/null 2>&1 && { cross=$c; break; }
done
if [ -n "$cross" ]; then
  printf '#include "hvs6_vblank.h"\nint vb_probe(const struct hvs6v_io *io, const struct hvs6v_budget *b)\n{\n\treturn (int)hvs6v_wait_frame(io, b).code;\n}\n' > "$out/aarch64_probe.c"
  if ! "$cross" -std=c99 -O1 -Wall -Wextra -Werror -ffreestanding -I"$h" -c "$out/aarch64_probe.c" -o "$out/aarch64_probe.o"; then
    echo "$name: aarch64 compile failed ($cross)"
    exit 1
  fi
  echo "aarch64 compile: ok ($cross)"
else
  echo "aarch64 compile: skipped (no aarch64 cross compiler)"
fi

o=$("$out/hvs6_vblank_mock" 2>&1)
rc=$?
printf '%s\n' "$o"
if [ "$rc" -ne 0 ] || ! printf '%s\n' "$o" | tail -n 1 | grep -Eq '^hvs6_vblank: ([0-9]+) of \1 checks, 0 failed$'; then
  echo "$name: exited $rc or without its result line"
  exit 1
fi
exit 0
