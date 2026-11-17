#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Draft the per-image before/after of a Pi 4 dEQP-GLES3 rerun.

  pi4-gles3-rerun-diff.py --before DIR... --after DIR [--tree T] [--mustpass M]
                          [--fails TSV] [--es2-before README --es2-after README]
  pi4-gles3-rerun-diff.py --self-test

DIR is a session directory, tools/cts/log/<date>-pi4-gles3-<sha8>-rows*/ or
...-rerun/ (<LABEL>.console.txt per image); --before and --after take several
(--after too). The script edits no file and prints to stdout:

(a) A markdown table, one row per image of the tree that has a console in the
    after directories: Pass / Fail / NotSupported / other / without-result and
    a verdict (DONE, all passed | DONE | aborted: no complete deqp summary),
    before and after, and the deltas. The counts are the mustpass view of the
    image: the mustpass cases (--mustpass, default gles3-mustpass.txt) that the
    image's filters select in the tree (--tree, default pi4-gles3-tree.txt);
    a selected case without a result line is "without". Re-splits: the BEFORE
    result of an image is looked up per case, in the status of the same case
    in any before console (a case in no before console is "without"), so an
    image that was split (S32c -> S32c1..S32c4) is compared with the cases it
    now selects, under its new label, and no old label is needed. A label of an
    after console that is not in the tree is listed and left out. Images
    without an after console are not in the table.
(b) The replacement block for docs/es3/pi4-gles3-fails.tsv: the rows
    (<image>.console.txt:<Status> <case>, sorted by case name, "NoResult" for a
    case that never began) of every image of (a) from the after consoles; every
    other row of --fails (default docs/es3/pi4-gles3-fails.tsv) unchanged, unless
    its case now belongs to an image of (a) (a re-split). Rows only: the '#'
    header is the owner's to rewrite. The file is the baseline of the rows that
    were not rerun; it covers several sessions, so run on the before directories
    alone the block is NOT the file: a section lists the images whose
    before-console row count differs from the file's (stale rows).
(c) With --es2-before / --es2-after (the README.md of a Pi 4 ES 2.0 regression
    directory, tools/cts/log/2026-11-pi4-es2-<sha8>/): each table row before and
    after, marked equal, better or worse. worse = fewer Pass or more Fail /
    NotSupported / other / mustpass cases without Pass than before, an image that
    is missing after, or a verdict that is not DONE after one that was. The exit
    status is 1 when any ES 2.0 row is worse.

A tally of improved / unchanged / regressed images ends the output: improved =
more Pass and no fewer-Pass-style loss, regressed = fewer Pass or more Fail /
other, unchanged = same five counts and verdict (mixed changes count as
regressed).
"""
import argparse
import glob
import importlib.util
import os
import re
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))


def _tally_module():
    spec = importlib.util.spec_from_file_location("deqp_uart_tally", os.path.join(HERE, "deqp-uart-tally.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


TALLY = _tally_module()
STATUS_COLS = ("Pass", "Fail", "NotSupported", "other", "without")


def read_mustpass(path):
    with open(path, encoding="utf-8") as f:
        return [l.strip() for l in f if l.strip() and not l.startswith("#")]


def read_tree(path):
    """[(label, [filter patterns])] in the tree's order."""
    images, label = [], None
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            m = re.match(r"# image (\S+):", line)
            if m:
                label = m.group(1)
            elif line and not line.startswith("#") and label:
                images.append((label, line.split(",")))
                label = None
    return images


def read_consoles(dirs):
    """({label: scan}, {case: status} over all consoles, {label: complete})."""
    scans, got, complete = {}, {}, {}
    for d in dirs:
        for c in sorted(glob.glob(os.path.join(d, "*.console.txt"))):
            label = os.path.basename(c)[:-len(".console.txt")]
            with open(c, encoding="utf-8", errors="replace") as f:
                s = TALLY.scan(f.read())
            if label in scans:
                raise SystemExit("pi4-gles3-rerun-diff: image %s is in two directories (%s)" % (label, d))
            scans[label] = s
            got.update(s["got"])
            complete[label] = any("complete=1" in t for t in s["tail"])
    return scans, got, complete


def counts(cases, got):
    """(Pass, Fail, NotSupported, other, without) of the cases under got."""
    c = dict.fromkeys(STATUS_COLS, 0)
    for case in cases:
        st = got.get(case)
        c["without" if st is None else st if st in ("Pass", "Fail", "NotSupported") else "other"] += 1
    return tuple(c[k] for k in STATUS_COLS)


def verdict(c, complete):
    if not complete:
        return "aborted: no complete deqp summary"
    return "DONE, all passed" if c[1] == c[2] == c[3] == c[4] == 0 else "DONE"


def image_cases(tree, mustpass):
    out = {}
    for label, pats in tree:
        rx = TALLY.filter_regex(pats)
        out[label] = [c for c in mustpass if rx.match(c)]
    return out


def classify(b, a):
    """'improved', 'unchanged' or 'regressed' for the count tuples."""
    if a == b:
        return "unchanged"
    bad = a[0] < b[0] or a[1] > b[1] or a[3] > b[3] or a[4] > b[4]
    return "regressed" if bad else "improved"


def diff(before_dirs, after_dirs, tree, mustpass):
    """(rows, skipped labels, after got, after cases per label). A row is
    (label, before counts, before verdict, after counts, after verdict, class)."""
    _, bgot, bcomp = read_consoles(before_dirs)
    ascans, agot, acomp = read_consoles(after_dirs)
    cases = image_cases(tree, mustpass)
    rows, skipped = [], []
    for label, _ in tree:
        if label not in ascans:
            continue
        cs = cases[label]
        a = counts(cs, ascans[label]["got"])
        b = counts(cs, bgot)
        # the before verdict is that of the before console of the same label, else of
        # the cases alone (a re-split label has no before console)
        bc = bcomp.get(label, b[4] == 0)
        rows.append((label, b, verdict(b, bc), a, verdict(a, acomp[label]), classify(b, a)))
    skipped = sorted(l for l in ascans if l not in cases)
    return rows, skipped, ascans, cases


def table(rows):
    out = ["| image | before: Pass | Fail | NotSupported | other | without | verdict "
           "| after: Pass | Fail | NotSupported | other | without | verdict | delta Pass | Fail | NotSupported | other | without | change |",
           "|---|" + "---|" * 18]
    for label, b, bv, a, av, cl in rows:
        d = ["%+d" % (x - y) for x, y in zip(a, b)]
        out.append("| %s | %s | %s | %s | %s | %s | %s |" % (
            label, " | ".join(map(str, b)), bv, " | ".join(map(str, a)), av, " | ".join(d), cl))
    return out


def tsv_rows(label, cs, got):
    out = []
    for case in cs:
        st = got.get(case)
        if st is None:
            st = "NoResult"
        if st != "Pass":
            out.append((case, "%s.console.txt:%s %s\n" % (label, st, case)))
    return out


def fails_block(fails_path, ascans, cases, rows):
    """(the new block text, {label: row count in --fails}, kept rows count)."""
    rerun = {r[0] for r in rows}
    taken = {}
    for label in rerun:
        for c in cases[label]:
            taken[c] = label
    new, file_counts = [], {}
    with open(fails_path, encoding="utf-8") as f:
        for line in f:
            if line.startswith("#") or not line.strip():
                continue
            m = re.match(r"(\S+)\.console\.txt:(\S+) (\S+)$", line.rstrip("\n"))
            if not m:
                raise SystemExit("pi4-gles3-rerun-diff: %s: unreadable row: %s" % (fails_path, line.strip()))
            file_counts[m.group(1)] = file_counts.get(m.group(1), 0) + 1
            if m.group(1) in rerun or m.group(3) in taken:
                continue
            new.append((m.group(3), line if line.endswith("\n") else line + "\n"))
    kept = len(new)
    for label in sorted(rerun):
        new.extend(tsv_rows(label, cases[label], ascans[label]["got"]))
    return "".join(r for _, r in sorted(new)), file_counts, kept


def stale_section(file_counts, before_dirs, tree, mustpass):
    """Labels whose before-console not-pass count differs from the file's."""
    scans, _, _ = read_consoles(before_dirs)
    cases = image_cases(tree, mustpass)
    out = []
    for label in sorted(set(scans) | set(file_counts)):
        mine = len(tsv_rows(label, cases[label], scans[label]["got"])) if label in scans and label in cases else None
        theirs = file_counts.get(label, 0)
        if mine is not None and mine != theirs:
            out.append("%s: fails.tsv %d rows, before consoles %d" % (label, theirs, mine))
        elif mine is None and theirs and label not in cases:
            out.append("%s: fails.tsv %d rows, image not in the tree" % (label, theirs))
    return out


ES2_ROW = re.compile(r"^\|\s*(\d+)\s*\|")


def read_es2(path):
    """{image: {column: value}} of the README's results table."""
    rows, cols = {}, None
    with open(path, encoding="utf-8") as f:
        for line in f:
            if not line.startswith("|"):
                if cols and rows:
                    break
                continue
            cells = [c.strip() for c in line.strip().strip("|").split("|")]
            if cells[0] == "row":
                cols = cells
            elif cols and ES2_ROW.match(line) and len(cells) == len(cols):
                d = dict(zip(cols, cells))
                rows[d["image"]] = d
    if not rows:
        raise SystemExit("pi4-gles3-rerun-diff: %s: no results table (| row | image | ...)" % path)
    return rows


def num(s):
    s = s.replace(",", "")
    return int(s) if s.isdigit() else None


def es2_compare(before, after):
    """[(image, before text, after text, mark)]; mark in equal, better, worse."""
    out = []
    for img, b in before.items():
        a = after.get(img)
        keys = [k for k in ("mustpass", "Pass", "Fail", "NotSupported", "other") if k in b]
        bt = "%s | %s" % (" ".join("%s %s" % (k, b[k]) for k in keys), b.get("verdict", ""))
        if a is None:
            out.append((img, bt, "missing", "worse"))
            continue
        at = "%s | %s" % (" ".join("%s %s" % (k, a.get(k, "?")) for k in keys), a.get("verdict", ""))
        if bt == at:
            out.append((img, bt, at, "equal"))
            continue
        g = lambda d, k: num(d.get(k, "")) or 0
        worse = (g(a, "Pass") < g(b, "Pass") or g(a, "Fail") > g(b, "Fail") or g(a, "NotSupported") > g(b, "NotSupported")
                 or g(a, "other") > g(b, "other")
                 or (b.get("verdict", "").startswith("DONE") and not a.get("verdict", "").startswith("DONE")))
        better = (g(a, "Pass") > g(b, "Pass") or g(a, "Fail") < g(b, "Fail") or g(a, "NotSupported") < g(b, "NotSupported")
                  or g(a, "other") < g(b, "other"))
        out.append((img, bt, at, "worse" if worse else "better" if better else "equal"))
    for img in after:
        if img not in before:
            out.append((img, "-", "new image", "equal"))
    return out


def report(args, out):
    mustpass = read_mustpass(args.mustpass)
    tree = read_tree(args.tree)
    rows, skipped, ascans, cases = diff(args.before, args.after, tree, mustpass)
    out.append("## (a) per image, before / after")
    out.append("")
    out.extend(table(rows))
    out.append("")
    for l in skipped:
        out.append("(after console of image %s is not in the tree: left out)" % l)
    tally = {"improved": [], "unchanged": [], "regressed": []}
    for r in rows:
        tally[r[5]].append(r[0])
    out.append("")
    out.append("## (b) block for docs/es3/pi4-gles3-fails.tsv")
    out.append("")
    block, file_counts, kept = fails_block(args.fails, ascans, cases, rows)
    out.append("rows: %d (%d kept from %s, %d from the after consoles of %d images)" % (
        block.count("\n"), kept, os.path.basename(args.fails), block.count("\n") - kept, len(rows)))
    out.append("")
    out.append("```tsv")
    out.append(block.rstrip("\n"))
    out.append("```")
    stale = stale_section(file_counts, args.before, tree, mustpass)
    out.append("")
    out.append("fails.tsv rows that the before consoles do not give (the file is a merge of older sessions): %s" % (
        "none" if not stale else ""))
    out.extend("- " + s for s in stale)
    worse = False
    if args.es2_before and args.es2_after:
        out.append("")
        out.append("## (c) ES 2.0 regression rows")
        out.append("")
        out.append("| image | before | after | mark |")
        out.append("|---|---|---|---|")
        for img, bt, at, mark in es2_compare(read_es2(args.es2_before), read_es2(args.es2_after)):
            out.append("| %s | %s | %s | %s |" % (img, bt, at, mark))
            worse |= mark == "worse"
        out.append("")
        out.append("ES 2.0: %s" % ("a row is WORSE" if worse else "no row worse"))
    out.append("")
    out.append("## tally")
    out.append("")
    for k in ("improved", "unchanged", "regressed"):
        out.append("%s: %d%s" % (k, len(tally[k]), " (%s)" % " ".join(tally[k]) if tally[k] else ""))
    return 1 if worse else 0


def self_test():
    fails = []

    def check(name, cond, detail=""):
        print("  ok    %s" % name if cond else "  FAIL  %s %s" % (name, detail))
        if not cond:
            fails.append(name)

    with tempfile.TemporaryDirectory() as td:
        def w(rel, text):
            p = os.path.join(td, rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "w", encoding="utf-8") as f:
                f.write(text)
            return p

        names = ["dEQP-GLES3.g.a.c%d" % i for i in range(1, 5)] + ["dEQP-GLES3.g.b.c%d" % i for i in range(1, 4)]
        must = w("must.txt", "\n".join(names) + "\n")
        tree = w("tree.txt", "# image X1: 4\ndEQP-GLES3.g.a.*\n# image Y1: 2\ndEQP-GLES3.g.b.c1,dEQP-GLES3.g.b.c2\n"
                 "# image Y2: 1\ndEQP-GLES3.g.b.c3\n")

        def console(res, complete=True):
            lines = ["deqp: %s %s" % (s, c) for c, s in res]
            lines.append("deqp: done: %d passed, 0 failed, 0 not supported, 0 warnings, complete=%d" % (
                sum(s == "Pass" for _, s in res), 1 if complete else 0))
            return "\n".join(lines) + "\n"

        c = lambda i, s: ("dEQP-GLES3.g.a.c%d" % i, s)
        # before: X1 has one Fail; Y (old label) ran b.c1 Pass, b.c2 Fail and aborted before c3
        w("b/X1.console.txt", console([c(1, "Pass"), c(2, "Fail"), c(3, "Pass"), c(4, "NotSupported")]))
        w("b/Y.console.txt", console([("dEQP-GLES3.g.b.c1", "Pass"), ("dEQP-GLES3.g.b.c2", "Fail")], False))
        # after: X1 better (all Pass), Y1 as before, Y2 now runs (new label of a re-split)
        w("a/X1.console.txt", console([c(i, "Pass") for i in range(1, 5)]))
        w("a/Y1.console.txt", console([("dEQP-GLES3.g.b.c1", "Pass"), ("dEQP-GLES3.g.b.c2", "Fail")]))
        w("a/Y2.console.txt", console([("dEQP-GLES3.g.b.c3", "Pass")]))
        w("a/Z9.console.txt", console([]))
        fails_tsv = w("fails.tsv", "# header\n"
                      "X1.console.txt:Fail dEQP-GLES3.g.a.c2\nX1.console.txt:NotSupported dEQP-GLES3.g.a.c4\n"
                      "Y.console.txt:Fail dEQP-GLES3.g.b.c2\nY.console.txt:NoResult dEQP-GLES3.g.b.c3\n"
                      "Q1.console.txt:Fail dEQP-GLES3.g.q.c1\n")
        ns = argparse.Namespace(before=[os.path.join(td, "b")], after=[os.path.join(td, "a")], tree=tree,
                                mustpass=must, fails=fails_tsv, es2_before=None, es2_after=None)
        lines = []
        rc = report(ns, lines)
        text = "\n".join(lines)
        check("exit 0 without ES 2.0 options", rc == 0)
        check("X1 improved by 2 Pass (before 2 Pass 1 Fail 1 NotSupported)",
              "| X1 | 2 | 1 | 1 | 0 | 0 | DONE | 4 | 0 | 0 | 0 | 0 | DONE, all passed | +2 | -1 | -1 | +0 | +0 | improved |" in text, text)
        check("Y1 unchanged", re.search(r"\| Y1 \|.*\| unchanged \|", text) is not None)
        check("re-split Y2 compares with the old console's cases (before 'without')",
              "| Y2 | 0 | 0 | 0 | 0 | 1 | aborted: no complete deqp summary | 1 | 0 | 0 | 0 | 0 | DONE, all passed | +1 | +0 | +0 | +0 | -1 | improved |" in text, text)
        check("an after console not in the tree is named and left out",
              "Z9 is not in the tree" in text and "| Z9 |" not in text)
        block = text.split("```tsv\n")[1].split("\n```")[0] + "\n"
        check("block: rerun images from the after run, others kept, sorted by case",
              block == "Y1.console.txt:Fail dEQP-GLES3.g.b.c2\nQ1.console.txt:Fail dEQP-GLES3.g.q.c1\n", repr(block))
        check("the stale-row section names Y (its label is not in the tree)", "Y: fails.tsv 2 rows, image not in the tree" in text, text)
        check("tally counts", "improved: 2 (X1 Y2)" in text and "unchanged: 1 (Y1)" in text and "regressed: 0" in text, text)
        # self against self: every delta 0, block equals the rerun images' rows
        ns.before = ns.after
        lines = []
        report(ns, lines)
        t2 = "\n".join(lines)
        check("before = after: all deltas 0", all(set(re.findall(r"[+-]\d+", ln)) <= {"+0"} for ln in lines if ln.startswith("| ") and "after" not in ln), t2)
        check("before = after: tally has no improved/regressed", "improved: 0" in t2 and "regressed: 0" in t2 and "unchanged: 3" in t2)
        # a regression
        w("r/X1.console.txt", console([c(1, "Pass"), c(2, "Fail"), c(3, "Fail"), c(4, "Pass")]))
        ns.before, ns.after = [os.path.join(td, "a")], [os.path.join(td, "r")]
        lines = []
        report(ns, lines)
        check("a new Fail is regressed", "regressed: 1 (X1)" in "\n".join(lines))
        # ES 2.0
        hdr = "| row | image | mustpass | Pass | Fail | NotSupported | other | begun | run | verdict |\n|---|---|---|---|---|---|---|---|---|---|\n"
        e = lambda p, f, v="DONE": "| 1 | D | 10 | %d | %d | 0 | 0 | 10 | 1 s | %s |\n| 2 | T1 | 5 | 5 | 0 | 0 | 0 | 5 | 1 s | DONE |\n| | total | 15 | | | | | | | |\n" % (p, f, v)
        e0 = w("e0.md", "# t\n" + hdr + e(10, 0))
        check("ES 2.0 against itself: all equal", [m for *_, m in es2_compare(read_es2(e0), read_es2(e0))] == ["equal", "equal"])
        e1 = w("e1.md", "# t\n" + hdr + e(9, 1))
        e2 = w("e2.md", "# t\n" + hdr + e(10, 0).replace("| 5 | 5 | 0", "| 5 | 4 | 1", 1))
        check("fewer Pass is worse", [m for *_, m in es2_compare(read_es2(e0), read_es2(e1))][0] == "worse")
        check("more Pass is better", [m for *_, m in es2_compare(read_es2(e1), read_es2(e0))][0] == "better")
        check("a new Fail in another row is worse", [m for *_, m in es2_compare(read_es2(e0), read_es2(e2))] == ["equal", "worse"])
        ns.es2_before, ns.es2_after = e0, e1
        check("exit 1 when an ES 2.0 row is worse", report(ns, []) == 1)
        ns.es2_before, ns.es2_after = e0, e0
        check("exit 0 for ES 2.0 against itself", report(ns, []) == 0)
    print("pi4-gles3-rerun-diff self-test: %s" % ("FAILED (%d)" % len(fails) if fails else "ok"))
    return 1 if fails else 0


def main():
    ap = argparse.ArgumentParser(description="Per-image before/after of a Pi 4 dEQP-GLES3 rerun (edits no file)")
    ap.add_argument("--before", nargs="+", metavar="DIR")
    ap.add_argument("--after", nargs="+", metavar="DIR")
    ap.add_argument("--tree", default=os.path.join(HERE, "pi4-gles3-tree.txt"))
    ap.add_argument("--mustpass", default=os.path.join(HERE, "gles3-mustpass.txt"))
    ap.add_argument("--fails", default=os.path.join(ROOT, "docs", "es3", "pi4-gles3-fails.tsv"))
    ap.add_argument("--es2-before")
    ap.add_argument("--es2-after")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.before or not a.after:
        ap.error("--before and --after are required")
    if bool(a.es2_before) != bool(a.es2_after):
        ap.error("--es2-before and --es2-after go together")
    out = []
    rc = report(a, out)
    print("\n".join(out))
    return rc


if __name__ == "__main__":
    sys.exit(main())
