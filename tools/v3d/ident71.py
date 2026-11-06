#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""ident71: decode the V3D identity words (4.2 and 7.1) into named fields.

Input (a file, or stdin with "-"), any mix of:
  - lines `NAME=0xHEX` (or `NAME: 0xHEX`, `NAME 0xHEX`) as a console log
    prints them: HUB_IDENT0..3, CORE_IDENT0..3 (CTL_IDENT*, V3D_CTL_0_IDENT*
    are the same words), MMU_DEBUG_INFO;
  - the one-line form of driver/src/v3d/v3d_hw.c,
    `HUB_IDENT0..3 a b c d CTL_IDENT0..2 e f g MMU_DEBUG_INFO h` (hex, no 0x);
  - a debugfs `v3d_ident` text: a `4.2.14.0` revision (tver.rev.iprev.compat,
    what the Pi 4 note records as "V3D 4.2.14.0"), used only for the fields
    the raw words do not give; a debugfs `v3d_regs` dump gives raw words.

Field layout, one source per line:
  hub IDENT1 TVER 3:0, REV 7:4, NCORES 11:8, WITH_TFU 17; IDENT2 WITH_MMU 8;
  core IDENT0 VER 31:24; core IDENT1 VPM_SIZE 31:28 (units of 8 KiB, Mesa),
  QUPS 11:8, NSLC 7:4, REV 3:0; MMU_DEBUG_INFO PA_WIDTH 11:8, VA_WIDTH 7:4
  (widths are 30 + field): driver/src/v3d/v3d_regs.h and v3d_hw.c.
  hub IDENT3 IPREV 15:8 and compat_rev 23:16: Mesa
  src/broadcom/common/v3d_device_info.c (MIT).
  hub IDENT3 IPIDX 7:0, core IDENT1 NSEM 23:16 and NTMU 15:12, MMU
  VERSION 3:0: Linux drivers/gpu/drm/v3d/v3d_regs.h (GPL), as facts only; see
  driver/docs/rpi5-v3d71-facts.md section 1 for the lines.
The device generation is Mesa's: ver = core VER * 10 + core IDENT1 REV.
The variant consequences (U2) are those of Mesa v3d_device_info.h/.c and
v3d_limits.h: has_accumulators, draw index, unpack sat/max0, default attribute
values, clipper granularity, CLE readahead, render targets, framebuffer size.

  ident71.py FILE...          print the decoded fields of each file
  ident71.py --field NAME F   print one field's value (exit 1 if absent)
  ident71.py --compare A B    print every field side by side; exit 1 on any
                              difference (a field only one side has is not
                              compared and is marked `-`)
Exit 2: unreadable or empty input.
"""
import re
import sys

RAW_NAMES = ["HUB_IDENT0", "HUB_IDENT1", "HUB_IDENT2", "HUB_IDENT3",
             "CORE_IDENT0", "CORE_IDENT1", "CORE_IDENT2", "CORE_IDENT3",
             "MMU_DEBUG_INFO"]

NAME_RE = re.compile(
    r"\b((?:V3D_)?(?:HUB_CTL|HUB|CTL_0|CTL|CORE_?0?)?_?IDENT[0-3]"
    r"|(?:V3D_)?(?:MMU0_|MMU_)DEBUG_INFO)"
    r"\b\s*(?:\([^)]*\))?\s*[=:]?\s*0x([0-9a-fA-F]+)", re.I)
ONE_LINE_RE = re.compile(
    r"HUB_IDENT0\.\.3\s+((?:[0-9a-f]+\s+){3}[0-9a-f]+)\s+(?:CTL|CORE)_IDENT0\.\.\d\s+"
    r"((?:[0-9a-f]+\s+)+?)MMU_DEBUG_INFO\s+([0-9a-f]+)", re.I)
REV_RE = re.compile(r"\b(\d+)\.(\d+)\.(\d+)\.(\d+)\b")


def canon(name):
    n = name.upper().replace("V3D_", "").replace("MMU0_", "MMU_")
    if "DEBUG_INFO" in n:
        return "MMU_DEBUG_INFO"
    if n.startswith("HUB"):
        return "HUB_IDENT" + n[-1]
    return "CORE_IDENT" + n[-1]


def parse(text):
    """-> (raw words dict, revision tuple or None)"""
    raw = {}
    m = ONE_LINE_RE.search(text)
    if m:
        for i, v in enumerate(m.group(1).split()):
            raw["HUB_IDENT%d" % i] = int(v, 16)
        for i, v in enumerate(m.group(2).split()):
            raw["CORE_IDENT%d" % i] = int(v, 16)
        raw["MMU_DEBUG_INFO"] = int(m.group(3), 16)
    for m in NAME_RE.finditer(text):
        raw.setdefault(canon(m.group(1)), int(m.group(2), 16))
    m = REV_RE.search(text)
    rev = tuple(int(x) for x in m.groups()) if m else None
    return raw, rev


def ascii4(v):
    s = "".join(chr((v >> (8 * i)) & 0xff) for i in range(4))
    return s.rstrip("\0") if all(32 <= ord(c) < 127 or c == "\0" for c in s) else "?"


def decode(raw, rev):
    f = {}
    for k in RAW_NAMES:
        if k in raw:
            f["raw." + k] = "0x%08x" % raw[k]
    h0, h1, h2, h3 = (raw.get("HUB_IDENT%d" % i) for i in range(4))
    c0, c1 = raw.get("CORE_IDENT0"), raw.get("CORE_IDENT1")
    mm = raw.get("MMU_DEBUG_INFO")
    if h0 is not None:
        f["hub.magic"] = ascii4(h0)
    if h1 is not None:
        f["hub.tver"] = h1 & 0xf
        f["hub.rev"] = (h1 >> 4) & 0xf
        f["hub.ncores"] = (h1 >> 8) & 0xf
        f["hub.with_tfu"] = (h1 >> 17) & 1
        f["hub.ident1_other"] = "0x%08x" % (h1 & ~(0xfff | 1 << 17))
        f["hub.ver"] = f["hub.tver"] * 10 + f["hub.rev"]
    if h2 is not None:
        f["hub.with_mmu"] = (h2 >> 8) & 1
    if h3 is not None:
        f["hub.ipidx"] = h3 & 0xff
        f["hub.iprev"] = (h3 >> 8) & 0xff
        f["hub.compat_rev"] = (h3 >> 16) & 0xff
    elif rev is not None:
        if h1 is None:
            f["hub.tver"], f["hub.rev"] = rev[0], rev[1]
            f["hub.ver"] = rev[0] * 10 + rev[1]
        f["hub.iprev"], f["hub.compat_rev"] = rev[2], rev[3]
    if c0 is not None:
        f["core.magic"] = ascii4(c0 & 0xffffff)
        f["core.ver"] = (c0 >> 24) & 0xff
    if c1 is not None:
        f["core.rev"] = c1 & 0xf
        f["core.nslc"] = (c1 >> 4) & 0xf
        f["core.qups"] = (c1 >> 8) & 0xf
        f["core.ntmu"] = (c1 >> 12) & 0xf
        f["core.nsem"] = (c1 >> 16) & 0xff
        f["core.vpm_units"] = (c1 >> 28) & 0xf
        f["core.vpm_bytes"] = f["core.vpm_units"] * 8192
        f["core.qpu_count"] = f["core.nslc"] * f["core.qups"]
    if mm is not None:
        f["mmu.version"] = mm & 0xf
        f["mmu.va_width"] = 30 + ((mm >> 4) & 0xf)
        f["mmu.pa_width"] = 30 + ((mm >> 8) & 0xf)
    if "core.ver" in f and "core.rev" in f:
        f["ver"] = f["core.ver"] * 10 + f["core.rev"]
        variant(f)
    elif "hub.ver" in f:
        f["ver"] = f["hub.ver"]
        variant(f)
    return f


def variant(f):
    """Mesa's device-info consequences (v3d_device_info.h/.c, v3d_limits.h)."""
    ver = f["ver"]
    rev = f.get("hub.iprev")
    compat = f.get("hub.compat_rev")
    f["mesa.supported"] = int(ver in (42, 71))
    f["mesa.has_accumulators"] = int(ver < 71)
    f["mesa.needs_default_attribute_values"] = int(ver <= 42)
    f["mesa.unpack_sat"] = int(ver > 45)
    f["mesa.max_render_targets"] = 4 if ver < 71 else 8
    f["mesa.max_framebuffer_size"] = 4096 if ver < 71 else 7680
    if ver == 42:
        f["mesa.clipper_xy_granularity"] = 256
        f["mesa.cle_readahead"] = 256
    elif ver == 71:
        f["mesa.clipper_xy_granularity"] = 64
        f["mesa.cle_readahead"] = 1024
    if rev is not None:
        f["mesa.draw_index"] = int(ver > 71 or (ver == 71 and rev >= 10))
        if compat is not None:
            f["mesa.unpack_max0"] = int(ver > 71 or (ver == 71 and (
                rev >= 7 or (rev == 6 and compat >= 4))))


def load(path):
    try:
        text = sys.stdin.read() if path == "-" else open(path, errors="replace").read()
    except OSError as e:
        sys.exit("ident71: %s" % e)
    f = decode(*parse(text))
    if not f:
        print("ident71: %s: no identity words found" % path, file=sys.stderr)
        sys.exit(2)
    return f


def show(path):
    for k, v in load(path).items():
        print("%-36s %s" % (k, v))


def compare(a, b):
    fa, fb = load(a), load(b)
    keys = list(fa) + [k for k in fb if k not in fa]
    diff = common = 0
    print("%-36s %-14s %-14s" % ("field", "A", "B"))
    for k in keys:
        va, vb = fa.get(k), fb.get(k)
        mark = ""
        if va is not None and vb is not None:
            common += 1
            if va != vb:
                mark, diff = "  DIFF", diff + 1
        print("%-36s %-14s %-14s%s" % (k, "-" if va is None else va,
                                       "-" if vb is None else vb, mark))
    print("ident71: %d fields compared, %d differ" % (common, diff))
    return 1 if diff or not common else 0


def main(argv):
    if len(argv) == 4 and argv[1] == "--compare":
        return compare(argv[2], argv[3])
    if len(argv) == 4 and argv[1] == "--field":
        f = load(argv[3])
        if argv[2] not in f:
            return 1
        print(f[argv[2]])
        return 0
    if len(argv) >= 2 and not argv[1].startswith("--"):
        for p in argv[1:]:
            if len(argv) > 2:
                print("== " + p)
            show(p)
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
