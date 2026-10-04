#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Bookkeeping for a dEQP run that is restarted after a crash or a GPU hang.

  cts_resume.py --caselist FILE --results DIR [--repair] > remaining.txt

Prints, in case-list order, the cases that still have no result in any .qpa
under DIR. With --repair it first closes the record of a case that was started
and never finished, which is what the log of a process killed mid-case looks
like.

Why the repair matters. dEQP writes

    #beginTestCaseResult dEQP-GLES2.functional.foo
    ... the case log ...
    #endTestCaseResult

and the CTS's own BatchResultParser.parseFile() only yields cases that reached
the closing line. A case that took the process down with it (SIGKILL, an OOM
kill, a wedged ioctl the in-process crash handler never gets to run after)
leaves a dangling #beginTestCaseResult and therefore no result at all. Resuming
on "every case without a result" would put that case first in the next run and
crash again, forever. Closing it with

    #terminateTestCaseResult Crash

is what dEQP itself writes when its crash handler does get to run, so the case
is counted as a crash by the same parser, and the run moves past it.
"""
import argparse
import glob
import os
import sys

CTS = os.environ.get("CTS", os.path.expanduser("~/com/github/KhronosGroup/VK-GL-CTS"))
sys.path.insert(0, os.path.join(CTS, "scripts", "log"))
from log_parser import BatchResultParser  # noqa: E402

BEGIN = b"#beginTestCaseResult"
END = b"#endTestCaseResult"
TERMINATE = b"#terminateTestCaseResult"


def dangling_case(path):
    """The case a .qpa opened and never closed, or None."""
    open_case = None
    with open(path, "rb") as f:
        for line in f:
            if line.startswith(BEGIN):
                parts = line.split()
                open_case = parts[1].decode("utf-8", "ignore") if len(parts) > 1 else "?"
            elif line.startswith(END) or line.startswith(TERMINATE):
                open_case = None
    return open_case


def repair(path, status="Crash"):
    """Close a dangling record so the CTS parser reports it as `status`."""
    name = dangling_case(path)
    if name is None:
        return None
    # The kill can land mid-line, and the parser is line based.
    needs_newline = False
    with open(path, "rb") as f:
        size = f.seek(0, os.SEEK_END)
        if size > 0:
            f.seek(size - 1)
            needs_newline = f.read(1) != b"\n"
    with open(path, "ab") as f:
        if needs_newline:
            f.write(b"\n")
        f.write(TERMINATE + b" " + status.encode() + b"\n")
    return name


def results_in(results_dir):
    """Every case name that has a result under results_dir."""
    done = set()
    for path in sorted(glob.glob(os.path.join(results_dir, "*.qpa"))):
        try:
            done |= {r.name for r in BatchResultParser().parseFile(path)}
        except Exception as exc:  # a truncated log must not stop the resume
            print("cts_resume: %s: %s" % (path, exc), file=sys.stderr)
    return done


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--caselist", required=True)
    ap.add_argument("--results", required=True)
    ap.add_argument("--repair", action="store_true",
                    help="close a dangling record as Crash before counting")
    ap.add_argument("--repair-status", default="Crash",
                    help="status to close it with (Crash or Timeout)")
    args = ap.parse_args()

    if args.repair:
        for path in sorted(glob.glob(os.path.join(args.results, "*.qpa"))):
            name = repair(path, args.repair_status)
            if name:
                print("cts_resume: %s: closed %s as %s"
                      % (os.path.basename(path), name, args.repair_status),
                      file=sys.stderr)

    done = results_in(args.results)
    with open(args.caselist) as f:
        remaining = [l.strip() for l in f if l.strip() and l.strip() not in done]
    # One name per line, and nothing at all when there is nothing left: the
    # caller counts lines, and "\n".join([]) still prints a newline, which made
    # a finished run look like it had one case to go.
    for name in remaining:
        print(name)
    return 0


if __name__ == "__main__":
    sys.exit(main())
