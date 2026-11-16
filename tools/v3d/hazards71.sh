#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# hazards71.sh - rebuild tools/v3d/hazards71.c against Mesa 26.2.1 and write
# tools/v3d/hazards71.tsv: Mesa's qpu_validate verdict at V3D 7.1 for each
# hand-chosen instruction sequence (the format is in hazards71.c's header).
#
# Usage:
#   tools/v3d/hazards71.sh           rebuild, rewrite hazards71.tsv
#   tools/v3d/hazards71.sh --check   rebuild, compare with hazards71.tsv byte
#                                    for byte, change nothing
#   --tree DIR / --build DIR         instead of MESA_V3D_TREE / HAZARDS71_BUILD
#
# MESA_V3D_TREE names a Mesa 26.2.1 source tree (tools/v3d/README.md: the tree
# of the v3d_shc set-up). Its generated headers are looked for in
# $HAZARDS71_BUILD (default $MESA_V3D_TREE/build-v3d) when that directory
# exists; the sources need none. The binary goes to $TMPDIR (or the repository's
# build/hazards71), never to /tmp.
#
# Exit status: 0 ok; 1 the table differs (--check) or the build failed;
# 77 MESA_V3D_TREE is not set or is not a Mesa tree (a gap, not a pass).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
TSV="$HERE/hazards71.tsv"
mode=
M=${MESA_V3D_TREE-}
B=${HAZARDS71_BUILD-}
while [ $# -gt 0 ]; do
  case "$1" in
  --check) mode=--check ;;
  --tree) M=${2?}; shift ;;
  --build) B=${2?}; shift ;;
  *)
    sed -n '9,16p' "$0" >&2
    exit 2
    ;;
  esac
  shift
done

if [ -z "$M" ] || [ ! -f "$M/src/broadcom/compiler/qpu_validate.c" ]; then
  echo "hazards71: gap: MESA_V3D_TREE is not set to a Mesa 26.2.1 tree (tools/v3d/README.md)" >&2
  exit 77
fi
if ! grep -q '^26\.2\.1' "$M/VERSION" 2>/dev/null; then
  echo "hazards71: gap: $M is not Mesa 26.2.1" >&2
  exit 77
fi
[ -n "$B" ] || B=$M/build-v3d

OUT=${TMPDIR:-$HERE/../../build}/hazards71-build
mkdir -p "$OUT"
inc=(-I"$M/include" -I"$M/src" -I"$M/src/broadcom")
for d in "$B/include" "$B/src"; do [ -d "$d" ] && inc+=(-I"$d"); done

cc -O1 -std=gnu11 -w -DNDEBUG -DMESA_DEBUG=1 -DHAVE_ENDIAN_H -D_GNU_SOURCE \
  "${inc[@]}" \
  "$HERE/hazards71.c" \
  "$M/src/broadcom/qpu/qpu_pack.c" "$M/src/broadcom/qpu/qpu_instr.c" \
  -lm -o "$OUT/hazards71"

"$OUT/hazards71" > "$OUT/hazards71.tsv"
# the case count: the lines that are not the `#` header
cases() { grep -c -v '^#' "$1"; }
if [ "$mode" = --check ]; then
  if cmp -s "$OUT/hazards71.tsv" "$TSV"; then
    echo "hazards71: $(cases "$TSV") cases, identical to hazards71.tsv"
  else
    diff "$TSV" "$OUT/hazards71.tsv" | head -20 >&2 || true
    echo "hazards71: the table differs from hazards71.tsv" >&2
    exit 1
  fi
else
  cp "$OUT/hazards71.tsv" "$TSV"
  echo "hazards71: wrote $(cases "$TSV") cases to hazards71.tsv"
fi
