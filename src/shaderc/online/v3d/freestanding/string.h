/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* The C library declarations Mesa's qpu_pack.c / qpu_instr.c and
 * qpu_mesa.c include, for a target with no C library headers
 * (target_os = "none": Zephyr and bare metal). build.rs puts this directory
 * first on those targets only; the definitions are the platform's (Zephyr's
 * libc, or the bare-metal application's, baremetal.mk). */
#ifndef YAKOGL_FREESTANDING_STRING_H
#define YAKOGL_FREESTANDING_STRING_H
#include <stddef.h>
void *memcpy(void *restrict dst, const void *restrict src, size_t n);
void *memmove(void *dst, const void *src, size_t n);
void *memset(void *dst, int c, size_t n);
int memcmp(const void *a, const void *b, size_t n);
size_t strlen(const char *s);
#endif
