/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hvs6_dlist_mock: driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist.h against
 * channel registers and display-list memory built here, word by word, from
 * the field positions the header cites (rpi-6.12.y, vc4_regs.h and
 * vc4_plane.c). No register is touched: the words are plain arrays.
 *
 * Checked: VERSION to generation (C0, D0, anything else unknown); the
 * channel register offsets of both generations; an enabled, running channel
 * 0 and its list start; a disabled channel; a unity XRGB8888 plane (address,
 * pitch, size, position, order, rgb32); a scaled plane (POS1 present); the
 * end-of-list word; a word without VALID; NEXT too short and past the end; a
 * start outside the memory; an unknown format code; a tiled buffer; an
 * address above 4 GiB through PTR0's upper bits; the format names.
 *
 * Prints "hvs6_dlist_mock: N of N checks, 0 failed"; exit 0 when none failed.
 */
#include <stdio.h>
#include <string.h>

#include "hvs6_dlist.h"

static int checks, failed;
#define CHECK(cond, ...)                                                       \
	do {                                                                   \
		checks++;                                                      \
		if (!(cond)) {                                                 \
			failed++;                                              \
			printf("  FAIL line %d: ", __LINE__);                   \
			printf(__VA_ARGS__);                                   \
			printf("\n");                                          \
		}                                                              \
	} while (0)

/* The display-list memory: what a probe reads from HVS base + 0x4000. */
#define DL_WORDS 64u
static uint32_t dl[DL_WORDS];

/*
 * Write one single-plane linear entry at index at, in the order the header
 * describes: CTL0, POS0, CTL2, [POS1], POS2, POS3, PTR0..PTR2, END. Returns
 * the entry's length (its NEXT).
 */
static uint32_t put_plane(uint32_t at, uint32_t fmt, uint32_t order, bool unity, uint32_t x,
			  uint32_t y, uint32_t w, uint32_t h, uint64_t addr, uint32_t pitch)
{
	uint32_t i = at, n;

	dl[i++] = 0; /* CTL0, completed below */
	dl[i++] = (y << 16) | x;
	dl[i++] = 0x0000fff0u; /* CTL2: alpha 0xfff, not decoded */
	if (!unity)
		dl[i++] = ((h * 2u - 1u) << 16) | (w * 2u - 1u); /* POS1 */
	dl[i++] = ((h - 1u) << 16) | (w - 1u);
	dl[i++] = 0xc0c0c0c0u; /* POS3 context */
	dl[i++] = (uint32_t)(addr >> 32) & 0xffu;
	dl[i++] = (uint32_t)addr;
	dl[i++] = pitch;
	dl[i++] = 1u << 31; /* the entry's own END word */
	n = i - at;
	dl[at] = (1u << 30) | (n << 24) | (unity ? 1u << 15 : 0u) | (order << 13) | fmt;
	return n;
}

static void test_gen_and_offsets(void)
{
	CHECK(hvs6_gen_from_version(0x53) == HVS6_GEN_C, "0x53 is C0");
	CHECK(hvs6_gen_from_version(0xabcd0054u) == HVS6_GEN_D, "0x54 in bits 7:0 is D0");
	CHECK(hvs6_gen_from_version(0x52) == HVS6_GEN_UNKNOWN, "0x52 unknown");
	CHECK(hvs6_gen_from_version(0) == HVS6_GEN_UNKNOWN, "0 unknown");

	CHECK(hvs6_chan_reg_offset(HVS6_GEN_C, 0, HVS6_REG_CTRL0) == 0x30, "C ctrl0");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_C, 0, HVS6_REG_LPTRS) == 0x3c, "C lptrs");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_C, 0, HVS6_REG_STATUS) == 0x44, "C status");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_C, 0, HVS6_REG_DL) == 0x48, "C dl");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_C, 1, HVS6_REG_CTRL0) == 0x50, "C disp1 ctrl0");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_C, 2, HVS6_REG_DL) == 0x88, "C disp2 dl");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_D, 0, HVS6_REG_CTRL0) == 0x100, "D ctrl0");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_D, 0, HVS6_REG_LPTRS) == 0x110, "D lptrs");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_D, 0, HVS6_REG_STATUS) == 0x118, "D status");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_D, 0, HVS6_REG_DL) == 0x11c, "D dl");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_D, 1, HVS6_REG_CTRL0) == 0x140, "D disp1 ctrl0");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_D, 2, HVS6_REG_DL) == 0x19c, "D disp2 dl");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_D, 3, HVS6_REG_CTRL0) == 0, "no channel 3");
	CHECK(hvs6_chan_reg_offset(HVS6_GEN_UNKNOWN, 0, HVS6_REG_CTRL0) == 0, "unknown gen");
	CHECK(HVS6_DLIST_OFFSET == 0x4000u && HVS6_CXM_SIZE == 4u, "dlist memory and its size");
}

static void test_channel(void)
{
	struct hvs6_chan c;
	/* enabled, FWIDTH 1920, LINES 1080 */
	uint32_t ctrl0 = (1u << 31) | (1920u << 16) | 1080u;
	/* MODE RUN, FRCNT 5, YLINE 123 */
	uint32_t status = (2u << 13) | (5u << 16) | 123u;

	hvs6_chan_decode(ctrl0, 3u, status, 3u, &c);
	CHECK(c.enabled && c.running && !c.empty, "channel 0 enabled and running");
	CHECK(c.mode == HVS6_STATUS_MODE_RUN && c.frcnt == 5 && c.yline == 123,
	      "status: mode %u frcnt %u yline %u", c.mode, c.frcnt, c.yline);
	CHECK(c.fwidth == 1920 && c.lines == 1080, "ctrl0: %ux%u", c.fwidth, c.lines);
	CHECK(c.head == 3 && c.active == 3, "list start: head %u active %u", c.head, c.active);

	/* fields above the masks are dropped */
	hvs6_chan_decode(ctrl0, 0xfffff005u, status, 0xfffff007u, &c);
	CHECK(c.head == 5 && c.active == 7, "HEADE/LACT masks: %u %u", c.head, c.active);

	/* disabled: no ENB, MODE DISABLED, FIFO empty */
	hvs6_chan_decode(0, 0, 1u << 22, 0, &c);
	CHECK(!c.enabled && !c.running && c.empty && c.mode == HVS6_STATUS_MODE_DISABLED,
	      "disabled channel");
	/* EOF: enabled bit left but not running */
	hvs6_chan_decode(1u << 31, 0, 3u << 13, 0, &c);
	CHECK(c.enabled && !c.running && c.mode == HVS6_STATUS_MODE_EOF, "EOF is not running");
	hvs6_chan_decode(1u << 31, 0, 1u << 13, 0, &c);
	CHECK(c.running && c.mode == HVS6_STATUS_MODE_INIT, "INIT counts as running");
}

static void test_xrgb8888(void)
{
	struct hvs6_plane p;
	uint32_t n, start = 3;

	memset(dl, 0, sizeof(dl));
	n = put_plane(start, HVS6_FMT_RGBA8888, HVS6_ORDER_ARGB, true, 0, 0, 1920, 1080,
		      0x3e000000u, 1920u * 4u);
	dl[start + n] = 1u << 31; /* the list's END */

	CHECK(n == 9, "unity entry is 9 words, %u", n);
	CHECK(hvs6_dlist_first_plane(dl, DL_WORDS, start, &p) == HVS6_OK, "XRGB8888 decodes");
	CHECK(p.addr == 0x3e000000u && p.pitch == 7680 && p.width == 1920 && p.height == 1080,
	      "addr %#llx pitch %u %ux%u", (unsigned long long)p.addr, p.pitch, p.width, p.height);
	CHECK(p.format_code == HVS6_FMT_RGBA8888 && p.rgb32 && p.order == HVS6_ORDER_ARGB,
	      "format %u rgb32 %d order %u", p.format_code, p.rgb32, p.order);
	CHECK(p.x == 0 && p.y == 0 && p.unity && p.nwords == 9, "pos %u,%u unity %d n %u", p.x,
	      p.y, p.unity, p.nwords);
	CHECK(hvs6_dlist_first_plane(dl, DL_WORDS, start + n, &p) == HVS6_ERR_END,
	      "the next entry is the end of the list");
}

static void test_scaled_rgb565(void)
{
	struct hvs6_plane p;
	uint32_t n;

	memset(dl, 0, sizeof(dl));
	n = put_plane(0, HVS6_FMT_RGB565, 2, false, 100, 50, 640, 480, 0x10000000u, 1280);
	CHECK(n == 10, "scaled entry is 10 words, %u", n);
	CHECK(hvs6_dlist_first_plane(dl, DL_WORDS, 0, &p) == HVS6_OK, "RGB565 decodes");
	CHECK(p.width == 640 && p.height == 480 && p.pitch == 1280 && p.addr == 0x10000000u,
	      "POS1 skipped: %ux%u pitch %u", p.width, p.height, p.pitch);
	CHECK(p.x == 100 && p.y == 50 && !p.unity && !p.rgb32, "pos %u,%u", p.x, p.y);
}

static void test_above_4g(void)
{
	struct hvs6_plane p;

	memset(dl, 0, sizeof(dl));
	put_plane(32, HVS6_FMT_RGBA8888, HVS6_ORDER_ABGR, true, 0, 0, 1280, 720,
		  0xab12345600ull, 5120);
	CHECK(hvs6_dlist_first_plane(dl, DL_WORDS, 32, &p) == HVS6_OK, "40-bit plane decodes");
	CHECK(p.addr == 0xab12345600ull, "40-bit address %#llx", (unsigned long long)p.addr);
	/* bits of PTR0 above UPPER_ADDR (VFLIP, UPM fields) do not reach the address */
	dl[32 + 5] |= 0x9fff0000u;
	CHECK(hvs6_dlist_first_plane(dl, DL_WORDS, 32, &p) == HVS6_OK &&
		      p.addr == 0xab12345600ull,
	      "PTR0 upper bits only: %#llx", (unsigned long long)p.addr);
}

static void test_errors(void)
{
	struct hvs6_plane p;
	uint32_t n;

	memset(dl, 0, sizeof(dl));
	dl[0] = 1u << 31;
	CHECK(hvs6_dlist_first_plane(dl, DL_WORDS, 0, &p) == HVS6_ERR_END, "empty list");
	CHECK(p.addr == 0 && p.width == 0, "cleared on error");
	CHECK(hvs6_dlist_first_plane(dl, DL_WORDS, 1, &p) == HVS6_ERR_INVALID, "zero word");
	CHECK(hvs6_dlist_first_plane(dl, DL_WORDS, DL_WORDS, &p) == HVS6_ERR_RANGE,
	      "start at the end");
	CHECK(hvs6_dlist_first_plane(NULL, DL_WORDS, 0, &p) == HVS6_ERR_RANGE, "no memory");

	n = put_plane(0, HVS6_FMT_RGBA8888, 2, true, 0, 0, 64, 64, 0x1000, 256);
	CHECK(hvs6_dlist_first_plane(dl, n - 1, 0, &p) == HVS6_ERR_SIZE, "entry past the end");
	CHECK(hvs6_dlist_first_plane(dl, n, 0, &p) == HVS6_OK, "entry fits exactly");
	dl[0] = (dl[0] & ~(0x3fu << 24)) | (7u << 24);
	CHECK(hvs6_dlist_first_plane(dl, DL_WORDS, 0, &p) == HVS6_ERR_SIZE, "NEXT 7 too short");

	put_plane(0, HVS6_FMT_RGBA8888, 2, false, 0, 0, 64, 64, 0x1000, 256);
	dl[0] = (dl[0] & ~(0x3fu << 24)) | (8u << 24);
	CHECK(hvs6_dlist_first_plane(dl, DL_WORDS, 0, &p) == HVS6_ERR_SIZE,
	      "NEXT 8 too short when scaled");

	put_plane(0, 20, 0, true, 0, 0, 64, 64, 0x1000, 256);
	CHECK(hvs6_dlist_first_plane(dl, DL_WORDS, 0, &p) == HVS6_ERR_FORMAT, "format 20");

	put_plane(0, HVS6_FMT_RGBA8888, 2, true, 0, 0, 64, 64, 0x1000, 256);
	dl[0] |= 1u << 20; /* ADDR_MODE 128B */
	CHECK(hvs6_dlist_first_plane(dl, DL_WORDS, 0, &p) == HVS6_ERR_TILED, "SAND128 refused");
}

static void test_names(void)
{
	CHECK(strcmp(hvs6_format_name(HVS6_FMT_RGBA8888), "RGBA8888") == 0, "RGBA8888 name");
	CHECK(strcmp(hvs6_format_name(HVS6_FMT_RGB565), "RGB565") == 0, "RGB565 name");
	CHECK(strcmp(hvs6_format_name(31), "YCBCR_YUV420_9_0_3PLANE") == 0, "code 31 name");
	CHECK(hvs6_format_name(18) == NULL && hvs6_format_name(23) == NULL, "18..23 unnamed");
	CHECK(hvs6_format_name(32) == NULL && hvs6_format_name(0xffffffffu) == NULL,
	      "out of range");
	for (uint32_t c = 0; c < 32; c++)
		CHECK((hvs6_format_name(c) != NULL) == (c < 18 || c >= 24), "code %u", c);
}

int main(void)
{
	test_gen_and_offsets();
	test_channel();
	test_xrgb8888();
	test_scaled_rgb565();
	test_above_4g();
	test_errors();
	test_names();
	printf("hvs6_dlist_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}
