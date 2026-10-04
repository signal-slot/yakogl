#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
#
# Which GLSL builtins the shipped shaders actually call, and in what order
# implementing them unblocks the most of them.
#
#   tools/gc7000-isa/builtin_usage.py            # both tables
#   tools/gc7000-isa/builtin_usage.py --order    # just the write order
#
# This exists because "frequency order" is the obvious answer and the wrong
# one. A shader compiles only when EVERY builtin it calls is implemented, so
# the decision-relevant number is not how often a builtin is called but how
# many additional shaders become fully covered when it is added. The two orders
# differ sharply: `clamp` is called 229 times and completes 2 shaders; `mix` is
# called 112 times and completes 16.
#
# Both counts are printed, so a reader can see that difference rather than
# trust this file's choice of which to sort by.
#
# The count is lexical: a name followed by `(` in the GLSL text, filtered to
# the builtin list. It cannot over-count (a user function may not take a
# builtin's name) and would under-count a builtin reached only through a macro,
# of which these tables have none.

import argparse
import os
import re
import sys
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
TABLES = {
    'gc7000': os.path.join(REPO, 'src/shaderc/vivante_blobs.rs'),
    'powervr': os.path.join(REPO, 'src/shaderc/blobs.rs'),
}

# The callable builtins of GLSL ES 1.00 plus the GLSL 450 spellings the
# offline rewriter produces (`texture` for `texture2D`, `texelFetch`, ...).
BUILTINS = set(
    """radians degrees sin cos tan asin acos atan sinh cosh tanh pow exp log exp2 log2 sqrt inversesqrt
    abs sign floor ceil fract mod min max clamp mix step smoothstep trunc round roundEven modf isnan isinf
    length distance dot cross normalize faceforward reflect refract
    matrixCompMult outerProduct transpose inverse determinant
    lessThan lessThanEqual greaterThan greaterThanEqual equal notEqual any all not
    texture2D texture2DProj texture2DLod texture2DProjLod textureCube textureCubeLod
    texture textureProj textureLod textureProjLod textureGrad texelFetch textureSize
    dFdx dFdy fwidth""".split()
)

ENTRY = re.compile(r'ShcSource \{\s*name:\s*"([^"]*)",\s*text:\s*concat!\((.*?)\),\s*\}', re.S)
LITERAL = re.compile(r'"((?:[^"\\]|\\.)*)"')
CALL = re.compile(r'\b([A-Za-z_][A-Za-z0-9_]*)\s*\(')


def sources(path):
    """name -> the set of builtins it calls, for every ShcSource in a table."""
    with open(path) as f:
        text = f.read()
    marker = 'pub static SHC_SOURCE_TABLE'
    if marker not in text:
        sys.exit(f'{path}: no {marker}')
    table = text[text.index(marker):]
    out = {}
    for name, body in ENTRY.findall(table):
        glsl = ''.join(lit.encode().decode('unicode_escape') for lit in LITERAL.findall(body))
        # A comment can hold a builtin's name in prose; strip them first.
        glsl = re.sub(r'/\*.*?\*/', '', glsl, flags=re.S)
        glsl = re.sub(r'//[^\n]*', '', glsl)
        out[name] = {m.group(1) for m in CALL.finditer(glsl) if m.group(1) in BUILTINS}
    return out


def write_order(uses):
    """Greedy: at each step the builtin that completes the most shaders.

    Stops when no single addition completes another shader. The rest need a
    combination, and that tail is reported separately rather than ordered by a
    rule the data does not support.
    """
    have, order = set(), []
    remaining = {n: u for n, u in uses.items() if u}
    covered = sum(1 for u in uses.values() if not u)
    while remaining:
        best, gain = None, 0
        for b in {x for u in remaining.values() for x in u} - have:
            g = sum(1 for u in remaining.values() if u <= have | {b})
            if g > gain:
                best, gain = b, g
        if not best:
            break
        have.add(best)
        done = [n for n, u in remaining.items() if u <= have]
        for n in done:
            del remaining[n]
        covered += len(done)
        order.append((best, len(done), covered))
    return order, have, remaining


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--table', choices=sorted(TABLES), default='gc7000')
    ap.add_argument('--order', action='store_true', help='print only the write order')
    args = ap.parse_args()

    uses = sources(TABLES[args.table])
    total = len(uses)
    none = sum(1 for u in uses.values() if not u)

    calls, shaders = Counter(), Counter()
    for _, used in uses.items():
        for n in used:
            shaders[n] += 1
    with open(TABLES[args.table]) as f:
        table = f.read()
    table = table[table.index('pub static SHC_SOURCE_TABLE'):]
    for _, body in ENTRY.findall(table):
        glsl = ''.join(lit.encode().decode('unicode_escape') for lit in LITERAL.findall(body))
        glsl = re.sub(r'/\*.*?\*/', '', glsl, flags=re.S)
        glsl = re.sub(r'//[^\n]*', '', glsl)
        for m in CALL.finditer(glsl):
            if m.group(1) in BUILTINS:
                calls[m.group(1)] += 1

    print(f'{args.table}: {total} sources, {none} of them ({100 * none / total:.0f} %) call no builtin at all')

    if not args.order:
        print(f"\n{'builtin':16s} {'calls':>6s} {'shaders':>8s}")
        for n, c in calls.most_common():
            print(f'{n:16s} {c:6d} {shaders[n]:8d}')
        print(f'\ndistinct builtins called: {len(calls)}')

    order, have, remaining = write_order(uses)
    print('\nwrite order -- each line is the builtin that completes the most shaders next\n')
    print(f"{'#':>2} {'builtin':14s} {'+shaders':>8s} {'cumulative':>10s} {'%':>7s}")
    for i, (b, g, c) in enumerate(order, 1):
        print(f'{i:2d} {b:14s} {g:8d} {c:10d} {100 * c / total:6.1f}%')

    if remaining:
        tail = Counter()
        for u in remaining.values():
            for b in u - have:
                tail[b] += 1
        print(f'\n{len(remaining)} sources still need a combination; what they are missing:')
        for b, c in tail.most_common():
            print(f'   {b:16s} {c:3d} shaders')


if __name__ == '__main__':
    main()
