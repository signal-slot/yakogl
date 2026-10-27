#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Build and run tools/v3d/tex_check.c (driver/src/v3d/v3d_tex.c against Mesa
# 26.2.1). tools/check.sh runs this; so can a person, from anywhere:
#
#   tools/v3d/tex_check.sh
#
# It needs nothing but the sources vendored in third_party/mesa-26.2.1:
# tools/v3d/mesa_ref.sh builds what the check reads from Mesa (its format
# table) into build/v3d-check/mesa/. To use a built Mesa tree instead, set
#
#   MESA_V3D_TREE=<mesa-26.2.1 with build-v3d>   (tools/v3d/README.md,
#                                                 "Set up once")
#
# Output goes to build/v3d-check/ (gitignored).
#
# Exit status: tex_check's own (0 only when every check passed and the count
# run is the count planned); 1 for a build failure; 2 when MESA_V3D_TREE is
# set and is not a Mesa tree, so that the caller can report "not measured"
# rather than "failed".
set -u
cd "$(dirname "$0")/../.." || exit 1

O=build/v3d-check
mkdir -p "$O" || exit 1

if [ -z "${MESA_V3D_TREE:-}" ]; then
  tools/v3d/mesa_ref.sh || exit 1
  T=third_party/mesa-26.2.1
  R=$O/mesa
  # Our code with -Wall -Wextra -Werror; Mesa's headers as system headers.
  # The shim reads Mesa's format table, so it is built with Mesa's flags.
  cc -c -std=c11 -O1 -w -D_GNU_SOURCE -DHAVE_ENDIAN_H -DHAVE_PTHREAD -DHAVE_STRUCT_TIMESPEC \
    -DV3D_VERSION=42 -Itools/v3d/mesa-stub -I"$R/gen" -Idriver/src/v3d/generated \
    -I"$T/include" -I"$T/src" -I"$T/src/broadcom" -I"$T/src/broadcom/cle" \
    -I"$T/src/gallium/include" -I"$T/src/gallium/drivers/v3d" -Itools/v3d \
    tools/v3d/tex_check_mesa_fmt.c -o "$O/shim.o" || exit 1
  cc -O1 -std=c11 -Wall -Wextra -Werror -D_GNU_SOURCE -DHAVE_ENDIAN_H \
    -isystem "$T/src" -isystem "$T/include" -isystem "$T/src/broadcom" -isystem "$R/gen" \
    -Idriver/src/v3d -Idriver/src/v3d/generated -Itools/v3d tools/v3d/tex_check.c \
    driver/src/v3d/v3d_tex.c driver/src/v3d/v3d_tiling_mesa.c \
    "$O/shim.o" "$R/fmt.o" "$R/fmt_blocks.o" "$R/libmesa_ref.a" -lexpat -lz -lm -lpthread \
    -o "$O/tex_check" || exit 1
  exec "$O/tex_check"
fi

M=$MESA_V3D_TREE
B=$M/build-v3d
for f in "$B/src/broadcom/libv3d_neon.a" "$B/src/broadcom/libbroadcom_v3d.a" \
  "$B/src/util/libmesa_util.a" "$B/src/broadcom/cle/v3d_packet_v42_pack.h" \
  "$M/src/gallium/drivers/v3d/v3dx_format_table.c"; do
  if [ ! -e "$f" ]; then
    echo "tex_check.sh: no Mesa tree: $f is missing (MESA_V3D_TREE=$M)"
    exit 2
  fi
done

# Mesa's own sources (its format table, and the shim that reads it) are built
# with the flags Mesa's meson gives libv3d-v42 and without our warning flags:
# Mesa's headers are not ours to keep warning-free.
MESA_FLAGS=(-std=c11 -O1 -D_GNU_SOURCE -DHAVE_ENDIAN_H -DHAVE_PTHREAD
  -DHAVE_STRUCT_TIMESPEC -DV3D_VERSION=42
  -I"$B/src/gallium/drivers/v3d" -I"$M/src/gallium/drivers/v3d" -I"$B/src" -I"$M/src"
  -I"$M/include" -I"$M/src/gallium/include" -I"$B/src/gallium/auxiliary"
  -I"$M/src/gallium/auxiliary" -I"$B/src/broadcom" -I"$M/src/broadcom"
  -I"$B/src/broadcom/cle" -I"$M/src/broadcom/cle" -I"$M/src/gallium/drivers"
  -I"$B/src/compiler/nir" -I"$M/src/compiler/nir" -I"$B/src/compiler" -I"$M/src/compiler"
  -I"$B/src/util" -I"$B/src/util/format" -I/usr/include/libdrm)

cc -c "${MESA_FLAGS[@]}" "$M/src/gallium/drivers/v3d/v3dx_format_table.c" -o "$O/fmt.o" ||
  exit 1
cc -c "${MESA_FLAGS[@]}" -Itools/v3d tools/v3d/tex_check_mesa_fmt.c -o "$O/shim.o" || exit 1

# Our code with -Wall -Wextra -Werror; Mesa's headers as system headers.
cc -O1 -std=c11 -Wall -Wextra -Werror -D_GNU_SOURCE -DHAVE_ENDIAN_H \
  -isystem "$M/src" -isystem "$M/include" -isystem "$M/src/broadcom" \
  -isystem "$B/src/broadcom" -isystem "$B/src" \
  -Idriver/src/v3d -Itools/v3d tools/v3d/tex_check.c driver/src/v3d/v3d_tex.c \
  "$O/shim.o" "$O/fmt.o" \
  "$B/src/broadcom/libv3d_neon.a" "$B/src/broadcom/libbroadcom_v3d.a" \
  "$B/src/util/libmesa_util.a" -lm -lpthread -o "$O/tex_check" || exit 1

"$O/tex_check"
