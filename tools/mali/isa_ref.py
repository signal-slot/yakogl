#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The board's shader binaries, from a pandecode trace, as test data.

    tools/mali/isa_ref.py --about <line> [--about <line>]... <trace>... \\
        > tools/mali/isa-ref/board-<date>.txt

Each --about line (at least one: where the trace came from, which Mesa wrote
it) becomes a "# " comment at the top.

A trace is Mesa's PANDECODE_DUMP_FILE output (PAN_MESA_DEBUG=trace,sync).
Each "Shader 0x... (GPU VA <va>) sz <n>" block lists the binary one
instruction per line: the 8 bytes of the word in memory order, then Mesa's
disassembly of it (pandecode stops at the first zero word). Every block of
every trace is written out, in trace order, under a "# binary <trace> <va>"
line; the lines themselves are copied unchanged. A trace with no block, or a
block with no instruction, is an error: an empty fixture is not a pass.

The fixture's other half, Mesa 26.2.1's disassembly of the same words, is
tools/mali/va_disasm_ref.sh's output.
"""

import os
import re
import sys

HEADER = re.compile(r'^Shader 0x[0-9a-f]+ \(GPU VA ([0-9a-f]+)\) sz \d+$')
INSTR = re.compile(r'^(?:[0-9a-f]{2} ){7}[0-9a-f]{2}    \S')


def main(argv):
    out = []
    paths = []
    args = argv[1:]
    while args:
        if args[0] == '--about' and len(args) > 1:
            out.append('# ' + args[1])
            args = args[2:]
        else:
            paths.append(args[0])
            args = args[1:]
    if not paths or not out:
        print(__doc__, file=sys.stderr)
        return 2
    for path in paths:
        lines = open(path, encoding='utf-8').read().split('\n')
        blocks = 0
        i = 0
        while i < len(lines):
            m = HEADER.match(lines[i])
            i += 1
            if not m:
                continue
            out.append(f'# binary {os.path.basename(path)} {m.group(1)}')
            n = 0
            while i < len(lines) and INSTR.match(lines[i]):
                out.append(lines[i])
                n += 1
                i += 1
            if n == 0:
                print(f'isa_ref.py: {path}: the block at VA {m.group(1)} has no instruction', file=sys.stderr)
                return 1
            blocks += 1
        if blocks == 0:
            print(f'isa_ref.py: {path}: no shader block', file=sys.stderr)
            return 1
    sys.stdout.write('\n'.join(out) + '\n')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
