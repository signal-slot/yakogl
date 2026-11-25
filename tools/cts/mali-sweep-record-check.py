#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
r"""Check a NanoPC-T6 gl_test sweep record: two identical runs, README fields, redaction.

  mali-sweep-record-check.py RECORD_DIR [--reference SHA]

RECORD_DIR is a T6 sweep record directory (the tools/cts/log/<date>-t6-sweep-<sha8>/
layout a panthor-gltest --sweep session leaves: README.md plus sweep-1.verdicts.txt
and sweep-2.verdicts.txt, the sweep run twice in one boot). The checker is read-only:
it reads RECORD_DIR and writes nothing.

--reference SHA names the earlier sweep record this one must cite (default 1a22bd38,
the commit of tools/cts/log/2026-11-t6-sweep-1a22bd38/). SHA is 7 to 40 hex digits;
--reference '' drops the readme-reference check. The 1a22bd38 record itself needs no
flag: its README names its own commit 1a22bd38e, which contains 1a22bd38, and the
readme-commit-sha check accepts its other named commits (0decb5f7, 68b9cdf26).

Checks, printed in this order, one `ok   <name>` or `FAIL <name>: <detail>` line each:

  verdicts-identical  sweep-1.verdicts.txt and sweep-2.verdicts.txt exist and are
                      byte-identical
  verdicts-summary    each of the two verdict files that exists holds a line matching
                      ^panthor-gltest: <digits> case(s) run: (`case(s)`, `cases` and
                      `case` all match); a missing file is left to verdicts-identical
  readme-boot-id      README.md names boot_id and gives a before and an after value
                      that are equal, either as an explicit `before ... <value> ...
                      after ... <value>` pair with equal values, or as the single
                      value of the form used in 2026-11-t6-sweep-1a22bd38/README.md
                      (`boot_id unchanged over both runs (`<value>`...)`, whose
                      before and after are that one value). A before/after pair that
                      differs fails even if the README also names an unchanged value.
  readme-reference    README.md names the reference sha anywhere (skipped entirely,
                      with no line printed, when --reference '' is given)
  readme-commit-sha   README.md names a commit sha other than the reference: a
                      backtick-quoted token of 8 to 40 hex digits whose first digits
                      are not the reference's (a full sha counts; a boot_id like
                      `5e2a981d-…` is quoted but not pure hex, and does not count).
                      With --reference '' any backtick-quoted sha counts.
  redaction           no file anywhere under RECORD_DIR matches IPv4
                      (?:[0-9]{1,3}\.){3}[0-9]{1,3}, MAC
                      (?:[0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}, /home/ or /dev/tty.
                      A hit is printed as <file>:<line>: <pattern name> and at most
                      8 hits are printed per check; the matched text is never printed.

Last line: `mali-sweep-record-check: <RECORD_DIR>: complete (n ok, 0 failed)` or
`... incomplete (n ok, m failed)`.
Exit 0 all checks ok, 1 any FAIL, 2 bad argument, unreadable RECORD_DIR or a
--reference that is neither '' nor 7 to 40 hex digits.

Self-test: sh tools/cts/mali-sweep-record-check-selftest.sh
"""
import argparse
import os
import re
import sys

README = "README.md"
V1 = "sweep-1.verdicts.txt"
V2 = "sweep-2.verdicts.txt"
DEFAULT_REFERENCE = "1a22bd38"

SUMMARY_RE = re.compile(r"^panthor-gltest: [0-9]+ case(?:\(s\)|s)? run:")
HEX_RUN_RE = re.compile(r"[0-9a-fA-F]{4,}")
SHA_TOKEN_RE = re.compile(r"`([0-9a-fA-F]{8,40})`")
BEFORE_VALUE_RE = re.compile(r"\bbefore\b[^0-9a-fA-F]*([0-9a-fA-F]{4,})")
AFTER_VALUE_RE = re.compile(r"\bafter\b[^0-9a-fA-F]*([0-9a-fA-F]{4,})")
UNCHANGED_RE = re.compile(r"\bunchanged\b")
REFERENCE_RE = re.compile(r"[0-9a-fA-F]{7,40}")

REDACTION_PATTERNS = [
    ("ipv4", re.compile(r"(?:[0-9]{1,3}\.){3}[0-9]{1,3}")),
    ("mac", re.compile(r"(?:[0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}")),
    ("home-path", re.compile(r"/home/")),
    ("tty-path", re.compile(r"/dev/tty")),
]


def read_bytes(path):
    with open(path, "rb") as f:
        return f.read()


def verdicts_identical_check(record):
    paths = {name: os.path.join(record, name) for name in (V1, V2)}
    missing = [name for name in (V1, V2) if not os.path.isfile(paths[name])]
    if missing:
        return False, "missing: " + ", ".join(missing)
    b1 = read_bytes(paths[V1])
    b2 = read_bytes(paths[V2])
    if b1 == b2:
        return True, ""
    l1 = b1.split(b"\n")
    l2 = b2.split(b"\n")
    diff_at = next((i for i in range(min(len(l1), len(l2))) if l1[i] != l2[i]),
                   min(len(l1), len(l2)))
    return False, ("differ: first difference at line %d (%s has %d lines, %s has %d)"
                   % (diff_at + 1, V1, len(l1), V2, len(l2)))


def verdicts_summary_check(record):
    problems = []
    for name in (V1, V2):
        path = os.path.join(record, name)
        if not os.path.isfile(path):
            continue  # verdicts-identical reports a missing file
        lines = read_bytes(path).decode("utf-8", "replace").split("\n")
        if not any(SUMMARY_RE.match(line) for line in lines):
            problems.append("%s: no summary line" % name)
    if problems:
        return False, "; ".join(problems)
    return True, ""


def boot_id_check(text):
    norm = " ".join(text.split())
    pair_ok = False
    pair_differs = False
    mentioned = False
    for m in re.finditer("boot_id", norm):
        mentioned = True
        window = norm[m.start():m.start() + 260]
        before = BEFORE_VALUE_RE.search(window)
        after = AFTER_VALUE_RE.search(window)
        if before and after:
            if before.group(1).lower() == after.group(1).lower():
                pair_ok = True
            else:
                pair_differs = True
        elif UNCHANGED_RE.search(window) and HEX_RUN_RE.search(window):
            # the 1a22bd38 form: one value, named as unchanged over the runs
            pair_ok = True
    if pair_differs:
        return False, "boot_id before and after values differ"
    if pair_ok:
        return True, ""
    if not mentioned:
        return False, "README.md does not name boot_id"
    return False, ("boot_id named without an equal before/after value pair "
                   "(accepted: `before ... <value> ... after ... <value>` or the "
                   "1a22bd38 form `boot_id unchanged ... (`<value>`...)`)")


def reference_check(text, reference):
    if reference.lower() in text.lower():
        return True, ""
    return False, "README.md does not name the reference commit %s" % reference


def commit_sha_check(text, reference):
    ref = reference.lower()
    for token in SHA_TOKEN_RE.findall(text):
        tok = token.lower()
        if ref and (tok.startswith(ref) or ref.startswith(tok)):
            continue  # the reference commit named again, not a new one
        return True, ""
    return False, ("README.md names no commit sha other than the reference "
                   "(a backtick-quoted token of 8 to 40 hex digits)")


def redaction_check(record):
    hits = []
    for root, dirs, files in os.walk(record):
        dirs.sort()
        for name in sorted(files):
            path = os.path.join(root, name)
            rel = os.path.relpath(path, record)
            try:
                data = read_bytes(path)
            except OSError:
                hits.append("%s: unreadable" % rel)
                continue
            for lineno, line in enumerate(data.decode("utf-8", "replace").split("\n"), 1):
                for pattern_name, pattern in REDACTION_PATTERNS:
                    if pattern.search(line):
                        hits.append("%s:%d: %s" % (rel, lineno, pattern_name))
    if hits:
        shown = "; ".join(hits[:8])
        if len(hits) > 8:
            shown += " (%d more hits)" % (len(hits) - 8)
        return False, shown
    return True, ""


def main(argv):
    ap = argparse.ArgumentParser(
        prog="mali-sweep-record-check.py",
        description="Check a NanoPC-T6 gl_test sweep record directory: two identical "
                    "runs, README fields, redaction. Read-only.",
    )
    ap.add_argument("record_dir",
                    help="record directory, e.g. tools/cts/log/<date>-t6-sweep-<sha8>/")
    ap.add_argument("--reference", default=DEFAULT_REFERENCE, metavar="SHA",
                    help="earlier sweep record's commit sha the README must name "
                         "(default %s); '' drops the readme-reference check"
                         % DEFAULT_REFERENCE)
    ns = ap.parse_args(argv[1:])
    reference = ns.reference
    if reference and not REFERENCE_RE.fullmatch(reference):
        print("mali-sweep-record-check: --reference must be '' or 7 to 40 hex digits",
              file=sys.stderr)
        return 2
    record = ns.record_dir
    if not os.path.isdir(record):
        print("mali-sweep-record-check: not a directory: " + record, file=sys.stderr)
        return 2

    checks = [
        ("verdicts-identical", lambda: verdicts_identical_check(record)),
        ("verdicts-summary", lambda: verdicts_summary_check(record)),
    ]
    readme_path = os.path.join(record, README)
    text = None
    if os.path.isfile(readme_path):
        try:
            text = read_bytes(readme_path).decode("utf-8", "replace")
        except OSError:
            text = None
    if text is None:
        checks.append(("readme-boot-id", lambda: (False, "README.md missing")))
        if reference:
            checks.append(("readme-reference", lambda: (False, "README.md missing")))
        checks.append(("readme-commit-sha", lambda: (False, "README.md missing")))
    else:
        checks.append(("readme-boot-id", lambda: boot_id_check(text)))
        if reference:
            checks.append(("readme-reference", lambda: reference_check(text, reference)))
        checks.append(("readme-commit-sha", lambda: commit_sha_check(text, reference)))
    checks.append(("redaction", lambda: redaction_check(record)))

    failed = 0
    for name, fn in checks:
        ok, detail = fn()
        if ok:
            print("ok   " + name)
        else:
            failed += 1
            print("FAIL %s: %s" % (name, detail))
    done = len(checks) - failed
    print("mali-sweep-record-check: %s: %s (%d ok, %d failed)"
          % (record, "incomplete" if failed else "complete", done, failed))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
