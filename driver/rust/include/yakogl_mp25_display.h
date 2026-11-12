/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * yakogl_mp25_display.h -- the C view of ffi::mp25_display (driver/rust/src/
 * ffi/mp25_display.rs): the STM32MP257F-DK's display path, LTDC -> DesignWare
 * DSI host with ST's MP25 PHY wrapper -> ADV7535 -> HDMI, for
 * driver/drivers/display/display_yakogl_stm32_ltdc.c (yakogl,stm32-ltdc),
 * driver/drivers/mipi_dsi/dsi_yakogl_stm32.c (yakogl,stm32-dsi) and
 * driver/drivers/display/bridge_adv7535.c (yakogl,adv7535).
 *
 * Conventions as yakogl_drivers.h: `cfg` points into dev->config, `data` into
 * dev->data, `env` at the shim's struct yd_env. Errors are negative errno;
 * every entry point returns -EINVAL for a NULL pointer it needs. The shim
 * serialises calls on one instance. LP64 only (aarch64).
 *
 * Not done here, the caller's before the first call (soc::stm32mp2::display):
 * the RCC gates and kernel clocks of the LTDC (ck_ker_ltdc, the pixel clock),
 * the DSI (CK_BUS_DSI, CK_KER_DSIPHY = pll_ref_hz, the lane-byte mux) and
 * I2C2; their resets; the SCMI requests (the HDMI 5 V, LDO5, and whatever
 * clock or voltage OP-TEE owns); the RIF settings; the pins; the ADV7535's
 * reset line (PB6) released; the I2C controller initialised.
 *
 * The ADV7535 is reached only through env->i2c_write / env->i2c_write_read
 * with the bus ids YD_MP25_ADV7535_BUS_*; the shim maps each id to its 7-bit
 * address on the board's bus (DK: main 0x3d, DSI/CEC 0x3c, EDID 0x3f,
 * packet 0x38), the secondary ones equal to the config's *_addr.
 *
 * The order (Linux's: DRM enables the CRTC, then the bridge chain):
 *
 *   1. yd_mp25_adv7535_init        chip probe; leaves it powered down
 *   2. yd_mp25_adv7535_hpd         1 = a sink is plugged
 *      yd_mp25_adv7535_read_edid   block 0 (needs HPD and the 5 V)
 *      yd_mp25_choose_mode         EDID preferred, else 720p60
 *   3. yd_mp25_ltdc_probe, yd_mp25_ltdc_configure, yd_mp25_ltdc_enable
 *                                  scanning, the pixel clock must run
 *   4. yd_mp25_dsi_probe, yd_mp25_dsi_attach, yd_mp25_dsi_enable
 *                                  host and PHY up, video mode, clock lane HS
 *   5. yd_mp25_adv7535_start       ONLY after yd_mp25_dsi_enable returned 0:
 *                                  the DSI host must already stream video with
 *                                  the clock lane in HS (ip::adv7535)
 *
 * Stop in reverse: yd_mp25_adv7535_stop, yd_mp25_dsi_disable,
 * yd_mp25_ltdc_disable. Flips: yd_mp25_ltdc_set_fb while enabled.
 */
#ifndef YAKOGL_MP25_DISPLAY_H
#define YAKOGL_MP25_DISPLAY_H

#include "yakogl_drivers.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ---- the mode ------------------------------------------------------------ */

#define YD_MP25_MODE_HSYNC_LOW (1u << 0)
#define YD_MP25_MODE_VSYNC_LOW (1u << 1)
#define YD_MP25_MODE_DE_LOW    (1u << 2)

/* One video mode for all three blocks, porch terms as the devicetree. */
struct yd_mp25_mode {
	uint32_t pixel_clock_hz;
	uint32_t hactive;
	uint32_t hfront;
	uint32_t hsync;
	uint32_t hback;
	uint32_t vactive;
	uint32_t vfront;
	uint32_t vsync;
	uint32_t vback;
	uint32_t flags;            /* YD_MP25_MODE_* */
};

/* What the ADV7535 sends. */
struct yd_mp25_hdmi_sink {
	uint8_t hdmi;              /* 1: HDMI with AVI infoframe, 0: DVI */
	uint8_t vic;               /* CTA-861 VIC (4 = 720p60, 16 = 1080p60), 0 none */
	uint8_t aspect_16_9;       /* 1: 16:9, 0: 4:3 */
	uint8_t reserved;
	uint32_t refresh_hz;
};

/* ---- LTDC (yakogl,stm32-ltdc) -------------------------------------------- */

#define YD_MP25_LTDC_PIXCLK_FALLING (1u << 0) /* GCR.PCPOL (pixclk-falling) */
#define YD_MP25_LTDC_BURSTLEN       (1u << 1) /* burst_bytes valid: L1BLCR */

/* Layer formats; there is no 24-bit RGB888 layer (-ENOTSUP). */
#define YD_MP25_LTDC_FMT_ARGB8888 0u
#define YD_MP25_LTDC_FMT_XRGB8888 1u
#define YD_MP25_LTDC_FMT_RGB565   2u

struct yd_mp25_ltdc_config {
	uintptr_t base;            /* device_map() of the registers */
	uint32_t flags;            /* YD_MP25_LTDC_* */
	uint32_t background;       /* BCCR, 0x00RRGGBB */
	uint32_t burst_bytes;      /* L1BLCR with YD_MP25_LTDC_BURSTLEN, 0 = max */
	uint32_t reserved;
};

struct yd_mp25_ltdc_data {
	uint32_t hwver;            /* IDR, 0x040101 on the MP25 */
	uint32_t bus_bytes;        /* pitch and address alignment */
	uint32_t layers;           /* LCR, 1..4 */
	uint32_t pitch;            /* bytes between line starts */
	uint32_t line_length;      /* CFBLR line length */
	uint32_t width;
	uint32_t height;
	uint32_t format;           /* YD_MP25_LTDC_FMT_* */
	uint32_t isr;              /* ISR as yd_mp25_ltdc_isr() last read it */
	uint8_t on;
	uint8_t probed;
	uint8_t configured;
	uint8_t reserved;
	uint64_t fb;               /* the buffer scanned out */
};

/* IDR/GC2R/LCR read. 0, or -ENODEV (no LTDC version: clock, reset, RIF). */
int yd_mp25_ltdc_probe(const struct yd_mp25_ltdc_config *cfg, struct yd_mp25_ltdc_data *data,
		       const struct yd_env *env);

/* Timings, polarities, background, layer 1 (pitch 0 = width x bpp); no
 * enables. Nothing written on -EINVAL (mode beyond the block or its 150 MHz
 * pad limit, pitch/address not a multiple of bus_bytes, address >= 4 GiB) or
 * -ENOTSUP (format). */
int yd_mp25_ltdc_configure(const struct yd_mp25_ltdc_config *cfg, struct yd_mp25_ltdc_data *data,
			   const struct yd_mp25_mode *mode, uint64_t fb_phys, uint32_t pitch,
			   uint32_t format);

/* Layer on, reload at vblank, LTDCEN; waits two frames for the reload.
 * -ETIMEDOUT: no pixel clock. -EINVAL before a configure. */
int yd_mp25_ltdc_enable(const struct yd_mp25_ltdc_config *cfg, struct yd_mp25_ltdc_data *data,
			const struct yd_env *env);

/* Layer off, immediate reload, LTDCEN cleared. */
int yd_mp25_ltdc_disable(const struct yd_mp25_ltdc_config *cfg, struct yd_mp25_ltdc_data *data,
			 const struct yd_env *env);

/* New buffer address, waits for the vblank reload. -ENODEV while disabled. */
int yd_mp25_ltdc_set_fb(const struct yd_mp25_ltdc_config *cfg, struct yd_mp25_ltdc_data *data,
			const struct yd_env *env, uint64_t fb_phys);

/* One line interrupt at the end of the active area, polled. -ETIMEDOUT
 * after two frames. */
int yd_mp25_ltdc_wait_frame(const struct yd_mp25_ltdc_config *cfg, const struct yd_env *env);

/* ISR read and cleared through ICR, kept in data->isr: bit 0 line, 1 FIFO
 * underrun warning, 2 transfer error, 6 FIFO underrun error. */
uint32_t yd_mp25_ltdc_isr(const struct yd_mp25_ltdc_config *cfg, struct yd_mp25_ltdc_data *data);

/* Eight colour bars (white, yellow, cyan, green, magenta, red, blue, black;
 * 0xFFRRGGBB) into len_px pixels; clean the cache afterwards. */
int yd_mp25_fill_color_bars(uint32_t *buf, size_t len_px, uint32_t width, uint32_t height,
			    uint32_t stride_px);

/* ---- DSI host and PHY (yakogl,stm32-dsi) --------------------------------- */

/* format: Zephyr's MIPI_DSI_PIXFMT_* order. */
#define YD_MP25_DSI_FMT_RGB888        0u
#define YD_MP25_DSI_FMT_RGB666        1u
#define YD_MP25_DSI_FMT_RGB666_PACKED 2u
#define YD_MP25_DSI_FMT_RGB565        3u

/* mode_flags: Linux's MIPI_DSI_* bits. Bit 9 means NO EoT packet (Linux);
 * map Zephyr's flags explicitly. */
#define YD_MP25_DSI_MODE_VIDEO            (1u << 0)
#define YD_MP25_DSI_MODE_VIDEO_BURST      (1u << 1)
#define YD_MP25_DSI_MODE_VIDEO_SYNC_PULSE (1u << 2)
#define YD_MP25_DSI_MODE_VIDEO_HSE        (1u << 4)
#define YD_MP25_DSI_MODE_NO_EOT_PACKET    (1u << 9)
#define YD_MP25_DSI_CLOCK_NON_CONTINUOUS  (1u << 10)
/* What the ADV7535 asks for (adv7533.c). */
#define YD_MP25_DSI_MODE_ADV7535                                                                    \
	(YD_MP25_DSI_MODE_VIDEO | YD_MP25_DSI_MODE_VIDEO_SYNC_PULSE | YD_MP25_DSI_MODE_NO_EOT_PACKET | \
	 YD_MP25_DSI_MODE_VIDEO_HSE)

/* ip::dw_mipi_dsi's attach steps, in data->failed_step. */
#define YD_MP25_DSI_STEP_CHECK      1u
#define YD_MP25_DSI_STEP_PHY_RATE   2u
#define YD_MP25_DSI_STEP_PHY_INIT   3u
#define YD_MP25_DSI_STEP_PHY_LOCK   4u
#define YD_MP25_DSI_STEP_STOP_STATE 5u

struct yd_mp25_dsi_config {
	uintptr_t base;            /* host (0x000) and ST's wrapper (0x400), 0x800 */
	uint32_t lanes;            /* data-lanes, when attach passes 0 */
	uint32_t pll_ref_hz;       /* pll-ref-frequency, CK_KER_DSIPHY, 17-32 MHz */
	uint32_t lane_rate_hz;     /* burst-clock-frequency, 0 = from the mode */
	uint32_t pixel_clock_hz;   /* pixel-clock-frequency, 0 = the mode's */
};

struct yd_mp25_dsi_data {
	struct yd_mp25_mode mode;  /* of the last enable */
	uint32_t lanes;            /* from attach */
	uint32_t format;
	uint32_t mode_flags;
	uint32_t lane_kbps;        /* the PLL's lane rate */
	uint32_t lane_mbps;        /* the rate the host's timings used */
	uint32_t pll_idf;
	uint32_t pll_ndiv;
	uint32_t pll_odf;
	uint32_t esc_div;
	uint32_t hline;            /* VID_HLINE_TIME, lane byte clocks */
	uint32_t failed_step;      /* YD_MP25_DSI_STEP_*, 0 = none */
	int32_t failed_errno;
	uint8_t attached;
	uint8_t on;
	uint8_t reserved[2];
};

/* VERSION is the MP25's 1.41. 0, or -ENODEV (clock, reset, RIF). */
int yd_mp25_dsi_probe(const struct yd_mp25_dsi_config *cfg, const struct yd_env *env);

/* The device on the link (mipi_dsi_attach): lanes (0 = cfg->lanes), format,
 * mode_flags (video mode required). No register touched. -EBUSY while on. */
int yd_mp25_dsi_attach(const struct yd_mp25_dsi_config *cfg, struct yd_mp25_dsi_data *data,
		       uint32_t lanes, uint32_t format, uint32_t mode_flags);

/* dw-mipi-dsi.c mode_set then atomic_enable: host registers, PHY timings,
 * ST's wrapper and PLL, PHY_RSTZ, lock and stop-state waits, two frames,
 * command mode, DSIEN, video mode with the clock lane in HS. A failure
 * leaves data->failed_step/failed_errno. -EINVAL before attach or for a lane
 * rate outside 80-2500 Mbit/s or the reference. */
int yd_mp25_dsi_enable(const struct yd_mp25_dsi_config *cfg, struct yd_mp25_dsi_data *data,
		       const struct yd_env *env, const struct yd_mp25_mode *mode);

/* Command mode, PLL and DSIEN off, host and PHY in reset. 0 when off. */
int yd_mp25_dsi_disable(const struct yd_mp25_dsi_config *cfg, struct yd_mp25_dsi_data *data);

/* ---- ADV7535 (yakogl,adv7535) -------------------------------------------- */

#define YD_MP25_ADV7535_BUS_MAIN    0u
#define YD_MP25_ADV7535_BUS_DSI_CEC 1u
#define YD_MP25_ADV7535_BUS_EDID    2u
#define YD_MP25_ADV7535_BUS_PACKET  3u

#define YD_MP25_ADV7535_NO_TIMING_GEN (1u << 0) /* adi,disable-timing-generator */

/* yd_mp25_choose_mode() results. */
#define YD_MP25_MODE_FROM_EDID        0
#define YD_MP25_MODE_FALLBACK_NO_EDID 1
#define YD_MP25_MODE_FALLBACK_LIMIT   2

#define YD_MP25_EDID_BLOCK 128u

struct yd_mp25_adv7535_config {
	uint8_t lanes;             /* adi,dsi-lanes, 2..4, = the DSI host's */
	uint8_t flags;             /* YD_MP25_ADV7535_* */
	uint8_t edid_addr;         /* edid-reg (7-bit), written into the chip */
	uint8_t packet_addr;       /* packet-reg */
	uint8_t cec_addr;          /* cec-reg */
	uint8_t reserved[3];
};

struct yd_mp25_adv7535_data {
	uint8_t revision;          /* CHIP_REVISION, 0x14 */
	uint8_t hpd;               /* last HPD read */
	uint8_t started;
	uint8_t probed;
};

/* adv7511_probe: revision, fixed patches, secondary addresses, powered down.
 * -EIO when the main map does not answer, -EINVAL for lanes outside 2..4. */
int yd_mp25_adv7535_init(const struct yd_mp25_adv7535_config *cfg,
			 struct yd_mp25_adv7535_data *data, const struct yd_env *env);

/* 1 plugged, 0 not, or a negative errno. */
int yd_mp25_adv7535_hpd(struct yd_mp25_adv7535_data *data, const struct yd_env *env);

/* EDID block into buf (YD_MP25_EDID_BLOCK bytes). -ENODEV with HPD low,
 * -EIO/-ETIMEDOUT from the DDC, -EBUSY once started (read before start). */
int yd_mp25_adv7535_read_edid(struct yd_mp25_adv7535_data *data, const struct yd_env *env,
			      uint32_t block, uint8_t *buf);

/* The EDID's preferred timing when good and within the LTDC (150 MHz) and
 * the ADV7535 on cfg->lanes (148.5 MHz, 891 Mbit/s a lane), else
 * 1280x720@60 (VIC 4). edid may be NULL. Returns YD_MP25_MODE_*. The sink
 * is HDMI only for VIC 4/16 with an EDID extension present, else DVI; the
 * caller may change it before start. */
int yd_mp25_choose_mode(const struct yd_mp25_adv7535_config *cfg, const uint8_t *edid, size_t len,
			struct yd_mp25_mode *mode, struct yd_mp25_hdmi_sink *sink);

/* adv7511_bridge_atomic_enable. Call only once yd_mp25_dsi_enable() has
 * returned 0 (video streaming, clock lane in HS). -EINVAL for a mode the
 * part cannot take on its lanes. */
int yd_mp25_adv7535_start(const struct yd_mp25_adv7535_config *cfg,
			  struct yd_mp25_adv7535_data *data, const struct yd_env *env,
			  const struct yd_mp25_mode *mode, const struct yd_mp25_hdmi_sink *sink);

/* Powered down, DSI receiver off. */
int yd_mp25_adv7535_stop(struct yd_mp25_adv7535_data *data, const struct yd_env *env);

_Static_assert(sizeof(struct yd_mp25_mode) == 40, "yd_mp25_mode");
_Static_assert(offsetof(struct yd_mp25_mode, hactive) == 4, "hactive");
_Static_assert(offsetof(struct yd_mp25_mode, hback) == 16, "hback");
_Static_assert(offsetof(struct yd_mp25_mode, vactive) == 20, "vactive");
_Static_assert(offsetof(struct yd_mp25_mode, vback) == 32, "vback");
_Static_assert(offsetof(struct yd_mp25_mode, flags) == 36, "mode flags");
_Static_assert(sizeof(struct yd_mp25_hdmi_sink) == 8, "yd_mp25_hdmi_sink");
_Static_assert(offsetof(struct yd_mp25_hdmi_sink, vic) == 1, "vic");
_Static_assert(offsetof(struct yd_mp25_hdmi_sink, aspect_16_9) == 2, "aspect_16_9");
_Static_assert(offsetof(struct yd_mp25_hdmi_sink, refresh_hz) == 4, "refresh_hz");
_Static_assert(sizeof(struct yd_mp25_ltdc_config) == 24, "yd_mp25_ltdc_config");
_Static_assert(offsetof(struct yd_mp25_ltdc_config, flags) == 8, "ltdc flags");
_Static_assert(offsetof(struct yd_mp25_ltdc_config, background) == 12, "background");
_Static_assert(offsetof(struct yd_mp25_ltdc_config, burst_bytes) == 16, "burst_bytes");
_Static_assert(sizeof(struct yd_mp25_ltdc_data) == 48, "yd_mp25_ltdc_data");
_Static_assert(offsetof(struct yd_mp25_ltdc_data, bus_bytes) == 4, "bus_bytes");
_Static_assert(offsetof(struct yd_mp25_ltdc_data, pitch) == 12, "pitch");
_Static_assert(offsetof(struct yd_mp25_ltdc_data, format) == 28, "format");
_Static_assert(offsetof(struct yd_mp25_ltdc_data, isr) == 32, "isr");
_Static_assert(offsetof(struct yd_mp25_ltdc_data, on) == 36, "ltdc on");
_Static_assert(offsetof(struct yd_mp25_ltdc_data, configured) == 38, "configured");
_Static_assert(offsetof(struct yd_mp25_ltdc_data, fb) == 40, "fb");
_Static_assert(sizeof(struct yd_mp25_dsi_config) == 24, "yd_mp25_dsi_config");
_Static_assert(offsetof(struct yd_mp25_dsi_config, lanes) == 8, "dsi lanes");
_Static_assert(offsetof(struct yd_mp25_dsi_config, pll_ref_hz) == 12, "pll_ref_hz");
_Static_assert(offsetof(struct yd_mp25_dsi_config, lane_rate_hz) == 16, "lane_rate_hz");
_Static_assert(offsetof(struct yd_mp25_dsi_config, pixel_clock_hz) == 20, "pixel_clock_hz");
_Static_assert(sizeof(struct yd_mp25_dsi_data) == 92, "yd_mp25_dsi_data");
_Static_assert(offsetof(struct yd_mp25_dsi_data, lanes) == 40, "data lanes");
_Static_assert(offsetof(struct yd_mp25_dsi_data, mode_flags) == 48, "mode_flags");
_Static_assert(offsetof(struct yd_mp25_dsi_data, lane_kbps) == 52, "lane_kbps");
_Static_assert(offsetof(struct yd_mp25_dsi_data, pll_odf) == 68, "pll_odf");
_Static_assert(offsetof(struct yd_mp25_dsi_data, hline) == 76, "hline");
_Static_assert(offsetof(struct yd_mp25_dsi_data, failed_errno) == 84, "failed_errno");
_Static_assert(offsetof(struct yd_mp25_dsi_data, attached) == 88, "attached");
_Static_assert(offsetof(struct yd_mp25_dsi_data, on) == 89, "dsi on");
_Static_assert(sizeof(struct yd_mp25_adv7535_config) == 8, "yd_mp25_adv7535_config");
_Static_assert(offsetof(struct yd_mp25_adv7535_config, flags) == 1, "adv flags");
_Static_assert(offsetof(struct yd_mp25_adv7535_config, edid_addr) == 2, "edid_addr");
_Static_assert(offsetof(struct yd_mp25_adv7535_config, packet_addr) == 3, "packet_addr");
_Static_assert(offsetof(struct yd_mp25_adv7535_config, cec_addr) == 4, "cec_addr");
_Static_assert(sizeof(struct yd_mp25_adv7535_data) == 4, "yd_mp25_adv7535_data");
_Static_assert(offsetof(struct yd_mp25_adv7535_data, started) == 2, "started");

#ifdef __cplusplus
}
#endif

#endif /* YAKOGL_MP25_DISPLAY_H */
