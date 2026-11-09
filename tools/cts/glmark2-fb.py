#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Decode a glmark2 image's frame dump (CONFIG_GLMARK2_FRAME_DUMP) to a PNG.

    tools/cts/glmark2-fb.py <console log> <out.png> [--scene SUBSTRING]
    tools/cts/glmark2-fb.py --rundir <run directory> <png directory>
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

--rundir: a mali-glmark2-run.sh run directory (log/<n>.out and .err, scenes.tsv).
Every scene whose log holds a dump is written to <png directory>/<NN>-<slug>.png
(NN: the scene's number, two digits; slug: its scene text from scenes.tsv, or
none without one) and prints one line: `glmark2-fb: <log>: <w>x<h> crc32 <hex>
ok`, `... no dump` or `... damaged (<reason>)`. Exit 0 when no dump is damaged,
whether or not some scenes have none; 1 when one is; 2 on a bad directory.
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


def slug(text):
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")[:48]


def run_dir(rundir, pngdir):
    import os
    logdir = os.path.join(rundir, "log")
    if not os.path.isdir(logdir):
        print("glmark2-fb: no log directory in " + rundir)
        return 2
    scenes = {}
    try:
        with open(os.path.join(rundir, "scenes.tsv"), errors="replace") as f:
            rows = [x.split("\t")[0] for x in f.read().splitlines()]
        for n, scene in enumerate(rows[1:] if rows and rows[0] == "scene" else rows, 1):
            scenes[n] = scene
    except OSError:
        pass
    ids = {}
    for fn in os.listdir(logdir):
        stem, _, ext = fn.rpartition(".")
        if stem.isdigit() and ext in ("out", "err"):
            ids.setdefault(int(stem), []).append(fn)
    os.makedirs(pngdir, exist_ok=True)
    rc = 0
    for n in sorted(ids):
        fns = sorted(ids[n])
        name = fns[0] if len(fns) == 1 else "%d.out" % n
        text = ""
        for fn in fns:
            with open(os.path.join(logdir, fn), errors="replace") as f:
                text += f.read() + "\n"
        r = decode(text)
        if r is None:
            print("glmark2-fb: %s: no dump" % name)
            continue
        _, w, h, rows, problems, end, pic = r
        reason = None
        if problems:
            reason = problems[0] + ("" if len(problems) == 1 else " and %d more" % (len(problems) - 1))
        elif end is None:
            reason = "no end line"
        else:
            crc = binascii.crc32(b"".join(rows)) & 0xFFFFFFFF
            if crc != end:
                reason = "frame crc32 %08x, board %08x" % (crc, end)
            elif pic is not None:
                diff = check_picture(w, h, rows, pic)
                if diff:
                    reason = "picture value differs: " + diff[0]
        s = slug(scenes.get(n, ""))
        png = os.path.join(pngdir, "%02d%s.png" % (n, "-" + s if s else ""))
        write_png(png, w, h, [x if x is not None else bytes(DAMAGED) * w for x in rows])
        if reason:
            print("glmark2-fb: %s: damaged (%s)" % (name, reason))
            rc = 1
        else:
            print("glmark2-fb: %s: %dx%d crc32 %08x ok" % (name, w, h, end))
    return rc


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
    # --rundir: two scenes with dumps, one without, one damaged
    import io
    import contextlib
    rd = os.path.join(d, "run")
    os.makedirs(os.path.join(rd, "log"))
    with open(os.path.join(rd, "scenes.tsv"), "w") as f:
        f.write("scene\tcommit\n[build] use-vbo=false\tx\n[texture] t=1\tx\n[shading]\tx\n[jellyfish]\tx\n")
    dam = list(lines)
    del dam[10]
    for n, ls in ((1, lines), (2, lines), (3, ["no dump here"]), (4, dam)):
        with open(os.path.join(rd, "log", "%d.out" % n), "w") as f:
            f.write("\n".join(ls) + "\n")
    open(os.path.join(rd, "log", "3.err"), "w").close()
    pd = os.path.join(d, "pngs")
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rc = run_dir(rd, pd)
    out = buf.getvalue().splitlines()
    expect("rundir exit (a damaged dump)", rc, 1)
    ok_line = "%dx%d crc32 %08x ok" % (w, h, binascii.crc32(b"".join(rows)) & 0xFFFFFFFF)
    expect("rundir lines", [x.split(" (")[0] for x in out],
           ["glmark2-fb: 1.out: " + ok_line, "glmark2-fb: 2.out: " + ok_line,
            "glmark2-fb: 3.out: no dump", "glmark2-fb: 4.out: damaged"])
    expect("rundir pngs", sorted(os.listdir(pd)),
           ["01-build-use-vbo-false.png", "02-texture-t-1.png", "04-jellyfish.png"])
    with open(os.path.join(pd, "01-build-use-vbo-false.png"), "rb") as f:
        expect("rundir png", f.read(8), b"\x89PNG\r\n\x1a\n")
    os.remove(os.path.join(rd, "log", "4.out"))
    with contextlib.redirect_stdout(io.StringIO()):
        expect("rundir exit (none damaged, one no dump)", run_dir(rd, pd), 0)
        expect("rundir bad directory", run_dir(os.path.join(d, "nope"), pd), 2)
    print("self-test: " + ("ok" if ok else "FAILED"))
    return 0 if ok else 1


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    if len(argv) == 4 and argv[1] == "--rundir":
        return run_dir(argv[2], argv[3])
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
