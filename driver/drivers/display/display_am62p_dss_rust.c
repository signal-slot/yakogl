/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * display_am62p_dss_rust.c -- TI AM62P DSS1 (yakogl,am62p-dss) as a Zephyr
 * display device, with the register logic in Rust: registration, the
 * yakogl_drv/am62p_dss.h API, the interrupt, the TISCI power and clock requests
 * and the safety layer's buffers stay here; every DSS register access is
 * driver/rust/src/soc/am62p/dss.rs over ip/ti_dss7.rs (written from the
 * AM62Px TRM, SPRUJ83D; libyakogl_drivers.a). It replaced the GPL-derived
 * display_am62p_dss.c (deleted 2026-10-02 after the board run) and keeps its
 * API and behaviour: the two-plane safety layer, the shared GO, the
 * pending/shown sequence numbers, the vsync callback, the statistics, the
 * __noinit buffers and the TISCI clock tolerance.
 *
 * Design and board plan: driver/docs/am62p-display-cold.md.
 */
#define DT_DRV_COMPAT yakogl_am62p_dss

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/display.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/util.h>
#include <zephyr/cache.h>
#include <zephyr/logging/log.h>
#include <yakogl_drv/am62p_dss.h>
#include <yakogl_drv/tisci_poll.h>
#include <yakogl_drivers.h>
#include <yakogl_drv/yd_env.h>
#include <string.h>
#include <errno.h>

LOG_MODULE_REGISTER(am62p_dss, CONFIG_YAKOGL_DRV_DSS_LOG_LEVEL);

BUILD_ASSERT(AM62P_DSS_FMT_RGB565 == 0x3 && AM62P_DSS_FMT_ARGB8888 == 0x7 &&
	     AM62P_DSS_FMT_XRGB8888 == 0x27, "DISPC_VID_ATTRIBUTES.FORMAT codes (SPRUJ83D 12.9.1.4.1.7)");

/* reg-names of the binding */
enum { R_COMMON, R_VID, R_VIDL1, R_OVR, R_VP, NREGS };

struct dss_config {
	uintptr_t phys[NREGS];   /* R_OVR, R_VP unused: ovr_vp[] by video port */
	uintptr_t ovr_vp[2][2];  /* {ovr1, vp1}, {ovr2, vp2} */
	size_t size;
	struct yd_am62p_dss_config yd;
	const struct device *tisci;
	uint32_t dev_id;
	const uint16_t *aux_dev_ids; /* other TISCI devices the DSS needs on */
	size_t num_aux_dev_ids;
	uint8_t fck_clk_id;
	uint8_t vp_clk_id;
	uint8_t vp_clk_div;
	void (*irq_config)(const struct device *dev);
};

struct dss_data {
	struct yd_am62p_dss_data yd;
	struct yd_env env;
	struct k_sem vsync;
	am62p_dss_vsync_cb_t vsync_cb;
	void *vsync_user;
};

/* Safety layer framebuffer: ARGB8888, in DDR so the DSS can fetch it.
 * Double-buffered: the renderer draws into the back buffer and flips at GO,
 * so the plane never scans a half-drawn frame. The two buffers dominate
 * .bss (~2.4 MB each); __noinit keeps z_bss_zero from clearing ~4.8 MB of
 * DDR at boot (that memset, before caches are effective, cost ~3 s of the
 * R5F start-up). Init clears them itself.
 */
#define SAFETY_W DT_INST_PROP(0, width)
#define SAFETY_H DT_INST_PROP(0, height)
static uint32_t safety_fb0[SAFETY_W * SAFETY_H] __noinit __aligned(64);
static uint32_t safety_fb1[SAFETY_W * SAFETY_H] __noinit __aligned(64);

static uint32_t ld(const uint32_t *p)
{
	return __atomic_load_n(p, __ATOMIC_SEQ_CST);
}

static void dss_isr(const struct device *dev)
{
	const struct dss_config *cfg = dev->config;
	struct dss_data *data = dev->data;
	uint32_t ev = yd_am62p_dss_isr(&cfg->yd, &data->yd);

	if (ev & YD_AM62P_DSS_EV_VSYNC) {
		k_sem_give(&data->vsync);
		if (data->vsync_cb) {
			data->vsync_cb(dev, data->vsync_user);
		}
	}
}

/* ---------------- public API (yakogl_drv/am62p_dss.h) ---------------- */

void *am62p_dss_safety_buffer(const struct device *dev, uint32_t *width, uint32_t *height,
			      uint32_t *stride_bytes)
{
	struct dss_data *data = dev->data;

	*width = SAFETY_W;
	*height = SAFETY_H;
	*stride_bytes = SAFETY_W * 4;
	return (void *)(uintptr_t)yd_am62p_dss_safety_back(&data->yd);
}

int am62p_dss_set_app_buffer(const struct device *dev, const struct am62p_dss_plane_cfg *cfg)
{
	struct dss_data *data = dev->data;
	struct yd_am62p_dss_plane p = {
		.seq = cfg->seq,
		.format = cfg->format,
		.addr = (uint64_t)cfg->pa,
		.width = cfg->width,
		.height = cfg->height,
		.stride_bytes = cfg->stride_bytes,
	};

	return yd_am62p_dss_set_app_buffer(&data->yd, &p);
}

uint32_t am62p_dss_app_busy(const struct device *dev, const uint64_t *pa, size_t n)
{
	struct dss_data *data = dev->data;

	return yd_am62p_dss_app_busy(&data->yd, pa, n);
}

int am62p_dss_enable_app_plane(const struct device *dev, bool enable)
{
	struct dss_data *data = dev->data;

	return yd_am62p_dss_enable_app_plane(&data->yd, enable);
}

int am62p_dss_safety_premultiplied(const struct device *dev, bool on)
{
	struct dss_data *data = dev->data;

	yd_am62p_dss_safety_premultiplied(&data->yd, on);
	return 0;
}

int am62p_dss_safety_flip(const struct device *dev)
{
	struct dss_data *data = dev->data;

	yd_am62p_dss_safety_flip(&data->yd);
	return 0;
}

bool am62p_dss_safety_back_free(const struct device *dev)
{
	struct dss_data *data = dev->data;

	return ld(&data->yd.safety_flip_pending) == 0;
}

int am62p_dss_output_enable(const struct device *dev, bool enable)
{
	struct dss_data *data = dev->data;

	yd_am62p_dss_output_enable(&data->yd, enable);
	return 0;
}

uint32_t am62p_dss_app_seq_shown(const struct device *dev)
{
	struct dss_data *data = dev->data;

	return ld(&data->yd.app_seq_shown);
}

void am62p_dss_set_vsync_callback(const struct device *dev, am62p_dss_vsync_cb_t cb, void *user)
{
	struct dss_data *data = dev->data;
	unsigned int key = irq_lock();

	data->vsync_user = user;
	data->vsync_cb = cb;
	irq_unlock(key);
}

int am62p_dss_wait_vsync(const struct device *dev, k_timeout_t timeout)
{
	const struct dss_config *cfg = dev->config;
	struct dss_data *data = dev->data;

	if (cfg->yd.flags & YD_AM62P_DSS_HAS_IRQ) {
		return k_sem_take(&data->vsync, timeout);
	}
	/* polling fallback */
	int64_t end = k_uptime_get() + k_ticks_to_ms_ceil64(timeout.ticks);

	while (k_uptime_get() < end) {
		if (yd_am62p_dss_poll_vsync(&cfg->yd, &data->yd)) {
			return 0;
		}
		k_sleep(K_MSEC(1));
	}
	return -EAGAIN;
}

void am62p_dss_get_stats(const struct device *dev, struct am62p_dss_stats *out)
{
	struct dss_data *data = dev->data;
	const struct yd_am62p_dss_stats *s = &data->yd.stats;

	*out = (struct am62p_dss_stats){
		.vsyncs = ld(&s->vsyncs),
		.app_frames = ld(&s->app_frames),
		.underflows = ld(&s->underflows),
		.sync_lost = ld(&s->sync_lost),
		.go_timeouts = ld(&s->go_timeouts),
		.go_waits = ld(&s->go_waits),
		.safety_flips = ld(&s->safety_flips),
		.app_plane_enabled = ld(&data->yd.app_enabled) != 0,
	};
}

/* ---------------- Zephyr display API (safety layer) ---------------- */

static int dss_write(const struct device *dev, const uint16_t x, const uint16_t y,
		     const struct display_buffer_descriptor *desc, const void *buf)
{
	struct dss_data *data = dev->data;
	uint32_t *fb = (uint32_t *)(uintptr_t)yd_am62p_dss_safety_front(&data->yd);
	const uint32_t *src = buf;

	if (x + desc->width > SAFETY_W || y + desc->height > SAFETY_H) {
		return -EINVAL;
	}
	for (uint32_t row = 0; row < desc->height; row++) {
		memcpy(&fb[(y + row) * SAFETY_W + x], src + row * desc->pitch, desc->width * 4);
	}
	sys_cache_data_flush_range(fb, sizeof(safety_fb0));
	return 0;
}

static int dss_blanking_off(const struct device *dev)
{
	return am62p_dss_output_enable(dev, true);
}

static int dss_blanking_on(const struct device *dev)
{
	return am62p_dss_output_enable(dev, false);
}

static void dss_get_capabilities(const struct device *dev, struct display_capabilities *caps)
{
	ARG_UNUSED(dev);
	memset(caps, 0, sizeof(*caps));
	caps->x_resolution = SAFETY_W;
	caps->y_resolution = SAFETY_H;
	caps->supported_pixel_formats = PIXEL_FORMAT_ARGB_8888;
	caps->current_pixel_format = PIXEL_FORMAT_ARGB_8888;
	caps->current_orientation = DISPLAY_ORIENTATION_NORMAL;
}

static int dss_set_pixel_format(const struct device *dev, const enum display_pixel_format pf)
{
	ARG_UNUSED(dev);
	return pf == PIXEL_FORMAT_ARGB_8888 ? 0 : -ENOTSUP;
}

static DEVICE_API(display, dss_api) = {
	.blanking_on = dss_blanking_on,
	.blanking_off = dss_blanking_off,
	.write = dss_write,
	.get_capabilities = dss_get_capabilities,
	.set_pixel_format = dss_set_pixel_format,
};

/* ---------------- init ---------------- */

static int dss_power_and_clocks(const struct device *dev)
{
	const struct dss_config *cfg = dev->config;
	uint32_t pixel_hz = cfg->yd.pixel_clock_hz;
	int rc;

	if (cfg->tisci == NULL) {
		LOG_WRN("no TISCI: assuming DSS already powered and clocked");
		return 0;
	}
	if (!device_is_ready(cfg->tisci)) {
		return -ENODEV;
	}
	rc = yakogl_drv_tisci_device_ensure_on(cfg->tisci, cfg->dev_id);
	if (rc) {
		LOG_ERR("TISCI: device %u on failed (%d)", cfg->dev_id, rc);
		return rc;
	}
	for (size_t i = 0; i < cfg->num_aux_dev_ids; i++) {
		rc = yakogl_drv_tisci_device_ensure_on(cfg->tisci, cfg->aux_dev_ids[i]);
		if (rc) {
			LOG_ERR("TISCI: device %u on failed (%d)", cfg->aux_dev_ids[i], rc);
			return rc;
		}
	}
	uint64_t target = (uint64_t)pixel_hz * cfg->vp_clk_div;

	rc = yakogl_drv_tisci_clock_set_freq(cfg->tisci, cfg->dev_id, cfg->vp_clk_id,
					 target - target / 20, target, target + target / 20);
	if (rc) {
		/* Another host (Linux) may own the clock: accept it if it already runs
		 * within 5 % of what we need.
		 */
		uint64_t cur = 0;

		if (yakogl_drv_tisci_clock_get_freq(cfg->tisci, cfg->dev_id, cfg->vp_clk_id, &cur) == 0 &&
		    cur >= target - target / 20 && cur <= target + target / 20) {
			LOG_WRN("TISCI: clock %u.%u not settable (%d), already at %llu Hz",
				cfg->dev_id, cfg->vp_clk_id, rc, (unsigned long long)cur);
		} else {
			LOG_ERR("TISCI: clock %u.%u -> %llu Hz failed (%d)", cfg->dev_id,
				cfg->vp_clk_id, (unsigned long long)target, rc);
			return rc;
		}
	}
	(void)yakogl_drv_tisci_clock_set_state(cfg->tisci, cfg->dev_id, cfg->fck_clk_id, true);
	(void)yakogl_drv_tisci_clock_set_state(cfg->tisci, cfg->dev_id, cfg->vp_clk_id, true);
	uint64_t actual = 0;

	if (yakogl_drv_tisci_clock_get_freq(cfg->tisci, cfg->dev_id, cfg->vp_clk_id, &actual) == 0) {
		LOG_INF("VP clock source %llu Hz (pixel clock %llu Hz)", (unsigned long long)actual,
			(unsigned long long)(actual / cfg->vp_clk_div));
	}
	return 0;
}

static int dss_init(const struct device *dev)
{
	const struct dss_config *cfg = dev->config;
	struct dss_data *data = dev->data;
	uintptr_t *base[NREGS] = {
		[R_COMMON] = &data->yd.common, [R_VID] = &data->yd.vid, [R_VIDL1] = &data->yd.vidl1,
		[R_OVR] = &data->yd.ovr,       [R_VP] = &data->yd.vp,
	};
	int rc;

	for (int i = 0; i < NREGS; i++) {
		uintptr_t phys = cfg->phys[i];

		if (i == R_OVR || i == R_VP) {
			phys = cfg->ovr_vp[cfg->yd.vp_index][i == R_VP];
		}
#ifdef CONFIG_MMU
		device_map(base[i], phys, cfg->size, K_MEM_CACHE_NONE);
#else
		*base[i] = phys;
#endif
	}
	yd_env_init(&data->env, (void *)dev);
	k_sem_init(&data->vsync, 0, 1);
	memset(safety_fb0, 0, sizeof(safety_fb0));
	memset(safety_fb1, 0, sizeof(safety_fb1));
	/* Both buffers are scanned out from DDR: the zeroes must leave the CPU
	 * cache, or whatever the previous OS left at these addresses shows. */
	sys_cache_data_flush_range(safety_fb0, sizeof(safety_fb0));
	sys_cache_data_flush_range(safety_fb1, sizeof(safety_fb1));
	data->yd.safety_buf[0] = (uint64_t)(uintptr_t)safety_fb0;
	data->yd.safety_buf[1] = (uint64_t)(uintptr_t)safety_fb1;

	rc = dss_power_and_clocks(dev);
	if (rc) {
		return rc;
	}
	rc = yd_am62p_dss_init(&cfg->yd, &data->yd, &data->env);
	if (rc) {
		LOG_ERR("DSS init failed (%d)", rc);
		return rc;
	}
	if (cfg->yd.flags & YD_AM62P_DSS_HAS_IRQ) {
		cfg->irq_config(dev);
	}
	return 0;
}

#define DSS_IRQ_CONFIG(inst)                                                                       \
	static void dss_irq_config_##inst(const struct device *dev)                                \
	{                                                                                          \
		IF_ENABLED(DT_INST_IRQ_HAS_IDX(inst, 0), (                                         \
			IRQ_CONNECT(DT_INST_IRQN(inst), DT_INST_IRQ(inst, priority), dss_isr,      \
				    DEVICE_DT_INST_GET(inst), DT_INST_IRQ(inst, flags));           \
			irq_enable(DT_INST_IRQN(inst));))                                          \
		ARG_UNUSED(dev);                                                                   \
	}

#define DSS_FLAG(inst, prop, bit) (DT_INST_PROP(inst, prop) ? (bit) : 0)

#define DSS_DEFINE(inst)                                                                           \
	BUILD_ASSERT(DT_INST_PROP(inst, video_port) == 1 || DT_INST_PROP(inst, video_port) == 2,  \
		     "video-port is 1 or 2");                                                      \
	DSS_IRQ_CONFIG(inst)                                                                       \
	static const uint16_t dss_aux_##inst[] = DT_INST_PROP_OR(inst, tisci_aux_device_ids, {}); \
	static const struct dss_config dss_cfg_##inst = {                                          \
		.phys = {                                                                          \
			[R_COMMON] = DT_INST_REG_ADDR_BY_NAME(inst, common),                       \
			[R_VID] = DT_INST_REG_ADDR_BY_NAME(inst, vid),                             \
			[R_VIDL1] = DT_INST_REG_ADDR_BY_NAME(inst, vidl1),                         \
		},                                                                                 \
		.ovr_vp = {                                                                        \
			{DT_INST_REG_ADDR_BY_NAME(inst, ovr1), DT_INST_REG_ADDR_BY_NAME(inst, vp1)}, \
			{DT_INST_REG_ADDR_BY_NAME(inst, ovr2), DT_INST_REG_ADDR_BY_NAME(inst, vp2)}, \
		},                                                                                 \
		.size = DT_INST_REG_SIZE_BY_NAME(inst, common),                                    \
		.yd = {                                                                            \
			.pixel_clock_hz = DT_INST_PROP(inst, pixel_clock_hz),                      \
			.hactive = DT_INST_PROP(inst, width),                                      \
			.hfp = DT_INST_PROP(inst, hfront_porch),                                   \
			.hsync = DT_INST_PROP(inst, hsync_len),                                    \
			.hbp = DT_INST_PROP(inst, hback_porch),                                    \
			.vactive = DT_INST_PROP(inst, height),                                     \
			.vfp = DT_INST_PROP(inst, vfront_porch),                                   \
			.vsync = DT_INST_PROP(inst, vsync_len),                                    \
			.vbp = DT_INST_PROP(inst, vback_porch),                                    \
			.flags = DSS_FLAG(inst, hsync_active_low, YD_AM62P_DSS_HSYNC_LOW) |        \
				 DSS_FLAG(inst, vsync_active_low, YD_AM62P_DSS_VSYNC_LOW) |        \
				 DSS_FLAG(inst, de_active_low, YD_AM62P_DSS_DE_LOW) |              \
				 DSS_FLAG(inst, pixclk_negedge, YD_AM62P_DSS_PIXCLK_NEGEDGE) |     \
				 (DT_INST_IRQ_HAS_IDX(inst, 0) ? YD_AM62P_DSS_HAS_IRQ : 0),        \
			.vp_index = DT_INST_PROP(inst, video_port) - 1,                            \
			.safety_width = DT_INST_PROP(inst, width),                                 \
			.safety_height = DT_INST_PROP(inst, height),                               \
		},                                                                                 \
		.tisci = DEVICE_DT_GET_OR_NULL(DT_INST_PHANDLE(inst, tisci)),                      \
		.dev_id = DT_INST_PROP(inst, tisci_device_id),                                     \
		.aux_dev_ids = dss_aux_##inst,                                                     \
		.num_aux_dev_ids = ARRAY_SIZE(dss_aux_##inst),                                     \
		.fck_clk_id = DT_INST_PROP(inst, tisci_fck_clk_id),                                \
		.vp_clk_id = DT_INST_PROP(inst, tisci_vp_clk_id),                                  \
		.vp_clk_div = DT_INST_PROP(inst, vp_clk_div),                                      \
		.irq_config = dss_irq_config_##inst,                                               \
	};                                                                                         \
	static struct dss_data dss_data_##inst;                                                    \
	DEVICE_DT_INST_DEFINE(inst, dss_init, NULL, &dss_data_##inst, &dss_cfg_##inst, POST_KERNEL, \
			      CONFIG_YAKOGL_DRV_DSS_AM62P_INIT_PRIORITY, &dss_api);

DT_INST_FOREACH_STATUS_OKAY(DSS_DEFINE)
