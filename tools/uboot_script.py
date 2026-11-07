#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Build, check and read U-Boot script images (`mkimage -T script`), no mkimage.

    tools/uboot_script.py build SCRIPT -o OUT [--name TEXT] [--arch arm64|arm]
                          [--time SECONDS]
    tools/uboot_script.py check IMAGE
    tools/uboot_script.py extract IMAGE [-o OUT]
    tools/uboot_script.py --self-test [--mkimage PATH]

Why: the STM32MP257F-DK's U-Boot (ST's 2023.10) runs `boot.scr.uimg` from the
bootfs partition when its distro loop falls through extlinux
(docs/notes/pre-study-stm32mp257f-dk.md, section 1 "U-Boot's flow"; #1281).
The board runner needs to write such a script for a one-shot boot and to read
the one ST's image carries, and the gate host has no u-boot-tools. The format
is U-Boot's legacy image (include/image.h, tools/mkimage.c):

    64-byte header, all big-endian:
      0  ih_magic  0x27051956     4  ih_hcrc  CRC-32 of the header, hcrc = 0
      8  ih_time   seconds        12 ih_size  length of the data
      16 ih_load   0              20 ih_ep    0
      24 ih_dcrc   CRC-32 of the data
      28 ih_os 5 (linux)  29 ih_arch 22 (arm64) or 2 (arm)
      30 ih_type 6 (script)  31 ih_comp 0 (none)  32 ih_name[32]
    data, as mkimage writes a script from one file:
      u32 length of the script, u32 0 (the end of the length table),
      the script bytes (no padding after the last file)

ih_time follows mkimage: SOURCE_DATE_EPOCH when set, else the script's
mtime; --time overrides both, so a build is reproducible. The name is cut at
32 bytes like mkimage's strncpy (no NUL when it is exactly 32 long).

`check` validates what U-Boot's `source` command checks and more (magic, both
CRCs, type script, no compression, the length table inside the data) and
prints the header; `extract` writes the first file of the script. Every
refusal is one line on stderr and exit 2. When mkimage is on PATH (or named
by --mkimage) the self-test compares the bytes with it; otherwise it says it
skipped that.
"""
import argparse
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

IH_MAGIC = 0x27051956
IH_NMLEN = 32
HEADER = struct.Struct(">7I4B32s")
assert HEADER.size == 64
IH_OS_LINUX = 5
IH_ARCH = {"arm": 2, "arm64": 22}
IH_TYPE_SCRIPT = 6
IH_COMP_NONE = 0


class Refused(Exception):
    pass


def script_data(script):
    """The data part mkimage writes for `-T script` with one file."""
    return struct.pack(">II", len(script), 0) + script


def build(script, name="", arch="arm64", time=0):
    if arch not in IH_ARCH:
        raise Refused("arch %r: not one of %s" % (arch, ", ".join(sorted(IH_ARCH))))
    if not 0 <= time <= 0xFFFFFFFF:
        raise Refused("time %d does not fit 32 bits" % time)
    data = script_data(script)
    name_b = name.encode()[:IH_NMLEN]
    fields = [IH_MAGIC, 0, time, len(data), 0, 0, zlib.crc32(data),
              IH_OS_LINUX, IH_ARCH[arch], IH_TYPE_SCRIPT, IH_COMP_NONE, name_b]
    hcrc = zlib.crc32(HEADER.pack(*fields))
    fields[1] = hcrc
    return HEADER.pack(*fields) + data


def parse(image):
    """Validate a script image; return (header dict, list of file bytes)."""
    if len(image) < HEADER.size:
        raise Refused("%d bytes: shorter than the 64-byte header" % len(image))
    f = HEADER.unpack_from(image)
    hdr = dict(zip(("magic", "hcrc", "time", "size", "load", "ep", "dcrc",
                    "os", "arch", "type", "comp", "name"), f))
    if hdr["magic"] != IH_MAGIC:
        raise Refused("magic 0x%08x, not 0x%08x" % (hdr["magic"], IH_MAGIC))
    zeroed = bytearray(image[:HEADER.size])
    zeroed[4:8] = b"\0\0\0\0"
    if zlib.crc32(bytes(zeroed)) != hdr["hcrc"]:
        raise Refused("header CRC 0x%08x, computed 0x%08x"
                      % (hdr["hcrc"], zlib.crc32(bytes(zeroed))))
    data = image[HEADER.size:]
    if len(data) < hdr["size"]:
        raise Refused("data %d bytes, header says %d (truncated)" % (len(data), hdr["size"]))
    data = data[:hdr["size"]]
    if zlib.crc32(data) != hdr["dcrc"]:
        raise Refused("data CRC 0x%08x, computed 0x%08x" % (hdr["dcrc"], zlib.crc32(data)))
    if hdr["type"] != IH_TYPE_SCRIPT:
        raise Refused("type %d, not %d (script)" % (hdr["type"], IH_TYPE_SCRIPT))
    if hdr["comp"] != IH_COMP_NONE:
        raise Refused("compression %d, only none (0) is read" % hdr["comp"])
    # The length table: u32 lengths up to a 0; files follow, each but the
    # last padded to 4 bytes (image_multi_getimg).
    lens, off = [], 0
    while True:
        if off + 4 > len(data):
            raise Refused("length table runs past the data")
        (n,) = struct.unpack_from(">I", data, off)
        off += 4
        if n == 0:
            break
        lens.append(n)
    if not lens:
        raise Refused("length table is empty")
    files = []
    for i, n in enumerate(lens):
        if off + n > len(data):
            raise Refused("file %d: %d bytes at %d run past the data (%d)" % (i, n, off, len(data)))
        files.append(data[off:off + n])
        off += n
        if i + 1 < len(lens):
            off = (off + 3) & ~3
    if off != len(data):
        raise Refused("%d bytes after the last file" % (len(data) - off))
    hdr["name"] = hdr["name"].split(b"\0", 1)[0].decode("latin-1")
    return hdr, files


def describe(hdr, files):
    arch = {v: k for k, v in IH_ARCH.items()}.get(hdr["arch"], str(hdr["arch"]))
    return ("script image: name %r, arch %s, os %d, time %d, data %d bytes "
            "(dcrc 0x%08x, hcrc 0x%08x), %d file(s) %s"
            % (hdr["name"], arch, hdr["os"], hdr["time"], hdr["size"], hdr["dcrc"],
               hdr["hcrc"], len(files), "+".join(str(len(x)) for x in files)))


def default_time(path):
    sde = os.environ.get("SOURCE_DATE_EPOCH")
    if sde:
        try:
            return int(sde)
        except ValueError:
            raise Refused("SOURCE_DATE_EPOCH=%r is not a number" % sde)
    return int(os.stat(path).st_mtime)


def write_out(path, data):
    tmp = path + ".tmp"
    with open(tmp, "wb") as fh:
        fh.write(data)
    os.replace(tmp, path)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--mkimage", help="the mkimage the self-test compares with")
    sub = ap.add_subparsers(dest="cmd")
    b = sub.add_parser("build", help="write a script image")
    b.add_argument("script")
    b.add_argument("-o", "--out", required=True)
    b.add_argument("--name", default="")
    b.add_argument("--arch", default="arm64", choices=sorted(IH_ARCH))
    b.add_argument("--time", type=int)
    c = sub.add_parser("check", help="validate a script image and print its header")
    c.add_argument("image")
    e = sub.add_parser("extract", help="write the script of an image")
    e.add_argument("image")
    e.add_argument("-o", "--out")
    a = ap.parse_args(argv)
    if a.self_test:
        return self_test(a.mkimage or shutil.which("mkimage"))
    try:
        if a.cmd == "build":
            with open(a.script, "rb") as fh:
                script = fh.read()
            t = a.time if a.time is not None else default_time(a.script)
            img = build(script, a.name, a.arch, t)
            parse(img)
            write_out(a.out, img)
            print("wrote %s: %d bytes, script %d bytes" % (a.out, len(img), len(script)))
            return 0
        if a.cmd in ("check", "extract"):
            with open(a.image, "rb") as fh:
                hdr, files = parse(fh.read())
            if a.cmd == "check":
                print(describe(hdr, files))
            elif a.out:
                write_out(a.out, files[0])
            else:
                sys.stdout.buffer.write(files[0])
            return 0
    except Refused as err:
        print("uboot_script.py: refused: %s" % err, file=sys.stderr)
        return 2
    except OSError as err:
        print("uboot_script.py: %s" % err, file=sys.stderr)
        return 2
    ap.print_usage(sys.stderr)
    return 2


def self_test(mk):
    n = [0]

    def ok(cond, what):
        if not cond:
            raise AssertionError(what)
        n[0] += 1

    def refused(img, what):
        try:
            parse(img)
        except Refused:
            n[0] += 1
            return
        raise AssertionError("not refused: " + what)

    script = b"echo yakogl one-shot\nsetenv bootcmd 'run distro_bootcmd'\n"
    img = build(script, "yakogl test", "arm64", 1762387200)
    # A golden vector, worked out field by field from include/image.h; it
    # pins the layout against a change of this file.
    ok(img[:4] == bytes.fromhex("27051956"), "magic")
    ok(img[8:12] == (1762387200).to_bytes(4, "big"), "time")
    ok(img[12:16] == (len(script) + 8).to_bytes(4, "big"), "size = script + table")
    ok(img[16:24] == bytes(8), "load and ep 0")
    ok(img[28:32] == bytes([5, 22, 6, 0]), "os linux, arch arm64, type script, comp none")
    ok(img[32:64] == b"yakogl test".ljust(32, b"\0"), "name")
    ok(img[64:72] == struct.pack(">II", len(script), 0), "length table")
    ok(img[72:] == script, "script follows, unpadded")
    ok(int.from_bytes(img[24:28], "big") == zlib.crc32(img[64:]), "dcrc")
    hz = bytearray(img[:64]); hz[4:8] = bytes(4)
    ok(int.from_bytes(img[4:8], "big") == zlib.crc32(bytes(hz)), "hcrc")
    hdr, files = parse(img)
    ok(files == [script] and hdr["name"] == "yakogl test", "round trip")
    ok("arch arm64" in describe(hdr, files), "describe")
    ok(build(b"x", "", "arm", 0)[29] == 2, "arch arm")
    ok(build(b"x", "n" * 40)[32:64] == b"n" * 32, "name cut at 32, no NUL")
    ok(parse(build(b"x", "n" * 40))[0]["name"] == "n" * 32, "32-byte name read back")
    ok(parse(img + b"trailing")[1] == [script], "bytes after ih_size ignored like U-Boot")
    for bad, what in ((lambda: build(b"x", arch="riscv"), "arch"),
                      (lambda: build(b"x", time=-1), "time")):
        try:
            bad()
            raise AssertionError("build accepted bad " + what)
        except Refused:
            n[0] += 1

    # A two-file multi layout (padding between files) is read, as U-Boot does.
    two = struct.pack(">III", 3, 2, 0) + b"abc\0" + b"de"
    hdr2 = [IH_MAGIC, 0, 0, len(two), 0, 0, zlib.crc32(two), 5, 22, 6, 0, b""]
    hdr2[1] = zlib.crc32(HEADER.pack(*hdr2))
    ok(parse(HEADER.pack(*hdr2) + two)[1] == [b"abc", b"de"], "two files, padded")

    def mutate(i, x):
        m = bytearray(img); m[i] ^= x
        return bytes(m)

    def reheader(data, **kw):
        f = list(HEADER.unpack_from(img))
        f[3], f[6] = len(data), zlib.crc32(data)
        for k, v in kw.items():
            f[{"type": 9, "comp": 10}[k]] = v
        f[1] = 0
        f[1] = zlib.crc32(HEADER.pack(*f))
        return HEADER.pack(*f) + data

    refused(img[:40], "short header")
    refused(mutate(0, 0x01), "bad magic")
    refused(mutate(40, 0x80), "header bit")
    refused(mutate(len(img) - 1, 0x01), "data bit")
    refused(img[:-1], "truncated")
    refused(reheader(img[64:], type=2), "type kernel")
    refused(reheader(img[64:], comp=1), "gzip")
    refused(reheader(struct.pack(">II", len(script) + 1, 0) + script), "length one too long")
    refused(reheader(struct.pack(">II", len(script) - 1, 0) + script), "length one too short")
    refused(reheader(struct.pack(">I", 0) + script), "empty table")
    refused(reheader(struct.pack(">I", 5)), "table without end")

    with tempfile.TemporaryDirectory() as d:
        src = os.path.join(d, "boot.cmd")
        out = os.path.join(d, "boot.scr.uimg")
        with open(src, "wb") as fh:
            fh.write(script)
        os.utime(src, (1762387200, 1762387200))
        env = dict(os.environ)
        env.pop("SOURCE_DATE_EPOCH", None)
        me = [sys.executable, "-I", os.path.abspath(__file__)]

        def run(args, **kw):
            return subprocess.run(me + args, capture_output=True, env=kw.get("env", env))

        r = run(["build", src, "-o", out, "--name", "yakogl test"])
        ok(r.returncode == 0, "cli build: %r" % r.stderr)
        with open(out, "rb") as fh:
            ok(fh.read() == img, "cli build uses the mtime")
        env2 = dict(env, SOURCE_DATE_EPOCH="42")
        r = run(["build", src, "-o", out], env=env2)
        with open(out, "rb") as fh:
            ok(parse(fh.read())[0]["time"] == 42, "SOURCE_DATE_EPOCH")
        r = run(["build", src, "-o", out, "--time", "7"], env=env2)
        with open(out, "rb") as fh:
            ok(parse(fh.read())[0]["time"] == 7, "--time wins")
        r = run(["check", out])
        ok(r.returncode == 0 and b"script image" in r.stdout, "cli check")
        r = run(["extract", out])
        ok(r.returncode == 0 and r.stdout == script, "cli extract")
        with open(out, "wb") as fh:
            fh.write(mutate(70, 0x01))
        r = run(["check", out])
        ok(r.returncode == 2 and b"refused" in r.stderr, "cli refusal exits 2")
        r = run(["check", os.path.join(d, "missing")])
        ok(r.returncode == 2, "cli missing file exits 2")

        if mk:
            ref = os.path.join(d, "ref.uimg")
            subprocess.run([mk, "-A", "arm64", "-O", "linux", "-T", "script", "-C", "none",
                            "-n", "yakogl test", "-d", src, ref], check=True,
                           capture_output=True, env=dict(env, SOURCE_DATE_EPOCH="1762387200"))
            with open(ref, "rb") as fh:
                ok(fh.read() == img, "bytes equal mkimage's")
            print("uboot_script.py self-test: compared with %s" % mk)
        else:
            print("uboot_script.py self-test: mkimage not on PATH, byte comparison skipped")
    print("uboot_script.py self-test: %d checks ok" % n[0])
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AssertionError as err:
        print("uboot_script.py self-test: FAILED: %s" % err, file=sys.stderr)
        sys.exit(1)
