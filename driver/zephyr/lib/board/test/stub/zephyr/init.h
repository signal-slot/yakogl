/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host stub: SYS_INIT records the hook and its level; the test runs them in
 * Zephyr's level order. */
#ifndef STUB_ZEPHYR_INIT_H
#define STUB_ZEPHYR_INIT_H
enum { PRE_KERNEL_1 = 0, POST_KERNEL = 1 };
void stub_sys_init_add(int (*fn)(void), int level);
#define SYS_INIT(fn, level, prio)                                              \
	static void __attribute__((constructor)) stub_reg_##fn(void)           \
	{                                                                      \
		stub_sys_init_add(fn, level);                                  \
	}
#endif
