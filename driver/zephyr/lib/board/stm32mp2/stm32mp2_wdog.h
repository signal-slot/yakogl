/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * stm32mp2_wdog.h -- OP-TEE's watchdog on the STM32MP257F-DK's A35
 * (stm32mp2_wdog.c, CONFIG_YAKOGL_STM32MP2_WDOG).
 *
 * The arm,smc-wdt protocol, as driver/rust/src/soc/stm32mp2/smc_wdt.rs has
 * it (Linux's arm_smc_wdt command set): x0 the function id, x1 the command,
 * x2 its argument; the answer is a signed 32-bit code in x0 and a value in
 * x1. The commands and codes below are smc_wdt.rs's CMD_* and RET_*.
 */
#ifndef YAKOGL_STM32MP2_WDOG_H
#define YAKOGL_STM32MP2_WDOG_H

#include <stdbool.h>
#include <stdint.h>

/* The node's arm,smc-id in ST's tree (stm32mp251.dtsi:78-82; the DK enables
 * it with timeout-sec = <32>, stm32mp257f-dk.dts:216-219;
 * docs/notes/pre-study-stm32mp257f-dk.md). */
#define MP2_SMCWDT_FUNC_ID 0xbc000000u
/* Linux's timeout on the board (#1281). */
#define MP2_SMCWDT_TIMEOUT_S 32u
/* The feeder's period. */
#define MP2_SMCWDT_FEED_MS 1000u

#define MP2_SMCWDT_CMD_INIT         0u
#define MP2_SMCWDT_CMD_SET_TIMEOUT  1u /* x2: the timeout in seconds */
#define MP2_SMCWDT_CMD_ENABLE       2u /* x2: 1 enable, 0 disable */
#define MP2_SMCWDT_CMD_PET          3u
#define MP2_SMCWDT_CMD_GET_TIMELEFT 4u

#define MP2_SMCWDT_RET_NOT_SUPPORTED  (-1)
#define MP2_SMCWDT_RET_INVALID_PARAMS (-2)
#define MP2_SMCWDT_RET_DENIED         (-3)
/* mp2_smcwdt_arm with a timeout of 0 (smc_wdt.rs Error::BadTimeout): no call
 * made. Not a code the service answers (its codes are -1 to -3). */
#define MP2_SMCWDT_BAD_TIMEOUT INT32_MIN

/* One command: the answer's code (0 success), its x1 in *value if value is
 * not NULL. */
int32_t mp2_smcwdt_cmd(uint32_t fid, uint64_t cmd, uint64_t arg, uint64_t *value);

/* SET_TIMEOUT, ENABLE 1, PET, in this order (SmcWdt::arm), stopping at the
 * first refusal: 0, the refusing command's code (and the command in
 * *refused_cmd if not NULL), or MP2_SMCWDT_BAD_TIMEOUT for timeout_s 0, with
 * no call made. */
int32_t mp2_smcwdt_arm(uint32_t fid, uint32_t timeout_s, uint64_t *refused_cmd);

/* PET. Success does not say the watchdog runs: OP-TEE answers PET with
 * success when it is disabled too; mp2_smcwdt_time_left tells. */
int32_t mp2_smcwdt_ping(uint32_t fid);

/* GET_TIMELEFT: seconds left before the reset in *s. */
int32_t mp2_smcwdt_time_left(uint32_t fid, uint64_t *s);

/* What the boot hooks did. */
struct mp2_wdog_state {
	int32_t arm_rc;       /* mp2_smcwdt_arm's result at PRE_KERNEL_1 0 */
	uint64_t refused_cmd; /* the command that refused, when arm_rc != 0 */
	uint32_t timeout_s;   /* the timeout armed */
	uint32_t feeds;       /* PETs the feeder sent */
	uint32_t pet_failures;
	int32_t first_pet_rc; /* the first refused PET's code, 0 if none */
	bool feeding;         /* the feeder runs (it stops at the lifetime) */
};
const struct mp2_wdog_state *mp2_wdog_state_get(void);

#endif /* YAKOGL_STM32MP2_WDOG_H */
