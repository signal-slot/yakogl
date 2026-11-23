#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Generate the shader NIR's tables from Mesa's own Python files, in Rust.

    tools/mali/gen_nir_tables.py <mesa-26.2.1 tree> --out <dir>

The shader NIR (src/shaderc/online/mali/snir/, docs/notes/mali-shader-nir.md
section 3.3) runs Mesa's NIR passes. This reads, from the Mesa tree given and
unmodified, nir_opcodes.py, nir_intrinsics.py, nir_algebraic.py,
nir_opt_algebraic.py and bifrost_nir_algebraic.py (imported, so Mesa's own
parser reads them), and the C headers that hold the compiler options of the
Mali-G610 and the enums those options and the rule conditions name. It
writes into <dir>:

  - opcodes.rs: `NirOp`, every opcode of nir_opcodes.py in Mesa's order, its
    info table (nir_opcodes_c.py's nir_op_info), the unsized conversions of
    nir_search, and `REACHABLE`: tools/mali/snir_ops.txt closed under the
    replacement of every algebraic transform that can fire on the G610;
  - intrinsics.rs: `NirIndex` (every const index) and `NirIntrinsic`, the
    intrinsics of INTRINSICS below, with nir_intrinsic_info's fields.

Rule conditions are evaluated, not looked up: `Cond` evaluates the C
expression of each condition against bifrost_compile.h's options for arch 10
and a GL shader's float controls (execution mode 0), once per stage, so a
condition on `info->stage` gives a stage mask. The generation fails (exit 1,
no file written) on what it cannot vouch for: a condition, an option or an
enum value it cannot evaluate, an opcode or intrinsic name Mesa does not
have, two Mesa names that give one Rust name.

The later parts of the note's generator (the rule tables, the constant
folds) are more entries of OUTPUTS: a function from the loaded `Mesa` to the
text of one file.
"""

import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OPS_FILE = os.path.join(HERE, 'snir_ops.txt')

# The intrinsics the translation and the ported passes make (the note's
# sections 4.5, 5 and 6.2) and those of the blend shader (blend_nir.rs
# `Intr`). M2 to M4 add names.
INTRINSICS = [
    # the translation (section 4.5)
    'load_deref', 'store_deref', 'load_input', 'load_interpolated_input',
    'load_barycentric_pixel', 'load_ubo', 'store_output',
    # vertex passes: nir_lower_viewport_transform, pan_nir_lower_vs_outputs,
    # the IDVS variants (sections 5.3, 5.5) and the vertex selection (6.2)
    'load_viewport_scale', 'load_viewport_offset', 'load_attribute_pan',
    'load_idvs_output_buf_index_pan', 'lea_buf_pan', 'store_global',
    'store_global_psiz_pan', 'load_shader_output_pan',
    # fragment passes: pan_nir_lower_fs_inputs, pan_nir_lower_fs_outputs
    'load_var_pan', 'load_var_buf_pan', 'load_cumulative_coverage_pan',
    'atest_pan', 'load_blend_descriptor_pan', 'blend_pan',
    # the blend shader (pan_blend.c)
    'load_blend_input_pan', 'load_sample_id', 'load_tile_pan', 'blend_return_pan',
]

# The Mali-G610 is arch 10; pan_compiler.c:94-97 gives arch 9 and 10 the
# table bifrost_nir_options_v9 (bifrost_compile.h DEFINE_OPTIONS).
GPU_ARCH = 10
OPTIONS_TABLE = 'bifrost_nir_options_v9'
# bifrost_nir_lower_algebraic_late's is_kraid is pan_use_kraid(), false in a
# build without WITH_PANFROST_RUST (pan_compiler.c:54-66).
IS_KRAID = 0
# A GL shader has no float controls: shader_info's
# float_controls_execution_mode is 0.
FLOAT_CONTROLS_MODE = 0
# The stages a GLSL ES 2 program has, and their bit in a stage mask.
STAGES = (('MESA_SHADER_VERTEX', 1), ('MESA_SHADER_FRAGMENT', 2))
ALL_STAGES = 3

# The C headers whose enums the options and the conditions name.
ENUM_HEADERS = [
    'src/compiler/shader_enums.h',
    'src/compiler/nir/nir_defines.h',
    'src/compiler/nir/nir_shader_compiler_options.h',
    'src/compiler/nir/nir.h',
]


class GenError(Exception):
    pass


# ---------------------------------------------------------------------------
# A small evaluator of C constant expressions: the operators of the rule
# conditions and of the option initialisers, integers, names from an
# environment and calls of the functions given.

_TOKEN = re.compile(r'\s*(?:(0[xX][0-9a-fA-F]+|\d+)[uUlL]*|([A-Za-z_]\w*)|'
                    r'(->|&&|\|\||==|!=|>=|<=|<<|>>|[!~&|^<>()?:,+\-*/%]))')
# binary operators by precedence, lowest first (C's order)
_BINARY = [
    ('||',), ('&&',), ('|',), ('^',), ('&',), ('==', '!='), ('<', '<=', '>', '>='),
    ('<<', '>>'), ('+', '-'), ('*', '/', '%'),
]


def _tokens(text):
    out = []
    pos = 0
    text = text.rstrip()
    while pos < len(text):
        m = _TOKEN.match(text, pos)
        if not m or m.end() == pos:
            raise GenError('cannot read %r at %r' % (text, text[pos:]))
        num, name, op = m.groups()
        if num is not None:
            out.append(('num', int(num, 0)))
        elif name is not None:
            out.append(('name', name))
        else:
            out.append(('op', op))
        pos = m.end()
    return out


class _Parser:
    """Recursive descent over the tokens; each operand is evaluated (all of
    them, with no short-circuit, so that every name is checked)."""

    def __init__(self, text, lookup, funcs):
        self.text = text
        self.toks = _tokens(text)
        self.k = 0
        self.lookup = lookup
        self.funcs = funcs

    def peek(self):
        return self.toks[self.k] if self.k < len(self.toks) else (None, None)

    def take(self, op):
        if self.peek() != ('op', op):
            raise GenError('%r: expected %r' % (self.text, op))
        self.k += 1

    def parse(self):
        v = self.ternary()
        if self.k != len(self.toks):
            raise GenError('%r: unread %r' % (self.text, self.toks[self.k:]))
        return v

    def ternary(self):
        c = self.binary(0)
        if self.peek() == ('op', '?'):
            self.k += 1
            a = self.ternary()
            self.take(':')
            b = self.ternary()
            return a if c else b
        return c

    def binary(self, level):
        if level == len(_BINARY):
            return self.unary()
        v = self.binary(level + 1)
        while self.peek()[0] == 'op' and self.peek()[1] in _BINARY[level]:
            op = self.peek()[1]
            self.k += 1
            w = self.binary(level + 1)
            v = _apply(op, v, w)
        return v

    def unary(self):
        kind, val = self.peek()
        if kind == 'op' and val in ('!', '~', '-', '+'):
            self.k += 1
            v = self.unary()
            return {'!': lambda x: int(not x), '~': lambda x: ~x, '-': lambda x: -x, '+': lambda x: x}[val](v)
        return self.primary()

    def primary(self):
        kind, val = self.peek()
        self.k += 1
        if kind == 'num':
            return val
        if kind == 'op' and val == '(':
            v = self.ternary()
            self.take(')')
            return v
        if kind != 'name':
            raise GenError('%r: unexpected %r' % (self.text, val))
        if self.peek() == ('op', '('):
            self.k += 1
            args = []
            if self.peek() != ('op', ')'):
                args.append(self.ternary())
                while self.peek() == ('op', ','):
                    self.k += 1
                    args.append(self.ternary())
            self.take(')')
            if val not in self.funcs:
                raise GenError('%r: unknown function %s' % (self.text, val))
            return int(self.funcs[val](*args))
        while self.peek() == ('op', '->'):
            self.k += 1
            kind2, member = self.peek()
            if kind2 != 'name':
                raise GenError('%r: a member name after ->' % self.text)
            self.k += 1
            val = val + '->' + member
        return int(self.lookup(val))


def _apply(op, a, b):
    if op == '||':
        return int(bool(a) or bool(b))
    if op == '&&':
        return int(bool(a) and bool(b))
    return {
        '|': lambda: a | b, '^': lambda: a ^ b, '&': lambda: a & b,
        '==': lambda: int(a == b), '!=': lambda: int(a != b),
        '<': lambda: int(a < b), '<=': lambda: int(a <= b),
        '>': lambda: int(a > b), '>=': lambda: int(a >= b),
        '<<': lambda: a << b, '>>': lambda: a >> b,
        '+': lambda: a + b, '-': lambda: a - b, '*': lambda: a * b,
        '/': lambda: int(a / b), '%': lambda: a % b,
    }[op]()


def c_eval(text, lookup, funcs=None):
    """The value of the C expression `text`; `lookup(name)` gives a name's
    value (a member chain such as `options->x` is one name) or raises."""
    return _Parser(text, lookup, funcs or {}).parse()


# util/macros.h
C_MACROS = {
    'BITFIELD_BIT': lambda b: 1 << b,
    'BITFIELD64_BIT': lambda b: 1 << b,
    'BITFIELD_MASK': lambda b: (1 << b) - 1,
}


def _read(*path):
    with open(os.path.join(*path)) as f:
        return f.read()


def _strip_comments(text):
    text = re.sub(r'/\*.*?\*/', ' ', text, flags=re.S)
    return re.sub(r'//[^\n]*', '', text)


def _split_top(text, sep=','):
    """`text` split at `sep` outside parentheses and braces."""
    out, depth, cur = [], 0, []
    for ch in text:
        if ch in '({':
            depth += 1
        elif ch in ')}':
            depth -= 1
        if ch == sep and depth == 0:
            out.append(''.join(cur))
            cur = []
        else:
            cur.append(ch)
    out.append(''.join(cur))
    return [s.strip() for s in out if s.strip()]


_UNKNOWN = object()


def c_enums(tree, headers):
    """Every enum constant and integer #define of the headers, by name. An enum with a
    preprocessor line inside, or a value the evaluator cannot read, gives
    unknown constants: naming one later stops the generation."""
    values = {}

    def lookup(name):
        v = values.get(name)
        if v is None or v is _UNKNOWN:
            raise GenError('enum value %s unknown' % name)
        return v

    for h in headers:
        text = _strip_comments(_read(tree, h))
        # an integer #define is a constant too (NIR_FP_MATH_CONTROL_BIT_COUNT)
        for m in re.finditer(r'^#define ([A-Za-z_]\w*)[ \t]+(0[xX][0-9a-fA-F]+|\d+)[uU]?[ \t]*$', text, re.M):
            values.setdefault(m.group(1), int(m.group(2), 0))
        for m in re.finditer(r'\benum\s*\w*\s*\{([^{}]*)\}', text):
            body = m.group(1)
            unknown = re.search(r'^\s*#', body, re.M) is not None
            nxt = 0
            for entry in _split_top(body):
                em = re.match(r'([A-Za-z_]\w*)\s*(?:=\s*(.*))?$', entry, re.S)
                if not em:
                    unknown = True
                    continue
                name, expr = em.group(1), em.group(2)
                val = _UNKNOWN
                if not unknown:
                    try:
                        val = c_eval(expr, lookup, C_MACROS) if expr is not None else nxt
                    except GenError:
                        val = _UNKNOWN
                values.setdefault(name, val)
                nxt = val + 1 if val is not _UNKNOWN else _UNKNOWN
                if nxt is _UNKNOWN:
                    unknown = True
    return values


def c_struct_fields(tree, header, struct):
    """The member names of `typedef struct <struct> { ... } <struct>;`."""
    text = _strip_comments(_read(tree, header))
    m = re.search(r'typedef struct %s \{(.*?)\n\} %s;' % (struct, struct), text, re.S)
    if not m:
        raise GenError('%s: no struct %s' % (header, struct))
    fields = set()
    for decl in m.group(1).split(';'):
        decl = ' '.join(decl.split())
        if not decl:
            continue
        fp = re.search(r'\(\s*\*\s*(\w+)\s*\)', decl)
        fm = fp or re.search(r'(\w+)\s*(?:\[[^\]]*\])?\s*(?::\s*\d+)?$', decl)
        if not fm:
            raise GenError('%s: cannot read the member %r' % (header, decl))
        fields.add(fm.group(1))
    return fields


def g610_options(tree, enums, fields):
    """bifrost_compile.h's DEFINE_OPTIONS(OPTIONS_TABLE, ...) as a dict:
    each member it sets, evaluated; a member it does not set is 0."""
    h = 'src/panfrost/compiler/bifrost/bifrost_compile.h'
    text = _read(tree, h).replace('\\\n', ' ')
    text = _strip_comments(text)
    dm = re.search(r'#define DEFINE_OPTIONS\(([^)]*)\)(.*)', text)
    im = re.search(r'\bDEFINE_OPTIONS\(%s,([^)]*)\);' % OPTIONS_TABLE, text)
    if not dm or not im:
        raise GenError('%s: no DEFINE_OPTIONS or no %s' % (h, OPTIONS_TABLE))
    params = [p.strip() for p in dm.group(1).split(',')][1:]
    body = dm.group(2)
    body = body[body.index('{') + 1:body.rindex('}')]

    def lookup(name):
        v = enums.get(name, _UNKNOWN)
        if v is _UNKNOWN:
            raise GenError('%s: %s unknown' % (h, name))
        return v

    env = {'true': 1, 'false': 0}
    args = [a.strip() for a in im.group(1).split(',')]
    if len(args) != len(params):
        raise GenError('%s: DEFINE_OPTIONS(%s) has %d arguments' % (h, OPTIONS_TABLE, len(args)))
    for p, a in zip(params, args):
        env[p] = c_eval(a, lambda n: env[n] if n in env else lookup(n), C_MACROS)
    opts = {}
    for init in _split_top(body):
        om = re.match(r'\.(\w+)\s*=\s*(.*)$', init, re.S)
        if not om:
            raise GenError('%s: cannot read %r' % (h, init))
        if om.group(1) not in fields:
            raise GenError('%s: %s is no member of nir_shader_compiler_options' % (h, om.group(1)))
        opts[om.group(1)] = c_eval(om.group(2), lambda n: env[n] if n in env else lookup(n), C_MACROS)
    return opts


class Cond:
    """The rule conditions of the algebraic passes on the G610.

    `mask(text)` is the set of stages (a STAGES bit mask) on which the C
    condition `text` holds, as the generated nir_algebraic C would evaluate
    it: `options` is the G610's table, `info` a GL shader's, `gpu_arch` and
    `is_kraid` bifrost_nir_lower_algebraic_late's parameters."""

    def __init__(self, enums, fields, options):
        self.enums = enums
        self.fields = fields
        self.options = options
        self.cache = {}

    def _value(self, name, stage):
        if name.startswith('options->'):
            f = name[len('options->'):]
            if f not in self.fields:
                raise GenError('%s is no member of nir_shader_compiler_options' % f)
            return self.options.get(f, 0)
        fixed = {
            'true': 1, 'false': 0, 'gpu_arch': GPU_ARCH, 'is_kraid': IS_KRAID,
            'info->stage': stage, 'info->float_controls_execution_mode': FLOAT_CONTROLS_MODE,
        }
        if name in fixed:
            return fixed[name]
        v = self.enums.get(name, _UNKNOWN)
        if v is _UNKNOWN:
            raise GenError('%s unknown' % name)
        return v

    @staticmethod
    def _float_controls(mode, bit_size):
        # nir.h:1371-1400: each tests a FLOAT_CONTROLS_* bit of the mode for
        # the bit size, so it is false for mode 0.
        if mode != 0:
            raise GenError('a float controls mode other than 0')
        return 0

    FUNCS = {
        'nir_is_denorm_flush_to_zero': _float_controls.__func__,
        'nir_is_denorm_preserve': _float_controls.__func__,
        'nir_is_rounding_mode_rtne': _float_controls.__func__,
        'nir_is_rounding_mode_rtz': _float_controls.__func__,
    }

    def mask(self, text):
        if text not in self.cache:
            m = 0
            for name, bit in STAGES:
                stage = self._value(name, 0)
                try:
                    v = c_eval(text, lambda n, s=stage: self._value(n, s), self.FUNCS)
                except GenError as e:
                    raise GenError('condition %r: %s' % (text, e))
                if v:
                    m |= bit
            self.cache[text] = m
        return self.cache[text]


# ---------------------------------------------------------------------------
# Loading Mesa's Python.

class Pass:
    """nir_algebraic.AlgebraicPass's parse, without rendering C."""

    def __init__(self, na, rules):
        self.expression_cond = {}
        self.variable_cond = {}
        self.xforms = [na.SearchAndReplace(x, self) for x in rules]


class Mesa:
    """What the outputs are made from: Mesa's Python tables, the parsed
    algebraic passes, the condition evaluator and the reachable opcodes."""

    def __init__(self, tree):
        nir = os.path.join(tree, 'src/compiler/nir')
        bif = os.path.join(tree, 'src/panfrost/compiler/bifrost')
        sys.path.insert(0, nir)
        # the Mesa tree is read, never written (no __pycache__ in it)
        sys.dont_write_bytecode = True
        argv = sys.argv
        sys.argv = ['nir_opt_algebraic.py', '--out', os.devnull]
        src = _read(nir, 'nir_opt_algebraic.py')
        src = src[:src.index('parser = argparse.ArgumentParser()')]
        ns = {'__name__': 'nir_opt_algebraic'}
        exec(compile(src, 'nir_opt_algebraic.py', 'exec'), ns)
        bsrc = _read(bif, 'bifrost_nir_algebraic.py')
        bsrc = bsrc[:bsrc.index('def main')]
        bns = {'__name__': 'bifrost_nir_algebraic'}
        exec(compile(bsrc, 'bifrost_nir_algebraic.py', 'exec'), bns)
        sys.argv = argv
        import nir_algebraic
        import nir_intrinsics
        import nir_opcodes
        self.na = nir_algebraic
        self.opcodes = nir_opcodes.opcodes
        self.intrinsics = nir_intrinsics.INTR_OPCODES
        self.indices = nir_intrinsics.INTR_INDICES
        self.passes = [(name, Pass(nir_algebraic, rules)) for name, rules in (
            ('nir_opt_algebraic', ns['optimizations']),
            ('nir_opt_algebraic_late', ns['late_optimizations']),
            ('bifrost_nir_lower_algebraic_late', bns['algebraic_late']))]
        self.enums = c_enums(tree, ENUM_HEADERS)
        fields = c_struct_fields(tree, 'src/compiler/nir/nir_shader_compiler_options.h',
                                 'nir_shader_compiler_options')
        self.options = g610_options(tree, self.enums, fields)
        self.cond = Cond(self.enums, fields, self.options)
        self.max_const_index = self._define(tree, 'src/compiler/nir/nir.h', 'NIR_INTRINSIC_MAX_CONST_INDEX')
        self.alu_type_masks = [self._define(tree, 'src/compiler/nir/nir_defines.h', 'NIR_ALU_TYPE_%s_MASK' % m)
                               for m in ('SIZE', 'BASE_TYPE')]
        self.search_convs = self._search_convs(tree)
        self.reachable = self._reachable()

    @staticmethod
    def _define(tree, header, name):
        m = re.search(r'^#define %s\s+(0x[0-9a-fA-F]+|\d+)$' % name, _read(tree, header), re.M)
        if not m:
            raise GenError('%s: no #define %s' % (header, name))
        return int(m.group(1), 0)

    def _search_convs(self, tree):
        """nir_search.c nir_op_matches_search_op: the unsized conversions
        nir_search matches, each with its sized opcodes."""
        text = _read(tree, 'src/compiler/nir/nir_search.c')
        body = text[text.index('nir_op_matches_search_op(nir_op nop'):]
        body = body[:body.index('#undef')]
        sizes = {'F': (16, 32, 64), 'I': (8, 16, 32, 64)}
        convs = []
        for kind, op in re.findall(r'^\s*MATCH_([FI])CONV_CASE\((\w+)\)$', body, re.M):
            sized = ['%s%d' % (op, b) for b in sizes[kind]]
            for s in sized:
                if s not in self.opcodes:
                    raise GenError('nir_search.c: %s has no opcode %s' % (op, s))
            convs.append((op, sized))
        if [c for c, _ in convs] != [c for c in self.na.conv_opcode_types if c in dict(convs)]:
            raise GenError('nir_search.c and nir_algebraic.py name other conversions')
        return convs

    def _expr_ops(self, v):
        if isinstance(v, self.na.Expression):
            yield v
            for s in v.sources:
                yield from self._expr_ops(s)

    def _reachable(self):
        """snir_ops.txt closed under the replacements of every transform
        that can fire: its condition holds on a stage and every opcode of its
        search is reachable (an unsized conversion when one of its sized
        opcodes is). An unsized conversion in a replacement adds its sized
        opcode, or all of them when the bit size is not fixed."""
        names = []
        for line in _read(OPS_FILE).splitlines():
            line = line.split('#', 1)[0].strip()
            if line:
                if line not in self.opcodes:
                    raise GenError('snir_ops.txt: %s is no opcode of nir_opcodes.py' % line)
                names.append(line)
        reach = set(names)
        convs = dict(self.search_convs)

        def matchable(op):
            return any(s in reach for s in convs[op]) if op in convs else op in reach

        xforms = [x for _, p in self.passes for x in p.xforms]
        changed = True
        while changed:
            changed = False
            for x in xforms:
                if not self.cond.mask(x.condition):
                    continue
                if not all(matchable(e.opcode) for e in self._expr_ops(x.search)):
                    continue
                for e in self._expr_ops(x.replace):
                    if e.opcode in convs:
                        b = e.c_bit_size
                        new = ['%s%d' % (e.opcode, b)] if b > 0 else convs[e.opcode]
                    else:
                        new = [e.opcode]
                    for op in new:
                        if op not in self.opcodes:
                            raise GenError('a replacement builds %s, no opcode' % op)
                        if op not in reach:
                            reach.add(op)
                            changed = True
        return reach


# ---------------------------------------------------------------------------
# The outputs.

def rust_name(name):
    """A Mesa name in CamelCase: `pack_32_4x8_split` is `Pack32_4x8Split`
    (an underscore stays only between two digits)."""
    out = ''
    for part in name.split('_'):
        if out and out[-1].isdigit() and part[:1].isdigit():
            out += '_'
        out += part[:1].upper() + part[1:]
    return out


def rust_names(names, what):
    out = {}
    seen = {}
    for n in names:
        r = rust_name(n)
        if r in seen:
            raise GenError('%s %s and %s are both %s' % (what, seen[r], n, r))
        seen[r] = n
        out[n] = r
    return out


def header(out, sources, copyrights, doc):
    lines = ['// SPDX-License-Identifier: Apache-2.0 AND MIT',
             '// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.']
    lines += ['// SPDX-FileCopyrightText: %s' % c for c in copyrights]
    lines += ['//', '// %s - GENERATED by tools/mali/gen_nir_tables.py from Mesa 26.2.1\'s' % out]
    lines += ['// %s' % s for s in sources]
    lines += ['// (MIT; LICENSES/MIT.txt). DO NOT EDIT: regenerate it.', '//']
    lines += ['//! %s' % d if d else '//!' for d in doc]
    lines += ['// The layout is the generator\'s: regenerated, not formatted.',
              '#![cfg_attr(rustfmt, rustfmt_skip)]', '']
    return lines


def _type_value(mesa, t):
    v = mesa.enums.get('nir_type_' + t, _UNKNOWN)
    if v is _UNKNOWN:
        raise GenError('nir_type_%s unknown' % t)
    return v


def _flags(mesa, prefix, words):
    v = 0
    for w in words.split():
        f = mesa.enums.get(prefix + w, _UNKNOWN)
        if f is _UNKNOWN:
            raise GenError('%s%s unknown' % (prefix, w))
        v |= f
    return v


def gen_opcodes(mesa):
    names = sorted(mesa.opcodes)
    rn = rust_names(names, 'opcodes')
    out = header('opcodes.rs', [
        'src/compiler/nir/nir_opcodes.py, the constants of src/compiler/nir/nir.h,',
        'nir_defines.h and nir_search.c',
    ], ['2014 Connor Abbott', '2014 Intel Corporation'], [
        'The NIR opcodes (`nir_op`): every opcode of nir_opcodes.py in Mesa\'s',
        'order (sorted by name, nir_opcodes_h.py), its `nir_op_info`',
        '(nir_opcodes_c.py), the unsized conversions nir_search matches',
        '(nir_search.c), and `REACHABLE`, the opcodes snir may build.',
    ])
    out.append('/// `nir_op`, in Mesa\'s order (the discriminant is Mesa\'s value).')
    out.append('#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]')
    out.append('#[repr(u16)]')
    out.append('pub enum NirOp {')
    out.extend('    %s,' % rn[n] for n in names)
    out.append('}')
    out.append('')
    out.append('/// Every opcode, by its value.')
    out.append('pub static ALL: [NirOp; %d] = [' % len(names))
    out.extend('    NirOp::%s,' % rn[n] for n in names)
    out.append('];')
    out.append('')
    out.append('/// `nir_alu_type` (nir_defines.h): a base type ORed with a bit size.')
    out.append('pub mod alu_type {')
    for t in ('invalid', 'int', 'uint', 'bool', 'float'):
        out.append('    pub const %s: u8 = %d;' % (t.upper(), _type_value(mesa, t)))
    out.append('    /// The bit size of a sized type (0 for an unsized one).')
    out.append('    pub const SIZE_MASK: u8 = %d;' % mesa.alu_type_masks[0])
    out.append('    pub const BASE_TYPE_MASK: u8 = %d;' % mesa.alu_type_masks[1])
    out.append('}')
    out.append('')
    out.append('/// `NIR_OP_IS_*` (nir.h), the bits of `NirOpInfo::algebraic_properties`.')
    out.append('pub mod op_is {')
    for p in ('2src_commutative', 'associative', 'selection', 'inexact_associative'):
        out.append('    pub const %s: u8 = %d;' % (('_' if p[0].isdigit() else '') + p.upper(),
                                                   _flags(mesa, 'NIR_OP_IS_', p.upper())))
    out.append('}')
    out.append('')
    out.append('/// `nir_fp_*` (nir.h), the bits of an ALU instruction\'s `fp_math_ctrl`.')
    out.append('pub mod fp {')
    fps = [n[len('nir_fp_'):] for n in mesa.enums if n.startswith('nir_fp_')]
    for p in fps:
        out.append('    pub const %s: u8 = %d;' % (p.upper(), _flags(mesa, 'nir_fp_', p)))
    out.append('}')
    out.append('')
    out.append('/// `nir_op_info` (nir.h), as nir_opcodes_c.py fills it.')
    out.append('#[derive(Debug)]')
    out.append('pub struct NirOpInfo {')
    out.append('    pub name: &\'static str,')
    out.append('    pub num_inputs: u8,')
    out.append('    /// 0: per component; else the fixed component count of the output.')
    out.append('    pub output_size: u8,')
    out.append('    pub output_type: u8,')
    out.append('    pub input_sizes: &\'static [u8],')
    out.append('    pub input_types: &\'static [u8],')
    out.append('    pub is_conversion: bool,')
    out.append('    /// `op_is` bits.')
    out.append('    pub algebraic_properties: u8,')
    out.append('    /// `fp` bits the opcode honours.')
    out.append('    pub valid_fp_math_ctrl: u8,')
    out.append('    /// In `REACHABLE`.')
    out.append('    pub reachable: bool,')
    out.append('}')
    out.append('')
    out.append('pub static NIR_OP_INFOS: [NirOpInfo; %d] = [' % len(names))
    for n in names:
        o = mesa.opcodes[n]
        out.append('    NirOpInfo { name: "%s", num_inputs: %d, output_size: %d, output_type: %d, '
                   'input_sizes: &[%s], input_types: &[%s], is_conversion: %s, algebraic_properties: %d, '
                   'valid_fp_math_ctrl: %d, reachable: %s },' % (
                       n, o.num_inputs, o.output_size, _type_value(mesa, o.output_type),
                       ', '.join(str(s) for s in o.input_sizes),
                       ', '.join(str(_type_value(mesa, t)) for t in o.input_types),
                       'true' if o.is_conversion else 'false',
                       _flags(mesa, 'NIR_OP_IS_', o.algebraic_properties.upper()),
                       _flags(mesa, 'nir_fp_', o.valid_fp_math_ctrl),
                       'true' if n in mesa.reachable else 'false'))
    out.append('];')
    out.append('')
    reach = [n for n in names if n in mesa.reachable]
    out.append('/// tools/mali/snir_ops.txt closed under the replacements of the algebraic')
    out.append('/// transforms that can fire on the Mali-G610 (gen_nir_tables.py `_reachable`):')
    out.append('/// the opcodes snir builds; any other one is a refusal.')
    out.append('pub static REACHABLE: [NirOp; %d] = [' % len(reach))
    out.extend('    NirOp::%s,' % rn[n] for n in reach)
    out.append('];')
    out.append('')
    out.append('/// nir_search\'s unsized conversions (`nir_search_op`, nir_search.h): a')
    out.append('/// search for one matches each of its sized opcodes.')
    out.append('#[derive(Clone, Copy, PartialEq, Eq, Debug)]')
    out.append('pub enum SearchConv {')
    out.extend('    %s,' % rust_name(c) for c, _ in mesa.search_convs)
    out.append('}')
    out.append('')
    out.append('impl SearchConv {')
    out.append('    /// `nir_op_for_search_op`: the sized opcode, or None for a bit size it lacks.')
    out.append('    pub fn sized(self, bit_size: u8) -> Option<NirOp> {')
    out.append('        Some(match (self, bit_size) {')
    for c, sized in mesa.search_convs:
        for s in sized:
            out.append('            (SearchConv::%s, %s) => NirOp::%s,' % (rust_name(c), s[len(c):], rn[s]))
    out.append('            _ => return None,')
    out.append('        })')
    out.append('    }')
    out.append('}')
    out.append('')
    out.append('impl NirOp {')
    out.append('    pub const COUNT: usize = %d;' % len(names))
    out.append('')
    out.append('    pub fn info(self) -> &\'static NirOpInfo {')
    out.append('        &NIR_OP_INFOS[self as usize]')
    out.append('    }')
    out.append('')
    out.append('    /// Mesa\'s name (`nir_op_infos[op].name`).')
    out.append('    pub fn name(self) -> &\'static str {')
    out.append('        self.info().name')
    out.append('    }')
    out.append('')
    out.append('    pub fn is_reachable(self) -> bool {')
    out.append('        self.info().reachable')
    out.append('    }')
    out.append('')
    out.append('    /// The opcode of Mesa\'s value `v`.')
    out.append('    pub fn from_u16(v: u16) -> Option<NirOp> {')
    out.append('        ALL.get(v as usize).copied()')
    out.append('    }')
    out.append('')
    out.append('    /// The opcode named `name` (the table is sorted by name).')
    out.append('    pub fn from_name(name: &str) -> Option<NirOp> {')
    out.append('        let k = NIR_OP_INFOS.binary_search_by(|i| i.name.cmp(name)).ok()?;')
    out.append('        Self::from_u16(k as u16)')
    out.append('    }')
    out.append('')
    out.append('    /// `nir_search_op_for_nir_op`: the unsized conversion of a sized one.')
    out.append('    pub fn search_conv(self) -> Option<SearchConv> {')
    out.append('        Some(match self {')
    for c, sized in mesa.search_convs:
        out.append('            %s => SearchConv::%s,' % (' | '.join('NirOp::' + rn[s] for s in sized), rust_name(c)))
    out.append('            _ => return None,')
    out.append('        })')
    out.append('    }')
    out.append('}')
    return out


def gen_intrinsics(mesa):
    for n in INTRINSICS:
        if n not in mesa.intrinsics:
            raise GenError('INTRINSICS: %s is no intrinsic of nir_intrinsics.py' % n)
    names = sorted(INTRINSICS)
    rn = rust_names(names, 'intrinsics')
    idx_names = [i.name for i in mesa.indices]
    ri = rust_names(idx_names, 'indices')
    out = header('intrinsics.rs', [
        'src/compiler/nir/nir_intrinsics.py and the constants of src/compiler/nir/nir.h',
    ], ['2018 Red Hat', '2014 Intel Corporation'], [
        'The NIR intrinsics snir makes (gen_nir_tables.py `INTRINSICS`), with',
        '`nir_intrinsic_info`\'s fields (nir_intrinsics_c.py), and every const index',
        '(`nir_intrinsic_index_flag`). An intrinsic\'s const indices sit in the',
        'slots of its `indices`, in nir_intrinsics.py\'s order, as Mesa\'s',
        '`const_index` does, so the printer prints them in Mesa\'s order.',
    ])
    out.append('/// `NIR_INTRINSIC_MAX_CONST_INDEX` (nir.h): the const index slots of one')
    out.append('/// intrinsic instruction.')
    out.append('pub const MAX_CONST_INDEX: usize = %d;' % mesa.max_const_index)
    out.append('')
    out.append('/// `nir_intrinsic_semantic_flag` (nir.h), the bits of `NirIntrinsicInfo::flags`.')
    out.append('pub mod flags {')
    fl = [n for n in mesa.enums if n.startswith('NIR_INTRINSIC_') and n[len('NIR_INTRINSIC_'):] in
          ('CAN_ELIMINATE', 'CAN_REORDER', 'SUBGROUP', 'QUADGROUP')]
    for n in fl:
        out.append('    pub const %s: u8 = %d;' % (n[len('NIR_INTRINSIC_'):], _flags(mesa, '', n)))
    out.append('}')
    out.append('')
    out.append('/// `nir_intrinsic_index_flag`: every const index of nir_intrinsics.py, in')
    out.append('/// Mesa\'s order.')
    out.append('#[derive(Clone, Copy, PartialEq, Eq, Debug)]')
    out.append('pub enum NirIndex {')
    out.extend('    %s,' % ri[n] for n in idx_names)
    out.append('}')
    out.append('')
    out.append('/// An index\'s name (`nir_intrinsic_index_names`), its C type and its slots.')
    out.append('#[derive(Debug)]')
    out.append('pub struct NirIndexInfo {')
    out.append('    pub name: &\'static str,')
    out.append('    pub c_type: &\'static str,')
    out.append('    pub slots: u8,')
    out.append('}')
    out.append('')
    out.append('pub static NIR_INDEX_INFOS: [NirIndexInfo; %d] = [' % len(idx_names))
    for i in mesa.indices:
        out.append('    NirIndexInfo { name: "%s", c_type: "%s", slots: %d },' % (i.name, i.c_data_type, i.size))
    out.append('];')
    out.append('')
    out.append('impl NirIndex {')
    out.append('    pub fn info(self) -> &\'static NirIndexInfo {')
    out.append('        &NIR_INDEX_INFOS[self as usize]')
    out.append('    }')
    out.append('}')
    out.append('')
    out.append('/// The intrinsics of `INTRINSICS`, sorted by name as Mesa\'s `nir_intrinsic_op`.')
    out.append('#[derive(Clone, Copy, PartialEq, Eq, Debug)]')
    out.append('pub enum NirIntrinsic {')
    out.extend('    %s,' % rn[n] for n in names)
    out.append('}')
    out.append('')
    out.append('/// `nir_intrinsic_info` (nir.h). `src_components` and `dest_components`')
    out.append('/// 0 mean the instruction\'s `num_components`, a source\'s -1 any count;')
    out.append('/// `bit_size_src` -1 means `dest_bit_sizes` (a mask of bit sizes, 0 for')
    out.append('/// any) decides.')
    out.append('#[derive(Debug)]')
    out.append('pub struct NirIntrinsicInfo {')
    out.append('    pub name: &\'static str,')
    out.append('    pub src_components: &\'static [i8],')
    out.append('    pub has_dest: bool,')
    out.append('    pub dest_components: u8,')
    out.append('    pub dest_bit_sizes: u8,')
    out.append('    pub bit_size_src: i8,')
    out.append('    /// The const indices with their first slot, in nir_intrinsics.py\'s order.')
    out.append('    pub indices: &\'static [(NirIndex, u8)],')
    out.append('    pub num_index_slots: u8,')
    out.append('    /// `flags` bits.')
    out.append('    pub flags: u8,')
    out.append('    pub sysval: bool,')
    out.append('}')
    out.append('')
    out.append('pub static NIR_INTRINSIC_INFOS: [NirIntrinsicInfo; %d] = [' % len(names))
    for n in names:
        o = mesa.intrinsics[n]
        mask = 0
        for b in o.bit_sizes:
            mask |= b
        if mask > 255:
            raise GenError('%s: bit sizes %r' % (n, o.bit_sizes))
        flags = 0
        for f in o.flags:
            for w in f.split('|'):
                flags |= _flags(mesa, '', w.strip())
        idx = ', '.join('(NirIndex::%s, %d)' % (ri[i.name], slot - 1) for i, slot in zip(o.indices, o.index_map))
        out.append('    NirIntrinsicInfo { name: "%s", src_components: &[%s], has_dest: %s, dest_components: %d, '
                   'dest_bit_sizes: %d, bit_size_src: %d, indices: &[%s], num_index_slots: %d, flags: %d, '
                   'sysval: %s },' % (
                       n, ', '.join(str(c) for c in o.src_components), 'true' if o.has_dest else 'false',
                       max(o.dest_components, 0), mask, o.bit_size_src, idx, o.num_index_slots, flags,
                       'true' if o.sysval else 'false'))
    out.append('];')
    out.append('')
    out.append('impl NirIntrinsic {')
    out.append('    pub fn info(self) -> &\'static NirIntrinsicInfo {')
    out.append('        &NIR_INTRINSIC_INFOS[self as usize]')
    out.append('    }')
    out.append('')
    out.append('    /// Mesa\'s name (`nir_intrinsic_infos[op].name`).')
    out.append('    pub fn name(self) -> &\'static str {')
    out.append('        self.info().name')
    out.append('    }')
    out.append('')
    out.append('    /// The slot of the const index `idx`, if the intrinsic has it')
    out.append('    /// (`nir_intrinsic_info::index_map` less one).')
    out.append('    pub fn slot(self, idx: NirIndex) -> Option<usize> {')
    out.append('        self.info().indices.iter().find(|&&(i, _)| i == idx).map(|&(_, s)| s as usize)')
    out.append('    }')
    out.append('}')
    return out


# One entry per generated file: its name in snir/ and the function that
# makes its lines.
OUTPUTS = [
    ('opcodes.rs', gen_opcodes),
    ('intrinsics.rs', gen_intrinsics),
]


def summary(mesa):
    """One line per pass: transforms, distinct condition strings and how
    many of them the evaluator decides (all, or the generation failed)."""
    lines = ['gen_nir_tables: %d opcodes, %d reachable from tools/mali/snir_ops.txt' % (
        len(mesa.opcodes), len(mesa.reachable))]
    for name, p in mesa.passes:
        conds = sorted(set(x.condition for x in p.xforms))
        on = sum(1 for x in p.xforms if mesa.cond.mask(x.condition))
        lines.append('gen_nir_tables: %s: %d transforms, %d condition strings decided, %d transforms enabled' % (
            name, len(p.xforms), len(conds), on))
    return lines


def main(argv):
    if len(argv) != 4 or argv[2] != '--out':
        print(__doc__, file=sys.stderr)
        return 2
    try:
        mesa = Mesa(argv[1])
        files = [(name, '\n'.join(fn(mesa)).rstrip('\n') + '\n') for name, fn in OUTPUTS]
        lines = summary(mesa)
    except GenError as e:
        print('gen_nir_tables: %s' % e, file=sys.stderr)
        return 1
    os.makedirs(argv[3], exist_ok=True)
    for name, text in files:
        with open(os.path.join(argv[3], name), 'w') as f:
            f.write(text)
    print('\n'.join(lines))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
