/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host stub: k_mem_phys_addr is the test's (rpi_mbox_mock.c), which gives the
 * message buffer the physical address the case asks for. */
#ifndef STUB_ZEPHYR_KERNEL_INTERNAL_MM_H
#define STUB_ZEPHYR_KERNEL_INTERNAL_MM_H
#include <stdint.h>
uintptr_t k_mem_phys_addr(void *virt);
#endif
