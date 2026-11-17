#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Write the i.MX8MP ES 2.0 regression summary README from a session's board.sh output.

    python3 -I tools/cts/imx8mp-es2-summary.py --commit <40 hex> --entries FIRST-LAST \\
        [--out README.md] [--rows "rows01-15"] FILE...

FILE holds the captured stdout of tools/board.sh imx8mp (use - for stdin). Only the
'board.sh imx8mp e<NN> rc=...' verdict lines are read; page lines (e<NN>-p<n>) and every other
line are dropped. The README carries the 40-hex commit and exactly one verdict line per entry
FIRST..LAST (the shape tools/cts/imx8mp-es2-regress.py reads and the plan's grep
'board.sh imx8mp e[0-9]* rc=' counts). It refuses (exit 2, nothing written) a commit that is
not 40 lowercase hex digits, a bad range, an entry outside the range, a duplicate or missing
entry, a line without a DEQP record of 3-4 hex words, and any hit of
imx8mp-gles3-redact.py --check on the text it would write. Without --out the README goes to stdout.
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
LINE = re.compile(r"board\.sh imx8mp e(\d+) rc=(-?\d+)\b.*")
REC = re.compile(r"\brecord=[0-9a-fA-F]+(,[0-9a-fA-F]+){2,3}\b")


def die(msg):
    print("es2 summary: " + msg, file=sys.stderr)
    sys.exit(2)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--commit", required=True)
    ap.add_argument("--entries", required=True, help="FIRST-LAST, counted from 0")
    ap.add_argument("--out")
    ap.add_argument("--rows", help="name for the title, e.g. rows01-15")
    ap.add_argument("files", nargs="+")
    a = ap.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", a.commit):
        die("--commit must be the full 40-hex commit, got %d chars" % len(a.commit))
    m = re.fullmatch(r"(\d+)-(\d+)", a.entries)
    if not m or int(m.group(1)) > int(m.group(2)) or int(m.group(2)) > 28:
        die("--entries must be FIRST-LAST within 0-28")
    first, last = int(m.group(1)), int(m.group(2))
    got = {}
    for p in a.files:
        try:
            text = sys.stdin.read() if p == "-" else open(p, errors="replace").read()
        except OSError as e:
            die(str(e))
        for ln in text.splitlines():
            m = LINE.search(ln)
            if not m:
                continue
            i = int(m.group(1))
            if not first <= i <= last:
                die("entry e%02d is outside %d-%d" % (i, first, last))
            if i in got:
                die("entry e%02d appears twice" % i)
            if not REC.search(ln):
                die("entry e%02d has no record= of 3-4 hex words" % i)
            got[i] = m.group(0).rstrip()
    miss = [i for i in range(first, last + 1) if i not in got]
    if miss:
        die("missing entries: " + " ".join("e%02d" % i for i in miss))
    n = last - first + 1
    name = a.rows or "entries %d-%d" % (first, last)
    body = ["<!-- SPDX-License-Identifier: Apache-2.0 -->",
            "<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->",
            "# i.MX8MP ES 2.0 regression rows, %s (entries %d-%d of imx8mp-full-tree.txt)" % (name, first, last),
            "",
            "commit " + a.commit,
            "",
            "%d verdict lines, verbatim from `tools/board.sh imx8mp` (`record=` is the SNVS DEQP record" % n,
            "w0..w3; `back_after` is reboot to ssh, `uptime` Linux's at that moment). Compare with the",
            "stage-1 baseline: `python3 -I tools/cts/imx8mp-es2-regress.py <this README>`.",
            "",
            "```"]
    body += [got[i] for i in range(first, last + 1)]
    body += ["```", ""]
    text = "\n".join(body)
    with tempfile.TemporaryDirectory() as d:
        t = os.path.join(d, "README.md")
        open(t, "w").write(text)
        r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "imx8mp-gles3-redact.py"), "--check", t],
                           capture_output=True, text=True)
    if r.returncode != 0:
        die("redact check hit:\n" + r.stdout + r.stderr)
    if a.out:
        open(a.out, "w").write(text)
    else:
        sys.stdout.write(text)


if __name__ == "__main__":
    main()
