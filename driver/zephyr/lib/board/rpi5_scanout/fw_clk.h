/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * fw_clk.h: firmware clock queries for the Pi 5 display clocks (header-only,
 * C99, no Zephyr, no libc beyond <stdint.h>/<stddef.h>).
 *
 * Builds and parses property-interface buffers (the buffer layout of the
 * BCM2835 property channel: u32 size, u32 code, then tags of
 * (id, value-buffer bytes, request/response code, values...), then a 0 end
 * tag). Sending the buffer is the caller's (rpi_mbox.c-style) business.
 *
 * Citations: driver/docs/rpi5-v3d71-facts.md section 9 (clock tags,
 * rpi:include/soc/bcm2835/raspberrypi-firmware.h:55,56,58,61,79,81,82, clock
 * ids :165-182) and section 10 (display clocks,
 * rpi:drivers/gpu/drm/vc4/vc4_hdmi.c:1523-1526,1546-1553; HSM_MIN_CLOCK_FREQ
 * at vc4_hdmi.c:112).
 */
#ifndef YD5_FW_CLK_H
#define YD5_FW_CLK_H

#include <stddef.h>
#include <stdint.h>

/* Property tags (raspberrypi-firmware.h:55,56,58,61,79,81,82). */
#define YD5_TAG_GET_CLOCK_STATE         0x00030001u /* :55 */
#define YD5_TAG_GET_CLOCK_RATE          0x00030002u /* :56 */
#define YD5_TAG_GET_MAX_CLOCK_RATE      0x00030004u /* :58 */
#define YD5_TAG_GET_MIN_CLOCK_RATE      0x00030007u /* :61 */
#define YD5_TAG_GET_CLOCK_RATE_MEASURED 0x00030047u /* :79 */
#define YD5_TAG_SET_CLOCK_STATE         0x00038001u /* :81 */
#define YD5_TAG_SET_CLOCK_RATE          0x00038002u /* :82 */

/* Display clock ids (raspberrypi-firmware.h:165-182; facts section 10:
 * HVS core/disp at bcm2712-rpi-5-b.dts:288-291, HDMI hdmi/bvb at :276-286). */
#define YD5_CLK_CORE      4u  /* RPI_FIRMWARE_CORE_CLK_ID: HVS core */
#define YD5_CLK_M2MC      13u /* RPI_FIRMWARE_M2MC_CLK_ID: HDMI hdmi (HSM) */
#define YD5_CLK_PIXEL_BVB 14u /* RPI_FIRMWARE_PIXEL_BVB_CLK_ID: HDMI bvb */
#define YD5_CLK_DISP      16u /* RPI_FIRMWARE_DISP_CLK_ID: HVS disp */

/* Property buffer codes (property interface convention). */
#define YD5_FWCLK_REQUEST    0x00000000u
#define YD5_FWCLK_RESP_OK    0x80000000u /* buffer code, success */
#define YD5_FWCLK_TAG_RESP   0x80000000u /* bit 31 of a tag's code word */
#define YD5_FWCLK_STATE_ON   0x1u        /* GET_CLOCK_STATE: bit 0 on */
#define YD5_FWCLK_STATE_NONE 0x2u        /* bit 1 clock does not exist */

/* Each query tag: 3 header words + 2 value words (id, value). */
#define YD5_FWCLK_TAG_WORDS 5u
#define YD5_FWCLK_QUERIES   3u
#define YD5_FWCLK_MAX_IDS   16u
/* Words of a buffer for n clocks: size, code, 3 tags each, end tag. */
#define YD5_FWCLK_QUERY_WORDS(n) (2u + YD5_FWCLK_QUERIES * YD5_FWCLK_TAG_WORDS * (n) + 1u)
#define YD5_FWCLK_SET_WORDS      9u /* size, code, SET_CLOCK_RATE tag (6), end */

struct yd5_fwclk_info {
	uint32_t id;
	int present;       /* all three tags answered and the clock exists */
	int on;            /* state bit 0 */
	uint32_t rate;     /* GET_CLOCK_RATE, Hz */
	uint32_t measured; /* GET_CLOCK_RATE_MEASURED, Hz */
	int missing_resp;  /* a tag of this clock lacks its response bit */
};

static inline void yd5_fwclk_put_tag_(uint32_t *b, uint32_t tag, uint32_t vwords,
				      uint32_t id, uint32_t val)
{
	b[0] = tag;
	b[1] = vwords * 4u;
	b[2] = YD5_FWCLK_REQUEST;
	b[3] = id;
	b[4] = val;
}

/* Words written, or 0 if n is 0 / above YD5_FWCLK_MAX_IDS or cap too small.
 * The caller aligns buf (16 bytes) if it is sent to the firmware. */
static inline size_t yd5_fwclk_build_query(uint32_t *buf, size_t cap_words,
					   const uint32_t *ids, size_t n)
{
	static const uint32_t tags[YD5_FWCLK_QUERIES] = {
		YD5_TAG_GET_CLOCK_STATE, YD5_TAG_GET_CLOCK_RATE,
		YD5_TAG_GET_CLOCK_RATE_MEASURED,
	};
	size_t w = 2, i, q;

	if (n == 0 || n > YD5_FWCLK_MAX_IDS || cap_words < YD5_FWCLK_QUERY_WORDS(n))
		return 0;
	for (i = 0; i < n; i++)
		for (q = 0; q < YD5_FWCLK_QUERIES; q++) {
			yd5_fwclk_put_tag_(buf + w, tags[q], 2, ids[i], 0);
			w += YD5_FWCLK_TAG_WORDS;
		}
	buf[w++] = 0; /* end tag */
	buf[0] = (uint32_t)(w * 4u);
	buf[1] = YD5_FWCLK_REQUEST;
	return w;
}

/* Parse the answer of yd5_fwclk_build_query for the same n clocks. Returns 0
 * when the buffer is well-formed and the firmware accepted it; a clock whose
 * tag lacks the response bit has missing_resp=1 and present=0 (not an error
 * of the call). -1: malformed buffer; -2: buffer code is not success. */
static inline int yd5_fwclk_parse_query(const uint32_t *buf, size_t n,
					struct yd5_fwclk_info *out)
{
	size_t i, q;
	const uint32_t *t;

	if (n == 0 || n > YD5_FWCLK_MAX_IDS ||
	    buf[0] != YD5_FWCLK_QUERY_WORDS(n) * 4u)
		return -1;
	if (buf[1] != YD5_FWCLK_RESP_OK)
		return -2;
	t = buf + 2;
	for (i = 0; i < n; i++) {
		struct yd5_fwclk_info *o = &out[i];
		uint32_t state = 0;
		int resp = 1;

		o->id = t[3];
		o->present = o->on = o->missing_resp = 0;
		o->rate = o->measured = 0;
		for (q = 0; q < YD5_FWCLK_QUERIES; q++, t += YD5_FWCLK_TAG_WORDS) {
			if (t[3] != o->id || t[1] != 8u)
				return -1;
			if (!(t[2] & YD5_FWCLK_TAG_RESP)) {
				resp = 0;
				continue;
			}
			if (q == 0)
				state = t[4];
			else if (q == 1)
				o->rate = t[4];
			else
				o->measured = t[4];
		}
		o->missing_resp = !resp;
		o->on = resp && (state & YD5_FWCLK_STATE_ON) ? 1 : 0;
		o->present = resp && !(state & YD5_FWCLK_STATE_NONE);
	}
	return 0;
}

/* SET_CLOCK_RATE: id, rate Hz, skip_setting_turbo (1 = do not touch turbo).
 * Returns words written or 0. */
static inline size_t yd5_fwclk_build_set_rate(uint32_t *buf, size_t cap_words,
					      uint32_t id, uint32_t hz,
					      uint32_t skip_turbo)
{
	if (cap_words < YD5_FWCLK_SET_WORDS)
		return 0;
	buf[2] = YD5_TAG_SET_CLOCK_RATE;
	buf[3] = 12u;
	buf[4] = YD5_FWCLK_REQUEST;
	buf[5] = id;
	buf[6] = hz;
	buf[7] = skip_turbo ? 1u : 0u;
	buf[8] = 0;
	buf[0] = YD5_FWCLK_SET_WORDS * 4u;
	buf[1] = YD5_FWCLK_REQUEST;
	return YD5_FWCLK_SET_WORDS;
}

/* Parse the answer of build_set_rate: the rate the firmware set (it may
 * differ from the request) goes to *rate. 0 ok; -1 malformed; -2 buffer code
 * not success; -3 tag response bit missing; -4 the firmware answered rate 0
 * (it does not know the clock). */
static inline int yd5_fwclk_parse_set_rate(const uint32_t *buf, uint32_t id,
					   uint32_t *rate)
{
	if (buf[0] != YD5_FWCLK_SET_WORDS * 4u || buf[2] != YD5_TAG_SET_CLOCK_RATE ||
	    buf[3] != 12u || buf[5] != id)
		return -1;
	if (buf[1] != YD5_FWCLK_RESP_OK)
		return -2;
	if (!(buf[4] & YD5_FWCLK_TAG_RESP))
		return -3;
	*rate = buf[6];
	return buf[6] == 0 ? -4 : 0;
}

/* HDMI clock rules, vc4_hdmi.c:1523-1526 (HSM) and :1546-1553 (BVB). */
struct yd5_hdmi_clocks {
	uint64_t tmds_hz;    /* TMDS character rate = pixel * bpc / 8 */
	uint64_t hsm_min_hz; /* max(120 MHz, 1.01 * tmds); HSM_MIN :112 */
	uint32_t bvb_hz;     /* 300/150/75 MHz by pixel clock */
};

static inline struct yd5_hdmi_clocks yd5_hdmi_clocks(uint64_t pixel_clock_hz,
						     unsigned int bpc)
{
	struct yd5_hdmi_clocks c;

	c.tmds_hz = pixel_clock_hz * bpc / 8u;
	c.hsm_min_hz = c.tmds_hz / 100u * 101u + (c.tmds_hz % 100u) * 101u / 100u;
	if (c.hsm_min_hz < 120000000u)
		c.hsm_min_hz = 120000000u;
	c.bvb_hz = pixel_clock_hz > 297000000u ? 300000000u :
		   pixel_clock_hz > 148500000u ? 150000000u : 75000000u;
	return c;
}

/* ---- text formatter: ids, state and rates only, no addresses ---- */

static inline const char *yd5_fwclk_name(uint32_t id)
{
	switch (id) {
	case YD5_CLK_CORE:
		return "core";
	case YD5_CLK_M2MC:
		return "m2mc";
	case YD5_CLK_PIXEL_BVB:
		return "pixel_bvb";
	case YD5_CLK_DISP:
		return "disp";
	default:
		return "unknown";
	}
}

static inline size_t yd5_fwclk_put_str_(char *b, size_t cap, size_t at, const char *s)
{
	while (*s) {
		if (at + 1 < cap)
			b[at] = *s;
		at++, s++;
	}
	return at;
}

static inline size_t yd5_fwclk_put_u_(char *b, size_t cap, size_t at, uint32_t v)
{
	char t[10];
	size_t n = 0;

	do {
		t[n++] = (char)('0' + v % 10u);
		v /= 10u;
	} while (v);
	while (n--) {
		if (at + 1 < cap)
			b[at] = t[n];
		at++;
	}
	return at;
}

/* One line per clock: "u3 clk <name> id=<n> on=<0|1> rate=<hz>
 * measured=<hz>\n". Returns the length the text needs (snprintf style); the
 * output is NUL-terminated if cap > 0, and truncated if cap is too small. */
static inline size_t yd5_fwclk_format(char *buf, size_t cap,
				      const struct yd5_fwclk_info *info, size_t n)
{
	size_t at = 0, i;

	for (i = 0; i < n; i++) {
		at = yd5_fwclk_put_str_(buf, cap, at, "u3 clk ");
		at = yd5_fwclk_put_str_(buf, cap, at, yd5_fwclk_name(info[i].id));
		at = yd5_fwclk_put_str_(buf, cap, at, " id=");
		at = yd5_fwclk_put_u_(buf, cap, at, info[i].id);
		at = yd5_fwclk_put_str_(buf, cap, at, info[i].on ? " on=1" : " on=0");
		at = yd5_fwclk_put_str_(buf, cap, at, " rate=");
		at = yd5_fwclk_put_u_(buf, cap, at, info[i].rate);
		at = yd5_fwclk_put_str_(buf, cap, at, " measured=");
		at = yd5_fwclk_put_u_(buf, cap, at, info[i].measured);
		at = yd5_fwclk_put_str_(buf, cap, at, "\n");
	}
	if (cap)
		buf[at < cap ? at : cap - 1] = '\0';
	return at;
}

#endif /* YD5_FW_CLK_H */
