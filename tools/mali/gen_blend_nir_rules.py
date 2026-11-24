#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 AND MIT
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# SPDX-FileCopyrightText: 2014, 2016 Intel Corporation
# SPDX-FileCopyrightText: 2016 Red Hat
# SPDX-FileCopyrightText: 2018-2019 Alyssa Rosenzweig
# SPDX-FileCopyrightText: 2021 Collabora, Ltd.
#
# The MIT part is the data this script transcribes from Mesa 26.2.1 (MIT):
# the Mali-G610 option conditions of src/panfrost/compiler/bifrost/bifrost_compile.h
# and the fp-class relations of src/compiler/nir/nir_search_helpers.h.
"""Generate the NIR algebraic rules a Mali blend shader can meet, in Rust.

    tools/mali/gen_blend_nir_rules.py <mesa-26.2.1 tree> > src/shaderc/online/mali/blend_nir_rules.rs

The blend shader compiler (src/shaderc/online/mali/blend_nir.rs) runs Mesa's
NIR passes on the NIR that pan_blend_create_shader builds. Three of them are
nir_algebraic passes: nir_opt_algebraic and nir_opt_algebraic_late
(src/compiler/nir/nir_opt_algebraic.py) and bifrost_nir_lower_algebraic_late
(src/panfrost/compiler/bifrost/bifrost_nir_algebraic.py). This reads those
files with Mesa's own parser (nir_algebraic.py, nir_opcodes.py, imported from
the tree given, unmodified) and keeps, of each pass:

  - the automaton items (nir_algebraic.py TreeAutomaton._compute_items) built
    only of the opcodes a blend shader holds: the state of an instruction is
    the set of items it matches, and a use is visited again only when that set
    changes (nir_search.c nir_algebraic_update_automaton);
  - every transform whose search pattern is such an item and whose condition
    holds on the Mali-G610 (bifrost_compile.h's options for arch 10, a shader
    with no float controls), in the order the pass tries them.

A transform whose replacement needs an opcode the blend compiler does not
build is kept with `refuse`: if it matches, the shader is refused, never
compiled differently from Mesa. The generation fails (no output, exit 1) on
what the Rust side does not hold, rather than emit a table it would read
differently from Mesa: a condition string this script does not know; a
search with more than 8 commutative expressions (nir_search.c tries
1 << min(comm_exprs, 8) source orders) or a variable index of 32 or more
(blend_nir.rs `Match::vars`); more than 256 items (`State` is 256 bits);
a replacement that adds fp_math_ctrl (`exact`, `preserve_*`). A search's
`~`, `nsz`, `nnan`, `ninf` and `contract` exclude instructions with those
fp_math_ctrl bits set; a blend shader's instructions have none
(nir_builder's default `nir_fp_fast_math`, nir_builder.c:55), so the
exclusions never apply and blend_nir.rs does not read them.
"""

import math
import os
import sys

# The opcodes of a blend shader's NIR (blend_nir.rs `Op`), as the automaton
# names them (conversions by their unsized search opcode).
SHADER_OPS = {
    'mov', 'vec2', 'vec3', 'vec4', 'fadd', 'fmul', 'ffma', 'fneg', 'fsat', 'fmin', 'fmax',
    'fsub', 'fcanonicalize', 'f2f', 'iand', 'u2u', 'pack_32_4x8_split',
}
# The opcodes a replacement may build (blend_nir.rs `Op`).
BUILT_OPS = {
    'mov', 'vec2', 'vec3', 'vec4', 'fadd', 'fmul', 'ffma', 'fneg', 'fsat', 'fmin', 'fmax',
    'fsub', 'fcanonicalize', 'f2f16', 'iand', 'u2u8', 'pack_32_4x8_split',
}
RUST_OP = {
    'mov': 'Mov', 'vec2': 'Vec2', 'vec3': 'Vec3', 'vec4': 'Vec4', 'fadd': 'Fadd', 'fmul': 'Fmul',
    'ffma': 'Ffma', 'fneg': 'Fneg', 'fsat': 'Fsat', 'fmin': 'Fmin', 'fmax': 'Fmax', 'fsub': 'Fsub',
    'fcanonicalize': 'Fcanonicalize', 'f2f16': 'F2f16', 'f2f': 'F2f', 'iand': 'Iand', 'u2u8': 'U2u8',
    'u2u': 'U2u', 'pack_32_4x8_split': 'Pack32_4x8Split',
}

# Rule conditions on the Mali-G610: bifrost_compile.h DEFINE_OPTIONS(arch 10)
# and a shader with float_controls_execution_mode 0.
CONDITIONS = {
    'true': True,
    '!options->lower_flrp16': False, '!options->lower_flrp32': False, '!options->lower_flrp64': False,
    'options->lower_flrp16': True, 'options->lower_flrp32': True, 'options->lower_flrp64': True,
    'options->lower_fsat': False, '!options->lower_fsat': True,
    'options->has_fsub': False, 'options->lower_fneg': False,
    'options->support_16bit_alu': False,
    'options->force_f2f16_rtz && !nir_is_rounding_mode_rtne(info->float_controls_execution_mode, 16)': False,
    '!nir_is_denorm_flush_to_zero(info->float_controls_execution_mode, 16)': True,
    '!nir_is_denorm_flush_to_zero(info->float_controls_execution_mode, 32)': True,
    '!nir_is_denorm_flush_to_zero(info->float_controls_execution_mode, 64)': True,
    'gpu_arch >= 11': False,
    'is_kraid': False,
    '(info->stage != MESA_SHADER_VERTEX && info->stage != MESA_SHADER_GEOMETRY) && !options->intel_vec4': True,
    '!options->lower_pack_64_2x32_split': True,
    '!options->lower_extract_byte': True,
    '!options->lower_extract_word': True,
    '!options->vectorize_vec2_16bit': False,
    'options->vectorize_vec2_16bit': True,
    'options->has_iadd3': False,
    'options->has_iadd3 && !(options->lower_int64_options & nir_lower_iadd3_64)': False,
    'options->has_fsub && !(options->lower_doubles_options & nir_lower_dsub)': False,
    '!options->has_fcanonicalize': True,
}
MULADD = ('(options->float_mul_add{0} & nir_float_muladd_support_fuse) && '
          '(!(options->float_mul_add{0} & nir_float_muladd_support_has_fmad) || '
          '!(options->float_mul_add{0} & nir_float_muladd_support_prefers_split)) && '
          '(options->float_mul_add{0} & nir_float_muladd_support_has_ffma)')
MULADD_FMAD = ('(options->float_mul_add{0} & nir_float_muladd_support_fuse) && '
               '(!(options->float_mul_add{0} & nir_float_muladd_support_has_ffma) ||  '
               '(options->float_mul_add{0} & nir_float_muladd_support_prefers_split)) && '
               '(options->float_mul_add{0} & nir_float_muladd_support_has_fmad)')
for _s in (16, 32, 64):
    # float_mul_add = has_ffma | fuse (bifrost_compile.h:128-133)
    CONDITIONS[MULADD.format(_s)] = True
    CONDITIONS[MULADD_FMAD.format(_s)] = False

EXPR_COND = {
    None: 'None', 'is_used_once': 'UsedOnce', 'is_only_used_by_fadd': 'OnlyUsedByFadd',
    'is_not_only_used_by_fadd': 'NotOnlyUsedByFadd', 'is_only_used_as_float': 'OnlyUsedAsFloat',
    'is_only_used_as_float_nsz': 'OnlyUsedAsFloatNsz', 'is_used_by_non_fsat': 'UsedByNonFsat',
}
VAR_COND = {
    None: 'None', '(is_not_const)': 'NotConst', '(is_not_fmul)': 'NotFmul', '(is_fmul)': 'Fmul',
    '(is_nan)': 'Nan', '(is_any_comp_nan)': 'AnyCompNan',
}
# nir_search_helpers.h RELATION (fp class) conditions: (name, excluded classes)
RELATIONS = ['lt_zero', 'not_positive', 'gt_zero', 'not_negative', 'not_negative_or_negative_zero',
             'not_zero', 'zero_to_one', 'le_pos_one', 'gt_0_and_lt_1', 'ge_pos_one']
for _r in RELATIONS:
    _c = ''.join(w.capitalize() for w in _r.split('_'))
    VAR_COND['(is_%s)' % _r] = 'Fp(FpRel::%s, false)' % _c
    VAR_COND['(is_a_number_%s)' % _r] = 'Fp(FpRel::%s, true)' % _c
VAR_COND['(is_a_number)'] = 'Fp(FpRel::Number, true)'
VAR_COND['(is_finite)'] = 'Fp(FpRel::Finite, false)'
VAR_COND['(is_finite_not_zero)'] = 'Fp(FpRel::FiniteNotZero, false)'
VAR_COND['(is_integral)'] = 'Fp(FpRel::Integral, false)'
VAR_COND['(is_integral_not_negative)'] = 'Fp(FpRel::IntegralNotNegative, false)'


class GenError(Exception):
    pass


def load(tree):
    nir = os.path.join(tree, 'src/compiler/nir')
    bif = os.path.join(tree, 'src/panfrost/compiler/bifrost')
    sys.path.insert(0, nir)
    argv = sys.argv
    sys.argv = ['nir_opt_algebraic.py', '--out', os.devnull]
    src = open(os.path.join(nir, 'nir_opt_algebraic.py')).read()
    src = src[:src.index('parser = argparse.ArgumentParser()')]
    ns = {'__name__': 'nir_opt_algebraic'}
    exec(compile(src, 'nir_opt_algebraic.py', 'exec'), ns)
    bsrc = open(os.path.join(bif, 'bifrost_nir_algebraic.py')).read()
    bsrc = bsrc[:bsrc.index('def main')]
    bns = {'__name__': 'bifrost_nir_algebraic'}
    exec(compile(bsrc, 'bifrost_nir_algebraic.py', 'exec'), bns)
    sys.argv = argv
    import nir_algebraic
    return nir_algebraic, [('OPT', 'nir_opt_algebraic', ns['optimizations']),
                           ('LATE', 'nir_opt_algebraic_late', ns['late_optimizations']),
                           ('BIFROST', 'bifrost_nir_lower_algebraic_late', bns['algebraic_late'])]


class Pass:
    """nir_algebraic.AlgebraicPass's parse, without rendering C."""

    def __init__(self, na, rules):
        self.expression_cond = {}
        self.variable_cond = {}
        self.xforms = [na.SearchAndReplace(x, self) for x in rules]
        ta = na.TreeAutomaton.__new__(na.TreeAutomaton)
        ta.patterns = [t.search for t in self.xforms]
        ta._compute_items()
        self.automaton = ta


class Refuse(Exception):
    pass


def gen_pass(na, out, name, pass_name, rules):
    p = Pass(na, rules)
    a = p.automaton
    items = list(dict.fromkeys(a.items.values()))
    rel = [a.wildcard, a.const]
    changed = True
    while changed:
        changed = False
        for it in items:
            if it in rel or it.opcode not in SHADER_OPS:
                continue
            if all(c in rel for c in it.children):
                rel.append(it)
                changed = True
    if len(rel) > 256:
        raise GenError('%s: %d items, more than the 256 a State holds' % (pass_name, len(rel)))
    ids = {it: k for k, it in enumerate(rel)}
    keys = [(op, ch, it) for (op, ch), it in a.items.items() if it in ids and op in SHADER_OPS]
    pats = sorted(set(pt for it in rel for pt in it.patterns))
    rules_out = []
    for pt in pats:
        x = p.xforms[pt]
        if x.condition not in CONDITIONS:
            raise GenError('%s: unknown condition %r' % (pass_name, x.condition))
        if CONDITIONS[x.condition]:
            rules_out.append((pt, x))
    rule_index = {pt: k for k, (pt, _) in enumerate(rules_out)}
    nodes = []

    def node(v, search):
        if isinstance(v, na.Expression):
            if search and not 0 <= v.comm_expr_idx < 8 and v.comm_expr_idx != -1:
                raise GenError('%s: a commutative expression index %d' % (pass_name, v.comm_expr_idx))
            if not search and v.fp_math_ctrl_add() != 'nir_fp_fast_math':
                raise GenError('%s: a replacement adds %s' % (pass_name, v.fp_math_ctrl_add()))
            srcs = [node(s, search) for s in v.sources]
            op = v.opcode
            if op not in RUST_OP:
                if not search:
                    raise Refuse()
                # a search opcode a blend shader never holds (a sized
                # conversion of another size): the pattern never matches
                op = None
            cond = EXPR_COND.get(v.cond, 'Unknown')
            src1c = 'true' if len(v.sources) > 1 and isinstance(v.sources[1], na.Constant) else 'false'
            nodes.append('Node::Expr { op: Op::%s, bits: %d, swizzle: %d, comm: %d, src1_const: %s, cond: Cond::%s, '
                         'n: %d, srcs: [%s] }' % (RUST_OP[op] if op else 'Other', v.c_bit_size, v.swizzle, v.comm_expr_idx, src1c, cond,
                                                  len(srcs), ', '.join(str(s) for s in srcs + [0] * (4 - len(srcs)))))
        elif isinstance(v, na.Variable):
            if v.index >= 32:
                raise GenError('%s: a variable index %d' % (pass_name, v.index))
            cond = VAR_COND.get(v.cond, 'Unknown')
            swz = [na.swizzles[c] for c in v.swiz[1:]] if v.swiz else []
            nodes.append('Node::Var { var: %d, is_const: %s, cond: VarCond::%s, bits: %d, swizzle: [%s], swizzled: %s }' % (
                v.index, 'true' if v.is_constant else 'false', cond, v.c_bit_size,
                ', '.join(str(c) for c in (swz + [0, 1, 2, 3][len(swz):])), 'true' if swz else 'false'))
        elif isinstance(v, na.Constant):
            val = v.value
            if isinstance(val, bool):
                nodes.append('Node::Int { bits: %d, value: %d }' % (v.c_bit_size, int(val)))
            elif isinstance(val, float):
                if math.isnan(val):
                    lit = 'f64::NAN'
                elif math.isinf(val):
                    lit = 'f64::INFINITY' if val > 0 else 'f64::NEG_INFINITY'
                else:
                    lit = repr(val)
                    if math.copysign(1.0, val) < 0 and val == 0.0:
                        lit = '-0.0'
                nodes.append('Node::Float { bits: %d, value: %s }' % (v.c_bit_size, lit))
            else:
                nodes.append('Node::Int { bits: %d, value: %d }' % (v.c_bit_size, val))
        else:
            raise GenError('unknown value %r' % v)
        return len(nodes) - 1

    rule_lines = []
    for pt, x in rules_out:
        if x.search.comm_exprs > 8:
            raise GenError('%s: %s has %d commutative expressions' % (pass_name, x.search, x.search.comm_exprs))
        s = node(x.search, True)
        refuse = 'false'
        try:
            r = node(x.replace, False)
        except Refuse:
            r = 0
            refuse = 'true'
        rule_lines.append('    /* %d: %s -> %s */\n    Rule { search: %d, replace: %d, comm_exprs: %d, refuse: %s },' % (
            pt, str(x.search).replace('*/', '* /'), str(x.replace).replace('*/', '* /'), s, r, x.search.comm_exprs,
            refuse))
    out.append('/// %s: %d automaton items of the blend shader\'s opcodes, %d transforms.' % (
        pass_name, len(rel), len(rules_out)))
    out.append('pub(super) static %s: Pass = Pass {' % name)
    out.append('    n_items: %d,' % len(rel))
    out.append('    items: &[')
    for op, ch, it in keys:
        if op in ('__wildcard', '__const'):
            continue
        out.append('        Item { op: Op::%s, n: %d, children: [%s], id: %d },' % (
            RUST_OP[op], len(ch), ', '.join(str(ids[c]) for c in list(ch) + [rel[0]] * (4 - len(ch))), ids[it]))
    out.append('    ],')
    out.append('    item_rules: &[')
    for it in rel:
        rs = sorted(rule_index[pt] for pt in it.patterns if pt in rule_index)
        out.append('        &[%s],' % ', '.join(str(r) for r in rs))
    out.append('    ],')
    out.append('    rules: &[')
    out.extend(rule_lines)
    out.append('    ],')
    out.append('    nodes: &[')
    out.extend('        %s,' % n for n in nodes)
    out.append('    ],')
    out.append('};')
    out.append('')


HEADER = '''// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014, 2016 Intel Corporation
// SPDX-FileCopyrightText: 2021 Collabora, Ltd.
//
// blend_nir_rules.rs - GENERATED by tools/mali/gen_blend_nir_rules.py from
// Mesa 26.2.1's src/compiler/nir/nir_opt_algebraic.py and
// src/panfrost/compiler/bifrost/bifrost_nir_algebraic.py (MIT; LICENSES/MIT.txt),
// read with Mesa's nir_algebraic.py. DO NOT EDIT: regenerate it.
//
//! The algebraic transforms a blend shader can meet (blend_nir.rs `algebraic`):
//! per pass the automaton items built of the blend shader's opcodes (item 0
//! the wildcard, 1 a constant), the transforms whose root is one of them in
//! the order Mesa tries them, and their search and replace trees.
// The layout is the generator's: regenerated, not formatted.
#![cfg_attr(rustfmt, rustfmt_skip)]

use super::blend_nir::{Cond, FpRel, Item, Node, Op, Pass, Rule, VarCond};
'''


def main():
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        na, passes = load(sys.argv[1])
        out = [HEADER]
        for name, pass_name, rules in passes:
            gen_pass(na, out, name, pass_name, rules)
    except GenError as e:
        print('gen_blend_nir_rules: %s' % e, file=sys.stderr)
        return 1
    sys.stdout.write('\n'.join(out).rstrip('\n') + '\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
