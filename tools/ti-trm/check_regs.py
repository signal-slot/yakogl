#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check driver/rust/src/soc/am62p/regs.rs against a text copy of the AM62Px TRM.

    pdftotext -layout spruj83d.pdf /some/where/spruj83d.txt     (not in the repository)
    tools/ti-trm/check_regs.py /some/where/spruj83d.txt

regs.rs is written by hand from TI SPRUJ83D (there is no TI header to
generate it from). Every constant there carries a `// TRM: NAME` comment
naming its register, or `// TRM: NAME.FIELD` naming a field; `DSI:` stands
for the DSI controller's prefix DSI_TOP_VBUSP_CFG_DSI_0_DSI_. This script
finds each register's "<NAME> Register (Offset = <x>h)" line and its field
table in the TRM text and checks the offset, or the field's bit range
(constants written as `bits(hi, lo)` / `bit(n)`). Field names in the TRM's
table are cut at about 18 characters; a cut name matches when it is a
prefix of ours. It prints one line per mismatch and a summary, and exits 1
if anything did not match or was not found.

The TRM is TI's copyrighted document and is never added to the repository,
so this is a tool to run by hand (driver/docs/am62p-display-cold.md), not a
tools/check.sh row. Python 3 standard library only.
"""
import os
import re
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
REGS = os.path.join(ROOT, "driver/rust/src/soc/am62p/regs.rs")
DSI_PREFIX = "DSI_TOP_VBUSP_CFG_DSI_0_DSI_"
ACCESS = r"(?:R|W|R/W|NONE|R/W1TC|R/W1TS|RW|RO|WO)"


def load_trm(path):
    lines = open(path, encoding="utf-8", errors="replace").read().split("\n")
    heads = {}
    for i, line in enumerate(lines):
        m = re.match(r"^\s*1 (\S+) Register \(Offset = ([0-9A-Fa-f]+)h\)", line)
        if m and m.group(1) not in heads:
            heads[m.group(1)] = (i, int(m.group(2), 16))
    return lines, heads


def fields(lines, start):
    """(hi, lo, name) rows of the field table after line `start`."""
    out = []
    in_table = False
    for line in lines[start + 1:start + 600]:
        if re.match(r"^\s*1 \S+ Register \(Offset", line):
            break
        # The table's title ("... Register Field Descriptions") is sometimes
        # broken over two lines by the PDF layout.
        if "Field Descriptions" in line or line.strip() == "Descriptions":
            in_table = True
            continue
        if not in_table:
            continue
        m = re.match(r"^\s+(\d+)(?::(\d+))?\s+(\S+)\s+" + ACCESS + r"\s", line)
        if m:
            hi = int(m.group(1))
            lo = int(m.group(2)) if m.group(2) else hi
            out.append((hi, lo, m.group(3)))
    return out


def main():
    if len(sys.argv) != 2:
        sys.stderr.write("usage: check_regs.py <SPRUJ83D as text>\n")
        sys.exit(2)
    lines, heads = load_trm(sys.argv[1])
    ok = bad = 0
    for n, line in enumerate(open(REGS), 1):
        m = re.search(r"pub const (\w+): u32 = (.*?); // TRM: (\S+)", line)
        if not m:
            continue
        name, value, cite = m.groups()
        reg, _, fld = cite.partition(".")
        if reg.startswith("DSI:"):
            reg = DSI_PREFIX + reg[4:]
        if reg not in heads:
            print("regs.rs:%d %s: no register %s in the TRM" % (n, name, reg))
            bad += 1
            continue
        start, offset = heads[reg]
        if not fld:
            v = int(value, 0)
            if v != offset:
                print("regs.rs:%d %s = %#x, TRM %s offset %#x" % (n, name, v, reg, offset))
                bad += 1
            else:
                ok += 1
            continue
        mb = re.fullmatch(r"bits\((\d+), (\d+)\)", value)
        m1 = re.fullmatch(r"bit\((\d+)\)", value)
        if mb:
            hi, lo = int(mb.group(1)), int(mb.group(2))
        elif m1:
            hi = lo = int(m1.group(1))
        else:
            print("regs.rs:%d %s: value %r is not bits()/bit()" % (n, name, value))
            bad += 1
            continue
        rows = fields(lines, start)
        match = [r for r in rows if fld.startswith(r[2]) or r[2] == fld]
        if not match:
            print("regs.rs:%d %s: no field %s in %s" % (n, name, fld, reg))
            bad += 1
        elif not any((r[0], r[1]) == (hi, lo) for r in match):
            print("regs.rs:%d %s = %d:%d, TRM %s.%s %s" % (
                n, name, hi, lo, reg, fld, ", ".join("%d:%d" % (r[0], r[1]) for r in match)))
            bad += 1
        else:
            ok += 1
    print("check_regs: %d constants match the TRM, %d do not" % (ok, bad))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
