/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The board layer on the NanoPC-T6 (yakogl_board.h;
 * driver/docs/nanopc-t6-display.md, step D3):
 *   window   the last pages of the Mali OSAL's carve-out
 *            (osal_mali_board_window): the GPU's address space maps them
 *            with the rest and no allocation returns them. Uncached, so
 *            neither the GPU nor a check needs cache maintenance. The OSAL
 *            fixes the carve-out's layout when EGL starts it, so the window
 *            is taken before eglInitialize()
 *   screen   VP0 of the VOP on HDMI0, 1024x768 or 1920x1080 at 60 Hz as
 *            Linux left it: the first board_show() adopts it with the window
 *            at the screen's origin, the rest of a larger screen black
 *            (yd_t6_screen_show), and prints the adopt's record; a
 *            later show of another buffer is a flip, which never waits.
 *            Nothing is copied. An adopt that stopped is final
 *   record   the yk-record line on the UART (common/record.c)
 *   watchdog WDT0 (nanopc_t6.c), fed while board_wdog_alive() says so
 *   reset    t6_reset_to_linux(): PSCI SYSTEM_RESET, back into Armbian
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <errno.h>
#include <string.h>
#include <EGL/eglplatform.h>

#include <yakogl_drivers.h>

#include "yakogl_board.h"
#include "board_internal.h"
#include "nanopc_t6.h"
#include "osal_internal.h"

/* The display path's register blocks (rk3588-base.dtsi: vop, vop_mmu, pmu,
 * cru, sys_grf, hdptxphy0_grf, hdmi0), as the t6_display probe maps them. */
#define VOP_PHYS     0xfdd90000UL
#define VOP_MAP      0x2000
#define VOP_MMU_PHYS 0xfdd97000UL /* the two sets at +0xe00, +0xf00 */
#define PMU_PHYS     0xfd8d8000UL
#define CRU_PHYS     0xfd7c0000UL
#define SYS_GRF_PHYS 0xfd58c000UL
#define PHY_GRF_PHYS 0xfd5e0000UL
#define HDMI_PHYS    0xfde80000UL

static struct yd_t6_display_bases s_bases;
static struct yd_t6_screen s_screen;
static bool s_mapped;

int board_window(uint32_t w, uint32_t h, struct board_window *out)
{
	uint64_t phys;
	void *cpu;
	int rc;

	memset(out, 0, sizeof(*out));
	if (w == 0u || h == 0u || h > SIZE_MAX / 4u / w) {
		return -EINVAL;
	}
	rc = osal_mali_board_window((size_t)w * h * 4u, &phys, &cpu);
	if (rc != 0) {
		return rc;
	}
	*out = (struct board_window){
		.phys = phys,
		.cpu = cpu,
		.w = w,
		.h = h,
		.stride = w * 4u,
		.format = GLES_NATIVE_FORMAT_XRGB8888,
	};
	return 0;
}

static void delay_us(uint32_t us)
{
	k_busy_wait(us);
}

static void map_bases(void)
{
	mm_reg_t vop, mmu, pmu, cru, sys_grf, phy_grf, hdmi;

	device_map(&vop, VOP_PHYS, VOP_MAP, K_MEM_CACHE_NONE);
	device_map(&mmu, VOP_MMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&cru, CRU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&sys_grf, SYS_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&phy_grf, PHY_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&hdmi, HDMI_PHYS, 0x1000, K_MEM_CACHE_NONE);
	s_bases = (struct yd_t6_display_bases){
		.vop = vop,
		.vop_mmu = {mmu + 0xe00, mmu + 0xf00},
		.pmu = pmu,
		.cru = cru,
		.sys_grf = sys_grf,
		.phy_grf = phy_grf,
		.hdmi = hdmi,
	};
	s_mapped = true;
}

int board_show(const struct board_window *win)
{
	struct yd_t6_display_adopt adopt;
	bool first = s_screen.state == YD_T6_SCREEN_NEW;
	int rc;

	board_first_frame();
	/* the VOP's window is set up for XRGB8888, top row first */
	if (win->format != GLES_NATIVE_FORMAT_XRGB8888) {
		return -ENOTSUP;
	}
	if (!s_mapped) {
		map_bases();
	}
	rc = yd_t6_screen_show(&s_bases, &s_screen, win->phys, win->w, win->h, win->stride,
			       delay_us, &adopt);
	/* the show that left YD_T6_SCREEN_NEW was the adopt: its record */
	if (first && s_screen.state != YD_T6_SCREEN_NEW) {
		t6_display_print_adopt("board", rc, &adopt);
	}
	return rc;
}

#ifdef CONFIG_YAKOGL_BOARD_DISPLAY_BUSY
/* A flip above does not wait for the VOP to take the address, so the display
 * may still scan an earlier buffer: the screen keeps which. Before the first
 * show nothing is mapped and no buffer is busy. */
uint32_t board_display_busy(const uint64_t *phys, size_t n)
{
	if (!s_mapped) {
		return 0;
	}
	return yd_t6_screen_busy(&s_bases, &s_screen, phys, n);
}
#endif

int board_selector(void)
{
	return -1;
}

void board_sink_store(const uint32_t w[4])
{
	ARG_UNUSED(w); /* the UART line is the sink */
}

FUNC_NORETURN void board_reset(void)
{
	/* A single-buffered sample shows one buffer and never flips; the count
	 * says whether a double-buffered run reached the flip and the busy rule. */
	printk("board: screen state %u, %u flips\n", s_screen.state, s_screen.flips);
	t6_reset_to_linux("board: run over");
}
