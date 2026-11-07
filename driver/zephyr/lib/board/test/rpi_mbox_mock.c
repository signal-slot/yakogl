/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi_mbox_mock: driver/zephyr/lib/board/rpi4/rpi_mbox.c, unchanged, against
 * a mocked mailbox and a model of the firmware's property channel. The file is
 * #included so the test reaches its message buffer; the Zephyr symbols it
 * needs are the stubs under stub/zephyr/.
 *
 * One build per SoC (the SoC is a compile-time parameter of rpi_mbox.c):
 *   -DCONFIG_SOC_BCM2712   the Pi 5
 *   nothing                the Pi 4 (BCM2711)
 * Checked: where the mailbox is mapped; the word written to MAIL1 (the
 * buffer's bus address: the ARM physical address on the BCM2712, the
 * 0xc0000000 alias on the BCM2711, with channel 8); the cache flush before
 * and the invalidate after; the property buffer's layout as the firmware
 * reads it; the clock tags (V3D clock 5: state, rate, max, set, measured);
 * the framebuffer's ARM address and its Normal NC mapping; -ERANGE for a
 * buffer above 4 GiB with nothing written; -EIO for an error answer or an
 * unanswered tag; a message on another channel skipped; -ETIMEDOUT when the
 * firmware never answers.
 *
 * Prints "rpi_mbox_mock <soc>: N of N checks, 0 failed"; exit 0 when none failed.
 */
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

#include "rpi_mbox.c"

#ifdef CONFIG_SOC_BCM2712
#define SOC "bcm2712"
#define EXPECT_BASE 0x107c013880ull
#define EXPECT_ALIAS 0x00000000u
#else
#define SOC "bcm2711"
#define EXPECT_BASE 0xfe00b880ull
#define EXPECT_ALIAS 0xc0000000u
#endif

/* ---- the mock ------------------------------------------------------------ */

static uint8_t regs[0x40] __attribute__((aligned(64)));
static uint8_t fbmem[0x1000];
static unsigned long long map_phys[4];
static unsigned long map_size[4];
static unsigned int map_flags[4];
static int map_calls;

static uintptr_t buf_phys = 0x00100000u; /* what k_mem_phys_addr says of buf */
static int flushed, invalidated, flush_before_write;
static int bad_access;

/* the firmware model */
enum fw_mode { FW_OK, FW_ERROR, FW_NO_TAG_ACK, FW_SILENT, FW_OTHER_CHAN_FIRST };
static enum fw_mode mode;
static uint32_t written[16];
static int nwritten;
static uint32_t mail0[2];
static int nmail0;
static int layout_bad;
static uint32_t fb_w, fb_h;
#define FB_PHYS 0x3e000000u
#define V3D_HZ 960000000u
#define V3D_MAX_HZ 1000000000u
static uint32_t set_hz, set_state_id, set_state_on;

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
	if (map_calls < 4) {
		map_phys[map_calls] = phys;
		map_size[map_calls] = size;
		map_flags[map_calls] = flags;
	}
	map_calls++;
	*virt = phys == EXPECT_BASE ? (mm_reg_t)regs : (mm_reg_t)fbmem;
}

uintptr_t k_mem_phys_addr(void *virt)
{
	CHECK(virt == (void *)buf, "k_mem_phys_addr of %p, not the message buffer", virt);
	return buf_phys;
}

int sys_cache_data_flush_range(void *addr, size_t size)
{
	CHECK(addr == (void *)buf && size == sizeof(buf), "flush of %p + %zu", addr, size);
	flushed++;
	return 0;
}

int sys_cache_data_invd_range(void *addr, size_t size)
{
	CHECK(addr == (void *)buf && size == sizeof(buf), "invalidate of %p + %zu", addr, size);
	invalidated++;
	return 0;
}

void k_busy_wait(uint32_t usec)
{
	(void)usec;
}

static uint64_t cycles;
uint64_t k_cycle_get_64(void)
{
	return cycles += 10;
}

/* The firmware answers a property buffer: it walks the tags from word 2 to
 * the end tag, as raspberrypi.c:100-131 lays them out, and answers each. */
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
		switch (tag) {
		case 0x00030001: /* GET_CLOCK_STATE */
			v[1] = v[0] == RPI_FW_CLK_V3D ? 1 : 2;
			break;
		case 0x00030002: /* GET_CLOCK_RATE */
			v[1] = v[0] == RPI_FW_CLK_V3D ? V3D_HZ : 0;
			break;
		case 0x00030004: /* GET_MAX_CLOCK_RATE */
			v[1] = v[0] == RPI_FW_CLK_V3D ? V3D_MAX_HZ : 0;
			break;
		case 0x00030047: /* GET_CLOCK_RATE_MEASURED */
			v[1] = v[0] == RPI_FW_CLK_V3D ? V3D_HZ - 1000 : 0;
			break;
		case 0x00038001: /* SET_CLOCK_STATE */
			set_state_id = v[0];
			set_state_on = v[1];
			break;
		case 0x00038002: /* SET_CLOCK_RATE */
			set_hz = v[1];
			break;
		case 0x00048003: /* SET_PHYSICAL_WIDTH_HEIGHT */
			fb_w = v[0];
			fb_h = v[1];
			break;
		case 0x00040001: /* ALLOCATE */
			v[0] = FB_PHYS | EXPECT_ALIAS;
			v[1] = fb_w * fb_h * 4;
			break;
		case 0x00040008: /* GET_PITCH */
			v[0] = fb_w * 4;
			break;
		default: /* the other SET_* tags echo their values */
			break;
		}
		if (mode != FW_NO_TAG_ACK)
			buf[n + 2] = TAG_RESPONSE | bytes;
		n += 3 + bytes / 4;
	}
	if (n >= buf[0] / 4 || buf[n] != 0 || (n + 1) * 4 != buf[0])
		layout_bad++;
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
		return 0; /* never full */
	case MAIL0_STA:
		return nmail0 ? 0 : MBOX_EMPTY;
	case MAIL0_RD:
		if (!nmail0) {
			bad_access++;
			return 0;
		}
		uint32_t m = mail0[0];
		mail0[0] = mail0[1];
		nmail0--;
		return m;
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
	if (nwritten < (int)ARRAY_SIZE(written))
		written[nwritten] = val;
	nwritten++;
	flush_before_write += flushed > 0;
	if (mode == FW_SILENT)
		return;
	firmware();
	nmail0 = 0;
	if (mode == FW_OTHER_CHAN_FIRST)
		mail0[nmail0++] = (val & ~0xfu) | 9;
	mail0[nmail0++] = val; /* the answer comes back on the request's channel */
}

static void reset(enum fw_mode m)
{
	mode = m;
	nwritten = 0;
	nmail0 = 0;
	flushed = invalidated = flush_before_write = 0;
	layout_bad = 0;
}

int main(void)
{
	/* 1. the mailbox's address */
	CHECK(rpi_mbox_init() == 0, "rpi_mbox_init failed");
	CHECK(map_calls == 1 && map_phys[0] == EXPECT_BASE && map_size[0] == 0x40 &&
		      map_flags[0] == K_MEM_CACHE_NONE,
	      SOC ": mailbox mapped %d times at %#llx + %#lx flags %u, expected %#llx + 0x40", map_calls,
	      map_phys[0], map_size[0], map_flags[0], EXPECT_BASE);

	/* 2. one tag: the word to MAIL1, the cache around it, the answer */
	reset(FW_OK);
	uint32_t hz = 0;
	CHECK(rpi_clock_rate(RPI_FW_CLK_V3D, &hz) == 0 && hz == V3D_HZ, "GET_CLOCK_RATE: %u", hz);
	CHECK(nwritten == 1 && written[0] == ((uint32_t)buf_phys | EXPECT_ALIAS | CHAN_PROPERTY),
	      SOC ": MAIL1 got %#x (%d writes), expected %#x", written[0], nwritten,
	      (uint32_t)buf_phys | EXPECT_ALIAS | CHAN_PROPERTY);
	CHECK(flushed == 1 && flush_before_write == 1 && invalidated == 1,
	      "flush %d (before the write %d), invalidate %d", flushed, flush_before_write, invalidated);
	CHECK(layout_bad == 0, "the firmware could not read the buffer");
	CHECK(buf[0] == 8 * 4 && buf[2] == 0x00030002 && buf[3] == 8 && buf[7] == 0,
	      "GET_CLOCK_RATE buffer: size %u tag %#x bytes %u end %#x", buf[0], buf[2], buf[3], buf[7]);

	/* 3. the other clock calls for the V3D clock */
	reset(FW_OK);
	CHECK(rpi_clock_max_rate(RPI_FW_CLK_V3D, &hz) == 0 && hz == V3D_MAX_HZ, "GET_MAX_CLOCK_RATE: %u", hz);
	CHECK(rpi_clock_measured_rate(RPI_FW_CLK_V3D, &hz) == 0 && hz == V3D_HZ - 1000,
	      "GET_CLOCK_RATE_MEASURED: %u", hz);
	bool on = false, exists = false;
	CHECK(rpi_clock_state_get(RPI_FW_CLK_V3D, &on, &exists) == 0 && on && exists,
	      "GET_CLOCK_STATE: on %d exists %d", on, exists);
	CHECK(rpi_clock_state_get(99, &on, &exists) == 0 && !on && !exists,
	      "GET_CLOCK_STATE of no clock: on %d exists %d", on, exists);
	CHECK(rpi_clock_state(RPI_FW_CLK_V3D, true) == 0 && set_state_id == RPI_FW_CLK_V3D && set_state_on == 1,
	      "SET_CLOCK_STATE: id %u on %u", set_state_id, set_state_on);
	CHECK(rpi_clock_set_rate(RPI_FW_CLK_V3D, 800000000u) == 0 && set_hz == 800000000u,
	      "SET_CLOCK_RATE: %u", set_hz);
	CHECK(nwritten == 6 && layout_bad == 0, "%d requests, %d unreadable", nwritten, layout_bad);

	/* 4. the framebuffer: ARM address and mapping */
	reset(FW_OK);
	struct rpi_fb fb = { 0 };
	CHECK(rpi_fb_alloc(32, 16, &fb) == 0, "rpi_fb_alloc failed");
	CHECK(fb.phys == FB_PHYS && fb.width == 32 && fb.height == 16 && fb.pitch == 128 &&
		      fb.size == 32 * 16 * 4,
	      "fb phys %#x %ux%u pitch %u size %u", fb.phys, fb.width, fb.height, fb.pitch, fb.size);
	CHECK(map_calls == 2 && map_phys[1] == FB_PHYS && map_size[1] == fb.size &&
		      map_flags[1] == K_MEM_ARM_NORMAL_NC && fb.cpu == fbmem,
	      "fb mapped at %#llx + %#lx flags %u", map_phys[1], map_size[1], map_flags[1]);
	CHECK(layout_bad == 0, "the firmware could not read the framebuffer request");

	/* 5. a buffer above 4 GiB: refused, nothing written */
	if (sizeof(uintptr_t) > 4) {
		reset(FW_OK);
		buf_phys = (uintptr_t)0x100000000ull + 0x100000u;
		CHECK(rpi_clock_rate(RPI_FW_CLK_V3D, &hz) == -ERANGE, "buffer above 4 GiB not refused");
		uint32_t t[5] = { 0x00030002, 8, 0, RPI_FW_CLK_V3D, 0 };
		CHECK(rpi_mbox_request_timed(t, 5, NULL) == -ERANGE, "timed: buffer above 4 GiB not refused");
		CHECK(nwritten == 0 && flushed == 0, "%d writes, %d flushes for a refused buffer", nwritten,
		      flushed);
		buf_phys = 0x00100000u;
	}

	/* 6. the firmware's error answer, an unanswered tag */
	reset(FW_ERROR);
	CHECK(rpi_clock_rate(RPI_FW_CLK_V3D, &hz) == -EIO, "error answer not -EIO");
	reset(FW_NO_TAG_ACK);
	CHECK(rpi_clock_rate(RPI_FW_CLK_V3D, &hz) == -EIO, "unanswered tag not -EIO");
	reset(FW_NO_TAG_ACK);
	CHECK(rpi_fb_pan(16, NULL, NULL) == -EIO, "unanswered pan not -EIO");

	/* 7. a message on another channel is skipped */
	reset(FW_OTHER_CHAN_FIRST);
	CHECK(rpi_clock_rate(RPI_FW_CLK_V3D, &hz) == 0 && hz == V3D_HZ && nmail0 == 0,
	      "with a channel 9 message first: hz %u, %d left in MAIL0", hz, nmail0);

	/* 8. the timed request: its split and the caller's tags */
	reset(FW_OK);
	struct rpi_mbox_timing tm = { 0 };
	uint32_t t[5] = { 0x00030002, 8, 0, RPI_FW_CLK_V3D, 0 };
	CHECK(rpi_mbox_request_timed(t, 5, &tm) == 0 && t[4] == V3D_HZ && (t[2] & TAG_RESPONSE),
	      "timed GET_CLOCK_RATE: %u, code %#x", t[4], t[2]);
	CHECK(tm.write > 0 && tm.poll > 0 && tm.inval > 0, "split %llu/%llu/%llu",
	      (unsigned long long)tm.write, (unsigned long long)tm.poll, (unsigned long long)tm.inval);
	CHECK(rpi_mbox_request_timed(t, ARRAY_SIZE(buf), NULL) == -EINVAL, "oversized request accepted");

	/* 9. the firmware never answers */
	reset(FW_SILENT);
	CHECK(rpi_clock_rate(RPI_FW_CLK_V3D, &hz) == -ETIMEDOUT, "silent firmware not -ETIMEDOUT");

	CHECK(bad_access == 0, "%d accesses outside MAIL0_RD/STA, MAIL1_WRT/STA", bad_access);

	printf("rpi_mbox_mock %s: %d of %d checks, %d failed\n", SOC, checks - failed, checks, failed);
	return failed != 0;
}
