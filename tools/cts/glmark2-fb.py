#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Decode a glmark2 image's frame dump (CONFIG_GLMARK2_FRAME_DUMP) to a PNG.

    tools/cts/glmark2-fb.py <console log> <out.png> [--scene SUBSTRING]
    tools/cts/glmark2-fb.py --self-test

The log has, per scene (driver/docs/glmark2.md, "The frame dump"):

    glmark2-fb: <scene> <w>x<h> begin
    glmark2-fb: <y> <crc16 hex> <base64 of the row's RGB888>      (one per row)
    glmark2-fb: end crc32 <hex>

A row whose line is missing, garbled, of the wrong length or whose crc16 does
not match is reported and drawn magenta; the frame crc32 and the scene's
`glmark2-picture:` line (probes, crc16 grid) are checked when every row is
good. Exit 0: a complete picture that agrees; 1: damaged or missing rows or a
disagreement (rerun the board, never patch pixels); 2: no dump in the log.
Stdlib only.
"""
import base64
import binascii
import re
import struct
import sys
import zlib

DAMAGED = (255, 0, 255)


def crc16(data, crc=0xFFFF):
    """CRC-16/CCITT-FALSE, the image's crc16_byte."""
    for b in data:
        crc ^= b << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return crc


def write_png(path, w, h, rows):
    """rows: h byte strings of w*3 bytes."""
    raw = b"".join(b"\0" + r for r in rows)

    def chunk(tag, body):
        c = struct.pack(">I", len(body)) + tag + body
        return c + struct.pack(">I", binascii.crc32(tag + body) & 0xFFFFFFFF)

    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))
    with open(path, "wb") as f:
        f.write(png)


def decode(text, scene=None):
    """-> (scene, w, h, rows (None where bad), problems, end crc32 or None, picture text or None)"""
    begin = re.compile(r"glmark2-fb: (.*) (\d+)x(\d+) begin\s*$")
    cur = None
    dumps = []
    for line in text.splitlines():
        i = line.find("glmark2-fb: ")
        if i < 0:
            continue
        line = line[i:]
        m = begin.match(line)
        if m:
            cur = {"scene": m.group(1), "w": int(m.group(2)), "h": int(m.group(3)),
                   "lines": {}, "end": None, "bad": []}
            dumps.append(cur)
        elif cur is not None:
            m = re.match(r"glmark2-fb: end crc32 ([0-9a-f]{8})\s*$", line)
            if m:
                cur["end"] = int(m.group(1), 16)
                continue
            m = re.match(r"glmark2-fb: (\d+) ([0-9a-f]{4}) (\S+)\s*$", line)
            if m:
                cur["lines"][int(m.group(1))] = (int(m.group(2), 16), m.group(3))
            elif not line.startswith("glmark2-fb: end"):
                cur["bad"].append(line[:60])
    dumps = [d for d in dumps if scene is None or scene in d["scene"]]
    if not dumps:
        return None
    d = dumps[-1]
    w, h = d["w"], d["h"]
    rows, problems = [], []
    for y in range(h):
        got = d["lines"].get(y)
        row = None
        if got is None:
            problems.append("row %d missing" % y)
        else:
            try:
                row = base64.b64decode(got[1], validate=True)
            except (binascii.Error, ValueError):
                problems.append("row %d not base64" % y)
            else:
                if len(row) != w * 3:
                    problems.append("row %d has %d bytes, not %d" % (y, len(row), w * 3))
                    row = None
                elif crc16(row) != got[0]:
                    problems.append("row %d crc16 differs" % y)
                    row = None
        rows.append(row)
    for y in sorted(d["lines"]):
        if y >= h:
            problems.append("row %d is outside the %d rows" % (y, h))
    if d["bad"]:
        problems.append("%d unreadable fb line(s), first: %r" % (len(d["bad"]), d["bad"][0]))
    pic = None
    key = "glmark2-picture: " + d["scene"] + ": "
    for line in text.splitlines():
        i = line.find(key)
        if i >= 0:
            pic = line[i + len(key):].strip()
    return d["scene"], w, h, rows, problems, d["end"], pic


def picture_value(w, h, rows):
    """The image's capture_picture over the decoded rows (RGB; alpha reads ff)."""
    def px(x, y):
        return rows[y][x * 3:x * 3 + 3]

    pts = [(w // 2, h // 2), (w // 4, h // 4), (w * 3 // 4, h // 4), (w // 4, h * 3 // 4),
           (w * 3 // 4, h * 3 // 4)]
    probes = [px(x, y) for x, y in pts]
    c = cq = 0xFFFF
    s = [0, 0, 0]
    for gy in range(30):
        for gx in range(40):
            q = px((2 * gx + 1) * w // 80, (2 * gy + 1) * h // 60)
            c = crc16(q, c)
            cq = crc16(bytes(v >> 4 for v in q), cq)
            for i in range(3):
                s[i] += q[i]
    return probes, c, cq, [v // 1200 for v in s]


def check_picture(w, h, rows, pic):
    """Compare with a `glmark2-picture:` text; -> list of disagreements."""
    probes, c, cq, mean = picture_value(w, h, rows)
    out = []
    m = re.search(r"probes c (\w+), tl (\w+), tr (\w+), bl (\w+), br (\w+); crc16 (\w+), "
                  r"crc16q (\w+); mean (\d+) (\d+) (\d+)", pic)
    if not m:
        return ["the picture line is not readable: " + pic]
    for name, p, want in zip(("c", "tl", "tr", "bl", "br"), probes, m.groups()[:5]):
        if p.hex() != want[:6]:
            out.append("probe %s: dump %s, picture %s" % (name, p.hex(), want[:6]))
    if "%04x" % c != m.group(6):
        out.append("crc16: dump %04x, picture %s" % (c, m.group(6)))
    if "%04x" % cq != m.group(7):
        out.append("crc16q: dump %04x, picture %s" % (cq, m.group(7)))
    if [str(v) for v in mean] != list(m.groups()[7:]):
        out.append("mean: dump %s, picture %s" % (mean, m.groups()[7:]))
    return out


def run(log, png, scene=None):
    with open(log, errors="replace") as f:
        r = decode(f.read(), scene)
    if r is None:
        print("no glmark2-fb dump in " + log)
        return 2
    name, w, h, rows, problems, end, pic = r
    bad = sum(1 for x in rows if x is None)
    write_png(png, w, h, [x if x is not None else bytes(DAMAGED) * w for x in rows])
    print("%s: %dx%d, %d of %d rows good -> %s" % (name, w, h, h - bad, h, png))
    for p in problems[:40]:
        print("  damaged: " + p)
    if len(problems) > 40:
        print("  ... %d more" % (len(problems) - 40))
    if bad or problems:
        print("rerun the board for the damaged rows; pixels are never patched")
        return 1
    crc = binascii.crc32(b"".join(rows)) & 0xFFFFFFFF
    if end is None:
        print("  no end line (the log stops early)")
        return 1
    if crc != end:
        print("  frame crc32 differs: decoded %08x, board %08x" % (crc, end))
        return 1
    print("frame crc32 %08x agrees" % crc)
    if pic is None:
        print("no glmark2-picture line for this scene in the log: picture value not compared")
        return 0
    diff = check_picture(w, h, rows, pic)
    for d in diff:
        print("  picture value differs: " + d)
    if diff:
        return 1
    print("picture value agrees: probes, crc16, crc16q and mean of the dump match the board's line")
    return 0


def self_test():
    import os
    import tempfile
    w, h = 80, 60
    rows = [bytes((x * 3 + y) & 255 if c == 0 else (y * 4) & 255 if c == 1 else x & 255
                  for x in range(w) for c in range(3)) for y in range(h)]
    probes, c, cq, mean = picture_value(w, h, rows)
    pic = ("%dx%d; probes c %s00, tl %s00, tr %s00, bl %s00, br %s00; crc16 %04x, crc16q %04x; "
           "mean %d %d %d; black no" % ((w, h) + tuple(p.hex() for p in probes) + (c, cq) + tuple(mean)))
    lines = ["noise", "glmark2-fb: [a] b=c %dx%d begin" % (w, h)]
    for y, r in enumerate(rows):
        lines.append("[0.1] glmark2-fb: %d %04x %s" % (y, crc16(r), base64.b64encode(r).decode()))
    lines.append("glmark2-fb: end crc32 %08x" % (binascii.crc32(b"".join(rows)) & 0xFFFFFFFF))
    lines.append("glmark2-picture: [a] b=c: " + pic)
    d = tempfile.mkdtemp(prefix="glmark2-fb.")
    log, png = os.path.join(d, "log"), os.path.join(d, "out.png")
    ok = True

    def expect(what, got, want):
        nonlocal ok
        if got != want:
            ok = False
            print("FAIL %s: got %r, want %r" % (what, got, want))

    def put(ls):
        with open(log, "w") as f:
            f.write("\n".join(ls) + "\n")

    put(lines)
    expect("clean log", run(log, png), 0)
    with open(png, "rb") as f:
        data = f.read()
    expect("png signature", data[:8], b"\x89PNG\r\n\x1a\n")
    expect("png size", struct.unpack(">II", data[16:24]), (w, h))
    raw = zlib.decompress(data[data.index(b"IDAT") + 4:data.index(b"IEND") - 8])
    expect("png pixels", raw[1:1 + w * 3], rows[0])
    bad = list(lines)
    del bad[10]  # row 8 lost
    bad[20] = bad[20][:30] + "A" + bad[20][31:]  # row 19 garbled
    put(bad)
    expect("damaged log", run(log, png), 1)
    r = decode("\n".join(bad))
    expect("damaged rows", [y for y, x in enumerate(r[3]) if x is None], [8, 19])
    bad = [x.replace("probes c", "probes c ff", 1) if x.startswith("glmark2-picture") else x
           for x in lines]
    put(bad)
    expect("picture disagrees", run(log, png), 1)
    put(["nothing"])
    expect("no dump", run(log, png), 2)
    print("self-test: " + ("ok" if ok else "FAILED"))
    return 0 if ok else 1


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    scene = None
    if "--scene" in argv:
        i = argv.index("--scene")
        scene = argv[i + 1]
        del argv[i:i + 2]
    if len(argv) != 3:
        print(__doc__)
        return 2
    return run(argv[1], argv[2], scene)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
