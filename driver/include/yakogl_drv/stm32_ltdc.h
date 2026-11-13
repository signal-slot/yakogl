/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The STM32MP25 LTDC device (yakogl,stm32-ltdc;
 * driver/drivers/display/display_yakogl_stm32_ltdc.c), the owner of the DK's
 * display chain LTDC -> DSI host (yakogl,stm32-dsi) -> ADV7535
 * (yakogl,adv7535) -> HDMI. Zephyr's display API covers the picture:
 * get_capabilities() reports the chosen mode, write() copies pixels into the
 * driver's own frame buffer, blanking_off() brings the chain up and
 * blanking_on() takes it down. This header adds what the API has no call
 * for: the bring-up's record, the frame check, a flip to a foreign buffer
 * (a GPU-rendered frame) and the board hooks.
 *
 * The mode (chosen once, at the first API call): the EDID's preferred timing
 * when it is 1920x1080@60 (VIC 16, 148.5 MHz) and fits the node's hactive x
 * vactive and the frame buffer, else 1280x720@60 (VIC 4, 74.25 MHz).
 *
 * The bring-up (blanking_off), in the order of yakogl_mp25_display.h:
 * board ready, frame buffer allowed, ADV7535 init / HPD / EDID / mode,
 * the pixel clock, LTDC probe / configure / enable, DSI attach (host and PHY
 * up, video streaming), ADV7535 start, one frame.
 */
#ifndef YAKOGL_DRV_STM32_LTDC_H
#define YAKOGL_DRV_STM32_LTDC_H

#include <stdint.h>
#include <zephyr/device.h>
#include <yakogl_mp25_display.h>

#ifdef __cplusplus
extern "C" {
#endif

/* struct stm32_ltdc_status.step: the step that failed last, 0 none. */
#define STM32_LTDC_STEP_NONE       0u
#define STM32_LTDC_STEP_BOARD      1u  /* stm32_ltdc_board_ready() */
#define STM32_LTDC_STEP_FB         2u  /* no frame buffer, or the board refused it */
#define STM32_LTDC_STEP_ADV_INIT   3u  /* adv7535_init() */
#define STM32_LTDC_STEP_MODE       4u  /* no mode fits the node and the buffer */
#define STM32_LTDC_STEP_CLOCK      5u  /* stm32_ltdc_board_pixel_clock() */
#define STM32_LTDC_STEP_LTDC_PROBE 6u
#define STM32_LTDC_STEP_LTDC_CONF  7u
#define STM32_LTDC_STEP_LTDC_ON    8u
#define STM32_LTDC_STEP_DSI_MODE   9u  /* stm32_dsi_set_mode() */
#define STM32_LTDC_STEP_DSI        10u /* mipi_dsi_attach(): stm32_dsi_data() says where */
#define STM32_LTDC_STEP_ADV_START  11u
#define STM32_LTDC_STEP_FRAME      12u /* no frame after the chain came up */
#define STM32_LTDC_STEP_I2C        13u /* the bus did not come back after a re-power */

/* ISR bits (yd_mp25_ltdc_isr()). */
#define STM32_LTDC_ISR_LINE     (1u << 0)
#define STM32_LTDC_ISR_FUWARN   (1u << 1)
#define STM32_LTDC_ISR_TERR     (1u << 2)
#define STM32_LTDC_ISR_FUERR    (1u << 6)
/* What makes a frame bad: a bus error or an underrun error. */
#define STM32_LTDC_ISR_ERRORS   (STM32_LTDC_ISR_TERR | STM32_LTDC_ISR_FUERR)

struct stm32_ltdc_status {
	uint32_t step;              /* STM32_LTDC_STEP_* that failed last, 0 none */
	int32_t err;                /* its negative errno */
	int32_t choice;             /* YD_MP25_MODE_* of choose_mode, -1 not chosen */
	uint8_t selected;           /* the mode is chosen */
	uint8_t hpd;                /* 1 a sink was plugged */
	uint8_t edid_ok;            /* block 0 read */
	uint8_t on;                 /* the chain is up */
	uint8_t edid[YD_MP25_EDID_BLOCK];
	struct yd_mp25_mode mode;   /* what the chain runs */
	struct yd_mp25_hdmi_sink sink;
	uint32_t pixel_clock_hz;    /* the LTDC kernel clock the board reported */
	uint32_t pitch;             /* bytes per line of the frame buffer */
	uint32_t bpp;               /* bytes per pixel */
	uint64_t fb_phys;           /* the buffer scanned out */
	struct yd_mp25_ltdc_data ltdc; /* the LTDC's record (hwver, bus_bytes, isr) */
};

/* The driver's frame buffer, mapped normal non-cacheable: a CPU write needs
 * no cache maintenance, only the barrier the driver's write() issues. */
struct stm32_ltdc_fb {
	void *virt;
	uint64_t phys;
	uint64_t size;
	uint32_t pitch;
	uint32_t width;
	uint32_t height;
	uint32_t bpp;
};

/* The record; NULL for a device that is not a ready yakogl,stm32-ltdc. */
const struct stm32_ltdc_status *stm32_ltdc_status(const struct device *dev);

/* The driver's buffer at the chosen mode (selects the mode first). -ENODEV
 * not a ready LTDC, -ENOMEM the node has no frame buffer (chosen
 * yakogl,ltdc-fb), or the select's error. */
int stm32_ltdc_framebuffer(const struct device *dev, struct stm32_ltdc_fb *fb);

/* One frame: yd_mp25_ltdc_wait_frame() then the ISR read and cleared into
 * *isr (may be NULL). -ENODEV while the chain is down, -ETIMEDOUT no frame,
 * -EIO a transfer or underrun error (STM32_LTDC_ISR_ERRORS) in the ISR. */
int stm32_ltdc_wait_frame(const struct device *dev, uint32_t *isr);

/* Scan out another buffer (a GPU-rendered frame) at the chain's pitch and
 * format; waits for the vblank reload. The buffer is the caller's: below
 * 4 GiB, bus-aligned, allowed for the LTDC (stm32_ltdc_board_fb_check()).
 * fb_phys 0 goes back to the driver's buffer. -ENODEV while down. */
int stm32_ltdc_set_fb(const struct device *dev, uint64_t fb_phys);

/* ---- board hooks ---------------------------------------------------------
 * Weak defaults in the driver; the board layer overrides them
 * (driver/zephyr/lib/board/stm32mp2/display_power_mp2.c on the DK).
 */

/* 0 when the display path's clocks, resets, supplies, pins and RIF are up
 * (the board layer's power-on before the drivers initialised), else the
 * negative errno of what failed: then nothing of the chain is touched.
 * Default: 0. */
int stm32_ltdc_board_ready(void);

/* [phys, phys + size) may be scanned out by the LTDC (RIF/RISAF). 0, or a
 * negative errno. Default: 0. */
int stm32_ltdc_board_fb_check(uint64_t phys, uint64_t size);

/* Run the LTDC kernel clock at hz before the LTDC is configured; *actual_hz
 * is the rate read back. 0: it already ran at hz, nothing was reset. 1: the
 * path was powered again for the new rate, which pulsed the resets of the
 * LTDC, the DSI and I2C2 (the caller re-initialises the I2C controller and
 * the ADV7535). Negative errno: refused. Default: -ENOSYS (the clock is
 * what the devicetree says; *actual_hz = hz). */
int stm32_ltdc_board_pixel_clock(uint32_t hz, uint32_t *actual_hz);

#ifdef __cplusplus
}
#endif

#endif /* YAKOGL_DRV_STM32_LTDC_H */
