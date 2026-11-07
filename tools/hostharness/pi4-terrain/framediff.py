#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Per-frame cost of the last entry of two one-process.sh runs
(PI4T_PROFILE=1) that differ only in its frame count: every "frames"-phase
total of the longer run less the shorter one's, over the frames between
them. That difference is the last entry's own frames in the state the
entries before it left (docs/notes/pi4-slow-terrain.md): per GL/EGL entry
point the calls, wall and cpu us a frame, the V3D CPU waits and the TFU
model's jobs; the frame's wall, cpu and GL time; jobs and waits a frame.

usage: framediff.py <short log> <long log> <frames between them>
"""
import re
import sys


def parse(path):
    calls, waits, gpu, tot = {}, {}, {}, {}
    for line in open(path, errors="replace"):
        m = re.match(r"gm2: gl-time (\S+) setup=\S+ frames=(\d+),(\d+),(\d+)", line)
        if m:
            calls[m.group(1)] = tuple(int(x) for x in m.group(2, 3, 4))
        m = re.match(r"gm2: v3d-waits (\S+) setup=\d+ frames=(\d+)", line)
        if m:
            waits[m.group(1)] = int(m.group(2))
        m = re.match(r"gm2: gpu-time (\S+) setup=\S+ frames=(\d+),(\d+)", line)
        if m:
            gpu[m.group(1)] = (int(m.group(2)), int(m.group(3)))
        m = re.match(r"gm2: gl-frames swaps=\d+ counted=\d+ wall_ns=(\d+) cpu_ns=(\d+) gl_wall_ns=(\d+)"
                     r" gpu_jobs=(\d+) gpu_ns=(\d+)", line)
        if m:
            tot["wall"], tot["cpu"], tot["gl"], tot["gpu_jobs"], tot["gpu_ns"] = (int(x) for x in m.groups())
        m = re.match(r"gm2: v3d-frames counted=\d+ kicks=(\d+) waits=(\d+)", line)
        if m:
            tot["kicks"], tot["waits"] = int(m.group(1)), int(m.group(2))
    return calls, waits, gpu, tot


def main():
    a, b, n = parse(sys.argv[1]), parse(sys.argv[2]), int(sys.argv[3])
    ta, tb = a[3], b[3]
    d = {k: tb.get(k, 0) - ta.get(k, 0) for k in tb}
    print(f"over {n} frames: wall {d['wall'] / n / 1e3:.1f} us a frame, cpu {d['cpu'] / n / 1e3:.1f}, "
          f"in GL/EGL calls {d['gl'] / n / 1e3:.1f}; jobs {d.get('kicks', 0) / n:.2f} a frame, "
          f"CPU waits for a job {d.get('waits', 0) / n:.2f}; TFU model jobs {d['gpu_jobs'] / n:.2f} a frame, "
          f"model host time {d['gpu_ns'] / n / 1e3:.1f} us a frame (GPU, not CPU)")
    rows = []
    for name, (c, w, u) in b[0].items():
        c0, w0, u0 = a[0].get(name, (0, 0, 0))
        if c - c0:
            dw = b[1].get(name, 0) - a[1].get(name, 0)
            dg = b[2].get(name, (0, 0))[0] - a[2].get(name, (0, 0))[0]
            rows.append(((w - w0) / n, name, (c - c0) / n, (u - u0) / n, dw / n, dg / n))
    print("entry point | calls a frame | wall us a frame | cpu us a frame | v3d waits a frame | TFU jobs a frame")
    for w, name, c, u, dw, dg in sorted(rows, reverse=True):
        print(f"{name} | {c:.2f} | {w / 1e3:.1f} | {u / 1e3:.1f} | {dw:.2f} | {dg:.2f}")


if __name__ == "__main__":
    main()
