#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Generate tools/cts/imx8mp-gles3-tree.txt from tools/cts/gles3-mustpass.txt.

  imx8mp-gles3-gen.py > tools/cts/imx8mp-gles3-tree.txt

The mustpass names form a tree. Walking it in order, a node whose mustpass cases fit the group's per-entry
budget becomes one pattern "<node>.*"; a bigger node is split into its children (a node with only cases
below it is split by the leading '_' words of the case names, "<node>.<words>_*"). Consecutive patterns are
packed into one entry while the entry stays within the smallest budget of its patterns and within CHARS
characters. Budgets are cases per boot (the image's WDOG1 window is 600 s): the AM62P's whole-group images
ran up to 2,000 shader cases per window (RESULTS.md 1r); its texture/fbo/fragment rows were the slow ones
(texture.filtering .. texture.size outlived 600 s with 485 results, filtering.3d took 19 s a case), and the
GC7000UL needed the GLES2 texture groups split to 36-192 cases. So: shaders 1,000 (random 600), the other
groups 800; ubo 600, fragment_ops 500, fbo/fragment_out 400, transform_feedback 300; texture 150,
texture.filtering.3d 20 (19-25 s a case on the AM62P, RESULTS.md 1r: images of 22 cases fit), size 30,
format 40, shadow 80.
"""
import os, sys

CHARS = 1900   # under the 2035 limit
BUDGET = [      # (name prefix, cases per entry); first match wins
    ('functional.texture.filtering.3d', 20),
    ('functional.texture.size', 30),
    ('functional.texture.format', 40),
    ('functional.texture.shadow', 80),
    ('functional.texture', 150),
    ('functional.fbo', 400),
    ('functional.fragment_out', 400),
    ('functional.fragment_ops', 500),
    ('functional.transform_feedback', 300),
    ('functional.ubo', 600),
    ('functional.shaders.random', 600),
    ('functional.shaders', 1000),
]
DEFAULT = 800

def budget(name):
    n = name.split('.', 1)[1]
    for p, b in BUDGET:
        if n == p or n.startswith(p + '.'):
            return b
    return DEFAULT

def main():
    here = os.path.dirname(os.path.abspath(__file__))
    cases = [l.strip() for l in open(os.path.join(here, 'gles3-mustpass.txt')) if l.strip()]
    def split(names, depth):
        """yield (child, names) for the names sharing their first `depth` components; child None: a case"""
        groups = {}
        for n in names:
            parts = n.split('.')
            groups.setdefault(parts[depth] if len(parts) > depth + 1 else None, []).append(n)
        for k, g in groups.items():
            yield k, g
    pats = []
    def walk(names, depth):
        prefix = '.'.join(names[0].split('.')[:depth])
        if depth >= 2 and len(names) <= budget(names[0]):
            pats.append((prefix + '.*', len(names), budget(names[0])))
            return
        groups = list(split(names, depth))
        if all(k is None for k, _ in groups):   # only cases below: split the names by their '_' words
            leafsplit(names, depth, 1)
            return
        for k, g in groups:
            if k is None:       # a case that ends here
                for n in g:
                    pats.append((n, 1, budget(n)))
            else:
                walk(g, depth + 1)
    def leafsplit(names, depth, t):
        """names are cases sharing the first `depth` dot components; group them by the first t '_' words"""
        b = budget(names[0])
        byk = {}
        for n in names:
            words = n.split('.')[depth].split('_')
            if len(words) <= t:
                pats.append((n, 1, b))      # the whole name is the key
            else:
                byk.setdefault('_'.join(words[:t]), []).append(n)
        for k, g in byk.items():
            if len(g) > b and any(len(n.split('.')[depth].split('_')) > t + 1 for n in g):
                leafsplit(g, depth, t + 1)
            else:
                pre = '.'.join(g[0].split('.')[:depth])
                pats.append((pre + '.' + k + '_*', len(g), b))
    # depth 1: the children of dEQP-GLES3 (info, functional); dEQP-GLES3.functional is never under budget
    walk(cases, 1)
    entries, cur, n, cap = [], [], 0, 1 << 30
    for p, c, b in pats:
        nc = min(cap, b)
        # lifetime.* alone: on the AM62P its init dies (RESULTS.md 1r), and it must not take other cases with it
        iso = p.startswith('dEQP-GLES3.functional.lifetime.') != any(q.startswith('dEQP-GLES3.functional.lifetime.') for q in cur)
        if cur and (iso or n + c > nc or len(','.join(cur + [p])) > CHARS):
            entries.append(cur); cur, n, cap = [], 0, 1 << 30
            nc = b
        cur.append(p); n += c; cap = nc
    if cur: entries.append(cur)
    out = sys.stdout
    out.write("""# The whole dEQP-GLES3 tree for the Verdin i.MX8M Plus in %d table entries (one CONFIG_DEQP_CASE_TABLE entry per line; the selector i runs line i, counting from 0), DEQP_MODULE=gles3.
# Limit of the deqp sample (src/main.cpp case_argument): one entry is at most 2035 characters (arg[2048]), the selector index is masked to 8 bits (at most 256 entries). The longest entry here is %d.
# Generated by tools/cts/imx8mp-gles3-gen.py from tools/cts/gles3-mustpass.txt (%d cases); every mustpass case is matched by exactly one entry (tools/cts/check-imx8mp-gles3-tree.py). The filters match dEQP's whole tree; the tally restricts to the mustpass where it is given one.
# Sizing: an image's WDOG1 window is 600 s (boards/verdin_imx8mp_mimx8ml8_a53.conf), so an entry holds at most 1000 shader cases (random 600), 800 of the API groups, 400-600 ubo/fbo/fragment_out/fragment_ops, 300 transform_feedback, 150 texture, and 20 texture.filtering.3d (19-25 s a case on the AM62P: 1r ran it in images of 22 cases), 30 texture.size, 40 texture.format, 80 texture.shadow (AM62P 1r: 2,000 per image for shaders; the GC7000UL's GLES2 texture groups ran 36-192 cases an entry).
# To build: DEQP_MODULE=gles3 tools/cts/imx8mp-deqp-build.sh <name> 'dEQP-GLES3.info.*' --table tools/cts/imx8mp-gles3-tree.txt (docs/es3/board-recipe-imx8mp.md).
""" % (len(entries), max(len(','.join(e)) for e in entries), len(cases)))
    for e in entries:
        out.write(','.join(e) + '\n')

main()
