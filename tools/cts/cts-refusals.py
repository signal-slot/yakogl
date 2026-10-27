#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""cts-refusals.py - what the shader compiler REFUSED, bucketed by its own reason.

A dEQP failure line says "expected shaders to compile and link properly, but
failed to compile". That is dEQP's sentence, not ours, and it is the same
sentence for every cause -- so a run of 1,778 sub-cases produces one message
repeated, and no work list.

Our compiler's reason is in the .qpa: dEQP records the shader info log, and
ours reads `0:<line>: <reason>`. This buckets those by reason, with the count
and the cases behind each, which turns a wall of identical failures into an
ordered list of things to implement.

    python3 tools/cts/cts-refusals.py build/deqp-online-<hash>
    python3 tools/cts/cts-refusals.py --cases 8 <dir>   # more names per bucket

It reports ONLY compile refusals. A case that compiled and then drew the wrong
picture is a different question and is deliberately not mixed in here: on a
board where a driver defect can corrupt every verdict, the refusal histogram
stays readable when the verdicts do not.

**A refusal is not automatically a gap.** dEQP has negative tests -- a case
that hands the compiler illegal GLSL and PASSES when it is rejected. Measured
on the first full sweep: 8,636 cases carried a refusal and 825 of them PASSED,
so 645 refusals across 49 reasons were the compiler being RIGHT. Top of that
list is "no implicit conversion in GLSL ES 1.00", 262 cases, every one a
`shaders.invalid_implicit_conversions.*` test that exists to check exactly that
rejection. Read as a work list, that entry says "implement implicit
conversions" -- which would turn 262 passes into 262 failures.

So the histogram is split by the case's own verdict, and the work list is the
FAILING side only. A reason that appears on both is shown with its passing
count beside it, because that means the same message covers a real gap and a
correct rejection and the two have to be told apart before either is touched.
"""
import argparse
import re
import sys
from collections import defaultdict
from pathlib import Path

CASE = re.compile(r'<TestCaseResult[^>]*\bCasePath="([^"]+)"')
RESULT = re.compile(r'<Result\s+StatusCode="([^"]+)"')
# The info log, in both places dEQP writes one: a program's <InfoLog>, and the
# `<Section Name="Info Log"><Text>` of the cases that build their own program
# (attribute_location.*). Reading only the first missed 16 back-end refusals
# on the host PowerVR run (2026-09-24, usc-backend): 664 counted of 680.
INFO = re.compile(r'<InfoLog>(.*?)</InfoLog>|<Section Name="Info Log"[^>]*>\s*<Text>(.*?)</Text>', re.S)
# `0:<line>: '<tok>' : <message>` -- glslang's shape, which the online compiler
# adopted in fc88b41. The token is optional because not every path emits one.
#
# The entities matter and cost a near-miss. dEQP writes the info log XML-escaped,
# so the quotes around the token arrive as `&apos;`. The first version of this
# pattern excluded `&` from the message (to stop at an entity), which was fine
# while the message had no token -- and silently captured NOTHING the moment
# one appeared, because `&apos;` begins immediately after the colon. Every
# refusal would have bucketed under the empty string and the histogram would
# have been a single nameless row. Unescape first, then parse.
REASON = re.compile(r"0:(\d+):\s*(?:'([^']*)'\s*:\s*)?([^<\n]+)")
# The back end's refusals carry no `0:<line>:` -- they are about the program,
# not a line of it: `online shaderc: the code generator refused this fragment
# shader: Select(OutOfScratch)`. Matched only in a log with no `0:<line>:`
# reason, and keyed on the WHOLE line, never a prefix. Until 2026-09-24 this
# tool read nothing else, and 34 real gaps on the GC7000 (32 Select(OutOfScratch)
# in shaders.indexing, gl_FrontFacing, gl_PointCoord) never reached the work
# list, whose total read 180 where the .qpa held 214.
BACKEND = re.compile(r"online shaderc: ([^<\n]+)")

def unescape(s):
    """XML entities dEQP writes, in the order that avoids double-decoding."""
    for a, b in (('&lt;', '<'), ('&gt;', '>'), ('&quot;', '"'),
                 ('&apos;', "'"), ('&amp;', '&')):
        s = s.replace(a, b)
    return s

def scan(qpa):
    """Yield (case, reason, token, status) for every compile refusal in one
    .qpa; `status` is dEQP's StatusCode ('NoResult' when there is none)."""
    text = qpa.read_text(errors='replace')
    # Split on case boundaries so an info log is attributed to its own case.
    parts = CASE.split(text)
    # parts = [preamble, case1, body1, case2, body2, ...]
    for i in range(1, len(parts) - 1, 2):
        case, body = parts[i], parts[i + 1]
        m = RESULT.search(body)
        status = m.group(1) if m else 'NoResult'
        seen = []
        for log in INFO.findall(body):
            text = unescape(log[0] or log[1])
            found = False
            for m2 in REASON.finditer(text):
                tok = (m2.group(2) or '').strip()
                msg = m2.group(3).strip().rstrip('.')
                if not msg:
                    continue
                found = True
                if (msg, tok) not in seen:
                    seen.append((msg, tok))
            if not found:
                for m3 in BACKEND.finditer(text):
                    msg = m3.group(1).strip().rstrip('.')
                    if msg and (msg, '') not in seen:
                        seen.append((msg, ''))
        for msg, tok in seen:
            yield case, msg, tok, status

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('results', help='a results directory holding .qpa files')
    ap.add_argument('--cases', type=int, default=4,
                    help='how many case names to print per bucket (default 4)')
    ap.add_argument('--tokens', type=int, default=8,
                    help='how many constructs to print per reason (default 8)')
    a = ap.parse_args()

    root = Path(a.results)
    qpas = sorted(root.rglob('*.qpa'))
    if not qpas:
        sys.exit(f"cts-refusals: no .qpa under {root} -- nothing to read, "
                 f"which is not the same as no refusals")

    gaps = defaultdict(list)      # refusals on cases dEQP did not pass
    correct = defaultdict(list)   # refusals on cases dEQP PASSED (negative tests)
    tokens = defaultdict(lambda: defaultdict(list))   # reason -> token -> cases
    # NotSupported is its own count: dEQP excuses the refusal (ES 2.0 lets an
    # implementation refuse a dynamic loop), so it is neither a pass nor a
    # failure dEQP charges -- but the shader is valid and we refused it, so
    # its reasons stay on the work list.
    by_status = defaultdict(int)
    for q in qpas:
        for case, reason, tok, status in scan(q):
            passed = status == 'Pass'
            (correct if passed else gaps)[reason].append(case)
            if not passed:
                tokens[reason][tok or '(no token)'].append(case)
                by_status[status] += 1

    ngap = sum(len(v) for v in gaps.values())
    nok = sum(len(v) for v in correct.values())
    nns = by_status.get('NotSupported', 0)
    print(f"{len(qpas)} .qpa file(s), {ngap + nok} compile refusal(s), "
          f"{len(set(gaps) | set(correct))} distinct reason(s)")
    print(f"  on cases dEQP FAILED (real gaps)        : {ngap - nns}"
          f"  in {len(gaps)} reason(s)")
    print(f"  on cases dEQP NOT SUPPORTED (excused)   : {nns}")
    print(f"  on cases dEQP PASSED (correct refusals) : {nok}"
          f"  in {len(correct)} reason(s)")
    buckets = gaps
    if not gaps and not correct:
        print("no refusal carried a `0:<line>:` reason. Either nothing was "
              "refused, or the refusals came from somewhere that does not "
              "name itself -- check a failure log before reading this as clean.")
        return 0
    print()
    print("=== THE WORK LIST: refusals on cases dEQP failed or excused, most first ===")
    for reason, cases in sorted(gaps.items(), key=lambda kv: -len(kv[1])):
        also = f"   (the same message also on {len(correct[reason])} PASSING cases"
        also += " -- one message, two causes, separate them first)" if correct.get(reason) else ""
        print(f"{len(cases):5d}  {reason}{also if correct.get(reason) else ''}")
        # The per-construct split, which is the whole reason the token was
        # worth carrying: "assignment to this is not lowered yet" is one
        # message over many constructs, and the token says which.
        by_tok = sorted(tokens[reason].items(), key=lambda kv: -len(kv[1]))
        if len(by_tok) > 1 or (by_tok and by_tok[0][0] != '(no token)'):
            for tok, tcases in by_tok[:a.tokens]:
                print(f"         {len(tcases):5d}  {tok}")
                for c in tcases[:a.cases]:
                    print(f"                {c}")
            if len(by_tok) > a.tokens:
                print(f"         ... and {len(by_tok) - a.tokens} more construct(s)")
        else:
            for c in cases[:a.cases]:
                print(f"         {c}")
            if len(cases) > a.cases:
                print(f"         ... and {len(cases) - a.cases} more")

    only_ok = {r: c for r, c in correct.items() if r not in gaps}
    if only_ok:
        print()
        print("=== NOT WORK ITEMS: reasons seen only where dEQP PASSED ===")
        print("These are negative tests: the case hands the compiler illegal GLSL")
        print("and passes because we reject it. Implementing one of these turns")
        print("its passing cases into failures.")
        for reason, cases in sorted(only_ok.items(), key=lambda kv: -len(kv[1]))[:12]:
            print(f"{len(cases):5d}  {reason}")
        rest = len(only_ok) - 12
        if rest > 0:
            print(f"         ... and {rest} more reason(s)")
    return 0

if __name__ == '__main__':
    sys.exit(main())
