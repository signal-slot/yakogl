/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The i.MX8MP result sink: the record in the SNVS LPGPR (gc_result.h), the
 * selector, and the STAGE marks. The writer's rules are the board layer's
 * (common/record.c over driver/rust/src/board/record.rs); the gc_result_*
 * calls are that writer by their old names, and board_sink_store() below is
 * where every record lands.
 */

#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/sys_io.h>

#include "gc_result.h"
#include "yakogl_board.h"
#include "board_internal.h"
#ifdef CONFIG_GC_BOARD_WDOG_SELF_ARM
#include "gc_wdog_arm.h"
#endif

#define SNVS_BASE  0x30370000u
#define SNVS_LPGPR 0x90u /* LPGPR0..3 at 0x90, 0x94, 0x98, 0x9C. 0x68 aliases
			  * LPGPR0 and is deliberately not used. */

#ifdef CONFIG_GC_COMMON_RESULT_SNVS_KEEP_LPGPR0
#define REC_OFF 4u   /* the record is LPGPR1..3: w0, w1, w2 (w3 dropped) */
#else
#define REC_OFF 0u
#endif

static mm_reg_t s_snvs;
static int s_selector = -1;  /* what the reader asked this run for, if anything */

int gc_result_selector(void)
{
	return s_selector;
}

int board_selector(void)
{
	return s_selector;
}

/* Mapped before any driver runs, as the watchdog is (gc_sample.c), so that the
 * fatal handler can store a record without calling into the memory manager
 * from an exception. */
static int result_map(void)
{
	device_map(&s_snvs, SNVS_BASE, 0x1000, K_MEM_CACHE_NONE);
	/* The selector, read before the first record overwrites it. With
	 * KEEP_LPGPR0 the record starts one register later (gc_result.h). */
	if (sys_read32(s_snvs + SNVS_LPGPR + REC_OFF) == 0u) {
		uint32_t w1 = sys_read32(s_snvs + SNVS_LPGPR + REC_OFF + 4u);

		if ((w1 & 0xffff0000u) == GC_RESULT_SELECTOR_MAGIC) {
			s_selector = (int)(w1 & 0xffffu);
		}
	}
	gc_result_progress(GC_RESULT_WHERE_IMAGE);
	return 0;
}
SYS_INIT(result_map, PRE_KERNEL_1, 0);

static int result_kernel_up(void)
{
	gc_result_progress(GC_RESULT_WHERE_KERNEL);
	return 0;
}
SYS_INIT(result_kernel_up, APPLICATION, 0);

void gc_result_progress(uint32_t where)
{
	board_progress(where);
}

void gc_result_breadcrumb(unsigned int kind, uint8_t tag, uint32_t w1, uint32_t w2,
			  uint32_t w3)
{
	board_record_breadcrumb(kind, tag, w1, w2, w3);
}

void gc_result_stage(uint8_t stage, uint32_t value)
{
	uint32_t feeds = 0;

	if (!IS_ENABLED(CONFIG_GC_COMMON_RESULT_STAGES)) {
		return;
	}
#ifdef CONFIG_GC_BOARD_WDOG_SELF_ARM
	feeds = gc_wdog_arm_feeds();
#endif
	gc_result_breadcrumb(GC_RESULT_STAGE, stage, value, k_uptime_get_32(), feeds);
}

#ifdef CONFIG_GC_COMMON_RESULT_STAGES
/*
 * The driver boundaries as stages (gc_result.h, kind STAGE). Each SYS_INIT
 * runs right after the drivers of the priorities below it, so the last mark
 * a dead image left names the group of drivers it died in. The priorities
 * are the ones the gc_gl_triangle image runs (`west build -t initlevels`,
 * 2026-10-03); the asserts keep each mark after the drivers it names.
 */
#define STAGE_MARK(name, stage, level, prio)                                   \
	static int name(void)                                                  \
	{                                                                      \
		gc_result_stage(stage, 0u);                                    \
		return 0;                                                      \
	}                                                                      \
	SYS_INIT(name, level, prio)

STAGE_MARK(stage_media_early_clocks, GC_RESULT_STAGE_MEDIA_EARLY_CLOCKS, PRE_KERNEL_1, 6);
BUILD_ASSERT(CONFIG_CLOCK_CONTROL_INIT_PRIORITY == 30, "stage 0x12 follows the CCM driver at 30");
STAGE_MARK(stage_ccm, GC_RESULT_STAGE_CCM, PRE_KERNEL_1, 31);
BUILD_ASSERT(CONFIG_SERIAL_INIT_PRIORITY <= 50 && CONFIG_KERNEL_INIT_PRIORITY_DEVICE <= 50,
	     "stage 0x15 follows every PRE_KERNEL_1 driver");
STAGE_MARK(stage_pre_kernel_1_done, GC_RESULT_STAGE_PRE_KERNEL_1_DONE, PRE_KERNEL_1, 99);
BUILD_ASSERT(CONFIG_SYSTEM_CLOCK_INIT_PRIORITY == 0, "stage 0x16 follows the system clock");
STAGE_MARK(stage_sys_clock, GC_RESULT_STAGE_SYS_CLOCK, PRE_KERNEL_2, 1);
/* After gc_wdog_arm.c's feeder start (POST_KERNEL 0). */
STAGE_MARK(stage_post_kernel, GC_RESULT_STAGE_POST_KERNEL, POST_KERNEL, 1);
BUILD_ASSERT(CONFIG_GPIO_INIT_PRIORITY == 40, "stage 0x21 follows GPIO at 40");
STAGE_MARK(stage_gpio, GC_RESULT_STAGE_GPIO, POST_KERNEL, 41);
BUILD_ASSERT(CONFIG_I2C_INIT_PRIORITY == 50 && CONFIG_PWM_INIT_PRIORITY == 50,
	     "stage 0x22 follows PWM and I2C at 50");
STAGE_MARK(stage_pwm_i2c, GC_RESULT_STAGE_PWM_I2C, POST_KERNEL, 51);
#ifdef CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP
BUILD_ASSERT(CONFIG_YAKOGL_DRV_IMX8MP_MEDIA_INIT_PRIORITY == 60 &&
	     CONFIG_YAKOGL_DRV_IMX8MP_MEDIA_PD_INIT_PRIORITY == 61, "stage 0x23 follows media at 60, 61");
STAGE_MARK(stage_media, GC_RESULT_STAGE_MEDIA, POST_KERNEL, 62);
BUILD_ASSERT(CONFIG_YAKOGL_DRV_SN65DSI8X_INIT_PRIORITY == 75, "stage 0x24 follows the bridge at 75");
STAGE_MARK(stage_bridge, GC_RESULT_STAGE_BRIDGE, POST_KERNEL, 76);
BUILD_ASSERT(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_INIT_PRIORITY == 80, "stage 0x25 follows the display at 80");
STAGE_MARK(stage_display, GC_RESULT_STAGE_DISPLAY, POST_KERNEL, 81);
#endif
#endif /* CONFIG_GC_COMMON_RESULT_STAGES */

void gc_result_write(unsigned int kind, uint32_t w1, uint32_t w2, uint32_t w3)
{
	board_result(kind, w1, w2, w3);
}

void gc_result_write_tag(unsigned int kind, uint8_t tag, uint32_t w1, uint32_t w2,
			 uint32_t w3)
{
	board_result_tag(kind, tag, w1, w2, w3);
}

/* The sink (board_internal.h): one record into LPGPR0..3, magic last. */
void board_sink_store(const uint32_t w[4])
{
	mm_reg_t r = s_snvs;
	uint32_t w1 = w[1], w2 = w[2], w3 = w[3];

	if (r == 0) {
		return;
	}
	/* Clear the magic first: a record overwritten half way through must not
	 * read as the old magic over new words. */
	sys_write32(0u, r + SNVS_LPGPR + REC_OFF);
	__asm__ volatile("dsb sy" ::: "memory");
	sys_write32(w1, r + SNVS_LPGPR + REC_OFF + 4u);
	sys_write32(w2, r + SNVS_LPGPR + REC_OFF + 8u);
#ifdef CONFIG_GC_COMMON_RESULT_SNVS_KEEP_LPGPR0
	(void)w3;
#else
	sys_write32(w3, r + SNVS_LPGPR + 12u);
#endif
	/* Words 1..3 before the magic; the same rule as the fault record. dsb,
	 * not dmb, because these are Device-memory stores and the reset may come
	 * right after this returns. */
	__asm__ volatile("dsb sy" ::: "memory");
	sys_write32(w[0], r + SNVS_LPGPR + REC_OFF);
	__asm__ volatile("dsb sy" ::: "memory");
}
