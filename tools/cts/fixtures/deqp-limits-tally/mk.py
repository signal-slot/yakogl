#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Regenerate the fixture runs next to this file:
python3 -I tools/cts/fixtures/deqp-limits-tally/mk.py"""
import importlib.util
import os

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location(
    't', os.path.join(HERE, '..', '..', 'deqp-limits-tally.py'))
t = importlib.util.module_from_spec(spec)
spec.loader.exec_module(t)
names = t.require_cases()


def mk(n, drop=None, fail=None):
    d = os.path.join(HERE, n)
    os.makedirs(d, exist_ok=True)
    cs = [c for c in names if c != drop]
    with open(os.path.join(d, 'commit'), 'w') as f:
        f.write('0000000 v3d gles3\n')
    with open(os.path.join(d, 'all.txt'), 'w') as f:
        f.write('\n'.join(cs) + '\n')
    lines = []
    for c in cs:
        lines += (t.st_case(c, 'Fail', 'Expected 15, got 8') if c == fail
                  else t.st_case(c, 'Pass'))
    with open(os.path.join(d, 'c_000.qpa'), 'w') as f:
        f.write('\n'.join(lines) + '\n')


mk('allpass')
mk('onefail', fail=names[0])
mk('missing', drop=names[-1])
