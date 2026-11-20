#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# tri_validate71: Mesa 26.2.1's qpu_validate (src/broadcom/compiler/
# qpu_validate.c) at V3D 7.1.10 over the five Pi 5 triangle programs of
# tools/v3d/tri-shaders71/: Mesa's FS, VS and CS, and our online compiler's
# pass-through FS and VS. One verdict line per program, each program's
# disassembly (Mesa's v3d_qpu_disasm) under it, then
# "tri_validate71: 5 programs, N failed". A program the validator rejects counts
# as failed, with the instruction and the validator's message.
#
# Needs a Mesa 26.2.1 source tree (qpu_validate.c, qpu_pack.c, qpu_disasm.c;
# nothing is built in it): MESA_V3D_TREE, else
# $CONNECTED_DEPS_DIR/mesa-26.2.1-src/mesa-26.2.1 when it exists. Neither: a
# SKIPPED line and exit 0 (a pre-board check, not a gate row).
# Binary: build/v3d71/tri_validate71/ (never /tmp).
# Exit status: 0 when no Mesa program failed (or SKIPPED); 1 when a Mesa
# program or the build failed. A rejection of one of OUR programs is the
# named refusal the board session records (it is counted in the last line and
# printed with the instruction and message, but is not a gate failure: the
# online compiler's 7.1 back end is not finished, #7). Convention: tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=tri_validate71
M=${MESA_V3D_TREE-}
if [ -z "$M" ] || [ ! -f "$M/src/broadcom/compiler/qpu_validate.c" ]; then
  M=${CONNECTED_DEPS_DIR:-/nonexistent}/mesa-26.2.1-src/mesa-26.2.1
fi
if [ ! -f "$M/src/broadcom/compiler/qpu_validate.c" ] ||
  ! grep -q '^26\.2\.1' "$M/VERSION" 2>/dev/null; then
  echo "$name: SKIPPED: no Mesa 26.2.1 tree (set MESA_V3D_TREE, tools/v3d/README.md)"
  exit 0
fi

out=build/v3d71/$name
mkdir -p "$out" || exit 1
inc=(-I"$M/include" -I"$M/src" -I"$M/src/broadcom")
if [ -d "$M/build-v3d" ]; then
  for d in "$M/build-v3d/include" "$M/build-v3d/src"; do [ -d "$d" ] && inc+=(-I"$d"); done
fi
if ! cc -O1 -std=gnu11 -w -DNDEBUG -DMESA_DEBUG=1 -DHAVE_ENDIAN_H -D_GNU_SOURCE \
  "${inc[@]}" tools/v3d/tri_validate71.c \
  "$M/src/broadcom/qpu/qpu_pack.c" "$M/src/broadcom/qpu/qpu_instr.c" \
  "$M/src/broadcom/qpu/qpu_disasm.c" -lm -o "$out/tri_validate71" 2>"$out/cc.log"; then
  cat "$out/cc.log"
  echo "$name: the build failed"
  echo "$name: 5 programs, 5 failed"
  exit 1
fi

d=tools/v3d/tri-shaders71
failed=0
mesa_failed=0
run() { # json stage label
  "$out/tri_validate71" "$d/$1" "$2" "$3" || { failed=$((failed + 1)); [ "${3#mesa}" != "$3" ] && mesa_failed=1; }
}
run mesa_passthrough_interp_color.json fs "mesa fs"
run mesa_passthrough_interp_color.json vs "mesa vs"
run mesa_passthrough_interp_color.json cs "mesa cs"
run ours_passthrough_interp_color.json fs "ours fs"
run ours_passthrough_interp_color.json vs "ours vs"
echo "$name: 5 programs, $failed failed"
[ "$mesa_failed" -eq 0 ]
