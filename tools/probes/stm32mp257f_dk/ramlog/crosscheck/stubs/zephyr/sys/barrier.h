/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#ifndef YK_STUB_BARRIER_H
#define YK_STUB_BARRIER_H
/* defined by the harness (it can tear a write between data and head) */
void barrier_dmem_fence_full(void);
#endif
