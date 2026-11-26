#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Add a synthetic remainder to a scratch copy of the i.MX8MP dEQP-GLES3 campaign record.

  imx8mp-gles3-synth.py SCRATCH

SCRATCH is a directory that holds a copy of the files the end-of-campaign tools read (tools/cts/imx8mp-gles3-*,
tools/cts/gles3-mustpass.txt, docs/es3, and the three real slices 9bc82915-rows01-13, -rows17-34, -rows35-51
under tools/cts/log). The script adds, under SCRATCH/tools/cts/log, the 12 slices the board has not run
(rows14-16, 52-57, 58-61, 62-67, 68-75, 76-83, 84-94, 95-107, 108-117, 118-122, 123-128, 129-133) and the
ES 2.0 record tools/cts/log/2026-11-imx8mp-es2-9bc82915 (29 verdict lines equal to the stage-1 baseline).
Per entry of a synthetic slice: begun = its mustpass cases (tree and mustpass list), failed = the cases the
5454be1b host column (docs/es3/host-gc-gles3-5454be1b, vivante) lists as Fail, named in the row, n.s. =
its NotSupported cases, the rest passes. The numbers are invented from the host column: nothing here is a
board result. Run by tools/cts/imx8mp-gles3-e2e-selftest.sh; it writes only under SCRATCH.
"""
import importlib.util, os, sys

SHA = '9bc82915e06367b46e3a62b644dbf2bb659d72ad'
SLICES = [(14, 16), (52, 57), (58, 61), (62, 67), (68, 75), (76, 83), (84, 94), (95, 107), (108, 117),
          (118, 122), (123, 128), (129, 133)]
SPDX = ['SPDX-License-Identifier: Apache-2.0', 'SPDX-FileCopyrightText: 2026 Signal Slot, Inc.']
HEADER = '# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names (\';\'-separated)'


def main(root):
    cts = os.path.join(root, 'tools/cts')
    spec = importlib.util.spec_from_file_location('merge', os.path.join(cts, 'imx8mp-gles3-merge.py'))
    merge = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(merge)
    table = merge.load_table(os.path.join(cts, 'imx8mp-gles3-tree.txt'))
    per = merge.entry_cases(table, os.path.join(cts, 'gles3-mustpass.txt'))
    assert len(table) == 133
    fails, nsup = set(), set()
    host = os.path.join(root, 'docs/es3/host-gc-gles3-5454be1b/host-gc-gles3-notpass.tsv')
    for l in open(host):
        f = l.rstrip('\n').split('\t')
        if l.startswith('#') or len(f) < 3 or f[0] != 'vivante':
            continue
        if f[2] == 'Fail':
            fails.add(f[1])
        elif f[2] == 'NotSupported':
            nsup.add(f[1])
    log = os.path.join(cts, 'log')
    for name in ('rows01-13', 'rows17-34', 'rows35-51'):
        assert os.path.isdir(os.path.join(log, '2026-11-imx8mp-gles3-9bc82915-' + name)), name

    counts = {}
    for i, cases in enumerate(per):
        fl = [c for c in cases if c in fails]
        ns = [c for c in cases if c in nsup and c not in fails]
        counts[i] = (len(cases), len(cases) - len(fl) - len(ns), fl, len(ns))

    def lic(path, text):
        open(path, 'w').write(text)
        open(path + '.license', 'w').write('\n'.join(SPDX) + '\n' + 'Synthetic i.MX8MP dEQP-GLES3 slice file, '
                                           'written by tools/cts/imx8mp-gles3-synth.py.\n')

    for n, (a, b) in enumerate(SLICES):
        ents = range(a - 1, b)
        label = 'rows%d-%d' % (a, b)
        d = os.path.join(log, '2026-11-imx8mp-gles3-9bc82915-' + label)
        os.makedirs(d)
        day = 20 + n // 3
        hh = 8 + (n % 3) * 4
        verdicts = []
        for i in ents:
            begun, p, fl, ns = counts[i]
            w1 = (len(fl) << 16) | p
            verdicts.append('board.sh imx8mp e%02d rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled '
                            'result=record:DEQP resets=absent record=594b%04x,%08x,%08x,ffff%04x '
                            'back_after=60s uptime=25s' % (i, 0x800 + i, w1, ns, begun))
        readme = ['<!-- %s -->' % SPDX[0], '<!-- %s -->' % SPDX[1],
                  '# i.MX8MP dEQP-GLES3 table image es3-9bc82915, rows %d-%d (entries %d-%d), SYNTHETIC'
                  % (a, b, a - 1, b - 1), '',
                  'commit ' + SHA, '',
                  'Run 2026-11-%02d, %02d:00-%02d:50 UTC (synthetic: invented by tools/cts/imx8mp-gles3-synth.py '
                  'from the host column, not a board run).' % (day, hh, hh + 3), '',
                  '## Verdict lines', '', '```'] + verdicts + ['```', '']
        open(os.path.join(d, 'README.md'), 'w').write('\n'.join(readme))
        tsv, md = [HEADER], ['', '| entry | begun | pass | fail | notsup | other | no result | failing names |',
                             '|---|---|---|---|---|---|---|---|']
        for i, flt in enumerate(table):
            if i in ents:
                begun, p, fl, ns = counts[i]
                cells = [str(begun), str(p), str(len(fl)), str(ns), '0', '0', ';'.join(fl) or '-']
            else:
                cells = ['-'] * 7
            tsv.append('\t'.join([str(i), flt] + cells))
            md.append('| %d | %s |' % (i, ' | '.join(cells)))
        lic(os.path.join(d, 'results.tsv'), '\n'.join(tsv) + '\n')
        lic(os.path.join(d, 'rows.md'), '\n'.join(md[1:]) + '\n')
        open(os.path.join(d, 'verdicts.txt'), 'w').write(
            '\n'.join(['# ' + s for s in SPDX] + ['# every verdict line (synthetic)'] + verdicts) + '\n')
        for i in ents:
            open(os.path.join(d, 'e%02d.console.txt' % i), 'w').write(
                '\n'.join(['# ' + s for s in SPDX] +
                          ['# i.MX8MP dEQP-GLES3 table image es3-9bc82915, row e%02d (table entry %d): synthetic' % (i, i),
                           '# capture: 1 bytes in console.raw, 1 line(s) in console.ts', '%02d:00:00 ' % hh]) + '\n')
        open(os.path.join(d, 'console-scan.txt'), 'w').write(
            '\n'.join(['# ' + s for s in SPDX] + ['# synthetic: no console text'] +
                      ['== row e%02d (0 lines, 0 boot banner(s), 0 row(s))' % i for i in ents]) + '\n')

    # ES 2.0 record: the stage-1 baseline's counts as verdict lines
    d = os.path.join(log, '2026-11-imx8mp-es2-9bc82915')
    os.makedirs(d)
    lines = []
    for l in open(os.path.join(cts, 'imx8mp-full-tree-29460f96.results.tsv')):
        if l.startswith('#'):
            continue
        f = l.split('\t')
        begun, p, fl, ns, ot = (int(x) for x in f[2:7])
        lines.append('board.sh imx8mp e%02d rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled '
                     'result=record:DEQP resets=absent record=594b08cc,%08x,%08x,ffff%04x '
                     'back_after=60s uptime=20s' % (int(f[0]), (fl << 16) | p, (ot << 16) | ns, begun))
    assert len(lines) == 29
    open(os.path.join(d, 'README.md'), 'w').write(
        '\n'.join(['<!-- %s -->' % SPDX[0], '<!-- %s -->' % SPDX[1],
                   '# i.MX8MP ES 2.0 regression rows at 9bc82915 (SYNTHETIC)', '', 'commit ' + SHA, '', '```'] +
                  lines + ['```', '']))


if __name__ == '__main__':
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
