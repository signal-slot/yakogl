#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Selftest of tools/cts/am62p-gles3-merge.py and check-tree-log.py on the made-up
fixture slices next to this file (mkfixtures.py made them; no board data).

    python3 tools/cts/testdata/am62p-gles3-merge/selftest.py
"""
import glob
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CTS = os.path.dirname(os.path.dirname(HERE))
MERGE = os.path.join(CTS, "am62p-gles3-merge.py")
CHECK = os.path.join(CTS, "check-tree-log.py")
fails = []


def run(script, *args):
    p = subprocess.run([sys.executable, "-I", script] + list(args), capture_output=True, text=True)
    return p.returncode, p.stdout, p.stderr


def slices(kind):
    return sorted(glob.glob(os.path.join(HERE, kind, "*-am62p-gles3-*")))


def good(kind):
    return slices("good")[:1] + slices(kind)


def expect(name, cond, detail=""):
    print("%s: %s" % ("ok  " if cond else "FAIL", name))
    if not cond:
        fails.append(name)
        if detail:
            print(detail)


mp = ["--mustpass", os.path.join(HERE, "mustpass.txt")]

# a good merge
rc, out, err = run(CHECK, "--rows", "4", *slices("good"))
expect("check: the good slices pass", rc == 0 and "4 rows from commit aaaaaaaa" in out, err)
rc, out, err = run(MERGE, *mp, "--against", os.path.join(HERE, "old.tsv"), *slices("good"))
want = ("Fail\tdEQP-GLES3.t.a.y\tMismatch at pixel 3\n"
        "NotSupported\tdEQP-GLES3.t.b.x\tExtension missing\n"
        "Fail\tdEQP-GLES3.t.c.x\tCrash\n"
        "NoResult\tdEQP-GLES3.t.d.x\t\n")
body = "".join(l + "\n" for l in out.splitlines() if not l.startswith("#"))
expect("merge: the not-pass TSV (mustpass order, zz ignored, Crash is Fail, d.x no result)", rc == 0 and body == want, out)
expect("merge: totals Pass 2 Fail 2 NotSupported 1 QualityWarning 0 no result 1 = 6",
       "Pass 2  Fail 2  NotSupported 1  QualityWarning 0  no result 1  = 6 (mustpass 6)" in err, err)
expect("merge: delta against the old list (b.y fixed, b.x and c.x new)",
       "no longer not-pass 1, newly not-pass 2, outcome changed 0" in err
       and "Fail 2 -> 2" not in err and "NotSupported 0 -> 1" in err, err)

# a mixed-commit refusal
rc, out, err = run(MERGE, *mp, *good("mixed"))
expect("merge: refuses slices of two commits", rc == 1 and out == "" and "different commits: aaaaaaaa, bbbbbbbb" in err, err)
rc, out, err = run(CHECK, "--rows", "4", *good("mixed"))
expect("check: refuses slices of two commits", rc == 1 and "different commits" in err, err)

# a missing-row refusal
rc, out, err = run(CHECK, "--rows", "4", *good("missing"))
expect("check: refuses a missing row 4", rc == 1 and "no verdict line for row(s) 4" in err, err)
rc, out, err = run(CHECK, "--rows", "4", *slices("good")[:1])
expect("check: refuses rows 3-4 absent altogether", rc == 1 and "row(s) 3, 4 of 1..4" in err, err)

# an IP refusal
rc, out, err = run(CHECK, "--rows", "4", *good("ip"))
expect("check: refuses an IPv4 address and a /dev/tty path",
       rc == 1 and "IPv4 address" in err and "/dev/tty path" in err, err)

# a reset refusal
rc, out, err = run(CHECK, "--rows", "4", *good("reset"))
expect("check: refuses resets=2", rc == 1 and "resets is 2, not 0" in err, err)

# a host:port, a source file:line and a clock stamp in a message
import importlib.util
spec = importlib.util.spec_from_file_location("ctl", CHECK)
ctl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ctl)
expect("scan: host:port found", bool(ctl.scan_text("f", "connect to board-1.lan:2222")))
expect("scan: file.cpp:123 and 12:34:56 are not host:port",
       not ctl.scan_text("f", "at es3fFoo.cpp:123 on 12:34:56 version 3.0.1") and not ctl.scan_text("f", "Mesa 22.3.6"))

if fails:
    print("selftest: %d FAILED" % len(fails))
    sys.exit(1)
print("selftest: all ok")
