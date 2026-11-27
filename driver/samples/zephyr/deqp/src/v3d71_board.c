/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * v3d_deqp's board side on a Raspberry Pi 5 (V3D 7.1): the record and the end
 * lines of v3d_board.c, over the UART alone (no screen).
 *
 * v3d_board.c is the Pi 4's: its bring-up returns before the printk tap and
 * the build line when the mailbox framebuffer fails (v3d_board.c:1152-1157),
 * and the Pi 5's screen is not proven on the board
 * (driver/docs/rpi5-display.md), so this image has none. This file includes
 * v3d_board.c, unedited, so the tally, the record and the end lines are the
 * same bytes of code ("v3d_deqp %s: build ..." and "v3d_deqp: build %s %s: N
 * cases, ..." for tools/board.sh pi5 and the tally tools), and replaces three
 * things around it:
 *
 *  - the init hook: the tap is installed without a framebuffer, with the
 *    build line and the watchdog's announcement of the Pi 4's hook;
 *  - fbcon: the screen's drawing calls are a sink, so the record goes to the
 *    UART once (tap() writes every character to the UART first);
 *  - the EGL default display: passed through. The 7.1 OSAL takes the V3D's
 *    clock switch from the board layer when it gets no platform
 *    (osal_zephyr_v3d_default_platform,
 *    driver/zephyr/lib/board/rpi5/rpi5_v3d.c), so the Pi 4's
 *    --wrap=eglGetDisplay is not linked (CMakeLists.txt, V3D71 branch).
 *
 * The watchdog is the board layer's (CONFIG_YAKOGL_RPI5_WDOG, armed before
 * the kernel starts, held CONFIG_YAKOGL_RPI5_WDOG_HOLD_S seconds, which the
 * CMake branch holds equal to CONFIG_V3D_DEQP_LIFETIME_S).
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/printk-hooks.h>
#include <zephyr/sys/libc-hooks.h>
#include <stdint.h>

/* fbcon.c is the Pi 4 screen path and is not linked here: fbcon_attach, which
 * take_record() calls, is a no-op defined below. */
/* The EGL default display goes through: v3d_board.c's two wrappers become
 * plain functions nothing calls. */
#define __wrap_eglGetDisplay v3d71_egl_get_display_unused
#define __real_eglGetDisplay eglGetDisplay
#define __wrap_eglGetPlatformDisplay v3d71_egl_get_platform_display_unused
#define __real_eglGetPlatformDisplay eglGetPlatformDisplay
/* Its Pi 4 init hook stays, renamed and not registered. */
#define v3d_deqp_board_init v3d_deqp_pi4_board_init
#pragma push_macro("SYS_INIT")
#undef SYS_INIT
#define SYS_INIT(fn, level, prio)

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-function"
#include "v3d_board.c"
#pragma GCC diagnostic pop

#pragma pop_macro("SYS_INIT")
#undef v3d_deqp_board_init
#undef __wrap_eglGetDisplay
#undef __real_eglGetDisplay
#undef __wrap_eglGetPlatformDisplay
#undef __real_eglGetPlatformDisplay

void fbcon_attach(uint8_t *fb, uint32_t width, uint32_t pitch, uint32_t rows_px, uint32_t scale)
{
	ARG_UNUSED(fb);
	ARG_UNUSED(width);
	ARG_UNUSED(pitch);
	ARG_UNUSED(rows_px);
	ARG_UNUSED(scale);
}

static int v3d71_screenless_out(int c)
{
	return c;
}

static int v3d71_deqp_board_init(void)
{
	/* No screen: the UART's hook is the console, the screen's is a sink. */
	have_screen = true; /* take_record() prints, to the UART */
	uart_out = __printk_get_hook();
	fbcon_out = v3d71_screenless_out;
	__printk_hook_install(tap);
	__stdout_hook_install(stdout_tap);

	printk("v3d_deqp: build %s\n", V3D_BUILD);
	rpi_wdog_announce();
	printk("v3d_deqp: the record at %d s after boot at the latest\n",
	       CONFIG_V3D_DEQP_LIFETIME_S - CONFIG_V3D_DEQP_PHOTO_S);

	k_work_schedule(&deadline_work,
			K_TIMEOUT_ABS_MS((int64_t)(CONFIG_V3D_DEQP_LIFETIME_S -
						   CONFIG_V3D_DEQP_PHOTO_S) * 1000));
	t.t_start_ms = t.t_end_ms = k_uptime_get();
	t.last_group = -1; /* no case yet */
	t.refused_mark = v3d_refused_draws(); /* 0 before EGL starts the backend */
	snprintf(t.filter, sizeof(t.filter), "'%.*s'", (int)sizeof(t.filter) - 3, case_filter);
	t.prefix_len = common_prefix(case_filter);
	return 0;
}

SYS_INIT(v3d71_deqp_board_init, APPLICATION, 99);
