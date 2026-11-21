#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Self-test of tools/vc4/vc4_submit.py and vc4_shc.py on the stored reference
data of tools/testdata/vc4/ (what Mesa 26.2.1's vc4 driver submits for a clear
and for one triangle). Needs no Mesa tree and no GPU; takes no argument.

    python3 tools/vc4/vc4_selftest.py

For each case: the record reads (every opcode of the binner list is one of
vc4_packet.h's, the list ends at bin_cl_size, the shader records end at
shader_rec_size), its decode is the stored .decode.txt, the list ends with
INCREMENT_SEMAPHORE and FLUSH, and Mesa's own text dump names the same
packets at the same offsets. For the triangle: Mesa's QPU dump names the
words the driver handed the kernel, and vc4_shc.py --from gives the stored
JSON. Then the reader is shown to refuse: an opcode vc4_packet.h does not
have, a list that ends inside a packet, a struct whose sizes are not the
record's, a cut file, an unknown tag, and an opcode table that differs from
a header.
"""
import json
import re
import struct
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DATA = HERE.parent / "testdata" / "vc4"
sys.path.insert(0, str(HERE))
import vc4_submit  # noqa: E402

# case: (BOs, QPU words per shader, binner list bytes, packets, uniform words)
EXPECT = {
    "clear": (1, [], 21, 5, 0),
    "triangle": (2, [13, 25, 23], 92, 16, 8),
}
CL_LINE = re.compile(r"\b0x([0-9a-f]{8}) 0x([0-9a-f]{8}): 0x([0-9a-f]{2}) (.+)$")
checks = 0


def check(ok, what):
    global checks
    if not ok:
        sys.exit("vc4_selftest: FAILED: " + what)
    checks += 1


def refuses(data, what, needle):
    try:
        vc4_submit.Record(data)
    except ValueError as e:
        check(needle in str(e), "%s: refused, but with %r" % (what, str(e)))
        return
    check(False, "%s was not refused" % what)


def retag(data, find, replace):
    """`data` with the first SUBM's payload changed by replace(payload)."""
    at = 0
    while data[at:at + 4] != find:
        at += 8 + struct.unpack_from("<I", data, at + 4)[0]
    n = struct.unpack_from("<I", data, at + 4)[0]
    payload = replace(bytearray(data[at + 8:at + 8 + n]))
    return data[:at + 4] + struct.pack("<I", len(payload)) + bytes(payload) + data[at + 8 + n:]


def main():
    if len(sys.argv) != 1:
        sys.exit(__doc__)
    for case, (bos, shader_words, bcl_bytes, npackets, nuniforms) in EXPECT.items():
        prefix = str(DATA / case)
        data = Path(prefix + ".rec.bin").read_bytes()
        record = vc4_submit.Record(data)
        check(len(record.submits) == 1, "%s: one submit" % case)
        submit = record.submits[0]
        check(len(record.bos) == bos and [len(c) // 8 for _, c in record.shaders] == shader_words,
              "%s: %s" % (case, record.summary()))
        check(len(submit.bcl) == bcl_bytes == submit.fields["bin_cl_size"] and len(submit.packets) == npackets,
              "%s: binner list of %d bytes, %d packets" % (case, len(submit.bcl), len(submit.packets)))
        check(len(submit.uniform_words()) == nuniforms, "%s: uniform words" % case)
        check(record.decode() == Path(prefix + ".decode.txt").read_text(), "%s: decode is not .decode.txt" % case)
        names = [name for _, _, name, _ in submit.packets]
        check(names[-2:] == ["INCREMENT_SEMAPHORE", "FLUSH"], "%s: the list ends with %s" % (case, names[-2:]))
        # Mesa prints its dump before it adds those two packets (vc4_job.c)
        mesa = [(int(m.group(1), 16), int(m.group(3), 16)) for m in
                map(CL_LINE.search, Path(prefix + ".cl.txt").read_text().splitlines()) if m]
        check(mesa == [(at, op) for at, op, _, _ in submit.packets[:-2]],
              "%s: Mesa's dump names %d packets, not the list's first %d" % (case, len(mesa), npackets - 2))
        out = Path(prefix + ".stdout").read_text()
        check("renderer VC4 V3D 2.1" in out and "glGetError 0x0" in out, "%s: .stdout" % case)

    prefix = str(DATA / "triangle")
    record = vc4_submit.Record(Path(prefix + ".rec.bin").read_bytes())
    submit = record.submits[0]
    rec = submit.shader_records[0]
    dump, streams = vc4_submit.parse_shader_dump(Path(prefix + ".qpu.txt").read_text())
    check([d["stage"] for d in dump] == ["FRAGMENT", "VERTEX", "COORD"], "triangle: the dump's three shaders")
    for i, d in enumerate(dump):
        handle = submit.bo_handles[rec["hindex"][i]]
        check(record.shader_words(handle) == d["words"],
              "triangle: Mesa's %s words are not shader BO %d" % (d["stage"], handle))
    check([len(s) for s in streams] == [4, 4], "triangle: two uniform streams of 4 words")
    check([v for s in streams for v, _ in s] == submit.uniform_words(), "triangle: the dumped uniform values")
    # the scheduler's order, not the QIR's: the coordinate shader reads y first
    check([w for _, w in streams[1]][:2] == ["vp_y_scale", "vp_x_scale"] and
          dump[2]["qir_uniforms"][:2] == ["vp_x_scale", "vp_y_scale"], "triangle: the coordinate shader's order")
    p = subprocess.run([sys.executable, str(HERE / "vc4_shc.py"), "--from", prefix], capture_output=True, text=True)
    check(p.returncode == 0, "vc4_shc.py --from: " + p.stderr)
    check(p.stdout == Path(prefix + ".shc.json").read_text(), "vc4_shc.py --from is not triangle.shc.json")
    shc = json.loads(p.stdout)
    check([len(shc["shaders"][k]["qpu"]) for k in ("fs", "vs", "cs")] == EXPECT["triangle"][1], "the JSON's words")

    data = Path(prefix + ".rec.bin").read_bytes()
    head = vc4_submit.SUBMIT.size

    def bad_opcode(p):
        p[head + 16] = 2  # START_TILE_BINNING becomes an opcode vc4_packet.h lacks
        return p

    def short_list(p):
        # bin_cl_size ends inside the last draw packet (GL_ARRAY_PRIMITIVE at
        # 0x50, 10 bytes); the bytes after it count as shader records
        cut = struct.unpack_from("<I", p, 32)[0] - 0x55
        struct.pack_into("<I", p, 32, 0x55)
        struct.pack_into("<I", p, 36, struct.unpack_from("<I", p, 36)[0] + cut)  # shader_rec_size
        return p

    def one_more_handle(p):
        struct.pack_into("<I", p, 48, struct.unpack_from("<I", p, 48)[0] + 1)  # bo_handle_count
        return p

    refuses(retag(data, b"SUBM", bad_opcode), "an unknown opcode", "opcode 2 at offset 16")
    refuses(retag(data, b"SUBM", short_list), "a list that ends inside a packet", "runs past bin_cl_size")
    refuses(retag(data, b"SUBM", one_more_handle), "a struct that says more than the record holds", "the struct says")
    refuses(data[:-3], "a cut file", "the file has")
    refuses(b"ABCD" + struct.pack("<I", 0) + data, "an unknown tag", "unknown tag")

    header = "".join("        VC4_PACKET_%s = %d,\n" % (name, op) for op, (name, _) in vc4_submit.PACKETS.items())
    header += "".join("#define VC4_PACKET_%s_SIZE\t%d\n" % v for v in vc4_submit.PACKETS.values())
    check(vc4_submit.check_packet_table(header) == len(vc4_submit.PACKETS), "the opcode table against itself")
    try:
        vc4_submit.check_packet_table(header.replace("FLUSH_SIZE\t1", "FLUSH_SIZE\t2"))
        check(False, "a header with another FLUSH size was accepted")
    except ValueError:
        check(True, "")
    print("vc4_selftest: %d checks passed" % checks)


if __name__ == "__main__":
    main()
