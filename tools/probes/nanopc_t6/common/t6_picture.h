/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The CPU picture on HDMI0 as the NanoPC-T6 display probes show and check
 * it (t6_picture.c; driver/docs/nanopc-t6-display.md, section 5): the
 * frame buffer with the pattern, the mappings, the hold, the read-backs and
 * the PASS or FAIL line. t6_display (step D2, the adopt) and t6_cold (step
 * D8, the cold bring-up) differ only in how the picture gets on the screen.
 * Each line starts with "<probe>: ".
 */
#ifndef T6_PICTURE_H
#define T6_PICTURE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include <yakogl_drivers.h>

/* The mode both probes show (board::nanopc_t6_display::MODE). */
#define T6_PICTURE_WIDTH  1024
#define T6_PICTURE_HEIGHT 768
#define T6_PICTURE_STRIDE (T6_PICTURE_WIDTH * 4)
#define T6_PICTURE_HTOTAL 1344
#define T6_PICTURE_VTOTAL 806

/* The picture is held this long for the person at the display. */
#define T6_PICTURE_HOLD_S 30
/* The read-backs: the frame check's wait, the frames timed. */
#define T6_PICTURE_CHECK_WAIT_US 100000
#define T6_PICTURE_TIMED_FRAMES  60

/* The physical bases (driver/docs/nanopc-t6-display.md, section 1). */
#define T6_PICTURE_VOP_PHYS     0xfdd90000UL
#define T6_PICTURE_VOP_MAP      0x2000
#define T6_PICTURE_VOP_MMU_PHYS 0xfdd97000UL /* the two sets at +0xe00, +0xf00 */
#define T6_PICTURE_PMU_PHYS     0xfd8d8000UL
#define T6_PICTURE_CRU_PHYS     0xfd7c0000UL
#define T6_PICTURE_SYS_GRF_PHYS 0xfd58c000UL
#define T6_PICTURE_PHY_GRF_PHYS 0xfd5e0000UL
#define T6_PICTURE_HDMI_PHYS    0xfde80000UL

/* Draw the section 5 pattern (red, green / blue, yellow quadrants, a white
 * 1-pixel border) into the image's frame buffer (in its .bss, so below
 * 4 GiB and PA = VA), clean it to the point of coherency and print where it
 * is. Returns its physical address, or 0 after a FAIL line. */
uint64_t t6_picture_prepare(const char *probe);

/* Map the seven blocks of struct yd_t6_display_bases: the CRU with
 * cru_bytes, the HDMI QP TX0 with hdmi_bytes, the VOP with
 * T6_PICTURE_VOP_MAP, the others a page each. */
void t6_picture_map(struct yd_t6_display_bases *b, size_t cru_bytes, size_t hdmi_bytes);

/* The callbacks the C entries take: k_busy_wait, and the uptime in us. */
void t6_picture_delay_us(uint32_t us);
uint64_t t6_picture_now_us(void);

/* Hold the picture for T6_PICTURE_HOLD_S seconds, the watchdog fed, with a
 * line first and one every 10 s. */
void t6_picture_hold(const char *probe);

/* The section 5 read-backs (yd_t6_display_status) into *s, printed. Returns
 * yd_t6_display_status's rc. */
int t6_picture_status(const char *probe, const struct yd_t6_display_bases *b,
		      struct yd_t6_display_status *s);

/* The PASS line when the read-backs show the picture (a frame start, no
 * underflow, no bus error, Esmart0 on fb), else the FAIL line. */
bool t6_picture_verdict(const char *probe, int rc, const struct yd_t6_display_status *s,
			uint32_t fb);

#endif /* T6_PICTURE_H */
