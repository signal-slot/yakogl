#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# cl_golden: driver/src/v3d/v3d_cl.h's checked packers built for V3D 7.1
# (-DV3D_CL_VER=71, over the committed driver/src/v3d/generated/
# v3d_packet_v71_pack.h) against the packers of Mesa's own build
# (cle/v3d_packet_v71_pack.h), for every packet and record v3d_cl_packets.h
# lists at 7.1: the clear frame's (docs/notes/v3d71-clear-frame.md,
# tools/v3d/shim71/clear.clif) and every case of the gate's 4.2 row that
# exists at 7.1. The program is tools/v3d/cl_golden.c with cl_golden_mesa.c,
# the same source as the 4.2 row of tools/check.sh builds: 20,002 value sets
# a packet, the refusals, v3d_cl_emit. The last line is
# "cl_golden: N of N equal, 0 failed" when every case passed and the number
# run is the number planned.
# Reference: tools/v3d/mesa_ref.sh (build/v3d-check/mesa), or MESA_V3D_TREE with
# build-v3d as the v3d rows of tools/check.sh. Convention:
# tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=cl_golden
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1

tree=${MESA_V3D_TREE:-}
mt=third_party/mesa-26.2.1
ref=build/v3d-check/mesa
inc=(-D_GNU_SOURCE -DHAVE_ENDIAN_H -DV3D_CL_VER=71)
if [ -z "$tree" ]; then
  tools/v3d/mesa_ref.sh >"$dir/mesa_ref.log" 2>&1 || {
    tail -20 "$dir/mesa_ref.log"
    exit 1
  }
  inc+=(-I"$mt/src" -I"$mt/include" -I"$mt/src/broadcom" -I"$ref/gen")
elif [ -d "$tree/build-v3d" ]; then
  mb=$tree/build-v3d
  inc+=(-I"$tree/src" -I"$tree/include" -I"$tree/src/broadcom" -I"$mb/src/broadcom" -I"$mb/src")
else
  echo "MESA_V3D_TREE='$tree' has no build-v3d (unset it for the vendored reference build)"
  echo "gap: MESA_V3D_TREE"
  exit 77
fi
# Ours after Mesa's, as the 4.2 row orders them: v3d_cl.h finds the committed
# header next to itself; "cle/v3d_packet_v71_pack.h" is only in Mesa's build.
inc+=(-Idriver/src/v3d -Idriver/src/v3d/generated -I"$mt/src/broadcom" -I"$mt/src" -I"$mt/include")

cc -O1 -std=c11 "${inc[@]}" tools/v3d/cl_golden.c tools/v3d/cl_golden_mesa.c -lm \
  -o "$dir/cl_golden" >"$dir/build.log" 2>&1 || {
  tail -30 "$dir/build.log"
  exit 1
}

out=$("$dir/cl_golden" 2>&1)
status=$?
printf '%s\n' "$out" | grep -E 'FAIL|packets and records' | head -20
line=$(printf '%s\n' "$out" | grep -E '^cl_golden: [0-9]+ cases run \(planned [0-9]+\), [0-9]+ failed$' | tail -n 1)
if [ "$status" -ne 0 ] || [ -z "$line" ]; then
  echo "cl_golden: exit $status, ${line:-no result line}"
  exit 1
fi
run=$(printf '%s\n' "$line" | sed -E 's/^cl_golden: ([0-9]+) cases run \(planned ([0-9]+)\), ([0-9]+) failed$/\1/')
planned=$(printf '%s\n' "$line" | sed -E 's/^cl_golden: ([0-9]+) cases run \(planned ([0-9]+)\), ([0-9]+) failed$/\2/')
failed=$(printf '%s\n' "$line" | sed -E 's/^cl_golden: ([0-9]+) cases run \(planned ([0-9]+)\), ([0-9]+) failed$/\3/')
if [ "$run" != "$planned" ] || [ "$failed" != 0 ]; then
  echo "cl_golden: $((run - failed)) of $planned equal, $failed failed"
  exit 1
fi
echo "cl_golden: $run of $planned equal, 0 failed"
