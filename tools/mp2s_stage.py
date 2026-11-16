#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2s_stage.py - stage the DK standalone files under <root>/boot/yakogl/standalone/.

Runs on the board's Linux or on the host against a root directory.  Contract:
docs/notes/stm32mp257f-dk-standalone.md section 7 ("Files on bootfs").

  mp2s_stage.py [--root /] check-dir
  mp2s_stage.py [--root /] put <src> <relname>
  mp2s_stage.py [--root /] remove <relname>
  mp2s_stage.py [--root /] remove-all

Every write action prints one line "UNDO: ..." before it touches anything.
Exit 0 on success, 1 on a refusal or failure (the reason on stderr, naming the
file), 2 on a usage error.  Python 3 standard library only.

Test hook (selftest only): MP2S_STAGE_FAULT=readback makes the read-back
differ from what was written.
"""
import argparse
import hashlib
import os
import sys

REL = os.path.join("boot", "yakogl", "standalone")
CONTRACT = (
    "boot.scr.uimg",
    "boot.scr.uimg.sha256",
    "default.bin",
    "u-boot-env.bin",
    "boot-sha256.txt",
    "boot.scr.uimg.off",  # the card-undo tool's rename (section 5)
)
SELF = os.path.abspath(__file__)


class Refuse(Exception):
    pass


def _base(root):
    """The standalone directory; refuse if it resolves anywhere else."""
    base = os.path.join(os.path.realpath(root), REL)
    if os.path.lexists(base) and os.path.realpath(base) != base:
        raise Refuse("%s resolves to %s, outside the standalone directory"
                     % (base, os.path.realpath(base)))
    return base


def _path(root, relname):
    if relname not in CONTRACT:
        raise Refuse("%s: not a file of the contract (%s)" % (relname, ", ".join(CONTRACT)))
    p = os.path.join(_base(root), relname)
    if os.path.islink(p):
        raise Refuse("%s: is a symlink" % p)
    return p


def _scan(base):
    """Return (contract files present, foreign paths)."""
    known, foreign = [], []
    if not os.path.lexists(base):
        return known, foreign
    for dirpath, dirnames, filenames in os.walk(base, followlinks=False):
        rel_d = os.path.relpath(dirpath, base)
        for d in list(dirnames):
            full = os.path.join(dirpath, d)
            rel = os.path.normpath(os.path.join(rel_d, d))
            foreign.append(full)
            dirnames.remove(d)
        for f in filenames:
            full = os.path.join(dirpath, f)
            rel = os.path.normpath(os.path.join(rel_d, f))
            if rel in CONTRACT and not os.path.islink(full):
                known.append(rel)
            else:
                foreign.append(full)
    return known, foreign


def check_dir(root):
    base = _base(root)
    _, foreign = _scan(base)
    if foreign:
        raise Refuse("%s: foreign file in %s (not in the contract)" % (foreign[0], base))
    return base


def _fsync_dir(d):
    fd = os.open(d, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def _sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for blk in iter(lambda: f.read(1 << 20), b""):
            h.update(blk)
    return h.hexdigest()


def _cmd(root, *args):
    r = ["--root", root] if root != "/" else []
    return " ".join(["python3", SELF] + r + list(args))


def _undo(msg):
    print("UNDO: " + msg, flush=True)


def put(root, src, relname):
    dst = _path(root, relname)
    check_dir(root)
    _undo(_cmd(root, "remove", relname))
    if not os.path.isfile(src):
        raise Refuse("%s: source is not a regular file" % src)
    want = _sha(src)
    part = dst + ".part"
    d = os.path.dirname(dst)
    try:
        os.makedirs(d, exist_ok=True)
        if os.path.realpath(d) != d:
            raise Refuse("%s: resolves outside the standalone directory" % d)
        if os.path.lexists(part):
            os.unlink(part)
        fd = os.open(part, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
        with os.fdopen(fd, "wb") as out, open(src, "rb") as inp:
            for blk in iter(lambda: inp.read(1 << 20), b""):
                out.write(blk)
            out.flush()
            os.fsync(out.fileno())
        got = _sha(part)
        if os.environ.get("MP2S_STAGE_FAULT") == "readback":
            got = "0" * 64
        if got != want:
            raise Refuse("%s: read-back sha256 %s differs from source %s" % (dst, got, want))
        os.rename(part, dst)
        _fsync_dir(d)
    except BaseException:
        for p in (part, dst):
            try:
                os.unlink(p)
            except FileNotFoundError:
                pass
        raise
    print("put %s sha256 %s" % (relname, want))


def remove(root, relname):
    p = _path(root, relname)
    check_dir(root)
    _undo("%s   (the removed content is not kept)" % _cmd(root, "put", "<source>", relname))
    if os.path.lexists(p):
        os.unlink(p)
        _fsync_dir(os.path.dirname(p))
    print("removed %s" % relname)


def _rmdir_if_empty(d):
    try:
        os.rmdir(d)
        return True
    except OSError:
        return False


def remove_all(root):
    base = _base(root)
    if not os.path.lexists(base):
        print("nothing to remove")
        return
    check_dir(root)
    _undo("%s for each file removed   (the content is not kept)"
          % _cmd(root, "put", "<source>", "<relname>"))
    known, _ = _scan(base)
    for rel in known:
        os.unlink(os.path.join(base, rel))
    if _rmdir_if_empty(base):
        _fsync_dir(os.path.dirname(base))
    print("removed %d file(s)" % len(known))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default="/")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check-dir")
    p = sub.add_parser("put")
    p.add_argument("src")
    p.add_argument("relname")
    p = sub.add_parser("remove")
    p.add_argument("relname")
    sub.add_parser("remove-all")
    a = ap.parse_args(argv)
    try:
        if a.cmd == "check-dir":
            print("ok %s" % check_dir(a.root))
        elif a.cmd == "put":
            put(a.root, a.src, a.relname)
        elif a.cmd == "remove":
            remove(a.root, a.relname)
        else:
            remove_all(a.root)
    except Refuse as e:
        print("mp2s_stage: refused: %s" % e, file=sys.stderr)
        return 1
    except OSError as e:
        print("mp2s_stage: failed: %s" % e, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
