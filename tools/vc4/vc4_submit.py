#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Read what tools/vc4/drm_shim_ref.py recorded of Mesa's vc4 driver.

    tools/vc4/vc4_submit.py decode <file.rec.bin>
        print the record: the buffer objects, each shader's QPU words, and for
        each submit the fields of struct drm_vc4_submit_cl, the binner list
        packet by packet, the shader records and the uniform words.
    tools/vc4/vc4_submit.py extract <file.rec.bin> <out prefix>
        write the parts as raw files, for a test that wants to include them:
        <out prefix>.submit<N>.{struct,bcl,shader_rec,uniforms,bo_handles}.bin
        and <out prefix>.shader<handle>.bin (N counts submits from 0).
    tools/vc4/vc4_submit.py check-header <vc4_packet.h>
        compare PACKETS below with Mesa's
        src/gallium/drivers/vc4/kernel/vc4_packet.h (drm_shim_ref.py run does
        this on every run).

A record is a sequence of (4-byte tag, u32 length, bytes), little-endian, in
the order of the driver's ioctls; drm_shim_ref.py's docstring names the tags.
Reading fails (ValueError) on a tag it does not know, a length that runs past
the file, a binner list with an opcode outside PACKETS or one that does not
end exactly at bin_cl_size, and shader records that do not end exactly at
shader_rec_size.

The numbers here are facts read from Mesa 26.2.1 (include/drm-uapi/vc4_drm.h,
kernel/vc4_packet.h, kernel/vc4_validate.c); no Mesa text is in this file.
"""
import re
import struct
import sys
from pathlib import Path

# opcode: (name in vc4_packet.h without VC4_PACKET_, size in bytes with the opcode)
PACKETS = {
    0: ("HALT", 1),
    1: ("NOP", 1),
    4: ("FLUSH", 1),
    5: ("FLUSH_ALL", 1),
    6: ("START_TILE_BINNING", 1),
    7: ("INCREMENT_SEMAPHORE", 1),
    8: ("WAIT_ON_SEMAPHORE", 1),
    16: ("BRANCH", 5),
    17: ("BRANCH_TO_SUB_LIST", 5),
    24: ("STORE_MS_TILE_BUFFER", 1),
    25: ("STORE_MS_TILE_BUFFER_AND_EOF", 1),
    26: ("STORE_FULL_RES_TILE_BUFFER", 5),
    27: ("LOAD_FULL_RES_TILE_BUFFER", 5),
    28: ("STORE_TILE_BUFFER_GENERAL", 7),
    29: ("LOAD_TILE_BUFFER_GENERAL", 7),
    32: ("GL_INDEXED_PRIMITIVE", 14),
    33: ("GL_ARRAY_PRIMITIVE", 10),
    48: ("COMPRESSED_PRIMITIVE", 1),
    49: ("CLIPPED_COMPRESSED_PRIMITIVE", 1),
    56: ("PRIMITIVE_LIST_FORMAT", 2),
    64: ("GL_SHADER_STATE", 5),
    65: ("NV_SHADER_STATE", 5),
    66: ("VG_SHADER_STATE", 5),
    96: ("CONFIGURATION_BITS", 4),
    97: ("FLAT_SHADE_FLAGS", 5),
    98: ("POINT_SIZE", 5),
    99: ("LINE_WIDTH", 5),
    100: ("RHT_X_BOUNDARY", 3),
    101: ("DEPTH_OFFSET", 5),
    102: ("CLIP_WINDOW", 9),
    103: ("VIEWPORT_OFFSET", 5),
    104: ("Z_CLIPPING", 9),
    105: ("CLIPPER_XY_SCALING", 9),
    106: ("CLIPPER_Z_SCALING", 9),
    112: ("TILE_BINNING_MODE_CONFIG", 16),
    113: ("TILE_RENDERING_MODE_CONFIG", 11),
    114: ("CLEAR_COLORS", 14),
    115: ("TILE_COORDINATES", 3),
    # not a hardware packet: the driver's BO references, which the kernel removes
    254: ("GEM_HANDLES", 9),
}
GL_SHADER_STATE = 64

# struct drm_vc4_submit_cl (vc4_drm.h), 176 bytes
SURFACES = ("color_read", "color_write", "zs_read", "zs_write", "msaa_color_write", "msaa_zs_write")
SUBMIT = struct.Struct("<4Q5I2H4B" + "IIHH" * len(SURFACES) + "2IIB3xIQ4I")
SUBMIT_NAMES = (
    ["bin_cl", "shader_rec", "uniforms", "bo_handles", "bin_cl_size", "shader_rec_size", "shader_rec_count",
     "uniforms_size", "bo_handle_count", "width", "height", "min_x_tile", "min_y_tile", "max_x_tile", "max_y_tile"]
    + ["%s.%s" % (s, f) for s in SURFACES for f in ("hindex", "offset", "bits", "flags")]
    + ["clear_color[0]", "clear_color[1]", "clear_z", "clear_s", "flags", "seqno", "perfmonid", "in_sync",
       "out_sync", "pad2"])
POINTERS = ("bin_cl", "shader_rec", "uniforms", "bo_handles")
SUBMIT_FLAGS = ((1, "USE_CLEAR_COLOR"), (2, "FIXED_RCL_ORDER"), (4, "RCL_ORDER_INCREASING_X"),
                (8, "RCL_ORDER_INCREASING_Y"))
CREATE_BO = struct.Struct("<4I")          # size, flags, handle, pad
CREATE_SHADER_BO = struct.Struct("<IIQII")  # size, flags, data, handle, pad
assert SUBMIT.size == 176 and len(SUBMIT_NAMES) == 49 and CREATE_SHADER_BO.size == 24


def hexbytes(b):
    return " ".join("%02x" % x for x in b)


class Submit:
    """One DRM_VC4_SUBMIT_CL: the struct and the four blobs it points at."""

    def __init__(self, payload):
        if len(payload) < SUBMIT.size:
            raise ValueError("SUBM of %d bytes is shorter than the struct (%d)" % (len(payload), SUBMIT.size))
        self.struct = payload[:SUBMIT.size]
        self.fields = dict(zip(SUBMIT_NAMES, SUBMIT.unpack(self.struct)))
        f = self.fields
        for p in POINTERS:
            if f[p] != 0:
                raise ValueError("SUBM pointer %s is %#x, not the recorder's 0" % (p, f[p]))
        sizes = (f["bin_cl_size"], f["shader_rec_size"], f["uniforms_size"], f["bo_handle_count"] * 4)
        if SUBMIT.size + sum(sizes) != len(payload):
            raise ValueError("SUBM of %d bytes, the struct says %d + %s" % (len(payload), SUBMIT.size, sizes))
        at, parts = SUBMIT.size, []
        for n in sizes:
            parts.append(payload[at:at + n])
            at += n
        self.bcl, self.shader_rec, self.uniforms, handles = parts
        self.bo_handles = list(struct.unpack("<%dI" % f["bo_handle_count"], handles))
        self.bo_handles_raw = handles
        self.packets = self._walk()
        self.shader_records = self._records()

    def _walk(self):
        """[(offset, opcode, name, bytes)]; every opcode known, the end exact."""
        out, at, bcl = [], 0, self.bcl
        while at < len(bcl):
            op = bcl[at]
            if op not in PACKETS:
                raise ValueError("binner list: opcode %d at offset %d is not in vc4_packet.h" % (op, at))
            name, size = PACKETS[op]
            if at + size > len(bcl):
                raise ValueError("binner list: %s at offset %d runs past bin_cl_size %d" % (name, at, len(bcl)))
            out.append((at, op, name, bcl[at:at + size]))
            at += size
        return out

    def _records(self):
        """One dict per GL_SHADER_STATE of the list, in order (vc4_validate.c:
        the handle indices of the three shaders and of each attribute's BO,
        then the record; the packet's low 3 bits are the attribute count,
        0 meaning 8, and bit 3 selects the extended record)."""
        out, at, rec = [], 0, self.shader_rec
        for _, op, _, raw in self.packets:
            if op != GL_SHADER_STATE:
                continue
            bits = struct.unpack_from("<I", raw, 1)[0]
            nattr = bits & 7 or 8
            if bits & 8:
                raise ValueError("an extended shader record: not read by this tool")
            size = 36 + 8 * nattr
            if at + 4 * (3 + nattr) + size > len(rec):
                raise ValueError("shader record %d runs past shader_rec_size %d" % (len(out), len(rec)))
            hindex = list(struct.unpack_from("<%dI" % (3 + nattr), rec, at))
            at += 4 * (3 + nattr)
            body = rec[at:at + size]
            at += size
            # flags u16, a byte not used, the varying count, then code and
            # uniform addresses; vs and cs: u16 not used, attribute select
            # bits, attributes size, code and uniform addresses
            flags, _, fs_nvary, fs_code, _ = struct.unpack_from("<HBBII", body, 0)
            vs = struct.unpack_from("<HBBII", body, 12)
            cs = struct.unpack_from("<HBBII", body, 24)
            # address, bytes minus 1, stride, VS VPM offset, CS VPM offset
            attrs = [struct.unpack_from("<IBBBB", body, 36 + 8 * i) for i in range(nattr)]
            out.append(dict(hindex=hindex, raw=body, nattr=nattr, flags=flags, fs_nvary=fs_nvary,
                            fs_code=fs_code, vs=vs, cs=cs, attrs=attrs))
        if at != len(rec):
            raise ValueError("shader records end at %d, shader_rec_size is %d" % (at, len(rec)))
        if len(out) != self.fields["shader_rec_count"]:
            raise ValueError("%d GL_SHADER_STATE packets, shader_rec_count is %d"
                             % (len(out), self.fields["shader_rec_count"]))
        for r in out:
            for h in r["hindex"]:
                if h >= len(self.bo_handles):
                    raise ValueError("handle index %d of a shader record is past the table (%d)"
                                     % (h, len(self.bo_handles)))
        return out

    def uniform_words(self):
        if len(self.uniforms) % 4:
            raise ValueError("uniforms_size %d is not whole words" % len(self.uniforms))
        return list(struct.unpack("<%dI" % (len(self.uniforms) // 4), self.uniforms))

    def decode(self, lines):
        f = self.fields
        lines.append("  struct drm_vc4_submit_cl (%d bytes, the four pointers recorded as 0):" % SUBMIT.size)
        for name in SUBMIT_NAMES:
            v = f[name]
            if name in POINTERS + ("pad2",):
                continue
            if name.endswith(".hindex") and v == 0xffffffff:
                lines.append("    %-28s none" % name.rsplit(".", 1)[0])
                continue
            if "." in name and f[name.rsplit(".", 1)[0] + ".hindex"] == 0xffffffff:
                continue
            if name == "flags":
                names = [n for bit, n in SUBMIT_FLAGS if v & bit]
                lines.append("    %-28s %#x%s" % (name, v, " (%s)" % " | ".join(names) if names else ""))
            elif name.startswith("clear_") or name.endswith((".bits", ".offset")):
                lines.append("    %-28s %#x" % (name, v))
            else:
                lines.append("    %-28s %d" % (name, v))
        lines.append("  bo_handles (%d): %s" % (len(self.bo_handles), " ".join(str(h) for h in self.bo_handles)))
        lines.append("  binner list (%d bytes, %d packets):" % (len(self.bcl), len(self.packets)))
        for at, op, name, raw in self.packets:
            lines.append("    %04x: %3d %-28s %s" % (at, op, name, hexbytes(raw[1:])))
        lines.append("  shader records (%d bytes, %d):" % (len(self.shader_rec), len(self.shader_records)))
        for i, r in enumerate(self.shader_records):
            h = r["hindex"]
            lines.append("    record %d: %d attribute(s), flags %#x, %d bytes: %s"
                         % (i, r["nattr"], r["flags"], len(r["raw"]), hexbytes(r["raw"])))
            lines.append("      fs: handle index %d (BO %d), code offset %#x, %d varyings"
                         % (h[0], self.bo_handles[h[0]], r["fs_code"], r["fs_nvary"]))
            for j, (stage, s) in enumerate((("vs", r["vs"]), ("cs", r["cs"]))):
                lines.append("      %s: handle index %d (BO %d), code offset %#x, attribute select %#x, "
                             "attributes size %d" % (stage, h[1 + j], self.bo_handles[h[1 + j]], s[3], s[1], s[2]))
            for j, (addr, nbytes, stride, vs_off, cs_off) in enumerate(r["attrs"]):
                lines.append("      attribute %d: handle index %d (BO %d), offset %#x, %d bytes, stride %d, "
                             "VPM offset vs %d cs %d"
                             % (j, h[3 + j], self.bo_handles[h[3 + j]], addr, nbytes + 1, stride, vs_off, cs_off))
        words = self.uniform_words()
        lines.append("  uniforms (%d bytes, %d words):" % (len(self.uniforms), len(words)))
        for i, w in enumerate(words):
            lines.append("    %3d: 0x%08x  %r" % (i, w, struct.unpack("<f", struct.pack("<I", w))[0]))


class Record:
    """A whole .rec.bin."""

    def __init__(self, data):
        self.bos = []         # (handle, size) in creation order
        self.shaders = []     # (handle, bytes) in creation order
        self.submits = []
        at = 0
        while at < len(data):
            if at + 8 > len(data):
                raise ValueError("record cut inside a header at %d" % at)
            tag, n = data[at:at + 4], struct.unpack_from("<I", data, at + 4)[0]
            payload = data[at + 8:at + 8 + n]
            if len(payload) != n:
                raise ValueError("%r at %d says %d bytes, the file has %d" % (tag, at, n, len(payload)))
            at += 8 + n
            if tag == b"CRBO":
                if n != CREATE_BO.size:
                    raise ValueError("CRBO of %d bytes" % n)
                size, _, handle, _ = CREATE_BO.unpack(payload)
                self.bos.append((handle, size))
            elif tag == b"SHBO":
                size, flags, ptr, handle, _ = CREATE_SHADER_BO.unpack_from(payload)
                code = payload[CREATE_SHADER_BO.size:]
                if ptr != 0 or flags != 0 or size != len(code) or size % 8:
                    raise ValueError("SHBO handle %d: size %d, flags %d, pointer %#x, %d code bytes"
                                     % (handle, size, flags, ptr, len(code)))
                self.shaders.append((handle, code))
            elif tag == b"SUBM":
                self.submits.append(Submit(payload))
            else:
                raise ValueError("unknown tag %r at %d" % (tag, at - 8 - n))
        handles = [h for h, _ in self.bos] + [h for h, _ in self.shaders]
        if len(set(handles)) != len(handles):
            raise ValueError("a BO handle was created twice: %s" % handles)
        for s in self.submits:
            for h in s.bo_handles:
                if h not in handles:
                    raise ValueError("the submit names BO %d, which no ioctl created" % h)

    def shader_words(self, handle):
        code = dict(self.shaders)[handle]
        return list(struct.unpack("<%dQ" % (len(code) // 8), code))

    def summary(self):
        return "%d BO(s), %d shader(s) (%s QPU words), %d submit(s) (binner list %s bytes)" % (
            len(self.bos), len(self.shaders), "+".join(str(len(c) // 8) for _, c in self.shaders) or "0",
            len(self.submits), "+".join(str(len(s.bcl)) for s in self.submits) or "0")

    def decode(self):
        lines = ["# tools/vc4/vc4_submit.py decode: " + self.summary()]
        lines.append("buffer objects (DRM_VC4_CREATE_BO): " +
                     ", ".join("%d: %d bytes" % b for b in self.bos))
        for handle, code in self.shaders:
            lines.append("shader BO %d (DRM_VC4_CREATE_SHADER_BO), %d QPU words:" % (handle, len(code) // 8))
            lines += ["  0x%016x" % w for w in self.shader_words(handle)]
        for i, s in enumerate(self.submits):
            lines.append("submit %d (DRM_VC4_SUBMIT_CL):" % i)
            s.decode(lines)
        return "\n".join(lines) + "\n"

    def extract(self, prefix):
        out = []
        for handle, code in self.shaders:
            out.append(("%s.shader%d.bin" % (prefix, handle), code))
        for i, s in enumerate(self.submits):
            for part, data in (("struct", s.struct), ("bcl", s.bcl), ("shader_rec", s.shader_rec),
                               ("uniforms", s.uniforms), ("bo_handles", s.bo_handles_raw)):
                out.append(("%s.submit%d.%s.bin" % (prefix, i, part), data))
        for path, data in out:
            Path(path).write_bytes(data)
        return [p for p, _ in out]


def header_packets(text):
    """{opcode: (name, size)} of a vc4_packet.h."""
    codes = {m.group(1): int(m.group(2)) for m in re.finditer(r"^\s*VC4_PACKET_(\w+) = (\d+),", text, re.M)}
    sizes = {m.group(1): int(m.group(2)) for m in re.finditer(r"^#define VC4_PACKET_(\w+)_SIZE\s+(\d+)\s*$",
                                                              text, re.M)}
    if not codes or set(codes) != set(sizes):
        raise ValueError("vc4_packet.h: %d opcodes, %d sizes, not the same names: %s"
                         % (len(codes), len(sizes), sorted(set(codes) ^ set(sizes))))
    return {code: (name, sizes[name]) for name, code in codes.items()}


def check_packet_table(text):
    theirs = header_packets(text)
    if theirs != PACKETS:
        diff = sorted(set(theirs.items()) ^ set(PACKETS.items()))
        raise ValueError("PACKETS differs from vc4_packet.h: %s" % diff)
    return len(theirs)


# Mesa's VC4_DEBUG=qpu,qir text, each line after Mesa's "MESA: info: ":
#   MESA_SHADER_<STAGE> prog <p>/<v> QIR:   then the final QIR
#   MESA_SHADER_<STAGE> prog <p>/<v> QPU:   then "0x<16 hex> <disassembly>" a word
#   uniform <n>/<i>: 0x<8 hex> <what>       word i of compiled shader n's uniform
#                                           stream, as a draw writes it
DUMP_HEAD = re.compile(r"\bMESA_SHADER_(FRAGMENT|VERTEX|COORD) prog (\d+)/(\d+) (pre-opt QIR|QIR|QPU):\s*$")
QPU_LINE = re.compile(r"(?:^|\s)0x([0-9a-f]{16}) (.*)$")
QIR_UNIFORM = re.compile(r"\bu(\d+) \(([^()]*)\)")
UNIFORM_LINE = re.compile(r"\buniform (\d+)/(\d+): 0x([0-9a-f]{8}) (.*)$")


def parse_shader_dump(text):
    """(shaders, streams) of a .qpu.txt.

    shaders: in compile order, dicts with stage (FRAGMENT, VERTEX, COORD),
    prog, variant, words (ints), disasm (strings) and qir_uniforms: what each
    uniform the final QIR reads holds, in the QIR's order. That is not the
    order of the stream: the QPU scheduler orders the uniforms again as it
    places the instructions (vc4_qpu_schedule.c, schedule_instructions), and
    prints nothing of it.
    streams: in the order the draw wrote them, lists of (value, what): the
    words of one shader's uniform stream. A shader that reads no uniform has
    no entry."""
    shaders, order, cur, section = {}, [], None, None
    streams, last = [], None
    for line in text.splitlines():
        m = DUMP_HEAD.search(line)
        if m:
            key = (m.group(1), int(m.group(2)), int(m.group(3)))
            if key not in shaders:
                shaders[key] = dict(stage=key[0], prog=key[1], variant=key[2], words=[], disasm=[],
                                    qir_uniforms=[])
                order.append(key)
            cur, section = shaders[key], m.group(4)
            continue
        m = UNIFORM_LINE.search(line)
        if m:
            n, i = int(m.group(1)), int(m.group(2))
            if i == 0:
                streams.append([])
            elif last != (n, i - 1):
                raise ValueError("uniform dump: word %d/%d does not follow %s" % (n, i, last))
            streams[-1].append((int(m.group(3), 16), m.group(4).strip()))
            last = (n, i)
            continue
        if cur is None:
            continue
        if section == "QPU":
            m = QPU_LINE.search(line)
            if m:
                cur["words"].append(int(m.group(1), 16))
                cur["disasm"].append(m.group(2).strip())
        elif section == "QIR":
            found = QIR_UNIFORM.findall(line)
            if found:
                if len({u for u, _ in found}) != 1:
                    raise ValueError("a QIR instruction reads two uniforms: %s" % line.strip())
                cur["qir_uniforms"].append(found[0][1])
    return [shaders[k] for k in order], streams


# Mesa has no name for these three (qir_describe_uniform prints "??? <n>", the
# enum quniform_contents value of vc4_qir.h).
UNNAMED_KINDS = {"??? 10": "tex_msaa_addr", "??? 11": "ubo0_addr", "??? 12": "ubo1_addr"}


def uniform_kind(what):
    """What a uniform word holds, without its value: `constant`, `unif[3]`,
    `vp_x_scale`, `tex_p0[0]`, `ubo0_addr`, ... (the dump of a draw adds the
    value to a GL uniform: "unif[3] = 0x... / ...")."""
    if what.startswith("0x"):
        return "constant"
    what = what.split(" = ")[0]
    return UNNAMED_KINDS.get(what, what)


def has_bo_handle(kind):
    """Whether a uniform of this kind points into a BO: the driver then puts
    one BO handle index in front of the shader's uniform words (vc4_program.c
    counts them as num_texture_samples: a texture sample, a multisample
    texel fetch, an indirect uniform load)."""
    return kind.startswith("tex_p0[") or kind in UNNAMED_KINDS.values()


def main():
    a = sys.argv[1:]
    try:
        if len(a) == 2 and a[0] == "decode":
            sys.stdout.write(Record(Path(a[1]).read_bytes()).decode())
        elif len(a) == 3 and a[0] == "extract":
            print("\n".join(Record(Path(a[1]).read_bytes()).extract(a[2])))
        elif len(a) == 2 and a[0] == "check-header":
            print("vc4_submit: PACKETS is vc4_packet.h's %d opcodes and sizes" %
                  check_packet_table(Path(a[1]).read_text()))
        else:
            sys.stderr.write(__doc__)
            return 2
    except ValueError as e:
        sys.exit("vc4_submit: %s" % e)
    return 0


if __name__ == "__main__":
    sys.exit(main())
