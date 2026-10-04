/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Boot trace: one line per POST_KERNEL priority band, so a driver that
 * hangs or resets the board during init can be located from the console
 * alone (there is no debugger on the lab board). Off in normal builds.
 */
#include <zephyr/init.h>
#include <zephyr/sys/printk.h>

#define TRACE(prio)                                                                                \
	static int boot_trace_##prio(void)                                                         \
	{                                                                                          \
		printk("boot: POST_KERNEL %d\n", prio);                                            \
		return 0;                                                                          \
	}                                                                                          \
	SYS_INIT(boot_trace_##prio, POST_KERNEL, prio)

TRACE(1);   /* after the RTI feeder */
TRACE(39);  /* before the secure proxy mailbox (40) */
TRACE(44);  /* before TISCI (45) */
TRACE(46);  /* after TISCI power-on (46) */
TRACE(48);  /* after GPIO (47) */
TRACE(51);  /* after I2C (50) */
TRACE(69);  /* before DSI (70) */
TRACE(71);
TRACE(79);  /* before DSS (80) */
TRACE(81);
TRACE(89);  /* before the GPU (90) */
TRACE(91);
TRACE(94);  /* before the panel bridge (95) */
TRACE(96);
TRACE(99);
