#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""clif_shaders71: Mesa's V3D 7.1 triangle shaders out of a V3D_DEBUG=clif dump.

    python3 -I tools/v3d/clif_shaders71.py [--clif F] [--out-dir D] [--check-shim F]

Reads tools/v3d/pi5/triangle.clif (Mesa 26.2.2 on the Raspberry Pi 5, V3D
7.1.10, one Gouraud triangle after a 64x64 clear: tools/v3d/gl_ref.py
`triangle`) and writes, into tools/v3d/tri-shaders71/ (or --out-dir):

  mesa_passthrough_interp_color.json  the shape of
        tools/v3d/tri-shaders/passthrough_interp_color.json (the 4.2 file,
        chain.rs v3d_shc_json): gpu, uniform_names, attributes, fs, vs, cs,
        each stage with its flags, uniforms {contents, data} and code words;
  mesa_passthrough_interp_color.h     the same as C for a Zephyr probe
        (const uint64_t code arrays, uint32_t uniform streams, uniform kinds,
        record values). Both are generated; the header says so.

What the CLIF holds and what is read from it
--------------------------------------------
* Code: the shader buffer (resource_0x1000 in the board's dump) as 32-bit
  words, low word first; the three programs are the spans between the record's
  code addresses (FS, VS, CS in address order) and the end of the data.
  The 0x60 bytes in front of the first program are not code (not read).
* Uniform streams: the binary words of the record's uniform buffer, the FS and
  VS streams start at the record's FS/VS address, the CS stream at its own
  address; each runs to the next stream's start or the end of the data. The FS
  has no uniform: its stream address is the VS's (Mesa points an empty stream at
  the same place) and its code reads none (checked: no ldunif signal).
* Uniform kinds: the CLIF does not name them. A word is taken as Mesa's
  QUNIFORM_VIEWPORT_* when it equals, in this order, the next of the viewport
  values the same CLIF's CLIPPER_XY / CLIPPER_Z packets give (X scale = Y scale
  = half width, 2048.0 = 0x45000000; Z scale and Z offset 0.5), scanning the
  stream front to back; every other word is QUNIFORM_CONSTANT (contents 0, data
  = the word). Contents numbers are the 4.2 file's (2 viewport X scale, 3 Y
  scale, 4 Z offset, 5 Z scale). Both streams this gives are the 4.2 file's
  layout (X, 8 constants, Y, Z scale, Z offset, 4 constants; the CS without Z).
* The shader record: the board's Mesa packs the BCM2712 D0 layout ("GL Shader
  State Record Draw Index", v3d_packet.xml; tools/v3d/pi5/README.md "Differences"
  item 4), but the CLIF decodes it with the plain 7.1 layout, whose flags from
  bit 5 up sit 2 to 4 bits lower. This script puts the decoded plain-layout flags
  back at their plain bit positions (rebuilding the raw flag word, 24 bits) and
  reads that word at the D0 layout; the remaining fields (varyings count, VPM
  sizes, threadability, final-section, addresses) are at the same bits in both
  layouts. The result for this triangle: real pixel centre W (D0 bit 15) set,
  insert primitive ID (bit 18) clear, as the README says. Bits 10, 11 and
  21..23 have no field in the plain layout, so the CLIF shows nothing of them:
  UNSEEN gives them (disable implicit point/line varyings set, the rest clear,
  the shim's values); that is an assumption the CLIF cannot check.
* Not in the CLIF, filled from the sample's identity (the GLSL pair, the 4.2
  file): attribute names a_pos (location 0, record 0) and a_color (location 1,
  record 1); the FS input slots (num varyings 4: VARYING_SLOT_VAR0 = 32,
  components 0..3); vpm_size 65536 (the board's IDENT1, v3d_ident.txt);
  flat/noperspective/centroid flags 0 (the CLIF's ZERO_ALL_* packets);
  threads 4 for a 4-way threadable stage; the VS and CS `single_seg` is the
  record's "start in final thread section".

Cross-check against the drm-shim dump (--check-shim, default
tools/v3d/shim71/triangle.clif, V3D 7.1.7): the FS, VS and CS words, the three
uniform streams, the attribute records and every record field other than the
three D0-shifted flags are EQUAL to the board's 7.1.10 dump, as
tools/v3d/pi5/README.md "Differences" item 5 says; `--check-shim` verifies it
(exit 1 on a difference). The shim's record is the plain 7.1 layout; its
decoded flags are the plain ones, the board's D0 reading of the same flags equals
the shim's plain values at the shifted positions.

Stdlib only.
"""
import argparse
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
SRC = os.path.join(HERE, "pi5", "triangle.clif")
SHIM = os.path.join(HERE, "shim71", "triangle.clif")
OUT = os.path.join(HERE, "tri-shaders71")
BASE = "mesa_passthrough_interp_color"

# Flag bit positions of the record's first dword: plain 7.1 (what the CLIF
# decodes) and D0 (GL Shader State Record Draw Index), by the CLIF's label.
PLAIN = {
    "Point size in shaded vertex data": 0,
    "Enable clipping": 1,
    "Vertex ID read by coordinate shader": 2,
    "Instance ID read by coordinate shader": 3,
    "Base Instance ID read by coordinate shader": 4,
    "Vertex ID read by vertex shader": 5,
    "Instance ID read by vertex shader": 6,
    "Base Instance ID read by vertex shader": 7,
    "Fragment shader does Z writes": 8,
    "Turn off early-z test": 9,
    "Fragment shader uses real pixel centre W in addition to centroid W2": 12,
    "Enable Sample Rate Shading": 13,
    "Any shader reads hardware-written Primitive ID": 14,
    "Insert Primitive ID as first varying to fragment shader": 15,
    "Turn off scoreboard": 16,
    "Do scoreboard wait on first thread switch": 17,
    "Disable implicit point/line varyings": 18,
    "No prim pack": 19,
    "Never defer FEP depth writes": 20,
}
D0 = {
    "Point size in shaded vertex data": 0,
    "Enable clipping": 1,
    "Vertex ID read by coordinate shader": 2,
    "Instance ID read by coordinate shader": 3,
    "Base Instance ID read by coordinate shader": 4,
    "cs_basevertex": 5,
    "cs_drawindex": 6,
    "Vertex ID read by vertex shader": 7,
    "Instance ID read by vertex shader": 8,
    "Base Instance ID read by vertex shader": 9,
    "vs_basevertex": 10,
    "vs_drawindex": 11,
    "Fragment shader does Z writes": 12,
    "Turn off early-z test": 13,
    "Fragment shader uses real pixel centre W in addition to centroid W2": 15,
    "Enable Sample Rate Shading": 16,
    "Any shader reads hardware-written Primitive ID": 17,
    "Insert Primitive ID as first varying to fragment shader": 18,
    "Turn off scoreboard": 19,
    "Do scoreboard wait on first thread switch": 20,
    "Disable implicit point/line varyings": 21,
    "No prim pack": 22,
    "Never defer FEP depth writes": 23,
}

# D0 flag bits the CLIF's plain-layout decode has no field for (bits 10, 11 and
# 21..23): vs_basevertex, vs_drawindex, Disable implicit point/line varyings, No
# prim pack, Never defer FEP depth writes. Values assumed: the two VS-reads
# clear (the pass-through VS reads neither); Disable implicit set and the other
# two clear, the shim's values (README "Differences" 4: the board's flags read
# at D0 are the shim's). --check-shim compares those three with the shim's.
UNSEEN = {
    "vs_basevertex": 0,
    "vs_drawindex": 0,
    "Disable implicit point/line varyings": 1,
    "No prim pack": 0,
    "Never defer FEP depth writes": 0,
}

# Mesa's uniform contents numbers (as the 4.2 file).
C_CONSTANT, C_VP_XS, C_VP_YS, C_VP_ZO, C_VP_ZS = 0, 2, 3, 4, 5


def die(msg):
    sys.stderr.write("clif_shaders71: %s\n" % msg)
    sys.exit(1)


def parse(path):
    """-> dict: bufs {name: {offset: word}}, rec {label: text}, attrs [dict],
    packets {name: {label: text}} (first of each)."""
    lines = []
    with open(path, encoding="utf-8") as f:
        for ln in f:
            ln = ln.rstrip("\n")
            ln = re.sub(r"^MESA: info: ?", "", ln)
            lines.append(ln)
    bufs, rec, attrs, packets = {}, {}, [], {}
    cur_pkt = None
    i = 0
    mode = None  # None / 'binary' / 'rec' / 'attr'
    buf = None
    off = 0
    while i < len(lines):
        ln = lines[i]
        i += 1
        m = re.match(r"@buffer (\S+)", ln)
        if m:
            buf = m.group(1)
            bufs.setdefault(buf, {})
            mode = None
            continue
        m = re.match(r"@format binary /\* \[(\S+)\+0x([0-9a-f]+)\] \*/", ln)
        if m:
            mode, off = "binary", int(m.group(2), 16)
            continue
        if ln.startswith("@format shadrec_gl_main"):
            mode = "rec"
            continue
        if ln.startswith("@format shadrec_gl_attr"):
            mode = "attr"
            attrs.append({})
            continue
        if ln.startswith("@"):
            mode = None
            cur_pkt = None
            continue
        if mode == "binary":
            ws = ln.split()
            if ws and all(re.fullmatch(r"0x[0-9a-f]{8}", w) for w in ws):
                for w in ws:
                    bufs[buf][off] = int(w, 16)
                    off += 4
                continue
            mode = None
        if mode in ("rec", "attr"):
            m = re.match(r"\s*/\*\s*(.*?):\s*\*/\s*(.*)$", ln)
            if m:
                (rec if mode == "rec" else attrs[-1])[m.group(1).strip()] = m.group(2)
                continue
            mode = None
        # control-list packet fields (for the viewport values)
        m = re.match(r"([A-Z][A-Z0-9_]+)$", ln)
        if m:
            cur_pkt = m.group(1)
            packets.setdefault(cur_pkt, {})
            continue
        m = re.match(r"\s*/\*\s*(.*?):\s*\*/\s*(.*)$", ln)
        if m and cur_pkt:
            packets[cur_pkt].setdefault(m.group(1).strip(), m.group(2))
    return {"bufs": bufs, "rec": rec, "attrs": attrs, "packets": packets}


def num(s):
    return int(s.split()[0], 0)


def addr(s):
    m = re.match(r"\[(\S+)\+0x([0-9a-f]+)\]", s)
    if not m:
        die("not an address: %r" % s)
    return m.group(1), int(m.group(2), 16)


def f32(w):
    import struct
    return struct.unpack("<f", struct.pack("<I", w))[0]


def f32w(x):
    import struct
    return struct.unpack("<I", struct.pack("<f", x))[0]


def words(bufs, name, lo, hi):
    out = []
    for o in range(lo, hi, 4):
        if o not in bufs[name]:
            die("%s+0x%x is not in the dump's binary data" % (name, o))
        out.append(bufs[name][o])
    return out


def sig(w):
    return (w >> 53) & 0x1F


SIG_THRSW = 1


def thread_end_ok(code):
    """A program ends its thread by a thrsw with two delay slots after it
    (the last thrsw is the third word from the end), and no other thrsw after
    it."""
    sw = [i for i, w in enumerate(code) if sig(w) & 1 and sig(w) in (1, 3)]
    return bool(sw) and sw[-1] == len(code) - 3


def extract(clif, d0=True):
    d = parse(clif)
    r, bufs = d["rec"], d["bufs"]
    # Rebuild the raw flag word from the plain-layout reading, read it at D0.
    raw = 0
    for k, b in PLAIN.items():
        if k not in r:
            die("record field %r missing" % k)
        if num(r[k]):
            raw |= 1 << b
    if raw >> 21:
        die("unexpected raw flag bits above 20: 0x%x" % raw)
    if d0:
        # D0 bits the plain decode cannot show (10, 11, 21..23) are not read
        # from the CLIF: UNSEEN gives them (see the header).
        for k, v in UNSEEN.items():
            raw |= v << D0[k]
        flags = {k: (raw >> b) & 1 for k, b in D0.items()}
    else:  # a plain 7.1 record (the shim's): the decoded flags are the flags
        flags = {k: (num(r[k]) if k in PLAIN else 0) for k in D0}

    stages = {}
    for st, pre in (("fs", "Fragment Shader"), ("vs", "Vertex Shader"), ("cs", "Coordinate Shader")):
        cb, co = addr(r[pre + " Code Address"])
        ub, uo = addr(r[pre + " Uniforms Address"])
        stages[st] = dict(
            cbuf=cb, coff=co, ubuf=ub, uoff=uo,
            fourway=num(r[pre + " 4-way threadable"]),
            final=num(r[pre + " start in final thread section"]),
            nan=num(r[pre + " Propagate NaNs"]),
        )
    cbuf = stages["fs"]["cbuf"]
    if any(s["cbuf"] != cbuf for s in stages.values()):
        die("the three programs are not in one buffer")
    if cbuf not in bufs:
        die("code buffer %s has no binary data" % cbuf)
    cend = max(bufs[cbuf]) + 4
    starts = sorted(set(s["coff"] for s in stages.values()))
    for st, s in stages.items():
        nxt = [a for a in starts if a > s["coff"]]
        s["code"] = words(bufs, cbuf, s["coff"], nxt[0] if nxt else cend)
        if len(s["code"]) % 2:
            die("%s code is an odd number of words" % st)
        s["code"] = [s["code"][i] | (s["code"][i + 1] << 32) for i in range(0, len(s["code"]), 2)]
    ubuf = stages["vs"]["ubuf"]
    uend = max(bufs[ubuf]) + 4
    ustarts = sorted(set(s["uoff"] for s in stages.values()))
    # viewport values from the CLIF's own packets
    xy = d["packets"]["CLIPPER_XY"]
    z = d["packets"]["CLIPPER_Z"]
    vp = [f32w(float(xy["Viewport Half-Width in 1/64th of pixel"])),
          f32w(float(xy["Viewport Half-Height in 1/64th of pixel"])),
          f32w(float(z["Viewport Z Scale (Zc to Zs)"])),
          f32w(float(z["Viewport Z Offset (Zc to Zs)"]))]
    kinds = [C_VP_XS, C_VP_YS, C_VP_ZS, C_VP_ZO]
    for st, s in stages.items():
        nxt = [a for a in ustarts if a > s["uoff"]]
        w = words(bufs, ubuf, s["uoff"], nxt[0] if nxt else uend)
        if st == "fs":
            # no program reads one: its stream is the VS's address, empty
            w = []
        un, k = [], 0
        for x in w:
            if k < 4 and x == vp[k]:
                un.append((kinds[k], 0))
                k += 1
            else:
                un.append((C_CONSTANT, x))
        s["uwords"], s["uniforms"] = w, un
        s["thrend"] = thread_end_ok(s["code"])
        s["ldunif"] = sum(1 for c in s["code"] if sig(c) in (2, 3))
    if stages["fs"]["ldunif"]:
        die("the fragment shader reads uniforms; the stream is not empty")

    # attribute records
    at = []
    for a in d["attrs"]:
        at.append(dict(
            addr=addr(a["Address"]), vec=num(a["Vec size"]), type=num(a["Type"]),
            cs_n=num(a["Number of values read by Coordinate shader"]),
            vs_n=num(a["Number of values read by Vertex shader"]),
            stride=num(a["Stride"]), maxidx=num(a["Maximum Index"]),
            div=num(a["Instance Divisor"])))
    vcm = num(d["packets"]["VCM_CACHE_SIZE"]["Number of 16-vertex batches for binning"])
    rec = dict(
        flags=flags, raw_flags=raw,
        varyings=num(r["Number of varyings in Fragment Shader"]),
        cs_out=num(r["Coordinate Shader output VPM segment size"]),
        cs_in=num(r["Coordinate Shader input VPM segment size"]),
        vs_out=num(r["Vertex Shader output VPM segment size"]),
        vs_in=num(r["Vertex Shader input VPM segment size"]),
        cs_min_out=num(r["Min Coord Shader output segments required in play in addition to VCM cache size"]),
        cs_min_in=num(r["Min Coord Shader input segments required in play"]),
        vs_min_out=num(r["Min Vertex Shader output segments required in play in addition to VCM cache size"]),
        vs_min_in=num(r["Min Vertex Shader input segments required in play"]),
        vcm=vcm,
    )
    return dict(stages=stages, rec=rec, attrs=at, uni_buf_base=(ubuf, ustarts))


def hx(v, n=8):
    return "0x%0*x" % (n, v)


def vattr(at, key):
    v = [0] * 16
    for i, a in enumerate(at):
        v[i] = a[key]
    return v


def b(x):
    return "true" if x else "false"


def make_json(e):
    st, rc, at = e["stages"], e["rec"], e["attrs"]
    fl = rc["flags"]
    o = []
    o.append("{")
    o.append('  "gpu": { "ver": 71, "vpm_size": 65536, "vpm_size_source": "given", "prim": "triangles", '
             '"per_vertex_point_size": %s },' % b(fl["Point size in shaded vertex data"]))
    o.append('  "uniform_names": [],')
    names = ["a_pos", "a_color"]
    o.append('  "attributes": [%s],' % ", ".join(
        '{ "name": "%s", "location": %d, "records": [%d] }' % (names[i], i, i) for i in range(len(at))))

    def head(s, extra):
        return ('{ "threads": %d, "single_seg": %s, "spill_size": 0, "tmu_dirty_rcl": false, %s' %
                (4 if s["fourway"] else 2, b(s["final"]), extra))

    def uni(s):
        return "[%s]" % ", ".join('{ "contents": %d, "data": "%s" }' % (c, hx(d)) for c, d in s["uniforms"])

    def code(s):
        return "[%s]" % ", ".join('"%s"' % hx(c, 16) for c in s["code"])

    f = st["fs"]
    slots = ", ".join('{ "slot": 32, "component": %d }' % i for i in range(rc["varyings"]))
    o.append('  "fs": ' + head(f, '"num_inputs": %d, "input_slots": [%s], "flat_shade_flags": "0x00000000", '
                           '"noperspective_flags": "0x00000000", "centroid_flags": "0x00000000", '
                           '"writes_z": %s, "disable_ez": %s, "uses_center_w": %s, '
                           '"lock_scoreboard_on_first_thrsw": %s, "uses_discard": false, '
                           '"uses_implicit_point_line_varyings": %s, "uses_pid": %s, '
                           '"force_per_sample_msaa": %s,' % (
                               rc["varyings"], slots, b(fl["Fragment shader does Z writes"]),
                               b(fl["Turn off early-z test"]),
                               b(fl["Fragment shader uses real pixel centre W in addition to centroid W2"]),
                               b(fl["Do scoreboard wait on first thread switch"]),
                               b(not fl["Disable implicit point/line varyings"]),
                               b(fl["Insert Primitive ID as first varying to fragment shader"]),
                               b(fl["Enable Sample Rate Shading"]))))
    o.append('    "uniforms": %s,' % uni(f))
    o.append('    "code": %s },' % code(f))
    v = st["vs"]
    o.append('  "vs": ' + head(v, '"vpm_input_size": %d, "vpm_output_size": %d, "separate_segments": false, '
                           '"vcm_cache_size": %d, "uses_vid": %s, "uses_iid": %s, "uses_biid": %s, '
                           '"writes_psiz": %s, "vattr_sizes": %s,' % (
                               rc["vs_in"], rc["vs_out"], rc["vcm"],
                               b(fl["Vertex ID read by vertex shader"]), b(fl["Instance ID read by vertex shader"]),
                               b(fl["Base Instance ID read by vertex shader"]),
                               b(fl["Point size in shaded vertex data"]), json.dumps(vattr(at, "vs_n")))))
    o.append('    "uniforms": %s,' % uni(v))
    o.append('    "code": %s },' % code(v))
    c = st["cs"]
    o.append('  "cs": ' + head(c, '"vpm_input_size": %d, "vpm_output_size": %d, "separate_segments": false, '
                           '"vcm_cache_size": %d, "uses_vid": %s, "uses_iid": %s, "uses_biid": %s, '
                           '"writes_psiz": %s, "vattr_sizes": %s,' % (
                               rc["cs_in"], rc["cs_out"], rc["vcm"],
                               b(fl["Vertex ID read by coordinate shader"]),
                               b(fl["Instance ID read by coordinate shader"]),
                               b(fl["Base Instance ID read by coordinate shader"]),
                               b(fl["Point size in shaded vertex data"]), json.dumps(vattr(at, "cs_n")))))
    o.append('    "uniforms": %s,' % uni(c))
    o.append('    "code": %s }' % code(c))
    o.append("}")
    return "\n".join(o) + "\n"


def make_header(e):
    st, rc, at = e["stages"], e["rec"], e["attrs"]
    P = "MESA71_TRI"
    o = []
    o.append("/* SPDX-License-Identifier: Apache-2.0 */")
    o.append("/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */")
    o.append("/*")
    o.append(" * GENERATED by tools/v3d/clif_shaders71.py from tools/v3d/pi5/triangle.clif")
    o.append(" * (Mesa 26.2.2 V3D_DEBUG=clif on the Raspberry Pi 5, V3D 7.1.10, passthrough VS +")
    o.append(" * interp_color FS); do not edit. tools/v3d/check71.d/tri_shaders71.sh compares it.")
    o.append(" *")
    o.append(" * Shader record values are read at the BCM2712 D0 layout (GL Shader State Record")
    o.append(" * Draw Index); the flag word is that layout's first dword. Uniform kinds are the")
    o.append(" * script's inference (see its header); contents numbers are Mesa's QUNIFORM_*.")
    o.append(" */")
    o.append("#ifndef MESA_PASSTHROUGH_INTERP_COLOR_H")
    o.append("#define MESA_PASSTHROUGH_INTERP_COLOR_H")
    o.append("")
    o.append("#include <stdint.h>")
    o.append("")
    o.append("/* uniform kinds: contents numbers of the 4.2 JSON file */")
    o.append("#define %s_UNIFORM_CONSTANT 0u" % P)
    o.append("#define %s_UNIFORM_VIEWPORT_X_SCALE 2u" % P)
    o.append("#define %s_UNIFORM_VIEWPORT_Y_SCALE 3u" % P)
    o.append("#define %s_UNIFORM_VIEWPORT_Z_OFFSET 4u" % P)
    o.append("#define %s_UNIFORM_VIEWPORT_Z_SCALE 5u" % P)
    o.append("")
    for s in ("fs", "vs", "cs"):
        S = st[s]
        U = s.upper()
        o.append("/* %s: %d instructions, %d uniform words */" % (U, len(S["code"]), len(S["uniforms"])))
        o.append("#define %s_%s_CODE_COUNT %du" % (P, U, len(S["code"])))
        o.append("static const uint64_t mesa71_tri_%s_code[%d] = {" % (s, len(S["code"])))
        for c in S["code"]:
            o.append("\t0x%016xull," % c)
        o.append("};")
        o.append("#define %s_%s_UNIFORM_COUNT %du" % (P, U, len(S["uniforms"])))
        if S["uniforms"]:
            o.append("static const uint32_t mesa71_tri_%s_uniform[%d] = {" % (s, len(S["uniforms"])))
            for (c, d), w in zip(S["uniforms"], S["uwords"]):
                o.append("\t0x%08xu," % w)
            o.append("};")
            o.append("static const uint8_t mesa71_tri_%s_uniform_kind[%d] = {" % (s, len(S["uniforms"])))
            o.append("\t" + ", ".join("%d" % c for c, d in S["uniforms"]) + ",")
            o.append("};")
        o.append("#define %s_%s_FOUR_THREADS %d" % (P, U, S["fourway"]))
        o.append("#define %s_%s_FINAL_THREAD_SECTION %d" % (P, U, S["final"]))
        o.append("#define %s_%s_PROPAGATE_NANS %d" % (P, U, S["nan"]))
        o.append("")
    o.append("/* shader record (D0 layout) */")
    o.append("#define %s_REC_FLAGS 0x%08xu /* bit positions of the D0 layout */" % (P, 0))  # placeholder replaced below
    fl = rc["flags"]
    d0word = 0
    for k, bit in D0.items():
        if fl[k]:
            d0word |= 1 << bit
    o[-1] = "#define %s_REC_FLAGS 0x%08xu /* the D0 layout's first dword */" % (P, d0word)
    shortn = {
        "Point size in shaded vertex data": "POINT_SIZE",
        "Enable clipping": "ENABLE_CLIPPING",
        "Fragment shader does Z writes": "FS_Z_WRITES",
        "Turn off early-z test": "EARLY_Z_OFF",
        "Fragment shader uses real pixel centre W in addition to centroid W2": "FS_USES_REAL_CENTRE_W",
        "Enable Sample Rate Shading": "SAMPLE_RATE_SHADING",
        "Insert Primitive ID as first varying to fragment shader": "INSERT_PRIM_ID",
        "Turn off scoreboard": "SCOREBOARD_OFF",
        "Do scoreboard wait on first thread switch": "SCOREBOARD_WAIT_FIRST_THRSW",
        "Disable implicit point/line varyings": "DISABLE_IMPLICIT_PL_VARYINGS",
        "No prim pack": "NO_PRIM_PACK",
    }
    for k, n in shortn.items():
        o.append("#define %s_REC_%s %d" % (P, n, fl[k]))
    o.append("#define %s_REC_VARYINGS %du" % (P, rc["varyings"]))
    o.append("#define %s_REC_CS_OUT_VPM %du" % (P, rc["cs_out"]))
    o.append("#define %s_REC_CS_MIN_OUT %du" % (P, rc["cs_min_out"]))
    o.append("#define %s_REC_CS_IN_VPM %du" % (P, rc["cs_in"]))
    o.append("#define %s_REC_CS_MIN_IN %du /* the packed field minus one's value as the CLIF prints it */" % (P, rc["cs_min_in"]))
    o.append("#define %s_REC_VS_OUT_VPM %du" % (P, rc["vs_out"]))
    o.append("#define %s_REC_VS_MIN_OUT %du" % (P, rc["vs_min_out"]))
    o.append("#define %s_REC_VS_IN_VPM %du" % (P, rc["vs_in"]))
    o.append("#define %s_REC_VS_MIN_IN %du" % (P, rc["vs_min_in"]))
    o.append("#define %s_VCM_CACHE_BATCHES %du" % (P, rc["vcm"]))
    o.append("")
    o.append("/* attribute records: vec size, type (2 = float), values read by the CS and the VS, stride, max index */")
    o.append("#define %s_ATTR_COUNT %du" % (P, len(at)))
    o.append("struct mesa71_tri_attr { uint8_t vec_size, type, cs_values, vs_values; uint16_t stride; uint32_t max_index; uint32_t divisor; };")
    o.append("static const struct mesa71_tri_attr mesa71_tri_attr[%d] = {" % len(at))
    for a in at:
        o.append("\t{ %d, %d, %d, %d, %d, %du, %du }," % (a["vec"], a["type"], a["cs_n"], a["vs_n"], a["stride"], a["maxidx"], a["div"]))
    o.append("};")
    o.append("")
    o.append("#endif /* MESA_PASSTHROUGH_INTERP_COLOR_H */")
    return "\n".join(o) + "\n"


def check_shim(e, path):
    s = extract(path, d0=False)
    bad = []
    for st in ("fs", "vs", "cs"):
        for k in ("code", "uwords", "uniforms", "fourway", "final", "nan"):
            if e["stages"][st][k] != s["stages"][st][k]:
                bad.append("%s %s" % (st, k))
    if e["attrs"] != [dict(a, addr=x["addr"]) for a, x in zip(s["attrs"], e["attrs"])]:
        bad.append("attribute records")
    for k, v in e["rec"].items():
        if k not in ("flags", "raw_flags") and s["rec"][k] != v:
            bad.append("record " + k)
    # The shim's record is the plain layout (7.1.7), read as such; the board's
    # is read at D0: the same flags then mean the same.
    for k, v in e["rec"]["flags"].items():
        if s["rec"]["flags"][k] != v:
            bad.append("flag " + k)
    return bad


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("--clif", default=SRC)
    ap.add_argument("--out-dir", default=OUT)
    ap.add_argument("--check-shim", nargs="?", const=SHIM, default=None)
    a = ap.parse_args()
    e = extract(a.clif)
    for st, s in e["stages"].items():
        if not s["thrend"]:
            die("%s does not end in a thread switch with two delay slots" % st)
    if a.check_shim:
        bad = check_shim(e, a.check_shim)
        if bad:
            die("shim differs from the board's dump: " + ", ".join(bad))
        print("clif_shaders71: %s equals the board's shaders, uniforms, attributes and record" % os.path.relpath(a.check_shim, ROOT))
        return
    os.makedirs(a.out_dir, exist_ok=True)
    for ext, text in (("json", make_json(e)), ("h", make_header(e))):
        with open(os.path.join(a.out_dir, "%s.%s" % (BASE, ext)), "w", encoding="utf-8", newline="\n") as f:
            f.write(text)


if __name__ == "__main__":
    main()
