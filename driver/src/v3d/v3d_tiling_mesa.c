/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Mesa 26.2.1's texel tiling, compiled as it is: this file only includes
 * third_party/mesa-26.2.1/src/broadcom/common/v3d_tiling.c, unmodified,
 * with what its build would give it and without the warnings our builds
 * turn on (Mesa's meson compiles it as gnu11, where void * arithmetic is
 * fine). v3d_tex.c calls its v3d_store_tiled_image, v3d_utile_width and
 * v3d_utile_height.
 *
 * It needs util/format/u_format_gen.h, which Mesa's build generates; the
 * committed copy is driver/src/v3d/generated/util/format/u_format_gen.h:
 *
 *   python3 third_party/mesa-26.2.1/src/util/format/u_format_table.py \
 *       third_party/mesa-26.2.1/src/util/format/u_format.yaml --enums
 *
 * (Mesa's command, src/util/format/meson.build), which tools/check.sh
 * regenerates and compares.
 */

/* A release build of Mesa: assert() compiled out, as the driver never
 * aborts on the board (and v3d_tex.c only passes the cpp values and boxes
 * v3d_tiling.c accepts). */
#ifndef NDEBUG
#define NDEBUG
#endif

/* The byte order for util/u_endian.h when the build has no <endian.h>
 * (Zephyr, bare metal): see v3d_cl.h. */
#if !defined(UTIL_ARCH_LITTLE_ENDIAN) && !defined(HAVE_ENDIAN_H)
#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
#define UTIL_ARCH_LITTLE_ENDIAN 1
#define UTIL_ARCH_BIG_ENDIAN 0
#elif defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
#define UTIL_ARCH_LITTLE_ENDIAN 0
#define UTIL_ARCH_BIG_ENDIAN 1
#endif
#endif

#include "v3d_mesa_posix.h"

#pragma GCC diagnostic ignored "-Wpointer-arith"
#pragma GCC diagnostic ignored "-Wunused-parameter"
#pragma GCC diagnostic ignored "-Wsign-compare"
#pragma GCC diagnostic ignored "-Wpedantic"

#include "common/v3d_tiling.c"
