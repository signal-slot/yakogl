#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Unit tests of mrt4-verdict.py (python3 -I tools/cts/test_mrt4_verdict.py)."""
import contextlib
import importlib.util
import io
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location('mrt4_verdict', os.path.join(HERE, 'mrt4-verdict.py'))
mv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mv)

P = 'dEQP-GLES3.functional.'
EXPECTED = '\n'.join([
    '# comment',
    'case\tgpu\toutcome_at_3\texpected_outcome_at_4\treason',
    P + 'implementation_limits.max_draw_buffers\tv3d42\tFail\tPass\tr',
    P + 'implementation_limits.max_draw_buffers\tbxs\tFail\tPass\tr',
    P + 'implementation_limits.max_color_attachments\tv3d42\tFail\tPass\tr',
    P + 'state_query.integers.x\tv3d42\tFail\tunmeasured\tr',
    P + 'draw_buffers_indexed.random.m.{0-2}\tv3d42\tNotSupported (3 cases)\tNotSupported\tr',
    P + 'fragment_out.random.{5,7}\tbxs\tNotSupported\tPass|NotSupported; unmeasured\tr',
    P + 'fragment_out.array.*\tv3d42\tFail\tFail\tr',
    P + 'fragment_out.random.* some prose\tbxs\tFail\tFail\tr',
]) + '\n'

MAXDB = P + 'implementation_limits.max_draw_buffers'
MAXCA = P + 'implementation_limits.max_color_attachments'


def run(gpu, results, expected=EXPECTED, qpa=False):
    with tempfile.TemporaryDirectory() as d:
        e, r = os.path.join(d, 'e.tsv'), os.path.join(d, 'r.txt')
        with open(e, 'w') as f:
            f.write(expected)
        with open(r, 'w') as f:
            if qpa:
                for c, s in results.items():
                    f.write('#beginTestCaseResult %s\n<Result StatusCode="%s">x</Result>\n'
                            '#endTestCaseResult\n' % (c, s))
            else:
                f.write(''.join('%s\t%s\n' % kv for kv in results.items()))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = mv.main(['--gpu', gpu, '--expected', e, r])
    return rc, out.getvalue().splitlines()


V3D_OK = {MAXDB: 'Pass', MAXCA: 'Pass',
          P + 'draw_buffers_indexed.random.m.0': 'NotSupported',
          P + 'draw_buffers_indexed.random.m.1': 'NotSupported',
          P + 'draw_buffers_indexed.random.m.2': 'NotSupported',
          P + 'fragment_out.array.a': 'Fail', P + 'fragment_out.array.b': 'Fail',
          P + 'other.case': 'Crash'}


class Verdict(unittest.TestCase):
    def test_match_plain_and_qpa(self):
        for qpa in (False, True):
            rc, out = run('v3d', V3D_OK, qpa=qpa)
            self.assertEqual(rc, 0)
            self.assertEqual(out, ['MRT4 VERDICT v3d PASS n_match=7 n_regress=0 n_missing=0'])

    def test_regression(self):
        r = dict(V3D_OK, **{MAXDB: 'Fail'})
        rc, out = run('v3d', r)
        self.assertEqual(rc, 1)
        self.assertEqual(out[0], 'MRT4 v3d %s expected=Pass got=Fail REGRESSION' % MAXDB)
        self.assertEqual(out[-1], 'MRT4 VERDICT v3d FAIL n_match=6 n_regress=1 n_missing=0')

    def test_improvement_does_not_fail(self):
        r = dict(V3D_OK, **{P + 'fragment_out.array.a': 'Pass'})
        rc, out = run('v3d', r)
        self.assertEqual(rc, 0)
        self.assertEqual(out[0], 'MRT4 v3d %sfragment_out.array.a expected=Fail got=Pass IMPROVEMENT' % P)
        self.assertEqual(out[-1], 'MRT4 VERDICT v3d PASS n_match=6 n_regress=0 n_missing=0')

    def test_missing(self):
        r = dict(V3D_OK)
        del r[MAXCA]
        del r[P + 'draw_buffers_indexed.random.m.2']
        rc, out = run('v3d', r)
        self.assertEqual(rc, 1)
        self.assertIn('MRT4 v3d %s expected=Pass got=missing MISSING' % MAXCA, out)
        self.assertEqual(out[-1], 'MRT4 VERDICT v3d FAIL n_match=5 n_regress=0 n_missing=2')

    def test_wildcard_without_results_is_missing(self):
        r = {k: v for k, v in V3D_OK.items() if 'fragment_out.array' not in k}
        rc, out = run('v3d', r)
        self.assertEqual(rc, 1)
        self.assertIn('MRT4 v3d %sfragment_out.array.* expected=Fail got=missing MISSING' % P, out)

    def test_pvr_uses_its_own_rows(self):
        r = {MAXDB: 'Pass', P + 'fragment_out.random.5': 'Pass',
             P + 'fragment_out.random.7': 'NotSupported'}
        rc, out = run('pvr', r)
        self.assertEqual(rc, 0)
        self.assertEqual(out, ['MRT4 VERDICT pvr PASS n_match=3 n_regress=0 n_missing=0'])
        rc, out = run('pvr', dict(r, **{P + 'fragment_out.random.5': 'Fail'}))
        self.assertEqual(rc, 1)
        self.assertEqual(out[0], 'MRT4 pvr %sfragment_out.random.5 expected=Pass|NotSupported'
                         ' got=Fail REGRESSION' % P)

    def test_stable_order_and_ignored_cases(self):
        r = {MAXDB: 'Fail', MAXCA: 'Fail'}
        _, a = run('v3d', r)
        _, b = run('v3d', dict(reversed(list(r.items()))))
        self.assertEqual(a, b)
        self.assertTrue(a[0].startswith('MRT4 v3d ' + MAXDB))
        self.assertFalse(any('other.case' in l for l in run('v3d', V3D_OK)[1]))

    def test_expand(self):
        self.assertEqual(mv.expand('a.{1,3-5}.b'), ['a.1.b', 'a.3.b', 'a.4.b', 'a.5.b'])

    def test_unreadable(self):
        self.assertEqual(mv.main(['--gpu', 'v3d', '/nonexistent/r']), 2)


if __name__ == '__main__':
    unittest.main()
