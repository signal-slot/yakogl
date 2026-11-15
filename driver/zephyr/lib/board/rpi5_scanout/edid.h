/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * edid.h: header-only EDID reader and mode choice for the Pi 5 screen.
 *
 * C99, static inline, no Zephyr includes, no allocation. It has its own
 * constants and does not include rpi4/rpi_mbox.h.
 *
 * Sources (facts only; no Linux code is copied):
 *  - FW:   the Raspberry Pi firmware wiki, "Mailbox property interface"
 *          (github.com/raspberrypi/firmware/wiki/Mailbox-property-interface),
 *          tag table entry "Get EDID block", 0x00030020, and the property
 *          buffer layout section (buffer size, request code, tags, end tag;
 *          bit 31 of a tag's value length is the response bit).
 *          rpi:include/soc/bcm2835/raspberrypi-firmware.h names the tag
 *          RPI_FIRMWARE_GET_EDID_BLOCK in enum rpi_firmware_property_tag
 *          (the 0x0003xxxx "get" group, next to GET_CLOCK_RATE_MEASURED); the
 *          line is not verified: no Linux checkout was available.
 *  - EDID: VESA E-EDID 1.4 (base block: 8-byte header, version, detailed
 *          timing descriptors, extension count, checksum) and CTA-861
 *          (extension tag 0x02, data block collection, video data block with
 *          short video descriptors). Offsets below are those documents'.
 */
#ifndef YAKOGL_RPI5_SCANOUT_EDID_H
#define YAKOGL_RPI5_SCANOUT_EDID_H

#include <stdint.h>

/* ---- property mailbox: GET_EDID_BLOCK ------------------------------------ */

/* FW wiki, property buffer: request code 0, firmware's answer 0x80000000. */
#define EDID_FW_REQUEST 0x00000000u
#define EDID_FW_RESPONSE_OK 0x80000000u
/* FW wiki, tag table: Get EDID block. */
#define EDID_TAG_GET_EDID_BLOCK 0x00030020u
/* FW wiki, tag format: bit 31 of the value length word marks a response. */
#define EDID_TAG_RESPONSE_BIT 0x80000000u
/* FW wiki, Get EDID block: request u32 block number; response u32 block
 * number, u32 status (0 = ok), 128 bytes of EDID: value buffer 8 + 128. */
#define EDID_BLOCK_BYTES 128u
#define EDID_TAG_REQ_BYTES 136u
#define EDID_TAG_RESP_BYTES 136u
/* Property buffer in u32 words: size, code, tag, value bytes, request/response
 * length, then 34 value words (block, status, 32 data), end tag. */
#define EDID_MSG_WORDS 40u
#define EDID_MSG_BYTES (EDID_MSG_WORDS * 4u)

/* Word layout inside the buffer (FW wiki buffer layout). */
#define EDID_W_SIZE 0u
#define EDID_W_CODE 1u
#define EDID_W_TAG 2u
#define EDID_W_VALUE_BYTES 3u
#define EDID_W_REQ_RESP 4u
#define EDID_W_BLOCK 5u
#define EDID_W_STATUS 6u
#define EDID_W_DATA 7u
#define EDID_W_END (EDID_W_DATA + 32u)

enum edid_status {
	EDID_OK = 0,
	EDID_E_ARG = -1,       /* a null or too short argument */
	EDID_E_FW_CODE = -2,   /* buffer response code is not 0x80000000 */
	EDID_E_FW_TAG = -3,    /* tag id differs or its response bit is clear */
	EDID_E_FW_STATUS = -4, /* the tag's status word is nonzero */
	EDID_E_FW_BLOCK = -5,  /* the answer is for another block number */
	EDID_E_HEADER = -6,    /* base block header is not 00 FF*6 00 */
	EDID_E_CHECKSUM = -7,  /* the 128 bytes do not sum to 0 mod 256 */
	EDID_E_VERSION = -8,   /* EDID version is not 1.x */
	EDID_E_NO_TIMING = -9, /* no usable detailed timing descriptor */
	EDID_E_NO_EXT = -10,   /* no CEA-861 extension block */
};

/* Build a GET_EDID_BLOCK request for block `block` into msg[EDID_MSG_WORDS].
 * Returns the message size in bytes, or EDID_E_ARG. */
static inline int edid_build_request(uint32_t *msg, uint32_t block)
{
	uint32_t i;

	if (!msg)
		return EDID_E_ARG;
	for (i = 0; i < EDID_MSG_WORDS; i++)
		msg[i] = 0;
	msg[EDID_W_SIZE] = EDID_MSG_BYTES;
	msg[EDID_W_CODE] = EDID_FW_REQUEST;
	msg[EDID_W_TAG] = EDID_TAG_GET_EDID_BLOCK;
	msg[EDID_W_VALUE_BYTES] = EDID_TAG_RESP_BYTES;
	msg[EDID_W_REQ_RESP] = EDID_TAG_REQ_BYTES; /* request: bit 31 clear */
	msg[EDID_W_BLOCK] = block;
	msg[EDID_W_END] = 0; /* end tag */
	return (int)EDID_MSG_BYTES;
}

/* Parse the firmware's answer: checks the buffer response code, the tag id and
 * its response bit, the status word and the block number; copies the 128
 * bytes (little endian words) into out[128]. */
static inline int edid_parse_answer(const uint32_t *msg, uint32_t block,
				    uint8_t *out)
{
	uint32_t i, w;

	if (!msg || !out)
		return EDID_E_ARG;
	if (msg[EDID_W_CODE] != EDID_FW_RESPONSE_OK)
		return EDID_E_FW_CODE;
	if (msg[EDID_W_TAG] != EDID_TAG_GET_EDID_BLOCK ||
	    !(msg[EDID_W_REQ_RESP] & EDID_TAG_RESPONSE_BIT))
		return EDID_E_FW_TAG;
	if (msg[EDID_W_STATUS] != 0)
		return EDID_E_FW_STATUS;
	if (msg[EDID_W_BLOCK] != block)
		return EDID_E_FW_BLOCK;
	for (i = 0; i < EDID_BLOCK_BYTES / 4u; i++) {
		w = msg[EDID_W_DATA + i];
		out[4u * i + 0u] = (uint8_t)w;
		out[4u * i + 1u] = (uint8_t)(w >> 8);
		out[4u * i + 2u] = (uint8_t)(w >> 16);
		out[4u * i + 3u] = (uint8_t)(w >> 24);
	}
	return EDID_OK;
}

/* ---- EDID base block ------------------------------------------------------ */

/* VESA E-EDID 1.4 section 3.3: bytes 0..7 header 00 FF FF FF FF FF FF 00. */
#define EDID_OFF_HEADER 0u
/* section 3.4: byte 18 version, byte 19 revision. */
#define EDID_OFF_VERSION 18u
#define EDID_OFF_REVISION 19u
/* section 3.10: four 18-byte descriptors at 54, 72, 90, 108. */
#define EDID_OFF_DESC0 54u
#define EDID_DESC_BYTES 18u
#define EDID_DESC_COUNT 4u
/* section 3.11: byte 126 extension block count, byte 127 checksum. */
#define EDID_OFF_EXT_COUNT 126u
#define EDID_OFF_CHECKSUM 127u

/* Detailed timing descriptor, VESA E-EDID 1.4 table 3.21 (offsets in it). */
#define EDID_DT_PIXCLK 0u    /* u16 LE, 10 kHz units; 0 = not a timing */
#define EDID_DT_HACT_LO 2u
#define EDID_DT_HBLANK_LO 3u
#define EDID_DT_HHI 4u       /* [7:4] hactive[11:8], [3:0] hblank[11:8] */
#define EDID_DT_VACT_LO 5u
#define EDID_DT_VBLANK_LO 6u
#define EDID_DT_VHI 7u       /* [7:4] vactive[11:8], [3:0] vblank[11:8] */
#define EDID_DT_HFP_LO 8u    /* h front porch (sync offset) */
#define EDID_DT_HSYNC_LO 9u  /* h sync width */
#define EDID_DT_VFPSYNC_LO 10u /* [7:4] v front porch, [3:0] v sync width */
#define EDID_DT_SYNCHI 11u   /* [7:6] hfp[9:8], [5:4] hsync[9:8],
			      * [3:2] vfp[5:4], [1:0] vsync[5:4] */
#define EDID_DT_FLAGS 17u    /* [4:3]=11 digital separate, [2] vsync +,
			      * [1] hsync + */
#define EDID_DT_FLAG_HSYNC_POS 0x02u
#define EDID_DT_FLAG_VSYNC_POS 0x04u
#define EDID_DT_FLAG_SEPARATE_MASK 0x18u
#define EDID_DT_FLAG_SEPARATE 0x18u

/* ---- CTA-861 extension block --------------------------------------------- */

/* CTA-861: byte 0 tag 0x02, byte 1 revision, byte 2 offset d of the first
 * detailed timing descriptor (= end of the data block collection),
 * data blocks from byte 4. */
#define EDID_CEA_TAG 0x02u
#define EDID_CEA_OFF_DTD 2u
#define EDID_CEA_OFF_BLOCKS 4u
/* Data block header: [7:5] tag code, [4:0] payload length. Tag 2 = video. */
#define EDID_CEA_BLK_VIDEO 2u
/* Short video descriptor: [6:0] VIC; bit 7 = native, for VICs 1..64. */
#define EDID_SVD_VIC_MASK 0x7fu

#define EDID_VIC_640X480_60 1u
#define EDID_VIC_1280X720_60 4u
#define EDID_VIC_1920X1080_60 16u

/* Decoded timing. Pixel clock in kHz; porches and syncs in pixels / lines. */
struct edid_timing {
	uint32_t pixclk_khz;
	uint16_t h_active, h_fp, h_sync, h_bp;
	uint16_t v_active, v_fp, v_sync, v_bp;
	uint8_t hsync_pos; /* 1 = positive polarity */
	uint8_t vsync_pos;
};

/* Mode choice result. */
struct edid_mode {
	struct edid_timing t;
	uint8_t vic;       /* CEA VIC, 0 when the preferred timing was taken */
	uint8_t from_edid; /* 1 = chosen with an EDID, 0 = the 640x480 fallback */
};

static inline int edid_base_check(const uint8_t *b)
{
	static const uint8_t hdr[8] = { 0x00, 0xff, 0xff, 0xff,
					0xff, 0xff, 0xff, 0x00 };
	uint32_t i, sum = 0;

	if (!b)
		return EDID_E_ARG;
	for (i = 0; i < 8; i++)
		if (b[EDID_OFF_HEADER + i] != hdr[i])
			return EDID_E_HEADER;
	for (i = 0; i < EDID_BLOCK_BYTES; i++)
		sum += b[i];
	if ((sum & 0xffu) != 0)
		return EDID_E_CHECKSUM;
	if (b[EDID_OFF_VERSION] != 1u)
		return EDID_E_VERSION;
	return EDID_OK;
}

/* Number of extension blocks the base block announces. */
static inline int edid_ext_count(const uint8_t *b)
{
	return b ? (int)b[EDID_OFF_EXT_COUNT] : EDID_E_ARG;
}

/* Decode one 18-byte detailed timing descriptor. */
static inline int edid_decode_dtd(const uint8_t *d, struct edid_timing *t)
{
	uint32_t clk, hb, vb;

	if (!d || !t)
		return EDID_E_ARG;
	clk = (uint32_t)d[EDID_DT_PIXCLK] | ((uint32_t)d[EDID_DT_PIXCLK + 1] << 8);
	if (clk == 0)
		return EDID_E_NO_TIMING;
	t->pixclk_khz = clk * 10u;
	t->h_active = (uint16_t)(d[EDID_DT_HACT_LO] |
				 ((d[EDID_DT_HHI] & 0xf0u) << 4));
	hb = d[EDID_DT_HBLANK_LO] | ((uint32_t)(d[EDID_DT_HHI] & 0x0fu) << 8);
	t->v_active = (uint16_t)(d[EDID_DT_VACT_LO] |
				 ((d[EDID_DT_VHI] & 0xf0u) << 4));
	vb = d[EDID_DT_VBLANK_LO] | ((uint32_t)(d[EDID_DT_VHI] & 0x0fu) << 8);
	t->h_fp = (uint16_t)(d[EDID_DT_HFP_LO] |
			     (((uint32_t)d[EDID_DT_SYNCHI] & 0xc0u) << 2));
	t->h_sync = (uint16_t)(d[EDID_DT_HSYNC_LO] |
			       (((uint32_t)d[EDID_DT_SYNCHI] & 0x30u) << 4));
	t->v_fp = (uint16_t)(((d[EDID_DT_VFPSYNC_LO] >> 4) & 0x0fu) |
			     (((uint32_t)d[EDID_DT_SYNCHI] & 0x0cu) << 2));
	t->v_sync = (uint16_t)((d[EDID_DT_VFPSYNC_LO] & 0x0fu) |
			       (((uint32_t)d[EDID_DT_SYNCHI] & 0x03u) << 4));
	if (hb < (uint32_t)t->h_fp + t->h_sync || vb < (uint32_t)t->v_fp + t->v_sync)
		return EDID_E_NO_TIMING;
	t->h_bp = (uint16_t)(hb - t->h_fp - t->h_sync);
	t->v_bp = (uint16_t)(vb - t->v_fp - t->v_sync);
	if (t->h_active == 0 || t->v_active == 0)
		return EDID_E_NO_TIMING;
	if ((d[EDID_DT_FLAGS] & EDID_DT_FLAG_SEPARATE_MASK) ==
	    EDID_DT_FLAG_SEPARATE) {
		t->hsync_pos = (d[EDID_DT_FLAGS] & EDID_DT_FLAG_HSYNC_POS) != 0;
		t->vsync_pos = (d[EDID_DT_FLAGS] & EDID_DT_FLAG_VSYNC_POS) != 0;
	} else {
		t->hsync_pos = t->vsync_pos = 0;
	}
	return EDID_OK;
}

/* The preferred timing is the first descriptor (E-EDID 1.4, section 3.10.2). */
static inline int edid_preferred_timing(const uint8_t *b, struct edid_timing *t)
{
	if (!b || !t)
		return EDID_E_ARG;
	return edid_decode_dtd(b + EDID_OFF_DESC0, t);
}

/* Walk the CEA-861 extension's short video descriptors. Writes up to `max`
 * VICs (native bit stripped) into vics and returns how many the block holds,
 * or a negative status (EDID_E_NO_EXT when the block is not tag 0x02). */
static inline int edid_cea_vics(const uint8_t *ext, uint8_t *vics, uint32_t max)
{
	uint32_t pos, end, len, i;
	int n = 0;

	if (!ext)
		return EDID_E_ARG;
	if (ext[0] != EDID_CEA_TAG)
		return EDID_E_NO_EXT;
	end = ext[EDID_CEA_OFF_DTD];
	if (end < EDID_CEA_OFF_BLOCKS)
		return 0; /* no data block collection */
	if (end > EDID_OFF_CHECKSUM)
		end = EDID_OFF_CHECKSUM; /* byte 127 is the block checksum */
	pos = EDID_CEA_OFF_BLOCKS;
	while (pos < end) {
		len = ext[pos] & 0x1fu;
		if (pos + 1u + len > end)
			break; /* a block running past the collection */
		if ((ext[pos] >> 5) == EDID_CEA_BLK_VIDEO) {
			for (i = 0; i < len; i++) {
				if (vics && (uint32_t)n < max)
					vics[n] = ext[pos + 1u + i] & EDID_SVD_VIC_MASK;
				n++;
			}
		}
		pos += 1u + len;
	}
	return n;
}

static inline int edid_has_vic(const uint8_t *vics, int n, uint32_t max, uint8_t v)
{
	int i;

	for (i = 0; i < n && (uint32_t)i < max; i++)
		if (vics[i] == v)
			return 1;
	return 0;
}

/* CEA-861 timings of the three preferred VICs (CTA-861 table of video
 * formats: 1 = 640x480p60, 4 = 1280x720p60, 16 = 1920x1080p60). */
static inline int edid_cea_timing(uint8_t vic, struct edid_timing *t)
{
	if (!t)
		return EDID_E_ARG;
	switch (vic) {
	case EDID_VIC_640X480_60:
		t->pixclk_khz = 25175;
		t->h_active = 640; t->h_fp = 16; t->h_sync = 96; t->h_bp = 48;
		t->v_active = 480; t->v_fp = 10; t->v_sync = 2; t->v_bp = 33;
		t->hsync_pos = 0; t->vsync_pos = 0;
		return EDID_OK;
	case EDID_VIC_1280X720_60:
		t->pixclk_khz = 74250;
		t->h_active = 1280; t->h_fp = 110; t->h_sync = 40; t->h_bp = 220;
		t->v_active = 720; t->v_fp = 5; t->v_sync = 5; t->v_bp = 20;
		t->hsync_pos = 1; t->vsync_pos = 1;
		return EDID_OK;
	case EDID_VIC_1920X1080_60:
		t->pixclk_khz = 148500;
		t->h_active = 1920; t->h_fp = 88; t->h_sync = 44; t->h_bp = 148;
		t->v_active = 1080; t->v_fp = 4; t->v_sync = 5; t->v_bp = 36;
		t->hsync_pos = 1; t->vsync_pos = 1;
		return EDID_OK;
	default:
		return EDID_E_ARG;
	}
}

#define EDID_MAX_PIXCLK_KHZ 600000u

/* Mode choice. base is the checked base block, ext the CEA block or NULL.
 * Prefers VIC 16, 4, 1 when the extension lists them, else the preferred
 * timing when its pixel clock is <= 600 MHz, else 640x480 (VIC 1, from_edid
 * 0). A bad base block gives the 640x480 fallback as well. Always returns a
 * mode. */
static inline struct edid_mode edid_choose_mode(const uint8_t *base,
						const uint8_t *ext)
{
	static const uint8_t pref[3] = { EDID_VIC_1920X1080_60,
					 EDID_VIC_1280X720_60,
					 EDID_VIC_640X480_60 };
	struct edid_mode m;
	uint8_t vics[64];
	int n, i;

	m.vic = 0;
	m.from_edid = 0;
	if (base && edid_base_check(base) == EDID_OK) {
		if (ext) {
			n = edid_cea_vics(ext, vics, sizeof(vics));
			if (n > 0) {
				for (i = 0; i < 3; i++) {
					if (edid_has_vic(vics, n, sizeof(vics), pref[i])) {
						(void)edid_cea_timing(pref[i], &m.t);
						m.vic = pref[i];
						m.from_edid = 1;
						return m;
					}
				}
			}
		}
		if (edid_preferred_timing(base, &m.t) == EDID_OK &&
		    m.t.pixclk_khz <= EDID_MAX_PIXCLK_KHZ) {
			m.from_edid = 1;
			return m;
		}
	}
	(void)edid_cea_timing(EDID_VIC_640X480_60, &m.t);
	m.vic = EDID_VIC_640X480_60;
	return m;
}

#endif /* YAKOGL_RPI5_SCANOUT_EDID_H */
