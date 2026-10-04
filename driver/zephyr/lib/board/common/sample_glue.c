/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Board glue shared by the Zephyr samples: DSS application plane, the
 * hold-then-reset exit, and a fatal-error handler that resets instead of
 * halting (there is no reset button at hand on the lab board).
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/init.h>
#include <zephyr/sys/printk.h>
#include <zephyr/fatal.h>
#include <zephyr/logging/log.h>
#include <zephyr/sys/reboot.h>
#include <zephyr/cache.h>
#include <errno.h>
#ifdef CONFIG_PVR_SAMPLE_SHOW_ON_DSS
#include <safeui/am62p_dss.h>
#endif
#include <stdbool.h>
#include <EGL/eglplatform.h>
#include "sample_glue.h"
#ifdef CONFIG_GC_COMMON_RESULT_SNVS
#include <zephyr/arch/cpu.h>
#include <zephyr/arch/arm64/exception.h>
#include "gc_result.h"
#endif

LOG_MODULE_REGISTER(pvr_sample, LOG_LEVEL_INF);

/*
 * The build identity, first thing on the console.
 *
 * A row from a board run has to say which tree and which shader table
 * produced it. The commit alone does not: an image for a table-rebuild window
 * is built from a worktree whose src/shaderc/blobs.rs is deliberately
 * uncommitted, so two images from the same commit can hold different tables.
 * sample.cmake captures both at configure time (`git describe --always
 * --dirty` and the first eight hex digits of the table's SHA-256); printk
 * rather than LOG_INF so it cannot be lost to a log level or a dropped
 * backend, and SYS_INIT at PRE_KERNEL_2 so it precedes the GPU and the first
 * case.
 */
#ifndef PVR_SAMPLE_TREE
#define PVR_SAMPLE_TREE "unknown"
#endif
#ifndef PVR_SAMPLE_TABLE
#define PVR_SAMPLE_TABLE "none"
#endif

static int pvr_sample_print_build(void)
{
	printk("sample: build tree=%s table=%s\n", PVR_SAMPLE_TREE, PVR_SAMPLE_TABLE);
	return 0;
}
SYS_INIT(pvr_sample_print_build, PRE_KERNEL_2, 0);

void pvr_sample_flush_dcache(void *addr, size_t size)
{
	sys_cache_data_flush_and_invd_range(addr, size);
}

#if defined(CONFIG_PVR_SAMPLE_SHOW_ON_DSS) && defined(AM62P_DSS_PLANE_VFLIP)
#define PVR_SAMPLE_CAN_VFLIP 1
#else
#define PVR_SAMPLE_CAN_VFLIP 0
#endif

#ifdef CONFIG_PVR_SAMPLE_SHOW_ON_DSS
static void plane_init(struct am62p_dss_plane_cfg *plane, uintptr_t phys, uint32_t width,
		       uint32_t height, uint32_t stride_bytes, bool vflip)
{
	*plane = (struct am62p_dss_plane_cfg){
		.pa = phys, .width = width, .height = height,
		.stride_bytes = stride_bytes, .format = AM62P_DSS_FMT_XRGB8888,
	};
#if PVR_SAMPLE_CAN_VFLIP
	plane->vflip = vflip;
#else
	ARG_UNUSED(vflip);
#endif
}
#endif

static int show_on_dss(uintptr_t phys, uint32_t width, uint32_t height, uint32_t stride_bytes,
		       bool vflip)
{
#ifdef CONFIG_PVR_SAMPLE_SHOW_ON_DSS
	/* Layer 0; the driver's own safety layer above it stays transparent. */
	const struct device *dss = DEVICE_DT_GET(DT_NODELABEL(dss1));
	struct am62p_dss_plane_cfg plane;
	int rc;

	if (!device_is_ready(dss)) {
		LOG_ERR("DSS not ready");
		return -ENODEV;
	}
	plane_init(&plane, phys, width, height, stride_bytes, vflip);
	rc = am62p_dss_set_app_buffer(dss, &plane);
	if (rc == 0) {
		rc = am62p_dss_enable_app_plane(dss, true);
	}
	if (rc == 0) {
		rc = am62p_dss_output_enable(dss, true);
	}
	if (rc) {
		LOG_ERR("DSS application plane failed (%d)", rc);
	}
	return rc;
#else
	ARG_UNUSED(phys);
	ARG_UNUSED(width);
	ARG_UNUSED(height);
	ARG_UNUSED(stride_bytes);
	ARG_UNUSED(vflip);
	return 0;
#endif
}

int pvr_sample_show_on_dss(uintptr_t phys, uint32_t width, uint32_t height,
			   uint32_t stride_bytes)
{
	return show_on_dss(phys, width, height, stride_bytes, false);
}

uint32_t pvr_sample_window_flags(void)
{
	return PVR_SAMPLE_CAN_VFLIP ? GLES_NATIVE_WINDOW_BOTTOM_UP : 0;
}

int pvr_sample_show_window(uintptr_t phys, uint32_t width, uint32_t height,
			   uint32_t stride_bytes, uint32_t window_format)
{
	bool bottom_up = (window_format & GLES_NATIVE_WINDOW_BOTTOM_UP) != 0;

	if (bottom_up && !PVR_SAMPLE_CAN_VFLIP) {
		return -ENOTSUP;
	}
	if (bottom_up) {
		printk("sample: window scanned bottom row first (vflip)\n");
	}
	return show_on_dss(phys, width, height, stride_bytes, bottom_up);
}

uint32_t pvr_sample_vsync_count(void)
{
#ifdef CONFIG_PVR_SAMPLE_SHOW_ON_DSS
	const struct device *dss = DEVICE_DT_GET(DT_NODELABEL(dss1));
	struct am62p_dss_stats st;

	if (!device_is_ready(dss)) {
		return 0;
	}
	am62p_dss_get_stats(dss, &st);
	return st.vsyncs;
#else
	return 0;
#endif
}

static int flip_app_buffer(uintptr_t phys, uint32_t width, uint32_t height,
			   uint32_t stride_bytes, bool vflip)
{
#ifdef CONFIG_PVR_SAMPLE_SHOW_ON_DSS
	const struct device *dss = DEVICE_DT_GET(DT_NODELABEL(dss1));
	struct am62p_dss_plane_cfg plane;

	plane_init(&plane, phys, width, height, stride_bytes, vflip);
	return am62p_dss_set_app_buffer(dss, &plane);
#else
	ARG_UNUSED(phys);
	ARG_UNUSED(width);
	ARG_UNUSED(height);
	ARG_UNUSED(stride_bytes);
	ARG_UNUSED(vflip);
	return 0;
#endif
}

int pvr_sample_flip_app_buffer(uintptr_t phys, uint32_t width, uint32_t height,
			       uint32_t stride_bytes)
{
	return flip_app_buffer(phys, width, height, stride_bytes, false);
}

int pvr_sample_flip_window(uintptr_t phys, uint32_t width, uint32_t height,
			   uint32_t stride_bytes, uint32_t window_format)
{
	bool bottom_up = (window_format & GLES_NATIVE_WINDOW_BOTTOM_UP) != 0;

	if (bottom_up && !PVR_SAMPLE_CAN_VFLIP) {
		return -ENOTSUP;
	}
	return flip_app_buffer(phys, width, height, stride_bytes, bottom_up);
}

#ifdef CONFIG_PVR_SAMPLE_SHOW_ON_DSS
/* The display's counters, for judging the scan-out without looking at the
 * panel: vsyncs over the hold give the frame rate, underflows and sync_lost
 * must stay 0. */
static void log_dss_stats(const char *when, int64_t t_ms)
{
	const struct device *dss = DEVICE_DT_GET(DT_NODELABEL(dss1));
	struct am62p_dss_stats st;

	if (!device_is_ready(dss)) {
		return;
	}
	am62p_dss_get_stats(dss, &st);
	LOG_INF("dss stats %s t=%lld ms: vsyncs %u app_frames %u underflows %u sync_lost %u "
		"go_timeouts %u go_waits %u safety_flips %u app %d",
		when, t_ms, st.vsyncs, st.app_frames, st.underflows, st.sync_lost,
		st.go_timeouts, st.go_waits, st.safety_flips, st.app_plane_enabled);
}
#endif

int pvr_sample_finish(int rc)
{
#ifdef CONFIG_PVR_SAMPLE_SHOW_ON_DSS
	log_dss_stats("before hold", k_uptime_get());
#endif
	if (CONFIG_PVR_SAMPLE_HOLD_SECONDS > 0) {
		k_sleep(K_SECONDS(CONFIG_PVR_SAMPLE_HOLD_SECONDS));
	}
#ifdef CONFIG_PVR_SAMPLE_SHOW_ON_DSS
	log_dss_stats("after hold", k_uptime_get());
#endif
	if (IS_ENABLED(CONFIG_PVR_SAMPLE_REBOOT)) {
		LOG_INF("sample done (%d): resetting the SoC", rc);
		sys_reboot(SYS_REBOOT_COLD);
	}
	return rc;
}

#ifdef CONFIG_PVR_SAMPLE_REBOOT
#ifdef CONFIG_GC_COMMON_RESULT_SNVS
/* The kind-3 record's tag: how far the application got. dEQP overrides it
 * with the cases begun (deqp_snvs.c), the same count its own records carry. */
__weak uint8_t pvr_sample_fault_tag(void)
{
	return 0;
}
#endif

void k_sys_fatal_error_handler(unsigned int reason, const struct arch_esf *esf)
{
#ifdef CONFIG_GC_COMMON_RESULT_SNVS
	/* A board with no console (the i.MX8MP on a Mallow carrier) learns of
	 * a fatal error only from the SNVS record, and a reset that leaves the
	 * last case's record in place reads as "no result": write the FAULT
	 * record (gc_result.h: ESR, ELR, FAR) FIRST, before anything that can
	 * fault again. A Rust panic reaches here through abort() -> k_panic(),
	 * so its ESR is the SVC class (EC 0x15); a data abort's is EC 0x25. */
	gc_result_write_tag(GC_RESULT_FAULT, pvr_sample_fault_tag(),
			    (uint32_t)read_sysreg(ESR_EL1),
			    (uint32_t)(esf != NULL ? esf->elr : 0),
			    (uint32_t)read_sysreg(FAR_EL1));
#endif
	ARG_UNUSED(esf);
	LOG_ERR("fatal error %u: resetting the SoC", reason);
	sys_reboot(SYS_REBOOT_COLD);
}
#endif
