#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Read-only dump of the DK's display path as ST's Linux drives it.

Runs on the board (python3, root). It never writes a register, loads a
module, mounts anything or touches a service:

* registers: /dev/mem opened O_RDONLY, mmap PROT_READ, 32-bit loads (or the
  busybox `devmem ADDR 32` form, which has no write value);
* ADV7535: the regmap debugfs files that are already mounted;
* EDID, modes: /sys/class/drm; `modetest` in listing mode only (no mode set).

Output (see README.md): sections [regs], [adv7535], [edid], [modes],
[modetest].
"""
import argparse
import glob
import mmap
import os
import re
import subprocess
import sys

DEFAULT_ADV_MAPS = {0x3D: "main", 0x3C: "cec", 0x3F: "edid", 0x38: "packet"}
PAGE = 0x1000


# ---- read list ------------------------------------------------------------

def parse_readlist(text):
    """Return (blocks, names): blocks [(addr, words, name)], names {addr: name}."""
    blocks, names = [], {}
    for n, line in enumerate(text.splitlines(), 1):
        f = line.split()
        if not f or f[0] not in ("#@read", "#@name"):
            continue
        try:
            if f[0] == "#@read":
                blocks.append((int(f[1], 16), int(f[2], 0), f[3]))
            else:
                names[int(f[1], 16)] = f[2]
        except (IndexError, ValueError):
            raise ValueError("read list line %d: %r" % (n, line))
    return blocks, names


def word_name(addr, base, name, names):
    if addr in names:
        return names[addr]
    return "%s+0x%x" % (name, addr - base)


# ---- readers (each returns a list of int or None per word) ---------------

def _isolated(fn, *args):
    """Run fn in a child so a bus error kills the child, not the dump."""
    r, w = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.close(r)
        try:
            vals = fn(*args)
            os.write(w, (" ".join("%x" % v for v in vals)).encode())
            os._exit(0)
        except BaseException:
            os._exit(1)
    os.close(w)
    data = b""
    while True:
        chunk = os.read(r, 65536)
        if not chunk:
            break
        data += chunk
    os.close(r)
    _, status = os.waitpid(pid, 0)
    if status != 0:
        return None
    return [int(x, 16) for x in data.decode().split()]


def _mmap_words(addr, n):
    fd = os.open("/dev/mem", os.O_RDONLY | os.O_SYNC)
    try:
        base = addr & ~(PAGE - 1)
        off = addr - base
        length = (off + 4 * n + PAGE - 1) & ~(PAGE - 1)
        mm = mmap.mmap(fd, length, mmap.MAP_SHARED, mmap.PROT_READ,
                       offset=base)
        try:
            words = memoryview(mm).cast("I")
            return [words[off // 4 + i] for i in range(n)]
        finally:
            mm.close()
    finally:
        os.close(fd)


def _devmem_words(addr, n):
    out = []
    for i in range(n):
        res = subprocess.run(["devmem", "0x%x" % (addr + 4 * i), "32"],
                             stdout=subprocess.PIPE, check=True)
        out.append(int(res.stdout.split()[0], 0))
    return out


class Reader:
    def __init__(self, kind):
        self.kind = kind
        self.fixture = None
        if kind.startswith("file:"):
            self.fixture = {}
            with open(kind[5:]) as f:
                for line in f:
                    p = line.split()
                    if len(p) >= 2 and not p[0].startswith("#"):
                        self.fixture[int(p[0], 16)] = int(p[1], 16)

    def _block(self, addr, n):
        if self.fixture is not None:
            return [self.fixture[addr + 4 * i] for i in range(n)]
        fn = _devmem_words if self.kind == "devmem" else _mmap_words
        return _isolated(fn, addr, n)

    def words(self, addr, n):
        try:
            got = self._block(addr, n)
        except (KeyError, OSError, subprocess.SubprocessError):
            got = None
        if got is not None:
            return got
        out = []
        for i in range(n):
            try:
                one = self._block(addr + 4 * i, 1)
            except (KeyError, OSError, subprocess.SubprocessError):
                one = None
            out.append(one[0] if one else None)
        return out


def dump_regs(blocks, names, reader):
    lines = []
    for base, n, name in blocks:
        vals = reader.words(base, n)
        for i, v in enumerate(vals):
            a = base + 4 * i
            val = "ERR" if v is None else "0x%08x" % v
            lines.append("0x%08x %s %s" % (a, val, word_name(a, base, name, names)))
    return lines


# ---- ADV7535 regmap debugfs ----------------------------------------------

_REGMAP_DIR = re.compile(r"^(\d+)-00([0-9a-fA-F]{2})(?:-(.+))?$")
_REGMAP_LINE = re.compile(r"^\s*([0-9a-fA-F]+):\s*([0-9a-fA-F]+)\s*$")


def find_adv_regmaps(root, maps, main_addr):
    """Return [(mapname, i2c_addr, path)] on the bus that holds main_addr."""
    base = os.path.join(root, "sys/kernel/debug/regmap")
    found = {}
    for d in sorted(os.listdir(base)) if os.path.isdir(base) else []:
        m = _REGMAP_DIR.match(d)
        if not m:
            continue
        bus, addr = int(m.group(1)), int(m.group(2), 16)
        if addr in maps:
            found.setdefault(bus, []).append((maps[addr], addr, d))
    for bus, items in sorted(found.items()):
        if any(a == main_addr for _, a, _ in items):
            return [(n, a, os.path.join(base, d, "registers"))
                    for n, a, d in sorted(items, key=lambda t: t[1])]
    return []


def parse_regmap(text):
    return [(int(m.group(1), 16), int(m.group(2), 16))
            for m in map(_REGMAP_LINE.match, text.splitlines()) if m]


def dump_adv(root, maps, main_addr):
    lines = []
    maps_found = find_adv_regmaps(root, maps, main_addr)
    if not maps_found:
        return ["# no ADV7535 regmap in debugfs (not mounted, or the driver is not bound)"]
    for name, addr, path in maps_found:
        lines.append("# map %s i2c 0x%02x" % (name, addr))
        try:
            with open(path) as f:
                regs = parse_regmap(f.read())
        except OSError as e:
            lines.append("# map %s unreadable: %s" % (name, e.strerror))
            continue
        for off, val in regs:
            lines.append("%s 0x%02x 0x%02x" % (name, off, val))
    return lines


# ---- DRM: EDID and modes --------------------------------------------------

def drm_connectors(root):
    return sorted(glob.glob(os.path.join(root, "sys/class/drm/card*-*")))


def dump_edid(root):
    lines = []
    for c in drm_connectors(root):
        try:
            with open(os.path.join(c, "edid"), "rb") as f:
                data = f.read()
        except OSError:
            continue
        lines.append("%s %s" % (os.path.basename(c), data.hex() if data else "none"))
    return lines


def dump_modes(root):
    lines = []
    for c in drm_connectors(root):
        name = os.path.basename(c)
        for fname in ("status", "enabled"):
            try:
                with open(os.path.join(c, fname)) as f:
                    lines.append("%s %s %s" % (name, fname, f.read().strip()))
            except OSError:
                pass
        try:
            with open(os.path.join(c, "modes")) as f:
                for m in f.read().split():
                    lines.append("%s mode %s" % (name, m))
        except OSError:
            pass
    return lines


def dump_modetest(enabled):
    """Listing only: -c connectors, -e encoders, -p planes. No -s, no mode set."""
    if not enabled:
        return ["# modetest not run"]
    try:
        res = subprocess.run(["modetest", "-c", "-e", "-p"],
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             timeout=30)
    except (OSError, subprocess.SubprocessError):
        return ["# modetest not available"]
    return res.stdout.decode(errors="replace").splitlines()


# ---- output ---------------------------------------------------------------

def format_dump(regs, adv, edid, modes, modetest, kernel):
    out = ["# Linux display dump of the DK (read only). Format: see README.md.",
           "# kernel %s" % kernel]
    for title, body in (("regs", regs), ("adv7535", adv), ("edid", edid),
                        ("modes", modes), ("modetest", modetest)):
        out.append("[%s]" % title)
        out.extend(body)
    return "\n".join(out) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--readlist", required=True,
                    help="allow file with the #@read lines, or - for stdin")
    ap.add_argument("--reader", default="mmap",
                    help="mmap (default), devmem, or file:PATH (selftest)")
    ap.add_argument("--root", default="/", help="prefix for /sys (selftest)")
    ap.add_argument("--adv-main", default="0x3d", help="ADV7535 main map address")
    ap.add_argument("--no-modetest", action="store_true")
    a = ap.parse_args(argv)
    text = sys.stdin.read() if a.readlist == "-" else open(a.readlist).read()
    blocks, names = parse_readlist(text)
    if not blocks:
        sys.exit("no #@read lines in the read list")
    main_addr = int(a.adv_main, 0)
    maps = dict(DEFAULT_ADV_MAPS)
    kernel = os.uname().release
    sys.stdout.write(format_dump(
        dump_regs(blocks, names, Reader(a.reader)),
        dump_adv(a.root, maps, main_addr),
        dump_edid(a.root), dump_modes(a.root),
        dump_modetest(not a.no_modetest), kernel))


if __name__ == "__main__":
    main()
