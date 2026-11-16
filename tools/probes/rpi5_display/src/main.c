/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_display: U3 on the Raspberry Pi 5 (BCM2712) — is a scanout running when
 * a kernel= image starts, where does it read from, and do the bytes this image
 * writes reach it?
 *
 * driver/docs/rpi5-display.md section 6 is the step plan this image runs (Z0 to
 * Z4) and section 7 the rule its verdict lines name. The `u3` lines are the ones
 * tools/probes/rpi5_display/verdicts.sh reads: each is printed in its order,
 * once, where it was decided, and none is written before the reading that names
 * it. A step that could not run says so by name instead of guessing. The record
 * lines (the `u3 reg` and `u3 dlist` words of rpi5_scanout/snapshot.h, and the
 * rpi5_display: lines) are printed where they were read.
 *
 * The item's six steps map onto the design note's Z-steps as follows; each
 * begins with a `rpi5_display: step <n> <name> at <ms> ms since boot` line:
 *   1 dt-fb            the DTB in x0 (rpi_5/common/boot keeps it) through
 *                      fwfb/fw_fb_dt.c: `u3 dt-fb: <fmt> <w>x<h> stride <n>
 *                      status <okay|disabled> <addr-set|addr-0>`, `u3 dt-fb:
 *                      none (...)` or `u3 dt-fb: skip (<reason>)`
 *   2 scanout          Z0 and Z1's sampling: `u3 scanout:` and `u3 plane:`
 *   4 adopt-readback   Z1's adopt read-back: `u3 adopt-readback:`
 *   3 mailbox          Z2: `u3 fb-query: <present|absent|unknown|malformed>
 *                      (...)` (fb_query.h), `u3 fb-alloc: <granted|...> (...)`
 *                      and `u3 mailbox-fb:`
 *   4 mailbox-readback Z3: `u3 mailbox-readback:`, `u3 scanout-follows-mailbox:`
 *   5 split-screen     Z4: `u3 split-screen: drawn (...)|not drawn (...)`
 *   6 summary          `rpi5_display: PASS|FAIL (...)`, `measured:` and the
 *                      watchdog line
 * Step 4 runs twice because verdicts.sh's order puts the adopt read-back before
 * the mailbox. The eight lines verdicts.sh reads never say `skip`: a skipped
 * line makes its log incomplete, so a step that could not run prints FAIL (or
 * stopped, or no) with the reason. The record-only lines (dt-fb, fb-query,
 * fb-alloc) say `skip (<reason>)`.
 *
 *   Z0  get_throttled; the firmware clocks 4 (core), 16 (disp), 13 (hdmi/HSM),
 *       14 (bvb) and 5 (v3d); iommu4's control words; the whole register table
 *       of rpi5_scanout/snapshot.h read once and printed, so the HVS6 VERSION,
 *       CXM_SIZE and CONTROL are in it and a Linux L4 dump can be compared
 *       against it word for word.
 *       The clocks come first because of what a read can cost: the HSM clock
 *       answered 0 means no `hd` read at all (Linux's own reason: an HDMI
 *       register access with that clock at 0 "will lead to a silent CPU stall",
 *       rpi:drivers/gpu/drm/vc4/vc4_hdmi.c:3251-3262), and the design note's
 *       rule for the display clock is that a 0 is treated as "do not read the
 *       HVS" until a run shows otherwise (section 8). A clock whose tag the
 *       firmware did not answer is not an answered 0: only an answered 0 stops a
 *       read, and both cases are printed. A block that was not mapped, or not
 *       read, prints its words as 0xffffffff with a line naming why.
 *   Z1  channels 0..2 of the HVS, each read twice SAMPLE_GAP_MS apart and judged
 *       by rpi5_scanout/scanout_alive.h (an enabled channel, an enabled pixel
 *       valve, a frame count or line that moved). `u3 scanout:` is that judge's
 *       line for the first running channel, else for channel 0. For every
 *       channel whose DL or LPTRS names a list, the words from the list head
 *       (DL.LACT, or LPTRS.HEADE when LACT is 0) to the END word are read once
 *       and printed as they were read, and the plane is decoded out of those
 *       same captured words — so `u3 plane:` names a plane out of the printed
 *       record and not out of a second read. That plane then goes through
 *       rpi5_scanout/adopt.h; when adopt allows it its buffer is mapped, the
 *       pattern is written into it and read back twice 100 ms apart, the second
 *       read for the firmware's own drawing (section 8's last open item).
 *   Z2  the firmware's own framebuffer: the six GET tags first, then the Pi 4's
 *       one-request allocation rpi_fb_alloc_virtual (physical 1280x720, virtual
 *       1280x1440, depth 32, the pixel-order tag answered RGB, virtual offset
 *       0,0, allocate 4096, get pitch). Every granted value is printed, and a
 *       grant that is not the window asked for is named by its numbers
 *       (section 8), not used as if it were.
 *   Z3  the pattern in that buffer, written and read back twice; then the
 *       running channel's list read again and compared with the buffer by
 *       plane_match.h — with iommu4's pass-through state in front of it, because
 *       the address in a list is the address the HVS fetches from and above the
 *       pass-through window it may be an IOVA. The list is re-read after the
 *       drawing: the drawing cannot change the list, and reading it last keeps
 *       the two readings as far apart as this image can make them.
 *   Z4  the Pi 4's split screen on that buffer, or on the adopted plane when no
 *       grant read back and the adopted one did (pattern.h's layout): the console
 *       on top (fbcon, the hook chain the Pi 4's board layer uses) and the
 *       pattern below it. Both bands are read back against the pattern: the text
 *       band must have lost pattern pixels (the console drew there) and the
 *       pattern band must still hold the pattern. The verdict lines printed from
 *       here on are drawn into the text band, so the log is on the screen too.
 *
 * The image reads. It writes no scanout, pixel valve or HDMI register and asks
 * the firmware for nothing but the framebuffer; stopping the firmware's HDMI
 * (fw_stop.h, route C's write path) is not this image's.
 *
 * The PM watchdog is armed by the board layer in its first PRE_KERNEL_1 hook,
 * before main (CONFIG_YAKOGL_RPI5_WDOG, hold CONFIG_YAKOGL_RPI5_WDOG_HOLD_S), so
 * every step here is bounded: an image that hangs is reset by the SoC and
 * tryboot brings Linux back (tools/board.sh pi5).
 *
 * The terminal line is PASS when this image's own memory path held: a granted
 * framebuffer (or, without one, the adopted plane), its pattern read back
 * byte-equal twice, and the split screen drawn with both bands read back. That is this image's claim, not the panel's:
 * section 7's proof is the registers and the bytes, and a capture frame or a
 * person at the monitor confirms, never proves. Which route the board takes is
 * the host's decision, from the u3 lines (verdicts.sh, route_choice.sh); a route
 * A or B result is named by those lines passing or not, never by this line.
 */
#include <zephyr/cache.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/printk-hooks.h>
#include <zephyr/sys/sys_io.h>

#include <errno.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "fbcon.h"
#include "rpi_mbox.h"
#include "rpi_wdog.h"
#include "rpi5_screen.h"

#include "fdt_walk.h"
#include "fw_fb_dt.h"
#include "rpi5_boot.h"

#include "adopt.h"
#include "bus_addr.h"
#include "fb_query.h"
#include "fw_clk.h"
#include "hvs6_dlist.h"
#include "hvs6_fmt.h"
#include "pattern.h"
#include "pv_hdmi.h"
#include "scanout_alive.h"
#include "snapshot.h"
#include "throttled.h"

#include "plane_match.h"

#define FB_W          1280u
#define FB_H          720u
#define FB_VIRT       (FB_H * 2u) /* the flip window: the buffer is twice as tall as the screen */
#define SAMPLE_GAP_MS 100u       /* two samples span a frame at 60 Hz; Z3 reads twice too */
#define ALL_ONES      0xffffffffu
#define PAGE_SIZE_4K  0x1000ull
#define DTB_MAX       (4ull * 1024ull * 1024ull) /* the Pi 5's DTB is some 60 KiB (rpi_5/rpi5_dtb) */

/* Bases and sizes as driver/docs/rpi5-display.md section 2 lists them. The pixel
 * valves and the `hd` region come from pv_hdmi.h, which cites the device tree. */
#define HVS_BASE        0x107c580000ull /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:612-617 */
#define HVS_SIZE        0x1a000u        /* :615 (reg <0x7c580000 0x1a000> under axi) */
#define IOMMU4_BASE     0x1000005200ull /* rpi:arch/arm64/boot/dts/broadcom/bcm2712-ds.dtsi:292-297:
					 * two address cells, 0x10 0x5200, under axi; the hvs node's
					 * `iommus = <&iommu4>` is at bcm2712.dtsi:622 */
#define IOMMU4_SIZE     0x80u           /* :295 */
/* iommu4's words (rpi:drivers/iommu/bcm2712-iommu.c: MMMU_CTRL :24, ADDR_CAP :52,
 * BYPASS_START :193, BYPASS_END :63, DEBUG_INFO :74), the ones plane_match.h
 * classifies. */
#define IO_MMMU_CTRL    0x00u
#define IO_ADDR_CAP     0x14u
#define IO_BYPASS_START 0x1cu
#define IO_BYPASS_END   0x20u
#define IO_DEBUG_INFO   0x38u

/* The buffer step 5 draws the split screen into: the mailbox's grant when its
 * read-back passed, else the adopted firmware plane when its read-back passed
 * (32 bpp only: fbcon.c draws 32 bpp). `cpu` NULL: none. */
struct screen {
	uint8_t *cpu;
	uint32_t width, height, pitch, order;
	const char *what;
};

static mm_reg_t va_hvs, va_pv0, va_pv1, va_hd, va_iommu;
static bool hvs_on, pv_on, pv1_on, hd_on, iommu_on;

/* The whole register table, read once in Z0 and printed: every word a Linux L4
 * dump (linux_ref.sh) can be compared against, and the source of the channel
 * offsets chan_words_read() re-reads. */
static uint32_t snap_words[SNAP_COUNT];

/* A channel's display-list words, captured as they are read, so the `u3 dlist`
 * lines and the plane decoded from them are the same words of the same read. */
static uint32_t dlist[SNAP_DLIST_MAX];
static uint32_t dlist_at;    /* index in the list memory of dlist[0] */
static size_t dlist_n;       /* words captured */
static bool dlist_capture;

/* printk goes to the UART and, once fbcon is attached, to the framebuffer too:
 * fbcon_attach() replaces the hook, so the old one is kept and ours is put back
 * in front of both (the Pi 4's board layer's chain). */
static printk_hook_fn_t uart_out, fb_out;

static int console_out(int c)
{
	if (uart_out) {
		uart_out(c);
	}
	return fb_out ? fb_out(c) : c;
}

static void u3_line(void *ctx, const char *text)
{
	(void)ctx;
	printk("%s\n", text);
}

/* snapshot.h's injected reader. An unmapped or gated block reads as ALL_ONES and
 * the Z0 record names why (the "not read" lines below); 0xffffffff is also what
 * an unread register on a running bus can read as, so the words are never read
 * on their own — the line naming the block is part of the record. */
static uint32_t reg_read32(void *ctx, enum snap_block block, uint32_t off)
{
	(void)ctx;

	if (block == SNAP_HVS) {
		if (!hvs_on) {
			return ALL_ONES;
		}
		if (off >= SNAP_HVS_DLIST_OFFSET) {
			uint32_t idx = (off - SNAP_HVS_DLIST_OFFSET) / 4u;
			uint32_t val = sys_read32(va_hvs + off);

			if (dlist_capture && idx >= dlist_at && idx < dlist_at + SNAP_DLIST_MAX) {
				dlist[idx - dlist_at] = val;
				if (idx + 1u - dlist_at > dlist_n) {
					dlist_n = idx + 1u - dlist_at;
				}
			}
			return val;
		}
		return sys_read32(va_hvs + off);
	}
	if (block == SNAP_PV) {
		return pv_on ? sys_read32(va_pv0 + off) : ALL_ONES;
	}
	if (block == SNAP_HDMI0) {
		return hd_on ? sys_read32(va_hd + off) : ALL_ONES;
	}
	return ALL_ONES;
}

static const struct snap_io snap_io = { reg_read32, NULL };
static const struct snap_emit snap_emit = { u3_line, NULL };

/* A table entry by name: the channel and pixel-valve words are read through the
 * table because the table is the only place those offsets carry a citation. A
 * name the table does not hold is a bug in this file, and is printed as one. */
static const struct snap_reg *snap_reg(const char *name)
{
	size_t i;

	for (i = 0; i < SNAP_COUNT; i++) {
		if (strcmp(snap_table[i].name, name) == 0) {
			return &snap_table[i];
		}
	}
	printk("rpi5_display: the register table holds no %s\n", name);
	return NULL;
}

/* The Z0 word of one table entry (0xffffffff when it was not read). */
static uint32_t snap_word(const char *name)
{
	const struct snap_reg *r = snap_reg(name);
	size_t i;

	if (r == NULL) {
		return ALL_ONES;
	}
	for (i = 0; i < SNAP_COUNT; i++) {
		if (&snap_table[i] == r) {
			return snap_words[i];
		}
	}
	return ALL_ONES;
}

static bool map_block(mm_reg_t *out, uint64_t pa, uint32_t size, const char *name)
{
	*out = 0;
	printk("rpi5_display: mapping %s at 0x%llx, %u bytes\n", name, (unsigned long long)pa, size);
	device_map(out, (uintptr_t)pa, size, K_MEM_CACHE_NONE);
	if (*out == 0) {
		printk("rpi5_display: %s NOT mapped; every one of its words below reads 0x%08x\n", name,
		       ALL_ONES);
		return false;
	}
	printk("rpi5_display: %s read from 0x%llx, uncached\n", name, (unsigned long long)pa);
	return true;
}

/* A plane's buffer mapped the way the board layer maps a mailbox grant (Normal
 * non-cacheable: RPI_FB_MAPPING, rpi_mbox.h:36 and rpi_mbox.c's device_map of
 * the granted address), starting one page below it because a plane's address need
 * not be page aligned. NULL when this refuses, with the refusal named. */
static uint8_t *map_plane(uint64_t pa, uint64_t len)
{
	const uint64_t page = 4096ull;
	uint64_t start = pa & ~(page - 1ull);
	uint64_t off = pa - start;
	uint64_t span;
	mm_reg_t va = 0;

	if (len == 0 || len > (1ull << 28)) {
		printk("rpi5_display: a plane of %llu bytes is not mapped: the size is refused "
		       "(1 GiB at most)\n", (unsigned long long)len);
		return NULL;
	}
	span = (off + len + page - 1ull) & ~(page - 1ull);
	device_map(&va, (uintptr_t)start, (size_t)span, K_MEM_ARM_NORMAL_NC);
	if (va == 0) {
		printk("rpi5_display: the plane at 0x%llx is not mapped (0x%llx bytes refused instead of "
		       "0x%llx)\n", (unsigned long long)pa, (unsigned long long)span,
		       (unsigned long long)len);
		return NULL;
	}
	printk("rpi5_display: plane mapped covering 0x%llx..0x%llx for 0x%llx..0x%llx (%s)\n",
	       (unsigned long long)va, (unsigned long long)(va + span), (unsigned long long)pa,
	       (unsigned long long)(pa + len), RPI_FB_MAPPING);
	return (uint8_t *)va + off;
}

/* Every step starts with this line, so a record says when each step began and a
 * hang names the step it happened in. */
static void step_at(const char *step)
{
	printk("rpi5_display: step %s at %lld ms since boot\n", step, (long long)k_uptime_get());
}

/* The CPU's bytes cleaned to the point of coherency before anything is read
 * back or shown. The buffers here are mapped Normal non-cacheable, so the clean
 * finds nothing to write; it is made anyway, so a later cacheable mapping keeps
 * the same order (rpi5_scanout/fb_flush.h's rule: the HVS reads memory, not
 * the A76's caches). */
static int clean_range(void *p, uint64_t len)
{
	return sys_cache_data_flush_range(p, (size_t)len);
}

/* ---- step 1: the firmware's DTB ------------------------------------------------- */

static uint32_t dtb_be32(const uint8_t *p)
{
	return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | p[3];
}

/* A DTB page range mapped cacheable (the walker may read unaligned, which
 * device memory forbids: the rpi_5/rpi5_dtb probe's mapping). */
static const uint8_t *map_dtb(uint64_t pa, uint64_t len)
{
	uint64_t base = pa & ~(PAGE_SIZE_4K - 1ull);
	uint64_t off = pa - base;
	mm_reg_t va = 0;

	device_map(&va, (uintptr_t)base, (size_t)ROUND_UP(off + len, PAGE_SIZE_4K), K_MEM_CACHE_WB);
	return va == 0 ? NULL : (const uint8_t *)va + off;
}

/* The `u3 dt-fb:` line: the simple-framebuffer node of the DTB in x0, through
 * fwfb/fw_fb_dt.c (the token is fw_fb_dt_verdict's without its "dt-fb " word,
 * then fw_fb_dt_addr_token's). The address and the covering reserved region go
 * into a record line only. Nothing in this step decides a route: it says what
 * the firmware told a kernel= image, next to what the registers say. */
static void step_dtfb(const uint64_t sram_last)
{
	static struct fw_fb_dt r; /* four nodes of paths: not on the stack */
	const volatile struct rpi5_boot_state *s;
	const struct fw_fb_dt_node *n;
	const uint8_t *hdr, *blob;
	uint64_t dtb;
	uint32_t total;
	char tok[96];
	unsigned int i;
	int rc;

	step_at("1 dt-fb");
	if (!rpi5_boot_valid()) {
		printk("u3 dt-fb: skip (no boot record: x0 was not kept by boot_x0.S)\n");
		return;
	}
	s = rpi5_boot_state();
	dtb = s->x[0];
	printk("rpi5_display: x0 0x%llx, EL%llu, entry 0x%llx\n", (unsigned long long)dtb,
	       (unsigned long long)(s->current_el >> 2), (unsigned long long)s->entry_pc);
	if (dtb == 0 || (dtb & 7ull) != 0) {
		printk("u3 dt-fb: skip (x0 0x%llx is not a DTB address)\n", (unsigned long long)dtb);
		return;
	}
	if (dtb >= CONFIG_SRAM_BASE_ADDRESS && dtb <= sram_last) {
		printk("rpi5_display: the DTB address lies inside this image's RAM, so Zephyr may have "
		       "written over it; the walker below refuses a blob that no longer holds together\n");
	}
	hdr = map_dtb(dtb, 40u);
	if (hdr == NULL) {
		printk("u3 dt-fb: skip (the DTB header page at 0x%llx is not mapped)\n",
		       (unsigned long long)dtb);
		return;
	}
	if (dtb_be32(hdr) != 0xd00dfeedu) {
		printk("u3 dt-fb: skip (no FDT magic at x0: 0x%08x)\n", dtb_be32(hdr));
		return;
	}
	total = dtb_be32(hdr + 4);
	if (total < 40u || total > DTB_MAX) {
		printk("u3 dt-fb: skip (FDT totalsize %u is refused)\n", total);
		return;
	}
	blob = map_dtb(dtb, total);
	if (blob == NULL) {
		printk("u3 dt-fb: skip (the %u-byte DTB at 0x%llx is not mapped)\n", total,
		       (unsigned long long)dtb);
		return;
	}
	(void)memset(&r, 0, sizeof(r));
	rc = fw_fb_dt_parse(blob, total, &r);
	printk("rpi5_display: DTB at 0x%llx, %u bytes: parse rc %d, %u simple-framebuffer node(s), %u "
	       "reserved-memory region(s)\n", (unsigned long long)dtb, total, rc, r.count, r.nresv);
	for (i = 0; i < r.count && i < FW_FB_DT_MAX_NODES; i++) {
		n = &r.node[i];
		printk("rpi5_display: dt-fb node %s: status %s, reg %s 0x%llx size 0x%llx, %ux%u stride %u "
		       "format %s, %s\n", n->path, n->status_okay ? "okay" : "disabled",
		       n->have_reg ? "set" : "missing", (unsigned long long)n->addr,
		       (unsigned long long)n->size, n->width, n->height, n->stride,
		       n->have_format ? n->format : "none",
		       n->in_reserved ? "inside a /reserved-memory region" : "in no /reserved-memory region");
		if (n->in_reserved) {
			printk("rpi5_display: dt-fb node %u's reserved region 0x%llx size 0x%llx\n", i,
			       (unsigned long long)n->resv_addr, (unsigned long long)n->resv_size);
		}
	}
	if (rc != FDT_OK) {
		printk("u3 dt-fb: none (the blob is refused by the walker: %d)\n", rc);
		return;
	}
	fw_fb_dt_verdict(&r, tok, sizeof(tok));
	n = fw_fb_dt_pick(&r);
	printk("u3 dt-fb: %s%s%s\n", strncmp(tok, "dt-fb ", 6) == 0 ? tok + 6 : tok, n ? " " : "",
	       n ? fw_fb_dt_addr_token(n) : "");
}

/* ---- step 3: fb_query's question, before the allocation ------------------------ */

/* The six GET tags sent as fb_query.h builds them and classified by its parser:
 * the `u3 fb-query:` line. rpi_mbox_request_timed() adds the header and the
 * end tag and returns the buffer's status as its rc, so the header word is
 * put back from the rc before fbq_parse reads the answer. */
static void step_fbquery(void)
{
	uint32_t w[FBQ_QUERY_WORDS];
	uint32_t n = fbq_build_query(w);
	struct rpi_mbox_timing tm;
	struct fbq_result r;
	int rc;

	(void)memset(&tm, 0, sizeof(tm));
	rc = rpi_mbox_request_timed(&w[2], n - 3u, &tm);
	if (rc != 0 && rc != -EIO) {
		printk("u3 fb-query: skip (the query was not answered: %d)\n", rc);
		return;
	}
	w[1] = rc == 0 ? FBQ_STATUS_SUCCESS : FBQ_STATUS_SUCCESS | 1u;
	r = fbq_parse(w, n);
	printk("rpi5_display: fb-query rc %d, seen 0x%02x answered 0x%02x, %llu cycles waiting\n", rc,
	       r.seen, r.answered, (unsigned long long)tm.poll);
	printk("u3 fb-query: %s (phys %ux%u virt %ux%u depth %u order %u pitch %u offset %u,%u)\n",
	       fbq_class_name(r.cls), r.phys_w, r.phys_h, r.virt_w, r.virt_h, r.depth, r.order, r.pitch,
	       r.voff_x, r.voff_y);
}

/* ---- Z0 -------------------------------------------------------------------- */

static void z0_throttled(void)
{
	uint32_t val = 0;
	struct thr_flags f;
	char line[128];
	int rc = rpi_mbox_tag(THR_TAG_GET_THROTTLED, &val, 1);

	(void)memset(&f, 0, sizeof(f));
	if (rc != 0) {
		printk("rpi5_display: get_throttled was not answered (%d); weak supply is not ruled "
		       "out\n", rc);
		return;
	}
	thr_decode(val, &f);
	(void)thr_format(line, sizeof(line), &f);
	printk("%s\n", line);
}

/* The clocks Z0 names. `disp_zero` and `hsm_zero` are set only for a clock the
 * firmware answered with 0: an unanswered clock stops nothing. `hsm_live` says
 * the opposite of `hsm_zero` for the HDMI HSM clock: the firmware answered its
 * rate and it is not 0 — the only state that makes an `hd` register read known
 * to be safe (the design note's rule, section 8). */
static void z0_clocks(bool *disp_zero, bool *hsm_zero, bool *hsm_live)
{
	static const uint32_t ids[] = { YD5_CLK_CORE, YD5_CLK_DISP, YD5_CLK_M2MC, YD5_CLK_PIXEL_BVB,
					RPI_FW_CLK_V3D };
	struct yd5_fwclk_info info[ARRAY_SIZE(ids)];
	char buf[ARRAY_SIZE(ids) * 96];
	size_t i;

	*disp_zero = false;
	*hsm_zero = false;
	*hsm_live = false;
	for (i = 0; i < ARRAY_SIZE(ids); i++) {
		bool on = false, exists = false;
		uint32_t rate = 0, meas = 0;
		int rc_state = rpi_clock_state_get(ids[i], &on, &exists);
		int rc_rate = rpi_clock_rate(ids[i], &rate);
		int rc_meas = rpi_clock_measured_rate(ids[i], &meas);

		info[i].id = ids[i];
		info[i].present = rc_state == 0 && rc_rate == 0 && rc_meas == 0 && exists;
		info[i].on = info[i].present ? (on ? 1 : 0) : 0;
		info[i].rate = rate;
		info[i].measured = meas;
		info[i].missing_resp = rc_state != 0 || rc_rate != 0 || rc_meas != 0;
		printk("rpi5_display: clk %s id %u: rc state %d rate %d measured %d, exists %d\n",
		       yd5_fwclk_name(ids[i]), ids[i], rc_state, rc_rate, rc_meas, exists ? 1 : 0);
		if (rc_rate == 0 && rate == 0) {
			if (ids[i] == YD5_CLK_DISP) {
				*disp_zero = true;
			}
			if (ids[i] == YD5_CLK_M2MC) {
				*hsm_zero = true;
			}
		} else if (rc_rate == 0 && rate != 0 && ids[i] == YD5_CLK_M2MC) {
			*hsm_live = true;
		}
	}
	(void)yd5_fwclk_format(buf, sizeof(buf), info, ARRAY_SIZE(ids));
	printk("%s", buf);
}

/* iommu4 as the firmware left it: its words printed as this probe's own record
 * (snapshot.h's line format names only its three blocks), then plane_match.h's
 * class of what those words mean for an address out of a display list. */
static void z0_iommu(struct pm_mmu *mmu)
{
	uint32_t ctrl = ALL_ONES, cap = ALL_ONES, start = ALL_ONES, end = ALL_ONES, dbg = ALL_ONES;

	if (iommu_on) {
		ctrl = sys_read32(va_iommu + IO_MMMU_CTRL);
		cap = sys_read32(va_iommu + IO_ADDR_CAP);
		start = sys_read32(va_iommu + IO_BYPASS_START);
		end = sys_read32(va_iommu + IO_BYPASS_END);
		dbg = sys_read32(va_iommu + IO_DEBUG_INFO);
	}
	printk("rpi5_display: iommu4 mmmu_ctrl +0x%02x=0x%08x, addr_cap +0x%02x=0x%08x, bypass_start "
	       "+0x%02x=0x%08x\n", IO_MMMU_CTRL, ctrl, IO_ADDR_CAP, cap, IO_BYPASS_START, start);
	printk("rpi5_display: iommu4 bypass_end +0x%02x=0x%08x, debug_info +0x%02x=0x%08x\n",
	       IO_BYPASS_END, end, IO_DEBUG_INFO, dbg);
	pm_mmu_state(ctrl, dbg, end, iommu_on, mmu);
	printk("rpi5_display: iommu4 read=%d enabled=%d has_bypass=%d unit_4m=%d pass_below=0x%llx, named "
	       "from the words above\n", mmu->read ? 1 : 0, mmu->enabled ? 1 : 0, mmu->has_bypass ? 1 : 0,
	       mmu->bypass_4m ? 1 : 0, (unsigned long long)mmu->pass_below);
	if (!mmu->enabled) {
		printk("rpi5_display: iommu4's enable bit is clear: a disabled MMU leaves every address "
		       "unchanged on its way to the HVS, so a list address is an ARM physical address (the "
		       "meaning of its enable bit, rpi:drivers/iommu/bcm2712-iommu.c:43,226-232; plane_match.h "
		       "names the inference and uses it)\n");
	} else {
		printk("rpi5_display: addresses below 0x%llx reach the HVS unchanged; at or above it an "
		       "address a list names may be an IOVA (rpi:drivers/iommu/bcm2712-iommu.c:98-99,184-187)\n",
		       (unsigned long long)mmu->pass_below);
	}
}

/* ---- Z1 -------------------------------------------------------------------- */

/* One channel's seven words, read through the table's cited offsets (0x30, 0x34,
 * 0x38, 0x3c, 0x40, 0x44, 0x48 plus 0x20 per channel for the C group; 0x100,
 * 0x104, 0x108, 0x110, 0x114, 0x118, 0x11c plus 0x40 for the D group). */
struct chan_words {
	uint32_t ctrl0, ctrl1, bgnd, lptrs, cob, status, dl;
};

static void chan_words_read(unsigned int ch, int d0, struct chan_words *w)
{
	static const char *const regs[7] = { "ctrl0", "ctrl1", "bgnd", "lptrs", "cob", "status", "dl" };
	uint32_t *const out[7] = { &w->ctrl0, &w->ctrl1, &w->bgnd, &w->lptrs, &w->cob,
				   &w->status, &w->dl };
	char name[24];
	size_t i;

	for (i = 0; i < ARRAY_SIZE(regs); i++) {
		const struct snap_reg *r;

		(void)snprintf(name, sizeof(name), "%s_disp%u_%s", d0 ? "d" : "c", ch, regs[i]);
		r = snap_reg(name);
		if (r == NULL || r->block != SNAP_HVS) {
			*out[i] = ALL_ONES;
			continue;
		}
		*out[i] = hvs_on ? sys_read32(va_hvs + r->off) : ALL_ONES;
	}
}

/* One sample of a channel, as scanout_alive.h takes it, with `valve` (0 or 1) naming
 * which pixel valve's words it carries. BCM2712 names two pixel valves
 * (rpi:bcm2712.dtsi:292-296 pixelvalve0 at 0x7c410000, :298-302 pixelvalve1 at
 * 0x7c411000, the only two pixelvalve nodes of the dtsi) and the HVS has three
 * channels (hvs's interrupt-names ch0/ch1/ch2-eof, rpi:bcm2712.dtsi:612-623), and
 * no register read here says which valve a channel feeds: that link is a DISP1
 * target field, and snapshot.h's table does not hold it. So every mapped valve is
 * read and chan_verdict below chooses between the two judgements; an unmapped
 * valve's words stay all ones, which is what sa_all_ones calls unclocked, so
 * chan_verdict is never fed such a judgement. */
static void chan_sample(uint32_t version, const struct chan_words *w, struct sa_sample *s,
			unsigned int valve)
{
	const struct snap_reg *pc = snap_reg("pv_control");
	const struct snap_reg *vc = snap_reg("pv_v_control");
	bool on = (valve == 0u) ? pv_on : pv1_on;
	mm_reg_t base = (valve == 0u) ? va_pv0 : va_pv1;

	s->hvs_version = version;
	s->ctrl0 = w->ctrl0;
	s->status = w->status;
	s->dl = w->dl;
	s->pv_control = (on && pc != NULL) ? sys_read32(base + pc->off) : ALL_ONES;
	s->pv_vcontrol = (on && vc != NULL) ? sys_read32(base + vc->off) : ALL_ONES;
}

/* The words and both valves' words of one channel, read at one instant: the two
 * samples a channel is judged from are made with sample_at() twice, `SAMPLE_GAP_MS`
 * apart, so all three channels share one time base. */
static void sample_at(uint32_t version, unsigned int ch, int d0, struct chan_words *w,
		      struct sa_sample s[2])
{
	chan_words_read(ch, d0, w);
	chan_sample(version, w, &s[0], 0u);
	chan_sample(version, w, &s[1], 1u);
}

/* Of the judgements made for one channel, one per mapped valve (indexed by valve
 * number; `mapped` says which slots are judgements at all), the one this image
 * names on the verdict line:
 *   a running one when any valve's words leave the channel running,
 *   else a reason that does not blame a valve (channel off, unknown stepping,
 *     frozen counters, or a read that came back all ones) — those come from the
 *     channel's own words and are the same for both judgements or contradicted by
 *     the other valve,
 *   else "pixel valve off", which then means both valves this SoC names.
 * No channel is claimed for a valve: the record lines say whose words each
 * judgement used. */
static struct sa_verdict chan_verdict(const struct sa_verdict *vv, const bool mapped[2],
				      unsigned int *valve_named)
{
	unsigned int pick = 0;
	unsigned int k;

	for (k = 1; k < 2; k++) {
		if (!mapped[k]) {
			continue;
		}
		if (vv[k].running) {
			pick = k;
			break;
		}
		if (!mapped[pick] || (vv[pick].reason == SA_PV_OFF && vv[k].reason != SA_PV_OFF)) {
			pick = k;
		}
	}
	*valve_named = pick;
	return vv[pick];
}

static void chan_record(unsigned int ch, const struct chan_words *b, const struct sa_verdict *v)
{
	struct hvs6_chan c;

	hvs6_chan_decode(b->ctrl0, b->lptrs, b->status, b->dl, &c);
	printk("rpi5_display: ch%u ctrl0 0x%08x ctrl1 0x%08x bgnd 0x%08x lptrs 0x%08x cob 0x%08x status "
	       "0x%08x dl 0x%08x\n", ch, b->ctrl0, b->ctrl1, b->bgnd, b->lptrs, b->cob, b->status, b->dl);
	printk("rpi5_display: ch%u enabled=%d running=%d empty=%d mode=%u fwidth=%u lines=%u head=%u "
	       "active=%u; frame count %u then %u, line %u then %u, %u ms apart\n", ch, c.enabled ? 1 : 0,
	       c.running ? 1 : 0, c.empty ? 1 : 0, c.mode, c.fwidth, c.lines, c.head, c.active, v->frcnt_a,
	       v->frcnt_b, v->yline_a, v->yline_b, SAMPLE_GAP_MS);
}

/* One channel judged with one pixel valve's words. Z0 already printed the mapping
 * of each valve, so the words here are the second half of that record. */
static void chan_valve_record(unsigned int ch, unsigned int valve, const struct sa_sample *s,
			      const struct sa_verdict *v)
{
	struct pv_control pc = pv_decode_control(s->pv_control);
	struct pv_vcontrol vc = pv_decode_vcontrol(s->pv_vcontrol);

	printk("rpi5_display: ch%u judged with pixelvalve%u: CONTROL 0x%08x (EN=%d clk_select=%u) "
	       "V_CONTROL 0x%08x (VIDEN=%d) -> %s\n", ch, valve, s->pv_control, pc.enabled ? 1 : 0,
	       pc.clk_select, s->pv_vcontrol, vc.viden ? 1 : 0,
	       v->running ? "running" : sa_reason_text(v->reason));
}

/* The list a channel runs: DL.LACT, or LPTRS.HEADE when LACT is 0 (a list
 * installed but not yet started). Both are words of the table, decoded by
 * hvs6_dlist.h's field masks. */
static uint32_t chan_head(unsigned int ch, const struct chan_words *w)
{
	struct hvs6_chan c;

	hvs6_chan_decode(w->ctrl0, w->lptrs, w->status, w->dl, &c);
	if (c.active != 0) {
		return c.active;
	}
	printk("rpi5_display: ch%u DL.LACT is 0, so its list head LPTRS.HEADE %u is read instead\n", ch,
	       c.head);
	return c.head;
}

/* Read channel ch's list from word `start`, print each word as it is read, and
 * decode the first plane out of those same captured words. */
static bool read_plane(unsigned int ch, uint32_t start, struct hvs6_plane *out)
{
	int err;

	dlist_at = start;
	dlist_n = 0;
	dlist_capture = true;
	(void)snapshot_dlist(&snap_io, &snap_emit, ch, start, SNAP_END_WORD_BIT31);
	dlist_capture = false;
	if (dlist_n == 0) {
		return false;
	}
	err = hvs6_dlist_first_plane(dlist, dlist_n, 0, out);
	if (err != HVS6_OK) {
		printk("rpi5_display: ch%u's list is not a single linear plane here: hvs6_dlist_first_plane "
		       "said %d over the %u words printed above\n", ch, err, (unsigned int)dlist_n);
		return false;
	}
	return true;
}

/* Channel ch's list read again and its first plane compared with `was`, the
 * plane the adopt read-back wrote into: the words for the adopt verdict's
 * "list ..." clause. The new list words are printed as `u3 dlist` lines like
 * every other read. */
static const char *adopt_list_still_names(unsigned int ch, int d0, const struct hvs6_plane *was)
{
	static char txt[96];
	struct chan_words w;
	struct hvs6_plane p;
	uint32_t head;

	chan_words_read(ch, d0, &w);
	head = chan_head(ch, &w);
	if (head == 0) {
		return "named by no list any more (DL.LACT and LPTRS.HEADE are 0)";
	}
	printk("rpi5_display: ch%u's list read again after the adopt read-back, from word %u\n", ch,
	       head);
	if (!read_plane(ch, head, &p)) {
		return "not decodable when read again";
	}
	if (p.addr == was->addr && p.pitch == was->pitch) {
		return "still names it";
	}
	(void)snprintf(txt, sizeof(txt), "names another plane now (0x%llx pitch %u)",
		       (unsigned long long)p.addr, p.pitch);
	return txt;
}

/* The plane's format as hvs6_fmt.h names it (the DRM-style name, so both sides of
 * a comparison are named in the same words), or the code alone when the table
 * does not list that order. */
static const char *plane_fmt(const struct hvs6_plane *p)
{
	const struct hvs6_fmt *f = hvs6_fmt_lookup(p->format_code, p->order);
	const char *nm = (f != NULL) ? f->name : hvs6_format_name(p->format_code);

	return (nm != NULL) ? nm : "unknown";
}

/* pm_result's own words, for a per-channel record line that must not be the `u3`
 * line itself: that line appears once in the log and verdicts.sh reads the first
 * one it finds. plane_match.h's reason already names what it found ("address
 * differs", "no plane in the display list", "not comparable: ..."), so the reason
 * is used when there is one and the verdict word otherwise. */
static const char *pm_words(const struct pm_result *r)
{
	static const char *const w[2] = { "no", "not comparable" };

	if (r->verdict == PM_YES) {
		return "the list names the buffer";
	}
	if (r->reason != NULL && r->reason[0] != '\0') {
		return r->reason;
	}
	return w[r->verdict == PM_NO ? 0 : 1];
}

/* The mailbox's pixel-order tag named as the same kind of name (pattern.h's and
 * hvs6_fmt.h's words: 0 is B first in memory, which hvs6_fmt.h calls XRGB8888). */
static const char *fb_fmt_name(uint32_t order)
{
	switch (order) {
	case RPI5_ADOPT_ORDER_BGR: return "XRGB8888";
	case RPI5_ADOPT_ORDER_RGB: return "XBGR8888";
	default: return "RGB565";
	}
}

/* The matching pattern layout for a 32 bpp buffer's memory order. */
static enum pattern_fmt fmt_of_order(uint32_t order)
{
	switch (order) {
	case RPI5_ADOPT_ORDER_BGR: return PATTERN_XRGB8888; /* B, G, R, X in memory */
	case RPI5_ADOPT_ORDER_RGB: return PATTERN_XBGR8888; /* R, G, B, X in memory */
	default: return PATTERN_RGB565;
	}
}

/* The plane's format named as adopt.h names a format. */
static enum rpi5_adopt_format adopt_fmt_of(const char *name)
{
	if (strcmp(name, "XRGB8888") == 0) {
		return RPI5_ADOPT_FMT_XRGB8888;
	}
	if (strcmp(name, "ARGB8888") == 0) {
		return RPI5_ADOPT_FMT_ARGB8888;
	}
	if (strcmp(name, "XBGR8888") == 0) {
		return RPI5_ADOPT_FMT_XBGR8888;
	}
	if (strcmp(name, "ABGR8888") == 0) {
		return RPI5_ADOPT_FMT_ABGR8888;
	}
	if (strcmp(name, "RGB565") == 0) {
		return RPI5_ADOPT_FMT_RGB565;
	}
	return RPI5_ADOPT_FMT_UNKNOWN;
}

/* adopt.h's translation, filled with bus_addr.h's. adopt.h's rule is a truthy
 * return for a translatable address and 0 for one that is not (adopt.h:48-49). */
static int bus2arm(void *ctx, uint64_t bus, uint64_t *arm)
{
	(void)ctx;
	return bcm2712_bus_to_arm(bus, arm) == 0 ? 1 : 0;
}

/* How many pixels of a rect differ from the rect's own first pixel, and what that
 * first pixel is (pattern.h's reader, so the bytes are taken in the buffer's own
 * format). This is the measurement that tells a blank console from a drawn one:
 * fbcon_attach() clears the rows it was given, which already changes every pixel
 * of a band that held the pattern, so "the band no longer holds the pattern" is
 * not evidence that a character reached the screen. A band that is one flat colour
 * after the attach and not one after the text is, and nothing here depends on what
 * colours fbcon.c uses (they are its own, not exported). */
static uint32_t rect_changed_from_first(const uint8_t *fb, uint32_t width, uint32_t height,
					 uint32_t pitch, uint32_t bpp_bits, struct pattern_rect r,
					 uint32_t *first)
{
	uint32_t bp = bpp_bits / 8u;
	uint32_t n = 0;
	uint32_t diff = 0;
	uint32_t x, y;

	*first = 0;
	for (y = r.y; y < r.y + r.h && y < height; y++) {
		for (x = r.x; x < r.x + r.w && x < width; x++) {
			uint32_t v = pattern_load(fb + (size_t)y * pitch + (size_t)x * bp, bp);

			if (n == 0u) {
				*first = v;
			} else if (v != *first) {
				diff++;
			}
			n++;
		}
	}
	return diff;
}

int main(void)
{
	struct chan_words wa[SA_NUM_CHANNELS], wb[SA_NUM_CHANNELS];
	struct sa_sample sa[SA_NUM_CHANNELS][2], sb[SA_NUM_CHANNELS][2];
	struct sa_verdict v[SA_NUM_CHANNELS];
	struct sa_verdict vv[SA_NUM_CHANNELS][2]; /* one judgement per pixel valve */
	bool valve_mapped[2] = { false, false }; /* filled with the mapping, before any judgement */
	unsigned int valve_used[SA_NUM_CHANNELS]; /* which valve's words v[] named */
	bool scan_running = false; /* v[scan_ch].running, once Z1 judged the channels */
	struct pm_mmu mmu;
	struct rpi5_fw_fb q;
	struct rpi_fb fb;
	struct screen adopt_scr = { NULL, 0, 0, 0, 0, NULL };
	struct screen scr = { NULL, 0, 0, 0, 0, NULL };
	uint32_t got[4] = { 0, 0, 0, 0 };
	bool disp_zero = false, hsm_zero = false, hsm_live = false;
	bool mailbox_ok = false, fb_readback = false, split_ok = false;
	bool plane_found = false, have_pp = false, have_buf = false, stepping_ok = false;
	struct pm_plane pp;
	struct pm_buffer buf;
	struct pm_result res;
	const char *follow_state = "no";
	uint32_t version = ALL_ONES;
	unsigned int scan_ch = 0;  /* the channel `u3 scanout:` judged */
	unsigned int plane_ch = 0; /* the channel `u3 plane:` names */
	int d0 = 0;
	int mbox_rc;
	int mbox_init_rc; /* rpi_mbox_init()'s: mbox_rc is the allocation's from step 3 on */
	const uint64_t sram_last = CONFIG_SRAM_BASE_ADDRESS + (uint64_t)CONFIG_SRAM_SIZE * 1024 - 1;
	char line[256];
	char plane_txt[160];
	const char *ad_state = "FAIL", *ad_why = "not reached";
	const char *fb_state = "FAIL", *fb_why = "not reached";
	const char *rb_state = "FAIL", *rb_why = "not reached";
	const char *split_state = "not drawn";
	struct hvs6_plane plane;

	(void)memset(&mmu, 0, sizeof(mmu));
	(void)memset(&q, 0, sizeof(q));
	(void)memset(&fb, 0, sizeof(fb));
	(void)memset(&plane, 0, sizeof(plane));
	(void)memset(&pp, 0, sizeof(pp));
	(void)memset(&buf, 0, sizeof(buf));
	(void)memset(&res, 0, sizeof(res));
	(void)snprintf(plane_txt, sizeof(plane_txt), "none");

	printk("rpi5_display: build %s\n", RPI5_DISPLAY_BUILD);
	/* The board layer armed the watchdog in its first PRE_KERNEL_1 hook,
	 * before main; this line names when the reset comes if feeding stops. */
	rpi_wdog_announce();
	printk("rpi5_display: memory: this image 0x%llx..0x%llx, dram 0x%llx..0x%llx (bus_addr.h)\n",
	       (unsigned long long)CONFIG_SRAM_BASE_ADDRESS, (unsigned long long)sram_last,
	       (unsigned long long)BCM2712_DRAM_BASE,
	       (unsigned long long)(BCM2712_DRAM_BASE + BCM2712_DRAM_SIZE - 1ull));

	/* ---- step 1: what the firmware's DTB says about a framebuffer ---- */
	step_dtfb(sram_last);

	/* ---- step 2: the running scanout, read-only (Z0, Z1) ---- */
	step_at("2 scanout");
	mbox_rc = rpi_mbox_init();
	printk("rpi5_display: mailbox init %d\n", mbox_rc);
	mbox_init_rc = mbox_rc;

	/* ---- Z0 ---- */
	if (mbox_rc == 0) {
		z0_throttled();
		z0_clocks(&disp_zero, &hsm_zero, &hsm_live);
	} else {
		printk("rpi5_display: Z0 asked no clocks: the mailbox did not answer (%d)\n", mbox_rc);
	}

	/* The hd region is read only on an answer that says clock 13 runs. The stall
	 * note covers a read with that clock at 0; a read whose clock state was never
	 * answered is the same unknown, so it is refused too, by name. */
	hd_on = hsm_live && map_block(&va_hd, HDMI0_HD_BASE, HDMI0_HD_SIZE, "hdmi0 hd");
	if (!hd_on) {
		const char *why = hsm_zero ? "it was answered 0" : "its rate was never answered";

		printk("rpi5_display: the hdmi0 hd region is NOT read: the hdmi clock (HSM, clock id %u) %s, and "
		       "an HDMI register read with that clock at 0 or unknown is the documented silent CPU stall "
		       "(rpi:vc4_hdmi.c:3251-3262)\n", YD5_CLK_M2MC, why);
	}
	pv_on = map_block(&va_pv0, PV0_BASE, PV0_SIZE, "pixelvalve0");
	pv1_on = map_block(&va_pv1, PV1_BASE, PV1_SIZE, "pixelvalve1");
	valve_mapped[0] = pv_on;
	valve_mapped[1] = pv1_on;
	if (pv1_on) {
		uint32_t c1 = sys_read32(va_pv1 + PV_CONTROL);
		uint32_t v1 = sys_read32(va_pv1 + PV_V_CONTROL);

		printk("rpi5_display: pixelvalve1 CONTROL 0x%08x (EN=%d) V_CONTROL 0x%08x (VIDEN=%d)\n", c1,
		       pv_decode_control(c1).enabled ? 1 : 0, v1, pv_decode_vcontrol(v1).viden ? 1 : 0);
	}
	iommu_on = map_block(&va_iommu, IOMMU4_BASE, IOMMU4_SIZE, "iommu4");
	if (iommu_on) {
		z0_iommu(&mmu);
	} else {
		pm_mmu_state(0, 0, 0, false, &mmu);
		printk("rpi5_display: iommu4 was not read, so nothing is known about it: a list address may "
		       "be an IOVA and an address comparison is not sound\n");
	}

	if (disp_zero) {
		printk("rpi5_display: the HVS is NOT read: the display clock (clock id %u) was answered 0. "
		       "Whether HVS registers stall the way the HDMI's do with a clock at 0 is stated nowhere "
		       "read, so a 0 is treated as \"do not read the HVS\" (driver/docs/rpi5-display.md "
		       "section 8)\n", YD5_CLK_DISP);
	} else {
		hvs_on = map_block(&va_hvs, HVS_BASE, HVS_SIZE, "hvs");
	}

	if (hvs_on) {
		(void)snapshot_take(&snap_io, snap_words);
		(void)snapshot_print(snap_words, &snap_emit);
		version = snap_word("version");
		d0 = sa_is_d0(version);
		/* The channel offsets belong to one stepping (the C group at 0x30.. with stride 0x20, the D
		 * group at 0x100.. with stride 0x40): read for the other one they are different
		 * registers, so nothing below a named stepping is read as a channel. */
		stepping_ok = (version & SA_HVS_VERSION_MASK) == SA_HVS_VERSION_C0 ||
			      (version & SA_HVS_VERSION_MASK) == SA_HVS_VERSION_D0;
		printk("rpi5_display: hvs version 0x%08x (mask 0x%02x) cxm_size 0x%08x control 0x%08x, read "
		       "as stepping %s\n", version, version & SA_HVS_VERSION_MASK, snap_word("cxm_size"),
		       snap_word("control"),
		       d0 ? "D (SCALER6D)" : ((version & SA_HVS_VERSION_MASK) == SA_HVS_VERSION_C0 ? "C (SCALER6)"
												 : "unknown"));
		if (pv_on) {
			struct pv_timing t = pv_decode_timing(snap_word("pv_horza"), snap_word("pv_horzb"),
							      snap_word("pv_verta"), snap_word("pv_vertb"));

			printk("rpi5_display: pixelvalve0 hactive %u htotal %u vactive %u vtotal %u: %s (the "
			       "offsets are the vc4 pixel valve layout; its BCM2712 use is unverified, as "
			       "rpi5_scanout/snapshot.h says)\n", t.hactive, t.htotal, t.vactive, t.vtotal,
			       pv_timing_valid(&t) ? "a timing is set" : "no timing set (it reads as zero)");
		}
		if (hd_on) {
			printk("rpi5_display: hd VID_CTL hdmi0 0x%08x enabled=%d, hdmi1 0x%08x enabled=%d (the "
			       "two connectors share this block and their register tables differ: "
			       "rpi:vc4_hdmi_regs.h:438 and :538)\n", snap_word("hdmi_vid_ctl"),
			       hd_vid_ctl_enabled(snap_word("hdmi_vid_ctl")) ? 1 : 0, snap_word("hdmi1_vid_ctl"),
			       hd_vid_ctl_enabled(snap_word("hdmi1_vid_ctl")) ? 1 : 0);
		}
	}

	/* ---- Z1: the channels, each sampled twice and judged once per valve ---- */
	if (hvs_on && stepping_ok && (pv_on || pv1_on)) {
		struct hvs6_plane first_plane;
		bool have_first = false;
		unsigned int first_ch = 0;
		unsigned int i, k;

		(void)memset(&first_plane, 0, sizeof(first_plane));
		printk("rpi5_display: each channel is judged twice, once with each mapped pixel valve's words. "
		       "BCM2712 names two pixel valves (rpi:bcm2712.dtsi:292-302) and the HVS three channels "
		       "(hvs's interrupt-names ch0/ch1/ch2-eof at :612-623), and no register read here says "
		       "which valve a channel feeds: that link is a DISP1 target field, not in snapshot.h's "
		       "table\n");

		/* A judgement per mapped valve; an unmapped valve's slot stays unused and chan_verdict
		 * skips it (its words would read as all ones, which is a read, not a valve). */
		(void)memset(vv, 0, sizeof(vv));
		for (i = 0; i < SA_NUM_CHANNELS; i++) {
			sample_at(version, i, d0, &wa[i], sa[i]);
		}
		k_msleep(SAMPLE_GAP_MS);
		for (i = 0; i < SA_NUM_CHANNELS; i++) {
			sample_at(version, i, d0, &wb[i], sb[i]);
			for (k = 0; k < 2u; k++) {
				if (valve_mapped[k]) {
					vv[i][k] = sa_judge(&sa[i][k], &sb[i][k], i);
				}
			}
			v[i] = chan_verdict(vv[i], valve_mapped, &valve_used[i]);
		}
		for (i = 1; i < SA_NUM_CHANNELS; i++) {
			if (v[i].running && !v[scan_ch].running) {
				scan_ch = i;
			}
		}
		scan_running = v[scan_ch].running != 0; /* pm_choose's `preferred` in Z3 */
		(void)sa_format(&v[scan_ch], line, sizeof(line));
		printk("%s\n", line); /* the `u3 scanout:` line, in scanout_alive.h's words */
		for (i = 0; i < SA_NUM_CHANNELS; i++) {
			chan_record(i, &wb[i], &v[i]);
			for (k = 0; k < 2u; k++) {
				if (!valve_mapped[k]) {
					printk("rpi5_display: ch%u judged with pixelvalve%u: the block was not "
					       "mapped, so its words were not read\n", i, (unsigned int)k);
					continue;
				}
				chan_valve_record(i, (unsigned int)k, &sb[i][k], &vv[i][k]);
			}
			printk("rpi5_display: ch%u is the judgement of pixelvalve%u's words (chan_verdict's "
			       "rule)\n", i, valve_used[i]);
		}

		/* Every channel that names a list is read and decoded. */
		for (i = 0; i < SA_NUM_CHANNELS; i++) {
			struct hvs6_plane p;
			uint32_t head = chan_head(i, &wb[i]);

			if (head == 0) {
				printk("rpi5_display: ch%u names no list: DL.LACT and LPTRS.HEADE are both "
				       "0\n", i);
				continue;
			}
			printk("rpi5_display: ch%u list read from word %u\n", i, head);
			if (!read_plane(i, head, &p)) {
				continue;
			}
			printk("rpi5_display: ch%u plane: address 0x%llx (list words, before the bus-to-ARM "
			       "step), screen position %u,%u, %ux%u, pitch %u, format code %u order %u (%s), "
			       "unity=%d, %u words\n", i, (unsigned long long)p.addr, p.x, p.y, p.width,
			       p.height, p.pitch, p.format_code, p.order, plane_fmt(&p), p.unity ? 1 : 0,
			       p.nwords);
			if (i == scan_ch) {
				plane = p;
				plane_ch = i;
				plane_found = true;
			} else if (!have_first) {
				first_plane = p;
				first_ch = i;
				have_first = true;
			}
		}
		if (!plane_found && have_first) {
			plane = first_plane;
			plane_ch = first_ch;
			plane_found = true;
			printk("rpi5_display: ch%u names no decodable plane, so ch%u's plane is the one "
			       "adopt and the list comparison use\n", scan_ch, first_ch);
		}

		if (plane_found) {
			uint64_t arm = plane.addr;
			const char *fmt = plane_fmt(&plane);

			if (bcm2712_bus_to_arm(plane.addr, &arm) != 0) {
				printk("rpi5_display: the list address 0x%llx is outside what bus_addr.h "
				       "translates (BCM2712_DISPLAY_ADDR_BITS %u); the number is printed as "
				       "the list holds it\n", (unsigned long long)plane.addr,
				       (unsigned int)BCM2712_DISPLAY_ADDR_BITS);
				arm = plane.addr;
			}
			(void)snprintf(plane_txt, sizeof(plane_txt), "addr=0x%llx fmt=%s pitch=%u w=%u h=%u",
				       (unsigned long long)arm, fmt, plane.pitch, plane.width, plane.height);
		}
	} else {
		const char *why = "the HVS was not read, so no channel was sampled";

		if (hvs_on && !pv_on && !pv1_on) {
			why = "neither pixel valve was mapped, so no valve's EN and VIDEN are known and "
			      "scanout_alive.h's rule has nothing to judge";
		} else if (hvs_on) {
			why = "the version word names neither stepping (0x53 C or 0x54 D), so every channel "
			      "offset would be another generation's register";
		}
		printk("u3 scanout: stopped not measured (%s)\n", why);
	}
	printk("u3 plane: %s\n", plane_txt);

	/* Z1's adopt decision, and the plane's own read-back. */
	step_at("4 adopt-readback");
	if (plane_found) {
		struct rpi5_adopt_plane ap;
		struct rpi5_adopt_env env;
		struct rpi5_adopt_out out;
		struct rpi5_adopt_result adopt_res;
		/* static: the verdict line below names the reason these buffers hold, and it is printed
		 * after this block ends. */
		static char ad_buf[256];

		printk("rpi5_display: the plane adopt and the read-back below work on is ch%u's list%s\n",
		       plane_ch,
		       plane_ch == scan_ch ? " (the channel `u3 scanout:' judged)"
					   : ", chosen because the scanned channel names none");
		ap.bus_addr = plane.addr; /* as the list holds it: adopt.h translates it */
		ap.format = adopt_fmt_of(plane_fmt(&plane));
		ap.pitch = plane.pitch;
		ap.w = plane.width;
		ap.h = plane.height;
		env.dram_base = BCM2712_DRAM_BASE;
		env.dram_size = BCM2712_DRAM_SIZE;
		env.reserved_base = CONFIG_SRAM_BASE_ADDRESS; /* this image */
		env.reserved_size = (uint64_t)CONFIG_SRAM_SIZE * 1024ull;
		env.bus_to_arm = bus2arm;
		env.ctx = NULL;
		/* adopt.h writes `out` only for a plane it accepts (adopt.h:115-171), so the numbers are
		 * printed on that branch alone — a refusal names its reason and no numbers. */
		(void)memset(&out, 0, sizeof(out));
		adopt_res = rpi5_adopt_check(&ap, &env, &out);
		if (adopt_res.verdict != RPI5_ADOPT) {
			printk("rpi5_display: adopt refused: %s\n", adopt_res.reason);
			(void)snprintf(ad_buf, sizeof(ad_buf), "adopt refused the plane: %s", adopt_res.reason);
			ad_why = ad_buf;
		} else {
			printk("rpi5_display: adopt possible: %s (phys 0x%llx, %ux%u, pitch %u, bpp %u, order "
			       "%u, %llu bytes)\n", adopt_res.reason, (unsigned long long)out.phys, out.width,
			       out.height, out.pitch, out.bpp, out.order, (unsigned long long)out.size);
			if (!pm_addr_passes_through(&mmu, out.phys)) {
				/* The buffer is real but its address is one the IOMMU may translate, so reading
				 * that number would read the wrong bytes: named, not attempted. */
				ad_why = "not attempted: the adopted address is at or above iommu4's "
					 "pass-through window, so it may be an IOVA rather than an address this "
					 "image can map";
			} else {
				uint8_t *plane_buf = map_plane(out.phys, out.size);

				if (plane_buf == NULL) {
					(void)snprintf(ad_buf, sizeof(ad_buf),
						       "the adopted buffer at 0x%llx could not be mapped",
						       (unsigned long long)out.phys);
					ad_why = ad_buf;
				} else {
					struct pattern_report r1, r2;
					enum pattern_fmt pf = fmt_of_order(out.order);
					const char *still;
					int filled, checked, cleaned;

					(void)memset(&r1, 0, sizeof(r1));
					(void)memset(&r2, 0, sizeof(r2));
					/* out.bpp is bytes per pixel (adopt.h:77); pattern.h's bpp is in bits. */
					filled = pattern_fill(plane_buf, out.width, out.height, out.pitch,
								out.bpp * 8u, pf);
					cleaned = clean_range(plane_buf, out.size);
					checked = pattern_check(plane_buf, out.width, out.height, out.pitch,
								out.bpp * 8u, pf, 1u, &r1);
					k_msleep(SAMPLE_GAP_MS);
					(void)pattern_check(plane_buf, out.width, out.height, out.pitch,
								out.bpp * 8u, pf, 1u, &r2);
					printk("rpi5_display: plane pattern %s fill %d, clean %d, check %d: checksum "
					       "0x%08x then 0x%08x over %u pixels, %u then %u differing\n",
					       pattern_fmt_name(pf), filled, cleaned, checked, r1.checksum,
					       r2.checksum, (unsigned int)r2.checked, r1.mismatches,
					       r2.mismatches);
					if (r2.first_x != 0 || r2.first_y != 0) {
						printk("rpi5_display: the second read differs first at %u,%u: "
						       "written 0x%08x, read 0x%08x\n", r2.first_x, r2.first_y,
						       r2.expected, r2.got);
					}
					/* The list is read again after the bytes: does the scanout still
					 * fetch from the buffer the pattern went into? */
					still = adopt_list_still_names(plane_ch, d0, &plane);
					if (filled == 0 && checked == 0 && r1.mismatches == 0 &&
					    r2.mismatches == 0) {
						ad_state = "PASS";
						if (out.bpp == 4u) { /* fbcon.c draws 32 bpp only */
							adopt_scr.cpu = plane_buf;
							adopt_scr.width = out.width;
							adopt_scr.height = out.height;
							adopt_scr.pitch = out.pitch;
							adopt_scr.order = out.order;
							adopt_scr.what = "the adopted firmware plane";
						}
						(void)snprintf(ad_buf, sizeof(ad_buf),
							       "%u x %u read back byte-equal twice, %u ms apart, "
							       "0 mismatches, checksum 0x%08x, list %s",
							       out.width, out.height, SAMPLE_GAP_MS, r2.checksum,
							       still);
						ad_why = ad_buf;
					} else {
						(void)snprintf(ad_buf, sizeof(ad_buf),
							       "%u of %u pixels differ on the first read and %u "
							       "on the second (checksum 0x%08x then 0x%08x), "
							       "list %s",
							       r1.mismatches, (unsigned int)r1.checked,
							       r2.mismatches, r1.checksum, r2.checksum, still);
						ad_why = ad_buf;
					}
				}
			}
		}
	} else if (hvs_on && stepping_ok && (pv_on || pv1_on)) {
		ad_why = "not attempted: no single linear plane was decoded from the lists that were read";
	} else if (hvs_on) {
		ad_why = "not attempted: the channels were not sampled, so no list was read (neither pixel valve "
			 "was mapped, or the version word names neither stepping)";
	} else {
		ad_why = "not attempted: no display list was read (the HVS was not read)";
	}
	printk("u3 adopt-readback: %s (%s)\n", ad_state, ad_why);

	/* ---- Z2: the firmware's own framebuffer ---- */
	step_at("3 mailbox");
	if (mbox_rc != 0) {
		printk("u3 fb-query: skip (the mailbox did not answer: %d)\n", mbox_rc);
		printk("u3 fb-alloc: skip (the mailbox did not answer: %d)\n", mbox_rc);
	}
	if (mbox_rc == 0) {
		static const char *const names[RPI5_GET_COUNT] = { "physical", "virtual", "depth", "order",
								   "pitch", "offset" };
		int answered = rpi5_screen_query(&q);
		size_t t;

		for (t = 0; t < (size_t)RPI5_GET_COUNT; t++) {
			printk("rpi5_display: get %s: rc %d code 0x%08x values %u %u\n", names[t], q.rc[t],
			       q.code[t], q.val[t][0], q.val[t][1]);
		}
		printk("rpi5_display: %d of %u framebuffer tags answered; the firmware reports a "
		       "framebuffer: %s\n", answered, (unsigned int)RPI5_GET_COUNT,
		       rpi5_screen_query_has_fb(&q) ? "yes" : "no");
		step_fbquery();

		mbox_rc = rpi_fb_alloc_virtual(FB_W, FB_H, FB_VIRT, &fb, got);
		printk("rpi5_display: FRAMEBUFFER_ALLOCATE physical %ux%u virtual %ux%u depth 32 order RGB "
		       "offset 0,0 allocate 4096 then GET_PITCH: rc %d\n", FB_W, FB_H, FB_W, FB_VIRT, mbox_rc);
		/* The allocation's class in fb_query.h's tokens. rpi_fb_alloc_virtual checks every
		 * answer itself and returns one rc, so -EIO stands for both of fb_query's "refused"
		 * (base or size 0) and "unknown" (a tag not answered): the record above says which
		 * tags were answered before it. */
		if (mbox_rc == 0) {
			printk("u3 fb-alloc: %s (%ux%u pitch %u %s size %u)\n",
			       fbq_class_name(FBQ_ALLOC_GRANTED), fb.width, fb.height, fb.pitch,
			       fb_fmt_name(fb.order), fb.size);
		} else if (mbox_rc == -ERANGE) {
			printk("u3 fb-alloc: %s another window (physical %ux%u virtual %ux%u)\n",
			       fbq_class_name(FBQ_ALLOC_GRANTED), got[0], got[1], got[2], got[3]);
		} else if (mbox_rc == -EIO) {
			printk("u3 fb-alloc: %s or %s (rc %d)\n", fbq_class_name(FBQ_ALLOC_REFUSED),
			       fbq_class_name(FBQ_TAG_UNKNOWN), mbox_rc);
		} else if (mbox_rc == -ENOMEM) {
			printk("u3 fb-alloc: %s but not mapped (rc %d)\n", fbq_class_name(FBQ_ALLOC_GRANTED),
			       mbox_rc);
		} else {
			printk("u3 fb-alloc: skip (the allocation was not answered: %d)\n", mbox_rc);
		}
		if (mbox_rc == 0) {
			static char fb_buf[96]; /* static: printed after this block, in the verdict line */

			mailbox_ok = true;
			fb_state = "PASS";
			(void)snprintf(fb_buf, sizeof(fb_buf), "%u x %u pitch %u %s", fb.width, fb.height,
				       fb.pitch, fb_fmt_name(fb.order));
			fb_why = fb_buf;
			printk("rpi5_display: granted %ux%u, %u virtual rows, pitch %u, size %u, ARM physical "
			       "0x%08x, order tag code 0x%08x value %u (%s), mapping %s\n", fb.width, fb.height,
			       FB_VIRT, fb.pitch, fb.size, fb.phys, fb.order_code, fb.order,
			       fb_fmt_name(fb.order), RPI_FB_MAPPING);
		} else if (mbox_rc == -ERANGE) {
			static char erange[192];

			/* Named, not used: the firmware may grant another window than the one asked for
			 * (driver/docs/rpi5-display.md section 8). */
			(void)snprintf(erange, sizeof(erange),
				       "the firmware granted physical %ux%u and virtual %ux%u, not %ux%u physical "
				       "and %u virtual rows", got[0], got[1], got[2], got[3], FB_W, FB_H, FB_VIRT);
			fb_why = erange;
		} else {
			static char mb_why[128];

			(void)snprintf(mb_why, sizeof(mb_why),
				       "the allocation and its tags were not answered as a grant (%d)", mbox_rc);
			fb_why = mb_why;
		}
	} else {
		static char no_mbox[96];

		(void)snprintf(no_mbox, sizeof(no_mbox), "not asked: the mailbox did not answer (%d)", mbox_rc);
		fb_why = no_mbox;
	}
	printk("u3 mailbox-fb: %s (%s)\n", fb_state, fb_why);

	/* ---- Z3: the pattern in the granted buffer, then the list again ---- */
	step_at("4 mailbox-readback");
	if (mailbox_ok) {
		enum pattern_fmt pf = fmt_of_order(fb.order);
		struct pattern_report r1, r2;
		static char rb_buf[192]; /* static: printed after this block, in the verdict line */
		const uint32_t bpp = 32u; /* the depth the allocation asked for; pattern.h's bpp is in bits */
		int filled, checked, cleaned;

		(void)memset(&r1, 0, sizeof(r1));
		(void)memset(&r2, 0, sizeof(r2));
		if (fb.pitch < fb.width * 4u || (uint64_t)fb.pitch * fb.height > (uint64_t)fb.size) {
			/* A named refusal, not a partial draw: a grant whose own numbers do not describe the
			 * screen it named is not written into (driver/docs/rpi5-display.md section 8). */
			(void)snprintf(rb_buf, sizeof(rb_buf),
				       "the grant does not cover the screen it names: pitch %u times height %u "
				       "is %llu bytes of the %u granted", fb.pitch, fb.height,
				       (unsigned long long)((uint64_t)fb.pitch * fb.height), fb.size);
			rb_why = rb_buf;
		} else {
			filled = pattern_fill(fb.cpu, fb.width, fb.height, fb.pitch, bpp, pf);
			cleaned = clean_range(fb.cpu, (uint64_t)fb.pitch * fb.height);
			checked = pattern_check(fb.cpu, fb.width, fb.height, fb.pitch, bpp, pf, 1u, &r1);
			k_msleep(SAMPLE_GAP_MS);
			(void)pattern_check(fb.cpu, fb.width, fb.height, fb.pitch, bpp, pf, 1u, &r2);
			printk("rpi5_display: mailbox buffer pattern %s fill %d, clean %d, check %d: checksum "
			       "0x%08x then 0x%08x over %u pixels, %u then %u differing\n", pattern_fmt_name(pf),
			       filled, cleaned, checked, r1.checksum, r2.checksum, (unsigned int)r2.checked,
			       r1.mismatches, r2.mismatches);
			if (filled == 0 && checked == 0 && r1.mismatches == 0 && r2.mismatches == 0) {
				fb_readback = true;
				rb_state = "PASS";
				(void)snprintf(rb_buf, sizeof(rb_buf), "%s pattern at 0x%08x read back "
							       "byte-equal twice, %u ms apart, 0 mismatches, "
							       "checksum 0x%08x", pattern_fmt_name(pf),
					       fb.phys, SAMPLE_GAP_MS, r2.checksum);
				rb_why = rb_buf;
			} else {
				(void)snprintf(rb_buf, sizeof(rb_buf),
					       "%u of %u pixels differ (first at %u,%u: written 0x%08x, read "
					       "0x%08x), checksum 0x%08x then 0x%08x", r1.mismatches,
					       (unsigned int)r1.checked, r1.first_x, r1.first_y, r1.expected,
					       r1.got, r1.checksum, r2.checksum);
				rb_why = rb_buf;
			}
		}
	}
	printk("u3 mailbox-readback: %s (%s)\n", rb_state, rb_why);

	/* Z3's decision, printed on every path. plane_match.h names a missing side by its
	 * reason ("no buffer", "no plane in the display list"), which is the honest answer to
	 * the question verdicts.sh asks: a log missing one of its lines reads as no verdict at
	 * all, which says less than a `no` with its reason named. */
	if (mailbox_ok) {
		buf.addr = (uint64_t)fb.phys; /* struct rpi_fb's .phys is documented as ARM physical */
		buf.pitch = fb.pitch;
		buf.w = fb.width;
		buf.h = fb.height;
		buf.fmt = fb_fmt_name(fb.order);
		have_buf = true;
	}
	if (mailbox_ok && hvs_on && stepping_ok) {
		struct chan_words w;
		struct hvs6_plane p;
		/* Every channel's list, read again after the allocation, with its own comparison: the
		 * running channel is the one whose list can answer the question, and it need not be the
		 * channel Z1's plane came from (Z1 reads the first plane of every channel that names a
		 * list, and a list may hold more than one plane). */
		struct pm_plane cand[SA_NUM_CHANNELS];
		struct pm_result cand_res[SA_NUM_CHANNELS];
		bool have_cand[SA_NUM_CHANNELS];
		unsigned int ncand = 0;
		unsigned int i, k;

		(void)memset(cand, 0, sizeof(cand));
		(void)memset(cand_res, 0, sizeof(cand_res));
		(void)memset(have_cand, 0, sizeof(have_cand));

		for (i = 0; i < SA_NUM_CHANNELS; i++) {
			uint32_t head;

			chan_words_read(i, d0, &w);
			head = chan_head(i, &w);
			if (head == 0) {
				printk("rpi5_display: ch%u names no list after the allocation either: DL.LACT "
				       "and LPTRS.HEADE are both 0\n", i);
				continue;
			}
			printk("rpi5_display: ch%u's list read again after the allocation, from word %u\n", i,
			       head);
			if (!read_plane(i, head, &p)) {
				continue;
			}
			if (bcm2712_bus_to_arm(p.addr, &cand[i].addr) != 0) {
				cand[i].addr = p.addr; /* printed as the list holds it */
			}
			cand[i].pitch = p.pitch;
			cand[i].w = p.width;
			cand[i].h = p.height;
			cand[i].code = p.format_code;
			cand[i].order = p.order;
			cand[i].fmt = plane_fmt(&p);
			have_cand[i] = true;
			ncand++;
			cand_res[i] = pm_compare(&cand[i], &buf, &mmu);
			printk("rpi5_display: ch%u now names a plane at 0x%llx, pitch %u, %ux%u, format code %u "
			       "order %u (%s): list-vs-buffer %s\n", i, (unsigned long long)cand[i].addr,
			       cand[i].pitch, cand[i].w, cand[i].h, cand[i].code, cand[i].order,
			       cand[i].fmt ? cand[i].fmt : "unknown", pm_words(&cand_res[i]));
		}
		if (ncand > 0u) {
			unsigned int chosen = 0;
			struct pm_result best = pm_choose(cand_res, have_cand, (unsigned int)SA_NUM_CHANNELS,
							  scan_running ? scan_ch : (unsigned int)SA_NUM_CHANNELS,
							  &buf, &chosen);

			pp = cand[chosen];
			have_pp = true;
			printk("rpi5_display: the decision line below names ch%u's list (pm_choose's rule): %u of "
			       "%u channels named a decodable list after the allocation, and %s\n", chosen, ncand,
			       (unsigned int)SA_NUM_CHANNELS, pm_words(&best));
		}
		/* The scanout sampled once more while our own bytes were in the buffer, on the channel Z1
		 * judged running, once with each mapped pixel valve's words (chan_verdict's rule). */
		if (pv_on || pv1_on) {
			struct chan_words c1, c2;
			struct sa_sample t1[2], t2[2];
			struct sa_verdict tv[2];
			struct sa_verdict v3;
			unsigned int used;

			(void)memset(tv, 0, sizeof(tv));
			sample_at(version, scan_ch, d0, &c1, t1);
			k_msleep(SAMPLE_GAP_MS);
			sample_at(version, scan_ch, d0, &c2, t2);
			for (k = 0; k < 2u; k++) {
				if (valve_mapped[k]) {
					tv[k] = sa_judge(&t1[k], &t2[k], scan_ch);
					chan_valve_record(scan_ch, k, &t2[k], &tv[k]);
				}
			}
			v3 = chan_verdict(tv, valve_mapped, &used);
			printk("rpi5_display: while our own bytes were in the buffer, ch%u: %s (from "
			       "pixelvalve%u's words)\n", scan_ch,
			       v3.running ? "running" : sa_reason_text(v3.reason), used);
		}
	}
	res = pm_compare(have_pp ? &pp : NULL, have_buf ? &buf : NULL, &mmu);
	(void)pm_format_line(&res, line, sizeof(line));
	printk("%s\n", line); /* the `u3 scanout-follows-mailbox:` line, plane_match.h's words */
	follow_state = res.verdict == PM_YES ? "yes" : "no";
	if (res.fmt_differs) {
		printk("rpi5_display: the two sides name the same 8888 buffer differently: the list says %s "
		       "(code %u order %u) and the mailbox's pixel-order tag %u is %s. A name, not a "
		       "difference in the bytes (plane_match.h: information only)\n",
		       pp.fmt ? pp.fmt : "unknown", pp.code, pp.order, fb.order, buf.fmt);
	}
	if (res.size_differs) {
		printk("rpi5_display: the list's plane is %ux%u and the buffer is %ux%u: information, not "
		       "part of the decision\n", pp.w, pp.h, buf.w, buf.h);
	}
	if (res.verdict == PM_NOT_COMPARABLE) {
		printk("rpi5_display: iommu4 leaves an address at or above 0x%llx ambiguous, so this "
		       "comparison is recorded and not trusted (driver/docs/rpi5-display.md section 6)\n",
		       (unsigned long long)mmu.pass_below);
	}

	/* ---- Z4: the Pi 4's split screen ---- */
	step_at("5 split-screen");
	if (mailbox_ok && fb_readback) {
		scr.cpu = fb.cpu;
		scr.width = fb.width;
		scr.height = fb.height;
		scr.pitch = fb.pitch;
		scr.order = fb.order;
		scr.what = "the mailbox buffer";
	} else if (adopt_scr.cpu != NULL) {
		scr = adopt_scr;
		printk("rpi5_display: no mailbox buffer read back, so the split screen goes into the adopted "
		       "firmware plane, whose read-back passed\n");
	}
	if (scr.cpu != NULL) {
		struct pattern_layout layout = split_layout(scr.width, scr.height);
		struct pattern_report text, band;
		enum pattern_fmt pf = fmt_of_order(scr.order);
		static char split_buf[192]; /* static: printed after this block, in the verdict line */
		const uint32_t bpp = 32u; /* both sources are 32 bpp; pattern.h's bpp is in bits */
		uint32_t pixels = layout.text.w * layout.text.h;
		uint32_t first_blank = 0, first_drawn = 0;
		uint32_t blank_diff, drawn_diff;
		int cleaned;

		(void)memset(&text, 0, sizeof(text));
		(void)memset(&band, 0, sizeof(band));
		/* The whole buffer holds the pattern (step 4 filled it and read it back); the band below
		 * is filled again so the expected values check_rect compares are fresh. */
		(void)pattern_fill_rect(scr.cpu, scr.width, scr.height, scr.pitch, bpp, pf, layout.pattern);
		uart_out = __printk_get_hook();
		fbcon_attach(scr.cpu, scr.width, scr.pitch, layout.text.h, 2);
		fb_out = __printk_get_hook();
		__printk_hook_install(console_out);
		/* The text band is counted twice, once before the text and once after it, because
		 * fbcon_attach() clears the rows it was given and that blank clear alone changes every
		 * pixel of a band that held the pattern. Flat after the attach and not flat after the
		 * text is the measurement that says a character was drawn (rect_changed_from_first). */
		blank_diff = rect_changed_from_first(scr.cpu, scr.width, scr.height, scr.pitch, bpp,
						     layout.text, &first_blank);
		(void)pattern_check_rect(scr.cpu, scr.width, scr.height, scr.pitch, bpp, pf, layout.pattern,
					 1u, &band);
		printk("rpi5_display: split screen in %s: console rows 0..%u, pattern rows %u..%u of %ux%u "
		       "(%s, pattern.h's split): the text band reads as %s (%u of %u pixels differ from its "
		       "first pixel 0x%08x) and the pattern band holds what was written (%u of %u pixels "
		       "differ)\n", scr.what, layout.text.h - 1u, layout.pattern.y, scr.height - 1u, scr.width,
		       scr.height, pattern_fmt_name(pf),
		       blank_diff == 0u ? "one flat colour, the console's own clear" : "not one flat colour",
		       blank_diff, pixels, first_blank, band.mismatches, (unsigned int)band.checked);
		printk("rpi5_display: these two lines go through the printk hook fbcon.c installed, so their "
		       "glyphs are pixels of the text band above\n");
		cleaned = clean_range(scr.cpu, (uint64_t)scr.pitch * scr.height);
		drawn_diff = rect_changed_from_first(scr.cpu, scr.width, scr.height, scr.pitch, bpp,
						     layout.text, &first_drawn);
		(void)pattern_check_rect(scr.cpu, scr.width, scr.height, scr.pitch, bpp, pf, layout.text, 1u,
					 &text);
		printk("rpi5_display: after the text (clean %d): %u of %u text-band pixels differ from its "
		       "first pixel 0x%08x, and %u of %u text-band pixels are no longer the pattern; pattern "
		       "band checksum 0x%08x\n", cleaned, drawn_diff, pixels, first_drawn, text.mismatches,
		       (unsigned int)text.checked, band.checksum);
		if (band.mismatches == 0u && blank_diff == 0u && drawn_diff > 0u && text.mismatches > 0u) {
			split_ok = true;
			(void)snprintf(split_buf, sizeof(split_buf),
				       "drawn (text on top, pattern below, in %s; pattern band 0 mismatches, "
				       "checksum 0x%08x)", scr.what, band.checksum);
		} else {
			(void)snprintf(split_buf, sizeof(split_buf),
				       "not drawn (in %s: blank %u of %u text pixels, drawn %u of %u after the "
				       "text, pattern band %u of %u differing)", scr.what, blank_diff, pixels,
				       drawn_diff, pixels, band.mismatches, (unsigned int)band.checked);
		}
		split_state = split_buf;
	} else {
		split_state = "not drawn (no buffer read back: neither the mailbox grant nor an adopted "
			      "32 bpp plane)";
	}
	printk("u3 split-screen: %s\n", split_state);

	/* ---- the terminal lines ---- */
	step_at("6 summary");
	if (mbox_init_rc != 0 && !split_ok) {
		printk("rpi5_display: FAIL (the mailbox did not answer: %d, so nothing was asked or "
		       "granted)\n", mbox_init_rc);
	} else if (mailbox_ok && fb_readback && split_ok) {
		printk("rpi5_display: PASS (framebuffer %s, %s, %s)\n", fb_why, rb_why, split_state);
	} else if (split_ok) {
		printk("rpi5_display: PASS (adopted plane: %s; %s)\n", ad_why, split_state);
	} else {
		printk("rpi5_display: FAIL (mailbox-fb %s: %s; mailbox-readback %s: %s; split-screen %s)\n",
		       fb_state, fb_why, rb_state, rb_why, split_state);
	}
	printk("rpi5_display: measured: hvs %d, plane %d, adopt %s, mailbox %d, readback %d, "
	       "list-vs-buffer %s, split %d\n", hvs_on ? 1 : 0, plane_found ? 1 : 0, ad_state,
	       mailbox_ok ? 1 : 0, fb_readback ? 1 : 0, follow_state, split_ok ? 1 : 0);
	printk("rpi5_display: feeding stopped, reset expected (the watchdog resets the SoC within "
	       "%u s, and tryboot returns the board to Linux)\n", (unsigned int)RPI_WDOG_HOLD_S);

	for (;;) {
		k_sleep(K_FOREVER);
	}
	return 0;
}
