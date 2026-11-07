#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Kicks a frame of one glmark2 host run with GM2_KICKS=1 on the PowerVR.

Reads the `gm2: draw <n> fb <fb> kick <k> submitted <s>` lines shim.c logs
after every draw (tools/hostharness/glmark2/README.md, GM2_KICKS) and the
`pvr_backend: ...` lines of the same log. A frame ends at a draw into fb 0
(the window: terrain's last pass, tilt_v). For each frame it prints the
draws, the framebuffer each draw rendered into and the kicks submitted
between two draws; the kicks a draw does not explain (more than the one
that closed the previous draw's frame) are the glGenerateMipmap levels
submitted between them. The summary is over every frame but the first
(setup) and the last (teardown may cut it).

usage: kicks.py <log>
"""
import re
import sys

DRAW = re.compile(r"^gm2: draw (\d+) fb (-?\d+) kick (\d+) submitted (\d+)$")


def main(path):
    draws = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            m = DRAW.match(line.strip())
            if m:
                draws.append(tuple(int(x) for x in m.groups()))
    if not draws:
        sys.exit(f"kicks.py: no 'gm2: draw' lines in {path} (GM2_KICKS=1, PowerVR)")
    frames, cur = [], []
    for d in draws:
        cur.append(d)
        if d[1] == 0:
            frames.append(cur)
            cur = []
    if cur:
        frames.append(cur)
    print(f"{len(draws)} draws, {len(frames)} frames (a frame ends at a draw into fb 0)")
    rows = []
    prev_sub = None
    for i, fr in enumerate(frames):
        first_sub = prev_sub if prev_sub is not None else fr[0][3]
        kicks = fr[-1][3] - first_sub
        seq = []
        last = first_sub
        for n, fb, k, s in fr:
            seq.append(f"fb{fb}+{s - last}")
            last = s
        rows.append((i, len(fr), kicks, " ".join(seq)))
        prev_sub = fr[-1][3]
    for i, nd, kicks, seq in rows:
        print(f"frame {i}: {nd} draws, {kicks} kicks submitted; per draw (its fb, kicks since the draw before): {seq}")
    body = rows[1:-1] if len(rows) > 2 else rows
    if body:
        k = sum(r[2] for r in body) / len(body)
        d = sum(r[1] for r in body) / len(body)
        print(f"frames 1..{len(rows) - 2}: {d:.2f} draws and {k:.2f} kicks submitted a frame")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
