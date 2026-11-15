/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_screen_mock: driver/zephyr/lib/board/rpi5/rpi5_screen.c over
 * driver/zephyr/lib/board/rpi4/rpi_mbox.c, both unchanged, against a mocked
 * mailbox and a model of the firmware's framebuffer tags, built for the
 * BCM2712 (-DCONFIG_SOC_BCM2712) and, as a control, the BCM2711.
 *
 * Checked: the six GET tags go out one request each in the layout the
 * firmware reads, and their answers are kept per tag; a firmware that has a
 * framebuffer, one that answers zeros, one that does not answer the tags and
 * one that errors; the allocation's ARM address (no alias on the BCM2712)
 * and the pixel order the firmware set; the pattern's pixel words in both
 * orders, written through a pitch wider than the row and read back clean;
 * a corrupted pixel found with its index.
 *
 * Prints "rpi5_screen_mock <soc>: N of N checks, 0 failed"; exit 0 when none
 * failed.
 */
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

#include "rpi_mbox.c"
#include "rpi5_screen.c"

#ifdef CONFIG_SOC_BCM2712
#define SOC "bcm2712"
#define EXPECT_BASE 0x107c013880ull
#define EXPECT_ALIAS 0x00000000u
#else
#define SOC "bcm2711"
#define EXPECT_BASE 0xfe00b880ull
#define EXPECT_ALIAS 0xc0000000u
#endif

static uint8_t regs[0x40] __attribute__((aligned(64)));
static uint8_t fbmem[64 * 32 * 4 + 4096];

static uintptr_t buf_phys = 0x00100000u;

enum fw_mode { FW_FB, FW_ZEROS, FW_NO_TAG_ACK, FW_ERROR };
static enum fw_mode mode;
static uint32_t mail0[2];
static int nmail0, nwritten, layout_bad, bad_access;
static uint32_t seen_tags[16];
static int nseen;
static uint32_t fb_w, fb_h;
static uint32_t order_set = 1;
#define FB_PHYS 0x3e000000u

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

void device_map(mm_reg_t *virt, unsigned long long phys, unsigned long size, unsigned int flags)
{
	(void)size;
	(void)flags;
	*virt = phys == EXPECT_BASE ? (mm_reg_t)regs : (mm_reg_t)fbmem;
}

uintptr_t k_mem_phys_addr(void *virt)
{
	(void)virt;
	return buf_phys;
}

int sys_cache_data_flush_range(void *addr, size_t size)
{
	(void)addr;
	(void)size;
	return 0;
}

int sys_cache_data_invd_range(void *addr, size_t size)
{
	(void)addr;
	(void)size;
	return 0;
}

void k_busy_wait(uint32_t usec)
{
	(void)usec;
}

uint64_t k_cycle_get_64(void)
{
	static uint64_t cycles;
	return cycles += 10;
}

/* The firmware walks the tags as raspberrypi.c:100-131 lays them out. */
static void firmware(void)
{
	uint32_t n = 2;

	if (buf[0] % 4 || buf[0] / 4 > ARRAY_SIZE(buf) || buf[1] != STATUS_REQUEST) {
		layout_bad++;
		return;
	}
	while (n < buf[0] / 4 && buf[n] != 0) {
		uint32_t tag = buf[n], bytes = buf[n + 1], *v = &buf[n + 3];

		if (bytes % 4 || buf[n + 2] != 0 || n + 3 + bytes / 4 >= buf[0] / 4) {
			layout_bad++;
			return;
		}
		if (nseen < (int)ARRAY_SIZE(seen_tags)) {
			seen_tags[nseen] = tag;
		}
		nseen++;
		switch (tag) {
		case 0x00040003: /* GET_PHYSICAL_WIDTH_HEIGHT */
			v[0] = mode == FW_FB ? 1920 : 0;
			v[1] = mode == FW_FB ? 1080 : 0;
			break;
		case 0x00040004: /* GET_VIRTUAL_WIDTH_HEIGHT */
			v[0] = mode == FW_FB ? 1920 : 0;
			v[1] = mode == FW_FB ? 1080 : 0;
			break;
		case 0x00040005: /* GET_DEPTH */
			v[0] = mode == FW_FB ? 32 : 0;
			break;
		case 0x00040006: /* GET_PIXEL_ORDER */
			v[0] = mode == FW_FB ? order_set : 0;
			break;
		case 0x00040008: /* GET_PITCH */
			v[0] = mode == FW_FB ? (fb_w ? fb_w * 4 : 1920 * 4) : 0;
			break;
		case 0x00048003: /* SET_PHYSICAL_WIDTH_HEIGHT */
			fb_w = v[0];
			fb_h = v[1];
			break;
		case 0x00048006: /* SET_PIXEL_ORDER */
			order_set = v[0];
			break;
		case 0x00040001: /* ALLOCATE */
			v[0] = FB_PHYS | EXPECT_ALIAS;
			v[1] = fb_w * fb_h * 4;
			break;
		case 0x00040009: /* GET_VIRTUAL_OFFSET */
			v[0] = 0;
			v[1] = 0;
			break;
		default:
			break;
		}
		if (mode != FW_NO_TAG_ACK) {
			buf[n + 2] = TAG_RESPONSE | bytes;
		}
		n += 3 + bytes / 4;
	}
	if (n >= buf[0] / 4 || buf[n] != 0 || (n + 1) * 4 != buf[0]) {
		layout_bad++;
	}
	buf[1] = mode == FW_ERROR ? 0x80000001u : STATUS_SUCCESS;
}

uint32_t sys_read32(mm_reg_t addr)
{
	uint32_t off = (uint32_t)(addr - (mm_reg_t)regs);

	if (addr < (mm_reg_t)regs || off >= sizeof(regs)) {
		bad_access++;
		return 0;
	}
	switch (off) {
	case MAIL1_STA:
		return 0;
	case MAIL0_STA:
		return nmail0 ? 0 : MBOX_EMPTY;
	case MAIL0_RD:
		if (!nmail0) {
			bad_access++;
			return 0;
		}
		nmail0--;
		return mail0[0];
	default:
		bad_access++;
		return 0;
	}
}

void sys_write32(uint32_t val, mm_reg_t addr)
{
	uint32_t off = (uint32_t)(addr - (mm_reg_t)regs);

	if (addr < (mm_reg_t)regs || off != MAIL1_WRT) {
		bad_access++;
		return;
	}
	nwritten++;
	firmware();
	nmail0 = 0;
	mail0[nmail0++] = val;
}

static void reset(enum fw_mode m)
{
	mode = m;
	nwritten = nseen = nmail0 = layout_bad = 0;
}

int main(void)
{
	struct rpi5_fw_fb s;
	const uint32_t want_tags[RPI5_GET_COUNT] = { 0x00040003, 0x00040004, 0x00040005,
						     0x00040006, 0x00040008, 0x00040009 };

	CHECK(rpi_mbox_init() == 0, "rpi_mbox_init failed");

	/* 1. a firmware with a framebuffer: six requests, each answered */
	reset(FW_FB);
	CHECK(rpi5_screen_query(&s) == RPI5_GET_COUNT, "fewer than %d tags answered", RPI5_GET_COUNT);
	CHECK(nwritten == RPI5_GET_COUNT && nseen == RPI5_GET_COUNT && layout_bad == 0,
	      SOC ": %d requests, %d tags, %d unreadable", nwritten, nseen, layout_bad);
	for (int i = 0; i < RPI5_GET_COUNT && i < nseen; i++) {
		CHECK(seen_tags[i] == want_tags[i], "request %d asked tag %#x, expected %#x", i, seen_tags[i],
		      want_tags[i]);
	}
	CHECK(rpi5_screen_query_has_fb(&s) == 1, "no framebuffer in the answers");
	CHECK(s.val[RPI5_GET_PHYSICAL][0] == 1920 && s.val[RPI5_GET_PHYSICAL][1] == 1080 &&
		      s.val[RPI5_GET_DEPTH][0] == 32 && s.val[RPI5_GET_PITCH][0] == 7680 &&
		      s.val[RPI5_GET_ORDER][0] == order_set,
	      "answers %u x %u depth %u pitch %u order %u", s.val[RPI5_GET_PHYSICAL][0],
	      s.val[RPI5_GET_PHYSICAL][1], s.val[RPI5_GET_DEPTH][0], s.val[RPI5_GET_PITCH][0],
	      s.val[RPI5_GET_ORDER][0]);
	CHECK(s.code[RPI5_GET_PHYSICAL] == (TAG_RESPONSE | 8), "physical code %#x", s.code[RPI5_GET_PHYSICAL]);

	/* 2. a firmware that answers zeros: answered, but no framebuffer */
	reset(FW_ZEROS);
	CHECK(rpi5_screen_query(&s) == RPI5_GET_COUNT && rpi5_screen_query_has_fb(&s) == 0,
	      "zero size taken for a framebuffer");

	/* 3. tags not answered, and an error answer: no framebuffer, no answer counted */
	reset(FW_NO_TAG_ACK);
	CHECK(rpi5_screen_query(&s) == 0 && rpi5_screen_query_has_fb(&s) == 0 &&
		      (s.code[RPI5_GET_PHYSICAL] & TAG_RESPONSE) == 0,
	      "unanswered tags counted");
	reset(FW_ERROR);
	CHECK(rpi5_screen_query(&s) == 0 && s.rc[RPI5_GET_PHYSICAL] == -EIO, "error answer: rc %d",
	      s.rc[RPI5_GET_PHYSICAL]);

	/* 4. the allocation: ARM address, order as set */
	reset(FW_FB);
	struct rpi_fb fb = { 0 };
	CHECK(rpi_fb_alloc(64, 32, &fb) == 0, "rpi_fb_alloc failed");
	CHECK(fb.phys == FB_PHYS && fb.pitch == 64 * 4 && fb.order == 1 && (fb.order_code & TAG_RESPONSE) &&
		      fb.cpu == fbmem,
	      SOC ": fb phys %#x pitch %u order %u", fb.phys, fb.pitch, fb.order);

	/* 5. pixel words in both orders */
	CHECK(rpi5_pixel(1, 0x11, 0x22, 0x33) == 0x00332211u, "RGB order word %#x", rpi5_pixel(1, 0x11, 0x22, 0x33));
	CHECK(rpi5_pixel(0, 0x11, 0x22, 0x33) == 0x00112233u, "BGR order word %#x", rpi5_pixel(0, 0x11, 0x22, 0x33));
	CHECK(rpi5_pattern_at(1, 64, 32, 0, 0) == 0x00ffffffu && rpi5_pattern_at(1, 64, 32, 63, 0) == 0x00000000u &&
		      rpi5_pattern_at(1, 64, 32, 40, 0) == rpi5_pixel(1, 255, 0, 0),
	      "bars: first %#x last %#x sixth %#x", rpi5_pattern_at(1, 64, 32, 0, 0),
	      rpi5_pattern_at(1, 64, 32, 63, 0), rpi5_pattern_at(1, 64, 32, 40, 0));
	CHECK(rpi5_pattern_at(1, 64, 32, 0, 31) == 0 && rpi5_pattern_at(1, 64, 32, 63, 31) == 0x000000ffu,
	      "ramp ends %#x %#x", rpi5_pattern_at(1, 64, 32, 0, 31), rpi5_pattern_at(1, 64, 32, 63, 31));
	CHECK(rpi5_pattern_at(1, 0, 32, 0, 0) == 0, "empty area");

	/* 6. write and read back, with a pitch wider than the row */
	uint32_t pitch = 64 * 4 + 64, bad = 99, first = 0;

	memset(fbmem, 0xa5, sizeof(fbmem));
	rpi5_pattern_fill(fbmem, pitch, 1, 64, 0, 16);
	bad = rpi5_pattern_verify(fbmem, pitch, 1, 64, 0, 16, &first);
	CHECK(bad == 0 && first == UINT32_MAX, "clean readback: %u wrong, first %u", bad, first);
	CHECK(((uint32_t *)fbmem)[64] == 0xa5a5a5a5u, "the padding past the row was written");
	bad = rpi5_pattern_verify(fbmem, pitch, 0, 64, 0, 16, &first);
	CHECK(bad > 0, "the other pixel order read back as the same");

	/* 7. a second area below the first, and a corrupted pixel */
	rpi5_pattern_fill(fbmem, pitch, 1, 64, 16, 32);
	CHECK(rpi5_pattern_verify(fbmem, pitch, 1, 64, 16, 32, NULL) == 0, "second area");
	((uint32_t *)(fbmem + (size_t)20 * pitch))[7] ^= 0x00010000u;
	bad = rpi5_pattern_verify(fbmem, pitch, 1, 64, 16, 32, &first);
	CHECK(bad == 1 && first == (20u - 16u) * 64u + 7u, "corrupt pixel: %u wrong, first %u", bad, first);
	CHECK(rpi5_pattern_verify(fbmem, pitch, 1, 64, 0, 16, NULL) == 0, "the first area changed");

	CHECK(bad_access == 0, "%d accesses outside the mailbox registers", bad_access);

	printf("rpi5_screen_mock %s: %d of %d checks, %d failed\n", SOC, checks - failed, checks, failed);
	return failed != 0;
}
