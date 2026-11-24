#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Cut a case's descriptors out of one vs_cube_edge_draw.py run
(tools/mali/testdata/vs_cube_edge_ref.sh).

    vs_cube_edge_cut.py <full trace> <title> <draw.out> <draw trace> > <case>.<ctx>.desc

<full trace> is the run's pandecode file (PAN_MESA_DEBUG=trace,dump: each
submit's command stream, decoded, then the memory of every buffer). The
case's draw is its last RUN_IDVS; the lines from there up to the memory
dump that follows are written to <draw trace> (what pan_ref_diff.py
extract --draw reads).

Prints:

- `# <title>`, then `# ` and each `vs_cube_edge_draw:` line of <draw.out>
  (GL_VERSION, the uniforms and attributes);
- per resource table of the draw (Position, Varying, Fragment), the
  entries that hold a sampler or a texture, as pandecode decodes them
  (the layout of pan_tex_ref.sh's .desc files);
- `words`: each of those descriptors' 32 bytes as eight little-endian
  32-bit words (word 0 first), from the dump of the memory that follows
  the draw, and for a texture its plane descriptors (Levels x 6 faces x
  array size, from `Surfaces`, eight words each; Mesa lays a cube's out
  face by face, each face's levels in turn).

The GPU addresses are the shim's: the same on every run of one build.
Exits 1 when a decoded descriptor has no dumped bytes.
"""
import re
import sys

ROW = re.compile(r"^([0-9A-Fa-f]{6})  ((?:[0-9A-Fa-f]{2} ){1,16})\s*$")
TABLE = re.compile(r"^  (\w+) resources resource table @([0-9a-f]+)$")
ENTRY = re.compile(r"^      Entry (\d+) @[0-9a-f]+:$")
DESC = re.compile(r"^    (Sampler|Texture) @([0-9a-f]+)")


def dumps(lines, start):
    """[(base va, bytearray)] of every memory block at or after line `start`,
    '*' rows expanded."""
    out = []
    i = start
    while i < len(lines):
        m = re.match(r"^Buffer: \S+ gpu ([0-9a-f]+)", lines[i])
        if not m:
            i += 1
            continue
        base, data, prev, star = int(m.group(1), 16), bytearray(), None, False
        i += 1
        while i < len(lines) and not lines[i].startswith("Buffer:"):
            r = ROW.match(lines[i])
            if r:
                off = int(r.group(1), 16)
                row = bytes(int(b, 16) for b in r.group(2).split())
                if star and prev is not None:
                    while len(data) < off:
                        data += prev
                if len(data) != off:
                    raise SystemExit("cut: a dump row at %x is not in order" % (base + off))
                data += row
                prev, star = row, False
            elif lines[i].strip() == "*":
                star = True
            elif lines[i].strip() and not lines[i].startswith(" "):
                break
            i += 1
        out.append((base, data))
    return out


def read(blocks, va, n):
    for base, data in blocks:
        if base <= va and va + n <= base + len(data):
            return data[va - base:va - base + n]
    raise SystemExit("cut: no dumped bytes at %x" % va)


def words(b):
    return " ".join("%08x" % int.from_bytes(b[k:k + 4], "little") for k in range(0, len(b), 4))


def main():
    trace, title, draw_out, draw_trace = sys.argv[1:]
    lines = open(trace, errors="replace").read().splitlines()
    idvs = [k for k, l in enumerate(lines) if " RUN_IDVS" in l]
    if not idvs:
        raise SystemExit("cut: %s has no draw" % trace)
    first = idvs[-1]
    end = next((k for k in range(first, len(lines)) if lines[k].startswith("Buffer:")), len(lines))
    with open(draw_trace, "w") as f:
        f.write("\n".join(lines[first:end]) + "\n")
    blocks = dumps(lines, end)

    out = ["# " + title]
    out += ["# " + l for l in open(draw_out).read().splitlines() if l.startswith("vs_cube_edge_draw:")]
    raw = ["words"]
    seen, i = set(), first
    while i < end:
        m = TABLE.match(lines[i])
        if not m:
            i += 1
            continue
        if m.group(1) in seen:
            break
        seen.add(m.group(1))
        out.append("%s resource table @%s" % (m.group(1), m.group(2)))
        i += 1
        while i < end and (lines[i].startswith("      ") or not lines[i].strip()):
            e = ENTRY.match(lines[i])
            i += 1
            if not e:
                continue
            block = []
            while i < end and lines[i].startswith("        "):
                block.append(lines[i][6:])
                i += 1
            descs = [DESC.match(b) for b in block]
            descs = [d for d in descs if d]
            if not descs:
                continue
            out.append("  Entry %s:" % e.group(1))
            out += [b for b in block if b.strip()]
            for d in descs:
                va = int(d.group(2), 16)
                raw.append("  %s entry %s %s @%x: %s" % (m.group(1), e.group(1), d.group(1), va, words(read(blocks, va, 32))))
                if d.group(1) != "Texture":
                    continue
                f = {}
                for b in block:
                    kv = re.match(r"^\s+(Surfaces|Levels|Array size|Dimension): (\S+)", b)
                    if kv and kv.group(1) not in f:
                        f[kv.group(1)] = kv.group(2)
                faces = 6 if f.get("Dimension") == "Cube" else 1
                count = int(f["Levels"]) * faces * int(f.get("Array size", "1"))
                surf = int(f["Surfaces"], 16)
                raw.append("    planes: %d (Levels %s x %d faces x array size %s) @%x" % (
                    count, f["Levels"], faces, f.get("Array size", "1"), surf))
                for p in range(count):
                    raw.append("    plane %d: %s" % (p, words(read(blocks, surf + 32 * p, 32))))
    if not seen:
        raise SystemExit("cut: no resource table in the draw")
    print("\n".join(out + raw))


if __name__ == "__main__":
    main()
