/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of tools/probes/rpi5_display/src/plane_match.h: the U3 decision
 * "does the HVS6 display list name the buffer the image has", the IOMMU4
 * pass-through classification, and the verdict line verdicts.sh reads. Built by
 * tools/probes/rpi5_display/src/plane_match_selftest.sh with ASan and UBSan.
 *
 * The synthetic numbers are the probe's own shapes (1280x720 at pitch 5120, a
 * firmware plane at 0x3c000000, an aperture at 40 GiB), not board readings: no
 * address, host or console path of any board appears here.
 *
 * Last line: "plane_match_test: N of N checks, 0 failed". Exit 0 when all pass.
 */
#include "../plane_match.h"

#include <stdio.h>
#include <string.h>

static int checks, failed;

static void check(const char *what, int ok)
{
	checks++;
	if (!ok) {
		failed++;
		printf("FAIL: %s\n", what);
	}
}

static void check_str(const char *what, const char *got, const char *want)
{
	checks++;
	if (strcmp(got, want) != 0) {
		failed++;
		printf("FAIL: %s\n  got:  %s\n  want: %s\n", what, got, want);
	}
}

static void check_has(const char *what, const char *got, const char *needle)
{
	checks++;
	if (!strstr(got, needle)) {
		failed++;
		printf("FAIL: %s: %s lacks \"%s\"\n", what, got, needle);
	}
}

/* A 1280x720 buffer at the pitch the board layer uses. */
static struct pm_buffer buf_(uint64_t addr)
{
	struct pm_buffer b;

	b.addr = addr;
	b.pitch = 5120u;
	b.w = 1280u;
	b.h = 720u;
	b.fmt = "XRGB8888";
	return b;
}

/* The same plane the list would name. */
static struct pm_plane plane_(uint64_t addr, uint32_t pitch, const char *fmt)
{
	struct pm_plane p;

	p.addr = addr;
	p.pitch = pitch;
	p.w = 1280u;
	p.h = 720u;
	p.code = 7u; /* RGBA8888 family */
	p.order = 2u; /* ARGB, the order XRGB8888 is listed with */
	p.fmt = fmt;
	return p;
}

/* iommu4 as the Linux driver leaves it when it runs: enabled, 40 GiB aperture,
 * everything below it passing through (rpi:drivers/iommu/bcm2712-iommu.c:115,
 * 198-200, 226-232). The register's value is the aperture end in 256 MB units,
 * with the enable bit set. */
#define APERTURE_40G (40ull << 30)
#define BYPASS_END_40G_256M (PM_BYPASS_END_ENABLE | (APERTURE_40G >> PM_BYPASS_UNIT_SHIFT))
#define BYPASS_END_40G_4M (PM_BYPASS_END_ENABLE | (APERTURE_40G >> PM_BYPASS_UNIT_4M_SHIFT))

int main(void)
{
	struct pm_mmu mmu, off;
	struct pm_buffer b = buf_(0x3c000000ull);
	struct pm_plane p = plane_(0x3c000000ull, 5120u, "XRGB8888");
	struct pm_result r;
	char line[256];

	/* iommu4 with its enable bit clear: nothing translates, so the address in
	 * the list is the address the HVS fetched from. The checks of the decision
	 * itself use it; the IOMMU checks below fill their own states. */
	pm_mmu_state(0, 0, 0, true, &off);

	/* ---- the decision itself -------------------------------------------- */

	r = pm_compare(&p, &b, &off);
	check("equal address and pitch is a yes", r.verdict == PM_YES);
	check("a yes names no reason", r.reason[0] == '\0');

	{
		struct pm_buffer other = buf_(0x40000000ull);

		r = pm_compare(&p, &other, &off);
		check("another address is a no", r.verdict == PM_NO);
		check_str("the reason names the address", r.reason, "address differs");
		check("and its address is carried for the line", r.buffer == &other);
	}

	{
		struct pm_plane odd = p; /* a copy: p stays the plane the list names */

		odd.pitch = 7680u;
		r = pm_compare(&odd, &b, &off);
		check("another pitch is a no", r.verdict == PM_NO);
		check_str("the reason names the pitch", r.reason, "pitch differs");

		odd.addr = 0x40000000ull;
		r = pm_compare(&odd, &b, &off);
		check("both differ is one no", r.verdict == PM_NO);
		check_str("and the address is what is named first", r.reason, "address differs");
	}

	check("no buffer is a no", pm_compare(&p, NULL, &off).verdict == PM_NO);
	check("no plane is a no", pm_compare(NULL, &b, &off).verdict == PM_NO);
	check_str("and says which", pm_compare(NULL, &b, &off).reason, "no plane in the display list");

	{
		struct pm_plane zero = plane_(0, 5120u, "XRGB8888");

		r = pm_compare(&zero, &b, &off);
		check("a list naming no address is a no", r.verdict == PM_NO);
		check_str("and says so", r.reason, "the display list names no address");
	}

	/* format and size are information, never the verdict */
	{
		struct pm_plane argb = plane_(0x3c000000ull, 5120u, "ARGB8888");

		r = pm_compare(&argb, &b, &off);
		check("another format name is still a yes", r.verdict == PM_YES);
		check("and is recorded as a format difference", r.fmt_differs);
		check("with the plane kept for the line", r.plane == &argb);
	}

	r = pm_compare(&p, &b, &off);
	check("the same format is not a difference", !r.fmt_differs);

	{
		struct pm_plane big = plane_(0x3c000000ull, 5120u, "XRGB8888");

		big.w = 1920u;
		big.h = 1080u;
		r = pm_compare(&big, &b, &off);
		check("another size is still a yes", r.verdict == PM_YES);
		check("and is recorded as a size difference", r.size_differs);
	}

	{
		struct pm_plane unknown = plane_(0x3c000000ull, 5120u, NULL);

		r = pm_compare(&unknown, &b, &off);
		check("an unlisted format name is still a yes", r.verdict == PM_YES);
		check("and no format difference is claimed", !r.fmt_differs);
	}

	/* ---- IOMMU4: is the comparison sound? -------------------------------- */

	pm_mmu_state(PM_MMMU_CTRL_ENABLE, PM_DEBUG_INFO_BYPASS, BYPASS_END_40G_256M, true, &mmu);
	check("an enabled MMU is enabled", mmu.enabled);
	check("the 40 GiB window is read out of the register", mmu.pass_below == APERTURE_40G);
	check("a DRAM address below it passes through", pm_addr_passes_through(&mmu, 0x3c000000ull));
	check("an address at the aperture does not", !pm_addr_passes_through(&mmu, APERTURE_40G));

	pm_mmu_state(PM_MMMU_CTRL_ENABLE, PM_DEBUG_INFO_BYPASS | PM_DEBUG_INFO_BYPASS_4M,
		    BYPASS_END_40G_4M, true, &mmu);
	check("the 4 MB unit names the same window", mmu.pass_below == APERTURE_40G);
	check("and is recorded as the 4 MB form", mmu.bypass_4m);

	pm_mmu_state(PM_MMMU_CTRL_ENABLE, PM_DEBUG_INFO_BYPASS, 0, true, &mmu);
	check("no bypass window means nothing passes through", mmu.pass_below == 0);
	check("so a DRAM address is not comparable", !pm_addr_passes_through(&mmu, 0x3c000000ull));

	r = pm_compare(&p, &b, &mmu);
	check("and the decision says not comparable", r.verdict == PM_NOT_COMPARABLE);
	check_str("with its reason", r.reason,
		  "not comparable: iommu4 leaves this address above its pass-through window, "
		  "so the list address may be an IOVA");

	pm_mmu_state(0, PM_DEBUG_INFO_BYPASS, 0, true, &mmu);
	check("a disabled MMU translates nothing", !mmu.enabled);
	check("so the address is comparable", pm_addr_passes_through(&mmu, 0x3c000000ull));
	check("and the decision is the plain one", pm_compare(&p, &b, &mmu).verdict == PM_YES);

	pm_mmu_state(0xffffffffu, 0xffffffffu, 0xffffffffu, true, &mmu);
	check("all ones reads enabled (the words are what they are)", mmu.enabled);
	check("and a bypass window (the probe prints the words with it)", mmu.pass_below > 0);

	pm_mmu_state(0, 0, 0, false, &mmu);
	check("unread words are not a class", !pm_addr_passes_through(&mmu, 0x3c000000ull));
	check("so the decision is not comparable", pm_compare(&p, &b, &mmu).verdict == PM_NOT_COMPARABLE);
	check("and a NULL mmu (no information at all) is the same not-comparable",
	      pm_compare(&p, &b, NULL).verdict == PM_NOT_COMPARABLE);

	/* ---- the verdict line verdicts.sh reads ------------------------------ */

	r = pm_compare(&p, &b, &off);
	(void)pm_format_line(&r, line, sizeof(line));
	check_has("a yes line", line, "u3 scanout-follows-mailbox: yes (list 0x3c000000 pitch 5120 ");
	check_has("and it carries both formats", line, "fmt XRGB8888)");

	{
		struct pm_buffer other = buf_(0x40000000ull);

		r = pm_compare(&p, &other, &off);
		(void)pm_format_line(&r, line, sizeof(line));
		check_has("a no line", line, "u3 scanout-follows-mailbox: no (address differs: list 0x3c000000");
		check_has("naming the buffer's address", line, "buffer 0x40000000 pitch 5120 1280x720");
	}

	pm_mmu_state(PM_MMMU_CTRL_ENABLE, PM_DEBUG_INFO_BYPASS, 0, true, &mmu);
	r = pm_compare(&p, &b, &mmu);
	(void)pm_format_line(&r, line, sizeof(line));
	check_has("not comparable prints no", line, "u3 scanout-follows-mailbox: no (not comparable:");
	check_has("with the reason", line, "may be an IOVA");

	{
		int need = pm_format_line(&r, line, sizeof(line));
		char small[16];
		int got = pm_format_line(&r, small, sizeof(small));

		check("the length returned is what the text needs", got == need && need > (int)sizeof(small));
		check("a too-small buffer is still NUL terminated", small[15] == '\0');
		check("and holds the line's start", strncmp(small, "u3 scanout-", 11) == 0);
		check("a NULL result writes nothing", pm_format_line(NULL, line, sizeof(line)) == 0);
	}

	/* ---- 40-bit addresses ------------------------------------------------ */

	{
		struct pm_plane high = plane_(0x100000000ull + 0x3c000000ull, 5120u, "XRGB8888");
		struct pm_buffer hb = buf_(0x100000000ull + 0x3c000000ull);

		r = pm_compare(&high, &hb, &off);
		check("a 40-bit address compares whole", r.verdict == PM_YES);
		(void)pm_format_line(&r, line, sizeof(line));
		check_has("and prints whole", line, "list 0x13c000000");
		check_has("the buffer's too", line, "buffer 0x13c000000");

		hb.addr = 0x100000000ull + 0x40000000ull;
		check("and a high bit is not dropped by the compare",
		      pm_compare(&high, &hb, &off).verdict == PM_NO);
	}

	/* ---- which comparison the one decision line names ---------------------- */

	{
		/* Three lists read after the allocation: ch0 names another buffer, ch1 names
		 * nothing decodable, ch2 names the buffer. */
		struct pm_plane other = plane_(0x40000000ull, 5120u, "XRGB8888");
		struct pm_plane high = plane_(APERTURE_40G + 0x1000ull, 5120u, "XRGB8888");
		struct pm_mmu live;
		struct pm_result res[3];
		bool have[3];
		unsigned int chosen = 9u;

		pm_mmu_state(PM_MMMU_CTRL_ENABLE, 0, BYPASS_END_40G_256M, true, &live);
		res[0] = pm_compare(&other, &b, &off);
		res[1] = pm_compare(NULL, &b, &off);
		res[2] = pm_compare(&p, &b, &off);
		have[0] = true;
		have[1] = false;
		have[2] = true;

		r = pm_choose(res, have, 3u, 1u, &b, &chosen);
		check("a yes on any list is the answer", r.verdict == PM_YES);
		check("and its channel is named", chosen == 2u);
		check("with its plane carried", r.plane == &p);

		have[1] = true; /* the preferred channel has a comparison of its own now */
		res[1] = pm_compare(&other, &b, &off);
		r = pm_choose(res, have, 3u, 0u, &b, &chosen);
		check("the first yes wins over a preferred slot", r.verdict == PM_YES && chosen == 2u);

		/* No yes anywhere: the running channel's comparison is the one, even when it is
		 * the IOMMU's open answer and another channel's is a plain no. */
		res[0] = pm_compare(&other, &b, &off);
		res[1] = pm_compare(&high, &b, &live);
		have[2] = false;
		r = pm_choose(res, have, 3u, 1u, &b, &chosen);
		check("the running channel's open answer outranks another channel's no",
		      r.verdict == PM_NOT_COMPARABLE && chosen == 1u);

		/* No running channel (`preferred` past the last slot): a plain no first. */
		r = pm_choose(res, have, 3u, 3u, &b, &chosen);
		check("without a running channel a plain no is named", r.verdict == PM_NO && chosen == 0u);
		check_str("with its reason", r.reason, "address differs");

		res[0] = pm_compare(&high, &b, &live);
		r = pm_choose(res, have, 3u, 3u, &b, &chosen);
		check("when only open answers are left the first is named",
		      r.verdict == PM_NOT_COMPARABLE && chosen == 0u);

		have[0] = false;
		have[1] = false;
		r = pm_choose(res, have, 3u, 3u, &b, &chosen);
		check("no candidate at all is a no", r.verdict == PM_NO);
		check("and no channel is named", chosen == 3u);
		check_str("with the missing side named in pm_compare's words", r.reason,
			  "no plane in the display list");
		check("the buffer carried", r.buffer == &b);
		check("no plane on such a result", r.plane == NULL);

		r = pm_choose(NULL, NULL, 0u, 0u, &b, &chosen);
		check("an empty set is a no, not a crash", r.verdict == PM_NO && chosen == 0u);
	}

	printf("plane_match_test: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}
