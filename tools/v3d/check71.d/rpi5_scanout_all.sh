#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# rpi5_scanout_all: all Pi 5 display headers in one translation unit.
#
# driver/zephyr/lib/board/test/rpi5_scanout_all.c includes every header of
# driver/zephyr/lib/board/rpi5_scanout/ twice (a header without its include
# guard clashes in the second pass) and its main touches one symbol of each.
# It is compiled with the host cc under -std=c11 -Wall -Wextra -Werror
# (linked, never run) and, when an aarch64 compiler that builds a trivial
# freestanding probe is found (aarch64-zephyr-elf-gcc under
# ZEPHYR_SDK_INSTALL_DIR, else aarch64-linux-gnu-gcc), for aarch64 with
# -ffreestanding. No such compiler: a note, the host compile is the verdict
# (the per-header mocks are the other check71.d scripts; this one does not
# rerun them). A header with no #include lines in the .c file is a failure.
#
# Builds into build/v3d71/rpi5_scanout_all/. Last line:
# "rpi5_scanout_all: N headers, 0 failed", exit 0; a failure names itself.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=rpi5_scanout_all
out=build/v3d71/$name
src=driver/zephyr/lib/board/test/rpi5_scanout_all.c
hdir=driver/zephyr/lib/board/rpi5_scanout
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
mkdir -p "$out" || exit 1

failed=0
n=0
for h in "$hdir"/*.h; do
  x=$(basename "$h" .h)
  n=$((n + 1))
  if [ "$(grep -c "\"../rpi5_scanout/$x\.h\"" "$src")" -ne 2 ]; then
    echo "FAIL    $name: $x.h is not included twice in $src"
    failed=$((failed + 1))
  fi
done

if "$cc" -std=c11 -Wall -Wextra -Werror "$src" -o "$out/$name.host" 2>"$out/host.err"; then
  echo "ok      host C11 compile and link ($cc)"
else
  echo "FAIL    $name: host C11 compile:"
  sed 's/^/        /' "$out/host.err"
  failed=$((failed + 1))
fi

cands=""
if [ -n "${ZEPHYR_SDK_INSTALL_DIR:-}" ]; then
  for g in "$ZEPHYR_SDK_INSTALL_DIR/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc" \
           "$ZEPHYR_SDK_INSTALL_DIR/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc"; do
    [ -x "$g" ] && cands="$cands $g"
  done
fi
command -v aarch64-linux-gnu-gcc >/dev/null 2>&1 && cands="$cands aarch64-linux-gnu-gcc"
printf '#include <stdarg.h>\n#include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>\n#include <stdio.h>\n#include <string.h>\nint p(void);\nint p(void) { return 0; }\n' >"$out/probe.c"
aacc=""
for c in $cands; do
  if "$c" -std=c11 -ffreestanding -c "$out/probe.c" -o "$out/probe.o" 2>/dev/null; then
    aacc=$c
    break
  fi
done
if [ -z "$aacc" ]; then
  echo "note    no usable aarch64 compiler: the aarch64 compile was not run"
elif "$aacc" -std=c11 -ffreestanding -Wall -Wextra -Werror -c "$src" \
  -o "$out/$name.aarch64.o" 2>"$out/aarch64.err"; then
  echo "ok      aarch64 compile ($aacc)"
else
  echo "FAIL    $name: aarch64 compile ($aacc):"
  sed 's/^/        /' "$out/aarch64.err"
  failed=$((failed + 1))
fi

echo "$name: $n headers, $failed failed"
[ "$failed" -eq 0 ]
