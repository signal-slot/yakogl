/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#ifndef YK_STUB_INIT_H
#define YK_STUB_INIT_H
/* the harness calls the registered init function itself */
#define SYS_INIT(fn, level, prio) int (*yk_sys_init)(void) = fn
#endif
