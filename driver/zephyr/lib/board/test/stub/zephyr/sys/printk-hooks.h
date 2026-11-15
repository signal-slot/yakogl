/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host stub: the printk character hook board_rpi5.c chains (rpi5_screen_mock.c
 * keeps the hook and sends printk's characters through it). */
#ifndef STUB_ZEPHYR_PRINTK_HOOKS_H
#define STUB_ZEPHYR_PRINTK_HOOKS_H
typedef int (*printk_hook_fn_t)(int c);
void __printk_hook_install(printk_hook_fn_t fn);
printk_hook_fn_t __printk_get_hook(void);
#endif
