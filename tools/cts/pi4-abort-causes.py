#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The refusing case and its cause, from Pi 4 dEQP consoles that stopped early.

    tools/cts/pi4-abort-causes.py [--refuse-list] <LABEL>.console.txt|<session dir> ...

Read-only. The input is what tools/cts/pi4-console-scan.py reads (the
<LABEL>.console.txt files of a tools/cts/log/<date>-pi4-gles3-*/ session, with
or without time stamps; a directory names its *.console.txt by name). On the
Pi 4 a dEQP-GLES3 image stops at the first draw the V3D back end refuses
(dEQP takes the GL_OUT_OF_MEMORY as a ResourceError and aborts the run), so
the rest of the label never begins and the label is re-split at that case
(docs/es3/board-recipe-pi4.md section 6, tools/cts/pi4-gles3-resplit.py).

One line per console, in the order given, fields separated by a TAB:

  LABEL  DONE                                  the run's "deqp: done: ... complete=1"
  LABEL  CASE  CLASS  N: TEXT                  any other console

CASE is the last case that began ("Test case '<case>'.."), '-' when none
did. CLASS is the cause, from the back end's own lines (src/v3d/backend.rs
count_refusal(): "yakogl/v3d: draw refused: <why>", logged once per cause):

  tile-memory   "a target larger than the tile memory is sized for (...)"
                (backend.rs MAX_TARGET; 2048x2048 at b1b5b845, 4096 later)
  texel-format  "a texel format the V3D does not sample" (texture.rs NOT_SAMPLED:
                ETC2/EAC, the RGB10A2 class)
  gpu-memory    "no empty job holds the draw" after an "out of GPU memory" /
                "no GPU memory for" line in the same case (the OSAL or the
                back end could not allocate, then the draw fit no job)
  job-builder   "the job builder refused the draw (a uniform, program or
                primitive it does not write)" (backend.rs, Attempt::Unsupported)
  unknown       any other refusal, or none found

N: TEXT is the console line (1-based line number, the text without its time
stamp) the class was taken from: the "draw refused:" line inside the
refusing case; when the case ended in "deqp: ResourceError" but the cause
was already logged by an earlier case (the line is printed once per cause),
the last "draw refused:" line before the case, else the record's
"  rN (first #K) <why>" cause line first seen at or before the case. "-"
when none (a console that stops without a refusal or a ResourceError: a
hang, a reset, a cut copy; pi4-console-scan.py tells those apart).

--refuse-list prints only "LABEL CASE" for each console that stopped at a
case: the --refuse-file input of tools/cts/pi4-gles3-resplit.py.

Exit status: 0 every console read (stopped or not); 2 bad input or usage.
The output depends on the files' contents and the argument order only.
"""
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def _scan_module():
    """tools/cts/pi4-console-scan.py's parsing (clean, stamps, inputs, labels)."""
    spec = importlib.util.spec_from_file_location('pi4_console_scan', os.path.join(HERE, 'pi4-console-scan.py'))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


SCAN = _scan_module()

# dEQP's tcuTestSessionExecutor: "Test case '<path>'.."
CASE_BEGIN = re.compile(r"^Test case '(dEQP-[^']+)'\.\.")
# deqp main.cpp: "deqp: done: %u passed, ..., complete=%d"
DONE = re.compile(r'^deqp: done: .*\bcomplete=1\b')
# src/v3d/backend.rs count_refusal()
REFUSED = re.compile(r'^yakogl/v3d: draw refused: (.*)')
# v3d_board.c print_record(): the cause list "  r0 (first #3) <why>"
RECORD_CAUSE = re.compile(r'^  r\d+ \(first #(\d+)\) (.*)')
# v3d_board.c record_case(): "v3d_deqp: case #N ..."
CASE_NO = re.compile(r'^v3d_deqp: case #(\d+) ')
# src/osal/zephyr_v3d.c / linux.c "osal(...): out of GPU memory for", backend.rs "no GPU memory for"
NO_MEMORY = re.compile(r'out of GPU memory for|no GPU memory for')

CAUSES = (
    ('tile-memory', re.compile(r'a target larger than the tile memory')),
    ('texel-format', re.compile(r'a texel format the V3D does not sample')),
    ('job-builder', re.compile(r'the job builder refused the draw')),
)
NO_EMPTY_JOB = re.compile(r'no empty job holds the draw')


def classify(why, oom):
    for name, pat in CAUSES:
        if pat.search(why):
            return name
    if NO_EMPTY_JOB.search(why):
        return 'gpu-memory' if oom else 'unknown'
    return 'unknown'


def analyse(lines):
    """lines: [(t, text)]. ('DONE',) or (case, class, 'N: TEXT')."""
    texts = [t for _, t in lines]
    if any(DONE.match(t) for t in texts):
        return ('DONE',)
    begin, case = None, '-'
    for i, t in enumerate(texts):
        m = CASE_BEGIN.match(t)
        if m:
            begin, case = i, m.group(1)
    if begin is None:
        return (case, 'unknown', '-')
    # The refusing case's window: from its "Test case" line to the end.
    window = range(begin, len(texts))
    oom = any(NO_MEMORY.search(texts[i]) for i in window)
    hit = next((i for i in window if REFUSED.match(texts[i])), None)
    resource_error = any(texts[i].startswith('deqp: ResourceError ') for i in window)
    if hit is not None:
        why = REFUSED.match(texts[hit]).group(1)
    elif not resource_error:
        # No refusal named and no ResourceError: the console stops for
        # another reason (a hang, a reset, a cut copy); not a refused draw.
        return (case, 'unknown', '-')
    else:
        # The cause was logged once already, by an earlier case.
        before = [i for i in range(begin) if REFUSED.match(texts[i])]
        if before:
            hit = before[-1]
            why = REFUSED.match(texts[hit]).group(1)
            # The memory line belongs with the case that logged the cause.
            s = max([i for i in range(hit + 1) if CASE_BEGIN.match(texts[i])] or [0])
            e = next((i for i in range(hit + 1, len(texts)) if CASE_BEGIN.match(texts[i])), len(texts))
            oom = any(NO_MEMORY.search(texts[i]) for i in range(s, e))
        else:
            # The record's cause list: the cause first seen at or before the case.
            num = None
            for i in window:
                m = CASE_NO.match(texts[i])
                if m and texts[i].rstrip().endswith(' ' + case):
                    num = int(m.group(1))
                    break
            best = None
            for i, t in enumerate(texts):
                m = RECORD_CAUSE.match(t)
                if m and (num is None or int(m.group(1)) <= num):
                    if best is None or int(m.group(1)) >= int(RECORD_CAUSE.match(texts[best]).group(1)):
                        best = i
            if best is None:
                return (case, 'unknown', '-')
            hit = best
            why = RECORD_CAUSE.match(texts[hit]).group(2)
    return (case, classify(why, oom), '%d: %s' % (hit + 1, texts[hit]))


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    refuse_list = False
    paths = []
    for a in argv:
        if a == '--refuse-list':
            refuse_list = True
        elif a in ('-h', '--help'):
            print(__doc__)
            return 0
        elif a.startswith('-'):
            print(__doc__, file=sys.stderr)
            return 2
        else:
            paths.append(a)
    if not paths:
        print(__doc__, file=sys.stderr)
        return 2
    out = []
    try:
        for p in paths:
            for f in SCAN.inputs(p):
                r = analyse(SCAN.read_log(f))
                label = SCAN.label_of(f)
                if refuse_list:
                    if r[0] not in ('DONE', '-'):
                        out.append('%s %s' % (label, r[0]))
                else:
                    out.append('\t'.join((label,) + r))
    except OSError as e:
        print('pi4-abort-causes: %s' % e, file=sys.stderr)
        return 2
    for line in out:
        print(line)
    return 0


if __name__ == '__main__':
    sys.exit(main())
