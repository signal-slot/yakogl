#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compare the cluster sample's frame on the board with the host reference, block
by block.

  compare.py <uart-log> <ref-dir>

The image prints a thumbnail of its window on the UART: "thumb <W>x<H> blocks
of <B> px, window <w>x<h>", then one line per block row, "thumb <row> <hex>",
six hex digits (R, G, B) per block, each the mean of a B x B block. This
builds the same thumbnail from <ref-dir>/ref-<w>x<h>.rgba (host/ref/run.sh)
and prints, per block, the largest channel difference: the distribution, the
worst blocks with their window coordinates, and a map (one character a
block: '.' within 8, '+' within 24, '#' beyond, 'g' the blind spot). The two
renderers differ at edges and glyphs, so a small difference in text blocks is
expected; a block that differs by a lot shows where the frame is wrong.

The blind spot: Slint's software renderer, which draws the reference, strokes
a Path with one colour (internal/renderers/software/lib.rs:3186), so where a
gradient-stroked Path draws (the cluster's dial arcs) the reference is not the
picture. run.sh writes those bands next to the reference
(ref-<w>x<h>.blind.txt, from host/ref/src/main.rs gradient_strokes); the
blocks they touch are marked 'g', counted apart and left out of the rest.

Exit status: 0 when every block row of the window was read, 1 otherwise (a
short or garbled capture is not a result), 2 on usage errors.
"""
import math
import re
import sys


def board_thumb(log_path):
    head = None
    rows = {}
    for line in open(log_path, errors="replace"):
        m = re.search(r"thumb (\d+)x(\d+) blocks of (\d+) px, window (\d+)x(\d+)", line)
        if m:
            head = tuple(int(g) for g in m.groups())
            rows = {}
            continue
        m = re.search(r"thumb (\d+) ([0-9a-f]+)\s*$", line)
        if m and head:
            rows[int(m.group(1))] = m.group(2)
    return head, rows


def ref_thumb(path, w, h, block):
    data = open(path, "rb").read()
    if len(data) != w * h * 4:
        raise SystemExit(f"{path}: {len(data)} bytes, want {w * h * 4} for {w}x{h}")
    bw, bh = w // block, h // block
    out = []
    for by in range(bh):
        row = []
        for bx in range(bw):
            s = [0, 0, 0]
            for y in range(by * block, (by + 1) * block):
                base = (y * w + bx * block) * 4
                for x in range(block):
                    p = base + x * 4
                    s[0] += data[p]
                    s[1] += data[p + 1]
                    s[2] += data[p + 2]
            n = block * block
            row.append(tuple((c + n // 2) // n for c in s))
        out.append(row)
    return out


def blind_bands(path):
    bands = []
    for line in open(path):
        if line.startswith("#") or not line.strip():
            continue
        bands.append(tuple(float(v) for v in line.split()))
    return bands


def to_design(x, y, w, h):
    # app-window.slint:110-129, as host/ref/src/main.rs to_design
    fit = max(1605.0, min(1920.0, 720.0 * w / h))
    s = min(w / fit, h / 720.0)
    return 960.0 + (x - w / 2.0) / s, 360.0 + (y - h / 2.0) / s


def in_band(band, dx, dy):
    cx, cy, r_in, r_out, a0, a1, margin = band
    vx, vy = dx - cx, dy - cy
    r = math.hypot(vx, vy)
    if r < r_in - margin or r > r_out + margin:
        return False
    a = math.degrees(math.atan2(vx, -vy))
    pad = math.degrees(margin / max(r, 1.0))
    return a0 - pad <= a <= a1 + pad


def block_blind(bands, bx, by, block, w, h):
    # a block is in the blind spot if any of 5 x 5 sample points is
    for i in range(5):
        for j in range(5):
            x = bx * block + (i + 0.5) * block / 5
            y = by * block + (j + 0.5) * block / 5
            dx, dy = to_design(x, y, w, h)
            if any(in_band(b, dx, dy) for b in bands):
                return True
    return False


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    head, rows = board_thumb(sys.argv[1])
    if not head:
        print("no 'thumb ... blocks of ... px, window ...' line in the log: the check did not run")
        return 1
    bw, bh, block, w, h = head
    missing = [r for r in range(bh) if r not in rows or len(rows[r]) != bw * 6]
    if missing:
        print(f"thumbnail incomplete: rows {missing} missing or short (want {bh} rows of {bw * 6} hex digits)")
        return 1
    ref = ref_thumb(f"{sys.argv[2]}/ref-{w}x{h}.rgba", w, h, block)
    blind_path = f"{sys.argv[2]}/ref-{w}x{h}.blind.txt"
    try:
        bands = blind_bands(blind_path)
    except OSError:
        print(f"{blind_path} is missing: rerun host/ref/run.sh (it writes the blind spot next to the reference)")
        return 1
    if not bands:
        print(f"{blind_path} names no band: the blind spot would be empty, which it is not")
        return 1
    diffs = []
    blind = []
    lines = []
    for by in range(bh):
        s = rows[by]
        line = ""
        for bx in range(bw):
            b = tuple(int(s[bx * 6 + 2 * c: bx * 6 + 2 * c + 2], 16) for c in range(3))
            r = ref[by][bx]
            d = max(abs(b[c] - r[c]) for c in range(3))
            if block_blind(bands, bx, by, block, w, h):
                blind.append((d, bx, by, b, r))
                line += "g"
                continue
            diffs.append((d, bx, by, b, r))
            line += "." if d <= 8 else "+" if d <= 24 else "#"
        lines.append(line)
    n = len(diffs)
    within = lambda t: sum(1 for d in diffs if d[0] <= t)
    print(f"window {w}x{h}, {bw}x{bh} blocks of {block} px: {n} blocks compared, {len(blind)} in the "
          f"blind spot ({len(bands)} gradient-stroked bands; largest difference there "
          f"{max((d[0] for d in blind), default=0)}, not a verdict)")
    print(f"  within 4: {within(4)}, within 8: {within(8)}, within 24: {within(24)}, beyond 24: {n - within(24)}")
    print(f"  mean largest-channel difference {sum(d[0] for d in diffs) / n:.2f}")
    print("  worst blocks (window x..x+B, y..y+B: board rgb, reference rgb, diff):")
    for d, bx, by, b, r in sorted(diffs, reverse=True)[:12]:
        print(f"    ({bx * block:4}, {by * block:3}): {b} vs {r}, {d}")
    print("  map ('.' within 8, '+' within 24, '#' beyond, 'g' the blind spot):")
    for line in lines:
        print("    " + line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
