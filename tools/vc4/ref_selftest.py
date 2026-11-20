#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Self-test of the stored VC4 reference data (tools/vc4/testdata/), no Mesa.

    tools/vc4/ref_selftest.py

For each recorded submit (<case>.submitN.*, written by tools/vc4/drm_shim_ref.sh
run through tools/vc4/vc4_submit_rec.c) it checks:

  - .struct.bin is a struct drm_vc4_submit_cl (176 bytes, Mesa 26.2.1
    include/drm-uapi/vc4_drm.h) with its four user pointers 0, and every field
    the .txt names has the value the struct holds;
  - the .bcl, .shader_rec, .uniforms and .bo_handles files are as long as the
    struct says, and the handle table is the one the .txt lists;
  - the binner control list decodes: walked packet by packet with the opcode
    and size table of src/gallium/drivers/vc4/kernel/vc4_packet.h (below),
    every opcode is one of that header's, the walk ends exactly at
    bin_cl_size, the list ends with FLUSH (vc4_job.c caps it so), and the
    number of GL_SHADER_STATE packets is shader_rec_count;
  - each BO file the .txt names exists: a shader BO with the size the .txt
    gives, a plain BO no longer than that and ending in a non-zero byte (the
    recorder leaves its zero tail out);

and for each vc4_shc output (<case>.shc.json) that its fs, vs and cs code is,
word for word, a shader BO the driver submitted for the same case.

Prints one line per case and "N checks, 0 failed"; exits 1 on any failure.
"""
import json
import os
import re
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "testdata")

# vc4_packet.h:28-81 (enum vc4_packet) and :84-121 (VC4_PACKET_*_SIZE)
PACKETS = {
    0: ("HALT", 1), 1: ("NOP", 1), 4: ("FLUSH", 1), 5: ("FLUSH_ALL", 1),
    6: ("START_TILE_BINNING", 1), 7: ("INCREMENT_SEMAPHORE", 1),
    8: ("WAIT_ON_SEMAPHORE", 1), 16: ("BRANCH", 5), 17: ("BRANCH_TO_SUB_LIST", 5),
    24: ("STORE_MS_TILE_BUFFER", 1), 25: ("STORE_MS_TILE_BUFFER_AND_EOF", 1),
    26: ("STORE_FULL_RES_TILE_BUFFER", 5), 27: ("LOAD_FULL_RES_TILE_BUFFER", 5),
    28: ("STORE_TILE_BUFFER_GENERAL", 7), 29: ("LOAD_TILE_BUFFER_GENERAL", 7),
    32: ("GL_INDEXED_PRIMITIVE", 14), 33: ("GL_ARRAY_PRIMITIVE", 10),
    48: ("COMPRESSED_PRIMITIVE", 1), 49: ("CLIPPED_COMPRESSED_PRIMITIVE", 1),
    56: ("PRIMITIVE_LIST_FORMAT", 2), 64: ("GL_SHADER_STATE", 5),
    65: ("NV_SHADER_STATE", 5), 66: ("VG_SHADER_STATE", 5),
    96: ("CONFIGURATION_BITS", 4), 97: ("FLAT_SHADE_FLAGS", 5), 98: ("POINT_SIZE", 5),
    99: ("LINE_WIDTH", 5), 100: ("RHT_X_BOUNDARY", 3), 101: ("DEPTH_OFFSET", 5),
    102: ("CLIP_WINDOW", 9), 103: ("VIEWPORT_OFFSET", 5), 104: ("Z_CLIPPING", 9),
    105: ("CLIPPER_XY_SCALING", 9), 106: ("CLIPPER_Z_SCALING", 9),
    112: ("TILE_BINNING_MODE_CONFIG", 16), 113: ("TILE_RENDERING_MODE_CONFIG", 11),
    114: ("CLEAR_COLORS", 14), 115: ("TILE_COORDINATES", 3), 254: ("GEM_HANDLES", 9),
}

# struct drm_vc4_submit_cl, vc4_drm.h:91-198, little endian, natural alignment
SURF = "IIHH"
LAYOUT = "<QQQQIIIIIHHBBBB" + SURF * 6 + "IIIB3xIQIIII"
SURFACES = ("color_read", "color_write", "zs_read", "zs_write", "msaa_color_write",
            "msaa_zs_write")


def unpack_submit(raw):
    v = list(struct.unpack(LAYOUT, raw))
    f = {}
    for name in ("bin_cl", "shader_rec", "uniforms", "bo_handles", "bin_cl_size",
                 "shader_rec_size", "shader_rec_count", "uniforms_size", "bo_handle_count",
                 "width", "height", "min_x_tile", "min_y_tile", "max_x_tile", "max_y_tile"):
        f[name] = v.pop(0)
    for s in SURFACES:
        for part in ("hindex", "offset", "bits", "flags"):
            f["%s.%s" % (s, part)] = v.pop(0)
    f["clear_color"] = (v.pop(0), v.pop(0))
    for name in ("clear_z", "clear_s", "flags", "seqno", "perfmonid", "in_sync", "out_sync",
                 "pad2"):
        f[name] = v.pop(0)
    return f


def walk_bcl(bcl):
    """[(offset, opcode name)] or raises ValueError"""
    out, i = [], 0
    while i < len(bcl):
        op = bcl[i]
        if op not in PACKETS:
            raise ValueError("offset %d: opcode %d is not in vc4_packet.h" % (i, op))
        name, size = PACKETS[op]
        if i + size > len(bcl):
            raise ValueError("offset %d: %s (%d bytes) runs past the end" % (i, name, size))
        out.append((i, name))
        i += size
    return out


class Checks:
    def __init__(self):
        self.n = 0
        self.failed = 0

    def check(self, ok, what):
        self.n += 1
        if not ok:
            self.failed += 1
            print("FAIL: " + what)


def read(path):
    with open(path, "rb") as f:
        return f.read()


def check_submit(c, prefix):
    base = os.path.basename(prefix)
    raw = read(prefix + ".struct.bin")
    c.check(len(raw) == struct.calcsize(LAYOUT) == 176, base + ": struct is 176 bytes")
    if len(raw) != struct.calcsize(LAYOUT):
        return [], ""
    f = unpack_submit(raw)
    c.check(f["bin_cl"] == f["shader_rec"] == f["uniforms"] == f["bo_handles"] == 0,
            base + ": the user pointers are 0")
    txt = {}
    bos = []
    for line in open(prefix + ".txt"):
        if line.startswith("#"):
            continue
        k, _, rest = line.rstrip("\n").partition(" ")
        if k == "bo":
            bos.append(rest.split(" ", 4))
        else:
            txt[k] = rest
    for k, val in txt.items():
        if k == "clear_color":
            want = " ".join("0x%08x" % x for x in f[k])
        elif k.endswith(".bits") or k.endswith(".flags") and "." in k:
            want = "0x%04x" % f[k]
        elif k in ("clear_s",):
            want = "0x%02x" % f[k]
        elif k.endswith(".hindex") or k.endswith(".offset") or k in ("clear_z", "flags"):
            want = "0x%08x" % f[k]
        else:
            want = str(f[k])
        c.check(val == want, "%s: %s is %s in the .txt, %s in the struct" % (base, k, val, want))
    for name, size in (("bcl", "bin_cl_size"), ("shader_rec", "shader_rec_size"),
                       ("uniforms", "uniforms_size")):
        c.check(len(read("%s.%s.bin" % (prefix, name))) == f[size],
                "%s: .%s.bin is %s bytes" % (base, name, size))
    handles = read(prefix + ".bo_handles.bin")
    c.check(len(handles) == 4 * f["bo_handle_count"], base + ": handle table length")
    table = list(struct.unpack("<%dI" % (len(handles) // 4), handles))
    c.check([int(b[1]) for b in bos] == table, base + ": the .txt lists the handle table")
    shader_bos = []
    for idx, handle, kind, size, fname in bos:
        if fname == "(not mapped)":
            continue
        data = read("%s.%s.bin" % (prefix, fname))
        if kind == "shader":
            c.check(len(data) == int(size), "%s: %s is %s bytes" % (base, fname, size))
        else:
            # a plain BO's file stops at its last non-zero byte
            c.check(len(data) <= int(size) and (not data or data[-1] != 0),
                    "%s: %s is at most %s bytes and ends non-zero" % (base, fname, size))
        if kind == "shader":
            shader_bos.append(data)
    bcl = read(prefix + ".bcl.bin")
    try:
        packets = walk_bcl(bcl)
        c.check(True, base + ": the binner list decodes")
    except ValueError as e:
        c.check(False, "%s: %s" % (base, e))
        packets = []
    if bcl:
        c.check(packets and packets[-1][1] == "FLUSH", base + ": the binner list ends with FLUSH")
    c.check(sum(1 for _, n in packets if n == "GL_SHADER_STATE") == f["shader_rec_count"],
            base + ": one GL_SHADER_STATE per shader record")
    return shader_bos, "%d packets, %d bytes" % (len(packets), len(bcl))


def main():
    c = Checks()
    names = sorted(os.listdir(DATA))
    prefixes = sorted({os.path.join(DATA, m.group(1)) for n in names
                       for m in [re.match(r"(.*\.submit\d+)\.txt$", n)] if m})
    c.check(len(prefixes) >= 3, "at least three recorded submits (clear, triangle, flat)")
    shader_words = {}
    for p in prefixes:
        bos, what = check_submit(c, p)
        case = os.path.basename(p).split(".")[0]
        shader_words.setdefault(case, []).extend(bos)
        print("vc4 ref: %s: %s" % (os.path.basename(p), what))
    for n in names:
        if not n.endswith(".shc.json"):
            continue
        case = n.split(".")[0]
        js = json.load(open(os.path.join(DATA, n)))
        for stage in ("fs", "vs", "cs"):
            words = b"".join(struct.pack("<Q", int(w, 16)) for w in js[stage]["code"])
            c.check(words in shader_words.get(case, []),
                    "%s: vc4_shc's %s is a shader BO of the %s submit" % (n, stage, case))
        print("vc4 ref: %s: fs %d, vs %d, cs %d instructions" % (
            n, len(js["fs"]["code"]), len(js["vs"]["code"]), len(js["cs"]["code"])))
    print("vc4 ref: %d checks, %d failed" % (c.n, c.failed))
    return 1 if c.failed else 0


if __name__ == "__main__":
    sys.exit(main())
