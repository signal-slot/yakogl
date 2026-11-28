#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""glmark2-one-commit-issues.py - issue comment texts for the one-commit glmark2 docs change.

    tools/cts/glmark2-one-commit-issues.py --am62p DIR --pi4 DIR --imx8mp DIR --commit SHA

Each DIR is one board's common record, records/<board>/common-<sha8>/ (what
board-stage.sh common wrote; read by tools/cts/common-row.py, which this imports).
It prints the comment a person posts on #1196, #1209, #15, #17 and #1606 (agents
never run gh) and a new-issue text for every scene whose picture differs from
tools/cts/glmark2-pictures.txt. Slow-scene figures are recorded, never judged. An
i.MX8MP record with a `measured-on` file naming armadillo-x2 is stated as measured on
the Armadillo-X2. Exit 0: texts printed; 1: a record's verdict is fail (the texts say
what does not close); 2: unreadable record or the records and --commit disagree on the
commit. Paths printed are relative to records/; no host, address or credential is read.
"""
import argparse
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BOARDS = ("am62p", "pi4", "imx8mp")
NAMES = {"am62p": "Toradex Verdin AM62P", "pi4": "Raspberry Pi 4", "imx8mp": "Toradex Verdin iMX8M Plus"}
OLD_PI4_TERRAIN = 161.79
OLD_PI4_COMMIT = "c4af4917"
PI4_NOTE = "docs/notes/pi4-glmark2-pictures-by-eye.md"


def load(name, fname):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, fname))
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def is_x2(d):
    try:
        with open(os.path.join(d["record"], "measured-on"), encoding="utf-8") as f:
            return re.search(r"(?i)\barmadillo-x2\b", f.read()) is not None
    except OSError:
        return False


def slow(d, key):
    sc = d["slow"].get(key)
    return None if sc is None or sc["ms"] is None else sc


def slow_fmt(d, key):
    sc = slow(d, key)
    return "%s absent" % key if sc is None else "%s %.2f ms (%d FPS)" % (key, sc["ms"], sc["fps"])


def where(d, cr):
    return cr.rel(d["record"]) + "/"


def measured(d):
    return "measured on the Armadillo-X2 (same i.MX8MP SoC and GC7000UL GPU; the Verdin i.MX8MP is away)" \
        if d.get("x2") else "measured on the %s" % NAMES[d["board_key"]]


def t1196(ds, commit, cr):
    ok = all(not d["fails"] for d in ds.values())
    L = ["#1196: the README glmark2 table from one main commit, `%s`" % commit, ""]
    for b in BOARDS:
        d = ds[b]
        L.append("- %s: score %s; %d scenes, %d failed, %s GL errors, %d out-of-memory scenes; "
                 "gl_test %s; GM `verdict=%s`, GT `verdict=%s`; %s%s; record `%s`" % (
                     NAMES[b], d["score"], d["nscenes"], d["failed"], d["gl_errors"], len(d["oom"]),
                     cr.gt_text(d), d["gm_verdict"], d["gt_verdict"], d["date"],
                     " (measured on the Armadillo-X2)" if d.get("x2") else "", where(d, cr)))
    L.append("")
    if ok:
        L.append("All three boards ran GM (glmark2-es2 800x600) and GT (gl_test) at `%s`: 33 scenes, 0 failed, "
                 "0 GL errors, 0 out-of-memory scenes. README.md's table, the three RESULTS.md 'Common set' rows, "
                 "STATUS.md and website/data.js take these scores. Closes #1196." % commit)
    else:
        L.append("Does NOT close #1196: " + "; ".join(
            "%s: %s" % (NAMES[b], "; ".join(ds[b]["fails"])) for b in BOARDS if ds[b]["fails"]))
    return "\n".join(L)


def t1209(d, commit, cr):
    sc = slow(d, "terrain")
    L = ["#1209: Raspberry Pi 4 terrain at `%s`" % commit, ""]
    if sc is None:
        L.append("The record has no terrain figure (%s); #1209 stays open." % where(d, cr))
        return "\n".join(L)
    ms = sc["ms"]
    L.append("[terrain] %.2f ms/frame (%d FPS) at `%s`, record `%s`; %.2f ms at `%s` before." % (
        ms, sc["fps"], commit, where(d, cr), OLD_PI4_TERRAIN, OLD_PI4_COMMIT))
    if ms < OLD_PI4_TERRAIN:
        L.append("The figure is below the %.2f ms of the issue: #1209 can be closed on this record." % OLD_PI4_TERRAIN)
    else:
        L.append("The figure is not below the %.2f ms of the issue: #1209 stays open." % OLD_PI4_TERRAIN)
    return "\n".join(L)


def t15(d, commit, cr):
    L = ["#15: i.MX8MP slow scenes at `%s`, %s" % (commit, measured(d)), ""]
    L.append("%s; %s; %s (recorded, not judged). Score %s. Record `%s`." % (
        slow_fmt(d, "terrain"), slow_fmt(d, "shadow"), slow_fmt(d, "refract"), d["score"], where(d, cr)))
    if d.get("x2"):
        L.append("These are Armadillo-X2 figures; the Verdin i.MX8MP has not run this commit.")
    L.append("The issue stays open: the figures are recorded, not a verdict on the cause.")
    return "\n".join(L)


def t17(ds, commit, cr):
    d = ds["imx8mp"]
    pics = d["pic_states"]
    L = ["#17: glmark2 pictures by eye at `%s`" % commit, "",
         "Pi 4: 33 of 33 pictures checked by eye, right (`%s`)." % PI4_NOTE,
         "i.MX8MP: still open; %s, the record gives %d scenes, match %d%s. Nobody has looked at "
         "them by eye yet." % (measured(d), sum(pics.values()), pics.get("match", 0),
                               ", mismatch %d" % pics["mismatch"] if pics.get("mismatch") else ""),
         "The issue stays open for the i.MX8MP only."]
    return "\n".join(L)


def t1606(commit):
    return "\n".join([
        "#1606: docs/release/checklist.md line 9 (glmark2 table)", "",
        "The README glmark2 table now comes from one main commit, `%s` (#1196). Line 9 is no longer left to #1196; "
        "the i.MX8MP pictures by eye stay with #17." % commit])


def new_issues(ds, commit, cr):
    out = []
    for b in BOARDS:
        d = ds[b]
        for s in d["mismatch"]:
            info = s
            name = s.split("]")[0].lstrip("[")
            out.append("\n".join([
                "NEW ISSUE: %s glmark2 picture of %s differs from the reference" % (NAMES[b], info), "",
                "At `%s` the picture of `%s` on the %s%s differs from column `%s` of tools/cts/glmark2-pictures.txt: %s."
                % (commit, info, NAMES[b], " (measured on the Armadillo-X2)" if d.get("x2") else "", b, d["pic"][s][1]),
                "Record `%s`. Scene %s; its figure is recorded, not judged." % (where(d, cr), name)]))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    for b in BOARDS:
        ap.add_argument("--" + b, required=True, metavar="DIR", help="records/%s/common-<sha8>/" % b)
    ap.add_argument("--commit", required=True, help="the origin/main commit (8 or more hex digits)")
    a = ap.parse_args()
    if not re.fullmatch(r"[0-9a-f]{8,40}", a.commit):
        print("glmark2-one-commit-issues.py: --commit must be a hex sha", file=sys.stderr)
        return 2
    commit = a.commit[:8]
    cr = load("common_row", "common-row.py")
    ct = cr.load_tally()
    ds = {}
    for b in BOARDS:
        try:
            d = cr.analyse(getattr(a, b), b, ct)
        except (ValueError, OSError) as e:
            print("glmark2-one-commit-issues.py: --%s: %s" % (b, e), file=sys.stderr)
            return 2
        d["board_key"] = b
        d["x2"] = b == "imx8mp" and is_x2(d)
        ds[b] = d
    bad = ["%s %s" % (b, ds[b]["sha8"]) for b in BOARDS if ds[b]["sha8"] != commit]
    if bad:
        print("glmark2-one-commit-issues.py: records not at --commit %s: %s" % (commit, ", ".join(bad)), file=sys.stderr)
        return 2
    texts = [t1196(ds, commit, cr), t1209(ds["pi4"], commit, cr), t15(ds["imx8mp"], commit, cr),
             t17(ds, commit, cr), t1606(commit)] + new_issues(ds, commit, cr)
    print("\n\n".join(texts))
    return 0 if all(not d["fails"] for d in ds.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
