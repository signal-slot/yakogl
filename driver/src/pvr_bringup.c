/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: Imagination Technologies Ltd.
 * The register definitions are the TI/Img Rogue DDK 25.2's (vendored
 * unmodified in third_party/ti-img-rogue-ddk-25.2, Dual MIT/GPLv2, used under
 * MIT: rgx_cr_defs_km.h); the GPU's reset is the DDK's own RGXStart
 * (pvr_fw_boot.c). This file began as a port of Linux
 * drivers/gpu/drm/imagination; the definitions it took from there were
 * replaced by the DDK's (docs/source-form.md, step 2) and its reset staging,
 * a hand copy of RGXResetSequence, is gone (step 7). Our code is Apache-2.0.
 * See NOTICE.
 *
 * PowerVR BXS-4-64 (Rogue Volcanic) GPU bring-up for Zephyr on TI AM62P.
 * See docs/gpu-port-plan.md for the milestone plan.
 *
 * Milestone 1 (this file): power + clock the GPU through TISCI, map its
 * register block, and read back the core BVNC to prove the GPU is alive.
 * BVNC for the AM62P part is 36.53.104.796 (BXS-4-64 MC1).
 *
 * Later milestones (GPU MMU, RISC-V firmware load, boot handshake, job submit,
 * per-context isolation, Slint-on-GPU) build on this device.
 */

#define DT_DRV_COMPAT img_img_rogue

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>
#include <zephyr/logging/log.h>
#include <zephyr/drivers/firmware/tisci/tisci.h>
#include <errno.h>

#include "pvr_ddk.h"
#include "pvr_internal.h"

LOG_MODULE_REGISTER(gpu_pvr, CONFIG_SAFEUI_GPU_PVR_LOG_LEVEL);

/*
 * Registers: rgx_cr_defs_km.h / rgxdefs_km.h of the vendored DDK
 * (driver/compat/pvr_ddk.h).
 *
 * RGX_CR_CORE_ID__PBVNC (0x0020; it aliases RGX_CR_CORE_REVISION at the same
 * offset; on a PBVNC core B != 0 so the packed 64-bit read is the right one)
 * decodes as B.V.N.C in four 16-bit fields, 36.53.104.796 for this part
 * (RGX_BVNC_KM_* of cores/rgxcore_km_36.53.104.796.h).
 *
 * This is a hardware register (not firmware-served): readable once the GPU is
 * powered, clocked and out of reset, before any firmware boot.
 */
#define PVR_CORE_ID_FIELD_MASK 0xffffULL
#define PVR_BXS_4_64_PBVNC                                              \
	(((uint64_t)RGX_BVNC_KM_B << RGX_CR_CORE_ID__PBVNC__BRANCH_ID_SHIFT) |  \
	 ((uint64_t)RGX_BVNC_KM_V << RGX_CR_CORE_ID__PBVNC__VERSION_ID_SHIFT) | \
	 ((uint64_t)RGX_BVNC_KM_N << RGX_CR_CORE_ID__PBVNC__NUMBER_OF_SCALABLE_UNITS_SHIFT) | \
	 ((uint64_t)RGX_BVNC_KM_C << RGX_CR_CORE_ID__PBVNC__CONFIG_ID_SHIFT))

struct pvr_config {
	uintptr_t reg_phys;
	size_t    reg_size;
	uint64_t  carveout_phys;   /* the GPU-visible DDR carveout (memory-region) */
	size_t    carveout_size;
	const struct device *tisci;
	const uint32_t *pd_ids;
	size_t pd_count;
	uint32_t clk_dev_id;
	uint32_t clk_id;
	uint64_t clk_hz;
};

struct pvr_data {
	mm_reg_t regs;   /* mapped GPU register base */
	bool     powered;
};

/* pvr_rd32/wr32/rd64/wr64 register helpers live in pvr_internal.h */

/*
 * Milestone 2, the reset staging before the firmware load, is gone (step 7 of
 * docs/source-form.md): it was RGXResetSequence copied by hand, plus a
 * SOFT_RESET2 pulse the DDK does not make. RGXStart (pvr_fw_boot.c) runs
 * the DDK's reset sequence itself before it releases the firmware processor,
 * and on Linux the DDK does nothing else between power-up and RGXStart.
 */

static int pvr_power_on(const struct pvr_config *cfg)
{
	int rc;

	if (cfg->tisci == NULL) {
		LOG_ERR("no TISCI phandle: cannot power the GPU");
		return -ENODEV;
	}
	if (!device_is_ready(cfg->tisci)) {
		LOG_ERR("TISCI device not ready");
		return -ENODEV;
	}

	/* Enable every GPU power domain (AM62P: 237 GPU + 242 GPUVD). */
	for (size_t i = 0; i < cfg->pd_count; i++) {
		rc = tisci_cmd_get_device_exclusive(cfg->tisci, cfg->pd_ids[i]);
		if (rc != 0) {
			LOG_ERR("TISCI: power domain %u on failed (%d)",
				cfg->pd_ids[i], rc);
			return rc;
		}
		LOG_INF("TISCI: power domain %u on", cfg->pd_ids[i]);
	}

	/* Get control of and enable the GPU core clock, then set 800 MHz.
	 * Mirrors the reference driver's manual power path (core clk up ->
	 * short delay -> reset deassert) — see gpu-bringup-spec.md M1.
	 *
	 * Non-fatal: on K3 the power domain already gates the GPU core clock,
	 * so it runs at its default rate once PD 237/242 are on. If the request
	 * fails we still proceed to the BVNC read: a correct read-back proves
	 * the clock is live; a garbage read tells us the clock really is off.
	 * (The NACK -19 this used to get was not a refusal: Zephyr's secure
	 * proxy driver swapped clk_id and request_state on the way out, see
	 * SAFEUI_MBOX_TI_SECURE_PROXY_TAIL_FIX. Host 12 owns this clock, as in Linux.)
	 */
	rc = tisci_cmd_get_clock(cfg->tisci, cfg->clk_dev_id, cfg->clk_id,
				 false /* needs_ssc */, false /* can change freq */,
				 false /* enable_input_term */);
	if (rc != 0) {
		LOG_WRN("TISCI: clock %u/%u get failed (%d); "
			"assuming PD-gated default clock",
			cfg->clk_dev_id, cfg->clk_id, rc);
	} else {
		rc = tisci_cmd_clk_set_freq(cfg->tisci, cfg->clk_dev_id, cfg->clk_id,
					    cfg->clk_hz, cfg->clk_hz, cfg->clk_hz);
		if (rc != 0) {
			LOG_WRN("TISCI: clock %u/%u -> %llu Hz failed (%d); "
				"continuing at default rate",
				cfg->clk_dev_id, cfg->clk_id, cfg->clk_hz, rc);
		} else {
			LOG_INF("TISCI: GPU clock %u/%u = %llu Hz",
				cfg->clk_dev_id, cfg->clk_id, cfg->clk_hz);
		}
	}

	{
		uint64_t hz = 0;

		if (tisci_cmd_clk_get_freq(cfg->tisci, cfg->clk_dev_id, cfg->clk_id, &hz) == 0) {
			LOG_INF("TISCI: GPU clock %u/%u runs at %llu Hz", cfg->clk_dev_id,
				cfg->clk_id, (unsigned long long)hz);
		}
	}

	k_busy_wait(1); /* let the core clock settle before reset deassert */

	/* Deassert any module-local resets (reset_state 0 = none asserted). */
	rc = tisci_cmd_set_device_resets(cfg->tisci, cfg->clk_dev_id, 0);
	if (rc != 0) {
		LOG_WRN("TISCI: reset deassert on %u failed (%d)",
			cfg->clk_dev_id, rc);
	}

	return 0;
}

static int pvr_read_bvnc(struct pvr_data *data)
{
	uint64_t core_id = pvr_rd64(data->regs, RGX_CR_CORE_ID__PBVNC);
	uint32_t b = (core_id >> RGX_CR_CORE_ID__PBVNC__BRANCH_ID_SHIFT) & PVR_CORE_ID_FIELD_MASK;
	uint32_t v = (core_id >> RGX_CR_CORE_ID__PBVNC__VERSION_ID_SHIFT) & PVR_CORE_ID_FIELD_MASK;
	uint32_t n = (core_id >> RGX_CR_CORE_ID__PBVNC__NUMBER_OF_SCALABLE_UNITS_SHIFT) & PVR_CORE_ID_FIELD_MASK;
	uint32_t c = (core_id >> RGX_CR_CORE_ID__PBVNC__CONFIG_ID_SHIFT) & PVR_CORE_ID_FIELD_MASK;

	LOG_INF("CORE_ID = 0x%016llx -> BVNC %u.%u.%u.%u", core_id, b, v, n, c);

	if (core_id == 0 || core_id == ~0ULL) {
		LOG_ERR("CORE_ID reads all-%s: GPU not powered/clocked or regs unmapped",
			core_id ? "ones" : "zeros");
		return -EIO;
	}

	if (b == RGX_BVNC_KM_B && v == RGX_BVNC_KM_V &&
	    n == RGX_BVNC_KM_N && c == RGX_BVNC_KM_C) {
		LOG_INF("PowerVR BXS-4-64 MC1 alive (BVNC %u.%u.%u.%u)", b, v, n, c);
	} else {
		LOG_WRN("unexpected BVNC %u.%u.%u.%u (expected %u.%u.%u.%u); "
			"check CORE_ID layout for this core",
			b, v, n, c, RGX_BVNC_KM_B, RGX_BVNC_KM_V,
			RGX_BVNC_KM_N, RGX_BVNC_KM_C);
	}

	return 0;
}

/* The carveout's physical base (pvr_internal.h PVR_GPU_MEM_PHYS): the Linux
 * harness allocates it at runtime; here it is the device tree's memory-region. */
unsigned long long pvr_carveout_phys;
size_t pvr_carveout_size;

static int pvr_init(const struct device *dev)
{
	const struct pvr_config *cfg = dev->config;
	struct pvr_data *data = dev->data;
	int rc;

	if (cfg->carveout_size < PVR_GPU_MEM_MIN_SIZE || cfg->carveout_size > 0xF0000000UL) {
		LOG_ERR("memory-region is %zu MiB; the GPU carveout needs %u MiB .. 3.75 GiB",
			cfg->carveout_size / (1024 * 1024), (unsigned)(PVR_GPU_MEM_MIN_SIZE / (1024 * 1024)));
		return -EINVAL;
	}
	pvr_carveout_phys = cfg->carveout_phys;
	pvr_carveout_size = cfg->carveout_size;

	rc = pvr_power_on(cfg);
	if (rc != 0) {
		return rc;
	}
	data->powered = true;

	/* A53 has an MMU: map the GPU register block (device, non-cached). */
	device_map(&data->regs, cfg->reg_phys, cfg->reg_size, K_MEM_CACHE_NONE);
	LOG_INF("GPU regs %#lx (%zu KiB) mapped at %#lx",
		(unsigned long)cfg->reg_phys, cfg->reg_size / 1024,
		(unsigned long)data->regs);

	rc = pvr_read_bvnc(data);
	if (rc != 0) {
		return rc;
	}

	LOG_INF("GPU bring-up milestone 1 complete (power + BVNC readback)");

	/* The core clock as the GPU sees it: RGX_CR_TIMER (0x0160) is a free-running
	 * 48-bit counter advancing once every 256 core clock cycles (DDK
	 * RGX_CRTIME_TICK_IN_CYCLES). TISCI refuses to tell this host the clock
	 * rate, so measure it over 100 ms. */
	{
		uint64_t t0 = pvr_rd64(data->regs, 0x0160) & 0xFFFFFFFFFFFFULL, t1;

		k_busy_wait(100000);
		t1 = pvr_rd64(data->regs, 0x0160) & 0xFFFFFFFFFFFFULL;
		LOG_INF("GPU core clock measured: %llu MHz (CR_TIMER %llu ticks in 100 ms)",
			(unsigned long long)((t1 - t0) * 256ULL * 10ULL / 1000000ULL),
			(unsigned long long)(t1 - t0));
	}

	/* M3+M4: build the GPU MMU, load the RISC-V firmware and boot it.
	 * Non-fatal to the device init: if the FW does not boot we still keep
	 * the GPU powered so the failure can be inspected. */
	rc = pvr_fw_boot(data->regs);
	if (rc != 0) {
		LOG_ERR("FW boot failed (%d); GPU left powered for inspection", rc);
	}

	return 0;
}

#define PVR_PD_IDS(inst) \
	static const uint32_t pvr_pd_ids_##inst[] = DT_INST_PROP(inst, tisci_power_domain_ids);

#define PVR_INIT(inst)                                                                       \
	PVR_PD_IDS(inst)                                                                     \
	static struct pvr_data pvr_data_##inst;                                              \
	static const struct pvr_config pvr_cfg_##inst = {                                    \
		.reg_phys = DT_INST_REG_ADDR(inst),                                          \
		.reg_size = DT_INST_REG_SIZE(inst),                                          \
		.carveout_phys = DT_REG_ADDR(DT_INST_PHANDLE(inst, memory_region)),          \
		.carveout_size = DT_REG_SIZE(DT_INST_PHANDLE(inst, memory_region)),          \
		.tisci = DEVICE_DT_GET_OR_NULL(DT_INST_PHANDLE(inst, tisci)),                \
		.pd_ids = pvr_pd_ids_##inst,                                                 \
		.pd_count = ARRAY_SIZE(pvr_pd_ids_##inst),                                   \
		.clk_dev_id = DT_INST_PROP(inst, tisci_clock_dev_id),                        \
		.clk_id = DT_INST_PROP(inst, tisci_clock_id),                               \
		.clk_hz = DT_INST_PROP(inst, clock_frequency),                               \
	};                                                                                   \
	DEVICE_DT_INST_DEFINE(inst, pvr_init, NULL, &pvr_data_##inst, &pvr_cfg_##inst,        \
			      POST_KERNEL, CONFIG_SAFEUI_GPU_PVR_INIT_PRIORITY, NULL);

DT_INST_FOREACH_STATUS_OKAY(PVR_INIT)
