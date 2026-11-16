/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * plane_match.h: the U3 decision "does the HVS6 display list name the buffer
 * the image has?" (tools/probes/rpi5_display, driver/docs/rpi5-display.md
 * sections 6 Z2/Z3 and 7). Header-only C11: no MMIO, no allocation, no Zephyr
 * header, so the host test (src/test/plane_match_test.c) runs the same
 * functions the image runs.
 *
 * The rule is the design note's section 7: a route counts as working when the
 * display list names the image's buffer, `PTR0.UPPER_ADDR:PTR1` equal to the
 * buffer's address and `PTR2.PITCH` equal to the pitch the board layer uses.
 * Those two are the yes/no. Size and format come along as information, never
 * as a part of it: the firmware and Linux name the same 8888 buffer
 * differently (XRGB8888 vs ARGB8888), and verdicts.sh reads its fmt comparison
 * as information too. A plane whose format the table does not name is still
 * matched on address and pitch, and its format is printed as unknown.
 *
 * IOMMU4 (BCM2712, the HVS's IOMMU: rpi:arch/arm64/boot/dts/broadcom/
 * bcm2712-ds.dtsi:292-299 with the `iommus = <&iommu4>` of hvs at
 * bcm2712.dtsi:622) is why "differs" is not always a fact: the address in the
 * list is the address the HVS fetches from, a DMA address. The driver's own
 * words (rpi:drivers/iommu/bcm2712-iommu.c:98-99,184-187): the IOVA region
 * starts at an aperture and "requests to lower addresses pass straight
 * through to SDRAM", "addresses below the 'aperture' are passed unchanged".
 * So an address below the pass-through window *is* an ARM physical address and
 * the comparison holds; above it the list address may be an IOVA and no verdict
 * is named: PM_NOT_COMPARABLE.
 *
 * pm_mmu_state() reads the state out of the words Z0 dumps (offsets and bits
 * cited below). Whether a *disabled* MMU passes addresses through is an
 * inference from the meaning of its enable bit (MMMU_CTRL_ENABLE, written by
 * the driver to turn the MMU on: rpi:drivers/iommu/bcm2712-iommu.c:43,226-232)
 * and not a fact a tree states; pm_mmu_state therefore says which words it
 * classified and the probe prints them next to the class.
 *
 * The line pm_format_line() writes is the `u3 scanout-follows-mailbox` line of
 * tools/probes/rpi5_display/verdicts.sh, whose grammar accepts `yes` or `no`
 * only. PM_NOT_COMPARABLE prints `no` with its reason named
 * ("not comparable: ..."): the question asked is whether the list names this
 * buffer, and "unknown" is the honest answer to that question, said in the
 * reason.
 */
#ifndef RPI5_DISPLAY_PLANE_MATCH_H
#define RPI5_DISPLAY_PLANE_MATCH_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

/* ---- the two sides ------------------------------------------------------- */

/* The buffer the image holds: the mailbox grant (rpi_mbox.h's struct rpi_fb,
 * whose .phys is documented as an ARM physical address) or the buffer of a
 * mapped adopted plane. */
struct pm_buffer {
	uint64_t addr; /* ARM physical */
	uint32_t pitch; /* bytes per row */
	uint32_t w, h;
	const char *fmt; /* name for the report, NULL when unknown */
};

/* The first plane of a channel's display list (hvs6_dlist.h's struct
 * hvs6_plane, translated to ARM physical by bus_addr.h). */
struct pm_plane {
	uint64_t addr; /* ARM physical, after the bus-to-ARM step */
	uint32_t pitch;
	uint32_t w, h;
	uint32_t code; /* CTL0.PIXEL_FORMAT */
	uint32_t order; /* CTL0.ORDERRGBA */
	const char *fmt; /* name from hvs6_fmt.h, NULL when unlisted */
};

/* ---- IOMMU4, as the registers leave it ----------------------------------- */

/* MMMU_CTRL, byte offset 0x00 (rpi:drivers/iommu/bcm2712-iommu.c:24). */
#define PM_MMMU_CTRL_ENABLE (1u << 0) /* :43 */
/* MMMU_DEBUG_INFO, 0x38 (:74): BYPASS_4M bit 28 (:80), BYPASS bit 29 (:81). */
#define PM_DEBUG_INFO_BYPASS_4M (1u << 28)
#define PM_DEBUG_INFO_BYPASS (1u << 29)
/* MMMU_BYPASS_END, 0x20: the enable bit (:64) and, in the low 31 bits, the end
 * of the pass-through window in units (rpi:drivers/iommu/bcm2712-iommu.c:63-64,
 * written at :198-203). */
#define PM_BYPASS_END_ENABLE (1u << 31)
#define PM_BYPASS_END_MASK 0x7fffffffu
/* Units of those two registers: 256 MB (ADDR_CAP_SHIFT, :52-53) or 4 MB
 * (IOMMU_HUGEPAGE_SHIFT = IOMMU_PAGE_SHIFT 12 + IOMMU_PAGEWORDS_SHIFT 10,
 * :110-111) when DEBUG_INFO names the 4 MB bypass granularity. */
#define PM_BYPASS_UNIT_SHIFT 28u
#define PM_BYPASS_UNIT_4M_SHIFT 22u

struct pm_mmu {
	bool read; /* the words were read; false: nothing is known */
	bool enabled; /* MMMU_CTRL.ENABLE */
	bool has_bypass; /* DEBUG_INFO.BYPASS: the block has the feature */
	bool bypass_4m; /* DEBUG_INFO.BYPASS_4M: the unit of BYPASS_END */
	uint64_t pass_below; /* addresses below this pass through; 0: none known */
};

/*
 * Classify the words Z0 read from iommu4 (MMMU_CTRL at +0x00,
 * MMMU_DEBUG_INFO at +0x38, MMMU_BYPASS_END at +0x20). `read` says whether the
 * block was read at all; 0x00000000 and 0xffffffff are passed through as the
 * values they are, and the probe prints the words next to the class so a reader
 * can see what the class was named from.
 */
static inline void pm_mmu_state(uint32_t ctrl, uint32_t debug_info, uint32_t bypass_end,
				bool read, struct pm_mmu *out)
{
	unsigned int shift;

	out->read = read;
	out->enabled = (ctrl & PM_MMMU_CTRL_ENABLE) != 0;
	out->has_bypass = (debug_info & PM_DEBUG_INFO_BYPASS) != 0;
	out->bypass_4m = (debug_info & PM_DEBUG_INFO_BYPASS_4M) != 0;
	shift = out->bypass_4m ? PM_BYPASS_UNIT_4M_SHIFT : PM_BYPASS_UNIT_SHIFT;
	out->pass_below = 0;
	if (bypass_end & PM_BYPASS_END_ENABLE) {
		uint64_t n = bypass_end & PM_BYPASS_END_MASK;

		/* The window's end in the register's units; a value that would
		 * overflow 64 bits means "not expressible", which no board holds. */
		if (n <= (UINT64_MAX >> shift)) {
			out->pass_below = n << shift;
		} else {
			out->pass_below = UINT64_MAX;
		}
	}
}

/* True when an address of this size reaches the HVS unchanged: the MMU is off,
 * or the pass-through window covers it. False when nothing is known. */
static inline bool pm_addr_passes_through(const struct pm_mmu *mmu, uint64_t addr)
{
	if (!mmu || !mmu->read) {
		return false;
	}
	if (!mmu->enabled) {
		return true;
	}
	return addr < mmu->pass_below;
}

/* ---- the decision -------------------------------------------------------- */

enum pm_verdict {
	PM_YES = 0, /* the list names this buffer */
	PM_NO, /* it names another, or not the same pitch */
	PM_NOT_COMPARABLE /* the two sides are not the same kind of address */
};

struct pm_result {
	enum pm_verdict verdict;
	const char *reason; /* short, never NULL; empty for a plain yes */
	const struct pm_plane *plane; /* as given, may be NULL */
	const struct pm_buffer *buffer; /* as given, may be NULL */
	bool fmt_differs; /* both names known and unequal (information) */
	bool size_differs; /* both sizes known and unequal (information) */
};

static inline struct pm_result pm_result_(enum pm_verdict v, const char *why,
					   const struct pm_plane *p, const struct pm_buffer *b)
{
	struct pm_result r;

	r.verdict = v;
	r.reason = why;
	r.plane = p;
	r.buffer = b;
	r.fmt_differs = false;
	r.size_differs = false;
	return r;
}

/*
 * The decision. `plane` is the plane decoded from the list after the buffer was
 * installed (NULL: no list, no plane); `buffer` is the buffer (NULL: none).
 *
 * `mmu` is the state read out of the iommu4 words, and a NULL `mmu` is the same
 * as one whose words were not read: nothing is known about the block, so the
 * list address may be an IOVA and the verdict is PM_NOT_COMPARABLE rather than
 * a match or a difference. That is deliberate — a caller who knows the question
 * does not apply (no IOMMU in the path, or an MMU that is off) says so by
 * filling the struct, and passing NULL is not a way to skip it.
 */
static inline struct pm_result pm_compare(const struct pm_plane *plane,
					   const struct pm_buffer *buffer,
					   const struct pm_mmu *mmu)
{
	struct pm_result r;

	if (!buffer) {
		return pm_result_(PM_NO, "no buffer", plane, buffer);
	}
	if (!plane) {
		return pm_result_(PM_NO, "no plane in the display list", plane, buffer);
	}
	if (plane->addr == 0) {
		return pm_result_(PM_NO, "the display list names no address", plane, buffer);
	}
	if (!pm_addr_passes_through(mmu, plane->addr)) {
		return pm_result_(PM_NOT_COMPARABLE, "not comparable: iommu4 leaves this address above its "
						    "pass-through window, so the list address may be an IOVA",
				 plane, buffer);
	}

	r = pm_result_(PM_YES, "", plane, buffer);
	r.fmt_differs = plane->fmt != NULL && buffer->fmt != NULL &&
			strcmp(plane->fmt, buffer->fmt) != 0;
	r.size_differs = plane->w != buffer->w || plane->h != buffer->h;

	if (plane->addr != buffer->addr) {
		r.verdict = PM_NO;
		r.reason = "address differs";
	} else if (plane->pitch != buffer->pitch) {
		r.verdict = PM_NO;
		r.reason = "pitch differs";
	}
	return r;
}

/*
 * Which comparison the one decision line names, when several lists were read.
 *
 * `res[0..n)` holds a pm_compare result for candidate i, and `have[i]` says whether
 * that slot holds one at all (a channel whose list named no decodable plane has
 * none). `preferred` is the channel the scanout was judged running on, or n when no
 * channel was judged running. The order:
 *   1 a yes answers the question, whatever another list says (the first yes, in
 *     index order);
 *   2 else the running channel's comparison, when it has one: only a running
 *     channel's list is what the HVS scans, so its answer is about the screen even
 *     when it is "not comparable", while another channel's is about a list nothing
 *     displays;
 *   3 else a plain no (a list naming another buffer or another pitch) over one the
 *     IOMMU leaves open;
 *   4 else the first in index order.
 * `*chosen` is the slot chosen, or n when there was none; the result returned then
 * names the missing side in the words pm_compare uses.
 */
static inline struct pm_result pm_choose(const struct pm_result *res, const bool *have,
					 unsigned int n, unsigned int preferred,
					 const struct pm_buffer *buffer, unsigned int *chosen)
{
	unsigned int i;
	unsigned int pick = n;

	if (n == 0u || !res || !have) {
		*chosen = 0u;
		return pm_result_(PM_NO, buffer ? "no plane in the display list" : "no buffer", NULL,
				 buffer);
	}
	for (i = 0; i < n; i++) {
		if (have[i] && res[i].verdict == PM_YES) {
			pick = i;
			break;
		}
	}
	if (pick == n && preferred < n && have[preferred]) {
		pick = preferred;
	}
	if (pick == n) {
		for (i = 0; i < n; i++) {
			if (have[i] && res[i].verdict == PM_NO) {
				pick = i;
				break;
			}
		}
	}
	if (pick == n) {
		for (i = 0; i < n; i++) {
			if (have[i]) {
				pick = i;
				break;
			}
		}
	}
	*chosen = pick;
	if (pick == n) {
		return pm_result_(PM_NO, buffer ? "no plane in the display list" : "no buffer", NULL,
				 buffer);
	}
	return res[pick];
}

/*
 * The `u3 scanout-follows-mailbox` line, without a newline. Returns the length
 * the text needs (snprintf's rule: n or more means it was truncated).
 *
 * Both branches name both addresses. They are equal on a yes, and naming them
 * anyway is what makes the line a record of the reading it judged (the same is
 * true of the size and the format name, which are printed as information).
 *
 *   yes  u3 scanout-follows-mailbox: yes (list 0x<addr> pitch <n> <w>x<h> fmt
 *        <fmt>; buffer 0x<addr> pitch <n> <w>x<h> fmt <fmt>)
 *   no   u3 scanout-follows-mailbox: no (<reason>: list 0x<addr> pitch <n>
 *        <w>x<h> fmt <fmt>, buffer 0x<addr> pitch <n> <w>x<h> fmt <fmt>)
 */
static inline int pm_format_line(const struct pm_result *r, char *out, size_t n)
{
	static const char *const word[2] = { "yes", "no" };
	const char *fmt_p, *fmt_b;
	int len;
	unsigned int wi;

	if (!r || !out) {
		return 0;
	}
	wi = r->verdict == PM_YES ? 0u : 1u; /* PM_NOT_COMPARABLE reads "no" too */
	fmt_p = r->plane && r->plane->fmt ? r->plane->fmt : "unknown";
	fmt_b = r->buffer && r->buffer->fmt ? r->buffer->fmt : "unknown";
	if (r->verdict == PM_YES) {
		len = snprintf(out, n,
			       "u3 scanout-follows-mailbox: %s (list 0x%llx pitch %u %ux%u fmt %s; "
			       "buffer 0x%llx pitch %u %ux%u fmt %s)",
			       word[wi], (unsigned long long)(r->plane ? r->plane->addr : 0),
			       r->plane ? r->plane->pitch : 0u, r->plane ? r->plane->w : 0u,
			       r->plane ? r->plane->h : 0u, fmt_p,
			       (unsigned long long)(r->buffer ? r->buffer->addr : 0),
			       r->buffer ? r->buffer->pitch : 0u, r->buffer ? r->buffer->w : 0u,
			       r->buffer ? r->buffer->h : 0u, fmt_b);
	} else {
		len = snprintf(out, n,
			       "u3 scanout-follows-mailbox: %s (%s: list 0x%llx pitch %u %ux%u fmt %s, "
			       "buffer 0x%llx pitch %u %ux%u fmt %s)",
			       word[wi], r->reason ? r->reason : "no reason",
			       (unsigned long long)(r->plane ? r->plane->addr : 0),
			       r->plane ? r->plane->pitch : 0u, r->plane ? r->plane->w : 0u,
			       r->plane ? r->plane->h : 0u, fmt_p,
			       (unsigned long long)(r->buffer ? r->buffer->addr : 0),
			       r->buffer ? r->buffer->pitch : 0u, r->buffer ? r->buffer->w : 0u,
			       r->buffer ? r->buffer->h : 0u, fmt_b);
	}
	return len;
}

#endif /* RPI5_DISPLAY_PLANE_MATCH_H */
