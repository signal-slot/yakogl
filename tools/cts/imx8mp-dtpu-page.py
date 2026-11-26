#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Decode gl_test case 222's dtpu page: checks 3 to 8, the verdict and the outcome.

usage:
  imx8mp-dtpu-page.py W1 W2 W3        the page's three words (hex, 0x optional), as
                                      `yakogl-tools record` shows them for kind PERF tag 0xb0
  imx8mp-dtpu-page.py --log PATH      a console capture: a file, a record directory (its
                                      console.txt) or - for stdin; the last `yk-record:` line
                                      of kind PERF, tag 0xb0 (w0 = 594b0cb0) is the page

The page is written by driver/tools/pvrlab/gl_test_dtpu.inc (dtpu_encode) at selector
GC_RESULT_PERF_SEL | 0xb0 (SEL=4272); docs/notes/gc-depth-sample.md, "Reading the dtpu
page", has the table:

  w1  bit 0         the page is valid (the arms ran)
      bits 3..8     bit n set = check n failed
      bits 12..15   verdict code 0..5
      bits 16..31   the first GL error of the arms (0 none)
  w2  bits 0..15    first differing top-down index of check 4, bits 16..31 of check 5
  w3  bits 0..15    the same for check 6, bits 16..31 for check 7; 4096 = none

The outcome is the row of docs/notes/gc-depth-sample-wire.md, "The three outcomes":
code 1 (a); code 2 (a) plus flush; code 3 (a), or (a) plus flush when check 7 failed;
code 4 (b); code 5 (c). Code 0 (check 3 failed) is no verdict: rerun the session.

Prints one line per check, then `verdict:` and `outcome:` lines. Exit status:
  0  a verdict: check 3 passed and the page is consistent
  1  no verdict: check 3 failed, the page is not valid, or its words disagree
     (the code is not the one its bits give, a bit without its index, a reserved bit)
  2  usage error: a word that is not a 32-bit hex number, a file that cannot be read
  3  the log holds no dtpu page (`no dtpu page` in it: check 1 or 2 stopped the case)
"""
import os
import re
import sys

PAGE_W0 = 0x594B0CB0  # YK_RECORD_MAGIC | YK_RECORD_KIND_PERF << 8 | 0xb0
NONE = 4096  # DTPU_PIXELS: no differing index
RECORD = re.compile(r"yk-record: ([0-9a-fA-F]{8}) ([0-9a-fA-F]{8}) ([0-9a-fA-F]{8}) ([0-9a-fA-F]{8})")
HEX = re.compile(r"(?:0[xX])?[0-9a-fA-F]{1,8}")

CHECKS = {
    3: "P0 off the ramp (today's path, the reference)",
    4: "P1 != P0 (D24X8 view not exact)",
    5: "P1 beyond 255 of P0",
    6: "P2 not the PE's word (raw A8R8G8B8 view)",
    7: "P3 off the deeper ramp or stale (no wait before the sample)",
    8: "a GL error in the arms",
}
VERDICTS = {
    0: "no verdict (check 3 failed)",
    1: "D24X8 exact",
    2: "D24X8 exact but stale without a wait (check 7)",
    3: "D24X8 with fewer bits (check 4 fails, 5 passes)",
    4: "D24X8 off but the raw view is the word (checks 4 and 5 fail, 6 passes)",
    5: "the LINEAR descriptor does not address the surface (check 6 fails too)",
}
# the check whose first differing index each half-word keeps
INDEX_OF = {4: (1, 0), 5: (1, 16), 6: (2, 0), 7: (2, 16)}
VALID_BITS = 0x1 | 0x1F8 | 0xF000 | 0xFFFF0000


def verdict_of(failed):
    """dtpu_verdict() of gl_test_dtpu.inc, from the set of failed checks."""
    if 3 in failed:
        return 0
    if 4 not in failed:
        return 2 if 7 in failed else 1
    if 5 not in failed:
        return 3
    return 4 if 6 not in failed else 5


def outcome_of(code, failed):
    if code == 0:
        return "none: check 3 failed, the record is not a verdict; rerun the same image and selector"
    if code in (1, 3):
        return "(a) plus flush" if 7 in failed else "(a)"
    return {2: "(a) plus flush", 4: "(b)", 5: "(c)"}[code]


def where(i):
    return "index %d (GL x %d, y %d)" % (i, i % 64, 63 - i // 64)


def decode(w1, w2, w3):
    """Return (lines, status) for the page words w1..w3."""
    words = (w1, w2, w3)
    lines = ["page: w1 0x%08x w2 0x%08x w3 0x%08x" % words]
    problems = []
    if not w1 & 1:
        problems.append("w1 bit 0 clear: the page is not valid (the arms did not run)")
    if w1 & ~VALID_BITS & 0xFFFFFFFF:
        problems.append("w1 reserved bits set: 0x%08x" % (w1 & ~VALID_BITS & 0xFFFFFFFF))
    failed = {n for n in CHECKS if w1 >> n & 1}
    gl_error = w1 >> 16
    for n in sorted(CHECKS):
        state = "FAIL" if n in failed else "pass"
        extra = ""
        if n in INDEX_OF:
            wi, shift = INDEX_OF[n]
            idx = words[wi] >> shift & 0xFFFF
            if idx != NONE:
                extra = ", first at " + where(idx)
            if (idx != NONE) != (n in failed):
                problems.append(
                    "check %d is %s but its index is %s" % (n, state, "none" if idx == NONE else idx))
            elif idx != NONE and idx > NONE:
                problems.append("check %d index %d is beyond the 64x64 target" % (n, idx))
        elif n == 8:
            if gl_error:
                extra = ", first GL error 0x%04x" % gl_error
            if bool(gl_error) != (8 in failed):
                problems.append("check 8 is %s but the GL error is 0x%04x" % (state, gl_error))
        lines.append("check %d: %s  %s%s" % (n, state, CHECKS[n], extra))
    code = w1 >> 12 & 0xF
    lines.append("verdict: %d %s" % (code, VERDICTS.get(code, "(not a verdict code)")))
    if code != verdict_of(failed):
        problems.append("verdict code %d, but the failed checks give %d" % (code, verdict_of(failed)))
    if problems:
        lines += ["inconsistent: " + p for p in problems]
        lines.append("outcome: none: the page's words disagree, it is not a verdict")
        return lines, 1
    lines.append("outcome: " + outcome_of(code, failed))
    return lines, 0 if code else 1


def parse_word(s):
    if not HEX.fullmatch(s):
        raise ValueError("not a 32-bit hex word: %r" % s)
    return int(s, 16)


def read_log(path):
    if path == "-":
        return sys.stdin.buffer.read().decode("utf-8", "replace")
    if os.path.isdir(path):
        path = os.path.join(path, "console.txt")
    with open(path, "rb") as f:
        return f.read().decode("utf-8", "replace")


def main(argv):
    try:
        if len(argv) == 2 and argv[0] == "--log":
            text = read_log(argv[1])
            pages = [m for m in RECORD.finditer(text) if int(m.group(1), 16) == PAGE_W0]
            if not pages:
                why = ("the log says 'no dtpu page': check 1 or 2 stopped the case, the CASE record is the result"
                       if "no dtpu page" in text else "no yk-record line of kind PERF tag 0xb0")
                print("no dtpu page: " + why)
                return 3
            words = [int(pages[-1].group(k), 16) for k in (2, 3, 4)]
            if len(pages) > 1:
                print("note: %d dtpu pages in the log, the last one is read" % len(pages))
        elif len(argv) == 3 and not any(a.startswith("-") for a in argv):
            words = [parse_word(a) for a in argv]
        else:
            sys.stderr.write(__doc__)
            return 2
    except (OSError, ValueError) as e:
        print("error: %s" % e, file=sys.stderr)
        return 2
    lines, status = decode(*words)
    print("\n".join(lines))
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
