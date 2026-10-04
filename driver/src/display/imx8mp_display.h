/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * imx8mp_display.h -- the display pipeline of the Verdin iMX8M Plus on a
 * Mallow carrier, owned by Zephyr.
 *
 * The chain is LCDIFv3 (0x32e80000) -> MIPI DSIM + D-PHY (0x32e60000) ->
 * TI SN65DSI84 DSI-to-LVDS bridge on I2C2 at 0x2c -> Riverdi
 * RVT101HVDWC00-B, 1280x800 LVDS, with the backlight on PWM3.
 *
 * Why this exists: a Zephyr image kexec'd out of Torizon arrives to a dark
 * panel, because Linux runs device_shutdown() and the DRM drivers turn the
 * backlight and the LCDIF off on the way out (measured 2026-09-20). The
 * crash route (kexec --crash) skips device_shutdown(), so the display is
 * still running there. Both states are handled: imx8mp_display_on() reads
 * the LCDIF enable bits and either adopts the running pipeline or brings
 * the whole chain up from cold.
 *
 * Compile-tested only. No i.MX8MP board has run this.
 */
#ifndef IMX8MP_DISPLAY_H
#define IMX8MP_DISPLAY_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <zephyr/sys/util.h>

/* What the bring-up found and computed. Filled by imx8mp_display_on();
 * a sample prints it so a person at the panel can check the numbers. */
struct imx8mp_display_info {
	bool adopted;            /* the LCDIF was already scanning on entry */
	bool on;                 /* the pipeline is running now */

	uint32_t width;          /* active pixels */
	uint32_t height;
	uint32_t pitch;          /* bytes per scan line */
	uint32_t bytes_pp;
	uint32_t pixel_clock_hz; /* what the CCM root actually produces */
	uint32_t refresh_mhz;    /* frames per second x 1000 */

	/* DSIM PLL, as programmed (samsung_dsim_pll_find_pms). */
	uint32_t dsi_hs_clock_hz;
	uint32_t dsi_byte_clock_hz;
	uint32_t dsi_esc_clock_hz;
	uint16_t pll_m;
	uint8_t  pll_p;
	uint8_t  pll_s;

	/* SN65DSI84 clock plan. */
	uint8_t lvds_clk_range;  /* REG 0x0a bits 3:1 */
	uint8_t dsi_clk_range;   /* REG 0x12 */
	uint8_t dsi_clk_div;     /* REG 0x0b bits 7:3 */
	uint8_t bridge_id[9];    /* REG 0x00..0x08, the "65DSI84 " string */
	uint8_t bridge_irq_stat; /* REG 0xe5 one millisecond after enable */
};

/*
 * Bring the panel up on `fb_phys` (a linear XRGB8888 surface, `pitch` bytes
 * per line, at least `height` lines; pitch 0 means width * 4).
 *
 * On a pipeline Linux left running this only re-points the scan-out address,
 * so the picture never blanks. On a dark pipeline it powers the MEDIAMIX and
 * MIPI-PHY1 domains, sets the CCM roots, releases the MEDIA_BLK_CTRL resets,
 * programs the LCDIF, the DSIM and its D-PHY, talks to the SN65DSI84 over
 * I2C and turns the backlight on.
 *
 * Returns 0 on success, a negative errno otherwise. It needs the kernel
 * (I2C, GPIO and millisecond sleeps), so it cannot run before POST_KERNEL.
 */
int imx8mp_display_on(uintptr_t fb_phys, uint32_t pitch);

/* Re-point the scan-out at another buffer and wait for the LCDIF to latch
 * it (CTRLDESCL0_5_SHADOW_LOAD_EN clears at the next vblank). */
int imx8mp_display_set_fb(uintptr_t fb_phys);

/*
 * The address the LCDIF is scanning right now, read back out of the
 * controller; 0 when it is not scanning or its layout cannot be read.
 *
 * This is what the panel is actually showing, which is not always what the
 * caller last asked for -- a set_fb whose shadow load did not latch leaves
 * the old address in place, and a program with several buffers can easily
 * show one it never drew. Comparing this with the buffer just rendered is
 * the one check that tells those apart without a person at the panel.
 */
uint64_t imx8mp_display_scanout(void);

/* Backlight off, LCDIF stopped, bridge held in reset. Leaves the domains
 * powered, so imx8mp_display_on() can bring it back. */
void imx8mp_display_off(void);

/* Never NULL; .on is false until imx8mp_display_on() has succeeded. */
const struct imx8mp_display_info *imx8mp_display_info(void);

/* True when the LCDIF is scanning right now, whoever started it. Cheap
 * enough to call from PRE_KERNEL_1 (two register reads, no kernel). */
bool imx8mp_display_is_running(void);

/*
 * Which stages of the chain are alive right now, read back from the
 * hardware: a bitmask a sample can encode into its return time when the
 * panel itself cannot be seen. 0 before the driver has mapped anything.
 */
#define IMX8MP_DISPLAY_HEALTH_LCDIF_RUNNING   BIT(0)
#define IMX8MP_DISPLAY_HEALTH_DSIM_PLL_STABLE BIT(1)
#define IMX8MP_DISPLAY_HEALTH_DSIM_HS_CLOCK   BIT(2)
#define IMX8MP_DISPLAY_HEALTH_BRIDGE_ANSWERS  BIT(3)   /* its ID reads over I2C */
#define IMX8MP_DISPLAY_HEALTH_BRIDGE_PLL_LOCK BIT(4)
#define IMX8MP_DISPLAY_HEALTH_BACKLIGHT_PWM   BIT(5)
unsigned int imx8mp_display_health(void);

/* The panel geometry the device tree describes, for a caller that has to
 * size a framebuffer before the display is up. */
void imx8mp_display_geometry(uint32_t *width, uint32_t *height);

/*
 * CTRLDESCL0_5 as it stands, so a check that fails can say which bit was
 * wrong.
 *
 * imx8mp_display_scanout() answers "what address is in the register", and a
 * shadow load puts one there whether or not the layer is enabled -- so it and
 * imx8mp_display_is_running() together are what a caller needs before it can
 * claim a frame was DISPLAYED rather than merely addressed.
 */
uint32_t imx8mp_display_layer_control(void);

#endif /* IMX8MP_DISPLAY_H */
