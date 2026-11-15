/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Register model of the BCM2712 HDMI0 hotplug bit and the DDC (brcmstb BSC)
 * controller with a sink holding a fixed 128-byte pattern, for
 * driver/zephyr/lib/board/rpi5_scanout/hdmi_hpd.h. Build and run with
 * tools/v3d/check71.d/hdmi_hpd.sh.
 */
#include <stdio.h>
#include <string.h>

#include "../rpi5_scanout/hdmi_hpd.h"

#define HDMI_BASE 0x1000000ull
#define BSC_BASE 0x2000000ull

enum mode { M_OK, M_NAK, M_TIMEOUT };

struct model {
	uint32_t hotplug;
	enum mode mode;
	uint8_t edid[128];
	uint32_t reg[0x58 / 4];
	unsigned ptr, bursts, writes, ends_with_stop, stray;
	uint32_t last_cond, first_rd_cond;
};

static uint32_t m_read(void *c, uint64_t a)
{
	struct model *m = c;

	if (a == HDMI_BASE + HDMI_HPD_REG_HOTPLUG)
		return m->hotplug;
	if (a >= BSC_BASE && a < BSC_BASE + 0x58)
		return m->reg[(a - BSC_BASE) / 4];
	m->stray++;
	return 0;
}

static void m_write(void *c, uint64_t a, uint32_t v)
{
	struct model *m = c;
	uint32_t o = (uint32_t)(a - BSC_BASE), i, len;

	if (a < BSC_BASE || a >= BSC_BASE + 0x58) {
		m->stray++;
		return;
	}
	m->reg[o / 4] = v;
	if (o != HDMI_HPD_BSC_IIC_ENABLE || !(v & HDMI_HPD_EN_ENABLE))
		return;
	if (m->mode == M_TIMEOUT)
		return;
	m->reg[o / 4] = v | HDMI_HPD_EN_INTRP;
	if (m->mode == M_NAK) {
		m->reg[o / 4] |= HDMI_HPD_EN_NOACK;
		return;
	}
	len = m->reg[HDMI_HPD_BSC_CNT / 4];
	if (!(m->reg[HDMI_HPD_BSC_CHIP_ADDRESS / 4] & 1u)) {
		m->ptr = m->reg[HDMI_HPD_BSC_DATA_IN(0) / 4] & 0xff;
		m->writes++;
		return;
	}
	if (m->bursts == 0)
		m->first_rd_cond = v;
	m->last_cond = v;
	for (i = 0; i < len && i < 32; i++) {
		uint32_t b = m->edid[(m->ptr + i) & 127];
		uint32_t *r = &m->reg[HDMI_HPD_BSC_DATA_OUT(i / 4) / 4];

		*r &= ~(0xffu << (8 * (i % 4)));
		*r |= b << (8 * (i % 4));
	}
	m->ptr += len;
	m->bursts++;
	m->ends_with_stop = !(v & HDMI_HPD_EN_NOSTOP);
}

static int n_checks, n_failed;
#define CHECK(c)                                                    \
	do {                                                        \
		n_checks++;                                         \
		if (!(c)) {                                         \
			n_failed++;                                 \
			printf("FAIL line %d: %s\n", __LINE__, #c); \
		}                                                   \
	} while (0)

static void init(struct model *m, enum mode mode)
{
	unsigned i;

	memset(m, 0, sizeof(*m));
	m->mode = mode;
	for (i = 0; i < 128; i++)
		m->edid[i] = (uint8_t)(i * 7 + 3);
}

int main(void)
{
	struct model m;
	hdmi_hpd_io io = { m_read, m_write, &m };
	hdmi_hpd_io bad = { 0, 0, 0 };
	uint8_t out[128];
	unsigned i;

	/* hotplug */
	init(&m, M_OK);
	m.hotplug = 1;
	CHECK(hdmi_hpd_detect(&io, HDMI_BASE) == 1);
	m.hotplug = 0;
	CHECK(hdmi_hpd_detect(&io, HDMI_BASE) == 0);
	m.hotplug = 0xfffffffeu; /* other bits do not count */
	CHECK(hdmi_hpd_detect(&io, HDMI_BASE) == 0);
	m.hotplug = 0x101;
	CHECK(hdmi_hpd_detect(&io, HDMI_BASE) == 1);
	CHECK(hdmi_hpd_detect(&bad, HDMI_BASE) == 0);
	CHECK(hdmi_hpd_detect(NULL, HDMI_BASE) == 0);
	CHECK(m.stray == 0);

	/* successful read */
	init(&m, M_OK);
	memset(out, 0xaa, sizeof(out));
	CHECK(hdmi_hpd_read_edid(&io, BSC_BASE, 8, out) == HDMI_HPD_OK);
	CHECK(memcmp(out, m.edid, 128) == 0);
	CHECK(m.writes == 1);
	CHECK(m.bursts == 4);
	CHECK(m.ptr == 128);
	CHECK(m.ends_with_stop == 1);
	CHECK(m.last_cond & HDMI_HPD_EN_NOSTART);
	CHECK(!(m.first_rd_cond & HDMI_HPD_EN_NOSTART));
	CHECK(m.first_rd_cond & HDMI_HPD_EN_NOSTOP);
	CHECK(m.reg[HDMI_HPD_BSC_CTLHI / 4] == HDMI_HPD_CTLHI_DATAREG_SIZE);
	CHECK(m.reg[HDMI_HPD_BSC_IIC_ENABLE / 4] == 0); /* left idle */
	CHECK(m.stray == 0);
	for (i = 0; i < 128; i++)
		CHECK(out[i] == (uint8_t)(i * 7 + 3));

	/* NAK: no sink on DDC */
	init(&m, M_NAK);
	CHECK(hdmi_hpd_read_edid(&io, BSC_BASE, 8, out) == HDMI_HPD_ERR_NAK);
	CHECK(m.bursts == 0);
	CHECK(m.reg[HDMI_HPD_BSC_IIC_ENABLE / 4] == 0);

	/* timeout: INTRP never rises; the polls are bounded */
	init(&m, M_TIMEOUT);
	CHECK(hdmi_hpd_read_edid(&io, BSC_BASE, 5, out) == HDMI_HPD_ERR_TIMEOUT);
	CHECK(m.bursts == 0 && m.writes == 0);
	CHECK(m.reg[HDMI_HPD_BSC_IIC_ENABLE / 4] == 0); /* transfer abandoned */

	/* bad arguments */
	init(&m, M_OK);
	CHECK(hdmi_hpd_read_edid(NULL, BSC_BASE, 8, out) == HDMI_HPD_ERR_ARG);
	CHECK(hdmi_hpd_read_edid(&bad, BSC_BASE, 8, out) == HDMI_HPD_ERR_ARG);
	CHECK(hdmi_hpd_read_edid(&io, BSC_BASE, 0, out) == HDMI_HPD_ERR_ARG);
	CHECK(hdmi_hpd_read_edid(&io, BSC_BASE, 8, NULL) == HDMI_HPD_ERR_ARG);
	CHECK(m.bursts == 0 && m.writes == 0 && m.stray == 0);

	if (n_failed) {
		printf("hdmi_hpd_mock: %d of %d checks failed\n", n_failed, n_checks);
		return 1;
	}
	printf("hdmi_hpd_mock: %d of %d checks, 0 failed\n", n_checks, n_checks);
	return 0;
}
