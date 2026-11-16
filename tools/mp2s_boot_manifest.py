#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The /boot sha256 manifest of the STM32MP257F-DK standalone: record, compare.

    tools/mp2s_boot_manifest.py record DIR [-o OUT]
    tools/mp2s_boot_manifest.py record --from FILE [-o OUT]
    tools/mp2s_boot_manifest.py compare DIR FILE [--allow PREFIX]...
    tools/mp2s_boot_manifest.py compare-records A B [--allow PREFIX]...

FILE and A or B may be '-' (stdin); both sides of one compare may not.
OUT defaults to '-'.  record takes a directory or a listing, never both.

The manifest is sorted lines: per regular file '<sha256>  <path>' plus
'@mode<TAB><octal><TAB><path>', and per symlink
'@link<TAB><target><TAB><path>'.  Paths are relative to the recorded
directory, '/'-separated, no './' and no '..'.  Only regular files and
symlinks are recorded; DIR is read only.  A
`find . -type f -print0 | sort -z | xargs -0 sha256sum` listing, run inside
DIR, parses as a manifest: its './' is stripped and its absolute paths are
kept as they are.  Such a listing tracks neither modes nor symlinks, so a
mode is compared only when both sides recorded one, and a symlink entry on
one side is not a difference when the other side recorded no symlink at all.

record --from turns such a listing (for example the ssh capture of /boot)
into the canonical manifest.  Record before writing the manifest into DIR:
boot-sha256.txt itself then shows up as an allowed 'added' at every later
compare, never as a self-hash.

A difference is added, removed or changed (content, mode, link target, or
file <-> symlink).  Differences under the allowed prefixes (default
'yakogl/standalone/', a file's path relative to the recorded directory;
--allow replaces the default) are printed and not judged.  The diff lines
are followed by exactly one verdict line, and the exit is 0 for the first
two and 1 for the third:

  mp2s-boot: same: <n> files
  mp2s-boot: allowed: <k> difference(s) under the allowed prefixes
  mp2s-boot: NOT same: <k> difference(s) outside the allowed prefixes

Exit 2 (one line on stderr) for a usage error, a directory that is not one,
a line that is not a manifest line, and '-' on both sides.  Python 3
stdlib only.
"""
import argparse
import hashlib
import os
import stat
import sys

DEFAULT_ALLOW = ["yakogl/standalone/"]


def die(msg):
    print(msg, file=sys.stderr)
    sys.exit(2)


def scan(root):
    """path -> (kind, value, mode); kind 'file' (sha256) or 'link' (target)."""
    ents = {}
    for dp, dns, fns in os.walk(root, followlinks=False):
        dns.sort()
        names = fns + [d for d in dns if os.path.islink(os.path.join(dp, d))]
        for n in sorted(names):
            p = os.path.join(dp, n)
            rel = os.path.relpath(p, root).replace(os.sep, "/")
            st = os.lstat(p)
            if stat.S_ISLNK(st.st_mode):
                ents[rel] = ("link", os.readlink(p), None)
            elif stat.S_ISREG(st.st_mode):
                h = hashlib.sha256()
                with open(p, "rb") as f:
                    for b in iter(lambda: f.read(1 << 20), b""):
                        h.update(b)
                ents[rel] = ("file", h.hexdigest(), stat.S_IMODE(st.st_mode))
    return ents


def render(ents):
    lines = []
    for p, (k, v, m) in ents.items():
        if k == "file":
            lines.append((p, 0, "%s  %s" % (v, p)))
            if m is not None:
                lines.append((p, 1, "@mode\t%04o\t%s" % (m, p)))
        else:
            lines.append((p, 2, "@link\t%s\t%s" % (v, p)))
    return "".join(l + "\n" for _, _, l in sorted(lines))


def parse(text, name):
    sha, mode, link = {}, {}, {}
    for i, l in enumerate(text.split("\n"), 1):
        if not l:
            continue
        if l.startswith("@mode\t"):
            _, m, p = l.split("\t", 2)
            try:
                mode[p] = int(m, 8)
            except ValueError:
                die("%s:%d: bad mode: %s" % (name, i, m))
        elif l.startswith("@link\t"):
            _, t, p = l.split("\t", 2)
            link[p] = t
        elif len(l) > 66 and l[64:66] == "  " and all(c in "0123456789abcdef" for c in l[:64]):
            p = l[66:]
            if p.startswith("./"):
                p = p[2:]
            sha[p] = l[:64]
        else:
            die("%s:%d: not a manifest line" % (name, i))
    ents = {}
    for p, h in sha.items():
        ents[p] = ("file", h, mode.get(p))
    for p, t in link.items():
        if p in ents:
            die("%s: %s is both file and link" % (name, p))
        ents[p] = ("link", t, None)
    return ents


def read_text(path):
    if path == "-":
        return sys.stdin.read(), "stdin"
    try:
        with open(path, encoding="utf-8", errors="surrogateescape") as f:
            return f.read(), path
    except OSError as e:
        die("cannot read %s: %s" % (path, e))


def read_manifest(path):
    text, name = read_text(path)
    return parse(text, name)


def tracks_links(ents):
    """True for a side that recorded modes or symlinks (a tree scan)."""
    return any(e[0] == "link" or e[2] is not None for e in ents.values())


def diff(old, new, allow):
    """Print differences (old = recorded, new = now) and the verdict; exit count."""
    # A sha256sum listing side tracks neither modes nor symlinks: a symlink
    # entry present only on the other side is then not a difference.  A path
    # present on both sides is always compared.
    old_links, new_links = tracks_links(old), tracks_links(new)
    n_bad = 0
    n_allowed = 0
    for p in sorted(set(old) | set(new)):
        a, b = old.get(p), new.get(p)
        if a == b:
            continue
        if a is None or b is None:
            # The entry is on one side only: skip it when the side that has
            # no entry for this path tracked no symlinks (a sha256sum
            # capture), since it may hold the link and simply not record it.
            if (a or b)[0] == "link" and not (new_links if b is None else old_links):
                continue
            msg = "added: %s" % p if a is None else "removed: %s" % p
        else:
            why = []
            if a[0] != b[0]:
                why.append("%s -> %s" % (a[0], b[0]))
            elif a[1] != b[1]:
                why.append("content" if a[0] == "file" else
                           "link target %s -> %s" % (a[1], b[1]))
            if a[0] == b[0] == "file" and a[2] is not None and b[2] is not None and a[2] != b[2]:
                why.append("mode %04o -> %04o" % (a[2], b[2]))
            if not why:
                continue
            msg = "changed: %s (%s)" % (p, ", ".join(why))
        if any(p.startswith(x) for x in allow):
            n_allowed += 1
            msg += " (allowed)"
        else:
            n_bad += 1
        print(msg)
    if n_bad:
        print("mp2s-boot: NOT same: %d difference(s) outside the allowed prefixes" % n_bad)
    elif n_allowed:
        print("mp2s-boot: allowed: %d difference(s) under the allowed prefixes" % n_allowed)
    else:
        print("mp2s-boot: same: %d files" % len(new))
    return n_bad


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("record")
    r.add_argument("dir", nargs="?")
    r.add_argument("--from", dest="src", metavar="FILE",
                   help="a sha256sum listing instead of a directory ('-' = stdin)")
    r.add_argument("-o", "--output", default="-")
    for n, names in (("compare", ("dir", "file")), ("compare-records", ("a", "b"))):
        c = sub.add_parser(n)
        for name in names:
            c.add_argument(name)
        c.add_argument("--allow", action="append", metavar="PREFIX")
    a = ap.parse_args(argv)
    if a.cmd == "record":
        if (a.dir is None) == (a.src is None):
            die("record: exactly one of DIR or --from FILE")
        if a.src is not None:
            text, name = read_text(a.src)
            ents = parse(text, name)
        else:
            if not os.path.isdir(a.dir):
                die("not a directory: %s" % a.dir)
            ents = scan(a.dir)
        text = render(ents)
        if a.output == "-":
            sys.stdout.write(text)
        else:
            with open(a.output, "w", encoding="utf-8", errors="surrogateescape") as f:
                f.write(text)
        return 0
    allow = a.allow if a.allow else DEFAULT_ALLOW
    if a.cmd == "compare":
        if not os.path.isdir(a.dir):
            die("not a directory: %s" % a.dir)
        old, new = read_manifest(a.file), scan(a.dir)
    else:
        if a.a == "-" and a.b == "-":
            die("only one side can be '-'")
        old, new = read_manifest(a.a), read_manifest(a.b)
    return 1 if diff(old, new, allow) else 0


if __name__ == "__main__":
    sys.exit(main())
