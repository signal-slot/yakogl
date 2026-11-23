#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The screen proof's nonce on the host: a new one, and the bars a picture shows.

    t6-screen-nonce.py new
        Prints a new nonce: an order of the eight bar colours that
        t6-kms-mode.py's nonce_ok() accepts.
    t6-screen-nonce.py decode <picture.ppm> [<nonce>]
        Reads a binary PPM (P6, as `djpeg -pnm` writes it from the capture
        unit's JPEG) and prints one line: "t6-screen-nonce: <w>x<h>: bars
        <one digit per bar, ? for a bar of no one colour>". Exit 0 when the
        bars are <nonce> (without one: when they are any nonce), 1 when
        not, 2 for a file that is not such a picture or a usage error.
    t6-screen-nonce.py --self-test
        The holder's own drawing (draw_nonce() of t6-kms-mode.py, beside
        this) decoded back for several nonces, also through the changes an
        HDMI capture and a JPEG make; pictures that must not pass (black,
        white, a console, another run's nonce, the colour bars a capture
        unit shows without a signal); the PPM reader's refusals; and, when
        the host has cjpeg and djpeg, the round trip through a real JPEG.

Why it exists. nanopc-t6.sh --go with T6_SCREEN proves that the capture
unit on the board's HDMI output shows this run's screen: the holder draws
the run's nonce as vertical bars (t6-kms-mode.py, "The screen proof"), and
the runner asks the unit for pictures until one decodes to the nonce
(driver/docs/nanopc-t6-display.md, section 2(a)).

How a picture is read. The picture is cut into the holder's NONCE_BARS bars
by the same rule (bar k spans w * k // NONCE_BARS up to w * (k + 1) //
NONCE_BARS). In each bar a grid of samples away from the edges (the middle
half of the bar's width and of the picture's height) is read: a channel at
HIGH or above is on, at LOW or below off, anything between is neither. A
bar is the colour that SHARE of its samples have; otherwise it is "?". The
thresholds leave room for what HDMI capture and JPEG do to a colour (a
limited range of 16..235, noise, chroma blur at the bars' edges), not for a
colour that is half on.

Only the Python standard library; no picture library on the host.
"""

import importlib.util
import os
import secrets
import sys

HIGH = 160
LOW = 96
SHARE = 0.8


def holder():
    """t6-kms-mode.py, beside this: the nonce rule and the drawing."""
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "t6-kms-mode.py")
    spec = importlib.util.spec_from_file_location("t6_kms_mode", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class NotAPicture(Exception):
    pass


def read_ppm(data):
    """(w, h, rgb bytes) of a binary PPM with a maxval of 255 or less."""
    fields, i = [], 0
    while len(fields) < 4:
        while i < len(data) and data[i:i + 1].isspace():
            i += 1
        if data[i:i + 1] == b"#":
            while i < len(data) and data[i:i + 1] not in (b"\n", b"\r"):
                i += 1
            continue
        j = i
        while j < len(data) and not data[j:j + 1].isspace() and data[j:j + 1] != b"#":
            j += 1
        if j == i:
            raise NotAPicture("the PPM header ends after %d of its 4 fields" % len(fields))
        fields.append(data[i:j])
        i = j
        if fields[0] != b"P6":
            raise NotAPicture("not a colour PPM (P6): it starts with %r" % fields[0][:8])
    try:
        w, h, maxval = (int(f) for f in fields[1:])
    except ValueError:
        raise NotAPicture("the PPM header's size or maxval is not a number: %r" % b" ".join(fields[1:]))
    if w <= 0 or h <= 0 or not 0 < maxval <= 255:
        raise NotAPicture("a PPM of %dx%d with maxval %d (one byte per channel only)" % (w, h, maxval))
    pixels = data[i + 1:i + 1 + 3 * w * h]
    if len(pixels) != 3 * w * h:
        raise NotAPicture("a PPM of %dx%d holds %d of its %d bytes" % (w, h, len(pixels), 3 * w * h))
    if maxval != 255:
        pixels = bytes(v * 255 // maxval for v in pixels)
    return w, h, pixels


def bars(w, h, rgb, n):
    """One character per bar: its colour's digit (bit 2 red, bit 1 green,
    bit 0 blue), or "?"."""
    out = []
    ys = range(h // 4, max(h // 4 + 1, 3 * h // 4), max(1, h // 64))
    for k in range(n):
        x0, x1 = w * k // n, w * (k + 1) // n
        bw = x1 - x0
        xs = range(x0 + bw // 4, max(x0 + bw // 4 + 1, x1 - bw // 4), max(1, bw // 32))
        counts = [0] * 8
        total = 0
        for y in ys:
            row = y * w
            for x in xs:
                p = 3 * (row + x)
                total += 1
                digit = 0
                for c in rgb[p:p + 3]:
                    if c >= HIGH:
                        digit = digit * 2 + 1
                    elif c <= LOW:
                        digit = digit * 2
                    else:
                        digit = -1
                        break
                if digit >= 0:
                    counts[digit] += 1
        best = max(range(8), key=lambda d: counts[d])
        out.append(str(best) if total and counts[best] >= SHARE * total else "?")
    return "".join(out)


def decode(path, nonce=None, mod=None):
    """(exit code, line)."""
    mod = mod or holder()
    try:
        with open(path, "rb") as f:
            w, h, rgb = read_ppm(f.read())
    except OSError as e:
        return 2, "t6-screen-nonce: %s: %s" % (path, e.strerror)
    except NotAPicture as e:
        return 2, "t6-screen-nonce: %s: %s" % (path, e)
    got = bars(w, h, rgb, mod.NONCE_BARS)
    ok = got == nonce if nonce else mod.nonce_ok(got)
    return (0 if ok else 1), "t6-screen-nonce: %dx%d: bars %s" % (w, h, got)


def new_nonce(mod):
    while True:
        digits = list("01234567")
        secrets.SystemRandom().shuffle(digits)
        n = "".join(digits)
        if mod.nonce_ok(n):
            return n


def usage():
    print("usage: t6-screen-nonce.py new\n"
          "       t6-screen-nonce.py decode <picture.ppm> [<nonce>]\n"
          "       t6-screen-nonce.py --self-test", file=sys.stderr)
    return 2


# ---- the self-test ------------------------------------------------------------

def self_test():
    import ast
    import random
    import shutil
    import subprocess
    import tempfile

    mod = holder()
    count = [0]
    failed = []
    # The lines whose check has run; every check line must be among them at
    # the end, except those of jpeg_round_trip() when it is skipped by name.
    ran = set()

    def check(cond, what):
        count[0] += 1
        ran.add(sys._getframe(1).f_lineno)
        print("%s %s" % (what, "ok" if cond else "FAIL"))
        if not cond:
            failed.append(what)

    def screen(nonce, w=1920, h=1080, pad=64):
        """The holder's buffer for the nonce, as the unit's RGB picture."""
        pitch = 4 * w + pad
        buf = bytearray(pitch * h)
        mod.draw_nonce(buf, w, h, pitch, nonce)
        row = buf[:4 * w]
        rgb = bytearray(3 * w)
        rgb[0::3], rgb[1::3], rgb[2::3] = row[2::4], row[1::4], row[0::4]
        return bytes(rgb) * h

    def plain(value, w=1920, h=1080):
        return bytes((value,)) * (3 * w * h)

    def captured(rgb, w, seed):
        """What HDMI capture and JPEG may do: a limited range (16..235),
        noise of +-24, and each bar's first and last 8 pixels blurred into
        the neighbour's colour (chroma subsampling)."""
        rnd = random.Random(seed)
        out = bytearray(16 + v * 219 // 255 for v in rgb)
        for i in range(0, len(out), 7):
            out[i] = max(0, min(255, out[i] + rnd.randint(-24, 24)))
        h = len(out) // (3 * w)
        for k in range(1, mod.NONCE_BARS):
            x = w * k // mod.NONCE_BARS
            for y in range(h):
                p = 3 * (y * w + x)
                left, right = out[p - 3:p], out[p:p + 3]
                mix = bytes((a + b) // 2 for a, b in zip(left, right))
                out[p - 24:p + 24] = mix * 16
        return bytes(out)

    def ppm(rgb, w, h, header=None):
        return (header or b"P6\n%d %d\n255\n" % (w, h)) + rgb

    tmp = tempfile.mkdtemp(prefix="t6-screen-nonce.")
    try:
        def run(data, nonce=None):
            path = os.path.join(tmp, "p.ppm")
            with open(path, "wb") as f:
                f.write(data)
            return decode(path, nonce, mod)

        nonces = ["36104527", "70615243", "12345670", "52741630"]
        for n in nonces:
            check(mod.nonce_ok(n), "nonce: %s is one" % n)
            rc, line = run(ppm(screen(n), 1920, 1080), n)
            check(rc == 0 and line == "t6-screen-nonce: 1920x1080: bars %s" % n,
                  "decode: the holder's drawing of %s decodes back (%s)" % (n, line))
            rc, line = run(ppm(captured(screen(n), 1920, int(n)), 1920, 1080), n)
            check(rc == 0, "decode: %s through limited range, noise and blurred edges (%s)" % (n, line))
        rc, line = run(ppm(screen("36104527", 1024, 768, 0), 1024, 768), "36104527")
        check(rc == 0 and line.endswith("1024x768: bars 36104527"), "decode: at 1024x768 too (%s)" % line)
        rc, line = run(ppm(screen("36104527"), 1920, 1080))
        check(rc == 0, "decode without a nonce: the bars are a nonce (%s)" % line)

        # What must not pass.
        rc, line = run(ppm(screen("70615243"), 1920, 1080), "36104527")
        check(rc == 1 and line.endswith("bars 70615243"), "decode: another run's nonce is not this one (%s)" % line)
        for what, data, want in (
                ("black", plain(0), "00000000"),
                ("white", plain(255), "77777777"),
                ("grey", plain(128), "????????"),
                ("black at limited range", plain(16), "00000000")):
            rc, line = run(ppm(data, 1920, 1080), "36104527")
            check(rc == 1 and line.endswith("bars " + want), "decode: a %s screen does not pass (%s)" % (what, line))
            rc, line = run(ppm(data, 1920, 1080))
            check(rc == 1, "decode: a %s screen is no nonce at all (%s)" % (what, line))
        # A console: black, every other row of text cells grey-white.
        con = bytearray(plain(0))
        for y in range(1080):
            if (y // 16) % 2 == 0:
                for x in range(0, 1920, 9):
                    p = 3 * (y * 1920 + x)
                    con[p:p + 12] = b"\xc0" * 12
        rc, line = run(ppm(bytes(con), 1920, 1080))
        check(rc == 1 and "7" not in line.split()[-1], "decode: a console screen is no nonce (%s)" % line)
        for bars_order in mod.NONCE_NOT_USED:
            rc, line = run(ppm(screen(bars_order), 1920, 1080))
            check(rc == 1 and line.endswith("bars " + bars_order),
                  "decode: the bars %s decode, but are no nonce (%s)" % (bars_order, line))
        # Bars whose channels all lie between LOW and HIGH (100..150) are
        # no colour at all.
        dim = bytes(100 + v * 50 // 255 for v in screen("36104527"))
        rc, line = run(ppm(dim, 1920, 1080), "36104527")
        check(rc == 1 and line.endswith("bars ????????"), "decode: bars of 100..150 are read as no colour (%s)" % line)

        # The PPM reader.
        rgb = screen("36104527", 16, 4, 0)
        rc, line = run(ppm(rgb, 16, 4, b"P6\n# djpeg\n16 4 # size\n255\n"), "36104527")
        check(rc == 0, "ppm: comments in the header (%s)" % line)
        for what, data, want in (
                ("a grey PPM", b"P5\n16 4\n255\n" + bytes(64), "not a colour PPM (P6)"),
                ("two bytes per channel", ppm(rgb + rgb, 16, 4, b"P6\n16 4\n65535\n"), "one byte per channel only"),
                ("a short file", ppm(rgb[:-1], 16, 4), "holds 191 of its 192 bytes"),
                ("a header cut short", b"P6\n16 4", "header ends after 3 of its 4 fields"),
                ("a size that is no number", b"P6\n16 x\n255\n", "is not a number"),
                ("a JPEG", b"\xff\xd8\xff\xe0", "not a colour PPM")):
            rc, line = run(data)
            check(rc == 2 and want in line, "ppm: %s is refused (%s)" % (what, line))
        rc, line = decode(os.path.join(tmp, "absent.ppm"), None, mod)
        check(rc == 2 and "No such file" in line, "ppm: a missing file is refused (%s)" % line)

        made = {new_nonce(mod) for _ in range(200)}
        check(all(mod.nonce_ok(n) for n in made) and len(made) > 150,
              "new: 200 nonces, each one nonce_ok() accepts, %d different" % len(made))

        skipped = None
        cjpeg, djpeg = shutil.which("cjpeg"), shutil.which("djpeg")

        def jpeg_round_trip():
            for n in nonces[:2]:
                src = os.path.join(tmp, "s.ppm")
                with open(src, "wb") as f:
                    f.write(ppm(captured(screen(n), 1920, 7), 1920, 1080))
                jpg = subprocess.run([cjpeg, "-quality", "75", src], stdout=subprocess.PIPE, check=True).stdout
                back = subprocess.run([djpeg, "-pnm"], input=jpg, stdout=subprocess.PIPE, check=True).stdout
                rc, line = run(back, n)
                check(rc == 0, "jpeg: %s through cjpeg -quality 75 and djpeg -pnm (%d bytes) decodes back (%s)"
                      % (n, len(jpg), line))

        if cjpeg and djpeg:
            jpeg_round_trip()
        else:
            skipped = "the JPEG round trip (no %s on this host)" % " and no ".join(
                t for t, p in (("cjpeg", cjpeg), ("djpeg", djpeg)) if not p)
            print("skip: %s" % skipped)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    with open(os.path.abspath(__file__)) as f:
        tree = ast.parse(f.read())
    calls = [(c.lineno, c.end_lineno) for c in ast.walk(tree)
             if isinstance(c, ast.Call) and isinstance(c.func, ast.Name) and c.func.id == "check"]
    if skipped:
        fn = next(d for d in ast.walk(tree) if isinstance(d, ast.FunctionDef) and d.name == "jpeg_round_trip")
        calls = [(a, b) for a, b in calls if not fn.lineno <= a <= fn.end_lineno]
    not_run = [a for a, b in calls if not ran.intersection(range(a, b + 1))]
    print("t6-screen-nonce self-test: %d checks run, %d failed%s%s" % (
        count[0], len(failed), ", check lines not run: %s" % not_run if not_run else "",
        ", skipped: %s" % skipped if skipped else ""))
    return 0 if not failed and not not_run and count[0] else 1


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    if argv[1:] == ["new"]:
        print(new_nonce(holder()))
        return 0
    if argv[1:2] == ["decode"] and len(argv) in (3, 4):
        rc, line = decode(argv[2], argv[3] if len(argv) == 4 else None)
        print(line)
        return rc
    return usage()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
