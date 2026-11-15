/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The probe's console lines. A thin printf wrapper: the final verdict format
 * belongs to tools/probes/rpi5_display/verdicts.sh, which a later step follows.
 */
#ifndef RPI5_DISPLAY_VERDICT_H
#define RPI5_DISPLAY_VERDICT_H

#include <zephyr/sys/printk.h>

/* One line: "<step>: <message>". */
#define VERDICT(step, ...)                                                    \
	do {                                                                  \
		printk("%s: ", (step));                                       \
		printk(__VA_ARGS__);                                          \
		printk("\n");                                                 \
	} while (0)

#endif /* RPI5_DISPLAY_VERDICT_H */
