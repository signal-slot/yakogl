#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Build and run tools/v3d/tiling_bench.c (docs/notes/v3d-tiling-path.md):
#
#   tools/v3d/tiling_bench.sh [runs]
#
# Host build at -O2 and at -Os (the board image's level is Zephyr's default;
# the sample sets none). Needs only the sources vendored in third_party. Output
# goes to build/v3d-check/ (gitignored). Exit status: the bench's (1 when the
# two walks write different bytes), 1 for a build failure.
#
# With an aarch64 compiler (CC_aarch64_unknown_none, an aarch64 gcc, or clang
# with --target=aarch64) it also compiles the bench for aarch64 (not run) and
# says whether the st1 utile code is in the object.
set -u
cd "$(dirname "$0")/../.." || exit 1

O=build/v3d-check
mkdir -p "$O" || exit 1
T=third_party/mesa-26.2.1
INC=(-Idriver/src/v3d -Idriver/src/v3d/generated -I"$T/src/broadcom" -I"$T/src" -I"$T/include"
  -Itools/v3d)
SRC=(tools/v3d/tiling_bench.c driver/src/v3d/v3d_tex.c)

rc=0
for opt in -O2 -Os; do
  cc -std=c11 "$opt" -Wall -Wextra -Werror -D_GNU_SOURCE -DHAVE_ENDIAN_H "${INC[@]}" "${SRC[@]}" \
    -lm -o "$O/tiling_bench$opt" || exit 1
  echo "== host $opt"
  "$O/tiling_bench$opt" "$@" || rc=1
done

A=${CC_aarch64_unknown_none:-}
if [ -z "$A" ]; then
  for c in aarch64-linux-gnu-gcc aarch64-none-elf-gcc aarch64-unknown-linux-gnu-gcc; do
    command -v "$c" >/dev/null 2>&1 && { A=$c; break; }
  done
fi
AARCH=()
if [ -n "$A" ]; then
  AARCH=("$A")
elif command -v clang >/dev/null 2>&1 &&
  clang --target=aarch64-none-elf -x c -c /dev/null -o /dev/null -ffreestanding 2>/dev/null; then
  AARCH=(clang --target=aarch64-none-elf)
fi
if [ ${#AARCH[@]} -gt 0 ]; then
  echo "== aarch64 build (${AARCH[*]}), not run"
  # the bench's own object only: the question is which utile code is compiled in
  if "${AARCH[@]}" -std=c11 -Os -ffreestanding -w -D_GNU_SOURCE -DHAVE_ENDIAN_H "${INC[@]}" \
    -isystem /usr/include -c tools/v3d/tiling_bench.c -o "$O/tiling_bench_aarch64.o" 2>"$O/aarch64.err"; then
    if command -v llvm-objdump >/dev/null 2>&1; then
      n=$(llvm-objdump -d "$O/tiling_bench_aarch64.o" | grep -cE '\bst1\b')
      echo "aarch64 object: $n st1 instructions (the utile store is the NEON code when > 0)"
    else
      echo "aarch64 object built; no llvm-objdump to look inside"
    fi
  else
    echo "aarch64 object did not build (host headers; see $O/aarch64.err)"
  fi
else
  echo "== no aarch64 compiler: the aarch64 build is not checked"
fi
exit $rc
