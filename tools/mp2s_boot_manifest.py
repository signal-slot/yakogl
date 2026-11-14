#!/usr/bin/env python3
"""sha256 manifest of the STM32MP257F-DK's /boot, and its byte-for-byte compare.

docs/notes/stm32mp257f-dk-standalone.md sections 5 and 7 (acceptance c6).

  mp2s_boot_manifest.py record  <bootdir> <manifest>
  mp2s_boot_manifest.py compare <bootdir> <manifest> [--gone]

`record` writes one line per file under <bootdir>, `<sha256>  <relative
path>`, sorted by path (a symlink is `link  <path>  -> <target>`). Anything
under yakogl/standalone/ is left out: it is ours, and the manifest lives there.

`compare` lists every file that is changed, missing or new against the
manifest. The only allowed difference is yakogl/standalone/; with --gone that
directory must not exist either (after `uninstall`). Exit 0: identical.
Exit 1: a difference (the list is printed, then `NEEDS A PERSON`; the runner
never repairs ST's files). Exit 2: usage or an unreadable input.
"""
import hashlib
import os
import sys

OURS = "yakogl/standalone"


def ours(rel):
    return rel == OURS or rel.startswith(OURS + "/")


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def scan(root):
    """{relative path: 'sha256' or 'link -> target'} for every non-ours file."""
    out = {}
    for d, dirs, files in os.walk(root):
        dirs.sort()
        reld = os.path.relpath(d, root)
        names = files + [x for x in dirs if os.path.islink(os.path.join(d, x))]
        for n in sorted(names):
            rel = n if reld == "." else reld + "/" + n
            if ours(rel):
                continue
            p = os.path.join(root, rel)
            if os.path.islink(p):
                out[rel] = "link -> " + os.readlink(p)
            else:
                out[rel] = digest(p)
    return out


def render(entries):
    lines = []
    for rel in sorted(entries):
        v = entries[rel]
        if v.startswith("link -> "):
            lines.append("link  %s  %s\n" % (rel, v[5:]))
        else:
            lines.append("%s  %s\n" % (v, rel))
    return "".join(lines)


def parse(text):
    out = {}
    for ln in text.splitlines():
        if not ln:
            continue
        if ln.startswith("link  "):
            rel, sep, tgt = ln[6:].partition("  -> ")
            if not sep:
                raise ValueError("bad manifest line: %r" % ln)
            out[rel] = "link -> " + tgt
        else:
            h, _, rel = ln.partition("  ")
            if len(h) != 64 or not rel:
                raise ValueError("bad manifest line: %r" % ln)
            out[rel] = h
    return out


def main(argv):
    gone = "--gone" in argv
    args = [a for a in argv if a != "--gone"]
    if len(args) != 3 or args[0] not in ("record", "compare") or (gone and args[0] != "compare"):
        sys.stderr.write(__doc__)
        return 2
    verb, root, manifest = args
    if not os.path.isdir(root):
        sys.stderr.write("mp2s-manifest: not a directory: %s\n" % root)
        return 2
    try:
        now = scan(root)
        if verb == "record":
            tmp = manifest + ".part"
            with open(tmp, "w") as f:
                f.write(render(now))
            os.replace(tmp, manifest)
            print("mp2s-manifest: recorded %d files" % len(now))
            return 0
        with open(manifest) as f:
            rec = parse(f.read())
    except (OSError, ValueError) as e:
        sys.stderr.write("mp2s-manifest: %s\n" % e)
        return 2
    bad = ["changed  " + r for r in sorted(rec) if r in now and rec[r] != now[r]]
    bad += ["missing  " + r for r in sorted(rec) if r not in now]
    bad += ["new      " + r for r in sorted(now) if r not in rec]
    if gone and os.path.lexists(os.path.join(root, OURS)):
        bad.append("present  %s/ (must be gone)" % OURS)
    if bad:
        print("mp2s-manifest: /boot differs from the install record:")
        for b in bad:
            print("  " + b)
        print("mp2s-manifest: NEEDS A PERSON (ST's files are never repaired here)")
        return 1
    print("mp2s-manifest: %d files identical to the install record" % len(rec))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
