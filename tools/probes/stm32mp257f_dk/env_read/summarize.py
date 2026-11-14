#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""summarize.py DIR [AFTER_DIR]: render collect.sh's directory as markdown.

One directory: the Linux-side rows of section 11 of
docs/notes/stm32mp257f-dk-uboot-boot.md (the fw_printenv variables, the
u-boot-env partition checked with tools/uboot_env.py, the /boot listing).
Exit 0, or 1 when the environment image fails its check.

Two directories (before and after): the comparison c5 needs. It lists the
fw_printenv diff, the env image's sha256 and active copy, and the /boot
files whose sha256 changed, were added or removed, ignoring ./yakogl/.
Exit 0 when nothing differs outside yakogl/, 1 otherwise (2: usage).

Layout of the environment: MP2_ENV_SIZE, MP2_ENV_OFFSET, MP2_ENV_OFFSET2
(as tools/board-stm32mp2.sh), default 0x2000, 0x7e000, 0x7c000, redundant.
"""
import hashlib
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
UBOOT_ENV = os.path.join(HERE, "..", "..", "..", "uboot_env.py")
VARS = ("scriptaddr kernel_addr_r fdt_addr_r ramdisk_addr_r boot_targets "
        "boot_device boot_instance boot_prefixes boot_scripts "
        "boot_syslinux_conf bootdelay bootdelaykey bootcmd preboot "
        "bootcount bootlimit altbootcmd").split()
IGNORE = "./yakogl/"
LAYOUT = ["--redundant",
          "--size", os.environ.get("MP2_ENV_SIZE", "0x2000"),
          "--offset", os.environ.get("MP2_ENV_OFFSET", "0x7e000"),
          "--offset2", os.environ.get("MP2_ENV_OFFSET2", "0x7c000")]


def text(d, name):
    try:
        with open(os.path.join(d, name), encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return None


def env_vars(d):
    """fw_printenv.txt as a dict (None when missing)."""
    t = text(d, "fw_printenv.txt")
    if t is None:
        return None
    out = {}
    for line in t.splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            out[k] = v
    return out


def uboot_env(*args):
    r = subprocess.run([sys.executable, "-I", UBOOT_ENV] + LAYOUT + list(args),
                       capture_output=True, text=True)
    return r.returncode, (r.stdout + r.stderr).strip()


def env_image(d):
    """(sha256, size, check rc, check output, active copy) of u-boot-env.bin."""
    p = os.path.join(d, "u-boot-env.bin")
    try:
        with open(p, "rb") as f:
            data = f.read()
    except OSError:
        return None, 0, 2, "u-boot-env.bin missing", None
    rc, out = uboot_env("check", p)
    active = None
    for line in out.splitlines():
        if line.startswith("active: "):
            active = line[8:]
    return hashlib.sha256(data).hexdigest(), len(data), rc, out, active


def boot_hashes(d):
    """{path: sha256} from boot-sha256.txt, paths as ./name."""
    out = {}
    for line in (text(d, "boot-sha256.txt") or "").splitlines():
        if len(line) > 66 and line[64:66] == "  ":
            name = line[66:]
            if not name.startswith(("./", "/")):
                name = "./" + name
            out[name] = line[:64]
    return out


def boot_sizes(d):
    out = {}
    for line in (text(d, "boot-sizes.txt") or "").splitlines():
        a, _, b = line.partition(" ")
        if a.isdigit() and b:
            out[b] = int(a)
    return out


def stock(h):
    return {k: v for k, v in h.items() if not k.startswith(IGNORE)}


def cell(s):
    return str(s).replace("|", "\\|").replace("\n", " ")


def rows(title, head, body):
    print("### " + title + "\n")
    print("| " + " | ".join(head) + " |")
    print("|" + "---|" * len(head))
    for r in body:
        print("| " + " | ".join(cell(c) for c in r) + " |")
    print()


def single(d):
    ev = env_vars(d)
    bad = 0
    if ev is None:
        print("env_read: %s has no fw_printenv.txt" % d, file=sys.stderr)
        bad = 1
        ev = {}
    rows("fw_printenv", ["variable", "value"],
         [(v, ev[v] if v in ev else "(unset)") for v in VARS])
    sha, size, rc, out, active = env_image(d)
    dev = (text(d, "u-boot-env.dev") or "").strip()
    body = [("partition size", "%d bytes (0x%x)" % (size, size)),
            ("layout", " ".join(LAYOUT)),
            ("image sha256", sha or "(missing)"),
            ("active copy", active or "(none)")]
    for line in out.splitlines():
        body.append(("check", line))
    body.append(("check result", "ok" if rc == 0 else "FAILED (rc %d)" % rc))
    del dev  # the device path is the board's: it stays out of the table
    rows("u-boot-env partition", ["item", "value"], body)
    if rc:
        bad = 1
    mnt = (text(d, "boot-mount.txt") or "").split()
    part = ""
    if mnt:
        digits = mnt[0].rsplit("p", 1)[-1]
        part = digits if digits.isdigit() else ""
    df = (text(d, "boot-df.txt") or "").strip().splitlines()
    free = "(unread)"
    if len(df) >= 2 and len(df[-1].split()) >= 6:
        f = df[-1].split()
        free = "%s KiB free of %s KiB (%s used)" % (f[3], f[1], f[4])
    rows("/boot", ["item", "value"],
         [("bootfs partition number", part or "(unread)"),
          ("file system", mnt[1] if len(mnt) > 1 else "(unread)"),
          ("free space", free)])
    h, sz = boot_hashes(d), boot_sizes(d)
    rows("/boot files", ["file", "bytes", "sha256"],
         [(k, sz.get(k, "?"), h[k]) for k in sorted(h)])
    return bad


def compare(a, b):
    diffs = 0
    ea, eb = env_vars(a), env_vars(b)
    if ea is None or eb is None:
        print("env_read: a directory has no fw_printenv.txt", file=sys.stderr)
        return 1
    body = []
    for k in sorted(set(ea) | set(eb)):
        if ea.get(k) != eb.get(k):
            body.append((k, ea.get(k, "(unset)"), eb.get(k, "(unset)")))
    print("## fw_printenv diff: %s\n" % ("empty" if not body else "%d variable(s)" % len(body)))
    if body:
        rows("changed", ["variable", "before", "after"], body)
        diffs += len(body)
    sa, za, ra, _, aa = env_image(a)
    sb, zb, rb, _, ab = env_image(b)
    same = sa is not None and sa == sb
    rows("u-boot-env image", ["item", "before", "after"],
         [("sha256", sa or "(missing)", sb or "(missing)"),
          ("bytes", za, zb),
          ("active copy", aa or "(none)", ab or "(none)"),
          ("check", "ok" if ra == 0 else "FAILED", "ok" if rb == 0 else "FAILED"),
          ("identical", "yes" if same else "NO", "")])
    if not same or ra or rb:
        diffs += 1
    ha, hb = stock(boot_hashes(a)), stock(boot_hashes(b))
    if not ha or not hb:
        print("env_read: no /boot hashes to compare\n", file=sys.stderr)
        diffs += 1
    body = []
    for k in sorted(set(ha) | set(hb)):
        if ha.get(k) != hb.get(k):
            body.append((k, ha.get(k, "(absent)"), hb.get(k, "(absent)")))
    print("## /boot files outside yakogl/: %s\n" % ("unchanged" if not body else "%d differ" % len(body)))
    if body:
        rows("changed", ["file", "before", "after"], body)
        diffs += len(body)
    only = sorted(k for k in set(boot_hashes(a)) | set(boot_hashes(b))
                  if k.startswith(IGNORE) and boot_hashes(a).get(k) != boot_hashes(b).get(k))
    if only:
        print("(ignored: %d file(s) under yakogl/ differ)\n" % len(only))
    print("RESULT: %s" % ("IDENTICAL" if not diffs else "DIFFERENT (%d)" % diffs))
    return 1 if diffs else 0


def main(argv):
    if len(argv) not in (2, 3) or argv[1] in ("-h", "--help"):
        print(__doc__, file=sys.stderr)
        return 2
    for d in argv[1:]:
        if not os.path.isdir(d):
            print("env_read: %s is not a directory" % d, file=sys.stderr)
            return 2
    return single(argv[1]) if len(argv) == 2 else compare(argv[1], argv[2])


if __name__ == "__main__":
    sys.exit(main(sys.argv))
