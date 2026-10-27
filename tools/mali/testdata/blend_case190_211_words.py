#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The blend shaders of a pandecode trace (blend_case190_211_ref.sh), one
<prefix>.words (one 64-bit word per line, as the "words" of the
*.expect.json files: the 8 bytes little-endian) and <prefix>.txt (pandecode's
bytes and disassembly of the shader) for each "Blend shader" block, in the
order the trace has them.

    blend_case190_211_words.py <trace> <prefix> [<prefix> ...]

Every prefix takes the next block. Case 211 has one blend shader, case 190
two (its column 3, MIN on the colour, then column 4, 1-Ad / 1-As).
"""
import re
import sys


def blocks(trace):
    lines = open(trace).read().split("\n")
    out = []
    i = 0
    while i < len(lines):
        if lines[i].startswith("Blend shader "):
            i += 1
            assert lines[i].startswith("Shader "), lines[i]
            i += 1
            block = []
            while re.match(r"^(?:[0-9a-f]{2} ){7}[0-9a-f]{2}    \S", lines[i]):
                block.append(lines[i])
                i += 1
            out.append(block)
        else:
            i += 1
    return out


def main():
    if len(sys.argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    found = blocks(sys.argv[1])
    prefixes = sys.argv[2:]
    if len(found) != len(prefixes):
        print("blend_case words: %d blend shader(s) in the trace, %d prefix(es)" % (len(found), len(prefixes)),
              file=sys.stderr)
        return 1
    for prefix, block in zip(prefixes, found):
        with open(prefix + ".txt", "w") as f:
            f.write("".join(line + "\n" for line in block))
        with open(prefix + ".words", "w") as f:
            for line in block:
                b = line[:23].split(" ")
                f.write("".join(reversed(b)) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
