#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare two rows of Mesa's etnaviv feature databases.

Pinned comparison for the STM32MP257F-DK port (docs/notes/pre-study-stm32mp257f-dk.md,
sections 1, 3, 5.1): ST's `GCNANOULTRA31_VIP2` row for model 0x8000, revision
0x6205, product 0x80003, eco 0, customer 0x15 (what issue #1281 read on the
board) against NXP's `gc7000ULN_v123` eco 0 row (the i.MX8MP's), both from
Mesa 26.2.1: src/etnaviv/hwdb/{st,nxp}/gc_feature_database.h.

Usage:
  hwdb_compare.py [MESA_TREE] [--all-st] [--check EXPECTED | --write EXPECTED]

MESA_TREE defaults to $MESA_TREE. Output is sorted and stable. Without
--check the report goes to stdout (redirect it to refresh the expected file).

Exit codes: 0 equal (with --check: the report equals EXPECTED), 1 differ,
2 no Mesa tree (a gap, never a pass).
"""

import argparse
import hashlib
import os
import re
import sys

ST_HDR = "src/etnaviv/hwdb/st/gc_feature_database.h"
NXP_HDR = "src/etnaviv/hwdb/nxp/gc_feature_database.h"

# (chipID, chipVersion, productID, ecoID, customerID)
ST_PINNED = (0x8000, 0x6205, 0x80003, 0, 0x15)
NXP_PINNED = (0x7000, 0x6204, 0x70003, 0, 0x11)

ID_FIELDS = ("ChipID", "ChipRevision", "ProductID", "EcoID", "CustomerID")

# The limits and bits section 3 of the note names as shared.
SHARED = (
    "gcFEATURE_VALUE_InstructionCount",
    "gcFEATURE_VALUE_TempRegisters",
    "gcFEATURE_VALUE_NumberOfConstants",
    "gcFEATURE_VALUE_VaryingCount",
    "gcFEATURE_VALUE_NumPixelPipes",
    "gcFEATURE_VALUE_VertexOutputBufferSize",
    "gcFEATURE_VALUE_VertexCacheSize",
    "gcFEATURE_VALUE_ThreadCount",
    "gcFEATURE_VALUE_NumShaderCores",
    "gcFEATURE_VALUE_Streams",
    "gcFEATURE_BIT_REG_Halti5",
    "gcFEATURE_BIT_REG_UnifiedSamplers",
    "gcFEATURE_BIT_REG_InstructionCache",
    "gcFEATURE_BIT_SECURITY_AHB",
    "gcFEATURE_BIT_REG_BltEngine",
    "gcFEATURE_BIT_DEC400",
)

ENTRY_RE = re.compile(r"^    /\* (.*) \*/$")
FIELD_RE = re.compile(r"^        (\S+), /\* (\S+) \*/$")


class Row:
    def __init__(self, comment, line):
        self.comment = comment
        self.line = line  # line of the entry's name comment
        self.fields = {}

    def ids(self):
        f = self.fields
        return tuple(f[k] for k in ID_FIELDS)


def parse(path):
    """All rows of a gc_feature_database.h, in file order."""
    rows = []
    cur = None
    pending = None
    with open(path, encoding="utf-8", errors="replace") as fh:
        for n, text in enumerate(fh, 1):
            text = text.rstrip("\n")
            m = ENTRY_RE.match(text)
            if m:
                pending = (m.group(1), n)
                continue
            if text == "    {" and pending:
                cur = Row(*pending)
                pending = None
                continue
            if text == "    }," and cur:
                rows.append(cur)
                cur = None
                continue
            if cur:
                m = FIELD_RE.match(text)
                if m:
                    name = m.group(2)
                    if name in cur.fields:
                        raise SystemExit("duplicate field %s in row at line %d" % (name, cur.line))
                    v = m.group(1)
                    cur.fields[name] = int(v, 0) if v[:1].isdigit() else v
    return rows


def pick(rows, ids, what):
    hit = [r for r in rows if r.ids() == ids]
    if len(hit) != 1:
        raise SystemExit("%s: %d rows match %s" % (what, len(hit), fmt_ids(ids)))
    return hit[0]


def fmt_ids(ids):
    return " ".join("%s=%s" % (k, hex(v)) for k, v in zip(ID_FIELDS, ids))


def val(v):
    if v is None:
        return "absent"
    return hex(v) if isinstance(v, int) else v


def feature_names(*rows):
    names = set()
    for r in rows:
        names.update(k for k in r.fields if k.startswith(("gcFEATURE_BIT_", "gcFEATURE_VALUE_")))
    return sorted(names)


def diff(a, b):
    out = []
    for k in feature_names(a, b):
        va, vb = a.fields.get(k), b.fields.get(k)
        if va != vb:
            out.append((k, va, vb))
    return out


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        h.update(fh.read())
    return h.hexdigest()


def describe(tag, path, row):
    f = row.fields
    return "%s: %s, entry '%s' at %s:%d, product name %s" % (
        tag, fmt_ids(row.ids()), row.comment, os.path.basename(os.path.dirname(path)) + "/" + os.path.basename(path), row.line, f.get("ProductName") or '""')


def report(tree, all_st):
    st_path = os.path.join(tree, ST_HDR)
    nxp_path = os.path.join(tree, NXP_HDR)
    st_rows = parse(st_path)
    nxp_rows = parse(nxp_path)
    st = pick(st_rows, ST_PINNED, "st")
    nxp = pick(nxp_rows, NXP_PINNED, "nxp")

    lines = []
    ver = "unknown"
    try:
        with open(os.path.join(tree, "VERSION")) as fh:
            ver = fh.read().strip()
    except OSError:
        pass
    lines.append("# hwdb comparison, ST 0x8000/0x6205 against NXP i.MX8MP gc7000ULN_v123 eco 0")
    lines.append("mesa version: " + ver)
    lines.append("sha256 %s %s" % (sha256(st_path), ST_HDR))
    lines.append("sha256 %s %s" % (sha256(nxp_path), NXP_HDR))
    lines.append("")
    lines.append("[ids]")
    lines.append(describe("st ", st_path, st))
    lines.append(describe("nxp", nxp_path, nxp))
    lines.append("")

    d = diff(st, nxp)
    lines.append("[differences] %d of %d gcFEATURE_* fields (name, st, nxp)" % (
        len(d), len(feature_names(st, nxp))))
    for k, va, vb in d:
        lines.append("%s st=%s nxp=%s" % (k, val(va), val(vb)))
    lines.append("")

    lines.append("[shared limits and bits] (name, st, nxp)")
    for k in sorted(SHARED):
        va, vb = st.fields.get(k), nxp.fields.get(k)
        lines.append("%s st=%s nxp=%s %s" % (k, val(va), val(vb), "same" if va == vb else "DIFFERS"))
    equal = not d and st.ids() == nxp.ids()

    if all_st:
        lines.append("")
        lines.append("[other st rows] informational, not pinned; counts of differing "
                     "gcFEATURE_* fields against the pinned st row and against nxp")
        for r in st_rows:
            if r is st:
                continue
            if r.fields.get("ChipID") not in (0x7000, 0x8000):
                continue
            lines.append("%s '%s' line %d: vs st-pinned %d, vs nxp %d" % (
                fmt_ids(r.ids()), r.comment, r.line, len(diff(r, st)), len(diff(r, nxp))))
            for k, va, vb in diff(r, st):
                lines.append("  %s this=%s st-pinned=%s" % (k, val(va), val(vb)))
    return "\n".join(lines) + "\n", equal


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("mesa_tree", nargs="?", help="Mesa 26.2.1 source tree (default $MESA_TREE)")
    ap.add_argument("--check", metavar="EXPECTED", help="compare the report with this file")
    ap.add_argument("--write", metavar="EXPECTED", help="write the report to this file (refresh)")
    ap.add_argument("--all-st", action="store_true", help="also list the other ST candidate rows")
    args = ap.parse_args()

    tree = args.mesa_tree or os.environ.get("MESA_TREE")
    if not tree:
        print("gap: no Mesa tree (argument or $MESA_TREE)", file=sys.stderr)
        return 2
    missing = [p for p in (ST_HDR, NXP_HDR) if not os.path.isfile(os.path.join(tree, p))]
    if missing:
        print("gap: %s lacks %s" % (tree, ", ".join(missing)), file=sys.stderr)
        return 2

    text, equal = report(tree, args.all_st)
    if args.write:
        with open(args.write, "w", encoding="utf-8") as fh:
            fh.write(text)
        return 0
    if args.check:
        try:
            with open(args.check, encoding="utf-8") as fh:
                want = fh.read()
        except OSError as e:
            print("cannot read expected file: %s" % e, file=sys.stderr)
            return 2
        if want == text:
            print("hwdb_compare: matches %s" % args.check)
            return 0
        import difflib
        sys.stdout.writelines(difflib.unified_diff(
            want.splitlines(True), text.splitlines(True), args.check, "actual"))
        return 1
    sys.stdout.write(text)
    return 0 if equal else 1


if __name__ == "__main__":
    sys.exit(main())
