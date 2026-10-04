#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
#
# Compare our reading of every GC7000 shader we ship against Mesa's own
# disassembler, instruction by instruction, and print the disagreement count.
#
# This is the check that a table cannot give itself: a DIFFERENT tool, reading
# the same bytes. Mesa's etnaviv-disasm is built from src/etnaviv/isa/disasm.c
# over the isaspec decoder; ours is tools/gc7000-isa/decode.py over the table
# extract.py transcribed. They share no code.
#
#   tools/gc7000-isa/crosscheck.py                 # needs etnaviv-disasm built
#   tools/gc7000-isa/crosscheck.py --disasm <path>
#
# Build Mesa's side once (the bare target name does not resolve):
#   cd <mesa>/build-etnaviv && ninja src/etnaviv/isa/etnaviv-disasm
#
# Scope: this compares the OPCODE per instruction, which both tools print
# today. The operand rendering (register, write mask, swizzle) is
# shaderc-remote's listing work; when it lands this grows a second column.
# Reporting an opcode-only agreement as if it covered operands would be the
# kind of overclaim this file exists to avoid, so the output says which it is.

import argparse
import os
import re
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)

from decode import BLOBS, CLEAR, Table, decode_shader, parse_blobs  # noqa: E402

DEFAULT_TABLE = os.path.join(REPO, 'docs/online-shaderc/isa/gc7000-isa.json')
DEFAULT_DISASM = os.path.expanduser(
    '~/pvr-lab/mesa-build/mesa-26.2.1/build-etnaviv/src/etnaviv/isa/etnaviv-disasm')

# "000 [01821009 00000004 00000000 00010028] mov     t2.xy__, void, ..."
LINE = re.compile(r'^\s*(\d+)\s+\[([0-9a-f ]+)\]\s+(\S+)')


def mesa_opcodes(disasm, words):
    """Run Mesa's disassembler over one shader; return its opcode per slot."""
    with tempfile.NamedTemporaryFile(suffix='.bin', delete=False) as f:
        f.write(b''.join(struct.pack('<I', w) for w in words))
        path = f.name
    try:
        out = subprocess.run([disasm, path], capture_output=True, text=True,
                             timeout=60)
    finally:
        os.unlink(path)
    ops = {}
    for line in out.stdout.splitlines():
        m = LINE.match(line)
        if m:
            ops[int(m.group(1))] = m.group(3)
    return ops, out.stdout


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--table', default=DEFAULT_TABLE)
    ap.add_argument('--disasm', default=DEFAULT_DISASM)
    ap.add_argument('--verbose', action='store_true')
    args = ap.parse_args()

    if not os.path.isfile(args.disasm):
        sys.exit(
            f'{args.disasm}: not found.\n'
            'Build it with:  cd <mesa>/build-etnaviv && '
            'ninja src/etnaviv/isa/etnaviv-disasm')

    tab = Table(args.table)
    shaders = {}
    for p in (BLOBS, CLEAR):
        if os.path.isfile(p):
            for k, v in parse_blobs(p).items():
                shaders[f'{os.path.basename(p)}:{k}'] = v

    n_sh = n_in = 0
    agree = 0
    disagree = []
    modifiers = {}        # base agreed; Mesa additionally printed modifiers
    missing = []          # Mesa printed no line for a slot we decoded
    extra = []            # Mesa printed a slot we did not

    for name, words in sorted(shaders.items()):
        res, err = decode_shader(tab, words)
        if err:
            disagree.append((name, -1, 'ours', err, 'mesa', '-'))
            continue
        mops, _ = mesa_opcodes(args.disasm, words)
        n_sh += 1
        for r in res:
            n_in += 1
            ours = r['matches'][0] if len(r['matches']) == 1 else \
                ('<none>' if not r['matches'] else '<ambiguous>')
            theirs = mops.get(r['index'])
            if theirs is None:
                missing.append((name, r['index'], ours))
                continue
            # Mesa renders the display template: the bitset's name followed
            # by the modifier fields, "cmp.lt", "add.s32", "select.nz.u32".
            # Our decoder names the bitset only. Comparing the two as strings
            # compares different things -- the base opcode is the claim both
            # tools make, and the suffix is a rendering our listing does not
            # do yet. So split, count both, and never let a suffix-only
            # difference read as a decode disagreement.
            base = theirs.split('.', 1)[0]
            # Compare MNEMONICS, not bitset names: isaspec's displayname
            # collapses branch_unary/branch_binary to "branch" and the texkill
            # pair likewise, which is what Mesa prints.
            ours = tab.instructions[ours].get('mnemonic', ours) \
                if ours in tab.instructions else ours
            if base == ours:
                agree += 1
                if base != theirs:
                    modifiers[theirs] = modifiers.get(theirs, 0) + 1
            else:
                disagree.append((name, r['index'], ours, theirs))
        for idx in mops:
            if idx >= len(res):
                extra.append((name, idx, mops[idx]))

    print(f'shaders compared      {n_sh}')
    print(f'instructions compared {n_in}')
    print(f'  base opcode AGREES  {agree}')
    print(f'  base opcode DISAGREES {len(disagree)}')
    nmod = sum(modifiers.values())
    print(f'  of the agreeing, Mesa also printed modifiers on {nmod} '
          f'({len(modifiers)} distinct forms)')
    for form, n in sorted(modifiers.items(), key=lambda kv: -kv[1])[:12]:
        print(f'      {form:20s} {n}')
    print(f'  Mesa printed no line for a slot we decoded: {len(missing)}')
    print(f'  Mesa printed a slot we did not:             {len(extra)}')
    for d in disagree[:20]:
        print(f'  DISAGREE {d[0]}[{d[1]}] ours={d[2]!r} mesa={d[3]!r}')
    for m in missing[:10]:
        print(f'  MISSING  {m[0]}[{m[1]}] ours={m[2]!r}')
    print()
    print('Scope: opcode only. Operand rendering is compared once '
          'shaderc-remote\'s listing packs registers and write masks.')
    sys.exit(1 if (disagree or missing or extra) else 0)


if __name__ == '__main__':
    main()
