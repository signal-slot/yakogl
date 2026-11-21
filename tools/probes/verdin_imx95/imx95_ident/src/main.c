/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * imx95_ident: what is the Verdin iMX95's GPU (README.md)? The pre-study's
 * B2 GPU half, U6:
 *   1. the SM's state of the GPU before (read only: PD 14, perf 9, clock 174)
 *   2. the GPU powered over SCMI (yd_imx95_gpu_power_on: soc::imx95::
 *      gpu_power, the port's own power-on); a refusal ends the run here
 *   3. every identity register the kernel side reads (gpu::mali::ident) and
 *      those panthor reads besides, one 32-bit read each, nothing written;
 *      printed as lines, as a table to paste into driver/rust/src/gpu/mali/
 *      mock.rs (G310, G310_ASSUMED), and against what mock.rs assumes
 *   4. the GPU off over SCMI (yd_imx95_gpu_power_off) and the SM's read-backs
 * then the verdict and the PSCI reset. No GPU register is read with the
 * domain off.
 */
#include <zephyr/devicetree.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>
#include <string.h>

#include <yakogl_drivers.h>

#include "verdin_imx95_scmi.h"
#include "vi95_probe.h"

#define GPU_NODE DT_NODELABEL(gpu)
BUILD_ASSERT(DT_REG_ADDR(GPU_NODE) == 0x4d900000, "the GPU is at 0x4d900000");

/* SM ids (soc::imx95::gpu_power): PD_GPU, PERF_GPU, CLK_GPU_CGC */
#define PD_GPU      14u
#define PERF_GPU    9u
#define CLK_GPU_CGC 174u

/* The identity registers, offsets from the GPU's base (panthor_regs.h;
 * gpu::mali's constants). `ours`: gpu::mali::ident reads it; `mock`: its
 * name in mock.rs's G310 table. */
static const struct {
	const char *name;
	uint32_t off;
	bool ours;
	const char *mock;
} regs[] = {
	{ "GPU_ID", 0x000, true, "GPU_ID" },
	{ "L2_FEATURES", 0x004, true, "GPU_L2_FEATURES" },
	{ "CORE_FEATURES", 0x008, false, NULL },
	{ "TILER_FEATURES", 0x00c, true, "GPU_TILER_FEATURES" },
	{ "MEM_FEATURES", 0x010, true, "GPU_MEM_FEATURES" },
	{ "MMU_FEATURES", 0x014, true, "GPU_MMU_FEATURES" },
	{ "AS_PRESENT", 0x018, true, "GPU_AS_PRESENT" },
	{ "CSF_ID", 0x01c, true, NULL },
	{ "GPU_STATUS", 0x034, true, NULL },
	{ "THREAD_MAX_THREADS", 0x0a0, false, NULL },
	{ "THREAD_MAX_WORKGROUP_SIZE", 0x0a4, false, NULL },
	{ "THREAD_MAX_BARRIER_SIZE", 0x0a8, false, NULL },
	{ "THREAD_FEATURES", 0x0ac, false, NULL },
	{ "TEXTURE_FEATURES_0", 0x0b0, false, NULL },
	{ "TEXTURE_FEATURES_1", 0x0b4, false, NULL },
	{ "TEXTURE_FEATURES_2", 0x0b8, false, NULL },
	{ "TEXTURE_FEATURES_3", 0x0bc, false, NULL },
	{ "SHADER_PRESENT_LO", 0x100, true, "GPU_SHADER_PRESENT" },
	{ "SHADER_PRESENT_HI", 0x104, true, NULL },
	{ "TILER_PRESENT_LO", 0x110, true, "GPU_TILER_PRESENT" },
	{ "TILER_PRESENT_HI", 0x114, true, NULL },
	{ "L2_PRESENT_LO", 0x120, true, "GPU_L2_PRESENT" },
	{ "L2_PRESENT_HI", 0x124, true, NULL },
	{ "COHERENCY_FEATURES", 0x300, true, NULL },
	{ "MCU_STATUS", 0x704, true, NULL },
};

/* What mock.rs holds for the G310 (G310 and G310_ASSUMED, driver/rust/src/
 * gpu/mali/mock.rs): `assumed` for the words no public source gave. */
static const struct {
	const char *name;
	uint32_t value;
	bool assumed;
} mock[] = {
	{ "GPU_ID", 0xac740001u, false },     { "L2_FEATURES", 0x07110306u, false },
	{ "TILER_FEATURES", 0x809u, false },  { "MEM_FEATURES", 0x1u, false },
	{ "MMU_FEATURES", 0x2830u, true },    { "AS_PRESENT", 0xffu, true },
	{ "SHADER_PRESENT_LO", 0x3u, true },  { "TILER_PRESENT_LO", 0x1u, true },
	{ "L2_PRESENT_LO", 0x1u, true },
};

static uint32_t words[ARRAY_SIZE(regs)];

static uint32_t word(const char *name)
{
	for (size_t i = 0; i < ARRAY_SIZE(regs); i++) {
		if (strcmp(regs[i].name, name) == 0) {
			return words[i];
		}
	}
	return 0;
}

/* The SM's view of the GPU, read only: PD 14's state, perf 9's level,
 * clock 174's configuration and rate. */
static void sm_state(const char *when)
{
	uint32_t r[3];
	int32_t st;
	int n;

	n = vi95_scmi(0x11, 0x5, (uint32_t[]){ PD_GPU }, 1, r, 1, &st);
	if (n >= 1 && st == 0) {
		VI95_P("%s pd %u state = 0x%08x (%s)", when, PD_GPU, r[0],
		       r[0] == 0 ? "on" : r[0] == 0x40000000u ? "off" : "other");
	} else {
		VI95_P("%s pd %u state = %s (%d), rc %d", when, PD_GPU, vi95_scmi_status(st), st, n);
	}
	n = vi95_scmi(0x13, 0x8, (uint32_t[]){ PERF_GPU }, 1, r, 1, &st);
	if (n >= 1 && st == 0) {
		VI95_P("%s perf %u level = %u", when, PERF_GPU, r[0]);
	} else {
		VI95_P("%s perf %u level = %s (%d), rc %d", when, PERF_GPU, vi95_scmi_status(st), st,
		       n);
	}
	/* CLOCK_CONFIG_GET (0xb), flags 0: attributes, config (bit 0 enabled) */
	n = vi95_scmi(0x14, 0xb, (uint32_t[]){ CLK_GPU_CGC, 0 }, 2, r, 2, &st);
	if (n >= 2 && st == 0) {
		VI95_P("%s clk %u config = 0x%08x (%s)", when, CLK_GPU_CGC, r[1],
		       (r[1] & 1u) ? "enabled" : "disabled");
	} else {
		VI95_P("%s clk %u config = %s (%d), rc %d", when, CLK_GPU_CGC, vi95_scmi_status(st),
		       st, n);
	}
	n = vi95_scmi(0x14, 0x6, (uint32_t[]){ CLK_GPU_CGC }, 1, r, 2, &st);
	if (n >= 2 && st == 0) {
		VI95_P("%s clk %u rate = %llu", when, CLK_GPU_CGC,
		       (unsigned long long)r[1] << 32 | r[0]);
	} else {
		VI95_P("%s clk %u rate = %s (%d), rc %d", when, CLK_GPU_CGC, vi95_scmi_status(st), st,
		       n);
	}
}

static void power_line(const char *what, int rc, const struct yd_imx95_gpu_power *p)
{
	if (rc == 0 && p->on) {
		VI95_P("%s rc 0: level %u (%u kHz), gate %llu Hz, asked %llu Hz", what, p->level_index,
		       p->level_khz, (unsigned long long)p->rate_hz[1] << 32 | p->rate_hz[0],
		       (unsigned long long)p->requested_hz[1] << 32 | p->requested_hz[0]);
	} else if (rc == 0) {
		VI95_P("%s rc 0", what);
	} else {
		VI95_P("%s rc %d: refusal %u, message 0x%04x, status %d (%s), detail 0x%08x%08x", what,
		       rc, p->refusal, p->message, p->status, vi95_scmi_status(p->status),
		       p->detail[1], p->detail[0]);
	}
}

int main(void)
{
	struct yd_imx95_gpu_power on = { 0 }, off = { 0 };
	mm_reg_t gpu;
	int rc_on, rc_off, differ = 0;

	vi95_probe_banner();
	sm_state("before");

	rc_on = yd_imx95_gpu_power_on(verdin_imx95_scmi, 0, &on);
	power_line("power on", rc_on, &on);
	if (rc_on != 0) {
		/* what was switched on stays on; the off is tried, its line says */
		rc_off = yd_imx95_gpu_power_off(verdin_imx95_scmi, &off);
		power_line("power off", rc_off, &off);
		sm_state("after");
		VI95_P("FAIL the GPU's power-on over SCMI stopped (refusal %u, status %s): no GPU "
		       "register was read",
		       on.refusal, vi95_scmi_status(on.status));
		vi95_probe_end();
	}

	sm_state("on");
	device_map(&gpu, DT_REG_ADDR(GPU_NODE), 0x1000, K_MEM_CACHE_NONE);
	for (size_t i = 0; i < ARRAY_SIZE(regs); i++) {
		words[i] = sys_read32(gpu + regs[i].off);
	}
	for (size_t i = 0; i < ARRAY_SIZE(regs); i++) {
		VI95_P("reg 0x%03x %s = 0x%08x%s", regs[i].off, regs[i].name, words[i],
		       regs[i].ours ? "" : " (panthor's, not read by ours)");
	}
	/* The form of mock.rs's G310 table, to paste (strip the prefix). */
	VI95_P("mock pub const G310: [(usize, u32); 9] = [");
	for (size_t i = 0; i < ARRAY_SIZE(regs); i++) {
		if (regs[i].mock != NULL) {
			VI95_P("mock     (%s, 0x%04x_%04x),", regs[i].mock, words[i] >> 16,
			       words[i] & 0xffffu);
		}
	}
	VI95_P("mock ];");
	VI95_P("mock pub const G310_ASSUMED: Assumed = Assumed { mmu_features: 0x%x, as_present: "
	       "0x%x, shader_present: 0x%x, tiler_present: %u, l2_present: %u };",
	       word("MMU_FEATURES"), word("AS_PRESENT"), word("SHADER_PRESENT_LO"),
	       word("TILER_PRESENT_LO"), word("L2_PRESENT_LO"));
	for (size_t i = 0; i < ARRAY_SIZE(mock); i++) {
		uint32_t got = word(mock[i].name);

		differ += got != mock[i].value;
		VI95_P("against mock.rs %s = read 0x%08x, mock 0x%08x (%s, %s)", mock[i].name, got,
		       mock[i].value, mock[i].assumed ? "assumed" : "sourced",
		       got == mock[i].value ? "same" : "DIFFERS");
	}

	/* Off: no GPU register is read after this. */
	rc_off = yd_imx95_gpu_power_off(verdin_imx95_scmi, &off);
	power_line("power off", rc_off, &off);
	sm_state("after");

	if ((word("GPU_ID") >> 16) != 0xac74u) {
		VI95_P("FAIL GPU_ID 0x%08x is not a Mali-G310 (0xac74....)", word("GPU_ID"));
	} else if (rc_off != 0) {
		VI95_P("FAIL the power-off over SCMI stopped (refusal %u, status %s)", off.refusal,
		       vi95_scmi_status(off.status));
	} else {
		VI95_P("PASS GPU_ID 0x%08x, SHADER_PRESENT 0x%08x, %d of %u mock.rs words differ",
		       word("GPU_ID"), word("SHADER_PRESENT_LO"), differ,
		       (unsigned int)ARRAY_SIZE(mock));
	}
	vi95_probe_end();
	return 0;
}
