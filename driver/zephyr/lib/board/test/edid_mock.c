/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * edid_mock: host test of driver/zephyr/lib/board/rpi5_scanout/edid.h against
 * hand-built EDID blocks: a 1080p monitor with a CEA block, a 720p-only one,
 * a bad checksum, a bad header, a missing extension, a monitor whose only
 * mode is above 600 MHz, and firmware answers (good, status != 0, bad code,
 * response bit clear, wrong block). Every check prints one line; the last
 * line is "N of N checks, 0 failed"; exit 0 when none failed.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "../rpi5_scanout/edid.h"

static int total, failed;

static void check(int ok, const char *what)
{
	total++;
	if (!ok)
		failed++;
	printf("%s: %s\n", ok ? "ok" : "FAIL", what);
}

static void fix_sum(uint8_t *b)
{
	unsigned i, s = 0;

	for (i = 0; i < 127; i++)
		s += b[i];
	b[127] = (uint8_t)(0x100u - (s & 0xffu));
}

/* 1920x1080@60 DTD (VESA: 148.5 MHz, 88/44/148, 4/5/36, +/+). */
static const uint8_t dtd_1080[18] = {
	0x02, 0x3a, 0x80, 0x18, 0x71, 0x38, 0x2d, 0x40, 0x58, 0x2c,
	0x45, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1e
};
/* 1280x720@60: 74.25 MHz, 110/40/220, 5/5/20, +/+. */
static const uint8_t dtd_720[18] = {
	0x01, 0x1d, 0x00, 0x72, 0x51, 0xd0, 0x1e, 0x20, 0x6e, 0x28,
	0x55, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1e
};
/* 2560x1440 at the field's maximum clock, 65535 * 10 kHz = 655.35 MHz,
 * negative sync: above the 600 MHz limit. */
static const uint8_t dtd_fast[18] = {
	0xff, 0xff, 0x00, 0x28, 0xa1, 0xa0, 0x28, 0x50, 0x30, 0x20,
	0x33, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x18
};

static void make_base(uint8_t *b, const uint8_t *dtd, int ext)
{
	static const uint8_t hdr[8] = { 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0 };

	memset(b, 0, 128);
	memcpy(b, hdr, 8);
	b[18] = 1;
	b[19] = 4;
	memcpy(b + 54, dtd, 18);
	b[126] = (uint8_t)ext;
	fix_sum(b);
}

static void make_cea(uint8_t *e, const uint8_t *vics, int n)
{
	int p = 4;

	memset(e, 0, 128);
	e[0] = 0x02;
	e[1] = 3;
	/* an audio block first, so the walk must skip it */
	e[p++] = (1u << 5) | 3u;
	e[p++] = 0x09; e[p++] = 0x07; e[p++] = 0x07;
	e[p++] = (uint8_t)((2u << 5) | (unsigned)n);
	memcpy(e + p, vics, (size_t)n);
	p += n;
	e[2] = (uint8_t)p;
	fix_sum(e);
}

static void make_answer(uint32_t *m, const uint8_t *edid, uint32_t block)
{
	unsigned i;

	edid_build_request(m, block);
	m[EDID_W_CODE] = EDID_FW_RESPONSE_OK;
	m[EDID_W_REQ_RESP] |= EDID_TAG_RESPONSE_BIT;
	m[EDID_W_STATUS] = 0;
	for (i = 0; i < 32; i++)
		m[EDID_W_DATA + i] = (uint32_t)edid[4 * i] |
				     ((uint32_t)edid[4 * i + 1] << 8) |
				     ((uint32_t)edid[4 * i + 2] << 16) |
				     ((uint32_t)edid[4 * i + 3] << 24);
}

int main(void)
{
	uint8_t base[128], ext[128], out[128], vics[16];
	uint32_t msg[EDID_MSG_WORDS];
	struct edid_timing t;
	struct edid_mode m;
	int n;

	/* ---- request builder ---- */
	n = edid_build_request(msg, 1);
	check(n == 160, "build: size in bytes is 160");
	check(msg[0] == 160 && msg[1] == 0, "build: size word, request code 0");
	check(msg[2] == 0x00030020u, "build: tag GET_EDID_BLOCK 0x00030020");
	check(msg[3] == 136 && msg[4] == 136, "build: value and request length 136");
	check(!(msg[4] & 0x80000000u), "build: response bit clear");
	check(msg[5] == 1 && msg[6] == 0, "build: block number 1, status 0");
	check(msg[39] == 0, "build: end tag");
	check(edid_build_request(NULL, 0) == EDID_E_ARG, "build: null buffer");

	/* ---- 1080p monitor ---- */
	{
		const uint8_t v[] = { 0x90, 16, 4, 1, 31 }; /* 0x90 = VIC 16 native */

		make_base(base, dtd_1080, 1);
		make_cea(ext, v, sizeof(v));
		check(edid_base_check(base) == EDID_OK, "1080p: base block valid");
		check(edid_ext_count(base) == 1, "1080p: one extension");
		check(edid_preferred_timing(base, &t) == EDID_OK, "1080p: preferred decodes");
		check(t.pixclk_khz == 148500, "1080p: pixel clock 148500 kHz");
		check(t.h_active == 1920 && t.v_active == 1080, "1080p: 1920x1080 active");
		check(t.h_fp == 88 && t.h_sync == 44 && t.h_bp == 148, "1080p: h 88/44/148");
		check(t.v_fp == 4 && t.v_sync == 5 && t.v_bp == 36, "1080p: v 4/5/36");
		check(t.hsync_pos == 1 && t.vsync_pos == 1, "1080p: sync positive");
		n = edid_cea_vics(ext, vics, sizeof(vics));
		check(n == 5, "1080p: five VICs");
		check(vics[0] == 16 && vics[1] == 16 && vics[2] == 4 && vics[4] == 31,
		      "1080p: VICs listed, native bit stripped");
		m = edid_choose_mode(base, ext);
		check(m.vic == 16 && m.from_edid == 1, "1080p: choice is VIC 16");
		check(m.t.h_active == 1920 && m.t.pixclk_khz == 148500, "1080p: choice timing");
	}

	/* ---- 720p-only monitor ---- */
	{
		const uint8_t v[] = { 4, 19 };

		make_base(base, dtd_720, 1);
		make_cea(ext, v, sizeof(v));
		check(edid_preferred_timing(base, &t) == EDID_OK, "720p: preferred decodes");
		check(t.pixclk_khz == 74250 && t.h_active == 1280 && t.v_active == 720,
		      "720p: 74250 kHz 1280x720");
		check(t.h_fp == 110 && t.h_sync == 40 && t.h_bp == 220 &&
			      t.v_fp == 5 && t.v_sync == 5 && t.v_bp == 20,
		      "720p: porches and syncs");
		m = edid_choose_mode(base, ext);
		check(m.vic == 4 && m.t.pixclk_khz == 74250 && m.t.v_active == 720,
		      "720p: choice is VIC 4");
	}

	/* ---- VGA only in the CEA block ---- */
	{
		const uint8_t v[] = { 1 };

		make_base(base, dtd_720, 1);
		make_cea(ext, v, sizeof(v));
		m = edid_choose_mode(base, ext);
		check(m.vic == 1 && m.t.h_active == 640 && m.from_edid == 1,
		      "vga: choice is VIC 1");
	}

	/* ---- no wanted VIC: preferred timing, or the 600 MHz limit ---- */
	{
		const uint8_t v[] = { 31 };

		make_base(base, dtd_720, 1);
		make_cea(ext, v, sizeof(v));
		m = edid_choose_mode(base, ext);
		check(m.vic == 0 && m.from_edid == 1 && m.t.h_active == 1280,
		      "other VICs: preferred timing taken");
		make_base(base, dtd_fast, 1);
		check(edid_preferred_timing(base, &t) == EDID_OK &&
			      t.pixclk_khz == 655350 && t.hsync_pos == 0 &&
			      t.vsync_pos == 0,
		      "fast: 655350 kHz decodes, negative sync");
		m = edid_choose_mode(base, ext);
		check(m.vic == 1 && m.from_edid == 0 && m.t.h_active == 640,
		      "fast: above 600 MHz falls back to 640x480");
	}

	/* ---- bad blocks ---- */
	make_base(base, dtd_1080, 0);
	base[100] ^= 0x01;
	check(edid_base_check(base) == EDID_E_CHECKSUM, "bad checksum detected");
	m = edid_choose_mode(base, NULL);
	check(m.vic == 1 && m.from_edid == 0, "bad checksum: 640x480 fallback");

	make_base(base, dtd_1080, 0);
	base[3] = 0x00;
	fix_sum(base);
	check(edid_base_check(base) == EDID_E_HEADER, "bad header detected");
	m = edid_choose_mode(base, NULL);
	check(m.vic == 1 && m.from_edid == 0, "bad header: 640x480 fallback");

	make_base(base, dtd_1080, 0);
	base[18] = 2;
	fix_sum(base);
	check(edid_base_check(base) == EDID_E_VERSION, "bad version detected");

	/* missing extension: base only, and a block that is not CEA */
	make_base(base, dtd_1080, 0);
	check(edid_ext_count(base) == 0, "no extension: count 0");
	m = edid_choose_mode(base, NULL);
	check(m.vic == 0 && m.from_edid == 1 && m.t.h_active == 1920,
	      "no extension: preferred timing taken");
	memset(ext, 0, 128);
	check(edid_cea_vics(ext, vics, sizeof(vics)) == EDID_E_NO_EXT,
	      "non-CEA block: EDID_E_NO_EXT");
	check(edid_cea_vics(NULL, vics, 1) == EDID_E_ARG, "vics: null block");
	{
		uint8_t zero[18] = { 0 };

		check(edid_decode_dtd(zero, &t) == EDID_E_NO_TIMING,
		      "dtd: zero pixel clock is not a timing");
	}

	/* a block whose length runs past the collection ends the walk */
	{
		const uint8_t v[] = { 16 };

		make_cea(ext, v, sizeof(v));
		ext[2] = 6; /* collection ends inside the video block */
		check(edid_cea_vics(ext, vics, sizeof(vics)) == 0,
		      "vics: truncated collection is not read past");
	}

	/* ---- firmware answers ---- */
	make_base(base, dtd_1080, 0);
	make_answer(msg, base, 0);
	check(edid_parse_answer(msg, 0, out) == EDID_OK, "answer: ok");
	check(memcmp(out, base, 128) == 0, "answer: 128 bytes copied in order");
	check(edid_base_check(out) == EDID_OK, "answer: copied block is valid");

	make_answer(msg, base, 0);
	msg[EDID_W_STATUS] = 1;
	check(edid_parse_answer(msg, 0, out) == EDID_E_FW_STATUS, "answer: status != 0");

	make_answer(msg, base, 0);
	msg[EDID_W_CODE] = 0x80000001u;
	check(edid_parse_answer(msg, 0, out) == EDID_E_FW_CODE, "answer: error response code");

	make_answer(msg, base, 0);
	msg[EDID_W_REQ_RESP] &= ~EDID_TAG_RESPONSE_BIT;
	check(edid_parse_answer(msg, 0, out) == EDID_E_FW_TAG, "answer: response bit clear");

	make_answer(msg, base, 0);
	msg[EDID_W_TAG] = 0x00030021u;
	check(edid_parse_answer(msg, 0, out) == EDID_E_FW_TAG, "answer: other tag");

	make_answer(msg, base, 1);
	check(edid_parse_answer(msg, 0, out) == EDID_E_FW_BLOCK, "answer: other block number");
	check(edid_parse_answer(NULL, 0, out) == EDID_E_ARG, "answer: null message");

	printf("%d of %d checks, %d failed\n", total - failed, total, failed);
	return failed ? 1 : 0;
}
