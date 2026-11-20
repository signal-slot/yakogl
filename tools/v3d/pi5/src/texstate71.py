#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Decode the V3D 7.1 texture shader state and sampler state records a
Mesa CLIF dump's fragment shaders sample through (the P10 references
tools/v3d/pi5/gl_*.clif of tools/v3d/pi5/src/gl_shapes.py).

    tools/v3d/pi5/src/texstate71.py <file.clif> [job]

Mesa's CLIF dump decodes control lists and shader records but prints the
buffers that hold the uniforms and the texture and sampler records as raw
words (`@format binary`). For every job (one per `@wait_render_all_cores`,
numbered from 1; all of them when no job is given) and every GL shader
record in it, this reads the fragment shader's uniform stream (from its
`Fragment Shader Uniforms Address` up to the next uniform stream of the
same buffer, or the end of the words dumped) and takes each pair of words
(P0, P1) where P0 with its low 4 bits cleared is a 32-byte aligned address
and P1 with its low 3 bits cleared an 8-byte aligned one, both inside words
the dump has: TMU Config Parameter 0 (texture state address, return words)
and Parameter 1 (sampler state address, flags). It prints both records
field by field with the 7.1 layouts of the vendored Mesa packet XML
(third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml: "Texture Shader
State" min_ver 71, "Sampler State"), enum values by name.

The pair rule is a reading aid, not a decoder of the uniform types (the dump
does not carry them): a uniform that happens to look like two addresses
would be printed too. Exits 1 when a job with a GL shader record yields no
pair.
"""
import os
import re
import sys
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
XML = os.path.join(HERE, "..", "..", "..", "..", "third_party", "mesa-26.2.1", "src",
                   "broadcom", "cle", "v3d_packet.xml")


def load_xml():
    root = ET.parse(os.path.normpath(XML)).getroot()
    enums = {}
    for e in root.iter("enum"):
        enums[e.get("name")] = {int(v.get("value")): v.get("name") for v in e.iter("value")}
    structs = {}
    for s in root.iter("struct"):
        name = s.get("name")
        minv = int(s.get("min_ver", "0"))
        maxv = int(s.get("max_ver", "999"))
        if name not in ("Texture Shader State", "Sampler State") or not minv <= 71 <= maxv:
            continue
        fields = []
        for f in s.iter("field"):
            values = {int(v.get("value")): v.get("name") for v in f.iter("value")}
            fields.append((f.get("name"), int(f.get("start")), int(f.get("size")),
                           f.get("type"), values))
        structs[name] = fields
    return enums, structs


def decode(words, fields, enums):
    """words: the record's 32-bit words, little-endian; one line a field,
    highest start bit first, as the XML lists them."""
    bits = 0
    for i, w in enumerate(words):
        bits |= w << (32 * i)
    out = []
    # the XML names the values of Swizzle A only, and gives Texture type as a
    # plain uint; both are printed with the names Mesa's packer uses
    swizzles = next((f[4] for f in fields if f[0] == "Swizzle A"), {})
    for name, start, size, typ, values in fields:
        if name.startswith("Swizzle "):
            values = swizzles
        elif name == "Texture type":
            values = enums.get("Texture Data Formats", {})
        v = (bits >> start) & ((1 << size) - 1)
        if typ == "u4.8":
            text = "%g" % (v / 256.0)
        elif typ == "s8.8":
            text = "%g" % ((v - (1 << 16) if v & 0x8000 else v) / 256.0)
        elif typ == "address":
            text = "0x%08x" % v
        elif typ == "bool":
            text = "1" if v else "0"
        else:
            text = str(v)
        name_of = values.get(v) or enums.get(typ, {}).get(v)
        if name_of and typ != "bool":
            text += " (%s)" % name_of
        out.append("      %s: %s" % (name, text))
    return out


def parse_jobs(path):
    """Each job: (memory {address: word} of the words dumped as binary,
    [(fs uniforms address, all uniform addresses of the job)])."""
    jobs, mem, recs, unif = [], {}, [], []
    cur_base = None
    addr = None
    with open(path) as fh:
        for raw in fh:
            if not raw.startswith("MESA: info: "):
                continue
            line = raw[len("MESA: info: "):].rstrip("\n")
            if line == "@wait_render_all_cores":
                jobs.append((mem, recs, unif))
                mem, recs, unif = {}, [], []
                continue
            m = re.match(r"@format binary /\* \[(\w+)_0x([0-9a-f]+)\+0x([0-9a-f]+)\] \*/", line)
            if m:
                cur_base = int(m.group(2), 16)
                addr = cur_base + int(m.group(3), 16)
                continue
            # trailing zero bytes are dumped as a blank range
            m = re.match(r"@format blank \d+ /\* \[\w+_0x([0-9a-f]+)\+0x([0-9a-f]+)\.\.0x([0-9a-f]+)\]",
                         line)
            if m:
                base = int(m.group(1), 16)
                lo, hi = base + int(m.group(2), 16), base + int(m.group(3), 16)
                for a in range(lo & ~3, hi + 1, 4):
                    mem.setdefault(a, 0)
                addr = None
                continue
            if line.startswith("@"):
                addr = None
                continue
            if addr is not None and re.match(r"^(0x[0-9a-f]{8} ?)+$", line.strip()):
                for w in line.split():
                    mem[addr] = int(w, 16)
                    addr += 4
                continue
            m = re.search(r"(Fragment|Vertex|Coordinate) Shader Uniforms Address: \*/ "
                          r"\[\w+\+0x[0-9a-f]+\] /\* 0x([0-9a-f]+) \*/", line)
            if m:
                a = int(m.group(2), 16)
                unif.append(a)
                if m.group(1) == "Fragment":
                    recs.append(a)
    return jobs


def main():
    if len(sys.argv) not in (2, 3):
        print(__doc__, file=sys.stderr)
        return 2
    enums, structs = load_xml()
    tex_fields = structs["Texture Shader State"]
    smp_fields = structs["Sampler State"]
    jobs = parse_jobs(sys.argv[1])
    want = int(sys.argv[2]) if len(sys.argv) == 3 else None
    status = 0
    for n, (mem, recs, unif) in enumerate(jobs, 1):
        if want is not None and n != want:
            continue
        for fs in recs:
            later = [a for a in unif if a > fs]
            end = min(later) if later else None
            stream = []
            a = fs
            while a in mem and (end is None or a < end):
                stream.append(mem[a])
                a += 4
            print("job %d: fragment shader uniforms 0x%08x, %d words: %s" %
                  (n, fs, len(stream), " ".join("0x%08x" % w for w in stream)))
            pairs = 0
            for i in range(len(stream) - 1):
                p0, p1 = stream[i], stream[i + 1]
                ta, sa = p0 & ~0xf, p1 & ~0x7
                if ta % 32 or not all(ta + 4 * k in mem for k in range(6)):
                    continue
                if not all(sa + 4 * k in mem for k in range(6)):
                    continue
                pairs += 1
                print("  uniform %d: TMU Config Parameter 0 0x%08x: texture state 0x%08x, "
                      "return words %d" % (i, p0, ta, p0 & 0xf))
                print("  uniform %d: TMU Config Parameter 1 0x%08x: sampler state 0x%08x, "
                      "per-pixel mask %d, unnormalized %d, output 32-bit %d" %
                      (i + 1, p1, sa, (p1 >> 2) & 1, (p1 >> 1) & 1, p1 & 1))
                tw = [mem[ta + 4 * k] for k in range(6)]
                sw = [mem[sa + 4 * k] for k in range(6)]
                print("    Texture Shader State (7.1) at 0x%08x: %s" %
                      (ta, " ".join("0x%08x" % w for w in tw)))
                print("\n".join(decode(tw, tex_fields, enums)))
                print("    Sampler State at 0x%08x: %s" % (sa, " ".join("0x%08x" % w for w in sw)))
                print("\n".join(decode(sw, smp_fields, enums)))
            if pairs == 0:
                print("  no TMU config pair")
                status = 1
    return status


if __name__ == "__main__":
    sys.exit(main())
