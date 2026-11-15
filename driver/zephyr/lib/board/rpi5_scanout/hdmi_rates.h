/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hdmi_rates.h: the HDMI clock rates the BCM2712 (Pi 5) HDMI0 block needs for
 * one mode, header-only C11, integer arithmetic only (uint64_t Hz).
 *
 * Facts: driver/docs/rpi5-v3d71-facts.md section 10 (the rows on the HDMI clock
 * requests, the BCM2712 HDMI variants and vc4_hvs_bind). Linux sources are
 * cited as file:line facts only, no code is copied:
 *   - HSM ("hdmi" firmware clock) minimum: the larger of 120 MHz and 101% of
 *     the TMDS character rate: vc4_hdmi.c:1523-1526, HSM_MIN_CLOCK_FREQ at :112.
 *   - BVB clock: 300 MHz above a 297 MHz pixel clock, 150 MHz above 148.5 MHz,
 *     else 75 MHz: vc4_hdmi.c:1546-1553.
 *   - max_pixel_clock of the BCM2712 variants is 600 MHz: vc4_hdmi.c:3538-3562.
 *   - HDMI 2.0 needs a core clock of at least 550 MHz, 4096x2160 at least
 *     600 MHz: vc4_hvs.c:2112-2135.
 * The TMDS character rate is the pixel clock scaled by bpc / 8 (HDMI 1.4
 * deep colour); above 340 MHz the link is HDMI 2.0 (scrambling, 1/40 ratio).
 */
#ifndef HDMI_RATES_H
#define HDMI_RATES_H

#include <stdint.h>

#define HDMI_RATES_HSM_MIN_HZ 120000000ULL
#define HDMI_RATES_BVB_LOW_HZ 75000000ULL
#define HDMI_RATES_BVB_MID_HZ 150000000ULL
#define HDMI_RATES_BVB_HIGH_HZ 300000000ULL
#define HDMI_RATES_BVB_MID_PIXEL_HZ 148500000ULL
#define HDMI_RATES_BVB_HIGH_PIXEL_HZ 297000000ULL
#define HDMI_RATES_MAX_PIXEL_HZ 600000000ULL
#define HDMI_RATES_HDMI20_TMDS_HZ 340000000ULL /* above: HDMI 2.0 link */
#define HDMI_RATES_CORE_HDMI20_HZ 550000000ULL
#define HDMI_RATES_CORE_4K_HZ 600000000ULL

/* verdict flags; 0 is ok */
#define HDMI_RATES_OK 0u
#define HDMI_RATES_OVER_MAX_PIXEL 0x1u /* pixel clock > 600 MHz */
#define HDMI_RATES_NEEDS_HDMI20 0x2u   /* TMDS rate > 340 MHz */
#define HDMI_RATES_CORE_TOO_LOW 0x4u   /* measured core < what the mode needs */
#define HDMI_RATES_BAD_ARGS 0x8u       /* pixel 0 or bpc not 8/10/12/16 */

struct hdmi_rates {
	uint64_t tmds_hz; /* TMDS character rate */
	uint64_t hsm_min_hz;
	uint64_t bvb_hz;
	uint64_t core_min_hz; /* 0 when the mode puts no demand on the core */
	uint32_t verdict;     /* HDMI_RATES_* flags */
};

/* Core clock the mode needs: 600 MHz for 4096 wide and up (4096x2160),
 * 550 MHz for an HDMI 2.0 link, else 0 (no requirement). */
static inline uint64_t hdmi_rates_core_min(uint64_t tmds_hz, uint32_t hactive)
{
	if (hactive >= 4096u)
		return HDMI_RATES_CORE_4K_HZ;
	return tmds_hz > HDMI_RATES_HDMI20_TMDS_HZ ? HDMI_RATES_CORE_HDMI20_HZ : 0;
}

/*
 * pixel_hz: pixel clock in Hz; bpc: bits per component (8, 10, 12, 16);
 * core_hz: measured core clock in Hz, 0 when unknown (no core verdict);
 * hactive: active width for the 4096x2160 rule, 0 when unknown.
 */
static inline struct hdmi_rates hdmi_rates_for_mode(uint64_t pixel_hz, uint32_t bpc,
						    uint64_t core_hz, uint32_t hactive)
{
	struct hdmi_rates r = { 0, 0, 0, 0, HDMI_RATES_OK };
	uint64_t hsm101;

	if (pixel_hz == 0 || (bpc != 8 && bpc != 10 && bpc != 12 && bpc != 16)) {
		r.verdict = HDMI_RATES_BAD_ARGS;
		return r;
	}
	r.tmds_hz = pixel_hz * bpc / 8u;
	hsm101 = (r.tmds_hz * 101u + 99u) / 100u; /* 101%, rounded up */
	r.hsm_min_hz = hsm101 > HDMI_RATES_HSM_MIN_HZ ? hsm101 : HDMI_RATES_HSM_MIN_HZ;
	if (pixel_hz > HDMI_RATES_BVB_HIGH_PIXEL_HZ)
		r.bvb_hz = HDMI_RATES_BVB_HIGH_HZ;
	else if (pixel_hz > HDMI_RATES_BVB_MID_PIXEL_HZ)
		r.bvb_hz = HDMI_RATES_BVB_MID_HZ;
	else
		r.bvb_hz = HDMI_RATES_BVB_LOW_HZ;
	r.core_min_hz = hdmi_rates_core_min(r.tmds_hz, hactive);
	if (pixel_hz > HDMI_RATES_MAX_PIXEL_HZ)
		r.verdict |= HDMI_RATES_OVER_MAX_PIXEL;
	if (r.tmds_hz > HDMI_RATES_HDMI20_TMDS_HZ)
		r.verdict |= HDMI_RATES_NEEDS_HDMI20;
	if (core_hz != 0 && r.core_min_hz != 0 && core_hz < r.core_min_hz)
		r.verdict |= HDMI_RATES_CORE_TOO_LOW;
	return r;
}

#endif /* HDMI_RATES_H */
