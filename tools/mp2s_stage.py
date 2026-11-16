#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2s_stage.py - the files of the DK's standalone boot under
/boot/yakogl/standalone (plan #362, item stage-files).

    tools/mp2s_stage.py [--root /] install --script boot.scr.uimg --env-image u-boot-env.bin
    tools/mp2s_stage.py [--root /] default --image <image> [--script-sha256 <hex>] [--free-bytes N]
    tools/mp2s_stage.py [--root /] remove-default
    tools/mp2s_stage.py [--root /] uninstall
    tools/mp2s_stage.py [--root /] status

docs/notes/stm32mp257f-dk-standalone.md, section 7 ("Files on bootfs") and
section 8. Python 3 stdlib only, so it runs on the board's Linux (as root) or
on the host against a mounted or fake root (--root). It writes files and
nothing else: never the environment, never a device, never a file outside
<root>/boot/yakogl/standalone, never one whose name is not in the table below.

    boot.scr.uimg            install   our script (a uImage)
    boot.scr.uimg.sha256     install   its sha256, "<hex>  boot.scr.uimg"
    u-boot-env.bin           install   the pristine environment image
    boot-sha256.txt          install   the record: every file under /boot
                                       but yakogl/standalone/
    default.bin              default   the image
    boot.scr.uimg.off        (tools/mp2s_card_undo.sh, read here, never written)

Every file is written as <name>.part, fsync'ed, renamed and the directory
fsync'ed; then its cached pages are dropped (posix_fadvise DONTNEED) and it
is read back by sha256. A mismatch removes the file and nothing else changes
(section 8, "staging or its read-back"). Before it writes, install and default
check that /boot is the bootfs partition of an SD card (mountinfo and sysfs
under --root; never the eMMC), that yakogl/ and yakogl/standalone/ are plain
directories, and that the stage directory holds nothing but the names above.

The record lines are "<sha256>  <path>" for a regular file and
"symlink  <path> -> <target>" for a symlink, relative to /boot and sorted. A
second install compares /boot with the existing record instead of taking a
new one, so a stock file that changed in between is reported, not recorded.
status and uninstall compare too; uninstall reads the record, removes the
stage directory (never yakogl/, which the one-shot shares) and then compares.

Exit codes: 0 done; 1 a write or its read-back failed (that file removed);
2 usage; 3 refused before anything changed; 4 NEEDS A PERSON: /boot differs
from the install record (the differences are listed; nothing is repaired).
"""
import argparse
import hashlib
import os
import re
import stat
import sys

STAGE = ("boot", "yakogl", "standalone")
SCRIPT = "boot.scr.uimg"
SCRIPT_SHA = "boot.scr.uimg.sha256"
ENV_IMAGE = "u-boot-env.bin"
RECORD = "boot-sha256.txt"
DEFAULT = "default.bin"
SCRIPT_OFF = "boot.scr.uimg.off"
INSTALL_FILES = (RECORD, ENV_IMAGE, SCRIPT_SHA, SCRIPT)  # written in this order
KNOWN = set(INSTALL_FILES) | {DEFAULT, SCRIPT_OFF}
PART = ".part"
UIMAGE_MAGIC = b"\x27\x05\x19\x56"
ENV_MAX = 512 * 1024             # the u-boot-env partition
LOAD_MAX = 256 * 1024 * 1024     # the window at 0xa0000000
CHUNK = 1 << 20


class Refuse(Exception):
    """Exit 3: nothing was changed."""


class Failed(Exception):
    """Exit 1: a write or its read-back failed; that file was removed."""


class Person(Exception):
    """Exit 4: /boot differs from the install record."""


def say(msg):
    print("stage: " + msg, flush=True)


# ---- paths -----------------------------------------------------------------
def stage_dir(root):
    return os.path.join(root, *STAGE)


def boot_dir(root):
    return os.path.join(root, "boot")


def is_part(name):
    return name.endswith(PART) and name[:-len(PART)] in KNOWN


def stage_path(root, name):
    """The one way to a path in the stage directory: NAME is a fixed name of
    the table (or its .part)."""
    if name not in KNOWN and not is_part(name):
        raise Refuse("'%s' is not a file of the stage directory" % name)
    return os.path.join(stage_dir(root), name)


def check_dirs(root):
    """yakogl/ and yakogl/standalone/, where they exist, are directories and
    not symlinks (a symlink would carry a write off bootfs). Returns whether
    the stage directory exists."""
    p = boot_dir(root)
    if not os.path.isdir(p):
        raise Refuse("no %s directory" % p)
    for part in STAGE[1:]:
        p = os.path.join(p, part)
        try:
            st = os.lstat(p)
        except FileNotFoundError:
            return False
        if not stat.S_ISDIR(st.st_mode):
            raise Refuse("%s is not a plain directory (symlink or file)" % p)
    return True


def stage_names(root):
    """The entries of the stage directory; refuses an unknown name and
    anything but a regular file."""
    d = stage_dir(root)
    names = sorted(os.listdir(d))
    for n in names:
        if n not in KNOWN and not is_part(n):
            raise Refuse("%s holds '%s', which is not ours: nothing is removed "
                         "or written" % (d, n))
        if not stat.S_ISREG(os.lstat(os.path.join(d, n)).st_mode):
            raise Refuse("%s/%s is not a regular file" % (d, n))
    return names


# ---- /boot is the SD card's bootfs ----------------------------------------
def rd(root, rel):
    try:
        with open(os.path.join(root, rel.lstrip("/"))) as fh:
            return fh.read().strip()
    except OSError:
        return None


def check_bootfs(root):
    """/boot is mounted from a partition named bootfs on a disk that says SD
    and is not mmcblk0 (tools/board-stm32mp2.sh dev_guard)."""
    src = None
    for ln in (rd(root, "proc/self/mountinfo") or "").splitlines():
        f = ln.split()
        if len(f) > 9 and f[4] == "/boot" and "-" in f:
            src = f[f.index("-") + 2]
    if src is None:
        raise Refuse("nothing is mounted on /boot")
    name = os.path.basename(src)
    m = re.fullmatch(r"(mmcblk[0-9]+)p[0-9]+", name)
    if not m:
        raise Refuse("/boot is mounted from %s, not a partition of an mmcblk "
                     "disk" % src)
    disk = m.group(1)
    if disk == "mmcblk0":
        raise Refuse("/boot is on mmcblk0 (the eMMC's index): never written")
    kind = rd(root, "sys/block/%s/device/type" % disk)
    if kind != "SD":
        raise Refuse("/boot is on %s of type '%s', not SD (the eMMC?)"
                     % (disk, kind if kind is not None else "unknown"))
    partname = None
    for ln in (rd(root, "sys/class/block/%s/uevent" % name) or "").splitlines():
        if ln.startswith("PARTNAME="):
            partname = ln[len("PARTNAME="):]
    if partname != "bootfs":
        raise Refuse("/boot is partition '%s' of the card, not bootfs"
                     % (partname or "unnamed"))
    return src


# ---- hashing and writing ---------------------------------------------------
def sha256_file(path, drop_cache=False):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        if drop_cache and hasattr(os, "posix_fadvise"):
            os.posix_fadvise(fh.fileno(), 0, 0, os.POSIX_FADV_DONTNEED)
        while True:
            b = fh.read(CHUNK)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def readback(path):
    """The file's sha256 as read back from the card, its cached pages
    dropped first."""
    return sha256_file(path, drop_cache=True)


def fsync_dir(d):
    fd = os.open(d, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def remove(root, name):
    p = stage_path(root, name)
    try:
        os.unlink(p)
        return True
    except FileNotFoundError:
        return False


def stage_file(root, name, src=None, data=None):
    """Write NAME from the file SRC or the bytes DATA: .part, fsync, rename,
    fsync of the directory, read-back. Returns (size, sha256)."""
    part = stage_path(root, name + PART)
    final = stage_path(root, name)
    remove(root, name + PART)
    h = hashlib.sha256()
    size = 0
    try:
        fd = os.open(part, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                     0o644)
        with os.fdopen(fd, "wb") as out:
            if data is not None:
                out.write(data)
                h.update(data)
                size = len(data)
            else:
                with open(src, "rb") as fh:
                    while True:
                        b = fh.read(CHUNK)
                        if not b:
                            break
                        out.write(b)
                        h.update(b)
                        size += len(b)
            out.flush()
            os.fsync(out.fileno())
        os.rename(part, final)
        fsync_dir(stage_dir(root))
    except OSError as e:
        remove(root, name + PART)
        remove(root, name)
        raise Failed("writing %s: %s; %s removed" % (final, e, name))
    want = h.hexdigest()
    got = readback(final)
    if got != want:
        remove(root, name)
        fsync_dir(stage_dir(root))
        raise Failed("read-back of %s: sha256 %s, wrote %s; %s removed"
                     % (final, got, want, name))
    return size, want


# ---- the /boot record ------------------------------------------------------
def boot_record(root):
    """The record lines of every file under /boot but yakogl/standalone/."""
    base = boot_dir(root)
    skip = os.path.join(*STAGE[1:])
    lines = []

    def walk(rel):
        d = os.path.join(base, rel)
        try:
            names = sorted(os.listdir(d))
        except OSError as e:
            raise Refuse("cannot list %s: %s" % (d, e))
        for n in names:
            r = os.path.join(rel, n) if rel else n
            if r == skip:
                continue
            p = os.path.join(base, r)
            st = os.lstat(p)
            if stat.S_ISLNK(st.st_mode):
                lines.append("symlink  %s -> %s" % (r, os.readlink(p)))
            elif stat.S_ISDIR(st.st_mode):
                walk(r)
            elif stat.S_ISREG(st.st_mode):
                try:
                    lines.append("%s  %s" % (sha256_file(p), r))
                except OSError as e:
                    raise Refuse("cannot read %s: %s" % (p, e))
            else:
                lines.append("special  %s" % r)

    walk("")
    return sorted(lines, key=lambda ln: ln.split("  ", 1)[1])


def record_diff(want, got):
    """Lines naming what differs between two records (by path)."""
    def by_path(lines):
        return {ln.split("  ", 1)[1].split(" -> ", 1)[0]: ln for ln in lines}
    w, g = by_path(want), by_path(got)
    out = []
    for p in sorted(set(w) | set(g)):
        if p not in g:
            out.append("missing  %s" % p)
        elif p not in w:
            out.append("new      %s" % p)
        elif w[p] != g[p]:
            out.append("changed  %s" % p)
    return out


def read_record(root):
    p = stage_path(root, RECORD)
    try:
        with open(p) as fh:
            return [ln for ln in fh.read().splitlines() if ln]
    except FileNotFoundError:
        return None


def compare(root, want):
    diff = record_diff(want, boot_record(root))
    if diff:
        for ln in diff:
            say("differs: " + ln)
        raise Person("NEEDS A PERSON: %d path(s) under /boot differ from the "
                     "install record; nothing was repaired" % len(diff))
    say("/boot matches the install record (%d entries)" % len(want))


def script_ok(root):
    """The staged script hashes to its .sha256 file; returns the hash."""
    try:
        with open(stage_path(root, SCRIPT_SHA)) as fh:
            line = fh.read().split()
    except FileNotFoundError:
        raise Refuse("not installed: no %s" % SCRIPT_SHA)
    if len(line) != 2 or line[1] != SCRIPT or not re.fullmatch(r"[0-9a-f]{64}", line[0]):
        raise Refuse("%s is not '<sha256>  %s'" % (SCRIPT_SHA, SCRIPT))
    p = stage_path(root, SCRIPT)
    if not os.path.exists(p):
        raise Refuse("not installed: no %s" % SCRIPT)
    got = sha256_file(p)
    if got != line[0]:
        raise Refuse("the staged %s hashes to %s, its .sha256 says %s"
                     % (SCRIPT, got, line[0]))
    return got


# ---- verbs -----------------------------------------------------------------
def cmd_install(root, a):
    with open(a.script, "rb") as fh:
        head = fh.read(64)
    if len(head) < 64 or head[:4] != UIMAGE_MAGIC:
        raise Refuse("%s is not a uImage (mkimage -T script)" % a.script)
    env_size = os.path.getsize(a.env_image)
    if not 0 < env_size <= ENV_MAX:
        raise Refuse("%s is %d bytes; an environment image is 1 to %d bytes"
                     % (a.env_image, env_size, ENV_MAX))
    check_bootfs(root)
    if check_dirs(root):
        names = stage_names(root)
        if DEFAULT in names or DEFAULT + PART in names:
            raise Refuse("%s is staged: a default may be set; nodefault first"
                         % DEFAULT)
        if SCRIPT_OFF in names:
            raise Refuse("%s is there (the card undo was run): uninstall first"
                         % SCRIPT_OFF)
    old = read_record(root) if os.path.isdir(stage_dir(root)) else None
    if old is not None:
        compare(root, old)
        record = None
    else:
        record = boot_record(root)
    os.makedirs(stage_dir(root), mode=0o755, exist_ok=True)
    check_dirs(root)
    if record is not None:
        stage_file(root, RECORD, data="".join(ln + "\n" for ln in record).encode())
        say("recorded %d entries under /boot in %s" % (len(record), RECORD))
    stage_file(root, ENV_IMAGE, src=a.env_image)
    want = sha256_file(a.script)
    stage_file(root, SCRIPT_SHA, data=("%s  %s\n" % (want, SCRIPT)).encode())
    size, got = stage_file(root, SCRIPT, src=a.script)
    if got != want:
        remove(root, SCRIPT)
        raise Failed("%s changed while it was staged; %s removed" % (a.script, SCRIPT))
    say("installed %s (%d bytes, sha256 %s); no environment write"
        % (SCRIPT, size, got))


def cmd_default(root, a):
    size = os.path.getsize(a.image)
    if not 0x40 <= size <= LOAD_MAX:
        raise Refuse("%s is %d bytes; the window takes 0x40 to 0x%x"
                     % (a.image, size, LOAD_MAX))
    check_bootfs(root)
    if not check_dirs(root):
        raise Refuse("not installed: no %s" % stage_dir(root))
    names = stage_names(root)
    if DEFAULT in names:
        raise Refuse("%s is already staged: nodefault first" % DEFAULT)
    if SCRIPT_OFF in names:
        raise Refuse("%s is there (the card undo was run): uninstall, then install"
                     % SCRIPT_OFF)
    got = script_ok(root)
    if a.script_sha256 is not None and got != a.script_sha256.lower():
        raise Refuse("the staged %s is %s, not the expected %s"
                     % (SCRIPT, got, a.script_sha256))
    free = a.free_bytes
    if free is None:
        st = os.statvfs(stage_dir(root))
        free = st.f_bavail * st.f_frsize
    if size > free // 2:
        raise Refuse("%s is %d bytes, more than half of bootfs's %d free bytes"
                     % (a.image, size, free))
    size, h = stage_file(root, DEFAULT, src=a.image)
    say("staged %s: %d bytes, sha256 %s (read back)" % (DEFAULT, size, h))


def cmd_remove_default(root, _a):
    if not check_dirs(root):
        say("nothing staged: no %s" % stage_dir(root))
        return
    stage_names(root)
    gone = [n for n in (DEFAULT + PART, DEFAULT) if remove(root, n)]
    fsync_dir(stage_dir(root))
    say("removed %s" % DEFAULT if DEFAULT in gone else "no %s staged" % DEFAULT)


def cmd_uninstall(root, _a):
    if not check_dirs(root):
        say("nothing installed: no %s" % stage_dir(root))
        return
    names = stage_names(root)
    if DEFAULT in names:
        raise Refuse("%s is staged: a default may be set; nodefault first" % DEFAULT)
    record = read_record(root)
    for n in names:
        remove(root, n)
    os.rmdir(stage_dir(root))
    fsync_dir(os.path.dirname(stage_dir(root)))
    say("removed %s (%s)" % (stage_dir(root), ", ".join(names) or "empty"))
    if record is None:
        raise Person("NEEDS A PERSON: there was no install record (%s) to "
                     "compare /boot with" % RECORD)
    compare(root, record)


def cmd_status(root, _a):
    if not check_dirs(root):
        say("not installed: no %s" % stage_dir(root))
        return
    names = stage_names(root)
    for n in names:
        p = stage_path(root, n)
        say("%-22s %10d bytes  sha256 %s" % (n, os.path.getsize(p), sha256_file(p)))
    if SCRIPT in names:
        script_ok(root)
        say("%s matches %s" % (SCRIPT, SCRIPT_SHA))
    record = read_record(root)
    if record is None:
        raise Person("NEEDS A PERSON: no install record (%s)" % RECORD)
    compare(root, record)


VERBS = {
    "install": cmd_install,
    "default": cmd_default,
    "remove-default": cmd_remove_default,
    "uninstall": cmd_uninstall,
    "status": cmd_status,
}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default="/")
    sub = ap.add_subparsers(dest="verb", required=True)
    p = sub.add_parser("install")
    p.add_argument("--script", required=True)
    p.add_argument("--env-image", required=True)
    p = sub.add_parser("default")
    p.add_argument("--image", required=True)
    p.add_argument("--script-sha256")
    p.add_argument("--free-bytes", type=int,
                   help="bootfs's free bytes instead of statvfs (a host root)")
    for v in ("remove-default", "uninstall", "status"):
        sub.add_parser(v)
    a = ap.parse_args(argv)
    for f in ("script", "env_image", "image"):
        v = getattr(a, f, None)
        if v is not None and not os.path.isfile(v):
            say("no such file: %s" % v)
            return 2
    root = os.path.abspath(a.root)
    try:
        VERBS[a.verb](root, a)
    except Refuse as e:
        say("REFUSE %s" % e)
        return 3
    except Failed as e:
        say("FAILED %s" % e)
        return 1
    except Person as e:
        say(str(e))
        return 4
    return 0


if __name__ == "__main__":
    sys.exit(main())
