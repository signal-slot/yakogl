#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mesa71_clear: tools/v3d/mesa71_clear.c packs the V3D 7.1 clear-only frame with
# Mesa's own 7.1 packers, reads it back with Mesa's clif_dump at version 71 and
# compares it, packet by packet (names and field values, BO addresses ignored),
# with tools/v3d/shim71/clear.clif, the CLIF Mesa's driver dumped for that clear.
# Exit 0 when decoder errors are 0 and mismatches are 0.
# Reference: tools/v3d/mesa_ref.sh (build/v3d-check/mesa), or MESA_V3D_TREE with
# build-v3d as the v3d rows of tools/check.sh. Convention:
# tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=mesa71_clear
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1

tree=${MESA_V3D_TREE:-}
mt=third_party/mesa-26.2.1
ref=build/v3d-check/mesa
inc=(-D_GNU_SOURCE -DHAVE_ENDIAN_H)
if [ -z "$tree" ]; then
  tools/v3d/mesa_ref.sh >"$dir/mesa_ref.log" 2>&1 || {
    tail -20 "$dir/mesa_ref.log"
    exit 1
  }
  inc+=(-I"$mt/src" -I"$mt/include" -I"$mt/src/broadcom" -I"$ref/gen")
  libs=("$ref/libmesa_ref.a" -lexpat -lz -lm -lpthread)
elif [ -d "$tree/build-v3d" ]; then
  mb=$tree/build-v3d
  inc+=(-I"$tree/src" -I"$tree/include" -I"$tree/src/broadcom" -I"$mb/src/broadcom" -I"$mb/src")
  libs=("$mb/src/broadcom/libbroadcom_v3d.a" "$mb/src/broadcom/cle/libbroadcom_cle.a"
    "$mb/src/broadcom/libv3d_neon.a" "$mb/src/util/libmesa_util.a" -lexpat -lz -lzstd -lm -lpthread)
else
  echo "MESA_V3D_TREE='$tree' has no build-v3d (unset it for the vendored reference build)"
  echo "gap: MESA_V3D_TREE"
  exit 77
fi

cc -O1 -std=gnu11 -Wall -Werror "${inc[@]}" tools/v3d/mesa71_clear.c "${libs[@]}" \
  -o "$dir/mesa71_clear" >"$dir/build.log" 2>&1 || {
  tail -30 "$dir/build.log"
  exit 1
}

TMPDIR=$dir "$dir/mesa71_clear" tools/v3d/shim71/clear.clif
