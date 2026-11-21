/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_scanout16: frame 1 of the Pi 5 display (docs/notes/rpi5-display.md
 * section 3, Q2). The RGB565 plane the firmware left running on HDMI0 is
 * adopted, 16 bpp fbcon text is drawn on its top rows and a pattern below,
 * the plane is read back, held for RPI5_SCANOUT16_HOLD_MS without a write,
 * and read back again. The lines named u3 and rpi5_scanout16 are the eleven
 * of verdicts.sh's header, in its order; each is printed even when an
 * earlier step failed (a skipped step prints FAIL, stopped or not drawn with
 * its reason).
 *
 *   dump  the read-only register dump of dump.h (HVS VERSION, channels 0..2,
 *         PV_CONTROL / PV_V_CONTROL of both pixel valves); it maps the HVS
 *         and the two valves, and the rest of the image reads through those
 *         mappings
 *   Z1    channels 0..2 sampled twice SAMPLE_GAP_MS apart, each judged by
 *         scanout_alive.h once per mapped valve, as rpi5_display does: the
 *         first running channel names line [1] (else channel 0). Its list
 *         (DL.LACT, or LPTRS.HEADE when LACT is 0) is read, walked to its END
 *         and the first plane decoded: line [2]
 *   adopt adopt.h's decision on that plane, iommu4 read so an address the
 *         IOMMU may translate is never written, the buffer mapped Normal
 *         non-cacheable (rpi5_display's mapping), then two hashes of the
 *         plane SAMPLE_GAP_MS apart (does the firmware draw before we write:
 *         hold_check.h's redraw A), the visible bytes cleared to FRAME16_BG
 *         and read back: line [3]
 *   draw  frame16.h: the text rows (line [4]) and the pattern (line [5]),
 *         the same frame rendered into a RAM image of the expected bytes,
 *         the written rows cleaned to memory (fb_flush.h), readback 1 byte-
 *         equal against the expected image (line [6])
 *   hold  hold_check.h: a reference sample right after the write, then up
 *         to HOLD_SAMPLES samples spread over the hold (STATUS, LPTRS, the
 *         list's words hashed, plane rows hashed, the valve's words), the
 *         hold end clamped by hold_clamp_end to the watchdog's feeding time;
 *         hold_judge decides. Then readback 2 (line [7]), FRCNT twice
 *         FRCNT_GAP_MS apart (line [8]), the valve (line [9]), the hold line
 *         (line [10]) and the result (line [11])
 *   then  the image stops feeding the watchdog and says so.
 *
 * The probe writes only the plane's bytes. It never writes a pixel valve,
 * HDMI, PHY or HVS register, nor a word of any display list.
 */
#include <zephyr/cache.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "adopt.h"
#include "bus_addr.h"
#include "fb_flush.h"
#include "hvs6_dlist.h"
#include "hvs6_fmt.h"
#include "pattern.h"
#include "pv_hdmi.h"
#include "scanout_alive.h"

#include "dump.h"
#include "frame16.h"
#include "hold_check.h"
#include "plane_match.h" /* tools/probes/rpi5_display/src: iommu4's pass-through */
#include "rpi_wdog.h"

#ifndef RPI5_SCANOUT16_HOLD_MS
#define RPI5_SCANOUT16_HOLD_MS 240000u
#endif

#define ALL_ONES       0xffffffffu
#define SAMPLE_GAP_MS  100u  /* Z1's two samples and the pre-write hashes */
#define FRCNT_GAP_MS   50u   /* line [8]: about 3 frames at 60 Hz, never 64 */
#define FRCNT_WAIT_MS  100u  /* a hold sample waits this long for FRCNT to move */
#define HOLD_SAMPLE_MS 10000u /* one hold sample per 10 s */
#define HOLD_SAMPLES   64u
#define HOLD_ROWS      64u   /* rows hashed per hold sample */
#define TEXT_ROWS      16u   /* fbcon rows wanted (fewer when they do not fit) */
#define LIST_MAX       64u   /* display-list words read from the head */
#define DL_WORDS       ((DUMP_HVS_SIZE - HVS6_DLIST_OFFSET) / 4u)

/* The expected image: the plane's width and height at pitch w * 2. */
#define EXP_MAX_W      2560u
#define EXP_MAX_H      1600u
#define EXP_BYTES      (EXP_MAX_W * EXP_MAX_H * 2u)

/* iommu4, read only (rpi5_display's figures): rpi:arch/arm64/boot/dts/
 * broadcom/bcm2712-ds.dtsi:292-297; words rpi:drivers/iommu/bcm2712-iommu.c
 * (MMMU_CTRL :24, BYPASS_END :63, DEBUG_INFO :74). */
#define IOMMU4_BASE    0x1000005200ull
#define IOMMU4_SIZE    0x80u
#define IO_MMMU_CTRL   0x00u
#define IO_BYPASS_END  0x20u
#define IO_DEBUG_INFO  0x38u

static volatile bool finished;

/* The feeding stops once the result line is out: the reset follows within
 * one watchdog period instead of at the end of the hold. */
bool rpi_wdog_alive(void)
{
	return !finished;
}

static uint8_t exp_buf[EXP_BYTES] __aligned(64);

static enum hvs6_gen gen = HVS6_GEN_UNKNOWN;
static unsigned int scan_ch;   /* the channel line [1] judged */
static unsigned int plane_ch;  /* the channel whose list names the plane */
static unsigned int valve;     /* the pixel valve line [9] reads */
static uint32_t list_at;       /* the list's first word in list memory */
static uint32_t list_n;        /* its words, walked to the END */
static uint32_t list_ref[LIST_MAX];
static uint32_t list_lptrs;    /* plane_ch's LPTRS when the list was read */

static struct frame16_plane fp;  /* the adopted plane (base NULL: none) */
static struct frame16_plane xp;  /* the expected image */
static struct hold_sample hs[HOLD_SAMPLES];

static uint32_t hvs_rd(uint32_t off)
{
	return dump_rd(dump_hvs, DUMP_HVS_SIZE, off);
}

static uint32_t pv_rd(unsigned int v, uint32_t off)
{
	return dump_rd(dump_pv_va[v & 1u], PV0_SIZE, off);
}

static uint32_t chan_rd(unsigned int ch, enum hvs6_chan_reg r)
{
	uint32_t off = hvs6_chan_reg_offset(gen, ch, r);

	return off != 0 ? hvs_rd(off) : ALL_ONES;
}

static uint32_t list_rd(uint32_t word)
{
	return word < DL_WORDS ? hvs_rd(HVS6_DLIST_OFFSET + word * 4u) : ALL_ONES;
}

/* ---- Z1 ------------------------------------------------------------------ */

static void chan_sample(unsigned int ch, unsigned int v, uint32_t version, struct sa_sample *s)
{
	s->hvs_version = version;
	s->ctrl0 = chan_rd(ch, HVS6_REG_CTRL0);
	s->status = chan_rd(ch, HVS6_REG_STATUS);
	s->dl = chan_rd(ch, HVS6_REG_DL);
	s->pv_control = pv_rd(v, SA_PV_CONTROL);
	s->pv_vcontrol = pv_rd(v, SA_PV_V_CONTROL);
}

/* rpi5_display's rule: a running judgement when one valve gives one, else
 * the reason that does not blame a valve, else "pixel valve off". */
static struct sa_verdict chan_pick(const struct sa_verdict vv[2], const bool mapped[2],
				   unsigned int *named)
{
	unsigned int pick = mapped[0] ? 0u : 1u;

	if (mapped[1] && pick == 0u &&
	    (vv[1].running || (!vv[0].running && vv[0].reason == SA_PV_OFF &&
			       vv[1].reason != SA_PV_OFF))) {
		pick = 1u;
	}
	*named = pick;
	return vv[pick];
}

/* plane_ch's list from its head: read, walked entry by entry to the END
 * word (included), the first plane decoded. */
static bool read_list(unsigned int ch, struct hvs6_plane *pl)
{
	uint32_t lptrs = chan_rd(ch, HVS6_REG_LPTRS);
	uint32_t dl = chan_rd(ch, HVS6_REG_DL);
	uint32_t head = dl & HVS6_DL_LACT_MASK;
	uint32_t got, idx = 0;
	bool end = false;
	int rc;

	if (lptrs == ALL_ONES || dl == ALL_ONES) {
		printk("rpi5_scanout16: ch%u list NOT read: LPTRS 0x%08x DL 0x%08x\n", ch, lptrs, dl);
		return false;
	}
	if (head == 0) {
		head = lptrs & HVS6_LPTRS_HEADE_MASK;
		printk("rpi5_scanout16: ch%u DL.LACT is 0, so its list is read from LPTRS.HEADE "
		       "%u\n", ch, head);
	}
	if (head == 0) {
		printk("rpi5_scanout16: ch%u names no list (DL.LACT and LPTRS.HEADE are 0)\n", ch);
		return false;
	}
	got = DL_WORDS - head < LIST_MAX ? DL_WORDS - head : LIST_MAX;
	for (uint32_t i = 0; i < got; i++) {
		list_ref[i] = list_rd(head + i);
	}
	while (idx < got) {
		uint32_t w = list_ref[idx], n;

		if (w & HVS6_CTL0_END) {
			end = true;
			idx++;
			break;
		}
		n = (w >> HVS6_CTL0_NEXT_SHIFT) & HVS6_CTL0_NEXT_MASK;
		if (n == 0) {
			break;
		}
		idx += n;
	}
	list_n = idx > got ? got : idx;
	if (list_n == 0) {
		list_n = got;
	}
	list_at = head;
	list_lptrs = lptrs;
	printk("rpi5_scanout16: ch%u list read from word %u: %u words, END %s; LPTRS 0x%08x "
	       "DL 0x%08x\n", ch, head, list_n, end ? "found" : "NOT found (walk stopped there)",
	       lptrs, dl);
	for (uint32_t i = 0; i < list_n; i++) {
		printk("rpi5_scanout16: ch%u dl[%u] = 0x%08x\n", ch, head + i, list_ref[i]);
	}
	rc = hvs6_dlist_first_plane(list_ref, got, 0, pl);
	if (rc != HVS6_OK) {
		printk("rpi5_scanout16: ch%u first plane NOT decoded (hvs6 error %d)\n", ch, rc);
		return false;
	}
	return true;
}

static const char *plane_fmt(const struct hvs6_plane *p)
{
	const struct hvs6_fmt *f = hvs6_fmt_lookup(p->format_code, p->order);
	const char *nm = f != NULL ? f->name : hvs6_format_name(p->format_code);

	return nm != NULL ? nm : "unknown";
}

static enum rpi5_adopt_format adopt_fmt_of(const char *name)
{
	static const struct {
		const char *name;
		enum rpi5_adopt_format f;
	} t[] = {
		{ "XRGB8888", RPI5_ADOPT_FMT_XRGB8888 }, { "ARGB8888", RPI5_ADOPT_FMT_ARGB8888 },
		{ "XBGR8888", RPI5_ADOPT_FMT_XBGR8888 }, { "ABGR8888", RPI5_ADOPT_FMT_ABGR8888 },
		{ "RGB565", RPI5_ADOPT_FMT_RGB565 },
	};

	for (size_t i = 0; i < ARRAY_SIZE(t); i++) {
		if (strcmp(name, t[i].name) == 0) {
			return t[i].f;
		}
	}
	return RPI5_ADOPT_FMT_UNKNOWN;
}

static int bus2arm(void *ctx, uint64_t bus, uint64_t *arm)
{
	ARG_UNUSED(ctx);
	return bcm2712_bus_to_arm(bus, arm) == 0 ? 1 : 0;
}

/* The plane mapped Normal non-cacheable from the page below it (a plane's
 * address need not be page aligned); NULL when refused. */
static uint8_t *map_plane(uint64_t pa, uint64_t len)
{
	const uint64_t page = 4096ull;
	uint64_t start = pa & ~(page - 1ull);
	uint64_t off = pa - start;
	uint64_t span;
	mm_reg_t va = 0;

	if (len == 0 || len > (1ull << 28)) {
		printk("rpi5_scanout16: a plane of %llu bytes is not mapped (256 MiB at most)\n",
		       (unsigned long long)len);
		return NULL;
	}
	span = (off + len + page - 1ull) & ~(page - 1ull);
	device_map(&va, (uintptr_t)start, (size_t)span, K_MEM_ARM_NORMAL_NC);
	if (va == 0) {
		printk("rpi5_scanout16: the plane at 0x%llx is NOT mapped (0x%llx bytes)\n",
		       (unsigned long long)pa, (unsigned long long)span);
		return NULL;
	}
	printk("rpi5_scanout16: plane mapped Normal non-cacheable at 0x%llx for 0x%llx..0x%llx\n",
	       (unsigned long long)(va + off), (unsigned long long)pa,
	       (unsigned long long)(pa + len));
	return (uint8_t *)va + off;
}

/* ---- cache clean (fb_flush.h) --------------------------------------------- */

static void clean_cb(uint64_t addr, uint64_t len, void *ctx)
{
	int *rc = ctx;
	int r = sys_cache_data_flush_range((void *)(uintptr_t)addr, (size_t)len);

	if (r != 0 && *rc == 0) {
		*rc = r;
	}
}

/* The text rows and the pattern band, two trackers over the plane (fb_flush.h's
 * split screen). Returns the number of clean calls; *rc the first error. */
static uint32_t clean_frame(uint32_t text_rows, int *rc)
{
	struct pattern_layout l = split_layout(fp.w, fp.h);
	uint32_t line = (uint32_t)sys_cache_data_line_size_get();
	struct fb_dirty d;
	uint32_t calls = 0;

	*rc = 0;
	if (line == 0) {
		line = FB_FLUSH_LINE_A76;
	}
	if (text_rows != 0 &&
	    fb_dirty_init_area(&d, (uint64_t)(uintptr_t)fp.base, fp.w, fp.h, 2, fp.pitch, 0,
			       text_rows * FRAME16_TEXT_ROW_PX, line, 1) == 0) {
		(void)fb_dirty_mark(&d, 0, text_rows * FRAME16_TEXT_ROW_PX);
		calls += fb_dirty_flush(&d, clean_cb, rc);
	}
	if (l.pattern.h != 0 &&
	    fb_dirty_init_area(&d, (uint64_t)(uintptr_t)fp.base, fp.w, fp.h, 2, fp.pitch,
			       l.pattern.y, l.pattern.y + l.pattern.h, line, 1) == 0) {
		(void)fb_dirty_mark(&d, l.pattern.y, l.pattern.h);
		calls += fb_dirty_flush(&d, clean_cb, rc);
	}
	return calls;
}

/* ---- hold samples (hold_check.h) ------------------------------------------ */

static uint32_t list_hash_now(void)
{
	uint32_t h = HOLD_FNV_BASIS;

	for (uint32_t i = 0; i < list_n; i++) {
		uint32_t w = list_rd(list_at + i);

		h = hold_fnv1a_n(h, (const volatile uint8_t *)&w, sizeof(w));
	}
	return h;
}

static uint32_t plane_hash_now(void)
{
	return fp.base != NULL ? hold_rows_hash(fp.base, fp.pitch, fp.h, HOLD_ROWS) : 0u;
}

/* Rows whose visible bytes differ from the expected image. */
static uint32_t rows_touched_now(void)
{
	uint32_t rows = 0;

	if (fp.base == NULL || xp.base == NULL) {
		return 0;
	}
	for (uint32_t y = 0; y < fp.h; y++) {
		const volatile uint8_t *a = (const volatile uint8_t *)fp.base + (size_t)y * fp.pitch;
		const uint8_t *b = (const uint8_t *)xp.base + (size_t)y * xp.pitch;

		for (uint32_t i = 0; i < fp.w * 2u; i++) {
			if (a[i] != b[i]) {
				rows++;
				break;
			}
		}
	}
	return rows;
}

/* One sample. With prev, STATUS is polled up to FRCNT_WAIT_MS until FRCNT
 * differs from prev's: two samples seconds apart can hold the same 6-bit
 * count by aliasing, while a running counter moves within a frame. */
static void take_sample(struct hold_sample *s, const struct hold_sample *prev,
			const struct hold_sample *ref)
{
	uint32_t st = chan_rd(scan_ch, HVS6_REG_STATUS);

	if (prev != NULL && !hold_unclocked(st)) {
		int64_t until = k_uptime_get() + FRCNT_WAIT_MS;

		while (hold_frcnt(st) == hold_frcnt(prev->status) && k_uptime_get() < until) {
			k_msleep(2);
			st = chan_rd(scan_ch, HVS6_REG_STATUS);
		}
	}
	s->status = st;
	s->dl_ptr = chan_rd(plane_ch, HVS6_REG_LPTRS);
	s->list_hash = list_hash_now();
	s->plane_hash = plane_hash_now();
	s->rows_touched = ref != NULL && s->plane_hash != ref->plane_hash ? rows_touched_now() : 0u;
	s->pv_control = pv_rd(valve, SA_PV_CONTROL);
	s->pv_vcontrol = pv_rd(valve, SA_PV_V_CONTROL);
}

static uint32_t list_words_changed(void)
{
	uint32_t n = 0;

	for (uint32_t i = 0; i < list_n; i++) {
		n += list_rd(list_at + i) != list_ref[i];
	}
	return n;
}

int main(void)
{
	struct sa_verdict v[SA_NUM_CHANNELS];
	unsigned int vused[SA_NUM_CHANNELS] = { 0 };
	bool mapped[2];
	bool z1 = false, have_plane = false, adopted = false, before_changed = false;
	bool drawn = false, clamped = false;
	struct hvs6_plane pl;
	struct pm_mmu mmu;
	struct frame16_rb rb1, rb2;
	struct hold_sample ref;
	struct hold_verdict hv;
	uint32_t version, text_rows = 0, changed = 0, nsamp = 0;
	uint32_t held_ms = 0, words = 0, fr_before, fr_after, pvc, pvv;
	const char *pat_why = NULL;
	const char *ad_state = "FAIL";
	const char *fail = NULL;
	char ad_why[200];
	char plane_txt[160];
	char line[240];
	mm_reg_t va_io = 0;
	int rc, te;

	memset(&pl, 0, sizeof(pl));
	memset(&ref, 0, sizeof(ref));
	memset(&fp, 0, sizeof(fp));
	memset(&xp, 0, sizeof(xp));
	(void)snprintf(plane_txt, sizeof(plane_txt), "none");
	(void)snprintf(ad_why, sizeof(ad_why), "not attempted: no plane was decoded");

	printk("rpi5_scanout16: build %s\n", RPI5_SCANOUT16_BUILD);
	rpi_wdog_announce();
	printk("rpi5_scanout16: frame 1: adopt the firmware's RGB565 plane, fbcon text and a "
	       "pattern, read back, hold %u ms, read back again; no pixel valve, HDMI, PHY, HVS "
	       "register or display-list write\n", (unsigned int)RPI5_SCANOUT16_HOLD_MS);
	rc = dump_all();
	printk("rpi5_scanout16: dump rc=%d\n", rc);

	version = hvs_rd(HVS6_VERSION);
	gen = hvs6_gen_from_version(version);
	mapped[0] = dump_pv_va[0] != 0;
	mapped[1] = dump_pv_va[1] != 0;

	/* iommu4: an address it may translate is never written. */
	device_map(&va_io, (uintptr_t)IOMMU4_BASE, IOMMU4_SIZE, K_MEM_CACHE_NONE);
	if (va_io != 0) {
		uint32_t c = sys_read32(va_io + IO_MMMU_CTRL);
		uint32_t d = sys_read32(va_io + IO_DEBUG_INFO);
		uint32_t e = sys_read32(va_io + IO_BYPASS_END);

		pm_mmu_state(c, d, e, true, &mmu);
		printk("rpi5_scanout16: iommu4 MMMU_CTRL 0x%08x DEBUG_INFO 0x%08x BYPASS_END 0x%08x: "
		       "%s, pass-through below 0x%llx\n", c, d, e,
		       mmu.enabled ? "enabled" : "disabled", (unsigned long long)mmu.pass_below);
	} else {
		pm_mmu_state(0, 0, 0, false, &mmu);
		printk("rpi5_scanout16: iommu4 NOT mapped: no plane address is known to pass "
		       "through, nothing is written\n");
	}

	/* ---- Z1: lines [1] and [2] ---- */
	if (dump_hvs != 0 && gen != HVS6_GEN_UNKNOWN && (mapped[0] || mapped[1])) {
		struct sa_sample a[SA_NUM_CHANNELS][2], b[SA_NUM_CHANNELS][2];
		unsigned int i, k;

		for (i = 0; i < SA_NUM_CHANNELS; i++) {
			for (k = 0; k < 2u; k++) {
				chan_sample(i, k, version, &a[i][k]);
			}
		}
		k_msleep(SAMPLE_GAP_MS);
		for (i = 0; i < SA_NUM_CHANNELS; i++) {
			struct sa_verdict vv[2];

			memset(vv, 0, sizeof(vv));
			for (k = 0; k < 2u; k++) {
				chan_sample(i, k, version, &b[i][k]);
				if (mapped[k]) {
					vv[k] = sa_judge(&a[i][k], &b[i][k], i);
				}
			}
			v[i] = chan_pick(vv, mapped, &vused[i]);
			printk("rpi5_scanout16: ch%u judged with pixelvalve%u: %s; FRCNT %u then %u, "
			       "line %u then %u, %u ms apart\n", i, vused[i],
			       v[i].running ? "running" : sa_reason_text(v[i].reason), v[i].frcnt_a,
			       v[i].frcnt_b, v[i].yline_a, v[i].yline_b, SAMPLE_GAP_MS);
		}
		for (i = 0; i < SA_NUM_CHANNELS; i++) {
			if (v[i].running) {
				scan_ch = i;
				break;
			}
		}
		valve = vused[scan_ch];
		(void)sa_format(&v[scan_ch], line, sizeof(line));
		printk("%s\n", line);
		z1 = true;

		/* The plane: the scanned channel's list first, then the others. */
		for (i = 0; i < SA_NUM_CHANNELS && !have_plane; i++) {
			unsigned int ch = (scan_ch + i) % SA_NUM_CHANNELS;

			if (read_list(ch, &pl)) {
				plane_ch = ch;
				have_plane = true;
			}
		}
		if (have_plane) {
			uint64_t arm = pl.addr;

			printk("rpi5_scanout16: ch%u plane: list address 0x%llx at %u,%u, %ux%u, pitch "
			       "%u, format %u order %u (%s), %u words\n", plane_ch,
			       (unsigned long long)pl.addr, pl.x, pl.y, pl.width, pl.height, pl.pitch,
			       pl.format_code, pl.order, plane_fmt(&pl), pl.nwords);
			if (bcm2712_bus_to_arm(pl.addr, &arm) != 0) {
				arm = pl.addr;
			}
			(void)snprintf(plane_txt, sizeof(plane_txt),
				       "addr=0x%llx fmt=%s pitch=%u w=%u h=%u",
				       (unsigned long long)arm, plane_fmt(&pl), pl.pitch, pl.width,
				       pl.height);
		}
	} else {
		printk("u3 scanout: stopped not measured (%s)\n",
		       dump_hvs == 0 ? "the HVS was not mapped"
		       : gen == HVS6_GEN_UNKNOWN ? "the HVS VERSION names neither stepping"
						 : "neither pixel valve was mapped");
		(void)snprintf(ad_why, sizeof(ad_why), "not attempted: no channel was sampled");
	}
	printk("u3 plane: %s\n", plane_txt);

	/* ---- adopt: line [3] ---- */
	if (have_plane) {
		struct rpi5_adopt_plane ap = { pl.addr, adopt_fmt_of(plane_fmt(&pl)), pl.pitch,
					       pl.width, pl.height };
		struct rpi5_adopt_env env = {
			BCM2712_DRAM_BASE, BCM2712_DRAM_SIZE, CONFIG_SRAM_BASE_ADDRESS,
			(uint64_t)CONFIG_SRAM_SIZE * 1024ull, bus2arm, NULL,
		};
		struct rpi5_adopt_out out;
		struct rpi5_adopt_result ar;
		uint8_t *base = NULL;

		memset(&out, 0, sizeof(out));
		ar = rpi5_adopt_check(&ap, &env, &out);
		if (ar.verdict != RPI5_ADOPT) {
			(void)snprintf(ad_why, sizeof(ad_why), "adopt refused the plane: %s",
				       ar.reason);
		} else if (out.bpp != 2u) {
			(void)snprintf(ad_why, sizeof(ad_why),
				       "the plane is %s, %u bytes per pixel: this probe draws RGB565 "
				       "only", plane_fmt(&pl), out.bpp);
		} else if (!pm_addr_passes_through(&mmu, out.phys)) {
			(void)snprintf(ad_why, sizeof(ad_why),
				       "not attempted: 0x%llx may be an IOVA (iommu4 %s)",
				       (unsigned long long)out.phys, mmu.read ? "enabled" : "not read");
		} else if ((uint64_t)out.width * out.height * 2u > EXP_BYTES) {
			(void)snprintf(ad_why, sizeof(ad_why),
				       "the plane %ux%u is larger than the expected image (%ux%u)",
				       out.width, out.height, EXP_MAX_W, EXP_MAX_H);
		} else if ((base = map_plane(out.phys, out.size)) == NULL) {
			(void)snprintf(ad_why, sizeof(ad_why), "the plane at 0x%llx was not mapped",
				       (unsigned long long)out.phys);
		} else {
			struct frame16_plane p = { base, out.pitch, out.width, out.height,
						   PATTERN_RGB565 };
			struct yd5_fc565 f = frame16_fc(&p);
			uint32_t h0 = hold_rows_hash(base, out.pitch, out.height, HOLD_ROWS);
			uint32_t h1, bad = 0;
			int ce, fe;

			k_msleep(SAMPLE_GAP_MS);
			h1 = hold_rows_hash(base, out.pitch, out.height, HOLD_ROWS);
			before_changed = h0 != h1;
			ce = yd5_fc565_clear(&f, 0, out.height, FRAME16_BG);
			fe = sys_cache_data_flush_range(base, (size_t)out.size);
			for (uint32_t y = 0; y < out.height; y++) {
				const volatile uint16_t *r =
					(const volatile uint16_t *)(base + (size_t)y * out.pitch);

				for (uint32_t x = 0; x < out.width; x++) {
					bad += r[x] != FRAME16_BG;
				}
			}
			printk("rpi5_scanout16: adopt phys 0x%llx %ux%u pitch %u, %llu bytes; plane "
			       "hash 0x%08x then 0x%08x %u ms apart before the write; clear %d "
			       "flush %d, %u pixels not 0x%04x\n", (unsigned long long)out.phys,
			       out.width, out.height, out.pitch, (unsigned long long)out.size, h0, h1,
			       SAMPLE_GAP_MS, ce, fe, bad, FRAME16_BG);
			if (ce == 0 && bad == 0) {
				ad_state = "PASS";
				fp = p;
				adopted = true;
				(void)snprintf(ad_why, sizeof(ad_why),
					       "%ux%u RGB565 at 0x%llx adopted, cleared and read back, "
					       "%u bytes; plane %s before the write", out.width,
					       out.height, (unsigned long long)out.phys,
					       out.width * out.height * 2u,
					       before_changed ? "CHANGED" : "steady");
			} else {
				(void)snprintf(ad_why, sizeof(ad_why),
					       "clear %d, %u of %u pixels did not read back 0x%04x", ce,
					       bad, out.width * out.height, FRAME16_BG);
			}
		}
	}
	printk("u3 adopt-readback: %s (%s)\n", ad_state, ad_why);

	/* ---- draw: lines [4], [5] and [6] ---- */
	if (adopted) {
		xp = (struct frame16_plane){ exp_buf, fp.w * 2u, fp.w, fp.h, PATTERN_RGB565 };
		memset(exp_buf, 0, (size_t)xp.pitch * xp.h); /* FRAME16_BG, as the plane */
		text_rows = frame16_text_max_rows(&fp);
		if (text_rows > TEXT_ROWS) {
			text_rows = TEXT_ROWS;
		}
		(void)frame16_render(&xp, text_rows, NULL, NULL, NULL);
	}
	te = frame16_draw_text(&fp, text_rows, &changed);
	drawn = frame16_draw_pattern(&fp, &pat_why);
	if (te != 0) {
		printk("rpi5_scanout16: text draw error %d (%s)\n", te,
		       te == FRAME16_E_PLANE ? "no adopted RGB565 plane"
		       : te == FRAME16_E_FIT ? "the rows do not fit" : yd5_fc565_err_name(te));
	}
	(void)frame16_fmt_text_rows(line, sizeof(line), changed);
	printk("%s\n", line);
	(void)frame16_fmt_pattern(line, sizeof(line), drawn, pat_why);
	printk("%s\n", line);
	if (adopted) {
		int crc;
		uint32_t calls = clean_frame(text_rows, &crc);

		printk("rpi5_scanout16: %u text rows of %u px and the pattern band cleaned to memory: "
		       "%u clean calls, rc %d\n", text_rows, FRAME16_TEXT_ROW_PX, calls, crc);
	}
	rb1 = frame16_readback(&fp, &xp);
	(void)frame16_fmt_readback(line, sizeof(line), 1, &rb1);
	printk("%s\n", line);

	/* ---- hold ---- */
	if (adopted) {
		uint32_t start, end, want, step;

		take_sample(&ref, NULL, NULL);
		ref.list_hash = HOLD_FNV_BASIS; /* the adopted snapshot, not a re-read */
		for (uint32_t i = 0; i < list_n; i++) {
			ref.list_hash = hold_fnv1a_n(ref.list_hash,
						     (const volatile uint8_t *)&list_ref[i], 4u);
		}
		ref.dl_ptr = list_lptrs;
		start = k_uptime_get_32();
		end = hold_clamp_end(start, RPI5_SCANOUT16_HOLD_MS, RPI_WDOG_HOLD_S, &clamped);
		want = end - start;
		nsamp = want / HOLD_SAMPLE_MS;
		if (nsamp == 0 && want != 0) {
			nsamp = 1;
		}
		if (nsamp > HOLD_SAMPLES) {
			nsamp = HOLD_SAMPLES;
		}
		step = nsamp != 0 ? want / nsamp : 0;
		printk("rpi5_scanout16: hold from %u ms to %u ms since boot (%u ms asked%s), %u "
		       "samples; reference STATUS 0x%08x LPTRS 0x%08x list 0x%08x plane 0x%08x\n",
		       start, end, (unsigned int)RPI5_SCANOUT16_HOLD_MS,
		       clamped ? ", clamped to the watchdog's feeding time" : "", nsamp, ref.status,
		       ref.dl_ptr, ref.list_hash, ref.plane_hash);
		for (uint32_t i = 0; i < nsamp; i++) {
			uint32_t at = start + (i + 1u) * step;
			uint32_t now = k_uptime_get_32();

			if (at > now) {
				k_msleep((int32_t)(at - now));
			}
			take_sample(&hs[i], i ? &hs[i - 1] : &ref, &ref);
			printk("rpi5_scanout16: hold sample %u at %u ms: FRCNT %u LPTRS 0x%08x list "
			       "0x%08x plane 0x%08x rows %u PV 0x%08x/0x%08x\n", i,
			       k_uptime_get_32() - start, hold_frcnt(hs[i].status), hs[i].dl_ptr,
			       hs[i].list_hash, hs[i].plane_hash, hs[i].rows_touched,
			       hs[i].pv_control, hs[i].pv_vcontrol);
		}
		held_ms = k_uptime_get_32() - start;
	}
	hv = hold_judge(&ref, hs, nsamp, before_changed);
	rb2 = frame16_readback(&fp, &xp);
	(void)frame16_fmt_readback(line, sizeof(line), 2, &rb2);
	printk("%s\n", line);

	/* ---- lines [8], [9] ---- */
	{
		uint32_t s0 = chan_rd(scan_ch, HVS6_REG_STATUS), s1;

		k_msleep(FRCNT_GAP_MS);
		s1 = chan_rd(scan_ch, HVS6_REG_STATUS);
		fr_before = hold_frcnt(s0);
		fr_after = hold_frcnt(s1);
		pvc = pv_rd(valve, SA_PV_CONTROL);
		pvv = pv_rd(valve, SA_PV_V_CONTROL);
		printk("rpi5_scanout16: ch%u STATUS 0x%08x then 0x%08x %u ms apart; pixelvalve%u "
		       "PV_CONTROL 0x%08x PV_V_CONTROL 0x%08x\n", scan_ch, s0, s1, FRCNT_GAP_MS,
		       valve, pvc, pvv);
		if (hold_unclocked(s0) || hold_unclocked(s1)) {
			fail = "STATUS read all ones";
		}
		if (hold_unclocked(pvc) || hold_unclocked(pvv)) {
			/* an all-ones read is not EN or VIDEN set: line [9] carries 0 */
			printk("rpi5_scanout16: pixelvalve%u read all ones (not mapped or unclocked); "
			       "line [9] reports 0\n", valve);
			pvc = 0;
			pvv = 0;
		}
	}
	(void)frame16_fmt_frcnt(line, sizeof(line), fr_before, fr_after);
	printk("%s\n", line);
	(void)frame16_fmt_pv(line, sizeof(line), pvc, pvv);
	printk("%s\n", line);

	/* ---- line [10] ---- */
	if (adopted) {
		words = list_words_changed();
	}
	printk("rpi5_scanout16: hold verdict %s (%s) at sample %d, redraw %s, rows touched %u, "
	       "%u list words differ from the adopted snapshot, %u plane bytes differ\n",
	       hv.pass ? "pass" : "FAIL", hold_reason_name(hv.reason), hv.index,
	       hold_redraw_name(hv.redraw), hv.rows_touched, words, rb2.mismatch);
	(void)frame16_fmt_hold(line, sizeof(line), held_ms, &hv, rb2.mismatch, words);
	printk("%s\n", line);

	/* ---- line [11] ---- */
	if (fail == NULL) {
		fail = !z1               ? "no channel was sampled"
		       : !have_plane     ? "no plane was decoded"
		       : !adopted        ? "the plane was not adopted"
		       : te != 0         ? "the text was not drawn"
		       : !drawn          ? "the pattern was not drawn"
		       : !rb1.pass       ? "readback 1 differs"
		       : !rb2.pass       ? "readback 2 differs after the hold"
		       : ((fr_after - fr_before) & HOLD_FRCNT_MASK) == 0 ? "FRCNT did not advance"
		       : !hold_pv_enabled(pvc, pvv) ? "PV_CONTROL.EN or PV_V_CONTROL.VIDEN is 0"
						    : NULL;
	}
	if (fail == NULL) {
		(void)snprintf(ad_why, sizeof(ad_why),
			       "plane adopted, %u text rows and the pattern read back byte-equal "
			       "twice, %u ms hold, FRCNT advanced, EN and VIDEN set", text_rows,
			       held_ms);
		(void)frame16_fmt_terminal(line, sizeof(line), true, ad_why);
	} else {
		(void)frame16_fmt_terminal(line, sizeof(line), false, fail);
	}
	printk("%s\n", line);

	finished = true;
	(void)frame16_fmt_feeding_stopped(line, sizeof(line));
	printk("%s (the PM watchdog resets the SoC, and tryboot returns the board to Linux)\n",
	       line);
	for (;;) {
		k_sleep(K_FOREVER);
	}
	return 0;
}
