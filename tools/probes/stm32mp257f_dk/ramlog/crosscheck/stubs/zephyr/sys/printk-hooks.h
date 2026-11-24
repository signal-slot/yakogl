/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#ifndef YK_STUB_PRINTK_HOOKS_H
#define YK_STUB_PRINTK_HOOKS_H
typedef int (*printk_hook_fn_t)(int c);
/* defined by the harness */
printk_hook_fn_t __printk_get_hook(void);
void __printk_hook_install(printk_hook_fn_t fn);
#endif
