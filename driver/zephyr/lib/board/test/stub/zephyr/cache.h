/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host stub: the cache maintenance rpi_mbox.c does; the test records the
 * ranges (rpi_mbox_mock.c). */
#ifndef STUB_ZEPHYR_CACHE_H
#define STUB_ZEPHYR_CACHE_H
#include <stddef.h>
int sys_cache_data_flush_range(void *addr, size_t size);
int sys_cache_data_invd_range(void *addr, size_t size);
#endif
