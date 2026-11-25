#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Give each T6-vs-iMX95 dEQP per-case difference a candidate G310 cause.

usage: python3 -I tools/cts/imx95-g310-cause.py [--summary] [FILE]

Reads a TSV (FILE or stdin), one case a line:
    case<TAB>t6_result<TAB>imx95_result
with results Pass, Fail, NotSupported, QualityWarning, CompatibilityWarning,
InternalError, Crash, Timeout or Missing. Blank lines and '#' lines are
copied through. Writes each case line plus a fourth column, `cause`, and with
--summary a per-cause count table after the lines (as '#' lines).

Causes (docs/notes/mali-g310-vs-g610.md sections 3 and 5). They are candidates
read from the case name and the two results, not findings:
  same             both boards gave one result
  both-fail        both non-Pass, results differ (an online-compiler refusal
                   of both boards, issue #11251, is the usual one)
  no-result        the iMX95 has no result (Missing, Timeout or Crash): a
                   hang or a lost row, not a G310 property
  tib-budget       a target of 12 bytes a pixel or more (multisample, float or
                   wide render targets): a G310v1-v3 has half the G610's tile
                   buffer
  tiler-levels     a tiler-heavy case (many primitives or a huge viewport):
                   the hierarchy mask follows TILER_FEATURES
  renderer-name    the case reads GL_RENDERER / GL_VERSION strings
  single-core      a G310's SHADER_PRESENT is 0x1: a timing or core-count case
  unexplained      the iMX95 differs and no rule names a G310 cause
"""
import re
import sys

RESULTS = (
    "Pass", "Fail", "NotSupported", "QualityWarning", "CompatibilityWarning",
    "InternalError", "Crash", "Timeout", "Missing",
)
NO_RESULT = ("Missing", "Timeout", "Crash")

# (cause, pattern over the lower-cased case name), first match wins
RULES = (
    ("renderer-name", re.compile(r"get_string|renderer|vendor|(^|[._])version")),
    ("tib-budget", re.compile(r"multisample|msaa|samples|rgba32|rgba16f|float|mrt|draw_buffers")),
    ("tiler-levels", re.compile(r"many_|large_|huge|max_(vertex|primitive)|scissor|viewport")),
    ("single-core", re.compile(r"performance|timing|parallel|thread|sync")),
)


def cause_of(case, t6, imx):
    if t6 == imx:
        return "same"
    if imx in NO_RESULT:
        return "no-result"
    if t6 != "Pass" and imx != "Pass":
        return "both-fail"
    name = case.lower()
    for cause, pattern in RULES:
        if pattern.search(name):
            return cause
    return "unexplained"


def process(lines):
    """Returns (output lines, {cause: count}); raises ValueError on a bad line."""
    out, counts = [], {}
    for n, raw in enumerate(lines, 1):
        line = raw.rstrip("\r\n")
        if not line.strip() or line.lstrip().startswith("#"):
            out.append(line)
            continue
        fields = line.split("\t")
        if len(fields) != 3:
            raise ValueError(f"line {n}: want case<TAB>t6<TAB>imx95, got {len(fields)} field(s)")
        case, t6, imx = fields
        for r in (t6, imx):
            if r not in RESULTS:
                raise ValueError(f"line {n}: unknown result {r!r}")
        cause = cause_of(case, t6, imx)
        counts[cause] = counts.get(cause, 0) + 1
        out.append("\t".join((case, t6, imx, cause)))
    return out, counts


def main(argv):
    args = argv[1:]
    summary = "--summary" in args
    args = [a for a in args if a != "--summary"]
    if len(args) > 1 or any(a.startswith("-") for a in args):
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        return 2
    try:
        if args:
            with open(args[0], encoding="utf-8") as f:
                lines = f.read().splitlines()
        else:
            lines = sys.stdin.read().splitlines()
        out, counts = process(lines)
    except (OSError, ValueError) as e:
        print(f"imx95-g310-cause: {e}", file=sys.stderr)
        return 1
    print("\n".join(out))
    if summary:
        print("# cause\tcases")
        for cause, n in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"# {cause}\t{n}")
        print(f"# total\t{sum(counts.values())}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
