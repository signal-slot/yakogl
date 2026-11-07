#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Per-frame figures of one glmark2 host run of terrain on the V3D build
(tools/hostharness/glmark2/run.sh with GM2_PROFILE=1, one entry), as
docs/notes/pi4-slow-terrain.md quotes them: per GL entry point the calls and
wall/cpu microseconds a counted frame, the back end's jobs and CPU waits a
frame (overall and per entry point), the TFU model's jobs a frame, the draw
steps, and the V3D back end's own counter lines (the frame-1 copy and TFU
mipmap counters, the render-image and tiled-copy lines).

usage: summarize.py <run.sh out dir>
"""
import pathlib
import re
import sys

STEPS = ["GlProgram", "GlAttribs", "GlTextures", "GlUniforms", "GlState"]


def triple(s):
    c, w, u = (int(x) for x in s.split(","))
    return c, w, u


def main():
    out = pathlib.Path(sys.argv[1])
    logs = sorted((out / "logs" / "v3d").glob("*.log"))
    if not logs:
        sys.exit(f"summarize.py: no logs in {out}/logs/v3d")
    text = logs[0].read_text(errors="replace").splitlines()
    counted = None
    for line in text:
        m = re.match(r"gm2: gl-frames swaps=(\d+) counted=(\d+) wall_ns=(\d+) cpu_ns=(\d+) gl_wall_ns=(\d+)"
                     r" gpu_jobs=(\d+) gpu_ns=(\d+)", line)
        if m:
            swaps, counted, wall, cpu, glw, gj, gns = (int(x) for x in m.groups())
    if not counted:
        sys.exit("summarize.py: no gm2: gl-frames line (GM2_PROFILE=1 needed)")
    n = counted
    settings = (out / "v3d-settings.txt").read_text().split() if (out / "v3d-settings.txt").exists() else []
    print(f"run {out.name}: {' '.join(settings)}; counted frames {n} (frames 2..{swaps})")
    print(f"frame: wall {wall / n / 1e3:.1f} us, cpu {cpu / n / 1e3:.1f} us, in GL/EGL calls {glw / n / 1e3:.1f} us; "
          f"TFU model jobs {gj / n:.2f} a frame, model host time {gns / n / 1e3:.1f} us a frame (GPU, not CPU)")
    rows, waits, gpu = [], {}, {}
    for line in text:
        m = re.match(r"gm2: gl-time (\S+) setup=(\S+) frames=(\S+) teardown=(\S+)", line)
        if m:
            c, w, u = triple(m.group(3))
            if c:
                rows.append((w, m.group(1), c, u))
        m = re.match(r"gm2: v3d-waits (\S+) setup=\d+ frames=(\d+)", line)
        if m:
            waits[m.group(1)] = int(m.group(2))
        m = re.match(r"gm2: gpu-time (\S+) setup=\S+ frames=(\d+),(\d+)", line)
        if m:
            gpu[m.group(1)] = (int(m.group(2)), int(m.group(3)))
        m = re.match(r"gm2: v3d-frames counted=(\d+) kicks=(\d+) waits=(\d+)", line)
        if m:
            print(f"v3d: jobs {int(m.group(2)) / n:.2f} a frame, CPU waits for a job {int(m.group(3)) / n:.2f} a frame")
        m = re.match(r"gm2: draw-steps counted=(\d+) scopes=(\d+) untimed=(\d+) ns=(\S+)", line)
        if m:
            ns = [int(x) for x in m.group(4).split(",")]
            parts = ", ".join(f"{s} {v / n / 1e3:.1f}" for s, v in zip(STEPS, ns))
            print(f"draw steps: {int(m.group(2)) / n:.1f} draws a frame; us a frame: {parts}; "
                  f"sum {sum(ns) / n / 1e3:.1f}")
    print("entry point | calls a frame | wall us a frame | cpu us a frame | v3d waits a frame | TFU jobs a frame")
    for w, name, c, u in sorted(rows, reverse=True):
        wt = waits.get(name, 0) / n
        gj = gpu.get(name, (0, 0))[0] / n
        print(f"{name} | {c / n:.2f} | {w / n / 1e3:.1f} | {u / n / 1e3:.1f} | {wt:.2f} | {gj:.2f}")
    for line in text:
        if line.startswith(("yakogl/v3d:", "osal(linux): v3d")) or "yakogl/v3d:" in line:
            print(line[line.index("yakogl/v3d:"):] if "yakogl/v3d:" in line else line)


if __name__ == "__main__":
    main()
