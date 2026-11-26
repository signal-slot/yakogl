#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Per-case diff of Verdin iMX95 (Mali-G310) dEQP-GLES2 slices against the
NanoPC-T6's (Mali-G610).

  python3 -I tools/cts/imx95-t6-diff.py [--markdown|--tsv] [--absent] [--limit N]
                                        [--rows LABEL[,LABEL|LABEL-LABEL]...]
                                        --imx95 SLICE [--imx95 SLICE]...
                                        [--t6 SLICE]...

A SLICE is a directory of the format tools/cts/mali-deqp-tally.py reads: an
iMX95 one as tools/cts/imx95-uart-slice.py writes it, a T6 one as
mali-deqp-run.sh does. Without --t6 the T6 side is the baseline,
tools/cts/log/2026-11-24-t6-gles2-S*-8ff01055 (the whole tree at 8ff01055).
The readers and the rerun/replaces rule are the tally's, imported from it:
a case superseded within one side is not counted.

Rows. A session covers some rows of the tree only (labels T6S<session>R<NN>,
the Nth '# image' row of that session in tools/cts/mali-gles2-tree.txt). The
T6 slices' chunks.tsv name their rows (label, cases) and their results.tsv
lists the cases row by row, which gives the row of each T6 case. The compared
rows are those the iMX95 slices ran (the labels of their rows.tsv, else of
their chunks.tsv), or --rows LABEL[,LABEL|LABEL-LABEL]... (a range is two
labels of one session, the second also as R<NN>). Only cases of those rows are
compared. A T6 case of such a row that the iMX95 results lack is Lost: a
difference, state "Lost" (cause "missing"), counted apart in the summary. The
rows outside the set are listed in the summary as not run (--absent makes
their cases differences, state "absent"). A case the iMX95 side has that the
T6 map has not for any row (the six extra cases of T6S1R05 and R09, or a
case of the slice's extra.tsv) is "extra": counted, listed, no difference.
An iMX95 case of a T6 row outside the set (--rows narrower than the slice)
is left out and counted.

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
--limit N lists at most N cases (the counts stay whole). A case of a row outside
the set is not a difference; --absent counts it as one (state "absent"). Text
by default, --markdown for tables. A slice against itself reports 0
differences. --tsv prints no report but one line a difference of the
compared rows (the cases the text lists as per case, --limit ignored), the
columns case, T6 result, iMX95 result, tab-separated, no header: the input
of imx95-g310-cause.py. A result is the dEQP status; a Lost case (missing, hang, crash, not-run, on
either side) reads "Lost" and ResourceError reads InternalError, the cause
tool's results. Extra cases and rows not run (also with --absent) are not
printed. Exit 0 (differences are the report, not an error), 2 for a
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
USAGE = ("usage: imx95-t6-diff.py [--markdown|--tsv] [--absent] [--limit N] "
         "[--rows LABEL[,LABEL|LABEL-LABEL]...] "
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


LABEL = re.compile(r"^T6S(\d+)R(\d+)$")


def label_key(lab):
    m = LABEL.match(lab)
    return (int(m.group(1)), int(m.group(2))) if m else (1 << 30, 0)


def t6_rows(paths):
    """({label: [cases]}, {case: label}) of T6 slices: chunks.tsv's rows in
    order take their cases from results.tsv's lines in order."""
    rows, of = {}, {}
    for p in paths:
        s = tally.load_slice(p)
        cases = [f[0] for _ln, f, _w in s["results"]]
        chunks = [f for _ln, f, _w in s["chunks"]]
        want = sum(int(c[2]) for c in chunks if c[2].isdigit())
        if want != len(cases):
            raise tally.Fatal("%s: chunks.tsv names %d cases, results.tsv has %d lines"
                              % (s["name"], want, len(cases)))
        at = 0
        for c in chunks:
            lab = c[1]
            n = int(c[2]) if c[2].isdigit() else 0
            if lab in rows:
                continue
            rows[lab] = cases[at:at + n]
            for x in rows[lab]:
                of.setdefault(x, lab)
            at += n
    return rows, of


def tsv_col(path, col):
    """Column col of a header-led tsv, or None when the file is absent."""
    try:
        with open(path, encoding="utf-8") as f:
            lines = f.read().splitlines()
    except OSError:
        return None
    if not lines:
        return []
    head = lines[0].split("\t")
    if col not in head:
        return []
    i = head.index(col)
    return [ln.split("\t")[i] for ln in lines[1:] if ln.strip() and len(ln.split("\t")) > i]


def imx_rows(paths):
    """(labels run, [(label, extra case)]) of the iMX95 slices."""
    labs, extra = [], []
    for p in paths:
        p = p.rstrip("/")
        got = tsv_col(os.path.join(p, "rows.tsv"), "label")
        if not got:
            got = tsv_col(os.path.join(p, "chunks.tsv"), "label") or []
        labs += [x for x in got if x not in labs]
        ex = tsv_col(os.path.join(p, "extra.tsv"), "case") or []
        lb = tsv_col(os.path.join(p, "extra.tsv"), "label") or []
        extra += list(zip(lb, ex))
    return labs, extra


def parse_rows(spec, known):
    """The labels of --rows SPEC; tally.Fatal for one the T6 map lacks."""
    out = []
    for part in spec.split(","):
        part = part.strip()
        if not part:
            continue
        if part in known:
            got = [part]
        elif "-" in part:
            lo, hi = part.split("-", 1)
            m1 = LABEL.match(lo)
            if re.match(r"^R?\d+$", hi) and m1:
                hi = "T6S%sR%02d" % (m1.group(1), int(hi.lstrip("R")))
            m2 = LABEL.match(hi)
            if not (m1 and m2 and m1.group(1) == m2.group(1)):
                raise tally.Fatal("--rows %r: not two labels of one session" % part)
            a, b = label_key(lo), label_key(hi)
            got = sorted((k for k in known if a <= label_key(k) <= b), key=label_key)
            if not got or lo not in known or hi not in known:
                raise tally.Fatal("--rows %r: a label of the range is not a T6 row" % part)
        else:
            raise tally.Fatal("--rows %r: not a T6 row label" % part)
        out += [x for x in got if x not in out]
    if not out:
        raise tally.Fatal("--rows is empty")
    return out


def restrict(t6, imx, rows, of, rowset, extra_cases, absent=False):
    """Cut both sides to the rows of rowset. Returns (t6, imx, info): the
    restricted (dict, order, warnings) triples and the summary's facts."""
    a, a_order, aw = t6
    b, b_order, bw = imx
    ta = {c: v for c, v in a.items() if of.get(c) in rowset}
    ta_order = [c for c in a_order if c in ta]
    tb = {c: v for c, v in b.items() if of.get(c) in rowset}
    tb_order = [c for c in b_order if c in tb]
    extra = [c for c in b_order if c not in of]
    extra += [c for c in extra_cases if c not in of and c not in extra]
    outside = [c for c in b_order if c in of and of[c] not in rowset]
    norun = {}
    for lab in sorted(rows, key=label_key):
        if lab not in rowset:
            norun[lab] = len(rows[lab])
    info = {"extra": extra, "outside": outside, "norun": norun,
            "norun_cases": [c for lab in norun for c in rows[lab] if c in a],
            "t6all": a, "run": sorted(rowset, key=label_key)}
    return (ta, ta_order, aw), (tb, tb_order, bw), info


def diff(t6, imx, info=None, absent=False):
    """(both, lost, extra, [(case, t6 (state, cause), imx (state, cause))])."""
    a, a_order, _ = t6
    b, b_order, _ = imx
    both = [c for c in a_order if c in b]
    lost = [c for c in a_order if c not in b]
    out = []
    for c in both:
        (sa, ca, ra), (sb, cb, rb) = a[c], b[c]
        if sa != sb or (sa != "Pass" and not same_refusal(ra, rb)):
            out.append((c, (sa, ca), (sb, cb)))
    out += [(c, a[c][:2], ("Lost", "missing")) for c in lost]
    if absent and info:
        t6all = info["t6all"]
        out += [(c, t6all[c][:2], ("absent", "-")) for c in info["norun_cases"]]
    return len(both), len(lost), len(info["extra"]) if info else 0, out


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


def labels(labs):
    return ", ".join(labs) if labs else "none"


def summary(info, res):
    """The row lines of both renderings: [(text)]."""
    both, lost, extra, _rows = res
    nr = info["norun"]
    return [
        "rows compared (%d): %s" % (len(info["run"]), labels(info["run"])),
        "rows not run (%d, %d T6 cases): %s"
        % (len(nr), sum(nr.values()), labels(list(nr))),
        "cases on both sides: %d; Lost (T6 case of a run row, missing from the iMX95 "
        "results): %d; extra (iMX95 case not in the T6 map, no difference): %d"
        % (both, lost, extra),
        "iMX95 cases of rows outside the set, left out: %d" % len(info["outside"]),
    ]


def render_text(t6p, imxp, res, limit, info):
    by_t, by_c = counts(res[3])
    rows = res[3]
    out = ["T6 slices: " + names(t6p), "iMX95 slices: " + names(imxp)]
    out += summary(info, res)
    if info["extra"]:
        out.append("extra cases: " + ", ".join(info["extra"][:20])
                   + (" ... and %d more" % (len(info["extra"]) - 20)
                      if len(info["extra"]) > 20 else ""))
    out.append("differences: %d" % len(rows))
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


def render_markdown(t6p, imxp, res, limit, info):
    both, oa, ob, rows = res
    by_t, by_c = counts(rows)
    out = ["T6 slices: " + ", ".join("`%s`" % n for n in names(t6p).split(", ")) + ".",
           "iMX95 slices: " + ", ".join("`%s`" % n for n in names(imxp).split(", ")) + ".",
           ""]
    out += ["- " + x for x in summary(info, res)]
    if info["extra"]:
        out.append("- extra cases: " + ", ".join("`%s`" % c for c in info["extra"][:20])
                   + (" ... and %d more" % (len(info["extra"]) - 20)
                      if len(info["extra"]) > 20 else ""))
    out += ["", "Differences: %d." % len(rows)]
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


TSV_STATE = {"hang": "Lost", "crash": "Lost", "not-run": "Lost",
             "ResourceError": "InternalError"}


def render_tsv(t6p, imxp, res, limit, info):
    """case, T6 result, iMX95 result per difference (no absent, no header)."""
    return "".join("%s\t%s\t%s\n" % (c, TSV_STATE.get(sa, sa), TSV_STATE.get(sb, sb))
                   for c, (sa, _ca), (sb, _cb) in res[3] if sb != "absent")


def main(argv):
    md = absent = tsv = False
    limit = None
    rows_spec = None
    t6p, imxp = [], []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--markdown":
            md = True
        elif a == "--tsv":
            tsv = True
        elif a == "--absent":
            absent = True
        elif a in ("--imx95", "--t6", "--limit", "--rows"):
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
            elif a == "--rows":
                rows_spec = v if rows_spec is None else rows_spec + "," + v
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
        rows, of = t6_rows(t6p)
        run, extra_rows = imx_rows(imxp)
        if rows_spec is not None:
            run = parse_rows(rows_spec, rows)
        else:
            miss = [x for x in run if x not in rows]
            if miss:
                raise tally.Fatal("iMX95 row(s) not in the T6 slices: " + ", ".join(miss))
        if not run:
            raise tally.Fatal("the iMX95 slices name no row; give --rows")
    except tally.Fatal as e:
        print("imx95-t6-diff: %s" % e, file=sys.stderr)
        return 2
    for w in t6[2]:
        print("imx95-t6-diff: warning: T6 side: " + w, file=sys.stderr)
    for w in imx[2]:
        print("imx95-t6-diff: warning: iMX95 side: " + w, file=sys.stderr)
    t6r, imxr, info = restrict(t6, imx, rows, of, set(run),
                               [c for _l, c in extra_rows], absent)
    res = diff(t6r, imxr, info, absent)
    lim = len(res[3]) if limit is None or tsv else limit
    render = render_tsv if tsv else render_markdown if md else render_text
    sys.stdout.write(render(t6p, imxp, res, lim, info))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
