#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Turn include/cluster/ui_layout.h into the Slint global both designs import.

The rich UI (A53, GPU) and the safety layer (R5F/M7, software renderer) are two
Slint designs that have to agree on where the speed digits are, to the pixel,
or the safe core's rectangles land somewhere other than the readouts they are
supposed to cover.  They used to agree by having the same numbers typed into
two theme.slint files, which is an agreement that lasts exactly until someone
edits one of them.

So the numbers are not typed twice.  ui_layout.h is the design, and this script
turns it into `ui/<panel>/cluster-layout.slint`, a Slint global named `Layout`
that both designs import through the Slint compiler's include path.  It does
not parse the header: it compiles it, with the host C compiler, and prints
every value.  Whatever the preprocessor computes is what Slint gets.

    scripts/gen_layout_slint.py            # regenerate both panels in place
    scripts/gen_layout_slint.py --check    # fail if the checked-in files differ

--check is what the test suite runs, so a change to ui_layout.h that was not
regenerated is a test failure rather than a picture nobody looks at closely.
"""
import argparse
import os
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
HEADER = ROOT / "include" / "cluster" / "ui_layout.h"

PANELS = [("1024x600", []), ("1280x800", ["-DCLUSTER_PANEL_1280X800"])]

# (C macro, Slint property name, kind).  Explicit rather than derived from the
# header's spelling: a reviewer can see exactly what crosses into the designs,
# and a macro that is not listed here is one the designs do not use.
#   px  -> <length>, in panel pixels
#   int -> <int>,  float -> <float>,  col -> <color> from an ARGB literal
LENGTHS_PALETTE_INTS = [
    # ---- the panel itself
    ("UI_W", "w", "px"),
    ("UI_H", "h", "px"),

    # ---- palette (ARGB)
    ("UI_COL_BG_TOP", "bg-top", "col"),
    ("UI_COL_BG_BOT", "bg-bottom", "col"),
    ("UI_COL_TEXT", "text", "col"),
    ("UI_COL_TEXT2", "text2", "col"),
    ("UI_COL_DIM", "dim", "col"),
    ("UI_COL_HAIR", "hairline", "col"),
    ("UI_COL_ACCENT", "accent", "col"),
    ("UI_COL_GEAR", "gear", "col"),
    ("UI_COL_GREEN", "green", "col"),
    ("UI_COL_AMBER", "amber", "col"),
    ("UI_COL_RED", "red", "col"),
    ("UI_COL_TRACK", "track", "col"),
    ("MD_COL_HR", "md-hr", "col"),
    ("MD_COL_SPO2", "md-spo2", "col"),
    ("MD_COL_RESP", "md-resp", "col"),

    # ---- font faces, chosen per panel (not scaled: the glyphs are bitmaps)
    ("UI_FS_SMALL", "small", "px"),
    ("UI_FS_LABEL", "label", "px"),
    ("UI_FS_MEDIUM", "medium", "px"),
    ("UI_FS_BIG", "big", "px"),
    ("UI_FS_SPEED", "speed", "px"),
    ("UI_FS_GEAR", "gear-size", "px"),
    ("UI_FS_VITALS", "vitals", "px"),
    ("UI_FW_REGULAR", "regular", "int"),
    ("UI_FW_SEMIBOLD", "semibold", "int"),
    ("UI_FW_LIGHT", "light", "int"),

    # ---- dials
    ("CL_LEFT_CX", "left-cx", "px"),
    ("CL_RIGHT_CX", "right-cx", "px"),
    ("CL_DIAL_CY", "dial-cy", "px"),
    ("CL_DIAL_R", "dial-r", "px"),
    ("CL_DIAL_W", "dial-w", "px"),
    ("CL_FACE_R", "face-r", "px"),
    ("CL_TICK_R0", "tick-r0", "px"),
    ("CL_TICK_MINOR", "tick-minor-len", "px"),
    ("CL_TICK_MAJOR", "tick-major-len", "px"),
    ("CL_NUM_R", "num-r", "px"),
    ("CL_DIAL_A0", "dial-a0", "float"),
    ("CL_DIAL_SWEEP", "dial-sweep", "float"),
    ("CL_RPM_MAX", "rpm-max", "int"),
    ("CL_SPEED_MAX", "speed-max", "int"),
    ("CL_DIAL_CAP_DY", "dial-cap-dy", "px"),
    ("CL_DIAL_UNIT_DY", "dial-unit-dy", "px"),
    ("CL_DIAL_VALUE_DY", "dial-value-dy", "px"),
    ("CL_DIAL_STATE_DY", "dial-state-dy", "px"),
    ("CL_TRIP_CAP_DY", "trip-cap-dy", "px"),
    ("CL_TRIP_VAL_DY", "trip-val-dy", "px"),
    ("CL_ODO_CAP_DY", "odo-cap-dy", "px"),
    ("CL_ODO_VAL_DY", "odo-val-dy", "px"),

    # ---- telltale row
    ("CL_TT_Y", "tt-y", "px"),
    ("CL_TT_H", "tt-h", "px"),
    ("CL_TT_SLOT_W", "tt-slot-w", "px"),
    ("CL_TT_SLOTS", "tt-slots", "int"),
    ("CL_TT_X0", "tt-x0", "px"),

    # ---- status row
    ("CL_STATUS_Y", "status-y", "px"),
    ("CL_STATUS_H", "status-h", "px"),
    ("CL_TIME_CX", "time-cx", "px"),
    ("CL_TIME_Y", "time-y", "px"),
    ("CL_GEAR_CX", "gear-cx", "px"),
    ("CL_GEAR_Y", "gear-y", "px"),
    ("CL_READY_X", "ready-x", "px"),
    ("CL_READY_Y", "ready-y", "px"),
    ("CL_READY_W", "ready-w", "px"),
    ("CL_READY_H", "ready-h", "px"),
    ("CL_READY_TEXT_Y", "ready-text-y", "px"),
    ("CL_HAIR_Y", "hair-y", "px"),
    ("CL_HAIR_X0", "hair-x0", "px"),
    ("CL_HAIR_X1", "hair-x1", "px"),

    # ---- speed
    ("CL_SPEED_CX", "speed-cx", "px"),
    ("CL_SPEED_TOP", "speed-top", "px"),
    ("CL_SPEED_UNIT_Y", "speed-unit-y", "px"),

    # ---- centre panel and logo
    ("CL_LOGO_X", "logo-x", "px"),
    ("CL_LOGO_Y", "logo-y", "px"),
    ("CL_LOGO_W", "logo-w", "px"),
    ("CL_LOGO_H", "logo-h", "px"),
    ("CL_INFO_X0", "info-x0", "px"),
    ("CL_INFO_X1", "info-x1", "px"),
    ("CL_INFO_Y", "info-y", "px"),
    ("CL_INFO_ROW_H", "info-row-h", "px"),

    # ---- bottom row
    ("CL_FUEL_BAR_X", "fuel-bar-x", "px"),
    ("CL_FUEL_BAR_Y", "fuel-bar-y", "px"),
    ("CL_FUEL_BAR_W", "fuel-bar-w", "px"),
    ("CL_FUEL_BAR_H", "fuel-bar-h", "px"),
    ("CL_FUEL_SEGS", "fuel-segs", "int"),
    ("CL_FUEL_SEG_W", "fuel-seg-w", "px"),
    ("CL_FUEL_SEG_DX", "fuel-seg-dx", "px"),
    ("CL_FUEL_TEXT_RX", "fuel-text-rx", "px"),
    ("CL_FUEL_TEXT_Y", "fuel-text-y", "px"),
    ("CL_FUEL_ICON_X", "fuel-icon-x", "px"),
    ("CL_FUEL_ICON_Y", "fuel-icon-y", "px"),
    ("CL_FUEL_UNIT_CX", "fuel-unit-cx", "px"),
    ("CL_FUEL_UNIT_Y", "fuel-unit-y", "px"),
    ("CL_POWER_BAR_X", "power-bar-x", "px"),
    ("CL_POWER_CAP_Y", "power-cap-y", "px"),
    ("CL_BRAND_CX0", "brand-cx0", "px"),
    ("CL_BRAND_DX", "brand-dx", "px"),
    ("CL_BRAND_CY", "brand-cy", "px"),
    ("CL_CAPTION_Y", "caption-y", "px"),
    ("CL_CAPTION_SUB_DY", "caption-sub-dy", "px"),
    ("CL_CAPTION_DOT_DY", "caption-dot-dy", "px"),
    ("CL_FOOTER_X", "footer-x", "px"),
    ("CL_FOOTER_Y", "footer-y", "px"),
    ("CL_FOOTER_RX", "footer-rx", "px"),

    # ---- medical monitor
    ("MD_TOPBAR_H", "md-topbar-h", "px"),
    ("MD_WAVE_X", "md-wave-x", "px"),
    ("MD_WAVE_W", "md-wave-w", "px"),
    ("MD_WAVE_Y", "md-wave-y", "px"),
    ("MD_WAVE_CH_H", "md-wave-ch-h", "px"),
    ("MD_WAVE_CHANNELS", "md-wave-channels", "int"),
    ("MD_WAVE_CMD_W", "md-wave-cmd-w", "int"),
    ("MD_WAVE_CMD_H", "md-wave-cmd-h", "int"),
    ("MD_WAVE_LABEL_DY", "md-wave-label-dy", "px"),
    ("MD_WAVE_BASE_DY", "md-wave-base-dy", "px"),
    ("MD_COL_X", "md-col-x", "px"),
    ("MD_TILE_Y", "md-tile-y", "px"),
    ("MD_TILE_H", "md-tile-h", "px"),
    ("MD_TILES", "md-tiles", "int"),
    ("MD_STATUS_Y", "md-status-y", "px"),
    ("MD_LABEL_X", "md-label-x", "px"),
    ("MD_RULE_X", "md-rule-x", "px"),
    ("MD_RULE_W", "md-rule-w", "px"),
    ("MD_VALUE_RX", "md-value-rx", "px"),
    ("MD_VALUE_DY", "md-value-dy", "px"),
    ("MD_UNIT_DY", "md-unit-dy", "px"),
    ("MD_LIMITS_DY", "md-limits-dy", "px"),
    ("MD_PILL_X", "md-pill-x", "px"),
    ("MD_PILL_Y", "md-pill-y", "px"),
    ("MD_PILL_W", "md-pill-w", "px"),
    ("MD_PILL_H", "md-pill-h", "px"),
    ("MD_PILL_DX", "md-pill-dx", "px"),
    ("MD_CAPTION_CX", "md-caption-cx", "px"),
    ("MD_CAPTION_Y", "md-caption-y", "px"),
]


# The font faces each design needs, as (Layout property, weight).
#
# The Slint compiler pre-renders bitmap glyphs only for sizes and weights it
# sees as LITERALS on a Text element -- a `Layout.small` reference is invisible
# to it -- so the seeds cannot be written against Layout and have to be
# generated per panel like everything else. Two sets rather than one: the
# safety layer never draws the medium and big faces, and an unused 60 px face
# is flash the M7 does not have to spend.
RICH_FACES = [("small", "regular"), ("label", "semibold"), ("medium", "semibold"),
              ("gear-size", "light"), ("big", "semibold"), ("speed", "light"),
              ("vitals", "semibold")]
SAFETY_FACES = [("small", "regular"), ("label", "semibold"), ("gear-size", "light"),
                ("speed", "light"), ("vitals", "semibold")]


def font_seeds(name, faces, by_name, doc):
    out = ["", "// %s" % doc,
           "export component %s inherits Rectangle {" % name,
           "    visible: false;"]
    for size, weight in faces:
        out.append('    Text { text: ""; font-size: %dpx; font-weight: %s; }'
                   % (int(by_name[size]), by_name[weight]))
    out.append("}")
    return out


def read_values(extra_flags):
    """Compile ui_layout.h and print every listed macro. The C preprocessor is
    the authority on what the header means; nothing here re-implements it."""
    lines = ['#include <stdio.h>', '#include "cluster/ui_layout.h"', "int main(void) {"]
    for macro, _, kind in LENGTHS_PALETTE_INTS:
        fmt = "%.6f" if kind == "float" else ("0x%08lx" if kind == "col" else "%ld")
        cast = "(double)" if kind == "float" else "(long)"
        lines.append('\tprintf("%s\\n", %s(%s));' % (fmt, cast, macro))
    lines.append("\treturn 0;\n}")
    src = "\n".join(lines) + "\n"

    cc = os.environ.get("CC", "cc")
    with tempfile.TemporaryDirectory() as td:
        cfile = pathlib.Path(td) / "dump.c"
        exe = pathlib.Path(td) / "dump"
        cfile.write_text(src)
        subprocess.run([cc, "-I", str(ROOT / "include"), *extra_flags,
                        str(cfile), "-o", str(exe)], check=True)
        out = subprocess.run([str(exe)], check=True, capture_output=True, text=True).stdout
    values = out.split()
    if len(values) != len(LENGTHS_PALETTE_INTS):
        raise SystemExit("dump printed %d values, expected %d"
                         % (len(values), len(LENGTHS_PALETTE_INTS)))
    return values


def slint_colour(argb_text):
    argb = int(argb_text, 16)
    a, rgb = (argb >> 24) & 0xFF, argb & 0xFFFFFF
    return "#%06x" % rgb if a == 0xFF else "#%06x%02x" % (rgb, a)


def render(panel, values):
    by_name = {}
    out = []
    out.append("// SPDX-License-Identifier: Apache-2.0")
    out.append("//")
    out.append("// Generated by scripts/gen_layout_slint.py from")
    out.append("// include/cluster/ui_layout.h for the %s panel. DO NOT EDIT." % panel)
    out.append("//")
    out.append("// The rich UI and the safety layer are separate Slint designs on separate")
    out.append("// cores, and they have to place the same elements at the same pixels. They do")
    out.append("// it by importing this file, which is the shared C header and nothing else, so")
    out.append("// the two layers are aligned by construction rather than by inspection.")
    out.append("")
    out.append("export global Layout {")
    for (macro, name, kind), raw in zip(LENGTHS_PALETTE_INTS, values):
        by_name[name] = raw
        if kind == "px":
            out.append("    out property <length> %s: %dpx;" % (name, int(raw)))
        elif kind == "int":
            out.append("    out property <int> %s: %d;" % (name, int(raw)))
        elif kind == "float":
            out.append("    out property <float> %s: %s;" % (name, raw.rstrip("0").rstrip(".")))
        elif kind == "col":
            out.append("    out property <color> %s: %s;" % (name, slint_colour(raw)))
        else:
            raise SystemExit("unknown kind %r" % kind)
    # One design pixel, on this panel. The pictograms in gauge-parts.slint are
    # drawn from literals of the 1024x600 design and must stay isotropic (a
    # circle drawn with two different scales is an ellipse), so they multiply
    # their literals by this rather than by a per-axis scale.
    unit = int(by_name["w"]) / 1024.0
    out.append("")
    out.append("    // one pixel of the 1024x600 reference design, on this panel")
    out.append("    out property <length> u: %spx;" % ("%g" % unit))
    out.append("}")
    out += font_seeds("RichFontSeeds", RICH_FACES, by_name,
                      "Every face the rich UI draws, so the compiler embeds them all.")
    out += font_seeds("SafetyFontSeeds", SAFETY_FACES, by_name,
                      "Every face the safety layer draws. A subset of the rich UI's.")
    return "\n".join(out) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true",
                    help="exit non-zero if a checked-in file is not what would be generated")
    args = ap.parse_args()

    bad = False
    for panel, flags in PANELS:
        text = render(panel, read_values(flags))
        path = ROOT / "ui" / panel / "cluster-layout.slint"
        if args.check:
            have = path.read_text() if path.exists() else ""
            if have != text:
                print("%s is out of date; run scripts/gen_layout_slint.py" % path, file=sys.stderr)
                bad = True
            else:
                print("%s up to date" % path)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
            print("wrote %s" % path)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
