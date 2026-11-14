/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pv_hdmi_mock: driver/zephyr/lib/board/rpi5_scanout/pv_hdmi.h against
 * register vectors built here from the field layout (independently of the
 * header's masks): 1920x1080 and 1280x720 CEA timings, a disabled pixel
 * valve, a disabled HDMI0, and the region addresses.
 *
 * Prints "pv_hdmi_mock: N of N checks, 0 failed"; exit 0 when none failed.
 */
#include <stdio.h>

#include "rpi5_scanout/pv_hdmi.h"

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

#define PACK(hi, lo) ((((uint32_t)(hi)) << 16) | (uint32_t)(lo))

static void timing(const char *name, uint32_t ha, uint32_t hfp, uint32_t hs, uint32_t hbp,
		   uint32_t va, uint32_t vfp, uint32_t vs, uint32_t vbp)
{
	struct pv_timing t = pv_decode_timing(PACK(hbp, hs), PACK(hfp, ha), PACK(vbp, vs),
					      PACK(vfp, va));

	CHECK(t.hactive == ha && t.vactive == va, "%s: active %ux%u", name, t.hactive, t.vactive);
	CHECK(t.hsync == hs && t.hbp == hbp && t.hfp == hfp, "%s: h porches %u/%u/%u", name,
	      t.hsync, t.hbp, t.hfp);
	CHECK(t.vsync == vs && t.vbp == vbp && t.vfp == vfp, "%s: v porches %u/%u/%u", name,
	      t.vsync, t.vbp, t.vfp);
	CHECK(t.htotal == ha + hfp + hs + hbp && t.vtotal == va + vfp + vs + vbp,
	      "%s: totals %ux%u", name, t.htotal, t.vtotal);
	CHECK(pv_timing_valid(&t), "%s: not valid", name);
}

int main(void)
{
	/* 1. region addresses */
	CHECK(PV0_BASE == 0x107c410000ull && PV0_SIZE == 0x100, "PV0");
	CHECK(PV1_BASE == 0x107c411000ull && PV1_SIZE == 0x100, "PV1");
	CHECK(HDMI0_HDMI_BASE == 0x107c701400ull && HDMI0_HDMI_SIZE == 0x300, "HDMI0 hdmi");
	CHECK(HDMI0_HD_BASE == 0x107c720000ull && HDMI0_HD_SIZE == 0x100, "HDMI0 hd");
	CHECK(HDMI0_REG_NAME_HDMI_IDX == 0 && HDMI0_REG_NAME_HD_IDX == 8, "reg-names order");
	CHECK(PV_CONTROL == 0x00 && PV_V_CONTROL == 0x04 && PV_HORZA == 0x0c && PV_HORZB == 0x10 &&
		      PV_VERTA == 0x14 && PV_VERTB == 0x18,
	      "PV offsets");

	/* 2. timings: 1080p60 (2200x1125) and 720p60 (1650x750) */
	timing("1080p", 1920, 88, 44, 148, 1080, 4, 5, 36);
	timing("720p", 1280, 110, 40, 220, 720, 5, 5, 20);
	{
		struct pv_timing t = pv_decode_timing(PACK(148, 44), PACK(88, 1920), PACK(36, 5),
						      PACK(4, 1080));
		CHECK(t.htotal == 2200 && t.vtotal == 1125, "1080p totals %ux%u", t.htotal, t.vtotal);
		t = pv_decode_timing(PACK(220, 40), PACK(110, 1280), PACK(20, 5), PACK(5, 720));
		CHECK(t.htotal == 1650 && t.vtotal == 750, "720p totals %ux%u", t.htotal, t.vtotal);
	}
	/* 16-bit extremes do not leak across the two fields */
	{
		struct pv_timing t = pv_decode_timing(0xffff0000u, 0x0000ffffu, 0x0001ffffu,
						      0xfffe0001u);
		CHECK(t.hbp == 0xffff && t.hsync == 0 && t.hfp == 0 && t.hactive == 0xffff,
		      "h field split");
		CHECK(t.vbp == 1 && t.vsync == 0xffff && t.vfp == 0xfffe && t.vactive == 1,
		      "v field split");
	}

	/* 3. control: enabled, HDMI clock select, 24 bit, FIFO level 0x10,
	 * clear-at-start, wait-hstart, trigger-underflow */
	{
		uint32_t v = 1u | (1u << 2) | (0x10u << 15) | (1u << 14) | (1u << 12) | (1u << 13);
		struct pv_control c = pv_decode_control(v);

		CHECK(c.enabled && !c.fifo_clr, "enable bits");
		CHECK(c.clk_select == 1 && c.pixel_rep == 0 && c.format == PV_CONTROL_FORMAT_24,
		      "clk %u rep %u fmt %u", c.clk_select, c.pixel_rep, c.format);
		CHECK(c.fifo_level == 0x10 && c.fifo_level_high == 0, "fifo level %u/%u",
		      c.fifo_level, c.fifo_level_high);
		CHECK(c.clr_at_start && c.wait_hstart && c.trigger_underflow, "flag bits");
		v = (4u << 21) | (3u << 25) | (2u << 4) | 2u;
		c = pv_decode_control(v);
		CHECK(!c.enabled && c.fifo_clr && c.format == PV_CONTROL_FORMAT_DSIV_24 &&
			      c.fifo_level_high == 3 && c.pixel_rep == 2,
		      "format/high-level/rep fields");
	}

	/* 4. PV disabled: all zero; running needs EN and VIDEN */
	{
		struct pv_control c = pv_decode_control(0);
		struct pv_vcontrol vc = pv_decode_vcontrol(0);
		struct pv_timing t = pv_decode_timing(0, 0, 0, 0);

		CHECK(!c.enabled && !vc.viden && !pv_timing_valid(&t) && t.htotal == 0,
		      "zero state decodes as off");
		CHECK(!pv_running(0, 0), "zero running");
		CHECK(!pv_running(PV_CONTROL_EN, 0), "EN without VIDEN running");
		CHECK(!pv_running(0, PV_VCONTROL_VIDEN), "VIDEN without EN running");
		CHECK(pv_running(PV_CONTROL_EN, PV_VCONTROL_VIDEN | PV_VCONTROL_CONTINUOUS),
		      "EN and VIDEN not running");
	}

	/* 5. PV_V_CONTROL bits */
	{
		struct pv_vcontrol vc = pv_decode_vcontrol(0x13);

		CHECK(vc.viden && vc.continuous && vc.interlace && !vc.command && !vc.dsi,
		      "vcontrol 0x13");
		vc = pv_decode_vcontrol(0x0c);
		CHECK(!vc.viden && vc.command && vc.dsi && !vc.interlace, "vcontrol 0x0c");
	}

	/* 6. HDMI0 HD_VID_CTL: at 0x44 in the hd region, bit 31 only */
	CHECK(HD_VID_CTL == 0x44 && HD_VID_CTL < HDMI0_HD_SIZE, "HD_VID_CTL offset");
	CHECK(hd_vid_ctl_enabled(0x80000000u), "bit 31 set");
	CHECK(hd_vid_ctl_enabled(0xffffffffu), "all set");
	CHECK(!hd_vid_ctl_enabled(0), "HDMI disabled (zero)");
	CHECK(!hd_vid_ctl_enabled(0x7fffffffu), "every bit but 31");

	printf("pv_hdmi_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed != 0;
}
