#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Criterion c1: after tools/mali/corpus-verdicts.sh, the femtovg and fvgtest
programs of tools/mali/corpus-verdicts.txt are ok or refused for a texture
only.  Prints each other refusal and exits 1; else prints a summary, exits 0.

    python3 -I tools/mali/fvg_refusals.py [tools/mali/corpus-verdicts.txt]
"""
import sys

DEFAULT = "tools/mali/corpus-verdicts.txt"
PREFIXES = ("tools/shaders/femtovg/", "tools/shaders/fvgtest/")

# The refusal texts (after "<path>: Valhall: ") that count as texture refusals.
TEXTURE_REFUSALS = (
    "a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet",
    "a texture look-up other than texture2D(sampler2D, coordinate) "
    "(a bias, a level, a projection, a cube map, a size query) is not lowered yet",
    "a second texture look-up in one fragment shader is not lowered yet",
)


def classify(lines):
    """Return (ok, texture, other); other is a list of the offending lines."""
    ok = texture = 0
    other = []
    for line in lines:
        line = line.rstrip("\n")
        if not line.startswith(PREFIXES):
            continue
        if line.endswith(": ok"):
            ok += 1
            continue
        _, _, text = line.partition(": ")
        if text.startswith("Valhall: "):
            text = text[len("Valhall: "):]
        if text in TEXTURE_REFUSALS:
            texture += 1
        else:
            other.append(line)
    return ok, texture, other


def main(argv):
    path = argv[1] if len(argv) > 1 else DEFAULT
    with open(path, encoding="utf-8") as fh:
        ok, texture, other = classify(fh)
    for line in other:
        print(line)
    print(f"{ok} ok, {texture} texture refusal(s), {len(other)} other")
    return 1 if other else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
