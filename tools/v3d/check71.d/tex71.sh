#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# tex71: driver/src/v3d/v3d_tex71.c (the V3D 7.1 TEXTURE_SHADER_STATE,
# SAMPLER_STATE and TMU parameter 2; roadmap P10 T2) against Mesa 26.2.1's
# 7.1 packing of the same inputs, with tools/v3d/tex71_check.c:
#
#  - the bytes: Mesa's own generated 7.1 packers (cle/v3d_packet_v71_pack.h
#    of the reference build) and Mesa's v3d_translate_pipe_swizzle, linked
#    from it;
#  - the values: the reference build cannot compile Mesa's Gallium/Vulkan
#    driver code that fills the records (v3dx_state.c, v3dvx_image.c,
#    v3d_resource.c need a pipe context), so tex71_check.c transcribes those
#    rules line for line with their file:line, and also compares two
#    records with words built straight from the 7.1 packet XML's bit
#    positions (third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml:
#    1633-1707), without any generated packer.
#
# Cases: RGBA8 with and without sRGB, mip chains and a base level, cubes, a
# 2D array view, a 3D texture, R/B swap and reverse (7.1.5+), BGRA by
# swizzle, LT, UB-linear and UIF/XOR level 0s with ub_pad, R8, RGBA16F,
# RGBA32F; every sampler of the filter x wrap grid (nearest and linear;
# clamp, repeat, mirror, border, mirror-once) at 8 LOD/bias sets, the
# compare functions, anisotropy and the border variants; TMU P0/P1
# (v3d_tex.c's, unchanged at 7.1) and P2; refusals and negative controls.
# The last line is "tex71: N of N checks, 0 failed" (exit 0).
#
# Built -std=gnu11 -Wall -Wextra -Werror (Mesa's headers as system headers)
# into build/v3d71/tex71/. Reference: tools/v3d/mesa_ref.sh
# (build/v3d-check/mesa), or MESA_V3D_TREE with build-v3d as the v3d rows
# of tools/check.sh. No C compiler: exit 77, "gap: cc". Convention:
# tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=tex71
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1

if ! command -v cc >/dev/null 2>&1; then
  echo "tex71: no C compiler (cc) on PATH"
  echo "gap: cc"
  exit 77
fi

tree=${MESA_V3D_TREE:-}
mt=third_party/mesa-26.2.1
ref=build/v3d-check/mesa
if [ -z "$tree" ]; then
  tools/v3d/mesa_ref.sh >"$dir/mesa_ref.log" 2>&1 || {
    tail -20 "$dir/mesa_ref.log"
    exit 1
  }
  minc=(-isystem "$mt/src" -isystem "$mt/include" -isystem "$mt/src/broadcom"
    -isystem "$mt/src/gallium/include" -isystem "$ref/gen" -isystem driver/src/v3d/generated)
  libs=("$ref/libmesa_ref.a" -lexpat -lz -lm -lpthread)
elif [ -d "$tree/build-v3d" ]; then
  mb=$tree/build-v3d
  minc=(-isystem "$tree/src" -isystem "$tree/include" -isystem "$tree/src/broadcom"
    -isystem "$tree/src/gallium/include" -isystem "$mb/src/broadcom" -isystem "$mb/src")
  libs=("$mb/src/broadcom/libbroadcom_v3d.a" "$mb/src/broadcom/cle/libbroadcom_cle.a"
    "$mb/src/util/libmesa_util.a" -lexpat -lz -lzstd -lm -lpthread)
else
  echo "MESA_V3D_TREE='$tree' has no build-v3d (unset it for the vendored reference build)"
  echo "gap: MESA_V3D_TREE"
  exit 77
fi

flags=(-std=gnu11 -O1 -Wall -Wextra -Werror -D_GNU_SOURCE -DHAVE_ENDIAN_H)
# Ours: the driver's headers, the committed generated 7.1 header (v3d_cl.h),
# Mesa's helpers it includes as system headers.
oinc=(-Idriver/src/v3d -Idriver/src/v3d/generated -isystem "$mt/src/broadcom"
  -isystem "$mt/src" -isystem "$mt/include")

build() { # build <object> <source> <flags...>
  local obj=$1 src=$2
  shift 2
  cc -c "${flags[@]}" "$@" "$src" -o "$dir/$obj" >>"$dir/build.log" 2>&1
}
: >"$dir/build.log"
ok=1
build mesa_side.o tools/v3d/tex71_check.c -DTEX71_MESA_SIDE "${minc[@]}" || ok=0
build check.o tools/v3d/tex71_check.c "${oinc[@]}" || ok=0
build v3d_tex71.o driver/src/v3d/v3d_tex71.c "${oinc[@]}" || ok=0
# v3d_tex.c (the layouts, v3d_tex_p0/p1) as the driver builds it: at 4.2
build v3d_tex.o driver/src/v3d/v3d_tex.c "${oinc[@]}" || ok=0
build v3d_tiling_mesa.o driver/src/v3d/v3d_tiling_mesa.c "${oinc[@]}" || ok=0
if [ "$ok" = 1 ]; then
  cc "$dir/check.o" "$dir/mesa_side.o" "$dir/v3d_tex71.o" "$dir/v3d_tex.o" \
    "$dir/v3d_tiling_mesa.o" "${libs[@]}" -o "$dir/tex71_check" >>"$dir/build.log" 2>&1 || ok=0
fi
if [ "$ok" != 1 ]; then
  tail -30 "$dir/build.log"
  echo "tex71: build failed"
  exit 1
fi

out=$("$dir/tex71_check" 2>&1)
status=$?
printf '%s\n' "$out" | grep -E '^(FAIL|  |tex71: .* compared)' | head -40
line=$(printf '%s\n' "$out" | grep -E '^tex71: [0-9]+ of [0-9]+ checks, [0-9]+ failed$' | tail -n 1)
if [ "$status" -ne 0 ] || [ -z "$line" ]; then
  echo "tex71: exit $status, ${line:-no result line}"
  exit 1
fi
pass=$(printf '%s\n' "$line" | sed -E 's/^tex71: ([0-9]+) of ([0-9]+) checks, ([0-9]+) failed$/\1/')
all=$(printf '%s\n' "$line" | sed -E 's/^tex71: ([0-9]+) of ([0-9]+) checks, ([0-9]+) failed$/\2/')
bad=$(printf '%s\n' "$line" | sed -E 's/^tex71: ([0-9]+) of ([0-9]+) checks, ([0-9]+) failed$/\3/')
if [ "$pass" != "$all" ] || [ "$bad" != 0 ]; then
  echo "$line"
  exit 1
fi
echo "$line"
