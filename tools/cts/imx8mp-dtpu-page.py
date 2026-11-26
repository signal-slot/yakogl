#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""imx8mp-dtpu-page.py - decode gl_test case 222 `dtpu`'s page into checks, verdict and outcome.

  python3 -I tools/cts/imx8mp-dtpu-page.py W1 W2 W3        (hex words, 0x optional)
  python3 -I tools/cts/imx8mp-dtpu-page.py --log CONSOLE   (a record's redacted console capture)

Where the page comes from (driver/samples/zephyr/gl_test/src/main_imx8mp.c):
  :191-193  gl_test_dtpu_page(): "case 222 dtpu's page of every check, selector GC_RESULT_PERF_SEL | 0xb0"
  :342-346  dtpu_page_sel(): the page is on when the selector is GC_RESULT_PERF_SEL + 0xb0 (0x1000+0xb0 = 4272)
  :477-482  the page is written as gc_result_write_tag(GC_RESULT_PERF, 0xb0, w[0], w[1], w[2]),
            after the console line "gc_gl_test: case 222 dtpu page 0xb0"
  :485      without the arms: "gc_gl_test: no dtpu page (case 222 did not reach its arms)" and no page
The console line of a record is YK_RECORD_LINE_FMT (driver/zephyr/lib/board/include/yakogl_record.h:21-22),
"yk-record: %08x %08x %08x %08x" = w0 w1 w2 w3, with w0 = 0x594b0000 | kind << 8 | tag (record.rs w0()),
kind PERF = 12, so the page's w0 is 0x594b0cb0 and w1..w3 are the record's 2nd..4th words.
(The words and bits are those of docs/notes/gc-depth-sample.md "Reading the dtpu page"; the code that
packs them is driver/tools/pvrlab/gl_test_dtpu.inc, not read here.)

Bit layout of the three words (main_imx8mp.c writes them unchanged, so the layout is the note's):
  w1 bit 0       page valid
  w1 bits 3..8   bit n set = check n FAILED (3 P0 off the ramp, 4 P1 != P0, 5 P1 beyond 255 of P0,
                 6 P2 not the PE's word, 7 P3 off the deeper ramp / stale, 8 a GL error in the arms)
  w1 bits 12..15 verdict code: 0 none (check 3 failed), 1 D24X8 exact, 2 exact but stale without a
                 wait (7), 3 fewer bits (4 fails, 5 passes), 4 raw view is the word (4,5 fail, 6 passes),
                 5 the LINEAR descriptor does not address the surface
  w1 bits 16..31 first GL error of the arms (0 none)
  w2 / w3        16-bit halves: first differing top-down index of check 4, 5 / check 6, 7; 4096 = none

Outcome (docs/notes/gc-depth-sample-wire.md "The three outcomes"):
  code 1, 3 -> (a); a failed check 7 on top of those -> (a)+flush; code 2 -> (a)+flush
  code 4 -> (b);  code 5 -> (c);  code 0 -> not a verdict.
Exit: 0 decoded, 1 check 3 failed ("not a verdict: check 3 failed, rerun"), 2 bad input.
"""
import re
import sys

NONE = 4096
CHECKS = {
    3: "P0 on the ramp (the CPU reference)",
    4: "P1 (D24X8 view) equals P0",
    5: "P1 within 255 of P0",
    6: "P2 (raw view) is the PE's word",
    7: "P3 on the deeper ramp (no stale cache)",
    8: "no GL error in the arms",
}
CODES = {
    0: "no verdict (check 3 failed)",
    1: "D24X8 exact",
    2: "D24X8 exact but stale without a wait",
    3: "D24X8 with fewer bits",
    4: "D24X8 off, the raw view is the word",
    5: "the LINEAR descriptor does not address the surface",
}
PAGE_W0 = 0x594B0CB0
LINE = re.compile(r"^yk-record: ([0-9a-fA-F]{8}) ([0-9a-fA-F]{8}) ([0-9a-fA-F]{8}) ([0-9a-fA-F]{8})\s*$")


class Bad(Exception):
    pass


def word(s):
    try:
        v = int(s, 16)
    except ValueError:
        raise Bad("not a hex word: %r" % s)
    if not 0 <= v <= 0xFFFFFFFF:
        raise Bad("word out of 32 bits: %r" % s)
    return v


def from_log(path):
    try:
        text = open(path, errors="replace").read()
    except OSError as e:
        raise Bad("%s: %s" % (path, e))
    page = None
    for ln in text.splitlines():
        m = LINE.match(ln.strip())
        if m and int(m.group(1), 16) == PAGE_W0:
            page = [int(m.group(i), 16) for i in (2, 3, 4)]
    if page is None:
        why = "no dtpu page" if "no dtpu page" in text else "no 'yk-record: 594b0cb0 ...' line"
        raise Bad("%s: %s (the case did not reach its arms, or the log is not a case 222 page run)" % (path, why))
    return page


def decode(w1, w2, w3):
    if not w1 & 1:
        raise Bad("page valid bit (w1 bit 0) is clear: w1=%08x" % w1)
    failed = {n: bool(w1 >> n & 1) for n in CHECKS}
    code = w1 >> 12 & 15
    if code not in CODES:
        raise Bad("verdict code %d is not 0..5" % code)
    err = w1 >> 16
    idx = {4: w2 & 0xFFFF, 5: w2 >> 16, 6: w3 & 0xFFFF, 7: w3 >> 16}
    for n, i in idx.items():
        if i > NONE:
            raise Bad("check %d index %d is neither an index (<4096) nor 4096 = none" % (n, i))
    if failed[3] != (code == 0):
        raise Bad("check 3 %s but verdict code is %d" % ("failed" if failed[3] else "passed", code))
    return failed, code, err, idx


def outcome(failed, code):
    if code == 0:
        return None
    if code in (1, 3):
        return "(a)+flush" if failed[7] else "(a)"
    return {2: "(a)+flush", 4: "(b)", 5: "(c)"}[code]


def main(argv):
    if len(argv) == 2 and argv[0] == "--log":
        w = from_log(argv[1])
    elif len(argv) == 3:
        w = [word(a) for a in argv]
    else:
        raise Bad("usage: imx8mp-dtpu-page.py W1 W2 W3 | --log CONSOLE")
    w1, w2, w3 = w
    failed, code, err, idx = decode(w1, w2, w3)
    print("page: w1=%08x w2=%08x w3=%08x" % (w1, w2, w3))
    for n in sorted(CHECKS):
        extra = ""
        if failed[n] and n in idx and idx[n] != NONE:
            i = idx[n]
            extra = " first at index %d (x=%d y=%d)" % (i, i % 64, 63 - i // 64)
        if n == 8 and failed[n]:
            extra = " GL error 0x%04x" % err
        print("check %d: %s - %s%s" % (n, "FAIL" if failed[n] else "pass", CHECKS[n], extra))
    print("verdict code %d: %s" % (code, CODES[code]))
    if code == 0:
        print("not a verdict: check 3 failed, rerun")
        return 1
    print("outcome: %s" % outcome(failed, code))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except Bad as e:
        print("imx8mp-dtpu-page: %s" % e, file=sys.stderr)
        sys.exit(2)
