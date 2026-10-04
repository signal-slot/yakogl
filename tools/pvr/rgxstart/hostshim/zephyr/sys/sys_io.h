/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host shim for tools/pvr/rgxstart/record_main.c: register access is the
 * recorder's (the same register file and trace format as
 * tools/pvr/rgxlayer_check.c). */
#ifndef HOSTSHIM_SYS_IO_H
#define HOSTSHIM_SYS_IO_H
#include <stdint.h>
typedef uintptr_t mm_reg_t;
uint32_t sys_read32(mm_reg_t addr);
void sys_write32(uint32_t data, mm_reg_t addr);
#endif
