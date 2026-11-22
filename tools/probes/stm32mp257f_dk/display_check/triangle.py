#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""triangle.py - judge a KVM screenshot of mp2_gpu_scanout's GPU triangle.

  triangle.py SHOT.png|SHOT.ppm

gc_triangle (driver/samples/zephyr/gc_triangle/src/main.c) clears its target
to XRGB 0x102030 (a dark blue-grey) and draws one Gouraud triangle: a red
vertex at the top centre, a green one bottom left, a blue one bottom right
(clip space (0,.8) (-.8,-.8) (.8,-.8)).  Prints one line:

  triangle: PASS: red apex top, green bottom left, blue bottom right ...
  triangle: FAIL: <reason>

The judge is scale tolerant (the KVM may scale or letterbox the picture, and
the scanned-out target may be smaller than the mode): it uses no absolute
coordinates.  Every pixel is classified red / green / blue when that channel
dominates; the three classes' centroids and the bounding box of all of them
must have the triangle's shape (red above the other two and between them
horizontally, green left of blue, green and blue at the same height), each
class must be a visible part of the picture, and most of the picture must be
dark (the clear colour or a black border), so colour bars, which also hold
pure red, green and blue, fail.

Exit 0 on PASS, 1 on FAIL, 2 on an unreadable input.  Standard library only;
the PNG/PPM reader is colorbars.py's.
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from colorbars import ImageError, load  # noqa: E402

MAXSAMPLES = 400000


def classify(px):
    r, g, b = px
    m = max(px)
    if m < 140:
        return "dark" if m < 100 else None
    for name, v, o1, o2 in (("red", r, g, b), ("green", g, r, b),
                            ("blue", b, r, g)):
        if v == m and o1 <= 0.45 * v and o2 <= 0.45 * v:
            return name
    return "other"


def judge_gpu_triangle(img, min_part=0.002, max_lit=0.45):
    step = 1
    while (img.width // step) * (img.height // step) > MAXSAMPLES:
        step += 1
    acc = dict((c, [0, 0, 0]) for c in ("red", "green", "blue"))
    n = bright = 0
    x0 = y0 = 10 ** 9
    x1 = y1 = -1
    for y in range(0, img.height, step):
        for x in range(0, img.width, step):
            n += 1
            c = classify(img.pixel(x, y))
            if c in ("dark", None):
                continue
            bright += 1
            if c in acc:
                a = acc[c]
                a[0] += 1
                a[1] += x
                a[2] += y
                x0, x1, y0, y1 = min(x0, x), max(x1, x), min(y0, y), max(y1, y)
    if not any(a[0] for a in acc.values()):
        return False, "no triangle: no red, green or blue pixels (blank or no signal)"
    missing = [c for c, a in acc.items() if a[0] < n * min_part]
    if missing:
        return False, "missing vertex colour: %s" % ", ".join(missing)
    if bright > n * max_lit:
        return False, ("too much colour: %.0f%% of the picture is lit "
                       "(colour bars or a full-screen pattern?)"
                       % (100.0 * bright / n))
    cen = dict((c, (a[1] / a[0], a[2] / a[0])) for c, a in acc.items())
    bw, bh = x1 - x0, y1 - y0
    if bw < img.width * 0.05 or bh < img.height * 0.05:
        return False, "triangle too small: %dx%d px" % (bw, bh)
    mx, my = x0 + bw / 2.0, y0 + bh / 2.0
    (rx, ry), (gx, gy), (bx, by) = cen["red"], cen["green"], cen["blue"]
    shown = " ".join("%s(%.2f,%.2f)" % (c, (cen[c][0] - x0) / bw,
                                         (cen[c][1] - y0) / bh)
                     for c in ("red", "green", "blue"))
    # blue at the top, red at the bottom right: red and blue exchanged
    if by < my < ry and gx < mx < rx and abs(gy - ry) < 0.2 * bh:
        return False, "red and blue swapped (%s): a pixel-format mistake" % shown
    bad = []
    if not (ry < my - 0.1 * bh and gy > my and by > my):
        bad.append("red is not the top vertex")
    if not gx < bx - 0.2 * bw:
        bad.append("green is not left of blue")
    if not gx < rx < bx:
        bad.append("red is not between green and blue")
    if abs(gy - by) > 0.15 * bh:
        bad.append("green and blue are not level")
    if bad:
        return False, "wrong shape: %s (%s)" % ("; ".join(bad), shown)
    return True, ("red apex top, green bottom left, blue bottom right, "
                  "box %dx%d at %d,%d (%s)" % (bw, bh, x0, y0, shown))


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("image")
    a = ap.parse_args(argv)
    try:
        img = load(a.image)
    except ImageError as e:
        print("triangle: FAIL: unreadable input: %s" % e)
        return 2
    ok, msg = judge_gpu_triangle(img)
    print("triangle: %s: %s" % ("PASS" if ok else "FAIL", msg))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
