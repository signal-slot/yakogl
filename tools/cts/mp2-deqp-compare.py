#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Diff the STM32MP257F-DK's dEQP-GLES2 per-case results against the i.MX8MP's failures.

    python3 -I tools/cts/mp2-deqp-compare.py [--strict] [--imx8mp FILE]
                                             [--quiet] DK_RESULT...

Both GPUs are the same Vivante family (src/vivante/profile.rs), so a case that
fails only on the DK stands out. Prints three sorted lists with counts:
DK-only failures, i.MX8MP-only failures, common failures. Last line:
"mp2-deqp-compare: dk-only N, imx8mp-only M, common K". Exit 0; with --strict
exit 1 when N > 0. Exit 2 on a usage or input error.

DK_RESULT (one or more) is read in any of these forms:
  - a UART log of the deqp sample (driver/samples/zephyr/deqp, the lines
    tools/cts/deqp-uart-tally.py parses): "deqp: <Status> <case>", with NULs,
    CRs and a leading HH:MM:SS dropped. Source: the DK console
    (tools/board-stm32mp2.sh, tools/board-stm32mp2-uboot.sh). A case fails
    when its status is none of Pass, NotSupported, QualityWarning and
    CompatibilityWarning.
  - a results.tsv, the per-row file tools/board.sh writes: header
    "# index<TAB>filter<TAB>begun<TAB>passed<TAB>failed<TAB>notsup<TAB>other
    <TAB>noresult<TAB>failing names"; the last column lists the failing
    case names separated by ';' ('-' for none). Source: the DEQP record words
    and DEQP_FAILS pages (driver/zephyr/lib/board/imx8mp/gc_result.c,
    gc_result.h "The three-word record"), decoded by tools/board.sh.
  - a plain list: "<Status> <case>" per line (deqp-uart-tally.py --statuses
    form) or a bare "<case>" (a failing name).
Several files are united; a later file's status for a case replaces an
earlier one's.

--imx8mp FILE (default tools/cts/imx8mp-full-tree-29460f96.results.tsv, the
i.MX8MP full-tree run tools/cts/RESULTS.md cites) is read in the same forms;
its failing set is the i.MX8MP failure list.
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
NONFAIL = ("Pass", "NotSupported", "QualityWarning", "CompatibilityWarning")
STATUSES = NONFAIL + ("Fail", "InternalError", "ResourceError", "Crash",
                      "Timeout", "NoResult")
LINE = re.compile(r"^(?:\d\d:\d\d:\d\d\s+)?(?:deqp: )?(\w+) (dEQP-\S+)\s*$")
NAME = re.compile(r"^dEQP-\S+$")


def parse(text):
    """-> (failing names, non-failing names) of one file."""
    text = text.replace("\0", "").replace("\r", "")
    fails, ok, status = set(), set(), {}
    for l in text.splitlines():
        if l.startswith("#"):
            continue
        f = l.split("\t")
        if len(f) >= 9 and f[0].isdigit():
            fails.update(n for n in f[8].split(";") if n and n != "-")
            continue
        m = LINE.match(l)
        if m and m.group(1) in STATUSES:
            status[m.group(2)] = m.group(1)
        elif NAME.match(l.strip()):
            fails.add(l.strip())
    fails |= {c for c, s in status.items() if s not in NONFAIL}
    ok = {c for c, s in status.items() if s in NONFAIL}
    return fails, ok


def load(paths):
    fails = set()
    for p in paths:
        with open(p, errors="replace") as fh:
            f, ok = parse(fh.read())
        fails = (fails - ok) | f
    return fails


def main():
    doc = __doc__.split("\n")
    ap = argparse.ArgumentParser(
        description=doc[0], epilog="\n".join(doc[2:]),
        formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("result", nargs="+", metavar="DK_RESULT")
    ap.add_argument("--imx8mp", default=os.path.join(
        HERE, "imx8mp-full-tree-29460f96.results.tsv"))
    ap.add_argument("--strict", action="store_true",
                    help="exit 1 when a case fails on the DK only")
    ap.add_argument("--quiet", action="store_true", help="counts only")
    a = ap.parse_args()
    try:
        dk = load(a.result)
        imx = load([a.imx8mp])
    except OSError as e:
        print("mp2-deqp-compare: %s" % e, file=sys.stderr)
        return 2
    dk_only, imx_only, common = sorted(dk - imx), sorted(imx - dk), sorted(dk & imx)
    for title, names in (("DK-only failures", dk_only),
                         ("i.MX8MP-only failures", imx_only),
                         ("common failures", common)):
        print("== %s: %d" % (title, len(names)))
        if not a.quiet:
            for n in names:
                print(n)
    print("mp2-deqp-compare: dk-only %d, imx8mp-only %d, common %d"
          % (len(dk_only), len(imx_only), len(common)))
    return 1 if a.strict and dk_only else 0


if __name__ == "__main__":
    sys.exit(main())
