/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Mesa's util/u_debug.h includes <unistd.h> for one thing: __normal_user(),
 * which calls geteuid(), getuid(), getegid() and getgid() (our builds never
 * reach it). With Zephyr's POSIX layer (CONFIG_POSIX_API, the dEQP images)
 * <unistd.h> is zephyr/posix/unistd.h, which has none of the four and pulls
 * in <zephyr/kernel.h>, whose ARRAY_SIZE() (strict) and likely()/unlikely()
 * replace Mesa's own and make v3d_tiling.c's ARRAY_SIZE(partial_boxes) a
 * compile error. So, in the two files that include Mesa's headers, that
 * header is marked as already included and the four are provided here. A
 * build without the POSIX layer (v3d_gl_test, the cluster) is unchanged: it
 * finds the C library's <unistd.h>, which has them.
 *
 * Include this first in a file that includes Mesa's util headers.
 */
#ifndef V3D_MESA_POSIX_H
#define V3D_MESA_POSIX_H

#if defined(__ZEPHYR__) && defined(CONFIG_POSIX_API)
#define ZEPHYR_INCLUDE_POSIX_UNISTD_H_
static inline unsigned int geteuid(void) { return 0; }
static inline unsigned int getuid(void) { return 0; }
static inline unsigned int getegid(void) { return 0; }
static inline unsigned int getgid(void) { return 0; }
#endif

#endif
