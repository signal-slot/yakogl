#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# hazards71: Mesa 26.2.1's qpu_validate verdicts at V3D 7.1 as an oracle table
# for src/shaderc/online/v3d/verify71.rs. Builds tools/v3d/hazards71.c with
# Mesa's own compiler/qpu_validate.c (copied byte for byte into the build
# directory, next to tools/v3d/hazards71/v3d_compiler.h), runs its built-in
# cases, and compares the table with the committed tools/v3d/hazards71/oracle.tsv
# byte for byte. The driver itself exits 1 when Mesa's verdict differs from the
# one its author read off the source.
#
#   tools/v3d/hazards71.sh            build, run, compare with oracle.tsv
#   tools/v3d/hazards71.sh --update   write oracle.tsv from Mesa's answers
#
# Needs MESA_V3D_TREE: a full Mesa 26.2.1 tree (src/broadcom/compiler is not
# in third_party/) with the v3d build of tools/v3d/README.md (build-v3d;
# MESA_V3D_BUILD names another build directory in it). Without it the script
# exits 77 with `gap: MESA_V3D_TREE`; a gap is not a pass. Output in
# build/v3d71/hazards71/.
set -u
cd "$(dirname "$0")/../.." || exit 1

name=hazards71
dir=build/v3d71/$name
oracle=tools/v3d/hazards71/oracle.tsv
mkdir -p "$dir" || exit 1

update=0
[ "${1:-}" = "--update" ] && update=1

tree=${MESA_V3D_TREE:-}
build=${MESA_V3D_BUILD:-build-v3d}
if [ ! -f "$tree/src/broadcom/compiler/qpu_validate.c" ] || [ ! -f "$tree/$build/src/util/libmesa_util.a" ]; then
  echo "MESA_V3D_TREE='$tree' has no src/broadcom/compiler/qpu_validate.c and $build/src/util/libmesa_util.a"
  echo "gap: MESA_V3D_TREE"
  exit 77
fi

cp "$tree/src/broadcom/compiler/qpu_validate.c" "$dir/qpu_validate.c" || exit 1
cp tools/v3d/hazards71/v3d_compiler.h "$dir/v3d_compiler.h" || exit 1
qpu=$tree/src/broadcom/qpu
cc -O2 -std=gnu11 -DNDEBUG -DMESA_DEBUG=1 -D_GNU_SOURCE -DHAVE_ENDIAN_H \
  -I"$dir" -I"$tree/include" -I"$tree/src" -I"$tree/src/broadcom" -I"$tree/$build/src" \
  tools/v3d/hazards71.c "$dir/qpu_validate.c" "$qpu/qpu_pack.c" "$qpu/qpu_instr.c" \
  "$qpu/qpu_disasm.c" "$tree/$build/src/util/libmesa_util.a" -lm -lpthread \
  -o "$dir/hazards71" >"$dir/cc.log" 2>&1 || {
  tail -20 "$dir/cc.log"
  echo "$name: build failed"
  exit 1
}

"$dir/hazards71" >"$dir/table.tsv" 2>"$dir/stderr.txt"
rc=$?
if [ "$rc" != 0 ]; then
  cat "$dir/stderr.txt"
  echo "$name: the driver exited $rc, 1 failed"
  exit 1
fi
n=$(wc -l <"$dir/table.tsv")
nfail=$(awk -F'\t' '$5 == "fail"' "$dir/table.tsv" | wc -l)

if [ "$update" = 1 ]; then
  cp "$dir/table.tsv" "$oracle" || exit 1
  echo "$name: wrote $oracle ($n cases, $nfail refused), 0 failed"
  exit 0
fi
if ! cmp -s "$dir/table.tsv" "$oracle"; then
  diff "$oracle" "$dir/table.tsv" | head -20
  echo "$name: the table differs from $oracle, 1 failed"
  exit 1
fi
echo "$name: $n cases ($nfail refused, $((n - nfail)) accepted) as Mesa's qpu_validate at ver 71 answers, 0 failed"
