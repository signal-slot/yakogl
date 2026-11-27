#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Decode gl_test case 222 `dtpu`'s page into checks 3..8, the verdict code
and the outcome it selects (docs/notes/gc-depth-sample-wire.md).

  imx8mp-dtpu-page.py <w1> <w2> <w3>       the record's words (hex, 0x optional)
  imx8mp-dtpu-page.py --log <console.log>  the record's (redacted) console
                                           capture; '-' reads stdin

Where the page comes from (read only; if the writer changes, this changes):

  driver/samples/zephyr/gl_test/src/main_imx8mp.c
    191-193  gl_test_dtpu_page(), the page of case 222 from gl_test_dtpu.inc,
             selector GC_RESULT_PERF_SEL | 0xb0
    336-353  dtpu_page_sel(): the selector is GC_RESULT_PERF_SEL + 0xb0
             (0x10b0, SEL=4272) and the image runs case 222
    477-485  gc_result_write_tag(GC_RESULT_PERF, 0xb0, w[0], w[1], w[2]): the
             page's w[0..2] are the record's w1..w3, tag 0xb0 in w0's low
             byte; "no dtpu page" (485) when case 222 did not reach its arms
  driver/tools/pvrlab/gl_test_dtpu.inc (included by gl_test.c)
    12-25    the bits: w1 bit 0 page valid; bit n (3..8) check n failed;
             bits 12..15 verdict code; bits 16..31 the arms' first GL error;
             w2 lo/hi first differing index of check 4 / check 5; w3 lo/hi of
             check 6 / check 7; 4096 = none; index = y_top * 64 + x
    33-40    the verdict codes DTPU_V_NO_REF (0) .. DTPU_V_NONE (5)
    48-57    dtpu_verdict(), recomputed below from the fail bits
    60-72    dtpu_encode()
  driver/zephyr/lib/board/include/yakogl_record.h
    19, 22, 36  YK_RECORD_MAGIC 0x594b0000, the console line
             "yk-record: %08x %08x %08x %08x", kind PERF = 12, so the page's
             w0 is 0x594b0cb0

Outcome per gc-depth-sample-wire.md, "The three outcomes": code 1 -> (a);
code 2 -> (a)+flush; code 3 (check 4 fails, 5 passes) -> (a), or (a)+flush
when check 7 also failed; code 4 -> (b); code 5 -> (c); code 0 (check 3
failed) is no verdict.

Exit status: 0 a verdict; 1 not a verdict (check 3 failed, rerun); 2 no
page, a malformed page or log, or a usage error. Host only, stdlib only;
run it as `python3 -I`.
"""
import re
import sys

PAGE_W0 = 0x594B0000 | 12 << 8 | 0xB0  # yakogl_record.h:19,36; main_imx8mp.c:482
NONE = 4096  # gl_test_dtpu.inc: DTPU_NONE
# bits gl_test_dtpu.inc:13-19 may set in w1's low half
W1_LOW_KNOWN = 0x1 | 0x1F8 | 0xF000

CHECKS = {
    3: "P0 (today's path) on the ramp, the reference",
    4: "P1 (D24X8 view) == P0",
    5: "P1 within 255 of P0",
    6: "P2 (raw A8R8G8B8 view) is the PE's word",
    7: "P3 on the deeper ramp (not stale)",
    8: "no GL error in the arms",
}

VERDICTS = {
    0: "no verdict (check 3 failed)",
    1: "D24X8 exact",
    2: "D24X8 exact but stale without a wait (check 7)",
    3: "D24X8 with fewer bits (check 4 fails, 5 passes)",
    4: "D24X8 off, the raw view is the word (check 6 passes)",
    5: "the LINEAR descriptor does not address the surface",
}

OUTCOMES = {
    "(a)": "a depth view of the surface, no copy, no CPU resolve on the draw path",
    "(a)+flush": "(a) plus the texture-cache flush of (a).9",
    "(b)": "a GPU copy, in the stream, into a layout the TPU reads",
    "(c)": "the CPU resolve stays; its three CPU fixes",
}

RECORD = re.compile(r"yk-record:\s*(.*)")
WORD = re.compile(r"(?:0[xX])?([0-9a-fA-F]{1,8})")


class Bad(Exception):
    pass


def parse_word(s):
    m = WORD.fullmatch(s.strip())
    if not m:
        raise Bad("not a 32-bit hex word: %r" % s)
    return int(m.group(1), 16)


def words_from_log(text):
    """The last dtpu page's w1..w3 in a console capture, and the page count."""
    pages = []
    no_page = False
    for n, line in enumerate(text.splitlines(), 1):
        if "no dtpu page" in line:
            no_page = True
        m = RECORD.search(line)
        if not m:
            continue
        fields = m.group(1).split()
        if not fields or fields[0].lower() != "%08x" % PAGE_W0:
            continue
        if len(fields) < 4 or not all(re.fullmatch(r"[0-9a-fA-F]{8}", f) for f in fields[:4]):
            raise Bad("line %d: malformed dtpu page record: %s" % (n, line.strip()))
        pages.append([int(f, 16) for f in fields[1:4]])
    if not pages:
        if no_page:
            raise Bad("no dtpu page: the log says check 1 or 2 stopped case 222; "
                      "its CASE record is the result")
        raise Bad("no dtpu page in the log (no 'yk-record: %08x ...' line)" % PAGE_W0)
    return pages[-1], len(pages)


def verdict_of(failed):
    # gl_test_dtpu.inc:48-57, dtpu_verdict()
    if failed[3]:
        return 0
    if not failed[4]:
        return 2 if failed[7] else 1
    if not failed[5]:
        return 3
    return 5 if failed[6] else 4


def outcome_of(code, failed):
    if code == 1:
        return "(a)"
    if code == 2:
        return "(a)+flush"
    if code == 3:
        return "(a)+flush" if failed[7] else "(a)"
    if code == 4:
        return "(b)"
    if code == 5:
        return "(c)"
    return None


def where(i):
    # gl_test_dtpu.inc:24: index = y_top * 64 + x; GL y = 63 - y_top
    return "index %d (GL x %d, y %d)" % (i, i % 64, 63 - i // 64)


def decode(w1, w2, w3, out):
    if not w1 & 1:
        raise Bad("w1 bit 0 is clear: not a valid dtpu page")
    if w1 & 0xFFFF & ~W1_LOW_KNOWN:
        raise Bad("w1 0x%08x sets bits outside gl_test_dtpu.inc's layout" % w1)
    failed = {n: bool(w1 >> n & 1) for n in range(3, 9)}
    code = w1 >> 12 & 0xF
    glerr = w1 >> 16
    idx = {4: w2 & 0xFFFF, 5: w2 >> 16, 6: w3 & 0xFFFF, 7: w3 >> 16}
    for n, i in idx.items():
        if i > NONE:
            raise Bad("check %d's index %d is beyond 4096" % (n, i))
        if (i != NONE) != failed[n]:
            raise Bad("check %d: fail bit %d but index %d" % (n, failed[n], i))
    if (glerr != 0) != failed[8]:
        raise Bad("check 8: fail bit %d but GL error 0x%04x" % (failed[8], glerr))
    if code not in VERDICTS:
        raise Bad("verdict code %d is not a DTPU_V_* code" % code)
    want = verdict_of(failed)
    if code != want:
        raise Bad("verdict code %d disagrees with the fail bits (they give %d)" % (code, want))

    out.write("page: w1=0x%08x w2=0x%08x w3=0x%08x\n" % (w1, w2, w3))
    for n in range(3, 9):
        if not failed[n]:
            out.write("check %d: pass (%s)\n" % (n, CHECKS[n]))
        elif n == 3:
            out.write("check 3: fail (%s; its index is not kept)\n" % CHECKS[3])
        elif n == 8:
            out.write("check 8: fail (first GL error 0x%04x)\n" % glerr)
        else:
            out.write("check %d: fail (%s) first at %s\n" % (n, CHECKS[n], where(idx[n])))
    out.write("verdict: %d (%s)\n" % (code, VERDICTS[code]))
    if failed[3]:
        return None
    o = outcome_of(code, failed)
    out.write("outcome: %s (%s)\n" % (o, OUTCOMES[o]))
    if failed[8]:
        out.write("note: check 8 failed, a GL error in the arms; read the outcome with care\n")
    return o


USAGE = "usage: imx8mp-dtpu-page.py <w1> <w2> <w3> | --log <console.log|->\n"


def main(argv):
    try:
        if len(argv) == 2 and argv[0] == "--log":
            if argv[1] == "-":
                data = sys.stdin.buffer.read()
            else:
                with open(argv[1], "rb") as f:
                    data = f.read()
            (w1, w2, w3), count = words_from_log(data.decode("latin-1"))
            if count > 1:
                sys.stdout.write("note: %d dtpu pages in the log, reading the last\n" % count)
        elif len(argv) == 3 and not any(a.startswith("-") for a in argv):
            w1, w2, w3 = (parse_word(a) for a in argv)
        else:
            sys.stderr.write(USAGE)
            return 2
        o = decode(w1, w2, w3, sys.stdout)
    except (Bad, OSError) as e:
        sys.stderr.write("imx8mp-dtpu-page: %s\n" % e)
        return 2
    if o is None:
        sys.stderr.write("not a verdict: check 3 failed, rerun\n")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
