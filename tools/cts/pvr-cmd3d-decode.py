#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Name the words of the AM62P render stall dump's 3D (and geometry) command.

driver/rust/src/gpu/pvr/render.rs prints the last fragment command as 80 raw
dwords when a render stops (`dump_last_frag_cmd`), eight to a line:

    render.rs:1834  " c3d[{:2}]: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}"

(preceded by "render: last FRAG cmd3d (seq N), fence F:"), and in verbose
mode at kick time (render.rs:1445) as

    render.rs:1445  "render: cmd3d[{:2}]: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}"

The same shape with a `cgeom[` prefix is accepted for a geometry command (the
tree prints none today). Read a serial log (file or stdin); each record (the
lines from index 0 on) is printed one word per line: dword number, value and
field name.

Where the names come from. The vendored DDK headers define only the shared
block of the command (CMDTA3D_SHARED, CMD_COMMON) and the size/offset checks
of RGXFWIF_CMD3D (rgx_fwif_alignchecks.h:88-90); the 3D register block and
its flag word belong to a DDK user-mode struct they do not define. Those
offsets are the ones the driver writes (render.rs, cited per field, taken from
the DDK capture: driver/docs/gpu-recipes/coeff_base_analysis.md section 1).
A dword no source places prints as `(unplaced)`; a flag bit no source names
prints as `unnamed 0x...`. pds_pr_bgnd's dword is placed by count only, not
by a capture: the driver writes it at dword 64 with the partial-render knob
(render.rs CMD3D_PDS_PR_BGND_DW, marked provisional; docs/notes/
pvr-render-pool.md, "Open, where pds_pr_bgnd sits"); pass --pr-bgnd-dw 64 to
name those dwords, or the dword a capture places it at.

    pvr-cmd3d-decode.py [--pr-bgnd-dw N] [LOG]
"""
import argparse
import re
import sys

PBE_STRIDE_DW = 4  # render.rs:57
MAX_RT = 3  # render.rs:61

# (dword, name, citation). CMDTA3D_SHARED is 16 bytes (ddk_abi_pins.h:236):
# sCmn (CMD_COMMON, one dword ui32FrameNum, rgx_fwif_shared.h:151-155) at 0
# (:169), sHWRTData at 4 (ddk_abi_pins.h:237; fwif.rs:351; shared.h:170),
# asPRBuffer at 8, two slots (ddk_abi_pins.h:238,242; shared.h:174-175;
# ZSBUFFER 0, MSAABUFFER 1: ddk_abi_pins.h:239-241, shared.h:123-125).
SHARED = [
    (0, "sCmn.ui32FrameNum", "rgx_fwif_shared.h:155"),
    (1, "sHWRTData", "ddk_abi_pins.h:237; fwif.rs:351"),
    (2, "asPRBuffer[ZSBUFFER]", "ddk_abi_pins.h:238-239"),
    (3, "asPRBuffer[MSAABUFFER]", "ddk_abi_pins.h:240,242"),
]

# RGXFWIF_CMD3D after the shared block, as render.rs writes it.
CMD3D = SHARED + [
    (4, "usc_pixel_output_ctrl", "render.rs:1407"),
    (5, "usc_clear_register[0]", "render.rs:1408"),
    (6, "usc_clear_register[1]", "render.rs:1408"),
    (7, "usc_clear_register[2]", "render.rs:1408"),
    (8, "usc_clear_register[3]", "render.rs:1408"),
    (9, "isp_bgobjdepth", "render.rs:1409"),
    (10, "isp_bgobjvals", "render.rs:1410"),
    (11, "isp_aa", "render.rs:1411"),
    (12, "isp_ctl", "render.rs:1412"),
    (13, "event_pixel_pds_info", "render.rs:1413"),
    (14, "view_idx", "coeff_base_analysis.md:47"),
    (15, "event_pixel_pds_data", "render.rs:1414"),
    (16, "isp_oclqry_stride", "coeff_base_analysis.md:49"),
    (76, "flags", "render.rs:1437"),
    (77, "(zls stride, RTAs only)", "render.rs:1437"),
    (78, "(sls stride, RTAs only)", "render.rs:1437"),
    (79, "execute_count", "render.rs:1438"),
]
# 64-bit bases at dw18.. (render.rs:1421-1426)
for _i, _n in enumerate(("isp_scissor_base", "isp_dbias_base", "isp_oclqry_base",
                         "isp_zlsctl", "isp_zload_store_base",
                         "isp_stencil_load_store_base")):
    CMD3D += [(18 + 2 * _i, _n + ".lo", "render.rs:%d" % (1421 + _i)),
              (19 + 2 * _i, _n + ".hi", "render.rs:%d" % (1421 + _i))]
# pbe_word[i][0] at 30 + i*PBE_STRIDE_DW, [1] two dwords on (render.rs:1430-1432)
for _i in range(MAX_RT):
    for _k, _h in enumerate(("lo", "hi")):
        CMD3D += [(30 + _i * PBE_STRIDE_DW + _k, "pbe_word[%d][0].%s" % (_i, _h), "render.rs:1431"),
                  (32 + _i * PBE_STRIDE_DW + _k, "pbe_word[%d][1].%s" % (_i, _h), "render.rs:1432")]
# pds_bgnd[3] at dw70.. (render.rs:1434-1436)
for _i in range(3):
    CMD3D += [(70 + 2 * _i, "pds_bgnd[%d].lo" % _i, "render.rs:%d" % (1434 + _i)),
              (71 + 2 * _i, "pds_bgnd[%d].hi" % _i, "render.rs:%d" % (1434 + _i))]

# RGXFWIF_CMDTA, as render.rs writes it.
CMDTA = SHARED[:2] + [
    (4, "vdm_ctrl_stream_base.lo", "render.rs:1371"),
    (5, "vdm_ctrl_stream_base.hi", "render.rs:1372"),
    (8, "ppp_ctrl", "render.rs:1373"),
    (9, "te_psg", "render.rs:1374"),
    (12, "flags", "render.rs:1375"),
    (13, "partial-render fence address", "render.rs:1376,1386"),
    (14, "partial-render fence value", "render.rs:1377,1387"),
]

# Flag bits. CMD3D.flags: DEPTHBUFFER and STENCILBUFFER from the DDK capture
# (backend.rs:728,732; driver/src/pvr_drv.h:63-64), GETVISRESULTS
# (backend.rs:739). No vendored header names a partial-render bit of this
# word; the partial-render state shows in the asPRBuffer slots and, once
# placed, pds_pr_bgnd. Geometry flags: render.rs:89-90.
FLAGS_3D = [(0x20, "GETVISRESULTS"), (0x80, "DEPTHBUFFER"), (0x100, "STENCILBUFFER")]
FLAGS_GEOM = [(0x1, "FIRSTKICK"), (0x2, "LASTKICK")]

LINE = re.compile(r"\b(c3d|cmd3d|cgeom)\[\s*(\d+)\]:((?: [0-9a-fA-F]{8}){8})\s*$")


def flags_text(v, table):
    names = [n for b, n in table if v & b]
    rest = v & ~sum(b for b, _ in table)
    if rest:
        names.append("unnamed 0x%x" % rest)
    return "+".join(names) if names else "none"


def decode(kind, words, pr_bgnd_dw):
    names = {dw: name for dw, name, _ in (CMDTA if kind == "cgeom" else CMD3D)}
    if kind != "cgeom" and pr_bgnd_dw is not None:
        for j in range(6):
            names[pr_bgnd_dw + j] = "pds_pr_bgnd[%d].%s" % (j // 2, ("lo", "hi")[j % 2])
    out = []
    for i, w in enumerate(words):
        name = names.get(i, "(unplaced)")
        extra = ""
        if name == "flags":
            extra = "  [%s]" % flags_text(w, FLAGS_GEOM if kind == "cgeom" else FLAGS_3D)
        elif name.startswith("asPRBuffer"):
            extra = "  [%s]" % ("none" if w == 0 else "set")
        out.append("  dw%-2d %08x  %s%s" % (i, w, name, extra))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("log", nargs="?", help="serial log (default stdin)")
    ap.add_argument("--pr-bgnd-dw", type=lambda s: int(s, 0), default=None,
                    help="dword of pds_pr_bgnd[0].lo (the driver's provisional slot is 64, "
                    "render.rs CMD3D_PDS_PR_BGND_DW; a DDK capture may place it elsewhere)")
    a = ap.parse_args()
    f = open(a.log, errors="replace") if a.log else sys.stdin
    recs = []
    for line in f:
        m = LINE.search(line.rstrip("\r\n"))
        if not m:
            continue
        kind, idx = m.group(1), int(m.group(2))
        if idx == 0 or not recs or recs[-1][0] != kind or len(recs[-1][1]) != idx:
            recs.append([kind, []])
        recs[-1][1].extend(int(x, 16) for x in m.group(3).split())
    if not recs:
        print("pvr-cmd3d-decode: no c3d/cmd3d/cgeom word lines found", file=sys.stderr)
        return 1
    for n, (kind, words) in enumerate(recs, 1):
        print("record %d: %s, %d words" % (n, kind, len(words)))
        print("\n".join(decode(kind, words, a.pr_bgnd_dw)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
