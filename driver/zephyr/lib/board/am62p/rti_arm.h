/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The arithmetic of arming the AM62P MAIN RTI0 watchdog (rti_arm.c), kept
 * free of Zephyr so tools/pvr/rti_arm_check.c tests it on the host.
 *
 * The RTI digital watchdog counts its functional clock down from
 * (DWDPRLD + 1) << 13 and resets the SoC (through the ESM) when it reaches
 * zero; DWDPRLD is 12 bits. So a timeout T at rate R needs
 * DWDPRLD = ((T * R) >> 13) - 1, which must fit 0..0xfff. The expiry is then
 * ((DWDPRLD + 1) << 13) / R, never longer than T.
 *
 * The rates: the parents of TISCI device 125 clock 0 as Linux's clock
 * debugfs on the board reported them (2026-09-29, read-only):
 * clk:125:1 25000000, clk:125:2 32768, clk:125:3 12500000, clk:125:4 32552.
 * Linux runs the RTI from parent 2 (32768 Hz); Zephyr's k3-am62-main.dtsi
 * gives main_rti0 clock-frequency 25000000. Only the 32 kHz parents can
 * hold 60 s in 12 bits; at 25 MHz the longest expiry is about 1.34 s.
 */
#ifndef YAKOGL_SAMPLE_RTI_ARM_H_
#define YAKOGL_SAMPLE_RTI_ARM_H_

#include <stdbool.h>
#include <stdint.h>

#define RTI_ARM_PRELOAD_SHIFT 13
#define RTI_ARM_PRELOAD_MAX   0xfffu

/* The parent clock rates of the RTI measured on the board (see above). */
#define RTI_ARM_RATE_32K    32768u
#define RTI_ARM_RATE_32K_RC 32552u
#define RTI_ARM_RATE_25M    25000000u
#define RTI_ARM_RATE_12M5   12500000u

/* True for a rate the board reported for this clock; anything else means
 * the clock tree is not what was measured, and arming refuses. */
static inline bool rti_arm_rate_known(uint64_t rate_hz)
{
	return rate_hz == RTI_ARM_RATE_32K || rate_hz == RTI_ARM_RATE_32K_RC ||
	       rate_hz == RTI_ARM_RATE_25M || rate_hz == RTI_ARM_RATE_12M5;
}

/* DWDPRLD for an expiry of at most `seconds` at `rate_hz`, or -1 when it
 * does not fit the 12-bit field (too long for the rate) or is zero. */
static inline int32_t rti_arm_preload(uint64_t rate_hz, uint32_t seconds)
{
	uint64_t units = ((uint64_t)seconds * rate_hz) >> RTI_ARM_PRELOAD_SHIFT;

	if (units == 0 || units - 1 > RTI_ARM_PRELOAD_MAX) {
		return -1;
	}
	return (int32_t)(units - 1);
}

/* The expiry in milliseconds that a preload gives at a rate (rounded down). */
static inline uint64_t rti_arm_expiry_ms(uint32_t preload, uint64_t rate_hz)
{
	return (((uint64_t)preload + 1) << RTI_ARM_PRELOAD_SHIFT) * 1000u / rate_hz;
}

#endif /* YAKOGL_SAMPLE_RTI_ARM_H_ */
