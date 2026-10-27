/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host shim for tools/v3d/hw_mock_test.c: log lines go to stdout. */
#ifndef HOSTSHIM_LOG_H
#define HOSTSHIM_LOG_H
#include <stdio.h>
#define LOG_MODULE_REGISTER(...)
#define LOG_LEVEL_INF 3
#define LOG_INF(fmt, ...) printf("  inf: " fmt "\n", ##__VA_ARGS__)
#define LOG_ERR(fmt, ...) printf("  err: " fmt "\n", ##__VA_ARGS__)
#endif
