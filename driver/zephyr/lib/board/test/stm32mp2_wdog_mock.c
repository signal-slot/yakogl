/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * stm32mp2_wdog_mock: driver/zephyr/lib/board/stm32mp2/stm32mp2_wdog.c,
 * unchanged, against a recording arm_smccc_smc. The file is #included so the
 * test reaches its statics (the hooks, the feeder timer); the Zephyr symbols
 * it needs are the stubs under stub/zephyr/, and the board layer core's
 * (stm32mp2.h) are defined here.
 *
 * Checked: the hooks and their levels and priorities (nanopc_t6.c's order),
 * the exact x0..x7 of every SMC for arming 32 s, a refusal at each of the
 * three steps (stop there, print what refused, PSCI reset at the report
 * hook), one feeder tick, the stop at the lifetime, a refused PET, and a
 * timeout of 0 (no call). The protocol's vectors are those of
 * driver/rust/src/soc/stm32mp2/smc_wdt.rs's tests (F = 0x8200_0001:
 * arm_sequence, arm_zero_timeout_makes_no_call, arm_stops_at_refusal,
 * code_is_32_bit, commands); the boot hooks use the board's function id,
 * 0xbc000000.
 *
 * Build and run (as tools/v3d/check71.d/wdog_mock.sh builds rpi_wdog_mock):
 *   cc -std=gnu11 -g -O1 -Wall -Wextra -Werror \
 *     -fsanitize=address,undefined -fno-sanitize-recover=undefined \
 *     -Idriver/zephyr/lib/board/test/stub -Idriver/zephyr/lib/board/stm32mp2 \
 *     driver/zephyr/lib/board/test/stm32mp2_wdog_mock.c -o stm32mp2_wdog_mock
 *
 * Prints "stm32mp2_wdog_mock: N of N checks, 0 failed"; exit 0 when none failed.
 */
#include <setjmp.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

/* The stub's SYS_INIT records the level only; this one keeps the priority. */
#include <zephyr/init.h>
#undef SYS_INIT
void mock_sys_init_add(int (*fn)(void), int level, int prio);
#define SYS_INIT(fn, level, prio)                                              \
	static void __attribute__((constructor)) mock_reg_##fn(void)           \
	{                                                                      \
		mock_sys_init_add(fn, level, prio);                            \
	}

#include "stm32mp2_wdog.c"

#define F    0x82000001ul /* smc_wdt.rs tests' F */
#define FID  0xbc000000ul /* the board's arm,smc-id */
#define LIFETIME_MS 20000u

/* ---- the mock ------------------------------------------------------------ */

struct call {
	unsigned long a[8];
};
static struct call calls[16];
static int ncalls;
static unsigned long replies[16][2]; /* a0, a1 of each answer; then 0, 0 */
static int nreplies, next_reply;

static int64_t now_ms;
static char out[2048];
static uint32_t noted_s;
static int noted_calls;
static jmp_buf reset_jmp;
static int resets;
static const char *reset_why;

static int checks, failed;
#define CHECK(cond, ...)                                                       \
	do {                                                                   \
		checks++;                                                      \
		if (!(cond)) {                                                 \
			failed++;                                              \
			printf("  FAIL line %d: ", __LINE__);                   \
			printf(__VA_ARGS__);                                   \
			printf("\n");                                          \
		}                                                              \
	} while (0)

void arm_smccc_smc(unsigned long a0, unsigned long a1, unsigned long a2, unsigned long a3,
		   unsigned long a4, unsigned long a5, unsigned long a6, unsigned long a7,
		   struct arm_smccc_res *res)
{
	if (ncalls < (int)ARRAY_SIZE(calls)) {
		calls[ncalls++] = (struct call){{a0, a1, a2, a3, a4, a5, a6, a7}};
	}
	memset(res, 0x5a, sizeof(*res)); /* a2..a7 are not read */
	if (next_reply < nreplies) {
		res->a0 = replies[next_reply][0];
		res->a1 = replies[next_reply][1];
		next_reply++;
	} else {
		res->a0 = 0;
		res->a1 = 0;
	}
}

int64_t k_uptime_get(void)
{
	return now_ms;
}

void printk(const char *fmt, ...)
{
	va_list ap;
	size_t n = strlen(out);

	va_start(ap, fmt);
	vsnprintf(out + n, sizeof(out) - n, fmt, ap);
	va_end(ap);
}

/* The board layer core's (stm32mp2.c). */
uint32_t mp2_lifetime_ms(void)
{
	return LIFETIME_MS;
}

void mp2_note_wdog_armed(uint32_t timeout_s)
{
	noted_s = timeout_s;
	noted_calls++;
}

FUNC_NORETURN void mp2_reset_to_linux(const char *why)
{
	resets++;
	reset_why = why;
	longjmp(reset_jmp, 1);
}

static struct {
	int (*fn)(void);
	int level, prio;
} hooks[8];
static int nhooks;

void mock_sys_init_add(int (*fn)(void), int level, int prio)
{
	hooks[nhooks].fn = fn;
	hooks[nhooks].level = level;
	hooks[nhooks].prio = prio;
	nhooks++;
}

/* Never called: stub/zephyr/init.h declares it, the SYS_INIT above replaces it. */
void stub_sys_init_add(int (*fn)(void), int level)
{
	(void)fn;
	(void)level;
}

/* ---- helpers ------------------------------------------------------------- */

static void reset_all(void)
{
	ncalls = 0;
	nreplies = 0;
	next_reply = 0;
	out[0] = '\0';
	noted_s = 0;
	noted_calls = 0;
	resets = 0;
	reset_why = NULL;
	memset(&s_wdog, 0, sizeof(s_wdog));
	s_wdog.arm_rc = MP2_SMCWDT_BAD_TIMEOUT;
	mp2_smcwdt_feeder.running = false;
}

static void reply(unsigned long a0, unsigned long a1)
{
	replies[nreplies][0] = a0;
	replies[nreplies][1] = a1;
	nreplies++;
}

static void expect_call(int i, unsigned long a0, unsigned long a1, unsigned long a2,
			unsigned long a3, const char *what)
{
	const unsigned long *a = i < ncalls ? calls[i].a : NULL;

	CHECK(a != NULL && a[0] == a0 && a[1] == a1 && a[2] == a2 && a[3] == a3 && a[4] == 0 &&
		      a[5] == 0 && a[6] == 0 && a[7] == 0,
	      "%s: call %d is [%#lx, %lu, %lu, %lu, %lu..%lu], expected [%#lx, %lu, %lu, %lu, 0..0]",
	      what, i, a ? a[0] : 0, a ? a[1] : 0, a ? a[2] : 0, a ? a[3] : 0, a ? a[4] : 0,
	      a ? a[7] : 0, a0, a1, a2, a3);
}

static int hook_index(int (*fn)(void))
{
	for (int i = 0; i < nhooks; i++) {
		if (hooks[i].fn == fn) {
			return i;
		}
	}
	return -1;
}

/* The boot as Zephyr runs it: PRE_KERNEL_1 0, 99, POST_KERNEL 0. Returns 1
 * when the report hook reset the board. */
static int boot(void)
{
	CHECK(mp2_smcwdt_boot_arm() == 0, "arm hook failed");
	if (setjmp(reset_jmp) != 0) {
		return 1;
	}
	CHECK(mp2_smcwdt_boot_report() == 0, "report hook failed");
	CHECK(mp2_smcwdt_feed_start() == 0, "feeder hook failed");
	return 0;
}

int main(void)
{
	/* 1. the hooks: nanopc_t6.c's order */
	int h;

	CHECK(nhooks == 3, "%d hooks registered, expected 3", nhooks);
	h = hook_index(mp2_smcwdt_boot_arm);
	CHECK(h >= 0 && hooks[h].level == PRE_KERNEL_1 && hooks[h].prio == 0,
	      "arm hook not at PRE_KERNEL_1 0");
	h = hook_index(mp2_smcwdt_boot_report);
	CHECK(h >= 0 && hooks[h].level == PRE_KERNEL_1 && hooks[h].prio == 99,
	      "report hook not at PRE_KERNEL_1 99");
	h = hook_index(mp2_smcwdt_feed_start);
	CHECK(h >= 0 && hooks[h].level == POST_KERNEL && hooks[h].prio == 0,
	      "feeder hook not at POST_KERNEL 0");

	/* 2. the protocol, smc_wdt.rs's tests' vectors */
	/* arm_sequence: arm(32) */
	reset_all();
	CHECK(mp2_smcwdt_arm(F, 32, NULL) == 0, "arm(32) refused");
	CHECK(ncalls == 3, "arm(32) made %d calls, expected 3", ncalls);
	expect_call(0, F, 1, 32, 0, "arm SET_TIMEOUT 32");
	expect_call(1, F, 2, 1, 0, "arm ENABLE 1");
	expect_call(2, F, 3, 0, 0, "arm PET");

	/* arm_zero_timeout_makes_no_call */
	reset_all();
	CHECK(mp2_smcwdt_arm(F, 0, NULL) == MP2_SMCWDT_BAD_TIMEOUT, "arm(0) not BadTimeout");
	CHECK(ncalls == 0, "arm(0) made %d calls", ncalls);

	/* arm_stops_at_refusal, at each of the three steps with each code; the
	 * answers are 32-bit codes in a 64-bit x0 (RET_DENIED as u32 as u64) */
	static const int32_t codes[] = {MP2_SMCWDT_RET_NOT_SUPPORTED, MP2_SMCWDT_RET_INVALID_PARAMS,
					MP2_SMCWDT_RET_DENIED};
	static const uint64_t step_cmd[] = {1, 2, 3};
	for (int step = 0; step < 3; step++) {
		for (unsigned int c = 0; c < ARRAY_SIZE(codes); c++) {
			uint64_t refused = 99;

			reset_all();
			for (int i = 0; i < step; i++) {
				reply(0, 0);
			}
			reply((unsigned long)(uint32_t)codes[c], 0);
			CHECK(mp2_smcwdt_arm(F, 32, &refused) == codes[c],
			      "step %d code %d not returned", step, codes[c]);
			CHECK(ncalls == step + 1, "step %d: %d calls, expected %d", step, ncalls,
			      step + 1);
			CHECK(refused == step_cmd[step], "step %d: refused cmd %llu", step,
			      (unsigned long long)refused);
		}
	}

	/* code_is_32_bit */
	reset_all();
	reply(~0ul, 0);
	CHECK(mp2_smcwdt_ping(F) == -1, "ping with x0 = u64::MAX not -1");
	expect_call(0, F, 3, 0, 0, "ping");
	reset_all();
	reply(0xffffffff00000000ul, 7);
	{
		uint64_t left = 0;

		CHECK(mp2_smcwdt_time_left(F, &left) == 0 && left == 7,
		      "time_left with x0 upper half set: not Ok(7)");
	}
	expect_call(0, F, 4, 0, 0, "time_left");

	/* commands: PET, GET_TIMELEFT, INIT (ENABLE 0 is never sent here) */
	reset_all();
	CHECK(mp2_smcwdt_ping(F) == 0, "ping");
	CHECK(mp2_smcwdt_time_left(F, NULL) == 0, "time_left");
	CHECK(mp2_smcwdt_cmd(F, MP2_SMCWDT_CMD_INIT, 0, NULL) == 0, "init");
	CHECK(ncalls == 3, "commands: %d calls", ncalls);
	expect_call(0, F, 3, 0, 0, "commands PET");
	expect_call(1, F, 4, 0, 0, "commands GET_TIMELEFT");
	expect_call(2, F, 0, 0, 0, "commands INIT");

	/* 3. the boot: arm 32 s with the board's id, the marker, the feeder */
	reset_all();
	CHECK(boot() == 0 && resets == 0, "the boot reset the board");
	CHECK(ncalls == 3, "boot made %d calls, expected 3", ncalls);
	expect_call(0, FID, 1, 32, 0, "boot SET_TIMEOUT 32");
	expect_call(1, FID, 2, 1, 0, "boot ENABLE 1");
	expect_call(2, FID, 3, 0, 0, "boot PET");
	CHECK(s_wdog.arm_rc == 0 && s_wdog.timeout_s == 32, "state rc %d timeout %u",
	      (int)s_wdog.arm_rc, (unsigned int)s_wdog.timeout_s);
	CHECK(noted_calls == 1 && noted_s == 32, "core not told (%d calls, %u s)", noted_calls,
	      (unsigned int)noted_s);
	CHECK(strstr(out, "gc_wdog_arm: armed OP-TEE arm,smc-wdt fid 0xbc000000, timeout 32 s") ==
		      out,
	      "marker missing or not first: '%s'", out);
	CHECK(mp2_smcwdt_feeder.running && mp2_smcwdt_feeder.first_ms == 1000 && mp2_smcwdt_feeder.period_ms == 1000 &&
		      s_wdog.feeding,
	      "feeder timer %d first %lld period %lld", mp2_smcwdt_feeder.running,
	      (long long)mp2_smcwdt_feeder.first_ms, (long long)mp2_smcwdt_feeder.period_ms);

	/* one feeder tick: one PET */
	ncalls = 0;
	now_ms = 1000;
	mp2_smcwdt_feeder.expiry(&mp2_smcwdt_feeder);
	CHECK(ncalls == 1 && s_wdog.feeds == 1 && mp2_smcwdt_feeder.running, "tick: %d calls, %u feeds",
	      ncalls, (unsigned int)s_wdog.feeds);
	expect_call(0, FID, 3, 0, 0, "tick PET");

	/* a refused PET: counted, printed once, the feeder goes on */
	ncalls = 0;
	out[0] = '\0';
	reply((unsigned long)(uint32_t)MP2_SMCWDT_RET_DENIED, 0);
	reply((unsigned long)(uint32_t)MP2_SMCWDT_RET_DENIED, 0);
	now_ms = 2000;
	mp2_smcwdt_feeder.expiry(&mp2_smcwdt_feeder);
	now_ms = 3000;
	mp2_smcwdt_feeder.expiry(&mp2_smcwdt_feeder);
	CHECK(ncalls == 2 && s_wdog.pet_failures == 2 && s_wdog.first_pet_rc == -3 &&
		      mp2_smcwdt_feeder.running,
	      "refused PETs: %d calls, %u failures, first %d", ncalls,
	      (unsigned int)s_wdog.pet_failures, (int)s_wdog.first_pet_rc);
	CHECK(strcmp(out, "mp2: OP-TEE watchdog PET answered -3\n") == 0, "PET refusal print: '%s'",
	      out);

	/* at the lifetime: no call, the feeder stops */
	ncalls = 0;
	now_ms = LIFETIME_MS;
	mp2_smcwdt_feeder.expiry(&mp2_smcwdt_feeder);
	CHECK(ncalls == 0 && !mp2_smcwdt_feeder.running && !s_wdog.feeding,
	      "at the lifetime: %d calls, timer %d", ncalls, mp2_smcwdt_feeder.running);
	now_ms = 0;

	/* 4. the boot with a refusal at each step: what refused, PSCI reset */
	static const char *const step_name[] = {"SET_TIMEOUT", "ENABLE", "PET"};
	for (int step = 0; step < 3; step++) {
		char want[128];

		reset_all();
		for (int i = 0; i < step; i++) {
			reply(0, 0);
		}
		reply((unsigned long)(uint32_t)MP2_SMCWDT_RET_DENIED, 0);
		CHECK(boot() == 1 && resets == 1 && reset_why != NULL &&
			      strcmp(reset_why, "watchdog refused") == 0,
		      "step %d: no PSCI reset after a refusal", step);
		CHECK(ncalls == step + 1, "step %d: %d calls", step, ncalls);
		snprintf(want, sizeof(want),
			 "mp2: OP-TEE watchdog arming FAILED: %s answered -3 (fid 0xbc000000)",
			 step_name[step]);
		CHECK(strstr(out, want) == out, "step %d: '%s'", step, out);
		CHECK(strstr(out, "gc_wdog_arm: armed") == NULL, "step %d: marker printed", step);
		CHECK(noted_calls == 0 && !mp2_smcwdt_feeder.running, "step %d: noted or feeding", step);
	}

	/* 5. the constants */
	CHECK(MP2_SMCWDT_FUNC_ID == 0xbc000000u && MP2_SMCWDT_TIMEOUT_S == 32 &&
		      MP2_SMCWDT_FEED_MS == 1000 && MP2_SMCWDT_CMD_INIT == 0 &&
		      MP2_SMCWDT_CMD_SET_TIMEOUT == 1 && MP2_SMCWDT_CMD_ENABLE == 2 &&
		      MP2_SMCWDT_CMD_PET == 3 && MP2_SMCWDT_CMD_GET_TIMELEFT == 4,
	      "constants moved");

	printf("stm32mp2_wdog_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed != 0;
}
