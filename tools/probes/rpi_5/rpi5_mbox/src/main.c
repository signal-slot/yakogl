/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_mbox: the Raspberry Pi 5's property mailbox alone, with the raw words
 * (docs/notes/rpi5-display.md, section 3, the first probe; it answers section
 * 4 Q1: why every request of rpi5_display's session of 2026-11-19 came back
 * -EIO with response code 0, GET_THROTTLED included, while rpi5_clear's
 * GET_CLOCK_RATE of clock 5 was answered from a kernel= image the same day).
 *
 * The two images' mailbox paths, compared:
 *
 *   same   the transport (driver/zephyr/lib/board/rpi4/rpi_mbox.c built for
 *          CONFIG_SOC_BCM2712: base 0x10_7c013880, channel 8, no bus alias),
 *          the call (rpi_mbox_tag, the untimed transact(); neither image sent
 *          these tags through rpi_mbox_request_timed), the buffer (rpi_mbox.c's
 *          static buf[64], 64-byte aligned, in the image's .bss in the same
 *          32 MiB sram0 from 0x200000), rpi_mbox_init() before the first call.
 *   differ the cache maintenance: rpi5_clear's prj.conf sets
 *          CONFIG_CACHE_MANAGEMENT=y, rpi5_display's does not, so in
 *          rpi5_display's image transact()'s sys_cache_data_flush_range() and
 *          sys_cache_data_invd_range() returned -ENOTSUP and did nothing
 *          (zephyr include/zephyr/cache.h:218-227) while the data cache was on:
 *          the firmware read the request from memory the CPU had not written
 *          back, and the CPU read the answer from its own cached copy, whose
 *          response code is still the request's 0. That is the -EIO with code 0.
 *   differ the first request: rpi5_display's was GET_THROTTLED; rpi5_clear's
 *          was SET_CLOCK_STATE of clock 5 (v3d_power_on's clock op), then its
 *          GET_CLOCK_RATE.
 *   differ what ran before rpi_mbox_init(): rpi5_display read the DTB in x0
 *          first (rpi_5/common/boot); rpi5_clear nothing.
 *
 * So every request is sent three ways, each labelled on its line:
 *
 *   display  the probe's own copy of transact() (rpi_mbox.c:62-89, the same
 *            registers, bounds and buffer layout) with no cache call: what
 *            rpi5_display's image ran. After the reply the same buffer is read
 *            through an uncached alias too (`memory` line): the words the
 *            firmware left in memory, which the cached read cannot show.
 *   clear    the same copy with the clean before and the invalidate after:
 *            what rpi5_clear's image ran.
 *   board    rpi_mbox_request_timed() of the board layer in this image (the
 *            transport every Pi 5 probe uses), built with the cache calls on.
 *            It keeps the response code word to itself: its line says
 *            `status n/a`, its rc says whether the word was 0x80000000.
 *
 * GET_THROTTLED is the very first request of the image, the display way, as it
 * was in rpi5_display's. The first request of rpi5_clear (SET_CLOCK_STATE) is
 * not sent: this image only reads.
 *
 * Every request prints its lines whatever happened before: a `regs` line (the
 * mailbox status and config words before and after, the word written and the
 * one that ended the wait), then
 *
 *   rpi5_mbox: tag <id> status <hex> answered <hex> value <hex> rc <n> way <w> (...)
 *
 * with status the response code word, answered the tag's response length and
 * flag word (bit 31: answered), value the tag's last value word (the rate for
 * a clock) and rc the transaction's (0, -EIO when the response code is not
 * 0x80000000, -ETIMEDOUT, -ERANGE, -ENODEV). <id> is the tag, and for a clock
 * `0x00030002:<clock id>`. The end line is `rpi5_mbox: done (<n> of 7
 * answered)` (GET_THROTTLED answered in some way) or `rpi5_mbox: FAIL
 * (<reason>)`; then the image stops feeding the PM watchdog (armed by the board
 * layer in its first PRE_KERNEL_1 hook, before main), which resets the SoC
 * within 10 s, and tryboot brings Linux back. verdicts.sh reads the log.
 */
#include <zephyr/kernel.h>
#include <zephyr/cache.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>

#include <errno.h>
#include <stdbool.h>
#include <string.h>

#include "rpi_mbox.h"
#include "rpi_wdog.h"

/* The board layer's values (rpi_mbox.c:27-40, from Linux's
 * drivers/mailbox/bcm2835-mailbox.c:41-50 and drivers/firmware/raspberrypi.c:23);
 * MAIL0_CNF is bcm2835-mailbox.c:44, read only. */
#define MBOX_BASE      0x107c013880ull
#define MBOX_SIZE      0x40u
#define MAIL0_RD       0x00
#define MAIL0_STA      0x18
#define MAIL0_CNF      0x1c
#define MAIL1_WRT      0x20
#define MAIL1_STA      0x38
#define MBOX_FULL      BIT(31)
#define MBOX_EMPTY     BIT(30)
#define CHAN_PROPERTY  8u
#define STATUS_SUCCESS 0x80000000u
#define TAG_RESPONSE   BIT(31)
#define FULL_TRIES     100000  /* rpi_mbox.c:70-74, 1 us apart */
#define REPLY_TRIES    1000000 /* rpi_mbox.c:77-86, 1 us apart */

/* Tag ids, include/soc/bcm2835/raspberrypi-firmware.h (rpi-6.12.y):
 * GET_FIRMWARE_REVISION 0x00000001, GET_CLOCK_RATE 0x00030002 (rpi_mbox.c:373),
 * GET_THROTTLED 0x00030046 (rpi5_scanout/throttled.h). Clock ids as
 * rpi5_scanout/fw_clk.h:35-38 and rpi_mbox.h:23. */
#define TAG_FW_REV     0x00000001u
#define TAG_CLOCK_RATE 0x00030002u
#define TAG_THROTTLED  0x00030046u

#define NREQ 7u

struct req {
	const char *name;
	uint32_t tag;
	uint32_t nval; /* value words: 1, or 2 for a clock (id, rate) */
	uint32_t v0;   /* the first value word sent: the clock id, else 0 */
};

static const struct req reqs[NREQ] = {
	{ "get_throttled", TAG_THROTTLED, 1, 0 },
	{ "get_firmware_revision", TAG_FW_REV, 1, 0 },
	{ "clock core", TAG_CLOCK_RATE, 2, 4 },
	{ "clock disp", TAG_CLOCK_RATE, 2, 16 },
	{ "clock hdmi-hsm", TAG_CLOCK_RATE, 2, 13 },
	{ "clock hdmi-bvb", TAG_CLOCK_RATE, 2, 14 },
	{ "clock v3d", TAG_CLOCK_RATE, 2, RPI_FW_CLK_V3D },
};

/* One cache line each, in .bss like rpi_mbox.c's buf. */
#define BUF_WORDS 16u
static uint32_t buf_display[BUF_WORDS] __aligned(64);
static uint32_t buf_clear[BUF_WORDS] __aligned(64);

static mm_reg_t mbox;
static volatile uint32_t *mem_display; /* buf_display through an uncached alias */
static volatile bool finished;

/* The feeding stops once the end line is out: the reset follows within one
 * watchdog period instead of at the end of the hold. */
bool rpi_wdog_alive(void)
{
	return !finished;
}

struct xact {
	uint32_t before[3], after[3]; /* MAIL0_STA, MAIL0_CNF, MAIL1_STA */
	uint32_t sent, reply;         /* the word written, the one that ended the wait */
	uint32_t dropped, last_other; /* messages of another channel read on the way */
	uint32_t pre[3];              /* display: memory's words 0..2 before the doorbell */
	int flush_rc, invd_rc;
	const char *stage;
	int rc;
};

static void regs(uint32_t r[3])
{
	r[0] = sys_read32(mbox + MAIL0_STA);
	r[1] = sys_read32(mbox + MAIL0_CNF);
	r[2] = sys_read32(mbox + MAIL1_STA);
}

/* size, request code, tag, value bytes, tag request code, values, end tag
 * (rpi_mbox.c:147-155) */
static void build(uint32_t *b, const struct req *q)
{
	memset(b, 0, BUF_WORDS * 4u);
	b[0] = (6u + q->nval) * 4u;
	b[1] = 0;
	b[2] = q->tag;
	b[3] = q->nval * 4u;
	b[4] = 0;
	b[5] = q->v0;
	b[5u + q->nval] = 0;
}

/* rpi_mbox.c's transact() step by step, with every register word kept, and
 * its two cache calls made only when `cache` is set. */
static void raw_xact(uint32_t *b, bool cache, struct xact *x)
{
	memset(x, 0, sizeof(*x));
	x->reply = 0xffffffffu;
	x->last_other = 0xffffffffu;
	x->flush_rc = x->invd_rc = 1; /* 1: not called */
	x->stage = "not sent";
	if (!mbox) {
		x->stage = "no mapping";
		x->rc = -ENODEV;
		return;
	}
	regs(x->before);
	uint64_t phys64 = k_mem_phys_addr(b);
	uint32_t phys = (uint32_t)phys64;

	if (phys64 != phys) {
		x->stage = "buffer above 4 GiB";
		x->rc = -ERANGE;
		regs(x->after);
		return;
	}
	if (cache)
		x->flush_rc = sys_cache_data_flush_range(b, BUF_WORDS * 4u);
	if (!cache && mem_display) {
		for (int i = 0; i < 3; i++)
			x->pre[i] = mem_display[i];
	}
	for (int t = 0; sys_read32(mbox + MAIL1_STA) & MBOX_FULL; t++) {
		if (t > FULL_TRIES) {
			x->stage = "MAIL1 stayed full";
			x->rc = -ETIMEDOUT;
			regs(x->after);
			return;
		}
		k_busy_wait(1);
	}
	x->sent = phys | CHAN_PROPERTY;
	sys_write32(x->sent, mbox + MAIL1_WRT);
	for (int t = 0;; t++) {
		if (!(sys_read32(mbox + MAIL0_STA) & MBOX_EMPTY)) {
			uint32_t msg = sys_read32(mbox + MAIL0_RD);

			if ((msg & 0xfu) == CHAN_PROPERTY) {
				x->reply = msg;
				break;
			}
			x->dropped++;
			x->last_other = msg;
		}
		if (t > REPLY_TRIES) {
			x->stage = "no reply";
			x->rc = -ETIMEDOUT;
			regs(x->after);
			return;
		}
		k_busy_wait(1);
	}
	if (cache)
		x->invd_rc = sys_cache_data_invd_range(b, BUF_WORDS * 4u);
	regs(x->after);
	x->stage = x->reply == x->sent ? "replied" : "replied with another buffer";
	x->rc = b[1] == STATUS_SUCCESS ? 0 : -EIO;
}

static void print_regs(const char *id, const char *way, const struct xact *x)
{
	printk("rpi5_mbox: regs tag %s way %s: before sta0 0x%08x cnf0 0x%08x sta1 0x%08x, after sta0 "
	       "0x%08x cnf0 0x%08x sta1 0x%08x, sent 0x%08x reply 0x%08x, %u other message(s) dropped "
	       "(last 0x%08x), %s\n",
	       id, way, x->before[0], x->before[1], x->before[2], x->after[0], x->after[1], x->after[2],
	       x->sent, x->reply, x->dropped, x->last_other, x->stage);
}

static bool answered(const uint32_t *w, const struct req *q)
{
	return w[1] == STATUS_SUCCESS && w[2] == q->tag && (w[4] & TAG_RESPONSE) != 0;
}

/* The display way; also the `memory` line. Answered as the CPU saw it. */
static bool way_display(const char *id, const struct req *q, bool *mem_ok)
{
	struct xact x;
	uint32_t m[BUF_WORDS];

	build(buf_display, q);
	raw_xact(buf_display, false, &x);
	print_regs(id, "display", &x);
	printk("rpi5_mbox: tag %s status 0x%08x answered 0x%08x value 0x%08x rc %d way display (no clean "
	       "before, no invalidate after: rpi5_display's path; buffer PA 0x%llx)\n",
	       id, buf_display[1], buf_display[4], buf_display[4u + q->nval], x.rc,
	       (unsigned long long)k_mem_phys_addr(buf_display));
	*mem_ok = false;
	if (mem_display) {
		for (uint32_t i = 0; i < BUF_WORDS; i++)
			m[i] = mem_display[i];
		*mem_ok = answered(m, q);
		printk("rpi5_mbox: memory tag %s status 0x%08x answered 0x%08x value 0x%08x way display (the "
		       "same buffer read uncached after the wait: size 0x%08x tag 0x%08x; before the doorbell "
		       "size 0x%08x code 0x%08x tag 0x%08x)\n",
		       id, m[1], m[4], m[4u + q->nval], m[0], m[2], x.pre[0], x.pre[1], x.pre[2]);
	} else {
		printk("rpi5_mbox: memory tag %s status 0xffffffff answered 0xffffffff value 0xffffffff way "
		       "display (not read: the uncached alias was not mapped)\n", id);
	}
	return x.rc == 0 && answered(buf_display, q);
}

static bool way_clear(const char *id, const struct req *q)
{
	struct xact x;

	build(buf_clear, q);
	raw_xact(buf_clear, true, &x);
	print_regs(id, "clear", &x);
	printk("rpi5_mbox: tag %s status 0x%08x answered 0x%08x value 0x%08x rc %d way clear (clean before "
	       "%d, invalidate after %d: rpi5_clear's path; buffer PA 0x%llx)\n",
	       id, buf_clear[1], buf_clear[4], buf_clear[4u + q->nval], x.rc, x.flush_rc, x.invd_rc,
	       (unsigned long long)k_mem_phys_addr(buf_clear));
	return x.rc == 0 && answered(buf_clear, q);
}

static bool way_board(const char *id, const struct req *q, int init_rc)
{
	uint32_t t[5] = { q->tag, q->nval * 4u, 0, q->v0, 0 };
	struct rpi_mbox_timing tm;
	int rc = init_rc;

	memset(&tm, 0, sizeof(tm));
	if (init_rc == 0)
		rc = rpi_mbox_request_timed(t, 3u + q->nval, &tm);
	printk("rpi5_mbox: tag %s status n/a answered 0x%08x value 0x%08x rc %d way board "
	       "(rpi_mbox_request_timed%s: write %llu poll %llu inval %llu cycles)\n",
	       id, t[2], t[2u + q->nval], rc, init_rc == 0 ? "" : " not called: rpi_mbox_init failed",
	       (unsigned long long)tm.write, (unsigned long long)tm.poll, (unsigned long long)tm.inval);
	return rc == 0 && t[0] == q->tag && (t[2] & TAG_RESPONSE) != 0;
}

int main(void)
{
	unsigned int n_any = 0, n_disp = 0, n_mem = 0, n_clear = 0, n_board = 0;
	bool throttled_ok = false;
	uint32_t r[3] = { 0xffffffffu, 0xffffffffu, 0xffffffffu };
	char id[24];

	printk("rpi5_mbox: build %s\n", RPI5_MBOX_BUILD);
	/* One early line so a record says which case ran: whether
	 * rpi_mbox.c's sys_cache_data_*() calls do anything (off: -ENOTSUP). */
	printk("rpi5_mbox: cache management %s\n",
	       IS_ENABLED(CONFIG_CACHE_MANAGEMENT) ? "on" : "off");
	printk("rpi5_mbox: cache: CONFIG_CACHE_MANAGEMENT=%d CONFIG_DCACHE=%d, data line %u bytes\n",
	       IS_ENABLED(CONFIG_CACHE_MANAGEMENT), IS_ENABLED(CONFIG_DCACHE),
	       (unsigned int)sys_cache_data_line_size_get());
	printk("rpi5_mbox: paths: same base, channel, buffer layout, untimed call and sram0 buffer in "
	       "rpi5_display and rpi5_clear; differ: CONFIG_CACHE_MANAGEMENT (display 0, clear 1), the "
	       "first request, the DTB read before init. Ways per request: display, clear, board\n");

	/* The board layer's mapping first, as both images did. */
	int init_rc = rpi_mbox_init();

	printk("rpi5_mbox: mailbox init %d (rpi_mbox_init)\n", init_rc);
	device_map(&mbox, MBOX_BASE, MBOX_SIZE, K_MEM_CACHE_NONE);
	printk("rpi5_mbox: probe mapping of 0x%llx: %s\n", (unsigned long long)MBOX_BASE,
	       mbox ? "mapped" : "NOT mapped");

	uint64_t pa = k_mem_phys_addr(buf_display);
	uint64_t page = pa & ~(uint64_t)0xfff;
	mm_reg_t alias = 0;

	device_map(&alias, (uintptr_t)page, 0x1000, K_MEM_CACHE_NONE);
	if (alias)
		mem_display = (volatile uint32_t *)(alias + (uintptr_t)(pa - page));
	printk("rpi5_mbox: buffers: display PA 0x%llx (uncached alias %s), clear PA 0x%llx, %u bytes each\n",
	       (unsigned long long)pa, alias ? "mapped" : "NOT mapped",
	       (unsigned long long)k_mem_phys_addr(buf_clear), BUF_WORDS * 4u);
	if (mbox)
		regs(r);
	printk("rpi5_mbox: regs at start: sta0 0x%08x cnf0 0x%08x sta1 0x%08x\n", r[0], r[1], r[2]);

	for (unsigned int i = 0; i < NREQ; i++) {
		const struct req *q = &reqs[i];
		bool mem_ok = false;

		if (q->tag == TAG_CLOCK_RATE)
			snprintk(id, sizeof(id), "0x%08x:%u", q->tag, q->v0);
		else
			snprintk(id, sizeof(id), "0x%08x", q->tag);
		printk("rpi5_mbox: request %u of %u: %s, tag %s\n", i + 1, NREQ, q->name, id);
		bool d = way_display(id, q, &mem_ok);
		bool c = way_clear(id, q);
		bool b = way_board(id, q, init_rc);

		n_disp += d;
		n_mem += mem_ok;
		n_clear += c;
		n_board += b;
		if (d || c || b) {
			n_any++;
			if (q->tag == TAG_THROTTLED)
				throttled_ok = true;
		}
	}

	r[0] = r[1] = r[2] = 0xffffffffu;
	if (mbox)
		regs(r);
	printk("rpi5_mbox: regs at end: sta0 0x%08x cnf0 0x%08x sta1 0x%08x\n", r[0], r[1], r[2]);
	printk("rpi5_mbox: answered: display %u of %u, memory %u of %u, clear %u of %u, board %u of %u\n",
	       n_disp, NREQ, n_mem, NREQ, n_clear, NREQ, n_board, NREQ);
	if (!mbox && init_rc != 0)
		printk("rpi5_mbox: FAIL (no mailbox mapping: rpi_mbox_init %d, probe mapping failed)\n", init_rc);
	else if (!throttled_ok)
		printk("rpi5_mbox: FAIL (GET_THROTTLED not answered in any way; %u of %u answered)\n", n_any,
		       NREQ);
	else
		printk("rpi5_mbox: done (%u of %u answered)\n", n_any, NREQ);

	finished = true;
	printk("rpi5_mbox: feeding stopped, reset expected (the PM watchdog resets the SoC within 10 s, "
	       "and tryboot returns the board to Linux)\n");
	for (;;)
		k_sleep(K_FOREVER);
	return 0;
}
