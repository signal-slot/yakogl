/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_display: U3, the Raspberry Pi 5's display path from a kernel= image
 * (driver/docs/rpi5-display.md, section 6, checks Z0 to Z4). The console is
 * the record: its lines are the ones tools/probes/rpi5_display/verdicts.sh
 * reads ("u3 ..."), printed once each, in its order, even when a step failed.
 *
 * The first action is the watchdog's: the board layer (rpi_wdog.c,
 * CONFIG_YAKOGL_RPI5_WDOG) arms the BCM2712 PM watchdog in the first
 * PRE_KERNEL_1 hook, before main() runs, and stops feeding it after
 * CONFIG_YAKOGL_RPI5_WDOG_HOLD_S; the reset brings Linux back through tryboot.
 * main() never feeds it. Registers are only read; the only writes are pixels,
 * into the framebuffer the mailbox gave or into the plane the firmware scans
 * (route A's write test).
 */
#include <errno.h>
#include <string.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>

#include "fbcon.h"
#include "hvs6_dlist.h"
#include "rpi_mbox.h"
#include "verdict.h"

#define PERIOD_S 10 /* WDOG_PERIOD_S of rpi_wdog.c: one arm */

/* ARM addresses, driver/docs/rpi5-display.md section 2. */
#define HVS_BASE 0x107c580000ull
#define HVS_SIZE 0x1a000u
#define PV0_BASE 0x107c410000ull
#define PV0_SIZE 0x100u
#define HD_BASE  0x107c720000ull
#define HD_SIZE  0x100u
#define PV_CONTROL   0x00u /* EN bit 0 */
#define PV_V_CONTROL 0x04u /* VIDEN bit 0 */
#define HD_VID_CTL   0x44u /* ENABLE bit 31 */

#define FW_CLK_HDMI 13u /* the HSM clock: no clock, no HDMI register read */

/* The Pi 4's split screen: the screen's size, console on top. */
#define SCREEN_W 1280u
#define SCREEN_H 720u

#define PATTERN(x, y) (0xff000000u | (((y) & 0xffu) << 16) | (((x) & 0xffu) << 8) | 0x55u)

static uintptr_t map(uint64_t phys, size_t size)
{
	uint8_t *va = NULL;

	device_map((mm_reg_t *)&va, phys, size, K_MEM_CACHE_NONE);
	return (uintptr_t)va;
}

static uint32_t rd(uintptr_t base, uint32_t off)
{
	return sys_read32(base + off);
}

static struct {
	bool hvs_ok;
	bool running;
	bool plane_ok;
	struct hvs6_plane plane;
	uint32_t chan;
	uintptr_t hvs;
	enum hvs6_gen gen;
} u3;

static void report_throttled(const char *when)
{
	uint32_t v = 0;
	int rc = rpi_mbox_tag(0x00030046u, &v, 1); /* GET_THROTTLED */

	if (rc) {
		VERDICT("u3 throttled", "%s: unreadable (%d)", when, rc);
	} else {
		VERDICT("u3 throttled", "%s: 0x%08x", when, (unsigned)v);
	}
}

/* Z0: clocks (is the HSM running?), HVS stepping. */
static bool z0(void)
{
	static const uint32_t ids[] = {4, 16, 13, 14, 5};
	bool hsm = false;

	for (size_t i = 0; i < ARRAY_SIZE(ids); i++) {
		uint32_t hz = 0;
		int rc = rpi_clock_measured_rate(ids[i], &hz);

		if (rc) {
			VERDICT("u3 clock", "id %u unreadable (%d)", (unsigned)ids[i], rc);
			continue;
		}
		VERDICT("u3 clock", "id %u measured %u Hz", (unsigned)ids[i], (unsigned)hz);
		if (ids[i] == FW_CLK_HDMI && hz != 0) {
			hsm = true;
		}
	}
	u3.hvs = map(HVS_BASE, HVS_SIZE);
	if (!u3.hvs) {
		VERDICT("u3 hvs", "not mapped");
		return hsm;
	}
	uint32_t ver = rd(u3.hvs, HVS6_VERSION);

	u3.gen = hvs6_gen_from_version(ver);
	u3.hvs_ok = u3.gen != HVS6_GEN_UNKNOWN;
	VERDICT("u3 hvs", "VERSION 0x%08x CXM_SIZE %u gen %s", (unsigned)ver,
		(unsigned)rd(u3.hvs, HVS6_CXM_SIZE),
		u3.gen == HVS6_GEN_C ? "C0" : u3.gen == HVS6_GEN_D ? "D0" : "unknown");
	return hsm;
}

/* Z1: which channel runs, and its first plane. */
static void z1(bool hsm)
{
	for (uint32_t c = 0; u3.hvs_ok && c < HVS6_CHANNELS; c++) {
		struct hvs6_chan ch;
		uint32_t ctrl0 = rd(u3.hvs, hvs6_chan_reg_offset(u3.gen, c, HVS6_REG_CTRL0));
		uint32_t lptrs = rd(u3.hvs, hvs6_chan_reg_offset(u3.gen, c, HVS6_REG_LPTRS));
		uint32_t stat = rd(u3.hvs, hvs6_chan_reg_offset(u3.gen, c, HVS6_REG_STATUS));
		uint32_t dl = rd(u3.hvs, hvs6_chan_reg_offset(u3.gen, c, HVS6_REG_DL));

		hvs6_chan_decode(ctrl0, lptrs, stat, dl, &ch);
		VERDICT("u3 chan", "%u CTRL0 0x%08x LPTRS 0x%08x STATUS 0x%08x DL 0x%08x mode %u frcnt %u",
			(unsigned)c, (unsigned)ctrl0, (unsigned)lptrs, (unsigned)stat, (unsigned)dl,
			(unsigned)ch.mode, (unsigned)ch.frcnt);
		if (!ch.running || u3.running) {
			continue;
		}
		u3.running = true;
		u3.chan = c;
		const uint32_t *dlm = (const uint32_t *)(u3.hvs + HVS6_DLIST_OFFSET);
		int rc = hvs6_dlist_first_plane(dlm, rd(u3.hvs, HVS6_CXM_SIZE), ch.active, &u3.plane);

		if (rc == HVS6_OK) {
			u3.plane_ok = true;
		} else {
			VERDICT("u3 dlist", "channel %u list at %u: decode error %d", (unsigned)c,
				(unsigned)ch.active, rc);
		}
	}
	uintptr_t pv = map(PV0_BASE, PV0_SIZE);

	if (pv) {
		VERDICT("u3 pv0", "CONTROL 0x%08x V_CONTROL 0x%08x", (unsigned)rd(pv, PV_CONTROL),
			(unsigned)rd(pv, PV_V_CONTROL));
	}
	if (hsm) {
		uintptr_t hd = map(HD_BASE, HD_SIZE);

		if (hd) {
			VERDICT("u3 hd", "VID_CTL 0x%08x", (unsigned)rd(hd, HD_VID_CTL));
		}
	} else {
		VERDICT("u3 hd", "skipped: HSM clock is 0");
	}
	VERDICT("u3 scanout", "%s (channel %u)", u3.running ? "running" : "stopped",
		(unsigned)u3.chan);
	if (u3.plane_ok) {
		const char *n = hvs6_format_name(u3.plane.format_code);

		VERDICT("u3 plane", "addr=0x%llx fmt=%s pitch=%u w=%u h=%u",
			(unsigned long long)u3.plane.addr, n ? n : "?", (unsigned)u3.plane.pitch,
			(unsigned)u3.plane.width, (unsigned)u3.plane.height);
	} else {
		VERDICT("u3 plane", "addr=0x0 fmt=none pitch=0 w=0 h=0");
	}
}

/* Writes the pattern through a mapping into rows [row0, row0 + h) and reads
 * every word back; returns the words written, *bad those that differ. */
static uint32_t pattern_readback(uint8_t *cpu, uint32_t w, uint32_t h, uint32_t pitch,
				 uint32_t row0, uint32_t *bad)
{
	uint32_t bad_n = 0;

	for (uint32_t y = 0; y < h; y++) {
		volatile uint32_t *p = (volatile uint32_t *)(cpu + (size_t)(row0 + y) * pitch);

		for (uint32_t x = 0; x < w; x++) {
			p[x] = PATTERN(x, y);
		}
	}
	for (uint32_t y = 0; y < h; y++) {
		volatile uint32_t *p = (volatile uint32_t *)(cpu + (size_t)(row0 + y) * pitch);

		for (uint32_t x = 0; x < w; x++) {
			if (p[x] != PATTERN(x, y)) {
				bad_n++;
			}
		}
	}
	*bad = bad_n;
	return w * h;
}

/* Route A's write test: the pattern into the lower half of the plane the
 * firmware scans (an IOMMU that translates would make the address an IOVA;
 * Z0's IOMMU read is a later step, so this stays a plain mapping test). */
static void adopt_test(void)
{
	if (!u3.plane_ok || !u3.plane.rgb32 || u3.plane.addr >= (1ull << 32)) {
		VERDICT("u3 adopt-readback", "FAIL no 32 bpp plane to write");
		return;
	}
	size_t size = (size_t)u3.plane.pitch * u3.plane.height;
	uint8_t *cpu = (uint8_t *)map(u3.plane.addr, size);

	if (!cpu) {
		VERDICT("u3 adopt-readback", "FAIL plane not mapped");
		return;
	}
	uint32_t bad, n = pattern_readback(cpu, u3.plane.width, u3.plane.height / 2u,
					   u3.plane.pitch, u3.plane.height / 2u, &bad);

	VERDICT("u3 adopt-readback", "%s (%u of %u words differ)", bad ? "FAIL" : "PASS",
		(unsigned)bad, (unsigned)n);
}

static struct rpi_fb fb;
static bool fb_ok;

/* Z2 and Z3: the Pi 4's one-request allocation, then the pattern. */
static void z2z3(void)
{
	uint32_t got[4] = {0};
	int rc = rpi_fb_alloc_virtual(SCREEN_W, SCREEN_H, 2u * SCREEN_H, &fb, got);

	if (rc) {
		VERDICT("u3 mailbox-fb", "FAIL (rc %d, granted %ux%u virtual %ux%u)", rc,
			(unsigned)got[0], (unsigned)got[1], (unsigned)got[2], (unsigned)got[3]);
		VERDICT("u3 mailbox-readback", "FAIL no framebuffer");
		VERDICT("u3 scanout-follows-mailbox", "no (no framebuffer)");
		return;
	}
	fb_ok = true;
	VERDICT("u3 mailbox-fb", "PASS (%ux%u pitch %u %s, addr 0x%x)", (unsigned)fb.width,
		(unsigned)fb.height, (unsigned)fb.pitch, RPI_FB_MAPPING, (unsigned)fb.phys);
	uint32_t bad, n = pattern_readback(fb.cpu, fb.width, fb.height / 2u, fb.pitch,
					   fb.height / 2u, &bad);

	VERDICT("u3 mailbox-readback", "%s (%u of %u words differ)", bad ? "FAIL" : "PASS",
		(unsigned)bad, (unsigned)n);

	/* Does the HVS list now name our buffer, and does the frame count run? */
	bool follows = false;
	uint32_t f0 = 0, f1 = 0;

	if (u3.hvs_ok) {
		uint32_t soff = hvs6_chan_reg_offset(u3.gen, u3.chan, HVS6_REG_STATUS);
		uint32_t doff = hvs6_chan_reg_offset(u3.gen, u3.chan, HVS6_REG_DL);
		struct hvs6_plane p;
		const uint32_t *dlm = (const uint32_t *)(u3.hvs + HVS6_DLIST_OFFSET);

		f0 = (rd(u3.hvs, soff) >> HVS6_STATUS_FRCNT_SHIFT) & HVS6_STATUS_FRCNT_MASK;
		k_msleep(100);
		f1 = (rd(u3.hvs, soff) >> HVS6_STATUS_FRCNT_SHIFT) & HVS6_STATUS_FRCNT_MASK;
		if (hvs6_dlist_first_plane(dlm, rd(u3.hvs, HVS6_CXM_SIZE),
					   rd(u3.hvs, doff) & HVS6_DL_LACT_MASK, &p) == HVS6_OK) {
			follows = p.addr == fb.phys && f0 != f1;
		}
	}
	VERDICT("u3 scanout-follows-mailbox", "%s (frcnt %u -> %u)", follows ? "yes" : "no",
		(unsigned)f0, (unsigned)f1);
}

/* Z4: text on the top half, the pattern below (drawn in Z3). */
static void z4(void)
{
	if (!fb_ok) {
		VERDICT("u3 split-screen", "skipped: no framebuffer");
		return;
	}
	fbcon_attach(fb.cpu, fb.width, fb.pitch, fb.height / 2u, 1);
	VERDICT("u3 split-screen", "drawn (text above row %u, pattern below)",
		(unsigned)(fb.height / 2u));
}

int main(void)
{
	printk("rpi5_display: build %s\n", RPI5_DISPLAY_BUILD);
	printk("rpi5_display: %s, watchdog armed at boot, fed for %d s, period %d s\n",
	       CONFIG_BOARD, CONFIG_YAKOGL_RPI5_WDOG_HOLD_S, PERIOD_S);

	if (rpi_mbox_init()) {
		printk("rpi5_display: FAIL (mailbox not mapped)\n");
	} else {
		report_throttled("before");
		bool hsm = z0();

		z1(hsm);
		adopt_test();
		z2z3();
		z4();
		report_throttled("after");
		printk("rpi5_display: %s\n",
		       fb_ok ? "PASS (mailbox framebuffer)" : "FAIL (no mailbox framebuffer)");
	}
	printk("rpi5_display: feeding stopped, reset expected\n");

	/* Stop; the watchdog resets the board. */
	for (;;) {
		k_sleep(K_SECONDS(10));
	}
	return 0;
}
