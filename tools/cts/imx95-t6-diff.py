#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Per-case diff of Verdin iMX95 (Mali-G310) dEQP-GLES2 slices against the
NanoPC-T6's (Mali-G610).

  python3 -I tools/cts/imx95-t6-diff.py [--markdown] [--absent] [--limit N]
                                        --imx95 SLICE [--imx95 SLICE]...
                                        [--t6 SLICE]...

A SLICE is a directory of the format tools/cts/mali-deqp-tally.py reads: an
iMX95 one as tools/cts/imx95-uart-slice.py writes it, a T6 one as
mali-deqp-run.sh does. Without --t6 the T6 side is the baseline,
tools/cts/log/2026-11-24-t6-gles2-S*-8ff01055 (the whole tree at 8ff01055).
The readers and the rerun/replaces rule are the tally's, imported from it:
a case superseded within one side is not counted.

Each case has on each side a state and a cause. The state is the dEQP status,
except Lost, which reads as its reason: hang (chunk-hang), crash
(chunk-crash), not-run (not-run, board-lost and any other). The cause is, as
mali-deqp-tally.py --tally --per-case takes it, the case's own refusal
(refusals.tsv, "refused, <stage> shader: <why>"), else the first
"yakogl-mali-refusal:" line of its refusals/<label>.txt ("refused:..." or
"errno:N"), else for a Lost case its reason, for Pass '-', and for any other
dEQP's Result text with names and numbers folded ('-' on the iMX95: its
console carries no Result text).

A pair differs when the states differ, or when both are the same non-Pass
status and the refusal causes differ (one refused and the other not, or two
refusals of other words). The Result text is not compared, since one side has
none. Two refusals are the same when their text is equal or one is the
other cut short (the Zephyr console cuts a line at 256 bytes, the T6's slice a
refusal at 600), at least 20 characters of each kept.

The report: how many cases both sides hold and how many only one; the
differences per transition (<T6 state>-><iMX95 state>: Pass->Fail, Fail->Pass,
Pass->hang, ...), per transition and the two causes, and per case (T6 state
and cause, iMX95 state and cause), in mustpass order of the T6 side.
--limit N lists at most N cases (the counts stay whole). A case on one side
only is not a difference; --absent counts it as one (state "absent"). Text
by default, --markdown for tables. A slice against itself reports 0
differences. Exit 0 (differences are the report, not an error), 2 for a
slice that cannot be read or a usage error.
"""
import glob
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BASELINE = os.path.join(HERE, "log", "2026-11-24-t6-gles2-S*-8ff01055")
LOST_STATE = {"chunk-hang": "hang", "chunk-crash": "crash"}
MIN_PREFIX = 20
USAGE = ("usage: imx95-t6-diff.py [--markdown] [--absent] [--limit N] "
         "--imx95 SLICE [--imx95 SLICE]... [--t6 SLICE]...")


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, filename))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


tally = load("mali_deqp_tally", "mali-deqp-tally.py")


def side(paths):
    """({case: (state, cause, refused)}, [case order], [warnings])."""
    slices = [tally.load_slice(p) for p in paths]
    ordered, superseded, dups, _errs = tally.resolve(slices)
    out, order, warn = {}, [], []
    if dups:
        warn.append("%d case(s) are in more than one slice of a side; the last read wins"
                    % len(dups))
    for s in ordered:
        own = tally.refusal_map(s) or {}
        pc = tally.refusal_causes(s)
        for i, (_ln, f, _w) in enumerate(s["results"]):
            if i in superseded[s["name"]]:
                continue
            case, st, det = f
            if st == "Lost":
                state = LOST_STATE.get(det.strip(), "not-run")
                rc, c = None, det.strip() or "-"
            else:
                state = st
                rc = tally.refusal_cause(own[case]) if case in own else pc.get(case)
                c = rc or ("-" if st == "Pass" else tally.normalize(det))
            if case not in out:
                order.append(case)
            out[case] = (state, c, rc)
    return out, order, warn


def trimmed(c):
    return re.sub(r"(?:\.\.\.|[\s)])+$", "", re.sub(r"\s+", " ", c)).strip()


def same_refusal(a, b):
    if a is None or b is None:
        return a is b
    a, b = trimmed(a), trimmed(b)
    if a == b:
        return True
    short, long_ = sorted((a, b), key=len)
    return len(short) >= MIN_PREFIX and long_.startswith(short)


def diff(t6, imx, absent=False):
    """(both, only_t6, only_imx, [(case, t6 (state, cause), imx (state, cause))])."""
    a, a_order, _ = t6
    b, b_order, _ = imx
    both = [c for c in a_order if c in b]
    only_a = [c for c in a_order if c not in b]
    only_b = [c for c in b_order if c not in a]
    out = []
    for c in both:
        (sa, ca, ra), (sb, cb, rb) = a[c], b[c]
        if sa != sb or (sa != "Pass" and not same_refusal(ra, rb)):
            out.append((c, (sa, ca), (sb, cb)))
    if absent:
        out += [(c, a[c][:2], ("absent", "-")) for c in only_a]
        out += [(c, ("absent", "-"), b[c][:2]) for c in only_b]
    return len(both), len(only_a), len(only_b), out


def counts(rows):
    by_t, by_c = {}, {}
    for _c, (sa, ca), (sb, cb) in rows:
        t = "%s->%s" % (sa, sb)
        by_t[t] = by_t.get(t, 0) + 1
        k = (t, ca, cb)
        by_c[k] = by_c.get(k, 0) + 1
    return (sorted(by_t.items(), key=lambda x: (-x[1], x[0])),
            sorted(by_c.items(), key=lambda x: (-x[1], x[0])))


def names(paths):
    return ", ".join(os.path.basename(p.rstrip("/")) for p in paths)


def render_text(t6p, imxp, res, limit):
    both, oa, ob, rows = res
    by_t, by_c = counts(rows)
    out = ["T6 slices: " + names(t6p), "iMX95 slices: " + names(imxp),
           "cases on both sides: %d; only on the T6 side: %d; only on the iMX95 side: %d"
           % (both, oa, ob),
           "differences: %d" % len(rows)]
    if rows:
        out.append("per transition (T6->iMX95):")
        out += ["  %-24s %6d" % (t, n) for t, n in by_t]
        out.append("per transition and cause:")
        out += ["  %-24s %6d  T6: %s | iMX95: %s" % (t, n, ca, cb) for (t, ca, cb), n in by_c]
        out.append("per case:")
        for c, (sa, ca), (sb, cb) in rows[:limit]:
            out.append("  %s  T6: %s (%s)  iMX95: %s (%s)" % (c, sa, ca, sb, cb))
        if len(rows) > limit:
            out.append("  ... and %d more" % (len(rows) - limit))
    return "\n".join(out) + "\n"


def cell(s):
    return s.replace("|", "\\|")


def render_markdown(t6p, imxp, res, limit):
    both, oa, ob, rows = res
    by_t, by_c = counts(rows)
    out = ["T6 slices: " + ", ".join("`%s`" % n for n in names(t6p).split(", ")) + ".",
           "iMX95 slices: " + ", ".join("`%s`" % n for n in names(imxp).split(", ")) + ".",
           "",
           "Cases on both sides: %d; only on the T6 side: %d; only on the iMX95 side: %d. "
           "Differences: %d." % (both, oa, ob, len(rows))]
    if rows:
        out += ["", "| T6 -> iMX95 | Cases |", "| --- | ---: |"]
        out += ["| %s | %d |" % (t, n) for t, n in by_t]
        out += ["", "| T6 -> iMX95 | T6 cause | iMX95 cause | Cases |",
                "| --- | --- | --- | ---: |"]
        out += ["| %s | %s | %s | %d |" % (t, cell(ca), cell(cb), n) for (t, ca, cb), n in by_c]
        out += ["", "| Case | T6 | T6 cause | iMX95 | iMX95 cause |",
                "| --- | --- | --- | --- | --- |"]
        out += ["| `%s` | %s | %s | %s | %s |" % (c, sa, cell(ca), sb, cell(cb))
                for c, (sa, ca), (sb, cb) in rows[:limit]]
        if len(rows) > limit:
            out.append("")
            out.append("... and %d more cases." % (len(rows) - limit))
    return "\n".join(out) + "\n"


def main(argv):
    md = absent = False
    limit = None
    t6p, imxp = [], []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--markdown":
            md = True
        elif a == "--absent":
            absent = True
        elif a in ("--imx95", "--t6", "--limit"):
            if i + 1 >= len(argv):
                print(USAGE, file=sys.stderr)
                return 2
            v = argv[i + 1]
            i += 1
            if a == "--limit":
                if not v.isdigit():
                    print(USAGE, file=sys.stderr)
                    return 2
                limit = int(v)
            else:
                (imxp if a == "--imx95" else t6p).append(v)
        else:
            print(USAGE, file=sys.stderr)
            return 2
        i += 1
    if not t6p:
        t6p = sorted(glob.glob(BASELINE))
    if not imxp or not t6p:
        print(USAGE if not imxp else "imx95-t6-diff: no T6 slice (%s)" % BASELINE,
              file=sys.stderr)
        return 2
    try:
        t6 = side(t6p)
        imx = side(imxp)
    except tally.Fatal as e:
        print("imx95-t6-diff: %s" % e, file=sys.stderr)
        return 2
    for w in t6[2]:
        print("imx95-t6-diff: warning: T6 side: " + w, file=sys.stderr)
    for w in imx[2]:
        print("imx95-t6-diff: warning: iMX95 side: " + w, file=sys.stderr)
    res = diff(t6, imx, absent)
    lim = len(res[3]) if limit is None else limit
    sys.stdout.write((render_markdown if md else render_text)(t6p, imxp, res, lim))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
