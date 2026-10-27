/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The BCM2711 property mailbox (see rpi_mbox.h). One request in flight at a
 * time; the sample is single-threaded.
 */
#include <zephyr/kernel.h>
#include <zephyr/cache.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>
#include <errno.h>
#include <string.h>

#include "rpi_mbox.h"

#define MBOX_BASE      0xfe00b880u   /* bcm283x.dtsi:100, bus 0x7e00b880 */
#define MAIL0_RD       0x00          /* bcm2835-mailbox.c:41, VC -> ARM */
#define MAIL0_STA      0x18          /* :43 */
#define MAIL1_WRT      0x20          /* :45, ARM -> VC */
#define MAIL1_STA      0x38          /* :46 */
#define MBOX_FULL      BIT(31)       /* :49 */
#define MBOX_EMPTY     BIT(30)       /* :50 */
#define CHAN_PROPERTY  8             /* raspberrypi.c:23 */
#define STATUS_REQUEST 0x00000000u   /* raspberrypi-firmware.h:15 */
#define STATUS_SUCCESS 0x80000000u   /* :16 */
#define TAG_RESPONSE   BIT(31)       /* the firmware sets it in a tag it answered */

/* The VC sees ARM physical memory at bus 0xc0000000 (bcm2711.dtsi:45). */
#define BUS_ALIAS      0xc0000000u

static mm_reg_t mbox;
static uint32_t buf[64] __aligned(64);

int rpi_mbox_init(void)
{
	device_map(&mbox, MBOX_BASE, 0x40, K_MEM_CACHE_NONE);
	return mbox ? 0 : -ENOMEM;
}

static int transact(void)
{
	uint32_t phys = (uint32_t)k_mem_phys_addr(buf);
	sys_cache_data_flush_range(buf, sizeof(buf));

	for (int t = 0; sys_read32(mbox + MAIL1_STA) & MBOX_FULL; t++) {
		if (t > 100000)
			return -ETIMEDOUT;
		k_busy_wait(1);
	}
	sys_write32((phys | BUS_ALIAS) | CHAN_PROPERTY, mbox + MAIL1_WRT);

	for (int t = 0;; t++) {
		if (!(sys_read32(mbox + MAIL0_STA) & MBOX_EMPTY)) {
			uint32_t msg = sys_read32(mbox + MAIL0_RD);
			if ((msg & 0xf) == CHAN_PROPERTY)
				break;
		}
		if (t > 1000000)
			return -ETIMEDOUT;
		k_busy_wait(1);
	}
	sys_cache_data_invd_range(buf, sizeof(buf));
	return buf[1] == STATUS_SUCCESS ? 0 : -EIO;
}

/* transact() with the three parts timed (rpi_mbox_request_timed) */
static int transact_timed(struct rpi_mbox_timing *tm)
{
	uint64_t t0 = k_cycle_get_64();
	uint32_t phys = (uint32_t)k_mem_phys_addr(buf);
	sys_cache_data_flush_range(buf, sizeof(buf));

	for (int t = 0; sys_read32(mbox + MAIL1_STA) & MBOX_FULL; t++) {
		if (t > 100000)
			return -ETIMEDOUT;
		k_busy_wait(1);
	}
	sys_write32((phys | BUS_ALIAS) | CHAN_PROPERTY, mbox + MAIL1_WRT);
	uint64_t t1 = k_cycle_get_64();

	for (int t = 0;; t++) {
		if (!(sys_read32(mbox + MAIL0_STA) & MBOX_EMPTY)) {
			uint32_t msg = sys_read32(mbox + MAIL0_RD);
			if ((msg & 0xf) == CHAN_PROPERTY)
				break;
		}
		if (t > 1000000)
			return -ETIMEDOUT;
		k_busy_wait(1);
	}
	uint64_t t2 = k_cycle_get_64();
	sys_cache_data_invd_range(buf, sizeof(buf));
	uint64_t t3 = k_cycle_get_64();
	if (tm) {
		tm->write = t1 - t0;
		tm->poll = t2 - t1;
		tm->inval = t3 - t2;
	}
	return buf[1] == STATUS_SUCCESS ? 0 : -EIO;
}

int rpi_mbox_request_timed(uint32_t *tags, uint32_t words, struct rpi_mbox_timing *t)
{
	if (!tags || words > ARRAY_SIZE(buf) - 3)
		return -EINVAL;
	buf[0] = (3 + words) * 4;
	buf[1] = STATUS_REQUEST;
	memcpy(&buf[2], tags, words * 4);
	buf[2 + words] = 0;
	int ret = transact_timed(t);
	memcpy(tags, &buf[2], words * 4);
	return ret;
}

int rpi_mbox_tag(uint32_t tag, uint32_t *val, uint32_t words)
{
	if (words > ARRAY_SIZE(buf) - 6)
		return -EINVAL;
	/* size, request code, tag, value buffer bytes, tag request code,
	 * values, end tag (raspberrypi.c:100-118) */
	buf[0] = (6 + words) * 4;
	buf[1] = STATUS_REQUEST;
	buf[2] = tag;
	buf[3] = words * 4;
	buf[4] = 0;
	memcpy(&buf[5], val, words * 4);
	buf[5 + words] = 0;
	int ret = transact();
	if (ret)
		return ret;
	if (!(buf[4] & TAG_RESPONSE))
		return -EIO;
	memcpy(val, &buf[5], words * 4);
	return 0;
}

int rpi_fb_alloc(uint32_t width, uint32_t height, struct rpi_fb *fb)
{
	/* Several tags in one request, so the firmware sets the mode at once:
	 * physical size, virtual size, depth, pixel order, allocate, pitch
	 * (raspberrypi-firmware.h:97-123). */
	static const uint32_t layout[][2] = {
		{ 0x00048003, 2 }, /* SET_PHYSICAL_WIDTH_HEIGHT */
		{ 0x00048004, 2 }, /* SET_VIRTUAL_WIDTH_HEIGHT */
		{ 0x00048005, 1 }, /* SET_DEPTH */
		{ 0x00048006, 1 }, /* SET_PIXEL_ORDER: 1 = RGB */
		{ 0x00040001, 2 }, /* ALLOCATE: in alignment, out bus address, size */
		{ 0x00040008, 1 }, /* GET_PITCH */
	};
	const uint32_t in[][2] = { { width, height }, { width, height }, { 32, 0 }, { 1, 0 },
				   { 4096, 0 }, { 0, 0 } };
	uint32_t at[ARRAY_SIZE(layout)];
	uint32_t n = 2;

	for (size_t i = 0; i < ARRAY_SIZE(layout); i++) {
		buf[n++] = layout[i][0];
		buf[n++] = layout[i][1] * 4;
		buf[n++] = 0;
		at[i] = n;
		for (uint32_t w = 0; w < layout[i][1]; w++)
			buf[n++] = in[i][w];
	}
	buf[n++] = 0;
	buf[0] = n * 4;
	buf[1] = STATUS_REQUEST;
	int ret = transact();
	if (ret)
		return ret;

	uint32_t bus = buf[at[4]], size = buf[at[4] + 1];
	fb->order_code = buf[at[3] - 1];
	fb->order = buf[at[3]];
	fb->width = buf[at[0]];
	fb->height = buf[at[0] + 1];
	fb->pitch = buf[at[5]];
	fb->size = size;
	if (bus == 0 || size == 0 || fb->pitch < fb->width * 4)
		return -EIO;
	fb->phys = bus & ~BUS_ALIAS; /* the ARM physical address */
	/* Normal non-cacheable, not K_MEM_CACHE_NONE: on arm64 that is
	 * Device-nGnRnE (zephyr arch/arm64/core/mmu.c:1083-1084), where every
	 * store is a separate, strongly ordered bus write. The framebuffer is
	 * memory: v3d_gl_cluster's copy bench (DEMO3N) wrote the same 5.5 MB in
	 * 26.7 ms through Device and 2.0 ms through Normal NC. Both are
	 * uncached, so nothing about coherence changes. What Normal allows that
	 * Device does not: stores merge and loads may be reordered or
	 * speculated. The V3D register accessors order them: sys_write32 starts
	 * and sys_read32 ends with a full dmb (zephyr
	 * include/zephyr/arch/arm64/sys_io.h:69-83), so CPU writes land before
	 * a kick and CPU reads come after the completion poll. The windows the
	 * GPU renders into (v3d_gl_cluster's, v3d_gl_slint's) have always been
	 * mapped this way. */
	mm_reg_t va;
	device_map(&va, fb->phys, size, K_MEM_ARM_NORMAL_NC);
	fb->cpu = (uint8_t *)va;
	return fb->cpu ? 0 : -ENOMEM;
}

/* ---- the flip ---------------------------------------------------------- */

#define TAG_ANSWERED 0x80000000u

static struct rpi_fb_counters flip;

/* Append a tag (id, value bytes, request code 0, values) to `w` at `*n`;
 * the index of its first value word. */
static uint32_t tag_add(uint32_t *w, uint32_t *n, uint32_t tag, uint32_t nval, const uint32_t *vals)
{
	w[(*n)++] = tag;
	w[(*n)++] = nval * 4;
	w[(*n)++] = 0;
	uint32_t at = *n;
	for (uint32_t i = 0; i < nval; i++)
		w[(*n)++] = vals ? vals[i] : 0;
	return at;
}

static bool tag_answered(const uint32_t *w, uint32_t at)
{
	return (w[at - 1] & TAG_ANSWERED) != 0;
}

int rpi_fb_alloc_virtual(uint32_t w, uint32_t h, uint32_t virt_h, struct rpi_fb *fb, uint32_t got[4])
{
	uint32_t t[48], n = 0;
	const uint32_t phys[2] = { w, h }, virt[2] = { w, virt_h }, depth = 32, order = 1, off[2] = { 0, 0 },
		       align[2] = { 4096, 0 };
	uint32_t a_phys = tag_add(t, &n, 0x00048003, 2, phys);  /* SET_PHYSICAL_WIDTH_HEIGHT */
	uint32_t a_virt = tag_add(t, &n, 0x00048004, 2, virt);  /* SET_VIRTUAL_WIDTH_HEIGHT */
	uint32_t a_depth = tag_add(t, &n, 0x00048005, 1, &depth);
	uint32_t a_order = tag_add(t, &n, 0x00048006, 1, &order); /* SET_PIXEL_ORDER: RGB */
	uint32_t a_off = tag_add(t, &n, 0x00048009, 2, off);    /* SET_VIRTUAL_OFFSET */
	uint32_t a_alloc = tag_add(t, &n, 0x00040001, 2, align); /* ALLOCATE */
	uint32_t a_pitch = tag_add(t, &n, 0x00040008, 1, NULL); /* GET_PITCH */
	int ret = rpi_mbox_request_timed(t, n, NULL);
	if (got) {
		got[0] = t[a_phys];
		got[1] = t[a_phys + 1];
		got[2] = t[a_virt];
		got[3] = t[a_virt + 1];
	}
	if (ret)
		return ret;
	if (!tag_answered(t, a_phys) || !tag_answered(t, a_virt) || !tag_answered(t, a_depth) ||
	    !tag_answered(t, a_off) || !tag_answered(t, a_alloc) || !tag_answered(t, a_pitch))
		return -EIO;
	if (t[a_phys] != w || t[a_phys + 1] != h || t[a_virt] != w || t[a_virt + 1] != virt_h ||
	    t[a_depth] != 32)
		return -ERANGE;
	uint32_t bus = t[a_alloc], size = t[a_alloc + 1], pitch = t[a_pitch];
	if (bus == 0 || pitch < w * 4 || (uint64_t)size < (uint64_t)pitch * virt_h)
		return -EIO;
	fb->width = w;
	fb->height = h;
	fb->pitch = pitch;
	fb->size = size;
	fb->order_code = t[a_order - 1];
	fb->order = t[a_order];
	fb->phys = bus & ~BUS_ALIAS;
	/* Normal non-cacheable, as rpi_fb_alloc (46835fe4, the reasons there):
	 * this helper was written before that change and merged after it */
	mm_reg_t va;
	device_map(&va, fb->phys, size, K_MEM_ARM_NORMAL_NC);
	fb->cpu = (uint8_t *)va;
	return fb->cpu ? 0 : -ENOMEM;
}

static void count(uint64_t *sum, uint64_t *max, const struct rpi_mbox_timing *t)
{
	uint64_t c = t->write + t->poll + t->inval;
	*sum += c;
	if (c > *max)
		*max = c;
}

int rpi_fb_pan(uint32_t y, uint32_t *echo_y, struct rpi_mbox_timing *t)
{
	uint32_t w[8], n = 0;
	const uint32_t off[2] = { 0, y };
	uint32_t at = tag_add(w, &n, 0x00048009, 2, off); /* SET_VIRTUAL_OFFSET alone */
	struct rpi_mbox_timing tm = { 0 };
	int ret = rpi_mbox_request_timed(w, n, &tm);
	if (ret == 0 && !tag_answered(w, at))
		ret = -EIO;
	flip.pans++;
	count(&flip.pan_cycles, &flip.pan_max_cycles, &tm);
	if (ret) {
		flip.pans_failed++;
	} else if (w[at] != 0 || w[at + 1] != y) {
		flip.pans_not_echoed++;
	}
	if (echo_y)
		*echo_y = ret ? UINT32_MAX : w[at + 1];
	if (t)
		*t = tm;
	return ret;
}

int rpi_fb_get_offset(uint32_t *x, uint32_t *y)
{
	uint32_t w[8], n = 0;
	uint32_t at = tag_add(w, &n, 0x00040009, 2, NULL); /* GET_VIRTUAL_OFFSET */
	int ret = rpi_mbox_request_timed(w, n, NULL);
	if (ret == 0 && !tag_answered(w, at))
		ret = -EIO;
	if (x)
		*x = ret ? UINT32_MAX : w[at];
	if (y)
		*y = ret ? UINT32_MAX : w[at + 1];
	return ret;
}

int rpi_fb_wait_vsync(struct rpi_mbox_timing *t)
{
	uint32_t w[8], n = 0, dummy = 0;
	uint32_t at = tag_add(w, &n, 0x0004800e, 1, &dummy); /* SET_VSYNC, as FBIO_WAITFORVSYNC */
	struct rpi_mbox_timing tm = { 0 };
	int ret = rpi_mbox_request_timed(w, n, &tm);
	if (ret == 0 && !tag_answered(w, at))
		ret = -EIO;
	flip.vsyncs++;
	count(&flip.vsync_cycles, &flip.vsync_max_cycles, &tm);
	if (ret)
		flip.vsyncs_failed++;
	if (t)
		*t = tm;
	return ret;
}

void rpi_fb_counters_get(struct rpi_fb_counters *out)
{
	if (out)
		*out = flip;
}

int rpi_clock_state(uint32_t id, bool on)
{
	uint32_t v[2] = { id, on ? 1u : 0u }; /* SET_CLOCK_STATE, bit 0 = on */
	return rpi_mbox_tag(0x00038001, v, 2);
}

int rpi_clock_rate(uint32_t id, uint32_t *hz)
{
	uint32_t v[2] = { id, 0 };
	int ret = rpi_mbox_tag(0x00030002, v, 2); /* GET_CLOCK_RATE */
	*hz = v[1];
	return ret;
}

int rpi_clock_max_rate(uint32_t id, uint32_t *hz)
{
	uint32_t v[2] = { id, 0 };
	int ret = rpi_mbox_tag(0x00030004, v, 2); /* GET_MAX_CLOCK_RATE */
	*hz = v[1];
	return ret;
}

int rpi_clock_set_rate(uint32_t id, uint32_t hz)
{
	/* SET_CLOCK_RATE: id, rate, skip-turbo (clk-raspberrypi.c:163-167) */
	uint32_t v[3] = { id, hz, 0 };
	return rpi_mbox_tag(0x00038002, v, 3);
}
