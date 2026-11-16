#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_reset_reason.py - classify TF-A BL2 "Reset reason:" lines in a DK
# console record (plan #362, item reset-reason).
#
#   tools/mp2s_reset_reason.py [--expect N] [FILE]     (FILE or stdin)
#
# The record is split into boots at each TF-A BL2 banner ("BL2: v2.10...");
# a "Reset reason:" line with no banner before it in its boot also starts a
# boot. One line per boot: index, the text after "Reset reason: ", a class.
#
# Classes (strings from stm32mp257f-dk-bringup.md, "Reset reason" table;
# matched case-insensitively on the key words, so the "(por_rstn)" suffix and
# the "(0x...)" raw word do not matter):
#   supply    "Power-on reset", "Brownout reset"
#   watchdog  "IWDG system reset" (checked first: it contains "system reset")
#   software  "System reset (SYSRST)", and "Pin reset from NRST" (the note
#             says the PSCI SYSTEM_RESET leaves one of the two, unverified
#             which)
#   unknown   anything else, and a boot with no Reset reason line
#
# Exit: 0 no boot is supply; 3 a boot is supply (prints "check the USB-C
# supply first"); 2 usage error, or --expect N not met.
import argparse
import re
import sys

BANNER = re.compile(r"\bBL2:\s+v\d")
HAS_REASON = re.compile(r"Reset reason:", re.I)
# text up to the raw word "(0x...)" or the end of the line
REASON = re.compile(r"Reset reason:\s*(.*?)\s*(?:\(0x[0-9a-fA-F]+\).*)?$", re.I)


def classify(text):
    t = text.lower()
    if "power-on reset" in t or "brownout reset" in t:
        return "supply"
    if "iwdg" in t:
        return "watchdog"
    if "system reset" in t or "pin reset" in t:
        return "software"
    return "unknown"


def split(lines):
    boots = []
    cur = None
    for ln in lines:
        if BANNER.search(ln):
            cur = {"text": None}
            boots.append(cur)
        m = HAS_REASON.search(ln)
        if m:
            if cur is None or cur["text"] is not None:
                cur = {"text": None}
                boots.append(cur)
            r = REASON.search(ln[m.start():])
            cur["text"] = (r.group(1) if r else ln.strip()) or None
    return boots


def main(argv):
    ap = argparse.ArgumentParser(description="classify BL2 Reset reason lines")
    ap.add_argument("file", nargs="?")
    ap.add_argument("--expect", type=int, metavar="N")
    try:
        a = ap.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0
    try:
        if a.file:
            with open(a.file, errors="replace") as f:
                lines = f.read().splitlines()
        else:
            lines = sys.stdin.read().splitlines()
    except OSError as e:
        print("usage: cannot read record: %s" % e, file=sys.stderr)
        return 2
    boots = split(lines)
    counts = {"supply": 0, "watchdog": 0, "software": 0, "unknown": 0}
    for i, b in enumerate(boots, 1):
        txt = b["text"]
        cls = classify(txt) if txt else "unknown"
        counts[cls] += 1
        print("boot %d: %s: %s" % (i, txt or "(no Reset reason line)", cls))
    print("summary: %d boot(s): %d supply, %d watchdog, %d software, %d unknown"
          % (len(boots), counts["supply"], counts["watchdog"],
             counts["software"], counts["unknown"]))
    if a.expect is not None and a.expect != len(boots):
        print("expected %d boot(s), found %d" % (a.expect, len(boots)),
              file=sys.stderr)
        return 2
    if counts["supply"]:
        print("check the USB-C supply first")
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
