#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Generate tools/shaders/offarm/: femtovg's fill_color.nodiscard.frag with N
dependent steps added to scissorMask's `if (scissorRadius > 0.0)` arm, the
arm a zero scissorRadius never runs (gl_test 193 offarm and 194 offarmn).

    distance = distance * frag[13].y + frag[13].z;

after `float distance = sdroundrect(...)`, N times, under one comment line.
N = 0 is fill_color.nodiscard.frag itself and is not written. The offline
table keys a row by the SHA-256 of the text, so every byte here is part of
the key: change the text and the rows must be regenerated
(tools/glsl2usc.sh).

    tools/gen_offarm.py           write the files
    tools/gen_offarm.py --check   exit 1 if a file differs from what this writes
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BASE = os.path.join(HERE, 'shaders', 'femtovg', 'fill_color.nodiscard.frag')
OUT = os.path.join(HERE, 'shaders', 'offarm')
ANCHOR = '        float distance = sdroundrect(pt, scissorExt, scissorRadius);\n'
STEP = '        distance = distance * frag[13].y + frag[13].z;\n'
# 193 offarm: 8, 32, 128. 194 offarmn: 0, 2, 4, 8, 12, 16, 24, 32, 48, 64, 96,
# 128, 192, 256.
SERIES = [2, 4, 8, 12, 16, 24, 32, 48, 64, 96, 128, 192, 256]


def text(base, n):
    pad = ('        // gl_test 193 offarm: %d dependent steps in the arm a zero scissorRadius never runs\n' % n
           + STEP * n)
    return base.replace(ANCHOR, ANCHOR + pad, 1)


def main():
    check = sys.argv[1:] == ['--check']
    if sys.argv[1:] and not check:
        sys.exit(__doc__)
    base = open(BASE, encoding='ascii').read()
    if base.count(ANCHOR) != 1:
        sys.exit('gen_offarm: %s: the scissor arm anchor is not there exactly once' % BASE)
    bad = []
    for n in SERIES:
        path = os.path.join(OUT, 'fill_color.nodiscard.offarm%d.frag' % n)
        want = text(base, n)
        if check:
            have = open(path, encoding='ascii').read() if os.path.exists(path) else None
            if have != want:
                bad.append(path)
        else:
            os.makedirs(OUT, exist_ok=True)
            with open(path, 'w', encoding='ascii', newline='\n') as f:
                f.write(want)
    if bad:
        sys.exit('gen_offarm: differs from the generator: ' + ', '.join(bad))


if __name__ == '__main__':
    main()
