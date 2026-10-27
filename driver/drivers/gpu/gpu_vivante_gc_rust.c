/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gpu_vivante_gc_rust.c -- the Zephyr side of the Vivante GC7000 kernel side
 * whose register logic is Rust (driver/rust, gpu::vivante; built whenever
 * CONFIG_YAKOGL_DRV_GPU_VIVANTE is, the only kernel side since the C one was
 * deleted).
 *
 * What is here is what only Zephyr can do: the device (DEVICE_DT_INST_DEFINE,
 * device_map of the GPU, GPC and CCM blocks), the interrupt (IRQ_CONNECT to
 * an ISR that calls the Rust handler), the kernel objects (the spinlock
 * around the event bitmap and the interrupt's bookkeeping, the semaphore an
 * event allocation sleeps on), the clock (gc_now_ns) and the log. Every
 * register sequence is a yd_gc_* call. The functions keep gc.h's and
 * gc_internal.h's names and signatures, so gc_render.c and gc_draw.c build
 * unchanged against either implementation.
 */
#define DT_DRV_COMPAT vivante_gc

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/irq.h>
#include <zephyr/logging/log.h>
#include <zephyr/sys/util.h>
#include <errno.h>
#include <string.h>

#include <yakogl_drivers.h>
#include <yakogl_drv/yd_env.h>

#include "gc.h"
#include "gc_internal.h"

LOG_MODULE_REGISTER(gpu_gc, CONFIG_YAKOGL_DRV_GPU_VIVANTE_LOG_LEVEL);

/* struct gc_dev's prefix is the Rust GcCore (src/ffi/vivante.rs asserts the
 * same numbers); struct gc_identity and struct gc_submit are GcIdentity and
 * GcSubmit. */
BUILD_ASSERT(offsetof(struct gc_dev, regs) == 16, "gc_dev.regs");
BUILD_ASSERT(offsetof(struct gc_dev, carveout) == 24, "gc_dev.carveout");
BUILD_ASSERT(offsetof(struct gc_dev, carveout_phys) == 32, "gc_dev.carveout_phys");
BUILD_ASSERT(offsetof(struct gc_dev, carveout_size) == 40, "gc_dev.carveout_size");
BUILD_ASSERT(offsetof(struct gc_dev, mtlb) == 48, "gc_dev.mtlb");
BUILD_ASSERT(offsetof(struct gc_dev, pta) == 56, "gc_dev.pta");
BUILD_ASSERT(offsetof(struct gc_dev, stlb_used) == 64, "gc_dev.stlb_used");
BUILD_ASSERT(offsetof(struct gc_dev, import_va) == 68, "gc_dev.import_va");
BUILD_ASSERT(offsetof(struct gc_dev, mmu_on) == 72, "gc_dev.mmu_on");
BUILD_ASSERT(offsetof(struct gc_dev, ring) == 80, "gc_dev.ring");
BUILD_ASSERT(offsetof(struct gc_dev, ring_gpu) == 88, "gc_dev.ring_gpu");
BUILD_ASSERT(offsetof(struct gc_dev, ring_pos) == 92, "gc_dev.ring_pos");
BUILD_ASSERT(offsetof(struct gc_dev, wait_pos) == 96, "gc_dev.wait_pos");
BUILD_ASSERT(offsetof(struct gc_dev, fe_running) == 100, "gc_dev.fe_running");
BUILD_ASSERT(offsetof(struct gc_dev, event_free) == 104, "gc_dev.event_free");
BUILD_ASSERT(offsetof(struct gc_dev, event_fence) == 108, "gc_dev.event_fence");
BUILD_ASSERT(offsetof(struct gc_dev, fence_next) == 228, "gc_dev.fence_next");
BUILD_ASSERT(offsetof(struct gc_dev, fence_done) == 232, "gc_dev.fence_done");
BUILD_ASSERT(offsetof(struct gc_dev, irq_errors) == 236, "gc_dev.irq_errors");
BUILD_ASSERT(offsetof(struct gc_dev, event_t0_ns) == 272, "gc_dev.event_t0_ns");
BUILD_ASSERT(offsetof(struct gc_dev, perf_last_done_ns) == 512, "gc_dev.perf_last_done_ns");
BUILD_ASSERT(offsetof(struct gc_dev, perf_busy_ns) == 520, "gc_dev.perf_busy_ns");
BUILD_ASSERT(offsetof(struct gc_dev, perf_kicks_done) == 528, "gc_dev.perf_kicks_done");
BUILD_ASSERT(offsetof(struct gc_dev, dump_seen) == 564, "gc_dev.dump_seen");
BUILD_ASSERT(sizeof(struct gc_identity) == 124, "struct gc_identity");
BUILD_ASSERT(offsetof(struct gc_identity, minor_features) == 32, "gc_identity.minor_features");
BUILD_ASSERT(offsetof(struct gc_identity, secure_ahb) == 80, "gc_identity.secure_ahb");
BUILD_ASSERT(offsetof(struct gc_identity, from_table) == 81, "gc_identity.from_table");
BUILD_ASSERT(offsetof(struct gc_identity, stream_count) == 84, "gc_identity.stream_count");
BUILD_ASSERT(offsetof(struct gc_identity, varyings_count) == 120, "gc_identity.varyings_count");
BUILD_ASSERT(sizeof(struct gc_submit) == 12, "struct gc_submit");

#define CORE(g) ((struct yd_gc_core *)(g))

/* The core clock the idle loop's WAIT delay is derived from. */
#define GC_CLOCK_HZ DT_PROP(DT_INST(0, vivante_gc), clock_frequency)

/* ---- the log: the Rust lines under this driver's module and level ---- */

static void gc_env_log(void *ctx, uint32_t level, const uint8_t *buf, size_t len)
{
	char line[161];

	ARG_UNUSED(ctx);
	if (len >= sizeof(line)) {
		len = sizeof(line) - 1;
	}
	memcpy(line, buf, len);
	line[len] = '\0';
	switch (level) {
	case 1:
		LOG_ERR("%s", line);
		break;
	case 2:
		LOG_WRN("%s", line);
		break;
	case 3:
		LOG_INF("%s", line);
		break;
	default:
		LOG_DBG("%s", line);
		break;
	}
}

static struct yd_env gc_env;

static const struct yd_env *env(void)
{
	if (gc_env.log == NULL) {
		yd_env_init(&gc_env, NULL);
		gc_env.log = gc_env_log;
	}
	return &gc_env;
}

/* ---- the device (gc.h) ---- */

struct gc_config {
	uintptr_t reg_phys;
	size_t reg_size;
	uintptr_t gpc_phys;
	size_t gpc_size;
	uintptr_t ccm_phys;
	size_t ccm_size;
	uint64_t carveout_phys;
	size_t carveout_size;
	uint32_t clock_hz;
};

struct gc_data {
	mm_reg_t regs;
	mm_reg_t gpc;
	mm_reg_t ccm;
	struct gc_identity id;
	bool alive;
};

uint32_t gc_read(const struct device *dev, uint32_t reg)
{
	return sys_read32(((struct gc_data *)dev->data)->regs + reg);
}

void gc_write(const struct device *dev, uint32_t reg, uint32_t val)
{
	sys_write32(val, ((struct gc_data *)dev->data)->regs + reg);
}

const struct gc_identity *gc_identity(const struct device *dev)
{
	return &((struct gc_data *)dev->data)->id;
}

bool gc_alive(const struct device *dev)
{
	return ((struct gc_data *)dev->data)->alive;
}

int gc_bringup_reset(mm_reg_t regs, const struct gc_identity *idp)
{
	return yd_gc_reset(regs, env(), idp);
}

static struct gc_data gc_attached;

int gc_bringup_attach(mm_reg_t regs, const struct gc_identity **id_out)
{
	struct gc_data *d = &gc_attached;
	int rc;

	d->regs = regs;
	d->alive = false;
	rc = yd_gc_identify(regs, env(), &d->id);
	if (rc) {
		return rc;
	}
	rc = yd_gc_reset(regs, env(), &d->id);
	if (rc) {
		return rc;
	}
	d->alive = true;
	if (id_out) {
		*id_out = &d->id;
	}
	return 0;
}

/*
 * How far the device init got, reported as it happens: `step` 0 = entered,
 * 1..7 = entering gpu::vivante::power::step `step` of the power-up,
 * GC_INIT_MARK_IDENTIFY / _RESET = entering that call, GC_INIT_MARK_DONE =
 * returned with `value` = rc. A weak no-op here, like gc_render_mark(); a
 * board that keeps a record across the reset overrides it (gc_board's
 * gc_sample.c: the kind STAGE breadcrumbs 0x30 + step of gc_result.h). It
 * must not touch the GPU, the GPC or the CCM.
 */
#define GC_INIT_MARK_IDENTIFY 8u
#define GC_INIT_MARK_RESET    9u
#define GC_INIT_MARK_DONE     10u
__weak void gc_init_mark(unsigned int step, int value)
{
	ARG_UNUSED(step);
	ARG_UNUSED(value);
}

static void gc_power_mark(void *ctx, uint32_t step)
{
	ARG_UNUSED(ctx);
	gc_init_mark(step, 0);
}

static int gc_init_steps(const struct device *dev)
{
	const struct gc_config *cfg = dev->config;
	struct gc_data *d = dev->data;
	int rc;

	device_map(&d->regs, cfg->reg_phys, cfg->reg_size, K_MEM_CACHE_NONE);
	device_map(&d->gpc, cfg->gpc_phys, cfg->gpc_size, K_MEM_CACHE_NONE);
	device_map(&d->ccm, cfg->ccm_phys, cfg->ccm_size, K_MEM_CACHE_NONE);
	gc_init_mark(0, 0);

	rc = yd_gc_power_on(d->ccm, d->gpc, env(), gc_power_mark, NULL);
	if (rc) {
		return rc;
	}
	gc_init_mark(GC_INIT_MARK_IDENTIFY, 0);
	rc = yd_gc_identify(d->regs, env(), &d->id);
	if (rc) {
		return rc;
	}
	gc_init_mark(GC_INIT_MARK_RESET, 0);
	rc = yd_gc_reset(d->regs, env(), &d->id);
	if (rc) {
		return rc;
	}
	d->alive = true;
	gc_dev_publish(dev, d->regs, cfg->carveout_phys, cfg->carveout_size, &d->id);
	LOG_INF("GC%x bring-up milestone 1 complete (Rust; core clock %u Hz, carveout 0x%llx+%zuM)",
		d->id.model, cfg->clock_hz, (unsigned long long)cfg->carveout_phys,
		cfg->carveout_size / (1024 * 1024));
	return 0;
}

static int gc_init(const struct device *dev)
{
	int rc = gc_init_steps(dev);

	gc_init_mark(GC_INIT_MARK_DONE, rc);
	return rc;
}

#define GC_DEFINE(inst)                                                                      \
	static struct gc_data gc_data_##inst;                                                \
	static const struct gc_config gc_config_##inst = {                                   \
		.reg_phys = DT_INST_REG_ADDR_BY_NAME(inst, gpu),                             \
		.reg_size = DT_INST_REG_SIZE_BY_NAME(inst, gpu),                             \
		.gpc_phys = DT_INST_REG_ADDR_BY_NAME(inst, gpc),                             \
		.gpc_size = DT_INST_REG_SIZE_BY_NAME(inst, gpc),                             \
		.ccm_phys = DT_INST_REG_ADDR_BY_NAME(inst, ccm),                             \
		.ccm_size = DT_INST_REG_SIZE_BY_NAME(inst, ccm),                             \
		.carveout_phys = DT_REG_ADDR(DT_INST_PHANDLE(inst, memory_region)),          \
		.carveout_size = DT_REG_SIZE(DT_INST_PHANDLE(inst, memory_region)),          \
		.clock_hz = DT_INST_PROP(inst, clock_frequency),                             \
	};                                                                                   \
	DEVICE_DT_INST_DEFINE(inst, gc_init, NULL, &gc_data_##inst, &gc_config_##inst,       \
			      POST_KERNEL, CONFIG_YAKOGL_DRV_GPU_VIVANTE_INIT_PRIORITY, NULL);

DT_INST_FOREACH_STATUS_OKAY(GC_DEFINE)

/* ---- the rest of gc_internal.h, over gpu::vivante ---- */

uint32_t gc_rd_fe(struct gc_dev *g, uint32_t off)
{
	return yd_gc_rd_fe(CORE(g), off);
}

int gc_hw_init(struct gc_dev *g)
{
	return yd_gc_hw_init(CORE(g), env());
}

int gc_mmu_init(struct gc_dev *g)
{
	return yd_gc_mmu_init(CORE(g), env());
}

int gc_mmu_map(struct gc_dev *g, uint32_t gpu_va, uint64_t phys, size_t size)
{
	return yd_gc_mmu_map(CORE(g), env(), gpu_va, phys, size);
}

int gc_mmu_enable(struct gc_dev *g)
{
	return yd_gc_mmu_enable(CORE(g), env());
}

void gc_mmu_dump(struct gc_dev *g)
{
	yd_gc_mmu_dump(CORE(g), env());
}

void gc_idle_state_decode(uint32_t idle)
{
	yd_gc_idle_state_decode(env(), idle);
}

void gc_fe_start(struct gc_dev *g, uint32_t address, uint32_t prefetch)
{
	yd_gc_fe_start(CORE(g), address, prefetch);
}

int gc_fe_wait_idle(struct gc_dev *g, uint32_t timeout_ms)
{
	return yd_gc_fe_wait_idle(CORE(g), env(), timeout_ms);
}

uint32_t gc_cmd_build_pta_config(struct gc_dev *g, uint32_t pta_index)
{
	return yd_gc_cmd_build_pta_config(CORE(g), pta_index);
}

int gc_ring_init(struct gc_dev *g)
{
	return yd_gc_ring_init(CORE(g), env(), GC_CLOCK_HZ);
}

int gc_ring_submit(struct gc_dev *g, const struct gc_submit *s, uint32_t event, uint32_t fence)
{
	return yd_gc_ring_submit(CORE(g), env(), s, event, fence, GC_CLOCK_HZ);
}

void gc_ring_dump(struct gc_dev *g)
{
	yd_gc_ring_dump(CORE(g), env());
}

/* Claim a free event for `fence`, sleeping on the event semaphore up to
 * `timeout_ms` while all 30 are in flight. The id or a negative errno. */
int gc_event_alloc(struct gc_dev *g, uint32_t fence, uint32_t timeout_ms)
{
	int64_t t0 = k_uptime_get();

	while (true) {
		k_spinlock_key_t key = k_spin_lock(&g->lock);
		int id = yd_gc_event_claim(CORE(g), fence);

		k_spin_unlock(&g->lock, key);
		if (id >= 0) {
			g->wait_ms += (uint32_t)(k_uptime_get() - t0);
			return id;
		}
		if (id != -EAGAIN) {
			LOG_ERR("event: bitmap offered an id the hardware does not have (0x%08x)",
				g->event_free);
			return id;
		}
		if (k_sem_take(&g->event_room, K_MSEC(timeout_ms)) != 0) {
			g->event_busy++;
			g->wait_ms += (uint32_t)(k_uptime_get() - t0);
			LOG_ERR("event: all %u events in flight for %u ms", GC_EVENT_COUNT,
				timeout_ms);
			return -EBUSY;
		}
	}
}

bool gc_irq_handle(struct gc_dev *g)
{
	uint32_t freed = 0;
	k_spinlock_key_t key = k_spin_lock(&g->lock);
	bool ours = yd_gc_irq_handle(CORE(g), gc_now_ns(), &freed);

	k_spin_unlock(&g->lock, key);
	while (freed-- > 0) {
		k_sem_give(&g->event_room);
	}
	return ours;
}

static void gc_isr(const void *arg)
{
	(void)gc_irq_handle((struct gc_dev *)arg);
}

void gc_irq_connect(struct gc_dev *g)
{
	ARG_UNUSED(g);
	IRQ_CONNECT(DT_INST_IRQN(0), DT_INST_IRQ(0, priority), gc_isr, &g_gc,
		    DT_INST_IRQ(0, flags));
	irq_enable(DT_INST_IRQN(0));
	LOG_INF("irq %d connected", DT_INST_IRQN(0));
}
