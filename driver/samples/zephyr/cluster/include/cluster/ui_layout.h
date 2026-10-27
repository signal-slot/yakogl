/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * One screen design, two owners, two panel sizes.
 *
 * The whole layout is decided here, once.  The platform core (R5F/M7) draws
 * only the safety-relevant elements of that design into its overlay plane;
 * the rich-UI application (A53) draws everything else around them.  Both
 * include this header so a value the safe core puts on screen sits exactly
 * where the application's design expects it -- there is no "safety band",
 * just one picture in which some pixels happen to come from the safe core.
 * When the application is gone the safe core fills the background itself and
 * the safety elements stay where they were; only the decoration disappears.
 *
 * The design is drawn once, for the 1024x600 AM62P panel, and every position
 * below is written as that design's number passed through UI_PX()/UI_PY().
 * On 1024x600 those are the identity, so the reference design is still
 * literally on the page; on the Verdin iMX8M Plus's 1280x800 DSI panel they
 * place the same picture over the whole panel.  There is one set of numbers,
 * so the two panels cannot drift apart -- and neither can the two owners,
 * because scripts/gen_layout_slint.py turns this header into the Slint
 * globals that both designs import (ui/<panel>/cluster-layout.slint).
 *
 * Fonts are the exception and are chosen per panel rather than scaled: the
 * glyphs are bitmaps pre-rendered at build time for the sizes named here, so
 * a size that is not on this list cannot be drawn sharply.
 *
 * Select the panel with CLUSTER_PANEL_1280X800 (or CONFIG_CLUSTER_PANEL_1280X800
 * from Kconfig); the default is the 1024x600 reference design.
 *
 * Angles: degrees, 0 at 12 o'clock, clockwise (see ui_draw.h).
 */
#ifndef CLUSTER_UI_LAYOUT_H_
#define CLUSTER_UI_LAYOUT_H_

#if defined(CLUSTER_PANEL_1280X800) || defined(CONFIG_CLUSTER_PANEL_1280X800)
#define UI_W 1280
#define UI_H 800
#else
#define UI_W 1024
#define UI_H 600
#endif

/* The size the design below is drawn at. */
#define UI_DESIGN_W 1024
#define UI_DESIGN_H 600

/*
 * A design coordinate, placed on this panel.
 *
 * UI_PY() is for vertical positions only.  Everything else -- horizontal
 * positions, and every length that must not distort a circle (radii, tick
 * lengths, box sizes) -- goes through UI_PX(), so a dial stays round.  The
 * two scales differ (1.25 across, 1.333 down on 1280x800), and using UI_PY()
 * for a radius is the one mistake that would show.
 */
#define UI_PX(v) (((v) * UI_W) / UI_DESIGN_W)
#define UI_PY(v) (((v) * UI_H) / UI_DESIGN_H)

/* palette (ARGB) */
#define UI_COL_BG_TOP   0xFF0B1018
#define UI_COL_BG_BOT   0xFF04060A
#define UI_COL_TEXT     0xFFF2F5F8
#define UI_COL_TEXT2    0xFFA8B2BD
#define UI_COL_DIM      0xFF6B7078
#define UI_COL_HAIR     0x40FFFFFF
#define UI_COL_ACCENT   0xFF6EA8FF
#define UI_COL_GEAR     0xFF7CB0FF
#define UI_COL_GREEN    0xFF4ADE80
#define UI_COL_AMBER    0xFFFFB02E
#define UI_COL_RED      0xFFFF4438
#define UI_COL_TRACK    0xFF141A22

/* ------------------------------------------------------------------ fonts */
/*
 * Noto Sans, pre-rendered at build time.  Picked per panel, not scaled: 1.25x
 * of 16 px is 20 px only because 20 px is also a size that reads well, and
 * 1.25x of 108 px would be 135 px, which is not obviously the right size for
 * the speed digits just because arithmetic produced it.
 */
#if UI_H >= 800
#define UI_FS_SMALL     20       /* Regular */
#define UI_FS_LABEL     26       /* SemiBold */
#define UI_FS_MEDIUM    38       /* SemiBold */
#define UI_FS_BIG       60       /* SemiBold */
#define UI_FS_SPEED     140      /* Light */
#define UI_FS_GEAR      44       /* Light */
#define UI_FS_VITALS    72       /* SemiBold, medical monitor */
#else
#define UI_FS_SMALL     16
#define UI_FS_LABEL     20
#define UI_FS_MEDIUM    30
#define UI_FS_BIG       48
#define UI_FS_SPEED     108
#define UI_FS_GEAR      34
#define UI_FS_VITALS    56
#endif
#define UI_FW_REGULAR   400
#define UI_FW_SEMIBOLD  600
#define UI_FW_LIGHT     300

/* ---------------------------------------------------------------- cluster */
/* two dials, mirror-symmetric */
#define CL_LEFT_CX      UI_PX(214)
#define CL_RIGHT_CX     (UI_W - CL_LEFT_CX)
#define CL_DIAL_CY      UI_PY(330)
#define CL_DIAL_R       UI_PX(166)   /* centre radius of the arc band */
#define CL_DIAL_W       UI_PX(10)    /* band width */
#define CL_FACE_R       UI_PX(178)   /* flat dial face (application) under the band */
#define CL_TICK_R0      UI_PX(181)   /* graduations just outside the face */
#define CL_TICK_MINOR   UI_PX(10)
#define CL_TICK_MAJOR   UI_PX(18)
#define CL_NUM_R        UI_PX(132)   /* scale numbers */
#define CL_DIAL_A0      -140.0f
#define CL_DIAL_SWEEP   280.0f
#define CL_RPM_MAX      8000
#define CL_SPEED_MAX    240          /* km/h */

/* captions and read-outs on the dial faces, as offsets from the dial centre.
 * They belong to the circle, so they scale with it (UI_PX), not with UI_H.
 */
#define CL_DIAL_CAP_DY   UI_PX(-78)  /* "ENGINE" / "DRIVE INFO" */
#define CL_DIAL_UNIT_DY  UI_PX(8)    /* "x1000 rpm" */
#define CL_DIAL_VALUE_DY UI_PX(-52)  /* the big rpm number */
#define CL_DIAL_STATE_DY UI_PX(62)   /* "RUNNING" / "OFF" */
#define CL_TRIP_CAP_DY   UI_PX(-48)  /* "TRIP" over the right dial */
#define CL_TRIP_VAL_DY   UI_PX(-28)
#define CL_ODO_CAP_DY    UI_PX(18)
#define CL_ODO_VAL_DY    UI_PX(38)

/* telltale row: 10 slots centred on the screen; safety-owned slots are drawn
 * by the safe core, the others by the application
 */
#define CL_TT_Y         UI_PY(14)
#define CL_TT_H         UI_PY(40)
#define CL_TT_SLOT_W    UI_PX(44)
#define CL_TT_SLOTS     10
#define CL_TT_X0        (UI_W / 2 - CL_TT_SLOTS * CL_TT_SLOT_W / 2)
#define CL_TT_X(i)      (CL_TT_X0 + (i) * CL_TT_SLOT_W)
#define CL_TT_TURN_L    0   /* safe core */
#define CL_TT_LIGHTS    1
#define CL_TT_PARK      2
#define CL_TT_ABS       3
#define CL_TT_ENGINE    4   /* safe core */
#define CL_TT_FUEL      5   /* safe core */
#define CL_TT_OIL       6
#define CL_TT_BATTERY   7
#define CL_TT_SEATBELT  8
#define CL_TT_TURN_R    9   /* safe core */

/* status row under the telltales */
#define CL_STATUS_Y     UI_PY(66)
#define CL_STATUS_H     UI_PY(36)
#define CL_TIME_CX      UI_PX(428)       /* application */
#define CL_TIME_Y       UI_PY(72)
#define CL_GEAR_CX      (UI_W / 2)       /* safe core */
#define CL_GEAR_Y       UI_PY(64)
#define CL_READY_X      UI_PX(556)       /* application: READY box */
#define CL_READY_Y      UI_PY(72)
#define CL_READY_W      UI_PX(88)
#define CL_READY_H      UI_PY(26)
#define CL_READY_TEXT_Y UI_PY(75)
#define CL_HAIR_Y       UI_PY(110)
#define CL_HAIR_X0      UI_PX(372)
#define CL_HAIR_X1      UI_PX(652)

/* speed read-out (safe core): digits centred, unit below */
#define CL_SPEED_CX     (UI_W / 2)
#define CL_SPEED_TOP    UI_PY(112)
#define CL_SPEED_UNIT_Y UI_PY(240)

/* the Slint logo between the dials, under the speed (application) */
#define CL_LOGO_X       UI_PX(423)
#define CL_LOGO_Y       UI_PY(303)
#define CL_LOGO_W       UI_PX(168)
#define CL_LOGO_H       UI_PX(67)

/* centre panel (application): trip / odo / range rows */
#define CL_INFO_X0      UI_PX(412)
#define CL_INFO_X1      UI_PX(612)
#define CL_INFO_Y       UI_PY(452)       /* just above the brand, near the bottom */
#define CL_INFO_ROW_H   UI_PY(30)

/* bottom: fuel (safe core: segments + litres; application: frame + pump icon) */
#define CL_FUEL_BAR_X   UI_PX(760)
#define CL_FUEL_BAR_Y   UI_PY(546)
#define CL_FUEL_BAR_W   UI_PX(240)
#define CL_FUEL_BAR_H   UI_PY(20)
#define CL_FUEL_SEGS    5
#define CL_FUEL_SEG_W   UI_PX(44)
#define CL_FUEL_SEG_DX  UI_PX(49)        /* pitch: 44 wide plus a 5 px gap */
#define CL_FUEL_TEXT_RX UI_PX(744)       /* litres, right-aligned */
#define CL_FUEL_TEXT_Y  UI_PY(540)
/*
 * The pump icon sits to the left of the litres, and the space between them is
 * clearance for a string, so it scales with the FONT and not with the panel.
 * That distinction is not pedantic: the reference design put the icon at
 * UI_PX(664), which left exactly one pixel between the icon and "27.5 L" at
 * 20 px -- and on 1280x800 the label face grows 1.30x while UI_PX grows 1.25x,
 * so the same numbers put the text six pixels INTO the icon. The safe core
 * draws the litres and the application draws the icon, so that is not an ugly
 * overlap, it is two cores writing the same pixels.
 *
 * Placing the icon from the text's own width instead fixes both panels with
 * one rule. CL_FUEL_TEXT_W is the widest the string gets ("45.0 L"), measured
 * on the host renders: 52 px at the 20 px face, 71 px at the 26 px one, so 2.8
 * times the face is the bound with a little to spare.
 */
#define CL_FUEL_TEXT_W  (UI_FS_LABEL * 28 / 10)
#define CL_FUEL_ICON_W  UI_PX(27)
#define CL_FUEL_ICON_GAP UI_PX(6)
#define CL_FUEL_ICON_X  (CL_FUEL_TEXT_RX - CL_FUEL_TEXT_W - CL_FUEL_ICON_GAP \
			 - CL_FUEL_ICON_W)
#define CL_FUEL_ICON_Y  UI_PY(540)
#define CL_FUEL_UNIT_CX UI_PX(960)       /* "km/h" caption over the fuel bar */
#define CL_FUEL_UNIT_Y  UI_PY(506)
/* bottom-left power bar (application) mirrors the fuel bar */
#define CL_POWER_BAR_X  UI_PX(24)
#define CL_POWER_CAP_Y  UI_PY(522)
#define CL_BRAND_CX0    UI_PX(434)       /* first of the six brand letters */
#define CL_BRAND_DX     UI_PX(26)
#define CL_BRAND_CY     UI_PY(568)
#define CL_CAPTION_Y    UI_PY(438)       /* safe core caption while the application is down */
#define CL_CAPTION_SUB_DY UI_PY(30)
#define CL_CAPTION_DOT_DY UI_PY(62)
#define CL_FOOTER_X     UI_PX(24)
#define CL_FOOTER_Y     UI_PY(578)       /* frame counters, tiny */
#define CL_FOOTER_RX    (UI_W - UI_PX(14))

/* ---------------------------------------------------------------- medical */
#define MD_TOPBAR_H     UI_PY(44)        /* application: bed, patient, time */
#define MD_WAVE_X       UI_PX(16)
#define MD_WAVE_W       UI_PX(688)
#define MD_WAVE_Y       UI_PY(52)
#define MD_WAVE_CH_H    UI_PY(100)
#define MD_WAVE_CHANNELS 5               /* ECG I, II, III, SpO2 pleth, resp */
/* The coordinate space the application builds its trace commands in: one
 * sample per design pixel, the same on every panel, so a wider channel shows
 * the same curve larger rather than needing a different producer. */
#define MD_WAVE_CMD_W   688
#define MD_WAVE_CMD_H   100
#define MD_WAVE_LABEL_DY UI_PY(4)
#define MD_WAVE_BASE_DY UI_PY(64)        /* trace baseline within a channel */

/* numeric column: three parameter tiles, then the status row */
#define MD_COL_X        UI_PX(720)
#define MD_TILE_Y       UI_PY(52)
#define MD_TILE_H       UI_PY(144)
#define MD_TILES        3
#define MD_STATUS_Y     (MD_TILE_Y + MD_TILES * MD_TILE_H)
#define MD_LABEL_X      UI_PX(736)
#define MD_RULE_X       UI_PX(728)      /* the rule across a tile */
#define MD_RULE_W       UI_PX(280)
#define MD_VALUE_RX     UI_PX(1000)      /* big value, right-aligned */
#define MD_VALUE_DY     UI_PY(34)
#define MD_UNIT_DY      UI_PY(40)        /* unit under the label */
#define MD_LIMITS_DY    UI_PY(98)        /* application: alarm limits under the unit */
#define MD_PILL_X       UI_PX(736)
#define MD_PILL_Y       UI_PY(502)
#define MD_PILL_W       UI_PX(96)
#define MD_PILL_H       UI_PY(26)
#define MD_PILL_DX      UI_PX(104)
#define MD_TILE_HR      0
#define MD_TILE_SPO2    1
#define MD_TILE_RESP    2
#define MD_COL_HR       0xFF4ADE80
#define MD_COL_SPO2     0xFF3FD0FF
#define MD_COL_RESP     0xFFFFD23F
#define MD_CAPTION_CX   (MD_WAVE_X + MD_WAVE_W / 2)
#define MD_CAPTION_Y    UI_PY(300)

#endif
