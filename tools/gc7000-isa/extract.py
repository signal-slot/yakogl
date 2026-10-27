#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
#
# Transcribe the Vivante GC7000 instruction encodings into YakoGL's own table.
#
# The encodings are read from Mesa's etnaviv isaspec, `src/etnaviv/isa/etnaviv.xml`,
# an XML description of the ISA. That tree is NOT vendored here: it is imported
# from wherever it sits on the build host, named by --mesa or by $MESA_TREE, and
# only the encoding facts are written out, in our own schema.
#
#   etnaviv.xml is Copyright (c) 2023 Igalia S.L., MIT licensed.
#
# Output: docs/online-shaderc/isa/gc7000-isa.json, explained by
# docs/online-shaderc/gc7000-isa.md. Regenerate with:
#
#   tools/gc7000-isa/extract.py -o docs/online-shaderc/isa/gc7000-isa.json --stats
#
# `--check` re-generates and diffs against the committed file instead of writing
# it, so a stale table fails loudly rather than drifting.
#
# The shape mirrors tools/isa-extract/ (the PowerVR side) on purpose: one
# generic table walker on the consumer's side, not a hundred hand-written arms.
# The two ISAs are nothing alike -- Rogue has variable-length instruction groups
# and phases, the GC7000 has fixed 128-bit instructions -- but the job the table
# does for a decoder and an encoder is the same.

import argparse
import hashlib
import json
import os
import subprocess
import sys
import xml.etree.ElementTree as ET

DEFAULT_MESA = os.path.expanduser('~/pvr-lab/mesa-build/mesa-26.2.1')
ISA_SUBPATH = 'src/etnaviv/isa/etnaviv.xml'

SCHEMA_VERSION = 3   # 2 added the display templates and overrides; 3 the named template fragments


def bit_range(el):
    """(low, high) of a <field>/<pattern>, which use either pos= or low=/high=."""
    if el.get('pos') is not None:
        p = int(el.get('pos'))
        return p, p
    return int(el.get('low')), int(el.get('high'))


def parse_templates(root):
    """The named display fragments the instruction templates expand.

    An instruction's `display` is `{INSTR_ALU} {DST:align=18}, {SRC0}, ...`,
    and `INSTR_ALU` is one of six `<template>` elements holding the modifier
    order -- `{NAME}{DST_FULL}{SAT}{SKPHP}{UNK}{TYPE}{PMODE}{THREAD}{RMODE}`.
    Without them a consumer has the sentence but not the words: it can see
    that a suffix goes after the name and not which suffixes, nor in what
    order. 2,967 of the 13,854 instructions we ship carry at least one.
    """
    return {t.get('name'): (t.text or '').strip() for t in root.findall('template')}


def parse_enums(root):
    out = {}
    for en in root.findall('enum'):
        name = en.get('name')
        values = []
        for v in en.findall('value'):
            values.append({
                'value': int(v.get('val')),
                # `name` is the symbolic form when it differs from the display
                # text; isaspec often carries only one of the two.
                'name': v.get('name'),
                'display': v.get('display'),
            })
        out[name] = {'values': values}
    return out


def parse_pattern(el):
    """A <pattern> is a bit string, MSB first, where 'x' is don't care.

    Returned as value+mask so a decoder can match with two ANDs and never has
    to care which bits were specified.
    """
    low, high = bit_range(el)
    text = (el.text or '').strip()
    width = high - low + 1
    if len(text) != width:
        raise ValueError(f'pattern {text!r} is {len(text)} bits, range is {width}')
    value = 0
    mask = 0
    for ch in text:            # MSB first
        value <<= 1
        mask <<= 1
        if ch == '1':
            value |= 1
            mask |= 1
        elif ch == '0':
            mask |= 1
        elif ch != 'x':
            raise ValueError(f'unknown pattern character {ch!r}')
    return {'low': low, 'high': high, 'value': value, 'mask': mask}


def parse_bitsets(root):
    out = {}
    for bs in root.findall('bitset'):
        name = bs.get('name')
        fields = {}
        for f in bs.findall('field'):
            low, high = bit_range(f)
            fields[f.get('name')] = {
                'low': low, 'high': high,
                'type': f.get('type'),
                'display': f.get('display'),
                # A field whose type is itself a bitset (DST is an
                # #instruction-dst, SRCn an #instruction-src) is decoded with
                # values taken from elsewhere in the instruction -- DST needs
                # DST_USE and COMPS. Those are the <param> children, and
                # dropping them leaves a consumer unable to expand the nested
                # operand bitsets at all.
                'params': [pr.get('name') for pr in f.findall('param')],
            }
        derived = {}
        for d in bs.findall('derived'):
            expr = d.find('expr')
            derived[d.get('name')] = {
                'type': d.get('type'),
                # The expression is kept verbatim: it is the authority on how
                # the pieces combine, and paraphrasing it would be a second
                # source of truth.
                'expr': (expr.text or '').strip() if expr is not None else None,
            }
        patterns = [parse_pattern(p) for p in bs.findall('pattern')]
        # How the bitset PRINTS. isaspec carries the disassembly format as
        # data too: a `display` template naming the fields in order, and
        # `override` blocks that replace it when a condition holds -- an
        # unused destination prints `void`, a source in the immediate register
        # file prints its 20-bit literal instead of a register. Without these
        # a consumer can decode an instruction perfectly and still not be able
        # to write the line etnaviv's own disassembler writes, so there is
        # nothing to compare against.
        disp = bs.find('display')
        overrides = []
        for ov in bs.findall('override'):
            ov_expr = ov.find('expr')
            ov_disp = ov.find('display')
            ov_derived = {}
            for d in ov.findall('derived'):
                e = d.find('expr')
                ov_derived[d.get('name')] = {
                    'type': d.get('type'),
                    'width': int(d.get('width')) if d.get('width') else None,
                    'expr': (e.text or '').strip() if e is not None else None,
                }
            overrides.append({
                'expr': (ov_expr.text or '').strip() if ov_expr is not None else None,
                'display': (ov_disp.text or '').strip() if ov_disp is not None else None,
                'derived': ov_derived,
            })
        out[name] = {
            'extends': bs.get('extends'),
            # The MNEMONIC, where it differs from the bitset name. isaspec uses
            # this to collapse an operand-count family to one printed op:
            # branch_unary and branch_binary both display as "branch", and the
            # texkill pair likewise. Note bit_findlsb/bit_findmsb carry NO
            # displayname -- those are genuinely different mnemonics that share
            # a 7-bit opcode, separated by a sub-opcode. Dropping this made our
            # listing disagree with Mesa's on 208 instructions that were decoded
            # identically.
            'displayname': bs.get('displayname'),
            'size': int(bs.get('size')) if bs.get('size') else None,
            'fields': fields,
            'derived': derived,
            'patterns': patterns,
            'display': (disp.text or '').strip() if disp is not None else None,
            'overrides': overrides,
        }
    return out


def resolve(bitsets):
    """Flatten each bitset's inheritance chain.

    A leaf opcode's own XML carries only its opcode bits; everything else comes
    from the chain above it. Resolving here means the consumer never walks
    `extends` and cannot get the order wrong.
    """
    resolved = {}

    def walk(name, seen):
        if name in resolved:
            return resolved[name]
        if name in seen:
            raise ValueError(f'cycle in extends at {name}')
        bs = bitsets[name]
        fields, derived, patterns, size = {}, {}, [], bs['size']
        parent = bs['extends']
        chain = []
        display = bs['display']
        overrides = []
        if parent:
            up = walk(parent, seen | {name})
            fields.update(up['fields'])
            derived.update(up['derived'])
            patterns.extend(up['patterns'])
            size = size or up['size']
            chain = up['chain'] + [parent]
            # The nearest template wins: a leaf that names its own layout uses
            # it, and one that does not inherits the shape its instruction
            # class prints in. Overrides accumulate, because an inherited
            # condition (an unused destination) still applies to a leaf.
            display = display or up['display']
            overrides = up['overrides'] + bs['overrides']
        else:
            overrides = list(bs['overrides'])
        fields.update(bs['fields'])
        derived.update(bs['derived'])
        patterns.extend(bs['patterns'])
        resolved[name] = {
            'extends': parent,
            'chain': chain,
            'size': size,
            'fields': fields,
            'derived': derived,
            'patterns': patterns,
            'display': display,
            'overrides': overrides,
        }
        return resolved[name]

    for n in bitsets:
        walk(n, set())
    return resolved


def opcode_of(entry):
    """The 7-bit opcode: OPC in bits 5:0 plus OPCODE_BIT6 at bit 80.

    Derived from the leaf's own fully-masked patterns rather than assumed, so a
    leaf that does not pin both reports None and is not silently given one.
    """
    low6 = None
    bit6 = None
    for p in entry['patterns']:
        if p['low'] == 0 and p['high'] == 5 and p['mask'] == 0x3f:
            low6 = p['value']
        elif p['low'] == 80 and p['high'] == 80 and p['mask'] == 1:
            bit6 = p['value']
    if low6 is None:
        return None
    return low6 | ((bit6 or 0) << 6)


def instr_constraint(entry):
    """Fold an instruction's patterns into one 128-bit (value, mask) pair.

    Two instructions can be told apart iff no 128-bit word satisfies both, i.e.
    they disagree on at least one bit that both of them pin.
    """
    value = 0
    mask = 0
    for p in entry['patterns']:
        width = p['high'] - p['low'] + 1
        pm = p['mask'] << p['low']
        pv = (p['value'] & ((1 << width) - 1)) << p['low']
        # A bit pinned twice must be pinned the same way, or the description
        # contradicts itself and the table would be unusable.
        clash = mask & pm & (value ^ pv)
        if clash:
            raise ValueError(f'pattern bits disagree at mask {clash:#x}')
        value = (value & ~pm) | (pv & pm)
        mask |= pm
    return value, mask


def verify_unique(instructions):
    """Every instruction must be distinguishable from every other.

    The 7-bit opcode alone is NOT a key -- the branch and texkill families share
    one, and so do bit_findlsb/bit_findmsb -- so this checks the whole pinned
    pattern set, which is what a decoder actually matches on.
    """
    cons = {n: instr_constraint(i) for n, i in instructions.items()}
    names = sorted(cons)
    ambiguous = []
    for a in range(len(names)):
        for b in range(a + 1, len(names)):
            na, nb = names[a], names[b]
            (va, ma), (vb, mb) = cons[na], cons[nb]
            both = ma & mb
            if ((va ^ vb) & both) == 0:
                ambiguous.append((na, nb))
    return ambiguous


def build(mesa_tree):
    path = os.path.join(mesa_tree, ISA_SUBPATH)
    if not os.path.isfile(path):
        sys.exit(
            f'{path}: not found.\n'
            'Point --mesa (or $MESA_TREE) at a Mesa checkout that has '
            'src/etnaviv/isa; the tree is not vendored in this repository.'
        )
    with open(path, 'rb') as f:
        raw = f.read()
    root = ET.fromstring(raw)

    enums = parse_enums(root)
    templates = parse_templates(root)
    bitsets = parse_bitsets(root)
    res = resolve(bitsets)

    # Leaves are the named (non-'#') bitsets: those are the instructions.
    instructions = {}
    for name, entry in res.items():
        if name.startswith('#'):
            continue
        instructions[name] = {
            'opcode': opcode_of(entry),
            'mnemonic': bitsets[name].get('displayname') or name,
            'extends': entry['extends'],
            'chain': entry['chain'],
            'size': entry['size'],
            'patterns': entry['patterns'],
            'fields': entry['fields'],
            'derived': entry['derived'],
            # The disassembly format, so a consumer can print the line
            # etnaviv's own disassembler prints and the two can be compared.
            'display': entry['display'],
            'overrides': entry['overrides'],
        }

    describe = None
    try:
        describe = subprocess.run(
            ['git', 'describe', '--always', '--dirty'],
            cwd=mesa_tree, capture_output=True, text=True, timeout=10,
        ).stdout.strip() or None
    except (OSError, subprocess.SubprocessError):
        pass
    version = None
    vf = os.path.join(mesa_tree, 'VERSION')
    if os.path.isfile(vf):
        with open(vf) as f:
            version = f.read().strip()

    return {
        'provenance': {
            'source': 'Mesa src/etnaviv/isa/etnaviv.xml, transcribed, not vendored',
            'source_copyright': 'Copyright (c) 2023 Igalia S.L.',
            'source_license': 'MIT',
            'mesa_version': version,
            'mesa_describe': describe,
            'input_sha256': {'etnaviv.xml': hashlib.sha256(raw).hexdigest()},
            'generator': 'tools/gc7000-isa/extract.py',
            'schema_version': SCHEMA_VERSION,
            'note': (
                'etnaviv.xml carries no per-model gating: one description covers '
                'the GC family, and what differs per model lives in the driver\'s '
                'feature bits, not in these bit layouts. Our target is the '
                'GC7000UL (model 0x7000, revision 0x6204, HALTI5). Instructions '
                'are a fixed 128 bits; the opcode is bits 5:0 plus bit 80.'
            ),
        },
        'templates': templates,
        'enums': enums,
        # Both the raw hierarchy and the flattened leaves: the first is the
        # authority, the second is what a decoder actually wants.
        'bitsets': bitsets,
        'instructions': instructions,
    }


def render(table):
    return json.dumps(table, indent=2, sort_keys=False) + '\n'


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--mesa', default=os.environ.get('MESA_TREE', DEFAULT_MESA))
    ap.add_argument('-o', '--output')
    ap.add_argument('--check', action='store_true')
    ap.add_argument('--stats', action='store_true')
    args = ap.parse_args()

    table = build(args.mesa)
    text = render(table)

    if args.stats:
        ins = table['instructions']
        print(f'enums        {len(table["enums"])}')
        print(f'bitsets      {len(table["bitsets"])} '
              f'({sum(1 for n in table["bitsets"] if n.startswith("#"))} abstract)')
        print(f'instructions {len(ins)}')
        noop = [n for n, i in ins.items() if i['opcode'] is None]
        print(f'  with a decodable opcode: {len(ins) - len(noop)}')
        if noop:
            print(f'  WITHOUT one: {", ".join(sorted(noop))}')
        # The 7-bit opcode is NOT a decode key: report where it is shared, then
        # report the thing that actually matters, whether the full pinned
        # pattern set separates every pair.
        seen = {}
        for n, i in ins.items():
            if i['opcode'] is not None:
                seen.setdefault(i['opcode'], []).append(n)
        dup = {k: v for k, v in seen.items() if len(v) > 1}
        print(f'  opcodes shared by more than one instruction: {len(dup)}'
              + (f' -> {dup}' if dup else ''))
        amb = verify_unique(ins)
        print(f'  pairs NOT separable by their full pattern set: {len(amb)}'
              + (f' -> {amb}' if amb else '   (decode is unambiguous)'))

    if args.check:
        if not args.output:
            sys.exit('--check needs --output')
        with open(args.output) as f:
            if f.read() != text:
                sys.exit(f'{args.output}: stale; re-run without --check')
        print(f'{args.output}: up to date')
        return

    if args.output:
        os.makedirs(os.path.dirname(args.output), exist_ok=True)
        with open(args.output, 'w') as f:
            f.write(text)
        with open(args.output, 'rb') as f:
            written = f.read()
        print(f'{args.output}: {len(written)} bytes, '
              f'sha256 {hashlib.sha256(written).hexdigest()[:16]}')
    else:
        sys.stdout.write(text)


if __name__ == '__main__':
    main()
