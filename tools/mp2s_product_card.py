#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Host-only builder of the STM32MP257F-DK product card's files.

Design: docs/notes/stm32mp257f-dk-standalone.md, section 2 ("The product, on a
card of its own"), with the variables of sections 3 and 7.

Writes into one OUTPUT DIRECTORY, nothing else:
  u-boot-env-copy1.bin, u-boot-env-copy2.bin   redundant U-Boot env images
  yakogl/standalone/boot.scr.uimg, default.bin copied from the given paths
  manifest.txt                                 sha256 of each output

The env is the input env with only these changed:
  mp2s_ver=1  mp2s_left=forever  mp2s_prefixes=<live boot_prefixes>
  boot_prefixes=/yakogl/standalone/ <live boot_prefixes>
Everything else keeps its bytes and its order (bootcmd, altbootcmd, bootcount
...). The tool never writes a device, a card image or a partition table, and
never removes anything (with --no-linux the manifest only says that the person
removes mmc0_extlinux/ from their own card).

Env image format: U-Boot's redundant environment, as tools/uboot_env.py
documents it (crc32 little-endian over the data area, one flags byte, then
NAME=VALUE entries each ending in NUL, a final NUL, padding with 0x00). Copy 1
is written with flags 1 and copy 2 with flags 0, so both CRCs are good and
copy 1 is the active one. The selftest reads the pair back with
tools/uboot_env.py.
"""
import argparse
import hashlib
import os
import stat
import struct
import sys
import zlib

HDR = 5
ACTIVE, OBSOLETE = 1, 0
FORBIDDEN = {"bootcmd", "altbootcmd", "bootcount", "bootlimit",
             "upgrade_available", "preboot"}
PREFIX = "/yakogl/standalone/"
BAD_ROOTS = ("/dev", "/sys", "/proc")


class Refuse(Exception):
    pass


def check_path(p, what, must_exist):
    for q in (os.path.abspath(p), os.path.realpath(p)):
        for r in BAD_ROOTS:
            if q == r or q.startswith(r + "/"):
                raise Refuse("%s %s is under %s" % (what, p, r))
    try:
        st = os.stat(p)
    except FileNotFoundError:
        if must_exist:
            raise Refuse("%s %s does not exist" % (what, p))
        return
    if stat.S_ISBLK(st.st_mode) or stat.S_ISCHR(st.st_mode):
        raise Refuse("%s %s is a block or char device" % (what, p))


def crc(data):
    return zlib.crc32(data) & 0xffffffff


def parse_data(data):
    """Entries from the data area, as a list of (name, value) of bytes."""
    end = data.find(b"\0\0")
    if end < 0:
        raise Refuse("env data has no double NUL")
    out = []
    for e in data[:end].split(b"\0"):
        n, sep, v = e.partition(b"=")
        if not sep or not n:
            raise Refuse("malformed env entry %r" % e[:40])
        out.append((n, v))
    return out


def read_env_image(raw):
    if len(raw) <= HDR:
        raise Refuse("env image too short")
    want = struct.unpack("<I", raw[:4])[0]
    if crc(raw[HDR:]) != want:
        raise Refuse("env image CRC mismatch")
    return parse_data(raw[HDR:])


def read_env_text(text):
    out = []
    for line in text.split(b"\n"):
        if not line.strip() or line.lstrip().startswith(b"#"):
            continue
        n, sep, v = line.partition(b"=")
        if not sep or not n:
            raise Refuse("env text line without name=value: %r" % line[:40])
        out.append((n, v))
    return out


def build_image(entries, size, flags):
    data = b"".join(n + b"=" + v + b"\0" for n, v in entries) + b"\0"
    if HDR + len(data) > size:
        raise Refuse("environment needs %d bytes, --env-size is %d"
                     % (HDR + len(data), size))
    data += b"\0" * (size - HDR - len(data))
    return struct.pack("<I", crc(data)) + bytes([flags]) + data


def edit(entries, extra):
    names = [n for n, _ in entries]
    if len(set(names)) != len(names):
        raise Refuse("duplicate names in the input env")
    d = dict(entries)
    if any(n.startswith(b"mp2s_") for n in names):
        raise Refuse("input env already has mp2s_* (a default is set)")
    live = d.get(b"boot_prefixes")
    if live is None:
        raise Refuse("input env has no boot_prefixes")
    if live.startswith(PREFIX.encode()):
        raise Refuse("boot_prefixes already carries %s" % PREFIX)
    new = [(b"mp2s_ver", b"1"), (b"mp2s_left", b"forever"),
           (b"mp2s_prefixes", live),
           (b"boot_prefixes", PREFIX.encode() + b" " + live)]
    for e in extra:
        n, sep, v = e.partition("=")
        if n in FORBIDDEN:
            raise Refuse("refusing to set %s" % n)
        if not sep or not n.startswith("mp2s_"):
            raise Refuse("--set takes mp2s_NAME=VALUE, got %r" % e)
        new.append((n.encode(), v.encode()))
    repl = dict(new)
    out = [(n, repl.pop(n, v)) for n, v in entries]
    out.extend(repl.items())
    return out


def sha(b):
    return hashlib.sha256(b).hexdigest()


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--env-text", help="input env, one name=value per line")
    g.add_argument("--env-image", help="input env image (redundant format)")
    ap.add_argument("--env-size", type=lambda s: int(s, 0), default=0x20000,
                    help="env size in bytes, header included (default 0x20000)")
    ap.add_argument("--script", required=True, help="boot.scr.uimg to copy")
    ap.add_argument("--image", required=True, help="default.bin to copy")
    ap.add_argument("--out", required=True, help="output directory (new or empty)")
    ap.add_argument("--no-linux", action="store_true",
                    help="manifest notes that the person removes mmc0_extlinux/")
    ap.add_argument("--set", action="append", default=[], metavar="mp2s_X=V",
                    help="extra mp2s_ variable")
    a = ap.parse_args(argv)
    try:
        inp = a.env_text or a.env_image
        for p, w, ex in ((inp, "input env", True), (a.script, "script", True),
                         (a.image, "image", True), (a.out, "output", False)):
            check_path(p, w, ex)
        if os.path.exists(a.out):
            if not os.path.isdir(a.out):
                raise Refuse("output %s is not a directory" % a.out)
            if os.listdir(a.out):
                raise Refuse("output directory %s is not empty" % a.out)
        for p in (inp, a.script, a.image):
            if not os.path.isfile(p):
                raise Refuse("%s is not a regular file" % p)
        with open(inp, "rb") as f:
            raw = f.read()
        entries = read_env_image(raw) if a.env_image else read_env_text(raw)
        edited = edit(entries, a.set)
        c1 = build_image(edited, a.env_size, ACTIVE)
        c2 = build_image(edited, a.env_size, OBSOLETE)
        with open(a.script, "rb") as f:
            script = f.read()
        with open(a.image, "rb") as f:
            image = f.read()
    except Refuse as e:
        print("mp2s_product_card: refused: %s" % e, file=sys.stderr)
        return 2
    except OSError as e:
        print("mp2s_product_card: %s" % e, file=sys.stderr)
        return 1

    files = [("u-boot-env-copy1.bin", c1), ("u-boot-env-copy2.bin", c2),
             ("yakogl/standalone/boot.scr.uimg", script),
             ("yakogl/standalone/default.bin", image)]
    os.makedirs(os.path.join(a.out, "yakogl", "standalone"), exist_ok=True)
    for rel, b in files:
        with open(os.path.join(a.out, rel), "wb") as f:
            f.write(b)
    lines = ["%s  %s" % (sha(b), rel) for rel, b in files]
    if a.no_linux:
        lines.append("NOTE: --no-linux: the person removes mmc0_extlinux/ from "
                     "their own card; this tool never removes it")
    with open(os.path.join(a.out, "manifest.txt"), "w") as f:
        f.write("\n".join(lines) + "\n")
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
