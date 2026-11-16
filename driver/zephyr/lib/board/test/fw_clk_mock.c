/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * fw_clk_mock: host test of rpi5_scanout/fw_clk.h against a small model of
 * the firmware's property interface. Build: cc -std=c99 -Wall -Wextra -Werror.
 */
#include <stdio.h>
#include <string.h>

#include "../rpi5_scanout/fw_clk.h"

static int total, failed;

static void check(int ok, const char *what)
{
	total++;
	if (!ok)
		failed++;
	printf("%s: %s\n", ok ? "ok" : "FAIL", what);
}

/* Firmware model: knows clocks 4, 13, 14 (13 off) and 16; any other id
 * answers state "does not exist" and rate 0. Tags listed in drop_resp are
 * answered without the response bit. */
static uint32_t drop_resp_tag; /* tag id to leave unanswered, 0 = none */
static uint32_t drop_resp_clk; /* only for this clock id */
static uint32_t fail_code;     /* nonzero: buffer code to answer with */

static int fw_known(uint32_t id, int *on, uint32_t *rate, uint32_t *meas)
{
	switch (id) {
	case 4:  *on = 1; *rate = 500000000u; *meas = 499999000u; return 1;
	case 13: *on = 0; *rate = 120000000u; *meas = 0; return 1;
	case 14: *on = 1; *rate = 75000000u; *meas = 74999000u; return 1;
	case 16: *on = 1; *rate = 600000000u; *meas = 600001000u; return 1;
	default: *on = 0; *rate = 0; *meas = 0; return 0;
	}
}

static void fw_process(uint32_t *b)
{
	uint32_t *t = b + 2;

	while (*t) {
		uint32_t tag = t[0], id = t[3];
		int on, known = 0;
		uint32_t rate, meas;

		known = fw_known(id, &on, &rate, &meas);
		switch (tag) {
		case YD5_TAG_GET_CLOCK_STATE:
			t[4] = (on ? 1u : 0u) | (known ? 0u : 2u);
			break;
		case YD5_TAG_GET_CLOCK_RATE:
			t[4] = rate;
			break;
		case YD5_TAG_GET_CLOCK_RATE_MEASURED:
			t[4] = meas;
			break;
		case YD5_TAG_SET_CLOCK_RATE:
			t[4] = known ? t[4] : 0; /* answers the rate set */
			break;
		}
		if (!(drop_resp_tag == tag && drop_resp_clk == id))
			t[2] = YD5_FWCLK_TAG_RESP | t[1];
		t += 3 + t[1] / 4u;
	}
	b[1] = fail_code ? fail_code : YD5_FWCLK_RESP_OK;
}

int main(void)
{
	static const uint32_t ids[4] = { YD5_CLK_CORE, YD5_CLK_M2MC,
					 YD5_CLK_PIXEL_BVB, YD5_CLK_DISP };
	uint32_t buf[64];
	struct yd5_fwclk_info in[5];
	size_t w;
	int rc;

	/* constants */
	check(YD5_TAG_GET_CLOCK_STATE == 0x30001u && YD5_TAG_GET_CLOCK_RATE == 0x30002u &&
		      YD5_TAG_GET_MAX_CLOCK_RATE == 0x30004u &&
		      YD5_TAG_GET_MIN_CLOCK_RATE == 0x30007u &&
		      YD5_TAG_GET_CLOCK_RATE_MEASURED == 0x30047u &&
		      YD5_TAG_SET_CLOCK_STATE == 0x38001u && YD5_TAG_SET_CLOCK_RATE == 0x38002u,
	      "tag numbers");
	check(YD5_CLK_CORE == 4 && YD5_CLK_M2MC == 13 && YD5_CLK_PIXEL_BVB == 14 &&
		      YD5_CLK_DISP == 16,
	      "clock ids");

	/* builder */
	memset(buf, 0xee, sizeof buf);
	w = yd5_fwclk_build_query(buf, 64, ids, 4);
	check(w == 2 + 3 * 5 * 4 + 1 && buf[0] == w * 4 && buf[1] == 0 && buf[w - 1] == 0,
	      "query buffer: size, code, end tag");
	check(buf[2] == YD5_TAG_GET_CLOCK_STATE && buf[3] == 8 && buf[4] == 0 && buf[5] == 4 &&
		      buf[7] == YD5_TAG_GET_CLOCK_RATE && buf[12] == YD5_TAG_GET_CLOCK_RATE_MEASURED &&
		      buf[17] == YD5_TAG_GET_CLOCK_STATE && buf[18 - 0] == 8 && buf[20] == 13,
	      "query buffer: tag order per clock");
	check(yd5_fwclk_build_query(buf, 10, ids, 4) == 0 && yd5_fwclk_build_query(buf, 64, ids, 0) == 0,
	      "query buffer: too small or empty refused");

	/* all four answered */
	drop_resp_tag = 0;
	fail_code = 0;
	fw_process(buf);
	rc = yd5_fwclk_parse_query(buf, 4, in);
	check(rc == 0, "parse: all four answered");
	check(in[0].id == 4 && in[0].present && in[0].on && in[0].rate == 500000000u &&
		      in[0].measured == 499999000u && !in[0].missing_resp,
	      "clock 4 core");
	check(in[1].id == 13 && in[1].present && !in[1].on && in[1].rate == 120000000u,
	      "clock 13 m2mc (off)");
	check(in[2].id == 14 && in[2].present && in[2].on && in[2].rate == 75000000u,
	      "clock 14 pixel_bvb");
	check(in[3].id == 16 && in[3].present && in[3].on && in[3].measured == 600001000u,
	      "clock 16 disp");

	/* unknown clock */
	{
		uint32_t id99[1] = { 99 };

		w = yd5_fwclk_build_query(buf, 64, id99, 1);
		fw_process(buf);
		rc = yd5_fwclk_parse_query(buf, 1, in);
		check(rc == 0 && in[0].id == 99 && !in[0].present && !in[0].on &&
			      in[0].rate == 0 && !in[0].missing_resp,
		      "unknown clock: not present, rate 0");
	}

	/* missing response bit */
	drop_resp_tag = YD5_TAG_GET_CLOCK_RATE;
	drop_resp_clk = 14;
	w = yd5_fwclk_build_query(buf, 64, ids, 4);
	fw_process(buf);
	rc = yd5_fwclk_parse_query(buf, 4, in);
	check(rc == 0 && in[2].missing_resp && !in[2].present && !in[0].missing_resp &&
		      !in[1].missing_resp && !in[3].missing_resp,
	      "missing response bit flagged on that clock only");
	drop_resp_tag = 0;

	/* buffer-level failures */
	w = yd5_fwclk_build_query(buf, 64, ids, 4);
	fail_code = 0x80000001u;
	fw_process(buf);
	check(yd5_fwclk_parse_query(buf, 4, in) == -2, "parse: buffer code not success");
	fail_code = 0;
	w = yd5_fwclk_build_query(buf, 64, ids, 4);
	fw_process(buf);
	check(yd5_fwclk_parse_query(buf, 3, in) == -1, "parse: wrong clock count is malformed");

	/* SET_CLOCK_RATE */
	{
		uint32_t rate = 0;

		w = yd5_fwclk_build_set_rate(buf, 64, YD5_CLK_PIXEL_BVB, 150000000u, 1);
		check(w == 9 && buf[0] == 36 && buf[2] == YD5_TAG_SET_CLOCK_RATE && buf[3] == 12 &&
			      buf[5] == 14 && buf[6] == 150000000u && buf[7] == 1 && buf[8] == 0,
		      "set rate buffer, skip-turbo word");
		fw_process(buf);
		check(yd5_fwclk_parse_set_rate(buf, 14, &rate) == 0 && rate == 150000000u,
		      "set rate answer");
		w = yd5_fwclk_build_set_rate(buf, 64, 99, 150000000u, 0);
		check(buf[7] == 0, "skip-turbo 0");
		fw_process(buf);
		check(yd5_fwclk_parse_set_rate(buf, 99, &rate) == -4, "set rate on unknown clock: rate 0");
		w = yd5_fwclk_build_set_rate(buf, 64, 14, 1, 1);
		check(yd5_fwclk_parse_set_rate(buf, 14, &rate) == -2, "set rate before the firmware answers");
		buf[1] = YD5_FWCLK_RESP_OK;
		check(yd5_fwclk_parse_set_rate(buf, 14, &rate) == -3, "set rate without tag response bit");
		check(yd5_fwclk_parse_set_rate(buf, 13, &rate) == -1, "set rate answer for another id");
		check(yd5_fwclk_build_set_rate(buf, 4, 14, 1, 1) == 0, "set rate: small buffer refused");
	}

	/* HDMI clock rules */
	{
		struct yd5_hdmi_clocks c;

		c = yd5_hdmi_clocks(25200000u, 8);
		check(c.tmds_hz == 25200000u && c.hsm_min_hz == 120000000u && c.bvb_hz == 75000000u,
		      "hdmi 25.2 MHz: hsm floor 120 MHz, bvb 75");
		c = yd5_hdmi_clocks(74250000u, 8);
		check(c.hsm_min_hz == 120000000u && c.bvb_hz == 75000000u, "hdmi 74.25 MHz");
		c = yd5_hdmi_clocks(148500000u, 8);
		check(c.hsm_min_hz == 149985000u && c.bvb_hz == 75000000u,
		      "hdmi 148.5 MHz: hsm 1.01x, bvb 75 (not above 148.5)");
		c = yd5_hdmi_clocks(297000000u, 8);
		check(c.hsm_min_hz == 299970000u && c.bvb_hz == 150000000u,
		      "hdmi 297 MHz: bvb 150 (not above 297)");
		c = yd5_hdmi_clocks(594000000u, 8);
		check(c.hsm_min_hz == 599940000u && c.bvb_hz == 300000000u,
		      "hdmi 594 MHz: bvb 300");
		c = yd5_hdmi_clocks(148500001u, 8);
		check(c.bvb_hz == 150000000u, "hdmi 148.500001 MHz: bvb 150");
		c = yd5_hdmi_clocks(74250000u, 10);
		check(c.tmds_hz == 92812500u && c.hsm_min_hz == 120000000u, "hdmi 10 bpc: tmds 1.25x");
		c = yd5_hdmi_clocks(148500000u, 12);
		check(c.tmds_hz == 222750000u && c.hsm_min_hz == 224977500u, "hdmi 12 bpc: tmds 1.5x");
	}

	/* formatter */
	{
		char txt[512];
		const char *want =
			"u3 clk core id=4 on=1 rate=500000000 measured=499999000\n"
			"u3 clk m2mc id=13 on=0 rate=120000000 measured=0\n"
			"u3 clk pixel_bvb id=14 on=1 rate=75000000 measured=74999000\n"
			"u3 clk disp id=16 on=1 rate=600000000 measured=600001000\n";
		size_t n, i;

		w = yd5_fwclk_build_query(buf, 64, ids, 4);
		fw_process(buf);
		yd5_fwclk_parse_query(buf, 4, in);
		n = yd5_fwclk_format(txt, sizeof txt, in, 4);
		check(n == strlen(want) && strcmp(txt, want) == 0, "formatter: one line per clock");
		check(strstr(txt, "0x") == NULL, "formatter: no hex, no addresses");
		for (i = 0; i < strlen(txt); i++)
			if (txt[i] != '\n' && (txt[i] < ' ' || txt[i] > '~'))
				break;
		check(i == strlen(txt), "formatter: printable text");
		n = yd5_fwclk_format(txt, 20, in, 4);
		check(n == strlen(want) && strlen(txt) == 19, "formatter: truncates, reports full length");
		check(yd5_fwclk_format(NULL, 0, in, 4) == strlen(want), "formatter: size-only call");
	}
	(void)w;

	printf("fw_clk_mock: %d of %d checks, %d failed\n", total - failed, total, failed);
	/* the result line must read "N of N checks, 0 failed" */
	return failed ? 1 : 0;
}
