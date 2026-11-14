#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Read, edit and write a U-Boot environment image byte for byte, no fw_setenv.

    tools/uboot_env.py [LAYOUT] print IMAGE [-n] [NAME ...]
    tools/uboot_env.py [LAYOUT] set IMAGE OUT NAME=VALUE ...
    tools/uboot_env.py [LAYOUT] unset IMAGE OUT NAME ...
    tools/uboot_env.py [LAYOUT] check IMAGE
    tools/uboot_env.py [LAYOUT] diff IMAGE_A IMAGE_B
  LAYOUT (before or after the subcommand):
    --size N        the size of one copy (CONFIG_ENV_SIZE); default the
                    image's length, or half of it with --redundant
    --redundant     two copies in IMAGE: copy 1 at --offset (default 0),
                    copy 2 at --offset2 (default --offset + --size)
    --offset N      where copy 1 starts in IMAGE (default 0)
    --offset2 N     where copy 2 starts (implies --redundant)
    --second FILE   copy 2 is in FILE at --offset2 (default 0; implies
                    --redundant); set and unset then write it to
                    --second-out, diff reads IMAGE_B's from --second-b
  N is decimal or 0x hex.

Why: the STM32MP257F-DK's ST Linux has fw_printenv but no fw_setenv and no
/etc/fw_env.config (#1281), so the board runner cannot arm a one-shot the way
tools/board.sh's uboot-mmc boards do. It reads the `u-boot-env` partition with
dd instead, edits it with this tool on the host and writes it back. The
format is U-Boot's (include/env.h, env/common.c, tools/env/fw_env.c):

    u32 CRC-32 (zlib's), little-endian, over the data area
    u8  flags                     (a redundant environment only)
    the data area: "name=value\\0" ... "\\0", then padding to the copy size

Copy 1 is U-Boot's primary (CONFIG_ENV_OFFSET), copy 2 the redundant one
(CONFIG_ENV_OFFSET_REDUND). With both CRCs valid the active copy is the one
env_import_redund picks: flags 0xff against 0x00 is the 0x00 one (the counter
wrapped), otherwise the higher flags, copy 1 on a tie. set and unset write
the other copy with flags = active + 1 (mod 256), as env_save does, and
leave the active one alone; a single environment is rewritten in place.

The edit is in place: existing variables keep their order and bytes, a new
one goes at the end, a changed one stays where it is. When the strings grow
they cover the padding; when they shrink, the bytes they no longer cover are
set to the padding byte (0xff when the byte after the strings' end is 0xff,
else 0x00) and the bytes after the old end are left as they were. No byte
outside the copy that is written changes. So a set and then an unset of a
variable the image did not have gives the input back byte for byte.

Refused, one line on stderr, exit 2 and no output file: a bad CRC (in either
copy of a pair), strings that do not end inside the data area or would not
fit it, a name that is empty or holds '=' or NUL, a value that holds NUL or
is empty (U-Boot reads "name=" as a deletion: use unset), a layout that does
not fit the file. print of a name that is not set exits 1, as fw_printenv
does; diff exits 1 when the images differ. check prints each copy's state
and exits 0 only when every copy's CRC is valid.
"""
import argparse
import os
import sys
import zlib


class Refused(Exception):
    pass


def number(arg):
    try:
        v = int(arg, 0)
    except ValueError:
        raise argparse.ArgumentTypeError("%r is not a number" % arg)
    if v < 0:
        raise argparse.ArgumentTypeError("%r is negative" % arg)
    return v


class Copy:
    """One copy of the environment: where it is, its header and its strings."""

    def __init__(self, label, path, offset, raw, redundant):
        self.label, self.path, self.offset, self.raw = label, path, offset, raw
        self.hdr = 5 if redundant else 4
        self.crc = int.from_bytes(raw[:4], "little")
        self.flags = raw[4] if redundant else None
        self.area = raw[self.hdr:]
        self.crc_ok = zlib.crc32(self.area) == self.crc
        self.entries, self.end = strings(self.area)

    def where(self, path=None):
        return "%s (bytes 0x%x..0x%x of %s)" % (
            self.label, self.offset, self.offset + len(self.raw) - 1, path or self.path)

    def problem(self):
        if not self.crc_ok:
            return "%s: bad CRC: stored 0x%08x, computed 0x%08x" % (
                self.where(), self.crc, zlib.crc32(self.area))
        if self.end is None:
            return "%s: CRC valid but the strings do not end inside the %d-byte data area" % (
                self.where(), len(self.area))
        return None


def strings(area):
    """The NUL-terminated strings up to the empty one; (list, end) where end
    is the offset after the empty string's NUL, or (list, None) when the
    area ends first."""
    out, p = [], 0
    while p < len(area):
        q = area.find(b"\0", p)
        if q < 0:
            return out, None
        if q == p:
            return out, p + 1
        out.append(area[p:q])
        p = q + 1
    return out, None


def name_of(entry):
    return entry.split(b"=", 1)[0]


def value_of(entry):
    parts = entry.split(b"=", 1)
    return parts[1] if len(parts) == 2 else b""


def lookup(entries, name):
    """The entry U-Boot uses for name (himport_r: the last one wins)."""
    hit = None
    for e in entries:
        if name_of(e) == name:
            hit = e
    return hit


def text(b):
    return repr(b.decode("utf-8", "backslashreplace"))


def check_name(name):
    if not name:
        raise Refused("empty variable name")
    if b"=" in name or b"\0" in name:
        raise Refused("variable name %s holds '=' or NUL" % text(name))


def apply_set(entries, assignments):
    out = list(entries)
    for a in assignments:
        if b"=" not in a:
            raise Refused("%s is not NAME=VALUE" % text(a))
        name, value = a.split(b"=", 1)
        check_name(name)
        if b"\0" in value:
            raise Refused("value of %s holds NUL" % text(name))
        if not value:
            raise Refused("empty value for %s: U-Boot reads NAME= as a deletion; use unset"
                          % text(name))
        new = name + b"=" + value
        hits = [i for i, e in enumerate(out) if name_of(e) == name]
        if hits:
            out[hits[-1]] = new
        else:
            out.append(new)
    return out


def apply_unset(entries, names):
    for n in names:
        check_name(n)
    drop = set(names)
    return [e for e in entries if name_of(e) not in drop]


def rewrite(copy, entries):
    """The data area of copy with its strings replaced by entries, in place."""
    data = b"".join(e + b"\0" for e in entries) + b"\0"
    area = copy.area
    if len(data) > len(area):
        raise Refused("the strings need %d bytes, the data area of %s has %d"
                      % (len(data), copy.label, len(area)))
    old = copy.end
    if len(data) >= old:
        return data + area[len(data):]
    pad = b"\xff" if area[old:old + 1] == b"\xff" else b"\0"
    return data + pad * (old - len(data)) + area[old:]


def header(area, flags):
    h = zlib.crc32(area).to_bytes(4, "little")
    return h if flags is None else h + bytes([flags])


def active_index(c1, c2):
    """env_import_redund's choice between two valid copies: 0 or 1."""
    f1, f2 = c1.flags, c2.flags
    if f1 == 0xff and f2 == 0:
        return 1
    if f2 == 0xff and f1 == 0:
        return 0
    return 1 if f2 > f1 else 0


class Image:
    """The file or files and the copies in them."""

    def __init__(self, path, a, second=None):
        self.redundant = bool(a.redundant or a.offset2 is not None or second)
        self.files = {path: read(path)}
        if second:
            if os.path.realpath(second) == os.path.realpath(path):
                raise Refused("--second names the image itself; use --redundant")
            self.files[second] = read(second)
        size = a.size
        if size is None:
            n = len(self.files[path]) - a.offset
            size = n // 2 if self.redundant and not second else n
        hdr = 5 if self.redundant else 4
        if size < hdr + 1:
            raise Refused("copy size %d: no room for the %d-byte header and a NUL" % (size, hdr))
        places = [(path, a.offset)]
        if self.redundant:
            off2 = a.offset2
            if off2 is None:
                off2 = 0 if second else a.offset + size
            places.append((second or path, off2))
        if len(places) == 2 and places[0][0] == places[1][0]:
            lo, hi = sorted(p[1] for p in places)
            if hi < lo + size:
                raise Refused("the copies at 0x%x and 0x%x overlap (size 0x%x)" % (lo, hi, size))
        self.size = size
        self.copies = []
        for i, (p, off) in enumerate(places):
            buf = self.files[p]
            if off + size > len(buf):
                raise Refused("copy %d at 0x%x size 0x%x runs past the end of %s (%d bytes)"
                              % (i + 1, off, size, p, len(buf)))
            self.copies.append(Copy("copy %d" % (i + 1), p, off, buf[off:off + size],
                                    self.redundant))

    def problems(self):
        return [m for m in (c.problem() for c in self.copies) if m]

    def active(self):
        bad = self.problems()
        if bad:
            raise Refused(bad[0])
        if not self.redundant:
            return 0
        return active_index(*self.copies)


def read(path):
    with open(path, "rb") as fh:
        return fh.read()


def write_out(path, data):
    tmp = path + ".tmp"
    with open(tmp, "wb") as fh:
        fh.write(data)
    os.replace(tmp, path)


def changed_range(old, new):
    diff = [i for i in range(len(old)) if old[i] != new[i]]
    return (diff[0], diff[-1]) if diff else None


def edit(img, entries_of, outs):
    """Rewrite the copy U-Boot would write; return the report and the files."""
    act = img.active()
    src = img.copies[act]
    entries = entries_of(src.entries)
    area = rewrite(src, entries)
    if img.redundant:
        dst = img.copies[1 - act]
        flags = (src.flags + 1) & 0xff
    else:
        dst, flags = src, None
    new = header(area, flags) + area
    files = {p: bytearray(b) for p, b in img.files.items()}
    files[dst.path][dst.offset:dst.offset + len(new)] = new
    out_path = outs[dst.path]
    r = changed_range(dst.raw, new)
    if img.redundant:
        msg = ("%s: wrote %s with flags 0x%02x (%s, flags 0x%02x, was active and is "
               "left alone; %s is active now)" % (out_path, dst.where(out_path), flags,
                                                  src.label, src.flags, dst.label))
    else:
        msg = "%s: wrote %s" % (out_path, dst.where(out_path))
    if r:
        msg += "; changed bytes 0x%x..0x%x" % (dst.offset + r[0], dst.offset + r[1])
    else:
        msg += "; no byte changed"
    used = len(b"".join(e + b"\0" for e in entries)) + 1
    msg += "; %d variables, %d of %d data bytes used" % (len(entries), used, len(area))
    return msg, {outs[p]: bytes(b) for p, b in files.items()}


def show(entries, names, value_only):
    if not names:
        for e in entries:
            sys.stdout.buffer.write(e + b"\n")
        return 0
    rc = 0
    for n in names:
        e = lookup(entries, n)
        if e is None:
            print("## Error: \"%s\" not defined" % n.decode("utf-8", "backslashreplace"), file=sys.stderr)
            rc = 1
            continue
        sys.stdout.buffer.write((value_of(e) if value_only else e) + b"\n")
    return rc


def diff(ea, eb):
    lines = []
    a = {name_of(e): e for e in ea}
    b = {name_of(e): e for e in eb}
    for n, e in a.items():
        if n not in b:
            lines.append(b"removed " + e)
        elif b[n] != e:
            lines.append(b"changed " + n + b": " + value_of(e) + b" -> " + value_of(b[n]))
    for n, e in b.items():
        if n not in a:
            lines.append(b"added   " + e)
    return lines


def layout_args(p, suppress):
    d = argparse.SUPPRESS if suppress else None
    p.add_argument("--size", type=number, default=d)
    p.add_argument("--offset", type=number, default=d if suppress else 0)
    p.add_argument("--offset2", type=number, default=d)
    p.add_argument("--redundant", action="store_true", default=d if suppress else False)
    p.add_argument("--second", default=d)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter,
                                 epilog=__doc__.split("\n", 2)[2])
    layout_args(ap, False)
    sub = ap.add_subparsers(dest="cmd")
    s = {}
    for name, hlp in (("print", "print the variables, as fw_printenv"),
                      ("set", "write OUT with NAME=VALUE set"),
                      ("unset", "write OUT with NAME removed"),
                      ("check", "check the CRC of every copy"),
                      ("diff", "variables added, removed or changed from A to B")):
        s[name] = sub.add_parser(name, help=hlp)
        layout_args(s[name], True)
    s["print"].add_argument("-n", action="store_true", help="print the value only")
    s["print"].add_argument("image")
    s["print"].add_argument("names", nargs="*")
    for name in ("set", "unset"):
        s[name].add_argument("image")
        s[name].add_argument("out")
        s[name].add_argument("args", nargs="+")
        s[name].add_argument("--second-out")
    s["check"].add_argument("image")
    s["diff"].add_argument("image")
    s["diff"].add_argument("image_b")
    s["diff"].add_argument("--second-b")
    # argparse (3.12.3) takes the NAME list of `print IMAGE [-n] [NAME ...]`
    # at the first run of positionals only, so names after -n come back as
    # leftovers; they are names when none looks like an option.
    a, rest = ap.parse_known_args(argv)
    if rest:
        if a.cmd != "print" or any(x.startswith("-") for x in rest):
            ap.error("unrecognized arguments: " + " ".join(rest))
        a.names += rest
    if a.cmd is None:
        ap.print_usage(sys.stderr)
        return 2
    if a.second and a.cmd == "diff" and not a.second_b:
        ap.error("diff with --second needs --second-b for IMAGE_B")
    if a.cmd in ("set", "unset") and bool(a.second) != bool(a.second_out):
        ap.error("--second and --second-out go together")
    raw = lambda xs: [os.fsencode(x) for x in xs]
    try:
        if a.cmd == "print":
            if a.n and len(a.names) != 1:
                raise Refused("-n takes exactly one name")
            img = Image(a.image, a, a.second)
            return show(img.copies[img.active()].entries, raw(a.names), a.n)
        if a.cmd in ("set", "unset"):
            img = Image(a.image, a, a.second)
            outs = {a.image: a.out}
            if a.second:
                outs[a.second] = a.second_out
            if len(set(outs.values())) != len(outs):
                raise Refused("the two outputs are the same file")
            args = raw(a.args)
            fn = (lambda e: apply_set(e, args)) if a.cmd == "set" else (lambda e: apply_unset(e, args))
            msg, files = edit(img, fn, outs)
            for p, data in files.items():
                write_out(p, data)
            print(msg)
            return 0
        if a.cmd == "check":
            img = Image(a.image, a, a.second)
            bad = img.problems()
            for c in img.copies:
                m = c.problem()
                if m:
                    print("BAD %s" % m)
                else:
                    print("ok  %s: CRC 0x%08x%s, %d variables, %d of %d data bytes used"
                          % (c.where(), c.crc,
                             "" if c.flags is None else ", flags 0x%02x" % c.flags,
                             len(c.entries), c.end, len(c.area)))
            if bad:
                print("uboot_env.py: refused: %d bad copy(ies): %s"
                      % (len(bad), ", ".join(m.split(" (")[0] for m in bad)), file=sys.stderr)
                return 2
            if img.redundant:
                print("active: %s" % img.copies[img.active()].label)
            return 0
        if a.cmd == "diff":
            ia = Image(a.image, a, a.second)
            ib = Image(a.image_b, a, a.second_b)
            lines = diff(ia.copies[ia.active()].entries, ib.copies[ib.active()].entries)
            for line in lines:
                sys.stdout.buffer.write(line + b"\n")
            return 1 if lines else 0
    except Refused as err:
        print("uboot_env.py: refused: %s" % err, file=sys.stderr)
        return 2
    except OSError as err:
        print("uboot_env.py: %s" % err, file=sys.stderr)
        return 2
    return 2


if __name__ == "__main__":
    sys.exit(main())
