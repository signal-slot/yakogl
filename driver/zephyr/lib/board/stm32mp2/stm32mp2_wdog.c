/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * OP-TEE's watchdog on the STM32MP257F-DK's A35 (CONFIG_YAKOGL_STM32MP2_WDOG;
 * docs/notes/pre-study-stm32mp257f-dk.md, section 4): the arm,smc-wdt
 * service, reached by SMC through Zephyr's arm_smccc_smc. The protocol is
 * driver/rust/src/soc/stm32mp2/smc_wdt.rs's, call for call
 * (stm32mp2_wdog.h); test/stm32mp2_wdog_mock.c pins every call.
 *
 * BL2 starts the watchdog and OP-TEE serves it, so it is already running when
 * U-Boot hands over (the note, r-optee-tfa); this file arms it again with the
 * timeout Linux uses, so an image that hangs, also with interrupts locked or
 * with a PSCI that returns, gives the board back by itself.
 *
 * Self-contained: its own init hooks, in nanopc_t6.c's order, beside the
 * core's (stm32mp2.c "Order"):
 *   PRE_KERNEL_1 0: arm (SET_TIMEOUT 32, ENABLE 1, PET), stopping at the
 *     first refusal. Nothing printed yet: the console comes at 60.
 *   PRE_KERNEL_1 99: the marker line tools/board.sh requires in an image
 *     ("gc_wdog_arm: armed", as on the i.MX8MP), or, if arming was refused,
 *     what refused and PSCI SYSTEM_RESET at once (mp2_reset_to_linux): an
 *     image without its watchdog must not run.
 *   POST_KERNEL 0: a 1 s timer PETs the watchdog until the core's lifetime
 *     (mp2_lifetime_ms), then stops: from there the core's lifetime reset,
 *     or the watchdog within its timeout, ends the run. The timer stops with
 *     a hang that locks interrupts, and the watchdog resets the SoC.
 * Never ENABLE 0 (the note: a client never sends it), and no INIT: SmcWdt::arm
 * sends none, though the note's reading of OP-TEE has a client call INIT
 * first; the return trial's console shows whether arming is accepted
 * without it. Nothing but the SMC is touched.
 */
#include <stddef.h>
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/arch/arm64/arm-smccc.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>

#include "stm32mp2.h"
#include "stm32mp2_wdog.h"

static struct mp2_wdog_state s_wdog = {
	.arm_rc = MP2_SMCWDT_BAD_TIMEOUT,
};

const struct mp2_wdog_state *mp2_wdog_state_get(void)
{
	return &s_wdog;
}

int32_t mp2_smcwdt_cmd(uint32_t fid, uint64_t cmd, uint64_t arg, uint64_t *value)
{
	struct arm_smccc_res res;

	arm_smccc_smc((unsigned long)fid, (unsigned long)cmd, (unsigned long)arg, 0, 0, 0, 0, 0,
		      &res);
	if (value != NULL) {
		*value = res.a1;
	}
	/* The code is 32 bits wide; the upper half is not part of it. */
	return (int32_t)(uint32_t)res.a0;
}

int32_t mp2_smcwdt_arm(uint32_t fid, uint32_t timeout_s, uint64_t *refused_cmd)
{
	static const uint64_t cmd[] = {MP2_SMCWDT_CMD_SET_TIMEOUT, MP2_SMCWDT_CMD_ENABLE,
				       MP2_SMCWDT_CMD_PET};
	const uint64_t arg[] = {timeout_s, 1, 0};

	if (timeout_s == 0) {
		return MP2_SMCWDT_BAD_TIMEOUT;
	}
	for (size_t i = 0; i < ARRAY_SIZE(cmd); i++) {
		int32_t rc = mp2_smcwdt_cmd(fid, cmd[i], arg[i], NULL);

		if (rc != 0) {
			if (refused_cmd != NULL) {
				*refused_cmd = cmd[i];
			}
			return rc;
		}
	}
	return 0;
}

int32_t mp2_smcwdt_ping(uint32_t fid)
{
	return mp2_smcwdt_cmd(fid, MP2_SMCWDT_CMD_PET, 0, NULL);
}

int32_t mp2_smcwdt_time_left(uint32_t fid, uint64_t *s)
{
	return mp2_smcwdt_cmd(fid, MP2_SMCWDT_CMD_GET_TIMELEFT, 0, s);
}

static const char *cmd_name(uint64_t cmd)
{
	switch (cmd) {
	case MP2_SMCWDT_CMD_SET_TIMEOUT:
		return "SET_TIMEOUT";
	case MP2_SMCWDT_CMD_ENABLE:
		return "ENABLE";
	case MP2_SMCWDT_CMD_PET:
		return "PET";
	default:
		return "?";
	}
}

static int mp2_smcwdt_boot_arm(void)
{
	s_wdog.timeout_s = MP2_SMCWDT_TIMEOUT_S;
	s_wdog.arm_rc = mp2_smcwdt_arm(MP2_SMCWDT_FUNC_ID, MP2_SMCWDT_TIMEOUT_S, &s_wdog.refused_cmd);
	if (s_wdog.arm_rc == 0) {
		mp2_note_wdog_armed(MP2_SMCWDT_TIMEOUT_S);
	}
	return 0;
}
SYS_INIT(mp2_smcwdt_boot_arm, PRE_KERNEL_1, 0);

static int mp2_smcwdt_boot_report(void)
{
	if (s_wdog.arm_rc != 0) {
		printk("mp2: OP-TEE watchdog arming FAILED: %s answered %d (fid 0x%08x): "
		       "resetting now\n",
		       cmd_name(s_wdog.refused_cmd), (int)s_wdog.arm_rc,
		       (unsigned int)MP2_SMCWDT_FUNC_ID);
		mp2_reset_to_linux("watchdog refused");
	}
	printk("gc_wdog_arm: armed OP-TEE arm,smc-wdt fid 0x%08x, timeout %u s, PET every %u ms "
	       "until %u ms of uptime\n",
	       (unsigned int)MP2_SMCWDT_FUNC_ID, (unsigned int)s_wdog.timeout_s,
	       (unsigned int)MP2_SMCWDT_FEED_MS, (unsigned int)mp2_lifetime_ms());
	return 0;
}
SYS_INIT(mp2_smcwdt_boot_report, PRE_KERNEL_1, 99);

static void feed(struct k_timer *t)
{
	int32_t rc;

	if (k_uptime_get() >= (int64_t)mp2_lifetime_ms()) {
		k_timer_stop(t);
		s_wdog.feeding = false;
		return;
	}
	rc = mp2_smcwdt_ping(MP2_SMCWDT_FUNC_ID);
	s_wdog.feeds++;
	if (rc != 0) {
		if (s_wdog.pet_failures++ == 0) {
			s_wdog.first_pet_rc = rc;
			printk("mp2: OP-TEE watchdog PET answered %d\n", (int)rc);
		}
	}
}
K_TIMER_DEFINE(mp2_smcwdt_feeder, feed, NULL);

static int mp2_smcwdt_feed_start(void)
{
	s_wdog.feeding = true;
	k_timer_start(&mp2_smcwdt_feeder, K_MSEC(MP2_SMCWDT_FEED_MS), K_MSEC(MP2_SMCWDT_FEED_MS));
	return 0;
}
SYS_INIT(mp2_smcwdt_feed_start, POST_KERNEL, 0);
