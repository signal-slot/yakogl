/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * The main-domain RTI watchdog a previous boot stage left running
 * (soc_a53_wdt.c, CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT).
 */
#ifndef SOC_AM62PX_A53_WDT_H
#define SOC_AM62PX_A53_WDT_H

#ifdef CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT
/* Stop feeding: the watchdog resets the SoC within one period (60 s).
 * Safe from a fatal-error handler; other CPUs keep running until then.
 */
void soc_am62px_inherited_wdt_release(void);
#else
static inline void soc_am62px_inherited_wdt_release(void)
{
}
#endif

#endif /* SOC_AM62PX_A53_WDT_H */
