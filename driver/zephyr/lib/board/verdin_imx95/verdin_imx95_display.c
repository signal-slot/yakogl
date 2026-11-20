/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Verdin iMX95's screen (verdin_imx95_display.h;
 * CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY): the Toradex Capacitive Touch Display
 * 10.1" DSI on the Mallow's X4, 1280x800 XRGB8888, cold, in the order of
 * driver/docs/imx95-display.md section 4(b). The register logic, the SCMI
 * requests and the bridge are the drivers crate's (board::verdin_imx95_display
 * through yd_imx95_display_up / _flip / _down); this file is what is
 * Zephyr's:
 *
 *   SCMI     the board's one raw channel (verdin_imx95_scmi.c), the GPU's too
 *   blocks   the x4_display node's nine reg entries (app.overlay), each
 *            mapped as Device memory (device_map)
 *   I2C      Zephyr's nxp,lpi2c driver: LPI2C3 for the SN65DSI83 at 0x2C
 *            (env I2C bus YD_IMX95_DISPLAY_BRIDGE_BUS), LPI2C2 for the
 *            expander
 *   EN       Zephyr's PCA-series GPIO driver (nxp,pcal6416) for the module's
 *            PCAL6416A at 0x20 on LPI2C2, line 3 (env GPIO line
 *            YD_IMX95_DISPLAY_LINE_EN), configured as a low output first
 *   waits    k_busy_wait (yd_env_init)
 *   pins     LPI2C2/3's from their nodes at their init, TPM6 channel 2's
 *            (the backlight) from the x4_display node, all through the
 *            System Manager's pin control (SCMI protocol 0x19)
 *
 * The two LPI2C buses and the expander are zephyr,deferred-init in the board
 * devicetree: their clocks (LPI2C2, LPI2C3) are the System Manager's, and
 * the nxp,lpi2c driver neither turns its clock on nor waits for one (it
 * writes the controller at its init and reads the rate over SCMI). So the
 * first show turns both clocks on through Zephyr's SCMI clock driver
 * (CLOCK_CONFIG_SET; AP-NS holds both, the note's section 4(b)), then
 * initialises the buses and the expander, then drives EN low (step 2 of the
 * note; the bridge's start holds it low 10 ms again before it raises it).
 * The rest is yd_imx95_display_up: DISPLAY and its pixel clock, the
 * display CSR, interleaver and DPU, CAMERA and the PHY clocks, the DW host
 * and the combo PHY, the bridge, FrameGen0, video mode, the link check,
 * the backlight.
 *
 * Built and host-tested (the Rust side), not run on a board yet.
 */
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/drivers/clock_control.h>
#include <zephyr/drivers/gpio.h>
#include <zephyr/drivers/i2c.h>
#include <zephyr/drivers/pinctrl.h>
#include <zephyr/dt-bindings/clock/imx95_clock.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <errno.h>

#include <yakogl_drivers.h>
#include <yakogl_drv/yd_env.h>

#include "verdin_imx95_display.h"
#include "verdin_imx95_scmi.h"

#define DISP_NODE   DT_NODELABEL(x4_display)
#define BRIDGE_NODE DT_PHANDLE(DISP_NODE, bridge)
#define EXP_NODE    DT_GPIO_CTLR(DISP_NODE, enable_gpios)
#define I2C_BRIDGE  DT_BUS(BRIDGE_NODE)
#define I2C_EXP     DT_BUS(EXP_NODE)
#define CLK_NODE    DT_NODELABEL(scmi_clk)

BUILD_ASSERT(DT_NODE_HAS_COMPAT_STATUS(DISP_NODE, yakogl_imx95_dsi_display, okay),
	     "the board layer's app.overlay has the X4 display (x4_display)");
BUILD_ASSERT(VERDIN_IMX95_SCREEN_W == YD_IMX95_DISPLAY_WIDTH &&
		     VERDIN_IMX95_SCREEN_H == YD_IMX95_DISPLAY_HEIGHT,
	     "the screen is the panel the Rust side drives");
BUILD_ASSERT(DT_REG_ADDR(I2C_BRIDGE) == 0x42530000 && DT_REG_ADDR(BRIDGE_NODE) == 0x2c,
	     "the bridge is the SN65DSI83 at 0x2C on LPI2C3 (I2C_2_DSI)");
BUILD_ASSERT(DT_REG_ADDR(I2C_EXP) == 0x44350000 && DT_REG_ADDR(EXP_NODE) == 0x20 &&
		     DT_GPIO_PIN(DISP_NODE, enable_gpios) == 3,
	     "EN is line 3 of the PCAL6416A at 0x20 on LPI2C2 (GPIO_10_DSI)");

/* Each block as the Rust side names it: the devicetree's address is the
 * header's, and its size covers what the Rust side reaches. */
#define BLOCK_CHECK(name, PHYS, SIZE)                                                       \
	BUILD_ASSERT(DT_REG_ADDR_BY_NAME(DISP_NODE, name) == (PHYS) &&                      \
			     DT_REG_SIZE_BY_NAME(DISP_NODE, name) >= (SIZE),                \
		     "x4_display reg \"" #name "\" is " #PHYS ", at least " #SIZE " bytes")
BLOCK_CHECK(dpu, YD_IMX95_DPU_PHYS, YD_IMX95_DPU_SIZE);
BLOCK_CHECK(csr, YD_IMX95_DISPLAY_CSR_PHYS, YD_IMX95_DISPLAY_CSR_SIZE);
BLOCK_CHECK(interleaver, YD_IMX95_INTERLEAVER_PHYS, YD_IMX95_INTERLEAVER_SIZE);
BLOCK_CHECK(dsi, YD_IMX95_DSI_PHYS, YD_IMX95_DSI_SIZE);
BLOCK_CHECK(stream, YD_IMX95_DSI_STREAM_CSR_PHYS, YD_IMX95_DSI_STREAM_CSR_SIZE);
BLOCK_CHECK(master, YD_IMX95_DSI_MASTER_CSR_PHYS, YD_IMX95_DSI_MASTER_CSR_SIZE);
BLOCK_CHECK(phy_csr, YD_IMX95_COMBO_PHY_CSR_PHYS, YD_IMX95_COMBO_PHY_CSR_SIZE);
BLOCK_CHECK(csi1, YD_IMX95_CSI1_PHYS, YD_IMX95_CSI1_SIZE);
BLOCK_CHECK(tpm, YD_IMX95_TPM6_PHYS, YD_IMX95_TPM6_SIZE);

PINCTRL_DT_DEFINE(DISP_NODE);

static const struct i2c_dt_spec vi95_i2c[] = {
	[YD_IMX95_DISPLAY_BRIDGE_BUS] = {
		.bus = DEVICE_DT_GET(I2C_BRIDGE),
		.addr = DT_REG_ADDR(BRIDGE_NODE),
	},
};
static const struct gpio_dt_spec vi95_gpio[] = {
	[YD_IMX95_DISPLAY_LINE_EN] = GPIO_DT_SPEC_GET(DISP_NODE, enable_gpios),
};
static const struct yd_env_devs vi95_devs = {
	.i2c = vi95_i2c,
	.n_i2c = ARRAY_SIZE(vi95_i2c),
	.gpio = vi95_gpio,
	.n_gpio = ARRAY_SIZE(vi95_gpio), /* no reset line: -ENOTSUP, never asked */
};

static struct yd_env vi95_env;
static struct yd_imx95_display_bases vi95_bases;
static struct yd_imx95_display vi95_disp;

#define MAP(field, name)                                                                     \
	device_map((mm_reg_t *)&vi95_bases.field, DT_REG_ADDR_BY_NAME(DISP_NODE, name),     \
		   DT_REG_SIZE_BY_NAME(DISP_NODE, name), K_MEM_CACHE_NONE)

/* The I2C side, before the first SCMI request of the picture: the LPI2C
 * clocks on, the buses and the expander initialised, EN low. */
static int vi95_i2c_up(void)
{
	static const struct {
		const char *what;
		uint32_t clk;
		const struct device *dev;
	} bus[] = {
		{ "LPI2C2", IMX95_CLK_LPI2C2, DEVICE_DT_GET(I2C_EXP) },
		{ "LPI2C3", IMX95_CLK_LPI2C3, DEVICE_DT_GET(I2C_BRIDGE) },
	};
	const struct device *clk = DEVICE_DT_GET(CLK_NODE);
	const struct device *exp = DEVICE_DT_GET(EXP_NODE);
	int rc;

	for (size_t i = 0; i < ARRAY_SIZE(bus); i++) {
		rc = clock_control_on(clk, (clock_control_subsys_t)(uintptr_t)bus[i].clk);
		if (rc == 0) {
			rc = device_init(bus[i].dev);
		}
		if (rc != 0 && rc != -EALREADY) {
			printk("verdin_imx95: display: %s (clock %u) not up: %d\n", bus[i].what,
			       bus[i].clk, rc);
			return rc;
		}
	}
	rc = device_init(exp);
	if (rc != 0 && rc != -EALREADY) {
		printk("verdin_imx95: display: the PCAL6416A at 0x20 not up: %d\n", rc);
		return rc;
	}
	rc = gpio_pin_configure_dt(&vi95_gpio[YD_IMX95_DISPLAY_LINE_EN], GPIO_OUTPUT_INACTIVE);
	if (rc != 0) {
		printk("verdin_imx95: display: GPIO_10_DSI (expander line 3) not an output: %d\n",
		       rc);
	}
	return rc;
}

static int vi95_display_up(uint64_t fb, uint32_t stride)
{
	int rc = vi95_i2c_up();

	if (rc == 0) {
		rc = pinctrl_apply_state(PINCTRL_DT_DEV_CONFIG_GET(DISP_NODE), PINCTRL_STATE_DEFAULT);
		if (rc != 0) {
			printk("verdin_imx95: display: the backlight's pin (TPM6 channel 2): %d\n",
			       rc);
		}
	}
	if (rc != 0) {
		vi95_disp.state = YD_IMX95_DISPLAY_FAILED;
		return -ENODEV;
	}
	MAP(dpu, dpu);
	MAP(csr, csr);
	MAP(interleaver, interleaver);
	MAP(dsi, dsi);
	MAP(stream, stream);
	MAP(master, master);
	MAP(phy_csr, phy_csr);
	MAP(csi1, csi1);
	MAP(tpm, tpm);
	yd_env_init_devs(&vi95_env, &vi95_devs);

	rc = yd_imx95_display_up(verdin_imx95_scmi, &vi95_bases, &vi95_env, fb, stride,
				 &vi95_disp);
	if (rc == 0) {
		printk("verdin_imx95: display up: %ux%u XRGB8888 at 0x%llx stride %u, "
		       "pixel clock %u Hz (DISP level %u), FrameGen0 in sync after %u us, "
		       "DSI host 0x%08x, %u kbit/s a lane, bridge IRQ_STAT 0x%02x, "
		       "backlight %u/255\n",
		       vi95_disp.width, vi95_disp.height, (unsigned long long)fb, stride,
		       vi95_disp.pix_hz[0], vi95_disp.level, vi95_disp.sync_us,
		       vi95_disp.host_version, vi95_disp.lane_kbps, vi95_disp.link_irq_stat,
		       vi95_disp.backlight);
	} else {
		printk("verdin_imx95: display stopped: rc %d, %s side (the line before says "
		       "where)\n",
		       rc,
		       vi95_disp.stop == YD_IMX95_DISPLAY_STOP_DSI   ? "DSI"
		       : vi95_disp.stop == YD_IMX95_DISPLAY_STOP_DPU ? "DPU"
								     : "no");
		vi95_disp.state = YD_IMX95_DISPLAY_FAILED;
		rc = -ENODEV;
	}
	return rc;
}

int verdin_imx95_display_show(uint64_t fb, uint32_t stride)
{
	int rc;

	switch (vi95_disp.state) {
	case YD_IMX95_DISPLAY_NEW:
		return vi95_display_up(fb, stride);
	case YD_IMX95_DISPLAY_UP:
		break;
	default:
		return -ENODEV;
	}
	if (stride != vi95_disp.stride) {
		return -ENOTSUP;
	}
	rc = yd_imx95_display_flip(&vi95_bases, &vi95_env, fb, &vi95_disp);
	if (rc == -ETIMEDOUT) {
		printk("verdin_imx95: display: no shadow load for 0x%llx\n",
		       (unsigned long long)fb);
	}
	return rc;
}

void verdin_imx95_display_down(void)
{
	int rc;

	if (vi95_disp.state != YD_IMX95_DISPLAY_UP) {
		return;
	}
	rc = yd_imx95_display_down(verdin_imx95_scmi, &vi95_bases, &vi95_env, &vi95_disp);
	printk("verdin_imx95: display down: %d after %u flip(s)\n", rc, vi95_disp.flips);
}
