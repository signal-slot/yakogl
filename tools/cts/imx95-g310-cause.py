#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Give each T6-vs-iMX95 per-case dEQP difference a candidate G310 cause.

usage: python3 -I tools/cts/imx95-g310-cause.py [--summary] [FILE]
       python3 -I tools/cts/imx95-g310-cause.py --self-test

Reads a TSV from FILE or stdin, one case a line:

    case <TAB> t6_result <TAB> imx95_result

with results Pass, Fail, NotSupported, QualityWarning, CompatibilityWarning,
InternalError, Crash, Timeout or Missing, and optional '#' comment lines
(kept, as are blank lines). Without --summary it writes the same lines plus a
fourth column, `cause`; with --summary it writes the per-cause count table
instead (cause, count, share of the differences, one example case).

A line whose two results are equal has the cause `-`. A difference gets the
first rule below that matches; the cause is a candidate named from the case
and the two results, not a finding: docs/notes/mali-g310-vs-g610.md section 5
lists what the G310 differs in, this tool maps a case to it.

  missing-t6        the T6 has no result (the case is not in its tree)
  missing-imx95     the iMX95 has none (the row hung or ended before it)
  one-core-time     the iMX95 timed out or crashed: SHADER_PRESENT 0x1 is
                    one shader core, the T6's 0x50005 is four
  renderer-name     a string-query case that the T6 passes and the iMX95 does
                    not: GL_RENDERER names the G310
  tib-budget        a framebuffer or multisample case that the T6 passes and
                    the iMX95 does not: a G310v2's tile buffer is half the
                    G610's (colour 8192 bytes, Z 4096; targets of 12 bytes a
                    pixel or more)
  tiler-levels      a case drawing very large or very many primitives that
                    the T6 passes and the iMX95 does not (hierarchy levels;
                    TILER_FEATURES 0x809 reads as the G610's)
  g310-gain         the T6 does not pass and the iMX95 does
  g310-regress      the T6 passes and the iMX95 does not (no rule above)
  class-change      both do not pass, with different results
"""
import re
import sys

RESULTS = ("Pass", "Fail", "NotSupported", "QualityWarning",
           "CompatibilityWarning", "InternalError", "Crash", "Timeout",
           "Missing")
TIB = re.compile(r"\.fbo\.|\.render\.|multisamp|blend|color_clear|"
                 r"depth_stencil|renderbuffer")
TILER = re.compile(r"large|many|max_|lots|\.long_")
RENDERER = re.compile(r"\.string\.|get_string|renderer|vendor")


def cause(case, t6, imx):
    if t6 == imx:
        return "-"
    if t6 == "Missing":
        return "missing-t6"
    if imx == "Missing":
        return "missing-imx95"
    if imx in ("Timeout", "Crash"):
        return "one-core-time"
    if t6 == "Pass":
        if RENDERER.search(case):
            return "renderer-name"
        if TIB.search(case):
            return "tib-budget"
        if TILER.search(case):
            return "tiler-levels"
        return "g310-regress"
    if imx == "Pass":
        return "g310-gain"
    return "class-change"


def parse(lines):
    """Yield (raw, None) for comment and blank lines, else (raw, fields)."""
    for n, raw in enumerate(lines, 1):
        raw = raw.rstrip("\n")
        if not raw.strip() or raw.startswith("#"):
            yield raw, None
            continue
        f = raw.split("\t")
        if len(f) != 3:
            sys.exit("imx95-g310-cause: line %d: want 3 tab-separated fields, got %d" % (n, len(f)))
        for r in f[1:]:
            if r not in RESULTS:
                sys.exit("imx95-g310-cause: line %d: unknown result '%s'" % (n, r))
        yield raw, tuple(f)


def run(lines, summary):
    out, counts, example = [], {}, {}
    for raw, rec in parse(lines):
        if rec is None:
            out.append(raw)
            continue
        c = cause(*rec)
        out.append(raw + "\t" + c)
        if c != "-":
            counts[c] = counts.get(c, 0) + 1
            example.setdefault(c, rec[0])
    if not summary:
        return out
    total = sum(counts.values())
    table = ["cause\tcount\tshare\texample"]
    for c in sorted(counts, key=lambda k: (-counts[k], k)):
        table.append("%s\t%d\t%.1f%%\t%s" % (c, counts[c], 100 * counts[c] / total, example[c]))
    table.append("total\t%d\t100.0%%\t-" % total if total else "total\t0\t-\t-")
    return table


def self_test():
    d = "dEQP-GLES2.functional."
    cases = [
        (d + "a.same", "Pass", "Pass", "-"),
        (d + "a.same_nonpass", "Fail", "Fail", "-"),
        (d + "x.only_imx", "Missing", "Pass", "missing-t6"),
        (d + "x.only_t6", "Pass", "Missing", "missing-imx95"),
        (d + "x.slow", "Pass", "Timeout", "one-core-time"),
        (d + "x.crash", "Fail", "Crash", "one-core-time"),
        (d + "fbo.render.rgba", "Pass", "Fail", "tib-budget"),
        (d + "multisampled.x", "Pass", "InternalError", "tib-budget"),
        (d + "draw.many_triangles", "Pass", "Fail", "tiler-levels"),
        (d + "state_query.string.renderer", "Pass", "Fail", "renderer-name"),
        (d + "shaders.plain", "Pass", "Fail", "g310-regress"),
        (d + "shaders.fixed", "Fail", "Pass", "g310-gain"),
        (d + "shaders.other", "Fail", "NotSupported", "class-change"),
    ]
    tsv = ["# comment", ""] + ["%s\t%s\t%s" % (c, a, b) for c, a, b, _ in cases]
    got = run(tsv, False)
    assert got[:2] == ["# comment", ""], got[:2]
    n = 0
    for (c, a, b, want), line in zip(cases, got[2:]):
        assert line == "%s\t%s\t%s\t%s" % (c, a, b, want), (line, want)
        n += 1
    tab = run(tsv, True)
    assert tab[0] == "cause\tcount\tshare\texample", tab[0]
    assert tab[-1].startswith("total\t11\t"), tab[-1]
    assert any(t.startswith("tib-budget\t2\t") for t in tab), tab
    assert run([], True) == ["cause\tcount\tshare\texample", "total\t0\t-\t-"]
    for bad in ("a\tPass", "a\tPass\tBogus"):
        try:
            run([bad], False)
        except SystemExit:
            n += 1
        else:
            raise AssertionError(bad)
    print("%d checks passed" % (n + 4))


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    summary = "--summary" in argv
    rest = [a for a in argv if a != "--summary"]
    if len(rest) > 1 or any(a.startswith("-") for a in rest):
        sys.exit(__doc__)
    if rest:
        with open(rest[0], encoding="utf-8") as fh:
            lines = fh.readlines()
    else:
        lines = sys.stdin.readlines()
    print("\n".join(run(lines, summary)))


if __name__ == "__main__":
    main(sys.argv[1:])
