#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Cut the frame descriptors out of one zs_seq.py run (tools/mali/pan_zs_ref.sh).

    cut.py <name>.trace <name>.stdout <trace label> > <name>.txt

<name>.trace is the pandecode file of the run (PAN_MESA_DEBUG=trace,dump:
each submit's command stream, decoded, then the memory of every buffer),
<name>.stdout zs_seq.py's own lines (the GL sequence), <trace label> the
name the excerpt gives the trace in its header. Prints a header (`#`
lines: the dEQP case, the GL calls as zs_seq.py printed them, the trace's
line count, the frame count and one summary line a frame) and then, for
frame n (the n-th RUN_FRAGMENT of the trace whose descriptors pandecode
decoded), blocks headed
`## frame<n>-<part> lines a-b` (a-b the lines of the full trace, the
header src/mali/trace.rs reads):

- `draw<k>`: the k-th RUN_IDVS since the frame before, its scissor and its
  Depth/stencil descriptor (the stencil and depth state; a scissored
  glClear Mesa runs as such a draw);
- `fbd`: the decoded Framebuffer descriptor (its Parameters without the
  tiler context), the ZS/CRC extension (CRC, ZS, S) and the colour render
  targets with their clear words;
- `words`: the raw bytes of the same descriptors, from the memory dump of
  the buffer holding the framebuffer descriptor that follows the frame:
  `Buffer: memory_<base> gpu <base>`, then every 16-byte row from the
  descriptor's address to the end of its last render target (0xc0 + 0x40
  per target: parameters, the ZS/CRC extension at +0x80, the targets at
  +0xc0), each row printed (no `*`).

A run that drew no frame (dEQP's NotSupported) has the header only. Exits
1 when a decoded frame has no dumped descriptor bytes.
"""
import re
import sys

CS = re.compile(r"^  (?:[0-9a-f]{2} ){8}")
ROW = re.compile(r"^([0-9A-Fa-f]{6})  ((?:[0-9A-Fa-f]{2} ){16})\s*$")


def dumps(lines):
    """[(line index, base va, {offset: (line index, 16 bytes)}, last line)]
    of every memory block, '*' rows expanded."""
    out = []
    i = 0
    while i < len(lines):
        m = re.match(r"^Buffer: memory_([0-9a-f]+) gpu ([0-9a-f]+)", lines[i])
        if not m:
            i += 1
            continue
        start, base, rows = i, int(m.group(2), 16), {}
        prev, star = None, False
        i += 1
        while i < len(lines) and not lines[i].startswith("Buffer:") and not CS.match(lines[i]):
            r = ROW.match(lines[i])
            if r:
                off = int(r.group(1), 16)
                if star and prev is not None:
                    for o in range(prev[0] + 16, off, 16):
                        rows[o] = (prev[1], prev[2])
                rows[off] = (i, r.group(2).split())
                prev, star = (off, i, r.group(2).split()), False
            elif lines[i].strip() == "*":
                star = True
            i += 1
        out.append((start, base, rows, star and prev, i - 1))
    return out


def field(lines, start, name):
    """The value of the first `name: value` line at or after a line that
    starts with `start` (stripped)."""
    on = start is None
    for l in lines:
        t = l.strip()
        if not on:
            on = t.startswith(start)
            continue
        if t.startswith(name + ":"):
            return t[len(name) + 1:].strip()
    return "?"


def frame_summary(n, va, draws, fbd, ext):
    """One line naming what a frame's descriptors say about its targets."""
    pre = "pre-frame %s / %s" % (field(fbd, None, "Pre Frame 0"), field(fbd, None, "Pre Frame 1"))
    z = "Z %s write %s clear %s" % (field(fbd, None, "Z Internal Format"), field(fbd, None, "Z Write Enable"),
                                    field(fbd, None, "Z Clear"))
    s = "S write %s clear %s" % (field(fbd, None, "S Write Enable"), field(fbd, None, "S Clear"))
    zs = "ZS plane %s at %s" % (field(ext, "ZS:", "Write Format"), field(ext, "ZS:", "Base"))
    sp = "S plane %s at %s" % (field(ext, "S:", "Write Format"), field(ext, "S:", "Base"))
    rt = "RT0 %s at %s clear %s" % (field(ext, "Color Render Targets", "Writeback Format"),
                                    field(ext, "Writeback buffer:", "Base"), field(ext, "Clear:", "Color 0"))
    return "frame%d: FBD %x, %d draw(s); %s; %s; %s; %s; %s; %s" % (n, va, draws, pre, z, s, zs, sp, rt)


def block(lines, a, b):
    while b > a and not lines[b].strip():
        b -= 1
    return a, b


def main():
    if len(sys.argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    raw = open(sys.argv[1], "rb").read()
    lines = raw.decode("utf-8", "replace").split("\n")
    seq = [l for l in open(sys.argv[2]).read().split("\n") if l.startswith("zs_seq: ")]
    mem = dumps(lines)
    frames, draws = [], []
    i = 0
    while i < len(lines):
        line = lines[i]
        if "RUN_IDVS" in line and CS.match(line):
            j = i + 1
            while j < len(lines) and not CS.match(lines[j]) and not lines[j].startswith("Buffer:"):
                j += 1
            draws.append((i, j))
            i = j
            continue
        if "RUN_FRAGMENT" in line and line.startswith("  ") and not line.startswith("    ") \
                and i + 1 < len(lines) and lines[i + 1].startswith("  Scissor"):
            j = i + 1
            while j < len(lines) and not CS.match(lines[j]):
                j += 1
            frames.append((i, j, draws))
            draws = []
            i = j
            continue
        i += 1
    out, summary = [], []
    for n, (a, b, fdraws) in enumerate(frames, 1):
        for k, (da, db) in enumerate(fdraws, 1):
            parts = []
            for want, stop in (("  Scissor", "  Low depth clamp"), ("  Depth/stencil", "  Primitive flags")):
                s = next((x for x in range(da, db) if lines[x].startswith(want)), None)
                if s is None:
                    continue
                e = next((x for x in range(s + 1, db) if lines[x].startswith(stop)), db)
                parts.append((s, e - 1))
            out.append("## frame%d-draw%d lines %d-%d" % (n, k, da + 1, da + 1))
            out.append(lines[da])
            for s, e in parts:
                out.append("## frame%d-draw%d lines %d-%d" % (n, k, s + 1, e + 1))
                out += lines[s:e + 1]
        fb = next(x for x in range(a, b) if lines[x].startswith("  Framebuffer @"))
        va = int(lines[fb].split("@")[1].rstrip(":"), 16)
        crc = next(x for x in range(fb, b) if lines[x].startswith("  CRC:"))
        heap = next((x for x in range(fb, crc) if lines[x].startswith("    Tiler Heap:")), crc)
        count = int(next(lines[x] for x in range(fb, heap) if "Render Target Count:" in lines[x]).split(":")[1])
        s, e = block(lines, fb, heap - 1)
        out.append("## frame%d-fbd lines %d-%d" % (n, s + 1, e + 1))
        out += lines[s:e + 1]
        s, e = block(lines, crc, b - 1)
        out.append("## frame%d-fbd lines %d-%d" % (n, s + 1, e + 1))
        out += lines[s:e + 1]
        summary.append(frame_summary(n, va, len(fdraws), lines[fb:heap], lines[crc:b]))
        size = 0xC0 + 0x40 * count
        dump = next((d for d in mem if d[0] > b and d[1] <= va and va - d[1] in d[2]), None)
        if dump is None:
            print("cut: %s: frame %d: no dump of %x" % (sys.argv[1], n, va), file=sys.stderr)
            return 1
        start, base, rows, _, _ = dump
        offs = range(va - base, va - base + size, 16)
        if any(o not in rows for o in offs):
            print("cut: %s: frame %d: the dump of %x ends early" % (sys.argv[1], n, va), file=sys.stderr)
            return 1
        first, last = min(rows[o][0] for o in offs), max(rows[o][0] for o in offs)
        out.append("## frame%d-words lines %d-%d" % (n, first + 1, last + 1))
        out.append("Buffer: memory_%x gpu %x" % (base, base))
        out += ["%06X  %s " % (o, " ".join(rows[o][1])) for o in offs]
    head = ["# SPDX-License-Identifier: Apache-2.0",
            "# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.",
            "#",
            "# The frames Mesa 26.2.1 (gallium panfrost on drm-shim, GPU_ID 0xa8670000,",
            "# PAN_MESA_DEBUG=linear,nocrc,trace,dump) made for one dEQP-GLES2 case,",
            "# regenerated by tools/mali/pan_zs_ref.sh (zs_seq.py, then cut.py). Do not edit.",
            "#", "# The GL sequence (zs_seq.py's lines; the case's calls in order):"]
    head += ["#   " + l[len("zs_seq: "):] for l in seq]
    # no hash of the trace: its host CPU addresses (pandecode's `Shader 0x...`
    # lines, pointers in some dumped buffers) change from run to run
    head += ["#", "# Trace: %s, %d lines" % (sys.argv[3], len(lines)), "# Frames: %d" % len(frames)]
    head += ["#   " + l for l in summary]
    print("\n".join(head + out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
