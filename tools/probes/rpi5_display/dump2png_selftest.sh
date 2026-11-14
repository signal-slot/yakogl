#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# dump2png_selftest: dump2png.py against synthetic dumps of every format. Each
# dump is converted, the PNG decoded again with a small standard-library
# decoder, and the pixels compared with the ones the dump was made from. Also
# the hex input forms, pitch padding, --crop, --stats and the bad-size error.
# The last line is "N of N checks, 0 failed" (exit 0) or "N of M checks, K
# failed" (exit 1). Python runs with -I; standard library only.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)" || exit 1
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/harness.py" <<'PYEOF'
import os
import struct
import subprocess
import sys
import zlib

TOOL, TMP = sys.argv[1], sys.argv[2]
checks = []


def check(name, ok, detail=""):
    checks.append(ok)
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else "  " + str(detail)))


def run(args, stdin=None):
    p = subprocess.run([sys.executable, "-I", TOOL] + args, input=stdin,
                       capture_output=True)
    return p.returncode, p.stdout.decode(), p.stderr.decode()


def decode_png(path):
    d = open(path, "rb").read()
    assert d[:8] == b"\x89PNG\r\n\x1a\n"
    pos, idat, hdr = 8, b"", None
    while pos < len(d):
        n, t = struct.unpack(">I4s", d[pos:pos + 8])
        body = d[pos + 8:pos + 8 + n]
        crc = struct.unpack(">I", d[pos + 8 + n:pos + 12 + n])[0]
        assert crc == zlib.crc32(t + body) & 0xFFFFFFFF, "bad crc"
        if t == b"IHDR":
            hdr = struct.unpack(">IIBBBBB", body)
        elif t == b"IDAT":
            idat += body
        pos += 12 + n
    w, h, depth, ctype = hdr[:4]
    assert depth == 8 and ctype in (2, 6)
    ch = 3 if ctype == 2 else 4
    raw = zlib.decompress(idat)
    assert len(raw) == h * (1 + w * ch)
    rows = []
    for y in range(h):
        off = y * (1 + w * ch)
        assert raw[off] == 0  # filter None
        rows.append([tuple(raw[off + 1 + x * ch:off + 1 + (x + 1) * ch]) for x in range(w)])
    return w, h, ch, rows


W, H = 5, 4
# expected RGB(A) pixels, chosen so that every component differs
pix = [[((x * 50 + 7) & 255, (y * 60 + x * 3 + 11) & 255, (x * 20 + y * 40 + 200) & 255, (x * 9 + y * 70 + 33) & 255)
        for x in range(W)] for y in range(H)]


def enc(fmt, p):
    r, g, b, a = p
    if fmt == "XRGB8888":
        return bytes([b, g, r, 0xEE])
    if fmt == "ARGB8888":
        return bytes([b, g, r, a])
    if fmt == "XBGR8888":
        return bytes([r, g, b, 0xEE])
    if fmt == "RGB565":
        return (((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)).to_bytes(2, "little")
    return (((b >> 3) << 11) | ((g >> 2) << 5) | (r >> 3)).to_bytes(2, "little")


def expect(fmt, p):
    r, g, b, a = p
    if fmt.endswith("565"):
        r, g, b = r >> 3, g >> 2, b >> 3
        r, g, b = (r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)
    return (r, g, b, a) if fmt == "ARGB8888" else (r, g, b)


def dump(fmt, pitch=None, fill=0xA5):
    bpp = 2 if fmt.endswith("565") else 4
    pitch = pitch or W * bpp
    out = bytearray()
    for row in pix:
        line = b"".join(enc(fmt, p) for p in row)
        out += line + bytes([fill]) * (pitch - len(line))
    return bytes(out)


def wr(name, data, mode="wb"):
    path = os.path.join(TMP, name)
    open(path, mode).write(data)
    return path


def convert(fmt, src, png, extra=()):
    return run([src, "-o", png, "--width", str(W), "--height", str(H),
                "--format", fmt] + list(extra))


for fmt in ("XRGB8888", "ARGB8888", "XBGR8888", "RGB565", "BGR565"):
    bpp = 2 if fmt.endswith("565") else 4
    want = [[expect(fmt, p) for p in row] for row in pix]
    for pitch in (W * bpp, W * bpp + 12):
        src = wr("d.bin", dump(fmt, pitch))
        png = os.path.join(TMP, "o.png")
        rc, _, err = convert(fmt, src, png, ["--pitch", str(pitch)])
        ok = rc == 0
        if ok:
            w, h, ch, rows = decode_png(png)
            ok = (w, h) == (W, H) and rows == want
        check("%s bin pitch %d" % (fmt, pitch), ok, err)

    # hex: address column + tag, one 8-digit word (or 4-digit halfword) per
    # token, and the same as plain bytes
    data = dump(fmt)
    step = 4 if bpp == 4 else 2
    lines = []
    for off in range(0, len(data), 16):
        toks = " ".join(data[i:i + step][::-1].hex() for i in range(off, min(off + 16, len(data)), step))
        lines.append("fb: 0x%08x: %s" % (0x1000 + off, toks))
    src = wr("d.hex", "\n".join(lines) + "\n", "w")
    png = os.path.join(TMP, "h.png")
    rc, _, err = convert(fmt, src, png)
    ok = rc == 0 and decode_png(png)[3] == want
    check("%s hex words with tag and address" % fmt, ok, err)

    lines = [data[i:i + 16].hex(" ") for i in range(0, len(data), 16)]
    src = wr("b.hex", "# comment\nheading\n" + "\n".join(lines) + "\n", "w")
    rc, _, err = convert(fmt, src, png, ["--input", "hex"])
    check("%s hex bytes with comment and heading" % fmt,
          rc == 0 and decode_png(png)[3] == want, err)

# one hex line per row as a single run of digits
data = dump("XRGB8888")
src = wr("r.hex", "".join("row%d: %s\n" % (y, data[y * 20:(y + 1) * 20].hex()) for y in range(H)), "w")
png = os.path.join(TMP, "r.png")
rc, _, err = convert("XRGB8888", src, png)
check("hex one run of digits per row", rc == 0 and decode_png(png)[3] == [[expect("XRGB8888", p) for p in r] for r in pix], err)

# stdin
rc, out, err = run(["-", "-o", png, "--width", str(W), "--height", str(H), "--format", "XRGB8888"],
                   stdin=dump("XRGB8888"))
check("stdin input", rc == 0 and decode_png(png)[3] == [[expect("XRGB8888", p) for p in r] for r in pix], err)

# crop
src = wr("c.bin", dump("XRGB8888"))
rc, _, err = convert("XRGB8888", src, png, ["--crop", "1,1,3,2"])
w, h, ch, rows = decode_png(png)
check("crop", rc == 0 and (w, h) == (3, 2)
      and rows == [[expect("XRGB8888", p) for p in r[1:4]] for r in pix[1:3]], err)
rc, _, err = convert("XRGB8888", src, png, ["--crop", "3,0,3,2"])
check("crop outside is an error", rc == 2 and "crop" in err, err)

# bad sizes
short = wr("s.bin", dump("XRGB8888")[:-5])
rc, _, err = convert("XRGB8888", short, os.path.join(TMP, "none.png"))
check("short dump is an error (exit 2, no PNG)",
      rc == 2 and "need" in err and not os.path.exists(os.path.join(TMP, "none.png")), err)
rc, _, err = convert("XRGB8888", src, png, ["--pitch", "8"])
check("pitch below width*bpp is an error", rc == 2, err)
rc, _, err = convert("XRGB8888", wr("long.bin", dump("XRGB8888") + b"\0" * 9), png)
check("long dump warns and converts", rc == 0 and "extra" in err, err)

# stats: top half text-like (two colours, mostly background), bottom a gradient
SW, SH = 16, 8
buf = bytearray()
for y in range(SH):
    for x in range(SW):
        if y < SH // 2:
            c = (255, 255, 255) if (x * 7 + y) % 5 == 0 else (0, 0, 0)
        else:
            c = (x * 16, y * 30, 255 - x * 16)
        buf += bytes([c[2], c[1], c[0], 0])
src = wr("st.bin", bytes(buf))
rc, out, err = run([src, "--width", str(SW), "--height", str(SH), "--format", "XRGB8888", "--stats"])
cells = [ln for ln in out.splitlines() if ln.startswith("cell")]
check("stats split screen verdicts", rc == 0 and len(cells) == 2
      and cells[0].endswith("text") and cells[1].endswith("pattern"), out + err)
rc, out, err = run([src, "--width", str(SW), "--height", str(SH), "--format", "XRGB8888",
                    "--stats", "--grid", "2x1", "--crop", "0,4,16,4"])
check("stats grid on a crop", rc == 0 and out.count("cell") == 2 and "pattern" in out, out + err)
flat = wr("f.bin", bytes([0x10, 0x20, 0x30, 0]) * (SW * SH))
rc, out, err = run([flat, "--width", str(SW), "--height", str(SH), "--format", "XRGB8888", "--stats"])
check("stats flat mean", rc == 0 and "mean=(48.0,32.0,16.0)" in out and out.rstrip().endswith("flat"), out + err)

# standard library only
src_text = open(TOOL).read()
mods = set()
for ln in src_text.splitlines():
    t = ln.split()
    if len(t) > 1 and t[0] in ("import", "from") and not ln.startswith(" "):
        mods.update(m.strip(",") for m in (t[1:] if t[0] == "import" else t[1:2]) if m != "as")
check("imports are standard library", mods <= set(sys.stdlib_module_names), sorted(mods))

bad = checks.count(False)
print("%d of %d checks, %d failed" % (len(checks) - bad, len(checks), bad))
sys.exit(1 if bad else 0)
PYEOF

python3 -I "$tmp/harness.py" "$here/dump2png.py" "$tmp"
exit $?
