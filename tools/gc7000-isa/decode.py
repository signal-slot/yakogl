#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
#
# A reference decoder for GC7000 shader binaries, driven entirely by the table
# tools/gc7000-isa/extract.py produces. It answers one question: does the table
# describe the shaders this library actually ships?
#
# It is a verification instrument, not the product. The shipping decoder and
# encoder are Rust, in src/shaderc/online/.
#
#   tools/gc7000-isa/decode.py --blobs      # every shader in the Vivante table
#   tools/gc7000-isa/decode.py --hex '0f811018 39001f20 0 0'
#
# Instructions are a fixed 128 bits = four dwords, little-endian, bit 0 being
# the least significant bit of the first dword.

import argparse
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
DEFAULT_TABLE = os.path.join(REPO, 'docs/online-shaderc/isa/gc7000-isa.json')
BLOBS = os.path.join(REPO, 'tests/reference/vivante_blobs.rs')
CLEAR = os.path.join(REPO, 'src/vivante/clear_shader.rs')


class Table:
    def __init__(self, path):
        with open(path) as f:
            self.t = json.load(f)
        self.enums = self.t['enums']
        self.instructions = self.t['instructions']
        self.cons = {}
        for name, i in self.instructions.items():
            value = mask = 0
            for p in i['patterns']:
                width = p['high'] - p['low'] + 1
                pm = p['mask'] << p['low']
                pv = (p['value'] & ((1 << width) - 1)) << p['low']
                value = (value & ~pm) | (pv & pm)
                mask |= pm
            self.cons[name] = (value, mask)

    def match(self, word):
        """Every instruction whose pinned bits agree with `word`."""
        return [n for n, (v, m) in self.cons.items() if (word & m) == v]

    def field(self, name, word, fname):
        f = self.instructions[name]['fields'].get(fname)
        if not f:
            return None
        width = f['high'] - f['low'] + 1
        raw = (word >> f['low']) & ((1 << width) - 1)
        out = {'raw': raw, 'name': None}
        ty = f.get('type')
        if ty and ty.startswith('#') and ty in self.enums:
            for v in self.enums[ty]['values']:
                if v['value'] == raw:
                    out['name'] = v['name'] or v['display']
                    break
        return out


def decode_word(tab, word):
    hits = tab.match(word)
    return hits


def decode_shader(tab, words):
    """Split a shader into 128-bit instructions and decode each."""
    out = []
    if len(words) % 4:
        return None, f'{len(words)} dwords is not a whole number of instructions'
    for i in range(0, len(words), 4):
        w = (words[i] | (words[i + 1] << 32)
             | (words[i + 2] << 64) | (words[i + 3] << 96))
        hits = decode_word(tab, w)
        out.append({'index': i // 4, 'word': w, 'matches': hits})
    return out, None


def parse_blobs(path):
    """Every `static CODE_n: [u32; k] = [...]` in a generated Rust table."""
    with open(path) as f:
        text = f.read()
    shaders = {}
    for m in re.finditer(r'static (CODE_\w+)\s*:\s*\[u32;\s*\d+\]\s*=\s*\[(.*?)\];',
                         text, re.S):
        words = [int(x, 16) for x in re.findall(r'0x([0-9a-fA-F]+)', m.group(2))]
        shaders[m.group(1)] = words
    return shaders


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--table', default=DEFAULT_TABLE)
    ap.add_argument('--blobs', action='store_true')
    ap.add_argument('--hex')
    ap.add_argument('--verbose', action='store_true')
    args = ap.parse_args()

    tab = Table(args.table)

    if args.hex:
        words = [int(x, 16) for x in args.hex.split()]
        res, err = decode_shader(tab, words)
        if err:
            sys.exit(err)
        for r in res:
            print(f"  [{r['index']}] {r['word']:#034x} -> {r['matches']}")
        return

    if not args.blobs:
        ap.error('give --blobs or --hex')

    shaders = {}
    for p in (BLOBS, CLEAR):
        if os.path.isfile(p):
            for k, v in parse_blobs(p).items():
                shaders[f'{os.path.basename(p)}:{k}'] = v

    total_sh = len(shaders)
    total_in = 0
    undecoded = []       # no instruction matches: the table cannot read it
    ambiguous = []       # more than one matches: the table cannot choose
    misaligned = []
    histogram = {}

    for name, words in sorted(shaders.items()):
        res, err = decode_shader(tab, words)
        if err:
            misaligned.append((name, err))
            continue
        for r in res:
            total_in += 1
            n = len(r['matches'])
            if n == 0:
                undecoded.append((name, r['index'], r['word']))
            elif n > 1:
                ambiguous.append((name, r['index'], r['matches']))
            else:
                histogram[r['matches'][0]] = histogram.get(r['matches'][0], 0) + 1

    print(f'shaders      {total_sh}')
    print(f'instructions {total_in}')
    print(f'  decoded to exactly one instruction: '
          f'{total_in - len(undecoded) - len(ambiguous)}')
    print(f'  no match  (table cannot read):      {len(undecoded)}')
    print(f'  ambiguous (table cannot choose):    {len(ambiguous)}')
    if misaligned:
        print(f'  not a whole number of instructions: {len(misaligned)}')
        for n, e in misaligned[:5]:
            print(f'      {n}: {e}')
    print(f'  distinct opcodes seen: {len(histogram)} of '
          f'{len(tab.instructions)} in the table')
    for op, n in sorted(histogram.items(), key=lambda kv: -kv[1]):
        print(f'      {op:22s} {n}')
    for n, i, w in undecoded[:10]:
        print(f'  NO MATCH {n}[{i}] {w:#034x}')
    for n, i, m in ambiguous[:10]:
        print(f'  AMBIGUOUS {n}[{i}] {m}')

    sys.exit(1 if (undecoded or ambiguous or misaligned) else 0)


if __name__ == '__main__':
    main()
