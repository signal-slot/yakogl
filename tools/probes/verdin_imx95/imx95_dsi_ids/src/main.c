/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * imx95_dsi_ids: is the X4 display path there, and what are its parts
 * (README.md)? Read-only identity with the domains on; nothing is started:
 *   1. DISPLAY (13) and CAMERA (3) on over SCMI (POWER_STATE_SET, read
 *      back), the only SCMI writes besides the two LPI2C clocks
 *   2. the DW MIPI DSI host's VERSION (U18) and its plain configuration
 *      words; the DSI stream and master CSRs and the combo PHY CSR words the
 *      design note's M4 names (U13 COMBO_PHY_FREQ_CONTROL, U16
 *      COMBO_PHY_MODE_CONTROL and TEST_MODE_CONTROL), one 32-bit read each
 *   3. LPI2C2 and LPI2C3 clocked over SCMI and initialised; the module's
 *      PCAL6416A at 0x20 read raw (it has no ID register: its eight
 *      registers answering is its identity)
 *   4. GPIO_10_DSI (the expander's line 3) low 20 ms, then high, and the
 *      SN65DSI83's ID (0x00..0x08) polled every 5 ms for 1 s (U21: when it
 *      first answers); then GPIO_10_DSI low again
 *   5. the domains back as they were found
 * then the verdict and the PSCI reset. The DPU is not read (U6: DISPLAY is
 * on here, but nothing needs it). The DW host's GEN_PLD_DATA (0x70) and
 * INT_ST0/1 (0xbc, 0xc0) change on read and are not read.
 */
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/drivers/clock_control.h>
#include <zephyr/drivers/gpio.h>
#include <zephyr/drivers/i2c.h>
#include <zephyr/dt-bindings/clock/imx95_clock.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>
#include <errno.h>
#include <string.h>

#include <yakogl_drivers.h>

#include "vi95_probe.h"

#define PD_CAMERA  3u
#define PD_DISPLAY 13u
#define POWER_ON   0u
#define POWER_OFF  0x40000000u

#define EXP_NODE    DT_NODELABEL(som_gpio_expander)
#define I2C_EXP     DT_NODELABEL(lpi2c2)
#define I2C_BRIDGE  DT_NODELABEL(lpi2c3)
#define CLK_NODE    DT_NODELABEL(scmi_clk)
#define EXP_ADDR    0x20
#define EXP_LINE_EN 3 /* GPIO_10_DSI, Toradex's overlay `enable-gpios` */
#define BRIDGE_ADDR 0x2c
BUILD_ASSERT(DT_REG_ADDR(EXP_NODE) == EXP_ADDR && DT_REG_ADDR(I2C_EXP) == 0x44350000 &&
		     DT_REG_ADDR(I2C_BRIDGE) == 0x42530000,
	     "the expander at 0x20 on LPI2C2, the bridge's bus LPI2C3");

/* ip::sn65dsi8x::regs::ID_BYTES: "58ISD   " and 0x01 at 0x00..0x08 */
static const uint8_t bridge_id[9] = { 0x35, 0x38, 0x49, 0x53, 0x44, 0x20, 0x20, 0x20, 0x01 };
#define ID_POLL_MS  5
#define ID_POLL_MAX 200 /* 1 s, as ip::sn65dsi8x's ID_TRIES x ID_RETRY_MS */

static int pd_state(uint32_t pd, uint32_t *state)
{
	int32_t st;
	int n = vi95_scmi(0x11, 0x5, &pd, 1, state, 1, &st);

	return n < 0 ? n : n < 1 ? -EBADMSG : st != 0 ? (int)st : 0;
}

/* POWER_STATE_SET (flags 0: synchronous), then read back. */
static int pd_set(uint32_t pd, uint32_t state)
{
	uint32_t w[3] = { 0, pd, state }, got, r[1];
	int32_t st;
	int n = vi95_scmi(0x11, 0x4, w, 3, r, 0, &st);

	if (n < 0) {
		return n;
	}
	if (st != 0) {
		VI95_P("pd %u set 0x%08x = %s (%d)", pd, state, vi95_scmi_status(st), st);
		return st;
	}
	n = pd_state(pd, &got);
	VI95_P("pd %u set 0x%08x, reads 0x%08x (rc %d)", pd, state, got, n);
	return n != 0 ? n : got == state ? 0 : -EIO;
}

/* `n` words from `phys`, one 32-bit read each, as "<what> 0xADDR = 0xVALUE". */
static void words(const char *what, uintptr_t phys, size_t size, const uint32_t *offs, size_t n,
		  uint32_t *out)
{
	mm_reg_t base;

	device_map(&base, phys, size, K_MEM_CACHE_NONE);
	for (size_t i = 0; i < n; i++) {
		uint32_t v = sys_read32(base + offs[i]);

		if (out != NULL) {
			out[i] = v;
		}
		VI95_P("%s 0x%08lx = 0x%08x", what, (unsigned long)(phys + offs[i]), v);
	}
}

static int i2c_up(void)
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
	int rc;

	for (size_t i = 0; i < ARRAY_SIZE(bus); i++) {
		rc = clock_control_on(clk, (clock_control_subsys_t)(uintptr_t)bus[i].clk);
		if (rc == 0) {
			rc = device_init(bus[i].dev);
		}
		VI95_P("%s (clock %u) up: %d", bus[i].what, bus[i].clk, rc);
		if (rc != 0 && rc != -EALREADY) {
			return rc;
		}
	}
	return 0;
}

int main(void)
{
	/* DW MIPI DSI host: VERSION, PWR_UP, CLKMGR_CFG, DPI_VCID, DPI_COLOR_CODING,
	 * MODE_CFG, VID_MODE_CFG, PHY_RSTZ, PHY_IF_CFG, PHY_STATUS (U18) */
	static const uint32_t dsi_offs[] = { 0x00, 0x04, 0x08, 0x0c, 0x10, 0x34,
					     0x38, 0xa0, 0xa4, 0xb0 };
	/* the stream CSR 0x00..0x0c, the master CSR 0x00..0x08 (M4) */
	static const uint32_t stream_offs[] = { 0x0, 0x4, 0x8, 0xc };
	static const uint32_t master_offs[] = { 0x0, 0x4, 0x8 };
	/* the combo PHY CSR's MIPI TX words 0x100..0x110: MODE_CONTROL (U16),
	 * FREQ_CONTROL (U13), TEST_MODE_CONTROL (U16), and the two after */
	static const uint32_t phy_offs[] = { 0x100, 0x104, 0x108, 0x10c, 0x110 };
	const struct device *exp_bus = DEVICE_DT_GET(I2C_EXP);
	const struct device *bridge_bus = DEVICE_DT_GET(I2C_BRIDGE);
	const struct device *exp = DEVICE_DT_GET(EXP_NODE);
	uint32_t found[2] = { 0, 0 }, dsi[ARRAY_SIZE(dsi_offs)];
	uint8_t er[8], id[9];
	int rc, exp_rc = -ENODEV, id_rc = -ENODEV, tries = 0;
	int64_t t_en = 0, t_id = 0;
	bool domains = false;

	vi95_probe_banner();
	rc = pd_state(PD_DISPLAY, &found[0]);
	VI95_P("before pd %u state = 0x%08x (rc %d)", PD_DISPLAY, found[0], rc);
	rc = pd_state(PD_CAMERA, &found[1]);
	VI95_P("before pd %u state = 0x%08x (rc %d)", PD_CAMERA, found[1], rc);

	/* 1. the domains on */
	rc = pd_set(PD_DISPLAY, POWER_ON);
	if (rc == 0) {
		rc = pd_set(PD_CAMERA, POWER_ON);
	}
	domains = rc == 0;
	if (!domains) {
		VI95_P("the domains did not come on (rc %d): no CSR is read", rc);
	} else {
		/* 2. the DSI side's identity, read only */
		words("dsi", YD_IMX95_DSI_PHYS, YD_IMX95_DSI_SIZE, dsi_offs, ARRAY_SIZE(dsi_offs),
		      dsi);
		VI95_P("dsi version = 0x%08x", dsi[0]);
		words("stream", YD_IMX95_DSI_STREAM_CSR_PHYS, 0x1000, stream_offs,
		      ARRAY_SIZE(stream_offs), NULL);
		words("master", YD_IMX95_DSI_MASTER_CSR_PHYS, 0x1000, master_offs,
		      ARRAY_SIZE(master_offs), NULL);
		words("phy", YD_IMX95_COMBO_PHY_CSR_PHYS, YD_IMX95_COMBO_PHY_CSR_SIZE, phy_offs,
		      ARRAY_SIZE(phy_offs), NULL);
	}

	/* 3. the buses, the expander raw */
	rc = i2c_up();
	if (rc == 0) {
		exp_rc = i2c_burst_read(exp_bus, EXP_ADDR, 0x00, er, sizeof(er));
		if (exp_rc == 0) {
			VI95_P("expander id = answers at 0x%02x: input 0x%02x%02x output 0x%02x%02x "
			       "polarity 0x%02x%02x config 0x%02x%02x",
			       EXP_ADDR, er[1], er[0], er[3], er[2], er[5], er[4], er[7], er[6]);
		} else {
			VI95_P("expander id = no answer at 0x%02x (%d)", EXP_ADDR, exp_rc);
		}
		rc = device_init(exp);
		if (rc != 0 && rc != -EALREADY) {
			VI95_P("the PCAL6416A's driver did not come up: %d", rc);
		} else {
			/* 4. EN low (the bridge in reset), then high, then the ID */
			rc = gpio_pin_configure(exp, EXP_LINE_EN, GPIO_OUTPUT_LOW);
			k_msleep(20);
			if (rc == 0) {
				rc = gpio_pin_set_raw(exp, EXP_LINE_EN, 1);
			}
			t_en = k_uptime_get();
			if (rc != 0) {
				VI95_P("GPIO_10_DSI not driven: %d", rc);
			}
			while (rc == 0 && tries < ID_POLL_MAX) {
				tries++;
				id_rc = i2c_burst_read(bridge_bus, BRIDGE_ADDR, 0x00, id, sizeof(id));
				if (id_rc == 0) {
					t_id = k_uptime_get();
					break;
				}
				k_msleep(ID_POLL_MS);
			}
			if (id_rc == 0) {
				VI95_P("bridge id = %02x %02x %02x %02x %02x %02x %02x %02x %02x (%s)",
				       id[0], id[1], id[2], id[3], id[4], id[5], id[6], id[7], id[8],
				       memcmp(id, bridge_id, sizeof(id)) == 0 ? "SN65DSI83"
										: "not the SN65DSI83's");
				VI95_P("bridge answered = after %lld ms, try %d (every %d ms)",
				       (long long)(t_id - t_en), tries, ID_POLL_MS);
			} else if (rc == 0) {
				VI95_P("bridge id = no answer at 0x%02x in %d tries over %lld ms (%d)",
				       BRIDGE_ADDR, tries, (long long)(k_uptime_get() - t_en),
				       id_rc);
			}
			(void)gpio_pin_set_raw(exp, EXP_LINE_EN, 0);
			VI95_P("GPIO_10_DSI low again");
		}
	}

	/* 5. the domains as found */
	if (found[1] == POWER_OFF) {
		(void)pd_set(PD_CAMERA, POWER_OFF);
	}
	if (found[0] == POWER_OFF) {
		(void)pd_set(PD_DISPLAY, POWER_OFF);
	}

	if (!domains) {
		VI95_P("FAIL DISPLAY and CAMERA did not come on over SCMI");
	} else if (exp_rc != 0 || id_rc != 0) {
		VI95_P("FAIL dsi version 0x%08x; expander %s, bridge %s", dsi[0],
		       exp_rc == 0 ? "answers" : "silent", id_rc == 0 ? "answers" : "silent");
	} else {
		VI95_P("PASS dsi version 0x%08x, expander answers, bridge %s after %lld ms", dsi[0],
		       memcmp(id, bridge_id, sizeof(id)) == 0 ? "SN65DSI83" : "UNKNOWN",
		       (long long)(t_id - t_en));
	}
	vi95_probe_end();
	return 0;
}
