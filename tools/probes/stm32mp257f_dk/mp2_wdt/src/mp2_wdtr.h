/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * mp2_wdtr.h -- the C ABI of driver/rust/src/soc/stm32mp2/smc_wdt_refresh.rs
 * (yakogl-drivers, feature stm32mp2, aarch64): the opt-in refresh of
 * OP-TEE's arm,smc-wdt. Its host test abi_probe_header_has_the_same_numbers
 * pins the numbers below.
 *
 * yd_mp2_wdtr_init: INIT, then SET_TIMEOUT timeout_s, ENABLE 1, PET,
 * stopping at the first refusal. 0; -EIO when the firmware refused (the
 * command and its code in *out); -EINVAL for a timeout under
 * MP2_WDTR_MIN_TIMEOUT_S or func_id 0 (no call made), or a timeout outside
 * INIT's bounds (only INIT sent; the bounds in *out).
 * yd_mp2_wdtr_pet: PET; yd_mp2_wdtr_time_left: GET_TIMELEFT, the seconds in
 * out->left_s (-EIO with code -8 while the watchdog is stopped). Both are
 * -ENODEV with no call made before a successful init.
 * There is no ENABLE 0: OP-TEE's stop makes a hang never reset. out may be
 * NULL.
 */
#ifndef MP2_WDTR_H
#define MP2_WDTR_H

#include <stddef.h>
#include <stdint.h>

/* The DK's arm,smc-id (stm32mp251.dtsi:78-82). */
#define MP2_WDTR_FUNC_ID_DK 0xbc000000u
/* OP-TEE's early interrupt (5 s before the timeout) needs 1 s more. */
#define MP2_WDTR_MIN_TIMEOUT_S 6u
/* OP-TEE panics this long before the timeout when nothing pets. */
#define MP2_WDTR_EARLY_S 5u
#define YD_MP2_WDTR_NO_CMD 0xffffffffu /* cmd when no call was made */

struct yd_mp2_wdtr {
	uint64_t min_s;  /* INIT's bounds (init) */
	uint64_t max_s;
	uint64_t left_s; /* GET_TIMELEFT's seconds (time_left) */
	uint32_t cmd;    /* the last command sent (on -EIO the refusing one) */
	int32_t code;    /* its code: 0 or the firmware's refusal */
};

_Static_assert(sizeof(struct yd_mp2_wdtr) == 32, "smc_wdt_refresh.rs YdMp2Wdtr");
_Static_assert(offsetof(struct yd_mp2_wdtr, left_s) == 16, "smc_wdt_refresh.rs YdMp2Wdtr");
_Static_assert(offsetof(struct yd_mp2_wdtr, cmd) == 24, "smc_wdt_refresh.rs YdMp2Wdtr");
_Static_assert(offsetof(struct yd_mp2_wdtr, code) == 28, "smc_wdt_refresh.rs YdMp2Wdtr");

int yd_mp2_wdtr_init(uint32_t func_id, uint32_t timeout_s, struct yd_mp2_wdtr *out);
int yd_mp2_wdtr_pet(struct yd_mp2_wdtr *out);
int yd_mp2_wdtr_time_left(struct yd_mp2_wdtr *out);

#endif /* MP2_WDTR_H */
