#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""glmark2 host profile: a before/after table of ms per frame per scene.

usage: compare.py [--entry-points [N]] [--metric gl|wall|cpu]
                  [--before-label L] [--after-label L] [--title T]
                  <before-out> <after-out> <powervr|vivante|v3d>
       compare.py [same options] --before DIR [DIR ...] --after DIR [DIR ...]
                  <powervr|vivante|v3d>      (or --gpu G in place of the last word)

Reads two output directories of run.sh GM2_PROFILE=1 runs (entries.txt and
logs/<gpu>/<i>.{log,rc}, the files report.py --profile reads) with report.py's
own parser, and prints a markdown table with one row per scene present in
both: the counted frames' ms a frame before and after, the delta (after
minus before) and the ratio (after / before). The default metric is the time
spent in GL and EGL calls (`gl`, the figure report.py's profile ranks the
entry points by); `wall` is the whole frame's wall time (glmark2 itself
included) and `cpu` the thread's cpu time. With --entry-points the table also
lists the N (default 3) entry points that took most wall time a frame in the
before run, each with its before and after ms.

Repeats (FINDINGS-variance.md, 'Recommendation'): give each side several
output directories, one per repeat, with --before and --after. With more than
one directory on a side each scene's cell is the minimum over that side's
repeats of the chosen metric, not the mean, and the table gains the repeat
count (n) and the CV (sample standard deviation over mean) of each side. A
delta whose size is below the before side's CV * minimum is within the noise
floor and carries a trailing '~'. A repeat whose frame wall time exceeds its
cpu time by more than 5% was disturbed by the host: it is left out of the
minimum and the CV and counted in a footnote (a scene with no undisturbed
repeat keeps all of them and the footnote says so). With --entry-points the
entry point times come from the repeat that gave the minimum. A side given one
directory in the --before/--after form, and the positional form, print the
single-run table above.
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import report  # noqa: E402

MS = 1e6
GPUS = ("powervr", "vivante", "v3d")


def load(out, gpu):
    """{scene: (ms a frame by metric, {entry point: wall ms a frame})} for the
    scenes of one run that have counted frames."""
    entries, runs = report.read_runs(out, [gpu])
    res = {}
    for i, e in enumerate(entries):
        r = runs[(gpu, i)]
        f = r["gl_frames"]
        if not f or not f["counted"]:
            continue
        n = f["counted"]
        eps = {name: t["frames"][1] / n / MS for name, t in r["gl_time"].items() if t["frames"][0]}
        res[e] = ({"gl": f["gl_wall"] / n / MS, "wall": f["wall"] / n / MS, "cpu": f["cpu"] / n / MS}, eps)
    return res


DISTURBED = 1.05


def pool(runs):
    """{scene: [load() cell of each repeat that has the scene]}."""
    res = {}
    for r in runs:
        for e, v in r.items():
            res.setdefault(e, []).append(v)
    return res


def reduce_side(runs, metric):
    """{scene: (cell, n, cv, dropped, all_disturbed)}; cell is (metrics, eps) of
    the undisturbed repeat with the lowest metric."""
    res = {}
    for e, reps in pool(runs).items():
        good = [v for v in reps if v[0]["wall"] <= v[0]["cpu"] * DISTURBED]
        all_dist = not good
        use = reps if all_dist else good
        best = min(use, key=lambda v: v[0][metric])
        xs = [v[0][metric] for v in use]
        cv = None
        if len(xs) > 1 and sum(xs):
            m = sum(xs) / len(xs)
            cv = (sum((x - m) ** 2 for x in xs) / (len(xs) - 1)) ** 0.5 / m
        res[e] = (best, len(use), cv, len(reps) - len(good) if not all_dist else 0, all_dist)
    return res


def fmt_cv(cv):
    return "-" if cv is None else "%.0f%%" % (cv * 100)


def table_repeats(bruns, aruns, args):
    bs, as_ = reduce_side(bruns, args.metric), reduce_side(aruns, args.metric)
    before, after = {e: v[0] for e, v in bs.items()}, {e: v[0] for e, v in as_.items()}
    bl, al = args.before_label, args.after_label
    o = ["| scene | %s min ms/frame | n | CV | %s min ms/frame | n | CV | delta ms | ratio |" % (bl, al),
         "|---|---|---|---|---|---|---|---|---|"]
    detail, notes = [], []
    for e in before:
        if e not in after:
            continue
        b, a = before[e][0][args.metric], after[e][0][args.metric]
        cv = bs[e][2]
        noise = " ~" if cv is not None and abs(a - b) < cv * b else ""
        o.append("| `%s` | %.3f | %d | %s | %.3f | %d | %s | %+.3f%s | %s |"
                 % (report.md_escape(e), b, bs[e][1], fmt_cv(cv), a, as_[e][1], fmt_cv(as_[e][2]), a - b, noise,
                    ratio(b, a)))
        for side, rs in ((bl, bs), (al, as_)):
            if rs[e][3]:
                notes.append("%d %s repeat%s of `%s`" % (rs[e][3], side, "" if rs[e][3] == 1 else "s", report.md_escape(e)))
            if rs[e][4]:
                notes.append("every %s repeat of `%s` (kept, none undisturbed)" % (side, report.md_escape(e)))
        if args.entry_points:
            beps, aeps = before[e][1], after[e][1]
            top = sorted(beps, key=lambda k: -beps[k])[:args.entry_points]
            detail.append((e, top, beps, aeps))
    o += entry_rows(detail, args)
    o += ["", "Minimum of the repeats of the chosen metric; CV = sample standard deviation / mean of the repeats "
          "kept; '~' marks a delta smaller than the %s CV * minimum (within noise)." % bl]
    if notes:
        o += ["", "Left out as disturbed (wall time more than 5%% above cpu time): %s." % "; ".join(notes)]
    skipped = sorted(set(before) ^ set(after))
    if skipped:
        o += ["", "Not in both runs (left out): %s." % ", ".join("`%s`" % report.md_escape(e) for e in skipped)]
    return "\n".join(o)


def entry_rows(detail, args):
    o = []
    if detail:
        o += ["", "Top entry points by wall ms a frame in the %s run:" % args.before_label, "",
              "| scene | entry point | %s ms/frame | %s ms/frame | delta ms |" % (args.before_label, args.after_label),
              "|---|---|---|---|---|"]
        for e, top, beps, aeps in detail:
            for k in top:
                b, a = beps[k], aeps.get(k, 0.0)
                o.append("| `%s` | `%s` | %.3f | %.3f | %+.3f |" % (report.md_escape(e), k, b, a, a - b))
    return o


def ratio(b, a):
    return "%.2fx" % (a / b) if b else "-"


def table(before, after, args):
    o = ["| scene | %s ms/frame | %s ms/frame | delta ms | ratio |" % (args.before_label, args.after_label),
         "|---|---|---|---|---|"]
    detail = []
    for e in before:
        if e not in after:
            continue
        b, a = before[e][0][args.metric], after[e][0][args.metric]
        o.append("| `%s` | %.3f | %.3f | %+.3f | %s |" % (report.md_escape(e), b, a, a - b, ratio(b, a)))
        if args.entry_points:
            beps, aeps = before[e][1], after[e][1]
            top = sorted(beps, key=lambda k: -beps[k])[:args.entry_points]
            detail.append((e, top, beps, aeps))
    o += entry_rows(detail, args)
    skipped = sorted(set(before) ^ set(after))
    if skipped:
        o += ["", "Not in both runs (left out): %s." % ", ".join("`%s`" % report.md_escape(e) for e in skipped)]
    return "\n".join(o)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("paths", nargs="*", metavar="before-out after-out gpu")
    ap.add_argument("--before", nargs="+", metavar="DIR")
    ap.add_argument("--after", nargs="+", metavar="DIR")
    ap.add_argument("--gpu", choices=GPUS)
    ap.add_argument("--entry-points", nargs="?", type=int, const=3, default=0, metavar="N")
    ap.add_argument("--metric", choices=("gl", "wall", "cpu"), default="gl")
    ap.add_argument("--before-label", default="before")
    ap.add_argument("--after-label", default="after")
    ap.add_argument("--title")
    args = ap.parse_args()
    if args.before or args.after:
        if not (args.before and args.after):
            ap.error("--before and --after go together")
        if args.gpu is None and args.after[-1] in GPUS:
            args.gpu = args.after.pop()
        if args.gpu is None and len(args.paths) == 1 and args.paths[0] in GPUS:
            args.gpu = args.paths[0]
        elif args.paths:
            ap.error("with --before/--after, give only the gpu besides")
        if args.gpu is None:
            ap.error("the gpu (powervr, vivante or v3d) is missing")
        bdirs, adirs = args.before, args.after
    else:
        if len(args.paths) != 3 or args.paths[2] not in GPUS:
            ap.error("usage: <before-out> <after-out> <powervr|vivante|v3d>")
        bdirs, adirs, args.gpu = [args.paths[0]], [args.paths[1]], args.paths[2]
    bruns, aruns = [load(d, args.gpu) for d in bdirs], [load(d, args.gpu) for d in adirs]
    repeats = len(bdirs) > 1 or len(adirs) > 1
    before, after = bruns[0], aruns[0]
    if repeats:
        before, after = reduce_side(bruns, args.metric), reduce_side(aruns, args.metric)
    if not set(before) & set(after):
        sys.exit("compare.py: no scene with counted frames in both runs")
    what = {"gl": "time in GL and EGL calls", "wall": "frame wall time", "cpu": "frame cpu time"}[args.metric]
    print("%s\n" % (args.title or "glmark2 host profile, %s: %s, ms per frame" % (args.gpu, what)))
    print(table_repeats(bruns, aruns, args) if repeats else table(before, after, args))


if __name__ == "__main__":
    main()
