/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host shim for tools/v3d/hw_mock_test.c: register access goes to the mock. */
#ifndef HOSTSHIM_SYS_IO_H
#define HOSTSHIM_SYS_IO_H
#include <stdint.h>
typedef uintptr_t mm_reg_t;
uint32_t mock_read32(mm_reg_t addr);
void mock_write32(uint32_t val, mm_reg_t addr);
#define sys_read32(a)     mock_read32(a)
#define sys_write32(v, a) mock_write32((v), (a))
#endif
