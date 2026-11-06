/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi_wdog_mock: driver/zephyr/lib/board/rpi4/rpi_wdog.c, unchanged, against
 * a mocked register block. The file is #included so the test reaches its
 * statics (arm, feed, the feeder timer); the Zephyr symbols it needs are the
 * stubs under stub/zephyr/.
 *
 * One build per SoC (the SoC is a compile-time parameter of rpi_wdog.c):
 *   -DCONFIG_SOC_BCM2712 -DCONFIG_YAKOGL_RPI5_WDOG ...   the Pi 5
 *   neither                                              the Pi 4 (BCM2711)
 * Checked: where the block is mapped, which hook arms and at which level, the
 * PM_PASSWORD in every write, PM_WDOG's 16.16 ticks for the period (and its
 * 20-bit clamp), PM_RSTC's full-reset field with the other bits kept, the
 * feeder's re-arm, and the stop (no write, no timer) at the deadline. The
 * BCM2711 write sequence is spelled out literally: it is the one in use today.
 *
 * Prints "rpi_wdog_mock <soc>: N of N checks, 0 failed"; exit 0 when none failed.
 */
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

#include "rpi_wdog.c"

#ifdef CONFIG_SOC_BCM2712
#define SOC "bcm2712"
#define EXPECT_PHYS 0x107d200000ull
#define EXPECT_PRE_HOOK 1 /* armed in PRE_KERNEL_1, the timer in POST_KERNEL */
#else
#define SOC "bcm2711"
#define EXPECT_PHYS 0xfe100000ull
#define EXPECT_PRE_HOOK 0
#endif

/* ---- the mock ------------------------------------------------------------ */

static uint8_t block[0x1000] __attribute__((aligned(0x1000)));
static uint32_t rstc_reg = 0x00000132; /* WRCFG = 3 and other bits set */
static uint32_t wdog_reg;
static unsigned long long mapped_phys;
static unsigned long mapped_size;
static int map_calls;
static int64_t now_ms;

struct wr {
	uint32_t off, val;
};
static struct wr wrs[64];
static int nwr;
static int bad_access; /* outside the block, or a read of anything but RSTC */
static int bad_password; /* writes whose top byte is not PM_PASSWORD */

static char out[1024];

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

void device_map(mm_reg_t *virt, unsigned long long phys, unsigned long size, unsigned int flags)
{
	map_calls++;
	mapped_phys = phys;
	mapped_size = size;
	CHECK(flags == K_MEM_CACHE_NONE, "register block mapped with flags %u", flags);
	*virt = (mm_reg_t)block;
}

uint32_t sys_read32(mm_reg_t addr)
{
	uint32_t off = (uint32_t)(addr - (mm_reg_t)block);
	if (addr < (mm_reg_t)block || off >= sizeof(block) || off != PM_RSTC) {
		bad_access++;
		return 0;
	}
	return rstc_reg;
}

void sys_write32(uint32_t val, mm_reg_t addr)
{
	uint32_t off = (uint32_t)(addr - (mm_reg_t)block);
	if (addr < (mm_reg_t)block || off >= sizeof(block)) {
		bad_access++;
		return;
	}
	if ((val & 0xff000000u) != 0x5a000000u)
		bad_password++;
	if (nwr < (int)(sizeof(wrs) / sizeof(wrs[0])))
		wrs[nwr++] = (struct wr){ off, val };
	if (off == PM_WDOG)
		wdog_reg = val & 0x000fffffu;
	else if (off == PM_RSTC)
		rstc_reg = val & 0x00ffffffu;
	else
		bad_access++;
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

static struct {
	int (*fn)(void);
	int level;
} hooks[8];
static int nhooks;

void stub_sys_init_add(int (*fn)(void), int level)
{
	hooks[nhooks].fn = fn;
	hooks[nhooks].level = level;
	nhooks++;
}

/* ---- helpers ------------------------------------------------------------- */

static void clear_log(void)
{
	nwr = 0;
}

static int run_level(int level)
{
	int n = 0;
	for (int i = 0; i < nhooks; i++) {
		if (hooks[i].level == level) {
			CHECK(hooks[i].fn() == 0, "hook at level %d failed", level);
			n++;
		}
	}
	return n;
}

static void expect_wr(int i, uint32_t off, uint32_t val, const char *what)
{
	CHECK(i < nwr && wrs[i].off == off && wrs[i].val == val,
	      "%s: write %d is %#x <- %#x, expected %#x <- %#x", what, i,
	      i < nwr ? wrs[i].off : 0, i < nwr ? wrs[i].val : 0, off, val);
}

int main(void)
{
	/* 1. the hooks and what they do */
	int pre = 0, post = 0;
	for (int i = 0; i < nhooks; i++) {
		pre += hooks[i].level == PRE_KERNEL_1;
		post += hooks[i].level == POST_KERNEL;
	}
	CHECK(pre == EXPECT_PRE_HOOK && post == 1, SOC ": %d PRE_KERNEL_1 and %d POST_KERNEL hooks", pre,
	      post);

	run_level(PRE_KERNEL_1);
	if (EXPECT_PRE_HOOK) {
		CHECK(map_calls == 1, "block mapped %d times by the early hook", map_calls);
		CHECK(nwr == 2, "early hook wrote %d registers, expected 2", nwr);
		CHECK(!feeder.running, "feeder timer started before POST_KERNEL");
		CHECK(out[0] == '\0', "console line printed before POST_KERNEL");
	} else {
		CHECK(map_calls == 0 && nwr == 0, "BCM2711 touched the SoC before POST_KERNEL");
	}
	run_level(POST_KERNEL);

	CHECK(map_calls == 1, "block mapped %d times", map_calls);
	CHECK(mapped_phys == EXPECT_PHYS, "mapped %#llx, expected %#llx", mapped_phys, EXPECT_PHYS);
	CHECK(mapped_size >= PM_WDOG + 4, "mapped size %#lx does not cover PM_WDOG", mapped_size);
	CHECK(feeder.running && feeder.first_ms == FEED_EVERY_MS && feeder.period_ms == FEED_EVERY_MS,
	      "feeder timer %d, first %lld, period %lld", feeder.running, (long long)feeder.first_ms,
	      (long long)feeder.period_ms);
	CHECK(strstr(out, "board: watchdog armed\n") == out, "armed line missing or not first: '%s'", out);
#ifndef CONFIG_SOC_BCM2712
	rpi_wdog_announce(); /* the Pi 4 image announces by itself, not the hook */
#endif
	CHECK(strstr(out, "board: PM watchdog resets the SoC ~") != NULL, "announce line missing");

	/* 2. the arm sequence: PM_WDOG, then PM_RSTC (read-modify-write) */
	CHECK(nwr == 2, "arm wrote %d registers, expected 2", nwr);
	expect_wr(0, 0x24, 0x5a000000u | (10u << 16), "arm PM_WDOG (10 s)");
	expect_wr(1, 0x1c, 0x5a000122u, "arm PM_RSTC (WRCFG=2, other bits kept)");
	CHECK(wdog_reg == 10u << 16, "PM_WDOG holds %#x", wdog_reg);
	CHECK((rstc_reg & PM_RSTC_WRCFG_MASK) == 0x20, "PM_RSTC WRCFG is %#x, not full reset", rstc_reg & 0x30);
	CHECK((rstc_reg & ~0x30u) == (0x132u & ~0x30u), "PM_RSTC other bits changed: %#x", rstc_reg);
	CHECK(WDOG_PERIOD_S == 10, "period is %d s", WDOG_PERIOD_S);

	/* 3. the announced time follows the hold */
	char want[96];
	snprintf(want, sizeof(want), "~%d s after boot", RPI_WDOG_HOLD_S + WDOG_PERIOD_S);
	CHECK(strstr(out, want) != NULL, "announce does not say '%s': %s", want, out);
#ifdef CONFIG_SOC_BCM2712
	CHECK(RPI_WDOG_HOLD_S == CONFIG_YAKOGL_RPI5_WDOG_HOLD_S, "hold %d", RPI_WDOG_HOLD_S);
#else
	CHECK(RPI_WDOG_HOLD_S == 50, "BCM2711 hold is %d, not the default 50", RPI_WDOG_HOLD_S);
#endif

	/* 4. feed: the same pair each tick, until the deadline */
	int64_t hold_ms = (int64_t)RPI_WDOG_HOLD_S * 1000;
	for (int tick = 1; tick <= 3; tick++) {
		clear_log();
		now_ms = (int64_t)tick * FEED_EVERY_MS;
		feeder.expiry(&feeder);
		CHECK(nwr == 2 && feeder.running, "tick %d: %d writes, timer %d", tick, nwr, feeder.running);
		expect_wr(0, 0x24, 0x5a0a0000u, "feed PM_WDOG");
		expect_wr(1, 0x1c, 0x5a000122u, "feed PM_RSTC");
	}
	clear_log();
	now_ms = hold_ms - 1;
	feeder.expiry(&feeder);
	CHECK(nwr == 2 && feeder.running, "just before the deadline: %d writes, timer %d", nwr, feeder.running);

	/* 5. disarm: at the deadline the feeding stops (no write, no timer) */
	clear_log();
	now_ms = hold_ms;
	feeder.expiry(&feeder);
	CHECK(nwr == 0, "feed at the deadline wrote %d registers", nwr);
	CHECK(!feeder.running, "feeder timer still running at the deadline");

	/* 6. arm() itself: the 16.16 ticks and the 20-bit clamp */
	static const struct {
		uint32_t seconds, reg;
	} t[] = {
		{ 0, 0x5a000000u }, { 1, 0x5a010000u }, { 10, 0x5a0a0000u },
		{ 15, 0x5a0f0000u }, { 16, 0x5a0fffffu }, { 100, 0x5a0fffffu },
	};
	for (unsigned i = 0; i < sizeof(t) / sizeof(t[0]); i++) {
		clear_log();
		arm(t[i].seconds);
		expect_wr(0, 0x24, t[i].reg, "arm(seconds)");
		expect_wr(1, 0x1c, 0x5a000122u, "arm RSTC");
	}
	/* RSTC read-modify-write from a clean value */
	rstc_reg = 0;
	clear_log();
	arm(1);
	expect_wr(1, 0x1c, 0x5a000020u, "arm RSTC from 0");

	/* 7. every write had the password, nothing strayed */
	CHECK(bad_password == 0, "%d writes without PM_PASSWORD", bad_password);
	CHECK(bad_access == 0, "%d accesses outside PM_WDOG/PM_RSTC (or reads of PM_WDOG)", bad_access);

#ifndef CONFIG_SOC_BCM2712
	/* BCM2711: the literal sequence of the Pi 4 image, as it is today */
	CHECK(PM_BASE == 0xfe100000u && PM_RSTC == 0x1c && PM_WDOG == 0x24 &&
		      PM_PASSWORD == 0x5a000000u && PM_WDOG_TICKS_MAX == 0xfffffu &&
		      PM_RSTC_WRCFG_MASK == 0x30u && PM_RSTC_FULL_RESET == 0x20u &&
		      FEED_EVERY_MS == 3000,
	      "BCM2711 constants moved");
#endif

	printf("rpi_wdog_mock %s: %d of %d checks, %d failed\n", SOC, checks - failed, checks, failed);
	return failed != 0;
}
