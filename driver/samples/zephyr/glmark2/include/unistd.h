/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * unistd.h for glmark2: Zephyr's <unistd.h> plus isatty(), which Zephyr does
 * not have with picolibc (only lib/libc/newlib/libc-hooks.c defines one).
 * glmark2's libmatrix/log.cc asks isatty(fileno(stdout)) to decide whether to
 * colour its lines; src/platform.c answers "not a terminal" (the console is a
 * UART log, read by tools, not a terminal).
 */
#ifndef GLMARK2_UNISTD_H
#define GLMARK2_UNISTD_H

#include_next <unistd.h>

#ifdef __cplusplus
extern "C" {
#endif

int isatty(int fd);

#ifdef __cplusplus
}
#endif

#endif /* GLMARK2_UNISTD_H */
