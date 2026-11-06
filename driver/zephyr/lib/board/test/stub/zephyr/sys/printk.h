/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host stub: printk goes to the test's capture buffer. */
#ifndef STUB_ZEPHYR_PRINTK_H
#define STUB_ZEPHYR_PRINTK_H
void printk(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
#endif
