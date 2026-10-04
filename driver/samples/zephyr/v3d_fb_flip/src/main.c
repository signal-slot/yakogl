/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * v3d_fb_flip: the display half of direct rendering on the Raspberry Pi 4.
 * No GPU. Does the firmware framebuffer pan between the two halves of a
 * 1920x1440 virtual buffer without tearing, and what does one pan cost?
 *
 * 1. SET/TEST tags through the property mailbox (tag numbers:
 *    include/soc/bcm2835/raspberrypi-firmware.h, Raspberry Pi linux
 *    rpi-6.12.y a553c4648f68; the firmware wiki "Mailbox property
 *    interface": a set tag's answer may differ from the request and must be
 *    checked). TEST_VIRTUAL_WIDTH_HEIGHT first, then one list: physical
 *    1920x720, virtual 1920x1440, depth 32, pixel order RGB, virtual offset
 *    0,0, allocate, pitch. Every answer is checked and printed; a refusal is
 *    printed with the fallbacks, and the image stops there.
 * 2. The CPU fills rows 0..719 black and 720..1439 white (alpha 255), each
 *    with a 16-pixel grey bar at the left (0x40 top, 0xc0 bottom) so a capture
 *    can tell the halves apart even if the scaler blends an edge.
 * 3. Four arms of ARM_MS each, the offset alternating 0,0 / 0,720:
 *      A  SET_VIRTUAL_OFFSET alone;
 *      B  SET_VSYNC, then SET_VIRTUAL_OFFSET (the control: Linux's
 *         FBIO_WAITFORVSYNC sends SET_VSYNC, bcm2708_fb.c:719-724);
 *      C  SET_VIRTUAL_OFFSET, then SET_VSYNC (is the offset latched at the
 *         next vsync?);
 *      D  the whole list Linux's pan resends (bcm2708_fb_set_par,
 *         bcm2708_fb.c:348-370) without ALLOCATE: physical, virtual, depth,
 *         offset, pitch.
 *    A, C and D are paced at 23 ms plus 0..16 ms of jitter, a period that is
 *    not a multiple of the refresh, so the pans land all over the scan; B
 *    paces itself on the vsync.
 * 4. Per arm, a line every 50 pans: the transaction time (min / mean / max,
 *    microseconds) split into write, poll (the firmware's own time) and
 *    invalidate, the vsync wait for B and C, and the answers that did not
 *    echo the requested offset. Each arm prints its start and end on the
 *    uptime clock, for the HDMI capture's bursts.
 *
 * The console is the mini UART only (no fbcon: it would draw into the
 * pattern). The watchdog resets the board RPI_WDOG_HOLD_S after boot.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>
#include <errno.h>
#include <stdbool.h>
#include <stdint.h>

#include "rpi_mbox.h"
#include "rpi_wdog.h"

#define W  1920u
#define H  720u
#define VH 1440u
#define ARM_MS 10000
#define PACE_US 23000u
#define JITTER_US 16000u
#define BLOCK 50u

/* tag numbers: raspberrypi-firmware.h */
#define TAG_GET_VIRTUAL_WH  0x00040004u
#define TAG_GET_DISPLAY_TIMING 0x00040017u
#define TAG_TEST_VIRTUAL_WH 0x00044004u
#define TAG_SET_PHYSICAL_WH 0x00048003u
#define TAG_SET_VIRTUAL_WH  0x00048004u
#define TAG_SET_DEPTH       0x00048005u
#define TAG_SET_PITCH       0x00048008u
#define TAG_SET_VOFFSET     0x00048009u
#define TAG_ANSWERED        0x80000000u

static uint64_t hz;

static uint32_t us(uint64_t cycles)
{
	return (uint32_t)(cycles * 1000000u / hz);
}

/* Append one tag (id, value bytes, request code 0, values) to `w` at `n`;
 * returns the index of its first value word, and advances `n`. */
static uint32_t add(uint32_t *w, uint32_t *n, uint32_t tag, uint32_t nval, const uint32_t *vals)
{
	w[(*n)++] = tag;
	w[(*n)++] = nval * 4;
	w[(*n)++] = 0;
	uint32_t at = *n;
	for (uint32_t i = 0; i < nval; i++)
		w[(*n)++] = vals ? vals[i] : 0;
	return at;
}

/* Was the tag whose first value is at `at` answered? */
static bool answered(const uint32_t *w, uint32_t at)
{
	return (w[at - 1] & TAG_ANSWERED) != 0;
}

static void fallbacks(void)
{
	printk("fallback 1: TEST_VIRTUAL_WIDTH_HEIGHT's answer above is the size the firmware offers\n");
	printk("fallback 2: max_framebuffer_height=1440 (and max_framebuffer_width=1920) in tryboot.txt, "
	       "then this image again\n");
	printk("fallback 3: ALLOCATE with our own base, a flip resending the list (Linux, bcm2708_fb.c:390-420)\n");
	printk("fallback 4: SET_PLANE 0x00048015 (firmware KMS, vc4_firmware_kms.c:64-110)\n");
	printk("fallback 5: direct render single-buffered into the visible framebuffer (tearing accepted)\n");
}

struct fb {
	uint32_t phys, size, pitch;
	uint32_t *cpu;
};

/* the flip helpers' allocation (rpi_fb_alloc_virtual, every answer checked
 * there); a refusal prints what was granted and the fallbacks */
static int alloc_fb(struct fb *fb)
{
	struct rpi_fb f;
	uint32_t got[4] = { 0 };
	int rc = rpi_fb_alloc_virtual(W, H, VH, &f, got);
	printk("fb: rc %d; granted physical %ux%u, virtual %ux%u", rc, got[0], got[1], got[2], got[3]);
	if (rc) {
		printk("\n");
		printk("REFUSED: %s\n", rc == -ERANGE ? "the firmware granted another size or depth"
				   : rc == -ENOMEM ? "the framebuffer could not be mapped"
						   : "a tag was not answered, or no buffer, or too small a buffer");
		fallbacks();
		return -EIO;
	}
	printk(", depth 32, offset 0,0, base (ARM) 0x%08x size %u, pitch %u\n", f.phys, f.size, f.pitch);
	fb->phys = f.phys;
	fb->size = f.size;
	fb->pitch = f.pitch;
	fb->cpu = (uint32_t *)f.cpu;
	return 0;
}

#define BORDER 64u /* the checker border's width */
#define CHECK  16u /* its squares */
#define DIAG   8u  /* the diagonal's thickness, in rows */

/* The pixel of half `top` at (x, yh), yh the row inside the half:
 * - a 64-pixel checker border of 16-pixel squares, so a capture's rectifier
 *   finds the quad and a scaled edge stays visible;
 * - inside it, black (top half) or white (bottom half);
 * - a 16-pixel grey bar just inside the border's left edge (0x40 top, 0xc0
 *   bottom) that names the half;
 * - a diagonal from the inner top-left to the inner bottom-right corner in
 *   the other colour: a tear shows as a diagonal broken at the tear's row.
 * The pixel order is RGB and every colour here has equal channels, so the
 * channels' byte order does not matter. */
static uint32_t pattern(bool top, uint32_t x, uint32_t yh)
{
	const uint32_t black = 0xff000000u, white = 0xffffffffu;
	if (x < BORDER || x >= W - BORDER || yh < BORDER || yh >= H - BORDER)
		return ((x / CHECK) + (yh / CHECK)) % 2 ? white : black;
	if (x < BORDER + 16)
		return top ? 0xff404040u : 0xffc0c0c0u;
	/* the inner rectangle's diagonal: row r of (H - 2 BORDER) maps to column
	 * BORDER + r * (W - 2 BORDER) / (H - 2 BORDER) */
	uint32_t r = yh - BORDER, cx = BORDER + r * (W - 2 * BORDER) / (H - 2 * BORDER);
	if (x + DIAG * (W - 2 * BORDER) / (H - 2 * BORDER) / 2 >= cx &&
	    x <= cx + DIAG * (W - 2 * BORDER) / (H - 2 * BORDER) / 2)
		return top ? white : black;
	return top ? black : white;
}

/* Both halves, the pattern above; 32-bit stores only (the mapping is Device
 * memory). Returns the rows that did not read back. */
static uint32_t fill(const struct fb *fb)
{
	uint32_t bad = 0;
	for (uint32_t y = 0; y < VH; y++) {
		volatile uint32_t *row = fb->cpu + (size_t)y * (fb->pitch / 4);
		bool top = y < H;
		uint32_t yh = top ? y : y - H;
		for (uint32_t x = 0; x < W; x++)
			row[x] = pattern(top, x, yh);
		if (row[0] != pattern(top, 0, yh) || row[W / 2] != pattern(top, W / 2, yh) ||
		    row[W - 1] != pattern(top, W - 1, yh))
			bad++;
	}
	return bad;
}

struct stats {
	uint32_t n, errors, mismatches, not_moved;
	uint64_t tot_sum, write_sum, poll_sum, inval_sum, vs_sum;
	uint32_t tot_min, tot_max, poll_min, poll_max, vs_min, vs_max, vs_n;
};

static void stats_reset(struct stats *s)
{
	*s = (struct stats){ .tot_min = UINT32_MAX, .poll_min = UINT32_MAX, .vs_min = UINT32_MAX };
}

static void stats_pan(struct stats *s, const struct rpi_mbox_timing *t)
{
	uint32_t w = us(t->write), p = us(t->poll), i = us(t->inval), tot = w + p + i;
	s->n++;
	s->tot_sum += tot;
	s->write_sum += w;
	s->poll_sum += p;
	s->inval_sum += i;
	s->tot_min = MIN(s->tot_min, tot);
	s->tot_max = MAX(s->tot_max, tot);
	s->poll_min = MIN(s->poll_min, p);
	s->poll_max = MAX(s->poll_max, p);
}

static void stats_vsync(struct stats *s, const struct rpi_mbox_timing *t)
{
	uint32_t v = us(t->write + t->poll + t->inval);
	s->vs_n++;
	s->vs_sum += v;
	s->vs_min = MIN(s->vs_min, v);
	s->vs_max = MAX(s->vs_max, v);
}

static void stats_print(char arm, uint32_t first, const struct stats *s)
{
	if (s->n == 0)
		return;
	printk("arm %c pans %u-%u: total us min %u mean %u max %u; write mean %u, poll min %u mean %u max %u, "
	       "inval mean %u",
	       arm, first, first + s->n - 1, s->tot_min, (uint32_t)(s->tot_sum / s->n), s->tot_max,
	       (uint32_t)(s->write_sum / s->n), s->poll_min, (uint32_t)(s->poll_sum / s->n), s->poll_max,
	       (uint32_t)(s->inval_sum / s->n));
	if (s->vs_n)
		printk("; vsync us min %u mean %u max %u", s->vs_min, (uint32_t)(s->vs_sum / s->vs_n), s->vs_max);
	printk("; offset not echoed %u, read back elsewhere %u, errors %u\n", s->mismatches, s->not_moved, s->errors);
}

/* SET_VSYNC: the flip helper */
static int vsync(struct rpi_mbox_timing *t)
{
	return rpi_fb_wait_vsync(t);
}

/* one pan to row `y`: the flip helper (SET_VIRTUAL_OFFSET alone), or (full,
 * arm D) Linux's list without ALLOCATE, kept here because it is the list
 * that does NOT pan; *echoed says whether the answer names 0,y */
static int pan(uint32_t y, bool full, uint32_t pitch, struct rpi_mbox_timing *t, bool *echoed)
{
	if (!full) {
		uint32_t echo;
		int rc = rpi_fb_pan(y, &echo, t);
		*echoed = rc == 0 && echo == y;
		return rc;
	}
	uint32_t w[40], n = 0;
	const uint32_t off[2] = { 0, y };
	{
		const uint32_t phys[2] = { W, H }, virt[2] = { W, VH }, depth = 32;
		add(w, &n, TAG_SET_PHYSICAL_WH, 2, phys);
		add(w, &n, TAG_SET_VIRTUAL_WH, 2, virt);
		add(w, &n, TAG_SET_DEPTH, 1, &depth);
	}
	uint32_t at = add(w, &n, TAG_SET_VOFFSET, 2, off);
	add(w, &n, TAG_SET_PITCH, 1, &pitch);
	int rc = rpi_mbox_request_timed(w, n, t);
	*echoed = rc == 0 && answered(w, at) && w[at] == 0 && w[at + 1] == y;
	return rc ? rc : (answered(w, at) ? 0 : -EIO);
}

/* GET_DISPLAY_TIMING (0x00040017; struct set_timings, vc4_firmware_kms.c:
 * 126-160, as vc4_fkms_get_fw_mode reads it, :1391-1420) for display numbers
 * 0..7; a display with a zero pixel clock is not driven. The refresh is the
 * pixel clock over htotal * vtotal. */
static void display_timings(void)
{
	for (uint32_t d = 0; d < 8; d++) {
		uint32_t w[16], n = 0, v[9] = { d };
		uint32_t at = add(w, &n, TAG_GET_DISPLAY_TIMING, 9, v);
		int rc = rpi_mbox_request_timed(w, n, NULL);
		if (rc || !answered(w, at) || w[at + 1] == 0)
			continue;
		uint32_t clock_khz = w[at + 1];
		uint32_t hdisplay = w[at + 2] & 0xffff, htotal = w[at + 3] >> 16;
		uint32_t vdisplay = w[at + 4] >> 16, vtotal = w[at + 6] & 0xffff;
		uint32_t vrefresh = w[at + 7] & 0xffff, vic = w[at] >> 16, flags = w[at + 8];
		uint32_t mhz_x1000 = htotal && vtotal ? (uint32_t)((uint64_t)clock_khz * 1000000u / htotal / vtotal) : 0;
		printk("display %u timing: %ux%u, pixel clock %u kHz, htotal %u, vtotal %u, vrefresh field %u, "
		       "computed %u.%03u Hz, vic %u, flags 0x%x\n",
		       d, hdisplay, vdisplay, clock_khz, htotal, vtotal, vrefresh, mhz_x1000 / 1000, mhz_x1000 % 1000,
		       vic, flags);
	}
}

static void get_virtual(const char *when)
{
	uint32_t w[8], n = 0;
	uint32_t at = add(w, &n, TAG_GET_VIRTUAL_WH, 2, NULL);
	int rc = rpi_mbox_request_timed(w, n, NULL);
	printk("get virtual %s: rc %d, answered %d, %ux%u\n", when, rc, answered(w, at), w[at], w[at + 1]);
}

static void test_virtual(const char *when)
{
	uint32_t w[8], n = 0;
	const uint32_t virt[2] = { W, VH };
	uint32_t at = add(w, &n, TAG_TEST_VIRTUAL_WH, 2, virt);
	int rc = rpi_mbox_request_timed(w, n, NULL);
	printk("test virtual 1920x1440 %s: rc %d, answered %d, answers %ux%u\n", when, rc, answered(w, at), w[at],
	       w[at + 1]);
}

/* GET_VIRTUAL_OFFSET after a pan, untimed (the pan's time is already
 * taken): whether the firmware's offset is now 0,y. The echo in the pan's
 * own answer is what the firmware says it did; this is what it holds. */
static bool offset_is(uint32_t y)
{
	uint32_t ox, oy;
	return rpi_fb_get_offset(&ox, &oy) == 0 && ox == 0 && oy == y;
}

static uint64_t now_us(void)
{
	return k_cycle_get_64() * 1000000u / hz;
}

static void run_arm(char arm, const char *what, uint32_t pitch)
{
	struct rpi_mbox_timing t, tv;
	struct stats s;
	bool echoed;
	uint32_t done = 0, first = 1, y = H;

	/* each half held 1 s before the pans, so a capture proves the card sees
	 * the signal and which half is which; then from the top half */
	(void)pan(0, false, pitch, &t, &echoed);
	printk("ARM %c HOLD top (black) from t=%lld ms, 1 s\n", arm, k_uptime_get());
	k_msleep(1000);
	(void)pan(H, false, pitch, &t, &echoed);
	printk("ARM %c HOLD bottom (white) from t=%lld ms, 1 s\n", arm, k_uptime_get());
	k_msleep(1000);
	(void)pan(0, false, pitch, &t, &echoed);
	k_msleep(200);
	printk("ARM %c START t=%lld ms: %s\n", arm, k_uptime_get(), what);
	stats_reset(&s);
	int64_t end = k_uptime_get() + ARM_MS;
	uint64_t next = now_us();
	int consecutive = 0;
	while (k_uptime_get() < end) {
		if (arm != 'B') {
			uint64_t n = now_us();
			if (next > n)
				k_usleep((int32_t)(next - n));
			/* 0..16 ms of jitter from the cycle counter's low bits */
			uint32_t jitter = (uint32_t)((k_cycle_get_32() * 2654435761u) >> 8) % JITTER_US;
			next = now_us() + PACE_US + jitter;
		} else {
			int rv = vsync(&tv);
			if (rv) {
				s.errors++;
				if (++consecutive >= 3)
					break;
				continue;
			}
			stats_vsync(&s, &tv);
		}
		int rc = pan(y, arm == 'D', pitch, &t, &echoed);
		if (rc) {
			s.errors++;
			if (++consecutive >= 3)
				break;
			continue;
		}
		consecutive = 0;
		stats_pan(&s, &t);
		s.mismatches += !echoed;
		s.not_moved += !offset_is(y);
		if (arm == 'C' && vsync(&tv) == 0)
			stats_vsync(&s, &tv);
		y = y ? 0 : H;
		done++;
		if (s.n == BLOCK) {
			stats_print(arm, first, &s);
			first += s.n;
			stats_reset(&s);
		}
	}
	stats_print(arm, first, &s);
	printk("ARM %c END t=%lld ms: %u pans%s\n", arm, k_uptime_get(), done,
	       consecutive >= 3 ? " (stopped: three failed requests in a row)" : "");
}

int main(void)
{
	printk("v3d_fb_flip: build %s\n", V3D_BUILD);
	rpi_wdog_announce();
	hz = sys_clock_hw_cycles_per_sec();
	printk("cycle counter: %llu Hz\n", hz);
	if (rpi_mbox_init()) {
		printk("REFUSED: the mailbox could not be mapped\n");
		return 0;
	}

	display_timings();
	/* the virtual size before this image asks for one: if TEST answers the
	 * same, TEST reports the current framebuffer, not what it could offer
	 * (the first run answered "offers 2x2") */
	get_virtual("before the allocation");
	test_virtual("before the allocation");

	struct fb fb;
	if (alloc_fb(&fb))
		return 0;
	get_virtual("after the allocation");
	test_virtual("after the allocation");
	uint32_t bad = fill(&fb);
	printk("halves filled: rows 0-719 black, 720-1439 white, each with a 64-px checker border, a grey bar "
	       "(0x40 / 0xc0) and a diagonal, pitch %u; "
	       "%u rows did not read back\n",
	       fb.pitch, bad);
	if (bad) {
		printk("REFUSED: the framebuffer did not read back what was written\n");
		return 0;
	}
	printk("capture: a diagonal broken at a row (or a black/white boundary) is a tear; the arms follow, "
	       "each after 1 s of each half, %d ms of pans each\n",
	       ARM_MS);

	run_arm('A', "SET_VIRTUAL_OFFSET alone, paced 23 ms + 0..16 ms", fb.pitch);
	run_arm('B', "SET_VSYNC then SET_VIRTUAL_OFFSET (the control)", fb.pitch);
	run_arm('C', "SET_VIRTUAL_OFFSET then SET_VSYNC, paced 23 ms + 0..16 ms", fb.pitch);
	run_arm('D', "the whole list (physical, virtual, depth, offset, pitch), paced 23 ms + 0..16 ms", fb.pitch);

	struct rpi_mbox_timing t;
	bool echoed;
	(void)pan(0, false, fb.pitch, &t, &echoed);
	struct rpi_fb_counters c;
	rpi_fb_counters_get(&c);
	printk("flip helpers: %u pans (%u failed, %u not echoed), mean %u us, max %u us; %u vsync waits (%u failed), "
	       "mean %u us, max %u us (arm D's list is not a helper call and is not counted)\n",
	       c.pans, c.pans_failed, c.pans_not_echoed, c.pans ? us(c.pan_cycles / c.pans) : 0, us(c.pan_max_cycles),
	       c.vsyncs, c.vsyncs_failed, c.vsyncs ? us(c.vsync_cycles / c.vsyncs) : 0, us(c.vsync_max_cycles));
	printk("v3d_fb_flip: done at t=%lld ms (top half shown); the watchdog resets the board\n", k_uptime_get());
	return 0;
}
