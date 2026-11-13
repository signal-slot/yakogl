#!/usr/bin/env python3
"""Record and compare a /boot tree byte for byte (STM32MP257F-DK standalone).

  record DIR -o FILE               write the manifest ('-' = stdout)
  compare DIR FILE [--allow P]     DIR now against a manifest ('-' = stdin)
  compare-records A B [--allow P]  two manifests ('-' = stdin, one side only)

Manifest: sorted lines, one per regular file '<sha256>  <path>', plus, per
file, '@mode<TAB><octal><TAB><path>' and, per symlink,
'@link<TAB><target><TAB><path>'.  Paths are relative to DIR, '/'-separated.
A difference is added, removed or changed (content, mode, link target, or
file <-> symlink).  Exit 1 on a difference outside the allowed prefixes
(default 'yakogl/standalone/'); differences under them print '(allowed)'.
DIR is only read.  Python 3 stdlib only.
"""
import argparse
import hashlib
import os
import stat
import sys

DEFAULT_ALLOW = ["yakogl/standalone/"]


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
            mode[p] = int(m, 8)
        elif l.startswith("@link\t"):
            _, t, p = l.split("\t", 2)
            link[p] = t
        elif len(l) > 66 and l[64:66] == "  ":
            sha[l[66:]] = l[:64]
        else:
            sys.exit("%s:%d: not a manifest line" % (name, i))
    ents = {}
    for p, h in sha.items():
        ents[p] = ("file", h, mode.get(p))
    for p, t in link.items():
        if p in ents:
            sys.exit("%s: %s is both file and link" % (name, p))
        ents[p] = ("link", t, None)
    return ents


def read_manifest(path):
    if path == "-":
        return parse(sys.stdin.read(), "stdin")
    with open(path, encoding="utf-8", errors="surrogateescape") as f:
        return parse(f.read(), path)


def diff(old, new, allow):
    """Print differences (old = recorded, new = now); return the bad count."""
    bad = 0
    for p in sorted(set(old) | set(new)):
        a, b = old.get(p), new.get(p)
        if a == b:
            continue
        if a is None:
            msg = "added: %s" % p
        elif b is None:
            msg = "removed: %s" % p
        else:
            why = []
            if a[0] != b[0]:
                why.append("%s -> %s" % (a[0], b[0]))
            elif a[1] != b[1]:
                why.append("content" if a[0] == "file" else
                           "link target %s -> %s" % (a[1], b[1]))
            if a[0] == b[0] == "file" and a[2] != b[2]:
                why.append("mode %04o -> %04o" % (a[2] or 0, b[2] or 0))
            msg = "changed: %s (%s)" % (p, ", ".join(why))
        if any(p.startswith(x) for x in allow):
            msg += " (allowed)"
        else:
            bad += 1
        print(msg)
    if bad == 0:
        print("same: %d files" % len(new))
    return bad


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("record")
    r.add_argument("dir")
    r.add_argument("-o", "--output", required=True)
    for n, args in (("compare", ("dir", "file")), ("compare-records", ("a", "b"))):
        c = sub.add_parser(n)
        for a in args:
            c.add_argument(a)
        c.add_argument("--allow", action="append", metavar="PREFIX")
    a = ap.parse_args(argv)
    if a.cmd in ("record", "compare") and not os.path.isdir(a.dir):
        sys.exit("not a directory: %s" % a.dir)
    if a.cmd == "record":
        text = render(scan(a.dir))
        if a.output == "-":
            sys.stdout.write(text)
        else:
            with open(a.output, "w", encoding="utf-8",
                      errors="surrogateescape") as f:
                f.write(text)
        return 0
    allow = a.allow if a.allow else DEFAULT_ALLOW
    if a.cmd == "compare":
        old, new = read_manifest(a.file), scan(a.dir)
    else:
        if a.a == "-" and a.b == "-":
            sys.exit("only one side can be '-'")
        old, new = read_manifest(a.a), read_manifest(a.b)
    return 1 if diff(old, new, allow) else 0


if __name__ == "__main__":
    sys.exit(main())
