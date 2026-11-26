#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compare a NanoPC-T6 dEQP-GLES2 run started from U-Boot with two baselines, per case.

    python3 -I tools/cts/t6-uboot-deqp-compare.py [--quiet] [--all]
        [--scope FILE] --uboot FILE... --kexec FILE... --armbian FILE...

  --uboot   the U-Boot-started Zephyr run (the board session's UART log)
  --kexec   the kexec-started Zephyr run (main de2f2c103, 2026-11-17): UART logs
            of its 76 images, or a "<Status> <case>" list made from them
  --armbian Armbian's Linux/panthor run (main 8ff01055, 2026-11-24): the four
            tools/cts/log/2026-11-24-t6-gles2-S?-8ff01055/ directories (their
            results.tsv) or the results.tsv files
Each option takes one or more files or directories (a directory is read by its
results.tsv, else by its *.log files); --uboot also takes what
tools/cts/t6-uboot-rows.sh leaves: its <out-dir> or its rows.tsv (each row's
record directory, the fourth column, is read by its image.log), or one record
directory of driver/tools/uboot-t6/prompt-go.sh (image.log, the UART from `go`
to Linux's login; uart.log and typed.log are not read: they repeat or lack
the image's lines). A row with exit code 0 or 11 must have an image.log with
outcome lines; a row of another code (a stop) or with no record ("-", a dry
run) adds what its image.log has, if any. Several files are united and a later
file's status for a case replaces an earlier one's. A file may be, line by line:
  "deqp: <Status> <case>" (the Zephyr deqp sample's console line, as
      tools/cts/deqp-uart-tally.py reads it: NULs and CRs dropped, a leading
      HH:MM:SS dropped),
  "<Status> <case>" (deqp-uart-tally.py --statuses),
  "<case><TAB><Status><TAB>..." (results.tsv of tools/cts/mali-deqp-run.sh,
      as tools/cts/mali-deqp-tally.py reads it; the "case" header is skipped).
Anything else is ignored.

Per case one line: "<case> uboot=<S> kexec=<S> armbian=<S> <class>", with "-"
for no outcome. The scope is the U-Boot run's cases (a one-row run), or
--scope FILE's case names (one per line, e.g. gles2-mustpass.txt) so that a
case the U-Boot run lacks is "missing", or --all for every case of any input.
Classes, by the pass side (Pass, NotSupported, QualityWarning,
CompatibilityWarning) against the fail side (everything else); newly failing
wins when the baselines disagree:
  same           U-Boot is on the same side as every baseline that has the case
  newly failing  U-Boot fails and a baseline passes
  newly passing  U-Boot passes and a baseline fails
  missing        U-Boot has no outcome for the case, or neither baseline has it
--quiet prints only the cases that are not "same". The last line is
"t6-uboot-deqp-compare: N cases, same A, newly failing B, newly passing C,
missing D". Exit 0; exit 2 on bad input (a usage error, an unreadable file, a
file with no outcome line).
"""
import argparse
import glob
import os
import re
import sys

PASSING = ("Pass", "NotSupported", "QualityWarning", "CompatibilityWarning")
STATUSES = PASSING + ("Fail", "InternalError", "ResourceError", "Crash",
                      "Timeout", "NoResult", "Lost")
LINE = re.compile(r"^(?:\d\d:\d\d:\d\d\s+)?(?:deqp: )?(\w+) (dEQP-\S+)\s*$")
TSV = re.compile(r"^(dEQP-[^\t ]+)\t(\w+)(?:\t.*)?$")


def parse(text):
    """-> {case: status} of one file's text (the last status of a case wins)."""
    out = {}
    for l in text.replace("\0", "").replace("\r", "").splitlines():
        m = TSV.match(l)
        if m and m.group(2) in STATUSES:
            out[m.group(1)] = m.group(2)
            continue
        m = LINE.match(l)
        if m and m.group(1) in STATUSES:
            out[m.group(2)] = m.group(1)
    return out


def rows_files(tsv):
    """image.log files of the record dirs named by a t6-uboot-rows.sh rows.tsv."""
    out = []
    with open(tsv, errors="replace") as fh:
        for l in fh:
            f = l.rstrip("\n").split("\t")
            if len(f) < 4:
                continue
            log = os.path.join(f[3], "image.log") if f[3] != "-" else None
            if log and os.path.isfile(log):
                if f[1] in ("0", "11"):
                    out.append((log, True))
                else:
                    with open(log, errors="replace") as lf:
                        if parse(lf.read()):
                            out.append((log, True))
            elif f[1] in ("0", "11"):
                raise ValueError("%s: row %s (exit %s) has no image.log in record %s"
                                 % (tsv, f[0], f[1], f[3]))
    return out


def files_of(p):
    if os.path.basename(p) == "rows.tsv" and os.path.isfile(p):
        return [f for f, _ in rows_files(p)]
    if os.path.isdir(p):
        if os.path.isfile(os.path.join(p, "rows.tsv")):
            return [f for f, _ in rows_files(os.path.join(p, "rows.tsv"))]
        if os.path.isfile(os.path.join(p, "image.log")):
            return [os.path.join(p, "image.log")]
        t = os.path.join(p, "results.tsv")
        if os.path.isfile(t):
            return [t]
        return sorted(glob.glob(os.path.join(p, "*.log")))
    return [p]


def load(paths):
    res = {}
    for p in paths:
        fs = files_of(p)
        if not fs:
            raise ValueError("%s: no results.tsv or *.log" % p)
        for f in fs:
            with open(f, errors="replace") as fh:
                one = parse(fh.read())
            if not one:
                raise ValueError("%s: no dEQP outcome line" % f)
            res.update(one)
    return res


def classify(u, k, a):
    base = [s for s in (k, a) if s is not None]
    if u is None or not base:
        return "missing"
    up = u in PASSING
    if not up and any(b in PASSING for b in base):
        return "newly failing"
    if up and any(b not in PASSING for b in base):
        return "newly passing"
    return "same"


def main():
    doc = __doc__.split("\n")
    ap = argparse.ArgumentParser(
        description=doc[0], epilog="\n".join(doc[2:]),
        formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--uboot", nargs="+", required=True)
    ap.add_argument("--kexec", nargs="+", required=True)
    ap.add_argument("--armbian", nargs="+", required=True)
    ap.add_argument("--scope", metavar="FILE")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--quiet", action="store_true")
    try:
        a = ap.parse_args()
    except SystemExit as e:
        return 2 if e.code else 0
    try:
        u, k, r = load(a.uboot), load(a.kexec), load(a.armbian)
        scope = None
        if a.scope:
            with open(a.scope, errors="replace") as fh:
                scope = {l.strip() for l in fh if l.strip().startswith("dEQP-")}
            if not scope:
                raise ValueError("%s: no case names" % a.scope)
    except (OSError, ValueError) as e:
        print("t6-uboot-deqp-compare: %s" % e, file=sys.stderr)
        return 2
    if a.all:
        cases = set(u) | set(k) | set(r)
    elif scope is not None:
        cases = scope
    else:
        cases = set(u)
    n = {"same": 0, "newly failing": 0, "newly passing": 0, "missing": 0}
    for c in sorted(cases):
        cl = classify(u.get(c), k.get(c), r.get(c))
        n[cl] += 1
        if cl != "same" or not a.quiet:
            print("%s uboot=%s kexec=%s armbian=%s %s" % (
                c, u.get(c, "-"), k.get(c, "-"), r.get(c, "-"), cl))
    print("t6-uboot-deqp-compare: %d cases, same %d, newly failing %d, "
          "newly passing %d, missing %d" % (
              len(cases), n["same"], n["newly failing"], n["newly passing"],
              n["missing"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
