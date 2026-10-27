/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host shim for tools/pvr/rgxstart/record_main.c: time is the recorder's. */
#ifndef HOSTSHIM_KERNEL_H
#define HOSTSHIM_KERNEL_H
#include <stdint.h>
void rec_busy_wait(uint32_t us);
#define k_busy_wait(us) rec_busy_wait(us)
#endif
