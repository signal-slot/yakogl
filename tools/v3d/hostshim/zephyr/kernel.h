/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host shim for tools/v3d/hw_mock_test.c: time is the mock's. */
#ifndef HOSTSHIM_KERNEL_H
#define HOSTSHIM_KERNEL_H
#include <stdint.h>
void mock_busy_wait(uint32_t us);
#define k_busy_wait(us) mock_busy_wait(us)
#endif
