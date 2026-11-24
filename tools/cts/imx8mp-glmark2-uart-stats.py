#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Timing and row-loss statistics of the glmark2 frame dumps in an i.MX8MP console capture.

    tools/cts/imx8mp-glmark2-uart-stats.py [--watchdog-s N] <record dir | capture file> ...

A record dir (tools/cts/imx8mp-glmark2-dumps.sh run) names its console.ts
("<unix time> line"), else console.txt ("HH:MM:SS line"), else
console-live.txt, else console.raw (no stamps: every time prints "-"). One
line per dump:

    <scene>: first <t>s last <t>s duration <s>s bytes <n> <n>/s damaged <n>
    missing <n> boot-to-end <t>s margin <s>s -> ok | re-run (<why>)

Times are seconds since the last "*** Booting Zephyr" banner before the dump
(the image's uptime, which the watchdog counts), else since the capture's first
stamped line. bytes: the glmark2-fb lines as sent, 2 bytes (CR LF) per line.
Rows are checked by glmark2-fb.py (imported, not changed): "missing" is a row
with no line, "damaged" any other row fault (not base64, wrong length, crc16
differs, outside the frame, unreadable line) plus a frame crc32 that differs.
A dump with no end line is damaged ("end line missing").

The watchdog window (seconds of uptime the image feeds WDOG1): --watchdog-s,
else $WDOG_S, else WDOG_S in imx8mp-glmark2-dumps.sh. margin = window minus
boot-to-end; negative is over the window.

Exit 0: every dump whole and inside the window; 1: a dump is damaged or over
the window (re-run it, never patch); 2: bad input or no dump. Stdlib only.
Self-test: tools/cts/imx8mp-glmark2-uart-stats-selftest.sh.
"""
import binascii
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("glmark2_fb", os.path.join(HERE, "glmark2-fb.py"))
fb = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fb)

NAMES = ("console.ts", "console.txt", "console-live.txt", "console.raw")


def window_default(arg):
    if arg is not None:
        return float(arg)
    if os.environ.get("WDOG_S"):
        return float(os.environ["WDOG_S"])
    try:
        with open(os.path.join(HERE, "imx8mp-glmark2-dumps.sh"), errors="replace") as f:
            m = re.search(r"^WDOG_S=\$\{WDOG_S:-(\d+)", f.read(), re.M)
        if m:
            return float(m.group(1))
    except OSError:
        pass
    return None


def stamped(raw):
    """-> list of (time or None, text). Understands <unix time> and HH:MM:SS stamps."""
    out = []
    day = 0
    prev = None
    for line in raw.splitlines():
        line = line.rstrip("\r")
        m = re.match(r"(\d+\.\d+|\d{9,}) ?(.*)$", line)
        if m:
            out.append((float(m.group(1)), m.group(2)))
            continue
        m = re.match(r"(\d\d):(\d\d):(\d\d) (.*)$", line)
        if m:
            t = int(m.group(1)) * 3600 + int(m.group(2)) * 60 + int(m.group(3))
            if prev is not None and t + day < prev - 43200:
                day += 86400
            prev = t + day
            out.append((float(t + day), m.group(4)))
            continue
        out.append((None, line))
    return out


def analyse(path, window):
    with open(path, errors="replace") as f:
        lines = stamped(f.read())
    stamps = [t for t, _ in lines if t is not None]
    base_first = stamps[0] if stamps else None
    begins = [i for i, (_, s) in enumerate(lines) if re.search(r"glmark2-fb: .* \d+x\d+ begin\s*$", s)]
    results = []
    for n, b in enumerate(begins):
        e = begins[n + 1] if n + 1 < len(begins) else len(lines)
        seg = lines[b:e]
        text = "\n".join(s for _, s in seg)
        scene = re.search(r"glmark2-fb: (.*) \d+x\d+ begin", seg[0][1]).group(1)
        r = fb.decode(text, scene)
        fbl = [(t, s) for t, s in seg if "glmark2-fb: " in s]
        ts = [t for t, _ in fbl if t is not None]
        t0 = base_first
        for t, s in lines[:b]:
            if t is not None and "*** Booting Zephyr" in s:
                t0 = t
        first = ts[0] - t0 if ts and t0 is not None else None
        last = ts[-1] - t0 if ts and t0 is not None else None
        nbytes = sum(len(s[s.index("glmark2-fb: "):]) + 2 for _, s in fbl)
        missing = damaged = 0
        why = []
        if r is None:
            damaged, why = 1, ["no decodable dump"]
        else:
            _, w, h, rows, problems, end, _pic = r
            missing = sum(1 for p in problems if re.match(r"row \d+ missing$", p))
            damaged = len(problems) - missing
            if missing:
                why.append("%d row(s) missing" % missing)
            if damaged:
                why.append("%d damaged row(s)/line(s)" % damaged)
            if not problems:
                if end is None:
                    damaged += 1
                    why.append("end line missing")
                elif binascii.crc32(b"".join(rows)) & 0xFFFFFFFF != end:
                    damaged += 1
                    why.append("frame crc32 differs")
        end_t = None
        for t, s in fbl:
            if t is not None and "glmark2-fb: end crc32 " in s and t0 is not None:
                end_t = t - t0
        dur = last - first if first is not None else None
        margin = window - end_t if window is not None and end_t is not None else None
        if margin is not None and margin < 0:
            why.append("over the watchdog window by %.1f s" % -margin)
        results.append(dict(scene=scene, first=first, last=last, dur=dur, bytes=nbytes,
                            damaged=damaged, missing=missing, end=end_t, margin=margin, why=why))
    return results


def fmt(v, spec="%.1f"):
    return "-" if v is None else spec % v


def main(argv):
    window_arg = None
    paths = []
    it = iter(argv)
    for a in it:
        if a == "--watchdog-s":
            window_arg = next(it, None)
        elif a in ("-h", "--help"):
            print(__doc__)
            return 0
        else:
            paths.append(a)
    if not paths:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        window = window_default(window_arg)
    except (ValueError, TypeError):
        print("uart-stats: bad watchdog window", file=sys.stderr)
        return 2
    rc = 0
    total = 0
    for p in paths:
        if os.path.isdir(p):
            c = [os.path.join(p, n) for n in NAMES if os.path.isfile(os.path.join(p, n))]
            if not c:
                print("uart-stats: no console capture in " + p, file=sys.stderr)
                return 2
            p = c[0]
        elif not os.path.isfile(p):
            print("uart-stats: no such file: " + p, file=sys.stderr)
            return 2
        for r in analyse(p, window):
            total += 1
            bps = r["bytes"] / r["dur"] if r["dur"] else None
            bad = bool(r["why"])
            rc = 1 if bad else rc
            print("%s: first %ss last %ss duration %ss bytes %d %s/s damaged %d missing %d "
                  "boot-to-end %ss margin %ss -> %s" % (
                      r["scene"], fmt(r["first"]), fmt(r["last"]), fmt(r["dur"]), r["bytes"],
                      fmt(bps, "%.0f"), r["damaged"], r["missing"], fmt(r["end"]), fmt(r["margin"]),
                      "re-run (%s)" % "; ".join(r["why"]) if bad else "ok"))
    if total == 0:
        print("uart-stats: no glmark2-fb dump found", file=sys.stderr)
        return 2
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
