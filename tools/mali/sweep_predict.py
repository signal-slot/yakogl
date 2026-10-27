#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Predict the verdicts of the next T6 (Mali) panthor-gltest sweep.

    python3 -I tools/mali/sweep_predict.py <record dir> [--cases N,M,...]
                [--corpus FILE] [--gl-test FILE] [--gen FILE] [--root DIR]

<record dir> holds sweep-<n>.verdicts.txt files (for example
tools/cts/log/2026-10-27-frameboard). The highest-numbered sweep is the "last"
verdict of every case; a case whose sweeps disagree is listed in the summary.
The line format is learned from the files: `<prefix> verdict case <n> <name>:
<VERDICT> <text>` (the prefix and the spacing are copied from the last sweep).

Output: one line per case in that same shape, with the predicted verdict,
then `#` summary lines (diff the lines, ignore the `#` ones):

  PASS                 stays PASS.
  PASS-CANDIDATE       the case was REFUSED and every shader it uses now
                       compiles (verdict `ok` in tools/mali/corpus-verdicts.txt).
                       Compiling is necessary, not sufficient: a frame need or
                       a wrong pixel still fails on the board.
  REFUSED <text>       a shader it uses is still refused (the gap is named
                       after the text), or its sources could not be found;
                       the text of the last sweep is kept.
  anything else        (FAIL, SKIP, NO-CASE) is kept as it was.

A case's shaders are found the way gen_gl_test_shaders.py finds the ES 3.00
cases: the run_case switch of gl_test.c names the case function, the function
body names `gl_test_*_src` symbols, and the ENTRIES table of
gen_gl_test_shaders.py maps each symbol to a file under tools/shaders. A
source not in corpus-verdicts.txt (or a case with no symbol) cannot be
judged: it stays REFUSED. The corpus file is read, not regenerated: run
tools/mali/corpus-verdicts.sh --write first when the compiler moved.
Standard library only.
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
LINE = re.compile(r"^(?P<head>.*?\bverdict case )(?P<num>\d+)(?P<mid> \S*?:)\s*(?P<verdict>[A-Z][A-Z-]*)(?P<rest>.*)$")
SWEEP = re.compile(r"^sweep-(\d+)\.verdicts\.txt$")
SYMBOL = re.compile(r"\bgl_test_\w+_src\b")


def read_sweeps(rec):
    """[(n, {case: (line parts)})] for the sweep files of rec, ordered by n."""
    files = sorted((int(m.group(1)), f) for f in os.listdir(rec) if (m := SWEEP.match(f)))
    if not files:
        raise SystemExit("sweep_predict: no sweep-<n>.verdicts.txt in %s" % rec)
    out = []
    for n, f in files:
        cases = {}
        with open(os.path.join(rec, f)) as fh:
            for line in fh:
                m = LINE.match(line.rstrip("\n"))
                if m:
                    cases[int(m.group("num"))] = m.groupdict()
        if not cases:
            raise SystemExit("sweep_predict: %s has no verdict line" % f)
        out.append((n, cases))
    return out


def entries(gen_path):
    """{symbol: tools/shaders-relative file} from the ENTRIES table of the generator.

    The T6 build compiles the ES 1.00 set (GL_TEST_ES100): a desktop-GLSL source
    is replaced by its ES 1.00 twin, as the generator's ES100_TWINS says."""
    with open(gen_path) as f:
        text = f.read()
    files = dict(re.findall(r"\('(gl_test_\w+_src)',\s*'([^']+)'", text))
    block = re.search(r"^ES100_TWINS = \{(.*?)^\}", text, re.S | re.M)
    twins = dict(re.findall(r"'([^']+)':\s*'([^']+)'", block.group(1))) if block else {}
    return {s: twins.get(f, f) for s, f in files.items()}


def case_symbols(gl_test_path):
    """{case number: set of gl_test_*_src symbols its function names}."""
    with open(gl_test_path) as f:
        c = f.read()
    bodies = dict(re.findall(r"^static int (case_\w+)\([^)]*\)\n\{(.*?)^\}", c, re.S | re.M))
    return {int(n): set(SYMBOL.findall(bodies.get(fn, "")))
            for n, fn in re.findall(r"case (\d+):\s*(?:rc = )?(case_\w+)\(", c)}


def corpus(path):
    """{source path: verdict text}: `ok` or the gap."""
    out = {}
    with open(path) as f:
        for line in f:
            if line.startswith("#") or ": " not in line:
                continue
            src, _, verdict = line.rstrip("\n").partition(": ")
            out[src] = verdict
    return out


def judge(num, symbols, files, verdicts):
    """(True, "") when every source of case num compiles; else (False, why)."""
    syms = symbols.get(num)
    if not syms:
        return False, "no shader source symbol in the case function (its GLSL is built at run time, or it is not in gl_test.c)"
    gaps = []
    for s in sorted(syms):
        f = files.get(s)
        if f is None:
            gaps.append("%s is not in ENTRIES" % s)
            continue
        v = verdicts.get("tools/shaders/" + f)
        if v is None:
            gaps.append("tools/shaders/%s is not in the corpus" % f)
        elif v != "ok":
            gaps.append("tools/shaders/%s: %s" % (f, v))
    return (not gaps), "; ".join(gaps)


def predict(sweeps, symbols, files, verdicts, only=None):
    """[(case, line, last verdict, predicted verdict, why)] for each case of the last sweep."""
    last = sweeps[-1][1]
    rows = []
    for num in sorted(last):
        if only is not None and num not in only:
            continue
        p = last[num]
        v = p["verdict"]
        pred, why = v, ""
        if v == "REFUSED":
            ok, why = judge(num, symbols, files, verdicts)
            if ok:
                pred, why = "PASS-CANDIDATE", ""
        rows.append((num, p, v, pred, why))
    return rows


def render(p, pred):
    if pred == p["verdict"]:
        return "%s%d%s %s%s" % (p["head"], int(p["num"]), p["mid"], pred, p["rest"])
    return "%s%d%s %s" % (p["head"], int(p["num"]), p["mid"], pred)


def main(argv):
    ap = argparse.ArgumentParser(description="predict the verdicts of the next T6 sweep")
    ap.add_argument("record")
    ap.add_argument("--cases", help="comma list of case numbers")
    ap.add_argument("--root", default=ROOT, help="repository root (default: this one)")
    ap.add_argument("--corpus")
    ap.add_argument("--gl-test")
    ap.add_argument("--gen")
    a = ap.parse_args(argv)
    r = a.root
    corp = a.corpus or os.path.join(r, "tools/mali/corpus-verdicts.txt")
    glt = a.gl_test or os.path.join(r, "driver/tools/pvrlab/gl_test.c")
    gen = a.gen or os.path.join(r, "driver/tools/pvrlab/gen_gl_test_shaders.py")
    only = None
    if a.cases:
        try:
            only = {int(x) for x in a.cases.split(",") if x.strip()}
        except ValueError:
            raise SystemExit("sweep_predict: --cases wants numbers: %r" % a.cases)
    sweeps = read_sweeps(a.record)
    rows = predict(sweeps, case_symbols(glt), entries(gen), corpus(corp), only)
    for _, p, _, pred, _ in rows:
        print(render(p, pred))
    tally = {}
    for _, _, _, pred, _ in rows:
        tally[pred] = tally.get(pred, 0) + 1
    print("# %d case(s) predicted from %s: %s" % (
        len(rows), "sweep-%d.verdicts.txt" % sweeps[-1][0], ", ".join("%s %d" % kv for kv in sorted(tally.items()))))
    for num, _, _, pred, why in rows:
        if why:
            print("# case %d stays REFUSED: %s" % (num, why))
    if len(sweeps) > 1:
        prev = sweeps[-2][1]
        for num, p, v, _, _ in rows:
            if num in prev and prev[num]["verdict"] != v:
                print("# case %d differs between sweeps: %s then %s" % (num, prev[num]["verdict"], v))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
