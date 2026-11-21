/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * imx95_fw: does the Mali-G310's CSF firmware boot from our side, and what
 * does it answer (README.md)? The pre-study's B3: yd_imx95_fw_up runs the
 * GPU's power-on over SCMI, its identity, soft reset and L2, AS0 over the
 * firmware's sections, the load, the MCU's boot, the global configuration
 * and one PING (the NanoPC-T6's Z3 code after the i.MX 95's power-on), then
 * reads CSG 0's control block back. Nothing is started after it: no group,
 * no queue. The firmware is driver/src/fw/mali_csffw_blob.h (made by
 * driver/tools/mkmalifw.sh, never committed), each loaded section backed by
 * static storage as t6_fw's, the shared section __nocache.
 */
#include <zephyr/cache.h>
#include <zephyr/devicetree.h>
#include <zephyr/kernel.h>
#include <zephyr/arch/arm64/lib_helpers.h> /* read_cntfrq_el0; after kernel.h */
#include <zephyr/kernel/mm.h>
#include <zephyr/linker/section_tags.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>

#include <yakogl_drivers.h>

#include "verdin_imx95_scmi.h"
#include "vi95_probe.h"

#include "fw/mali_csffw_blob.h"

#define GPU_NODE DT_NODELABEL(gpu)
BUILD_ASSERT(DT_REG_SIZE(GPU_NODE) >= YD_IMX95_GPU_MAP_MIN, "the GPU node covers the doorbells");

/* AS0's tables: nine pages for the firmware's sections, from sixteen. */
#define POOL_PAGES 16
static uint8_t pool[POOL_PAGES * 4096] __aligned(4096);

/* The board's mali_csffw.bin (arch10.12's file is the bytes of arch10.8's,
 * docs/notes/pre-study-imx95.md section 2): seven loaded sections, as
 * t6_fw backs them, in the order fw::Image::sections() lists them. */
static uint8_t sec0[64 * 1024] __aligned(4096);
static uint8_t sec1[64 * 1024] __aligned(4096);
static uint8_t sec2[64 * 1024] __aligned(4096);
static uint8_t sec3[128 * 1024] __aligned(4096);
static uint8_t sec4[256 * 1024] __aligned(4096);
static uint8_t sec5[256 * 1024] __aligned(4096);
static __nocache uint8_t shared[64 * 1024] __aligned(4096);

static const struct {
	uint32_t va;
	uint8_t *cpu;
	size_t size;
} sections[] = {
	{ 0x00400000, sec1, sizeof(sec1) }, { 0x00410000, sec2, sizeof(sec2) },
	{ 0x00000000, sec0, sizeof(sec0) }, { 0x00800000, sec3, sizeof(sec3) },
	{ 0x02000000, sec5, sizeof(sec5) }, { 0x01000000, sec4, sizeof(sec4) },
	{ 0x04000000, shared, sizeof(shared) },
};
#define SECTIONS ARRAY_SIZE(sections)

static const char *const step_name[] = {
	[YD_T6_FW_STEP_NONE] = "none",
	[YD_T6_FW_STEP_PARSE] = "the image was refused",
	[YD_T6_FW_STEP_PLACE] = "the backings do not fit the sections",
	[YD_T6_FW_STEP_GPU] = "the power-on, identity, reset or L2 stopped",
	[YD_T6_FW_STEP_MMU] = "AS0 stopped",
	[YD_T6_FW_STEP_BOOT] = "csf::boot refused",
	[YD_T6_FW_STEP_CLEAN] = "a cache clean of the load failed",
	[YD_T6_FW_STEP_VERSION] = "the GLB version is not the image's",
};

static void delay_us(uint32_t us)
{
	k_busy_wait(us);
}

static int clean(uintptr_t addr, size_t len)
{
	return sys_cache_data_flush_range((void *)addr, len);
}

static void on_handover(const struct yd_t6_gpu_up *h)
{
	ARG_UNUSED(h); /* WDOG3 is fed by the glue's timer */
}

static void record(const struct yd_t6_fw_up *o, const struct yd_imx95_fw_csg0 *c)
{
	VI95_P("image version 0x%08x, %u sections loaded, %u protected skipped, %u table pages, "
	       "%u flushes",
	       o->image_version, o->sections, o->protected_skipped, o->tables_used, o->flushed);
	VI95_P("GPU_ID 0x%08x SHADER_PRESENT 0x%08x%08x MMU_FEATURES 0x%08x shared PA 0x%08x%08x "
	       "clean rc %d",
	       o->gpu_id, o->shader_present[1], o->shader_present[0], o->mmu_features,
	       o->shared_pa[1], o->shared_pa[0], o->clean_rc);
	VI95_P("MCU_STATUS %u before, %u after, booted in %u us, JOB_INT_RAWSTAT 0x%08x",
	       o->mcu_status_before, o->mcu_status, o->boot_waited_us, o->job_raw);
	VI95_P("glb version = %u.%u.%u (0x%08x)", (o->version >> 24) & 0xff,
	       (o->version >> 16) & 0xff, o->version & 0xffff, o->version);
	VI95_P("glb features = 0x%08x instr 0x%08x", o->features, o->instr_features);
	VI95_P("groups = %u stride 0x%x", o->group_num, o->group_stride);
	VI95_P("csg0 = %u streams stride 0x%x, in 0x%08x out 0x%08x", o->csg0[2], o->csg0[3],
	       o->csg0[0], o->csg0[1]);
	VI95_P("cs0 features = 0x%08x (in 0x%08x out 0x%08x)", o->cs0[2], o->cs0[0], o->cs0[1]);
	if (c->read) {
		VI95_P("csg0 features = 0x%08x", c->features);
		VI95_P("csg0 suspend size = %u protm %u", c->suspend_size, c->protm_suspend_size);
	} else {
		VI95_P("csg0 suspend size = not read (refusal %u, detail 0x%08x)", c->refusal,
		       c->detail);
	}
	VI95_P("CORE_EN 0x%08x%08x, poweroff 0x%08x progress 0x%08x idle 0x%08x, ack_irq_mask "
	       "0x%08x",
	       o->core_en_mask[1], o->core_en_mask[0], o->poweroff_timer, o->progress_timer,
	       o->idle_timer, o->ack_irq_mask);
	VI95_P("init req 0x%08x acked 0x%08x in %u us, PING %s in %u us, GLB req 0x%08x ack 0x%08x",
	       o->init_req, o->acked, o->init_ack_us, o->ping_acked ? "acked" : "NOT acked",
	       o->ping_ack_us, o->glb_req, o->glb_ack);
}

int main(void)
{
	static struct yd_t6_gpu_up z1; /* hundreds of bytes: not on main()'s stack */
	struct yd_t6_fw_backing backings[SECTIONS];
	struct yd_imx95_gpu_power power = { 0 };
	struct yd_imx95_fw_csg0 csg0 = { 0 };
	struct yd_t6_fw_up o = { 0 };
	uint32_t timer_hz = (uint32_t)read_cntfrq_el0();
	mm_reg_t gpu;
	int64_t t0;
	int rc;

	vi95_probe_banner();
	VI95_P("image %u bytes, %u loaded sections, sha256 %s (mkmalifw.sh's), timer %u Hz",
	       (unsigned int)MALI_CSFFW_SIZE, (unsigned int)SECTIONS, MALI_CSFFW_SHA256, timer_hz);
	for (size_t i = 0; i < SECTIONS; i++) {
		backings[i].pa = k_mem_phys_addr(sections[i].cpu);
		backings[i].cpu = (uintptr_t)sections[i].cpu;
		backings[i].size = sections[i].size;
	}
	device_map(&gpu, DT_REG_ADDR(GPU_NODE), DT_REG_SIZE(GPU_NODE), K_MEM_CACHE_NONE);

	t0 = k_uptime_get();
	rc = yd_imx95_fw_up(verdin_imx95_scmi, 0, gpu, DT_REG_SIZE(GPU_NODE), mali_csffw_blob,
			    MALI_CSFFW_SIZE, backings, SECTIONS, pool, POOL_PAGES,
			    k_mem_phys_addr(pool), timer_hz, clean, delay_us, on_handover, &z1,
			    &power, &o, &csg0);
	VI95_P("rc %d step %u, %lld ms", rc, o.step, (long long)(k_uptime_get() - t0));
	if (power.on) {
		VI95_P("power on: level %u (%u kHz), gate %llu Hz", power.level_index,
		       power.level_khz, (unsigned long long)power.rate_hz[1] << 32 | power.rate_hz[0]);
	} else if (z1.stop == YD_IMX95_GPU_STOP_SCMI) {
		VI95_P("power on stopped: refusal %u, message 0x%04x, status %d (%s), detail "
		       "0x%08x%08x",
		       power.refusal, power.message, power.status, vi95_scmi_status(power.status),
		       power.detail[1], power.detail[0]);
	}

	if (rc == -EINVAL) {
		VI95_P("FAIL yd_imx95_fw_up refused its arguments");
	} else if (o.step == YD_T6_FW_STEP_NONE) {
		record(&o, &csg0);
		VI95_P("PASS GLB %u.%u.%u, %u groups, CS features 0x%08x, suspend %u/%u",
		       (o.version >> 24) & 0xff, (o.version >> 16) & 0xff, o.version & 0xffff,
		       o.group_num, o.cs0[2], csg0.suspend_size, csg0.protm_suspend_size);
	} else {
		VI95_P("stop %u refusal %u detail 0x%08x%08x acked 0x%08x", o.stop, o.refusal,
		       o.detail[1], o.detail[0], o.acked);
		if (o.step == YD_T6_FW_STEP_GPU) {
			VI95_P("GPU_ID 0x%08x, reset raw 0x%08x -> 0x%08x in %u us, L2 ready 0x%08x",
			       z1.ident[0], z1.reset[0], z1.reset[1], z1.reset[2], z1.l2_ready[0]);
		}
		if (o.step >= YD_T6_FW_STEP_BOOT) {
			record(&o, &csg0);
		}
		VI95_P("FAIL step %u (%s)", o.step,
		       o.step < ARRAY_SIZE(step_name) ? step_name[o.step] : "unknown");
	}
	vi95_probe_end();
	return 0;
}
