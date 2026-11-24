#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Selftest of check-tree-log.py --lossy on three made-up AM62P records (clean,
lossy but complete, lossy with a case missing); no board data.

    python3 tools/cts/testdata/check-tree-log/selftest.py
"""
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
CHECK = os.path.join(os.path.dirname(os.path.dirname(HERE)), "check-tree-log.py")
CASES = ["dEQP-GLES2.functional.fragment_ops.%s.c%d" % (g, i) for g in ("a", "b") for i in range(1, 5)]
TABLE = "dEQP-GLES2.functional.shaders.*\ndEQP-GLES2.functional.fragment_ops.*\n"
fails = []


def verdict(name, v, lost=0):
    return ("board.sh am62p %s rc=0 verdict=%s boot=x row boot_verdict=complete result=r resets=0 "
            "record=console.txt back_after=9s%s\n" % (name, v, " lost=%d" % lost if lost else ""))


def console(cases, summary_with_result):
    t = "deqp: case filter 'dEQP-GLES2.functional.fragment_ops.*'\n"
    t += "".join("deqp: Pass %s\n" % c for c in cases)
    return t + "deqp: done: 8 passed, 0 failed, 0 not supported, 0 warnings\ndeqp: summary: 8 cases begun, %d with a result, 0 without\n" % summary_with_result


def record(root, name, v, lost, cases, n):
    d = os.path.join(root, name)
    run = os.path.join(d, "20261124-1000-img2-aaaaaaaa")
    os.makedirs(run)
    with open(os.path.join(d, "stage.log"), "w") as f:
        f.write(verdict("img2", v, lost))
    with open(os.path.join(run, "console.txt"), "w") as f:
        f.write(console(cases, n))
    return d


def expect(name, cond, detail=""):
    print("%s: %s" % ("ok  " if cond else "FAIL", name))
    if not cond:
        fails.append(name)
        print(detail)


def run(*args):
    p = subprocess.run([sys.executable, "-I", CHECK, "--lossy"] + list(args), capture_output=True, text=True)
    return p.returncode, p.stdout, p.stderr


tmp = tempfile.mkdtemp(prefix="check-tree-log-", dir=os.environ.get("TMPDIR"))
try:
    table = os.path.join(tmp, "table.txt")
    mp = os.path.join(tmp, "mustpass.txt")
    open(table, "w").write(TABLE)
    open(mp, "w").write("".join(c + "\n" for c in CASES))
    opts = ["--tree", table, "--mustpass", mp]

    d = record(tmp, "clean-deqp-aaaaaaaa", "done", 0, CASES, 8)
    rc, out, err = run(*opts, d)
    expect("clean: exit 0, one ok line", rc == 0 and "ok, no done_lossy" in out and not err, out + err)

    # the verdict says done_lossy and 3 lines lost, but every expected case still has its line
    d = record(tmp, "lossy-complete-deqp-aaaaaaaa", "done_lossy", 3, CASES, 11)
    rc, out, err = run(*opts, d)
    expect("lossy but complete: exit 0, names image 2 and its 3 lost lines",
           rc == 0 and "lossy but complete: image 2 (img2) lost 3 console line(s)" in out, out + err)

    # lines of c3 and c4 of group b lost
    d = record(tmp, "lossy-missing-deqp-aaaaaaaa", "done_lossy", 2, CASES[:-2], 8)
    rc, out, err = run(*opts, d)
    expect("lossy with a case missing: exit non-zero, names image 2, 2 lost, the missing cases",
           rc == 1 and "LOSSY: image 2 (img2) lost 2 console line(s) and 2 of its 8" in err and CASES[-2] in err, out + err)

    # lost found from the console's own counters, with no lost= in any verdict
    d = record(tmp, "counters-deqp-aaaaaaaa", "done", 0, CASES[:-2], 8)
    rc, out, err = run(*opts, d)
    expect("lost seen only in the console counters is still caught", rc == 1 and "lost 2 console line(s)" in err, out + err)
finally:
    shutil.rmtree(tmp)

if fails:
    print("selftest: %d FAILED" % len(fails))
    sys.exit(1)
print("selftest: all ok")
