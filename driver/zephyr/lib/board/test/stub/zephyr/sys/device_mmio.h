/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host stub: device_map() hands out the test's mock register block and
 * records the physical address and size asked for; sys_read32/sys_write32
 * are the mock's (rpi_wdog_mock.c). */
#ifndef STUB_ZEPHYR_DEVICE_MMIO_H
#define STUB_ZEPHYR_DEVICE_MMIO_H
#include <stdint.h>
typedef uintptr_t mm_reg_t;
#define K_MEM_CACHE_NONE 2u
void device_map(mm_reg_t *virt, unsigned long long phys, unsigned long size, unsigned int flags);
uint32_t sys_read32(mm_reg_t addr);
void sys_write32(uint32_t val, mm_reg_t addr);
#endif
