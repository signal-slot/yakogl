/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pv_timing_mock: driver/zephyr/lib/board/rpi5_scanout/pv_timing.h against
 * register values written here as literals (computed by hand from the CEA-861
 * timings and the Linux rule), decoded with pv_hdmi.h; the mismatch reporter
 * and the mode-not-found error.
 *
 * Prints "pv_timing_mock: N of N checks, 0 failed"; exit 0 when none failed.
 */
#include <stdio.h>
#include <string.h>

#include "rpi5_scanout/pv_timing.h"

static int checks, failed;

#define CHECK(cond, ...)                                                                           \
	do {                                                                                       \
		checks++;                                                                          \
		if (!(cond)) {                                                                     \
			failed++;                                                                  \
			printf("FAIL line %d: ", __LINE__);                                        \
			printf(__VA_ARGS__);                                                       \
			printf("\n");                                                              \
		}                                                                                  \
	} while (0)

struct want {
	uint8_t vic;
	uint32_t clk_khz, horza, horzb, verta, vertb;
	uint32_t ha, hfp, hs, hbp, htot, va, vfp, vs, vbp, vtot;
	bool hpos, vpos;
};

static const struct want wants[] = {
	{1, 25175, 0x00300060, 0x00100280, 0x00210002, 0x000a01e0, 640, 16, 96, 48, 800, 480, 10, 2,
	 33, 525, false, false},
	{4, 74250, 0x00dc0028, 0x006e0500, 0x00140005, 0x000502d0, 1280, 110, 40, 220, 1650, 720, 5,
	 5, 20, 750, true, true},
	{16, 148500, 0x0094002c, 0x00580780, 0x00240005, 0x00040438, 1920, 88, 44, 148, 2200, 1080,
	 4, 5, 36, 1125, true, true},
};

#define BASE_CONTROL 0x00007002u /* FIFO_CLR | WAIT_HSTART | TRIGGER_UNDERFLOW | CLR_AT_START */
#define BASE_VCONTROL 0x2u       /* CONTINUOUS */

int main(void)
{
	char buf[64];

	CHECK(PVT_NUM_MODES == 3, "mode count %zu", PVT_NUM_MODES);

	for (size_t i = 0; i < sizeof(wants) / sizeof(wants[0]); i++) {
		const struct want *w = &wants[i];
		struct pvt_regs r;
		struct pv_timing t;
		struct pv_control c;
		struct pv_vcontrol v;
		const struct pvt_mode *m = pvt_find_vic(w->vic);
		uint32_t run_c, run_v;

		CHECK(m != NULL, "vic %u not found", w->vic);
		if (m == NULL) {
			continue;
		}
		CHECK(m->pixel_clock_khz == w->clk_khz, "vic %u clock %u", w->vic,
		      m->pixel_clock_khz);
		CHECK(pvt_regs_for_vic(w->vic, &r) == PVT_OK, "vic %u fill", w->vic);
		CHECK(r.horza == w->horza && r.horzb == w->horzb, "vic %u HORZ %08x %08x", w->vic,
		      r.horza, r.horzb);
		CHECK(r.verta == w->verta && r.vertb == w->vertb, "vic %u VERT %08x %08x", w->vic,
		      r.verta, r.vertb);
		CHECK(r.control == BASE_CONTROL && r.vcontrol == BASE_VCONTROL,
		      "vic %u control %08x vcontrol %08x", w->vic, r.control, r.vcontrol);
		CHECK(r.hsync_positive == w->hpos && r.vsync_positive == w->vpos,
		      "vic %u polarity", w->vic);

		t = pv_decode_timing(r.horza, r.horzb, r.verta, r.vertb);
		CHECK(t.hactive == w->ha && t.hfp == w->hfp && t.hsync == w->hs &&
			      t.hbp == w->hbp && t.htotal == w->htot,
		      "vic %u decoded h", w->vic);
		CHECK(t.vactive == w->va && t.vfp == w->vfp && t.vsync == w->vs &&
			      t.vbp == w->vbp && t.vtotal == w->vtot,
		      "vic %u decoded v", w->vic);
		CHECK(pv_timing_valid(&t), "vic %u valid", w->vic);
		/* 60 Hz within 2 %: clock = htotal * vtotal * rate */
		CHECK((uint64_t)w->htot * w->vtot * 59 < (uint64_t)w->clk_khz * 1000 &&
			      (uint64_t)w->htot * w->vtot * 61 > (uint64_t)w->clk_khz * 1000,
		      "vic %u refresh", w->vic);

		c = pv_decode_control(r.control);
		CHECK(c.format == PV_CONTROL_FORMAT_24 && c.pixel_rep == 0 && c.clr_at_start &&
			      c.wait_hstart && c.trigger_underflow && c.fifo_clr && !c.enabled,
		      "vic %u control decode", w->vic);
		v = pv_decode_vcontrol(r.vcontrol);
		CHECK(v.continuous && !v.interlace && !v.dsi && !v.command && !v.viden,
		      "vic %u vcontrol decode", w->vic);

		/* a running snapshot (EN, VIDEN, FIFO level and clock select
		 * set, FIFO_CLR clear) equals the mode */
		run_c = (r.control & ~PV_CONTROL_FIFO_CLR) | PV_CONTROL_EN |
			(1u << PV_CONTROL_CLK_SELECT_SHIFT) | (21u << PV_CONTROL_FIFO_LEVEL_SHIFT);
		run_v = r.vcontrol | PV_VCONTROL_VIDEN;
		CHECK(pvt_compare(&r, run_c, run_v, r.horza, r.horzb, r.verta, r.vertb) == 0,
		      "vic %u running snapshot differs", w->vic);
		CHECK(pvt_match_vic(run_c, run_v, r.horza, r.horzb, r.verta, r.vertb) == w->vic,
		      "vic %u match", w->vic);
	}

	/* mode not found and bad argument */
	{
		struct pvt_regs r;

		memset(&r, 0xaa, sizeof(r));
		CHECK(pvt_find_vic(0) == NULL && pvt_find_vic(2) == NULL && pvt_find_vic(255) == NULL,
		      "unknown vic found");
		CHECK(pvt_regs_for_vic(2, &r) == PVT_ERR_NOT_FOUND, "vic 2 error");
		CHECK(r.horza == 0xaaaaaaaau, "not-found error touched the output");
		CHECK(pvt_regs_for_vic(4, NULL) == PVT_ERR_ARG, "NULL out");
	}

	/* mismatch reporter: 1080p against 720p, then one field at a time */
	{
		struct pvt_regs r;
		uint32_t d;

		pvt_regs_for_vic(16, &r);
		CHECK(pvt_compare(&r, r.control, r.vcontrol, r.horza, r.horzb, r.verta, r.vertb) ==
			      0,
		      "self compare");

		d = pvt_compare(&r, r.control, r.vcontrol, 0x00dc0028, 0x006e0500, 0x00140005,
				0x000502d0);
		/* vsync is 5 in both */
		CHECK(d == (PVT_DIFF_HACTIVE | PVT_DIFF_HFP | PVT_DIFF_HSYNC | PVT_DIFF_HBP |
			    PVT_DIFF_VACTIVE | PVT_DIFF_VFP | PVT_DIFF_VBP),
		      "1080p vs 720p diff %x", d);
		CHECK(pvt_diff_str(d, buf, sizeof(buf)) > 0 &&
			      strcmp(buf, "hactive hfp hsync hbp vactive vfp vbp") == 0,
		      "names '%s'", buf);

		d = pvt_compare(&r, r.control, r.vcontrol, r.horza, r.horzb, r.verta, r.vertb + 1);
		CHECK(d == PVT_DIFF_VACTIVE, "vactive only %x", d);
		d = pvt_compare(&r, r.control, r.vcontrol, r.horza + (1u << 16), r.horzb, r.verta,
				r.vertb);
		CHECK(d == PVT_DIFF_HBP, "hbp only %x", d);
		d = pvt_compare(&r,
				r.control | (4u << PV_CONTROL_FORMAT_SHIFT) |
					(1u << PV_CONTROL_PIXEL_REP_SHIFT),
				r.vcontrol | PV_VCONTROL_INTERLACE, r.horza, r.horzb, r.verta,
				r.vertb);
		CHECK(d == (PVT_DIFF_FORMAT | PVT_DIFF_PIXEL_REP | PVT_DIFF_INTERLACE), "ctl %x", d);
		d = pvt_compare(&r, r.control, 0, r.horza, r.horzb, r.verta, r.vertb);
		CHECK(d == PVT_DIFF_CONTINUOUS, "continuous %x", d);

		/* a pixel valve the firmware never set up reads zero */
		d = pvt_compare(&r, 0, 0, 0, 0, 0, 0);
		CHECK((d & 0xff) == 0xff && pvt_match_vic(0, 0, 0, 0, 0, 0) == 0,
		      "zero snapshot %x", d);

		CHECK(pvt_match_vic(r.control, r.vcontrol, r.horza, r.horzb, r.verta,
				    r.vertb + 1) == 0,
		      "near miss matched");
	}

	/* diff string edge cases */
	CHECK(pvt_diff_str(0, buf, sizeof(buf)) == 4 && strcmp(buf, "none") == 0, "none");
	CHECK(pvt_diff_str(PVT_DIFF_VFP, buf, sizeof(buf)) == 3 && strcmp(buf, "vfp") == 0, "vfp");
	CHECK(pvt_diff_str(PVT_DIFF_HFP | PVT_DIFF_HBP, buf, 7) == -1 && strcmp(buf, "hfp") == 0,
	      "truncated '%s'", buf);
	CHECK(pvt_diff_str(PVT_DIFF_HFP, buf, 4) == 3, "exact fit");
	CHECK(pvt_diff_str(PVT_DIFF_HFP, buf, 3) == -1, "one short");
	CHECK(pvt_diff_str(1, buf, 0) == -1, "n = 0");

	if (failed) {
		printf("pv_timing_mock: %d of %d checks failed\n", failed, checks);
		return 1;
	}
	printf("pv_timing_mock: %d of %d checks, 0 failed\n", checks, checks);
	return 0;
}
