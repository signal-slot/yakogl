/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The CPU picture on HDMI0 as the NanoPC-T6 display probes show and check
 * it (t6_picture.c; driver/docs/nanopc-t6-display.md, section 5): the
 * frame buffer with the pattern, the mappings, the hold, the read-backs and
 * the PASS or FAIL line (the read-backs are the board glue's,
 * t6_display_read_back). t6_display (step D2, the adopt) and t6_cold (step
 * D8, the cold bring-up) differ only in how the picture gets on the screen.
 * Each line starts with "<probe>: ".
 */
#ifndef T6_PICTURE_H
#define T6_PICTURE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include <yakogl_drivers.h>

/* The buffer both probes draw and show at the screen's origin
 * (board::nanopc_t6_display::FB_WIDTH, FB_HEIGHT): the probes' size, not
 * the mode's. On a larger screen the rest is VP0's background. */
#define T6_PICTURE_WIDTH  1024
#define T6_PICTURE_HEIGHT 768
#define T6_PICTURE_STRIDE (T6_PICTURE_WIDTH * 4)

/* The picture is held this long for the person at the display (or the
 * capture unit): the seconds of the build option T6_PICTURE_HOLD_S,
 * t6_picture.cmake, 30 by default. t6_picture.c checks it against the
 * watchdog's lifetime. */
#ifndef T6_PICTURE_HOLD_S
#error "T6_PICTURE_HOLD_S is the build's (include common/t6_picture.cmake)"
#endif
/* The VOP's registers below 0x2000 are the only ones the probes use. The
 * physical bases are yakogl_drivers.h's YD_T6_DISPLAY_*_PHYS: HDMI0's. */
#define T6_PICTURE_VOP_MAP 0x2000

/* Draw the section 5 pattern (red, green / blue, yellow quadrants, a white
 * 1-pixel border) into the image's frame buffer (in its .bss, so below
 * 4 GiB and PA = VA), clean it to the point of coherency and print where it
 * is. Returns its physical address, or 0 after a FAIL line. */
uint64_t t6_picture_prepare(const char *probe);

/* Map the seven blocks of struct yd_t6_display_bases: the CRU with
 * cru_bytes, the HDMI QP TX0 with hdmi_bytes, the VOP with
 * T6_PICTURE_VOP_MAP, the others a page each. */
void t6_picture_map(struct yd_t6_display_bases *b, size_t cru_bytes, size_t hdmi_bytes);

/* Hold the picture for T6_PICTURE_HOLD_S seconds, the watchdog fed, with a
 * line first and one every 10 s. */
void t6_picture_hold(const char *probe);

/* The PASS line when the read-backs show the picture (a frame start, no
 * underflow, no bus error, Esmart0 on fb), else the FAIL line. */
bool t6_picture_verdict(const char *probe, int rc, const struct yd_t6_display_status *s,
			uint32_t fb);

#endif /* T6_PICTURE_H */
