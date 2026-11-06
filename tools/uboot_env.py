#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Read, edit and write a U-Boot environment image byte for byte.

    tools/uboot_env.py print [-n] FILE [NAME] [opts]
    tools/uboot_env.py set   IN OUT NAME=VALUE... [opts]
    tools/uboot_env.py unset IN OUT NAME... [opts]
    tools/uboot_env.py check FILE [opts]
    tools/uboot_env.py diff  A B [opts]

opts: --size N        env size (a partition or one copy); default: the file's
                      length (half of it with --redundant)
      --redundant     two copies (CRC, flag byte, data) at 0 and --size
      --second FILE   with --redundant: the second copy is in FILE, not in IN

Format: little-endian CRC32 (zlib) of the data area, then for a redundant env
one flag byte, then the data area: NAME=VALUE strings each ended by NUL, an
extra NUL, padding (0x00 or 0xff, kept as the input had it) up to the size.
A redundant pair's active copy is the valid one with the newer flag (255 then
0 counts as newer, as U-Boot has it); set/unset write the other copy with
flag = active + 1 and say which copy and which bytes changed. With --second
OUT receives the changed copy's file alone.

check exits 2 when any copy has a bad CRC (and says which); print and diff
read the active copy. Exit 0 done / valid / equal, 1 diff found a difference
or print did not find NAME, 2 refused or bad image. Nothing outside the env
area is touched (bytes past the copies are kept). Refused with no output
file: bad CRC, data not fitting, a name that is empty or contains '=' or NUL,
a value containing NUL, mixed padding bytes on set/unset.
"""
import argparse
import os
import sys
import zlib


class Refuse(Exception):
    pass


def build(size, hdr, red, pad, vs, flag, label):
    """The bytes of one copy of `size` bytes holding vs [(name, value)]."""
    body = b"".join(n + b"=" + v + b"\0" for n, v in vs) + b"\0"
    room = size - hdr
    if len(body) > room:
        raise Refuse(f"{label}: {len(body)} bytes of data do not fit the "
                     f"{room}-byte data area")
    data = body + bytes([pad]) * (room - len(body))
    out = zlib.crc32(data).to_bytes(4, "little")
    if red:
        out += bytes([flag])
    return out + data


class Copy:
    """One env copy at offset off of a file's bytes."""

    def __init__(self, buf, off, size, redundant, label):
        self.off, self.size, self.red, self.label = off, size, redundant, label
        self.hdr = 5 if redundant else 4
        if size <= self.hdr or off + size > len(buf):
            raise Refuse(f"{label}: size {size:#x} at {off:#x} does not fit "
                         f"the input ({len(buf):#x} bytes)")
        self.crc = int.from_bytes(buf[off:off + 4], "little")
        self.flag = buf[off + 4] if redundant else None
        d = bytes(buf[off + self.hdr:off + size])
        self.crc_ok = zlib.crc32(d) & 0xffffffff == self.crc
        self.vars = []
        self.pad = 0
        self.parse_err = None
        pos = 0
        while True:
            nul = d.find(b"\0", pos)
            if pos >= len(d) or nul < 0:
                self.parse_err = "no terminating NUL in the data area"
                break
            if nul == pos:  # an empty string: the terminator
                rest = d[pos + 1:]
                if rest == b"\xff" * len(rest) and rest:
                    self.pad = 0xff
                elif rest != b"\0" * len(rest):
                    self.parse_err = "padding after the terminator is mixed"
                break
            n, eq, v = d[pos:nul].partition(b"=")
            if not eq:
                self.parse_err = f"entry without '=' at data offset {pos:#x}"
                break
            self.vars.append((n, v))
            pos = nul + 1


class Env:
    def __init__(self, a, size, redundant, second=None):
        self.red = redundant
        self.a = a
        self.b = second
        size = size or len(a) // (2 if redundant and second is None else 1)
        self.size = size
        if not redundant:
            self.copies = [Copy(a, 0, size, False, "env")]
        elif second is not None:
            self.copies = [Copy(a, 0, size, True, "copy 1"),
                           Copy(second, 0, size, True, "copy 2")]
        else:
            self.copies = [Copy(a, 0, size, True, "copy 1"),
                           Copy(a, size, size, True, "copy 2")]
        self.active = self._pick()

    def _pick(self):
        if not self.red:
            return 0 if self.copies[0].crc_ok else None
        c1, c2 = self.copies
        if c1.crc_ok and c2.crc_ok:
            if c1.flag == 255 and c2.flag == 0:
                return 1
            if c2.flag == 255 and c1.flag == 0:
                return 0
            return 0 if c1.flag > c2.flag else 1
        if c1.crc_ok:
            return 0
        return 1 if c2.crc_ok else None

    def bad(self):
        return [c.label for c in self.copies if not c.crc_ok]

    def active_copy(self):
        if self.active is None:
            raise Refuse("bad CRC: " + ", ".join(self.bad()))
        return self.copies[self.active]

    def edit(self, fn):
        """Apply fn to the active copy's variables; return (image, note)."""
        if self.bad():
            raise Refuse("bad CRC: " + ", ".join(self.bad()))
        act = self.active_copy()
        if act.parse_err:
            raise Refuse(f"{act.label}: {act.parse_err}")
        vs = fn(list(act.vars))
        if not self.red:
            out = bytearray(self.a)
            out[0:act.size] = build(act.size, act.hdr, False, act.pad, vs, None,
                                    act.label)
            return bytes(out), f"{act.label}: bytes 0x0-{act.size:#x} rewritten"
        ti = 1 - self.active
        tgt = self.copies[ti]
        flag = (act.flag + 1) & 0xff
        new = build(tgt.size, tgt.hdr, True, act.pad, vs, flag, tgt.label)
        out = bytearray(self.b if self.b is not None and ti == 1 else self.a)
        out[tgt.off:tgt.off + tgt.size] = new
        return bytes(out), (
            f"{tgt.label} (inactive; {act.label} is active, flag {act.flag}) "
            f"rewritten with flag {flag}: bytes {tgt.off:#x}-"
            f"{tgt.off + tgt.size:#x}"
            + (" of its file" if self.b is not None else ""))


def valid_name(n):
    return bool(n) and "=" not in n and "\0" not in n


def load(args, path):
    try:
        raw = open(path, "rb").read()
        sec = open(args.second, "rb").read() if args.second else None
    except OSError as e:
        raise Refuse(str(e))
    if args.second and not args.redundant:
        raise Refuse("--second needs --redundant")
    return Env(raw, args.size, args.redundant, sec)


def text(b):
    return b.decode("latin-1")


def cmd_print(args):
    c = load(args, args.file).active_copy()
    if c.parse_err and not c.parse_err.startswith("padding"):
        raise Refuse(f"{c.label}: {c.parse_err}")
    for n, v in c.vars:
        if args.name is None:
            print(f"{text(n)}={text(v)}")
        elif text(n) == args.name:
            print(text(v) if args.n else f"{text(n)}={text(v)}")
            return 0
    if args.name is not None:
        print(f'## Error: "{args.name}" not defined', file=sys.stderr)
        return 1
    return 0


def finish(args, fn):
    img, note = load(args, args.inp).edit(fn)
    tmp = args.out + ".tmp"
    with open(tmp, "wb") as f:
        f.write(img)
    os.replace(tmp, args.out)
    print(note)
    return 0


def cmd_set(args):
    pairs = []
    for a in args.assign:
        n, eq, v = a.partition("=")
        if not eq or not valid_name(n):
            raise Refuse(f"bad assignment {a!r}: need NAME=VALUE, no '=' or "
                         "NUL in NAME")
        if "\0" in v:
            raise Refuse(f"value of {n} contains NUL")
        pairs.append((n.encode("latin-1"), v.encode("latin-1")))

    def fn(vs):
        for n, v in pairs:
            for i, (m, _) in enumerate(vs):
                if m == n:
                    vs[i] = (n, v)
                    break
            else:
                vs.append((n, v))
        return vs
    return finish(args, fn)


def cmd_unset(args):
    for n in args.names:
        if not valid_name(n):
            raise Refuse(f"bad name {n!r}")
    names = [n.encode("latin-1") for n in args.names]
    return finish(args, lambda vs: [x for x in vs if x[0] not in names])


def cmd_check(args):
    e = load(args, args.file)
    for i, c in enumerate(e.copies):
        st = "ok" if c.crc_ok else f"BAD CRC (stored {c.crc:#010x})"
        fl = f", flag {c.flag}" if e.red else ""
        act = ", active" if e.red and i == e.active else ""
        print(f"{c.label}: {st}{fl}{act}")
    return 2 if e.bad() else 0


def cmd_diff(args):
    a = load(args, args.a).active_copy().vars
    b = load(args, args.b).active_copy().vars
    da, db = dict(a), dict(b)
    rc = 0
    for n, v in a:
        if n not in db:
            print(f"-{text(n)}={text(v)}")
        elif db[n] != v:
            print(f"~{text(n)}: {text(v)} -> {text(db[n])}")
        else:
            continue
        rc = 1
    for n, v in b:
        if n not in da:
            print(f"+{text(n)}={text(v)}")
            rc = 1
    return rc


def main(argv):
    p = argparse.ArgumentParser(prog="uboot_env.py",
                                description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)

    def add(name, *pos):
        sp = sub.add_parser(name)
        sp.add_argument("--size", type=lambda s: int(s, 0))
        sp.add_argument("--redundant", action="store_true")
        sp.add_argument("--second")
        for a in pos:
            if a.endswith("*"):
                sp.add_argument(a[:-1], nargs="+")
            elif a.endswith("?"):
                sp.add_argument(a[:-1], nargs="?")
            else:
                sp.add_argument(a)
        return sp
    add("print", "file", "name?").add_argument("-n", action="store_true")
    add("set", "inp", "out", "assign*")
    add("unset", "inp", "out", "names*")
    add("check", "file")
    add("diff", "a", "b")
    args = p.parse_args(argv)
    fn = {"print": cmd_print, "set": cmd_set, "unset": cmd_unset,
          "check": cmd_check, "diff": cmd_diff}[args.cmd]
    try:
        return fn(args)
    except Refuse as e:
        print(f"uboot_env.py: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
