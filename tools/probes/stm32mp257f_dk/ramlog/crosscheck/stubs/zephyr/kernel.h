/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* host stub: only what ramlog.c takes from <zephyr/kernel.h> */
#ifndef YK_STUB_KERNEL_H
#define YK_STUB_KERNEL_H
#include <stdint.h>
#include <stddef.h>
#define BUILD_ASSERT(c, msg) _Static_assert(c, msg)
typedef uintptr_t mm_reg_t;
#define K_MEM_ARM_NORMAL_NC 1u
/* defined by the harness: points *va at the host buffer */
void device_map(mm_reg_t *va, uintptr_t phys, size_t size, uint32_t flags);
#endif
