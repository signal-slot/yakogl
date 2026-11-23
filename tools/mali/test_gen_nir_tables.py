#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tests of gen_nir_tables.py on a Mesa 26.2.1 tree.

    MESA_PAN_TREE=<mesa-26.2.1 tree> python3 -I tools/mali/test_gen_nir_tables.py

The condition evaluator must give every condition string the old blend
generator knows (gen_blend_nir_rules.py `CONDITIONS`, a table written by
hand) the value that table gives; the C expression reader must read what
the conditions and the options use; two runs must write the same bytes.
"""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True
sys.path.insert(0, HERE)
import gen_blend_nir_rules  # noqa: E402
import gen_nir_tables  # noqa: E402

TREE = os.environ.get('MESA_PAN_TREE', '')
MESA = None


def read(path):
    with open(path, 'rb') as f:
        return f.read()


def mesa():
    global MESA
    if MESA is None:
        MESA = gen_nir_tables.Mesa(TREE)
    return MESA


class CExpr(unittest.TestCase):
    def ev(self, text, **names):
        return gen_nir_tables.c_eval(text, lambda n: names[n.replace('->', '_')], gen_nir_tables.C_MACROS)

    def test_precedence_is_c_s(self):
        self.assertEqual(self.ev('1 | 2 & 0'), 1)
        self.assertEqual(self.ev('!0 && 1 || 0'), 1)
        self.assertEqual(self.ev('1 == 1 & 0'), 0)
        self.assertEqual(self.ev('(1 << 3) | 1'), 9)
        self.assertEqual(self.ev('a >= 9 ? ~(b) : ~0', a=10, b=64), ~64)
        self.assertEqual(self.ev('BITFIELD_BIT(2) | BITFIELD_MASK(3)'), 7)
        self.assertEqual(self.ev('0x10u & 16'), 16)

    def test_member_chains_are_names(self):
        self.assertEqual(self.ev('!options->x && info->stage != 4', options_x=0, info_stage=0), 1)

    def test_what_it_cannot_read_stops(self):
        for bad in ('1 +', 'a = 1', '"s"', 'f(1)', '(1'):
            with self.assertRaises(gen_nir_tables.GenError, msg=bad):
                self.ev(bad, a=1)


@unittest.skipUnless(TREE, 'MESA_PAN_TREE is not set')
class Conditions(unittest.TestCase):
    def test_the_old_table_s_values(self):
        """Every string of the hand table, with its value. The table was
        made for blend shaders, which are fragment shaders, so a string on
        info->stage is compared on the fragment stage."""
        cond = mesa().cond
        fs = dict(gen_nir_tables.STAGES)['MESA_SHADER_FRAGMENT']
        for text, want in gen_blend_nir_rules.CONDITIONS.items():
            m = cond.mask(text)
            if 'info->stage' not in text:
                self.assertIn(m, (0, gen_nir_tables.ALL_STAGES), text)
            self.assertEqual(bool(m & fs), want, text)

    def test_of_nir_opt_algebraic_s_176_the_old_table_knows_18(self):
        conds = set(x.condition for x in dict(mesa().passes)['nir_opt_algebraic'].xforms)
        self.assertEqual(len(conds), 176)
        self.assertEqual(len(conds & set(gen_blend_nir_rules.CONDITIONS)), 18)

    def test_the_stage_condition_is_a_mask(self):
        text = ('(info->stage != MESA_SHADER_VERTEX && info->stage != MESA_SHADER_GEOMETRY) '
                '&& !options->intel_vec4')
        self.assertEqual(mesa().cond.mask(text), dict(gen_nir_tables.STAGES)['MESA_SHADER_FRAGMENT'])

    def test_an_unknown_option_stops(self):
        with self.assertRaises(gen_nir_tables.GenError):
            mesa().cond.mask('options->no_such_option')


@unittest.skipUnless(TREE, 'MESA_PAN_TREE is not set')
class Generation(unittest.TestCase):
    def test_two_runs_write_the_same_bytes_and_no_host_path(self):
        outs = []
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            for d in (a, b):
                r = subprocess.run([sys.executable, '-I', os.path.join(HERE, 'gen_nir_tables.py'), TREE, '--out', d],
                                   capture_output=True, text=True,
                                   env=dict(os.environ, PYTHONDONTWRITEBYTECODE='1'))
                self.assertEqual(r.returncode, 0, r.stderr)
                outs.append({n: read(os.path.join(d, n)) for n in sorted(os.listdir(d))})
        self.assertEqual(outs[0], outs[1])
        self.assertEqual(sorted(outs[0]), sorted(n for n, _ in gen_nir_tables.OUTPUTS))
        for name, text in outs[0].items():
            self.assertNotIn(os.path.realpath(TREE).encode(), text, name)
            self.assertNotIn(os.path.expanduser('~').encode(), text, name)


if __name__ == '__main__':
    unittest.main()
