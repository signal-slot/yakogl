#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Static scan: no mp2s source writes protected variables, partitions or the eMMC.

    tools/mp2s_protect_scan.py [FILE...]
    tools/mp2s_protect_scan.py --self-test

With no FILE the scan reads the DK standalone sources (plan #362):
tools/board-stm32mp2-standalone.sh, the non-selftest tools/mp2s_* files other
than this one, and driver/tools/mp2-standalone/*.cmd and build.sh. Exit 0
clean, 3 with one `PROTECT: file:line: why: text` line per hit on stderr, 2 on
a usage error or an unreadable file.

The rules (docs/notes/stm32mp257f-dk-standalone.md, sections 1 and 8):
  var    never write the U-Boot variables bootcmd, bootcount, altbootcmd
         (setenv, env set/delete, fw_setenv, including a `-s -` script of
         NAME=VALUE lines, and the other ST names bootcmd_*, bootlimit,
         upgrade_available);
  emmc   never write anything on the eMMC: /dev/mmcblk0*, `mmc dev 0`,
         `mmc write|erase ... 0`, ext4write/fatwrite on `mmc 0`;
  part   never write the partitions fsbla1, fsbla2, metadata1, metadata2,
         fip-a, fip-b (a write verb on a line that names one: dd of=, a
         redirect, cp, install, mkfs, sgdisk, sfdisk, parted, mmc write,
         ext4write, fatwrite, fw_setenv ...).
A line is a hit only when it is code: comments, strings given to
echo/printf/note/die/warn (no redirect) and the definitions of the deny lists
themselves (NEVER_LABELS=, EMMC_RE=) are text. A line that carries
`protect-scan: allow` is skipped; none is used. The scan only reads. Not
scanned: the selftests, which hold the forbidden names as test data.
"""
import glob
import os
import re
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

PROT_VARS = r"(?:bootcmd(?:_\w+)?|bootcount|bootlimit|upgrade_available|altbootcmd)"
PARTS = r"(?:fsbla[12]|metadata[12]|fip-[ab])"
WRITE_VERB = (r"(?:\bof=|\bdd\b|\bcp\b|\binstall\b|\bmv\b|\brm\b|\bmkfs\S*|\bmke2fs\b|\bwipefs\b|"
              r"\bsgdisk\b|\bsfdisk\b|\bparted\b|\be2label\b|\btune2fs\b|\bfw_setenv\b|"
              r"\bmmc\s+(?:write|erase|swrite)\b|\bext4write\b|\bfatwrite\b|\bsaveenv\b|\btee\b|>)")
SETTER = r"(?:\bsetenv\b|\benv\s+(?:set|delete|import|default)\b|\bfw_setenv\b)"

RULES = [
    ("var", re.compile(SETTER + r"\s+(?:-\S+\s+)*['\"]?" + PROT_VARS + r"(?=[\s'\"=;]|$)"),
     "writes a protected U-Boot variable"),
    ("var", re.compile(r"(?:^|[\s'\"]|\\n)" + PROT_VARS + r"="),
     "NAME=VALUE line for a protected U-Boot variable"),
    ("emmc", re.compile(r"/dev/mmcblk0(?:p\d+)?(?!\w)"),
     "names the eMMC device /dev/mmcblk0*"),
    ("emmc", re.compile(r"\bmmc\s+dev\s+0\b"), "selects mmc 0 (the eMMC)"),
    ("emmc", re.compile(r"\bmmc\s+(?:write|erase|swrite)\s+\S+\s+0\b"), "writes mmc 0 (the eMMC)"),
    ("emmc", re.compile(r"\b(?:ext4write|fatwrite|ext4save)\s+mmc\s+0[:\s]"), "writes on mmc 0 (the eMMC)"),
]
PART_RE = re.compile(r"(?<![\w-])" + PARTS + r"(?![\w-])")
WRITE_RE = re.compile(WRITE_VERB)
TEXT_CMD = re.compile(r"^\s*(?:echo|printf|note|die|warn)\b[^>|]*$")
DEF_RE = re.compile(r"^\s*(?:NEVER_LABELS|EMMC_RE|PART_RE)=")


def default_files():
    out = [os.path.join(ROOT, "tools/board-stm32mp2-standalone.sh")]
    me = os.path.abspath(__file__)
    for p in sorted(glob.glob(os.path.join(ROOT, "tools/mp2s_*"))):
        if p == me or "selftest" in os.path.basename(p) or not os.path.isfile(p):
            continue
        out.append(p)
    out += sorted(glob.glob(os.path.join(ROOT, "driver/tools/mp2-standalone/*.cmd")))
    out.append(os.path.join(ROOT, "driver/tools/mp2-standalone/build.sh"))
    return [p for p in out if os.path.exists(p)]


def scan_text(text, py=False):
    hits = []
    for n, line in enumerate(text.splitlines(), 1):
        s = line.strip()
        if not s or s.startswith("#") or "protect-scan: allow" in s:
            continue
        if DEF_RE.match(s) or TEXT_CMD.match(s):
            continue
        for tag, rx, why in RULES:
            # a Python tool's NAME=VALUE strings are fixtures it reads, not writes
            if py and why.startswith("NAME=VALUE"):
                continue
            if rx.search(line):
                hits.append((n, tag, why, s))
        if PART_RE.search(line) and WRITE_RE.search(line):
            hits.append((n, "part", "write verb on a line naming a protected partition", s))
    return hits


def scan(paths):
    bad = 0
    for p in paths:
        try:
            with open(p, encoding="utf-8", errors="replace") as f:
                text = f.read()
        except OSError as e:
            print("mp2s_protect_scan: %s: %s" % (p, e.strerror), file=sys.stderr)
            return 2
        for n, tag, why, s in scan_text(text, p.endswith(".py")):
            bad += 1
            print("PROTECT: %s:%d: [%s] %s: %s" % (p, n, tag, why, s[:160]), file=sys.stderr)
    if bad:
        return 3
    print("ok: %d file(s) scanned, no write of a protected variable, partition or the eMMC" % len(paths))
    return 0


CLEAN = [
    "# setenv bootcmd x is a comment",
    'echo "never: setenv bootcmd, saveenv to mmcblk0"',
    'die "REFUSED: /dev/mmcblk0 is the eMMC"',
    'NEVER_LABELS="fsbla1 fsbla2 metadata1 metadata2 fip-a fip-b"',
    "EMMC_RE='^/dev/mmcblk0(p[0-9]+)?$'",
    "setenv mp2s_left 0",
    'setenv boot_prefixes "${mp2s_prefixes}"',
    "fw_setenv -s - <<< 'mp2s_left=2'",
    "cp img /boot/yakogl/standalone/default.bin",
    "dd if=/dev/mmcblk1p8 of=env.bin",
    "mmc dev 1",
    "setenv mp2s_note bootcmd_is_not_touched",
    "blkid /dev/mmcblk1p3 | grep fip-a",
]
DIRTY = [
    ("var", "setenv bootcmd 'run x'"),
    ("var", "\tsetenv altbootcmd reset"),
    ("var", "setenv bootcount 0"),
    ("var", "env set bootcmd x"),
    ("var", "fw_setenv bootcmd x"),
    ("var", "setenv mp2s_a 1; setenv bootcmd_stm32mp x"),
    ("var", "printf 'mp2s_left=1\\nbootcount=0\\n' | fw_setenv -s -"),
    ("var", "bootcmd=reset"),
    ("emmc", "dd if=img of=/dev/mmcblk0p4"),
    ("emmc", "mount /dev/mmcblk0p5 /mnt"),
    ("emmc", "mmc dev 0"),
    ("emmc", "mmc write ${loadaddr} 0 100"),
    ("emmc", "ext4write mmc 0:4 ${a} /f 10"),
    ("part", "dd if=x of=/dev/disk/by-partlabel/fip-a"),
    ("part", "sgdisk -d fsbla1 /dev/x"),
    ("part", "echo x > /dev/disk/by-partlabel/metadata2"),
    ("part", "install img /dev/disk/by-partlabel/fip-b"),
]


def self_test():
    n = bad = 0

    def check(label, ok):
        nonlocal n, bad
        n += 1
        if not ok:
            bad += 1
        print("%s %s" % ("ok  " if ok else "FAIL", label))

    for line in CLEAN:
        check("clean: %s" % line, not scan_text(line))
    for tag, line in DIRTY:
        check("%s: %s" % (tag, line.strip()), any(h[1] == tag for h in scan_text(line)))
    check("allow marker skips", not scan_text("setenv bootcmd x # protect-scan: allow"))
    check("hit is reported at line 3",
          [h[0] for h in scan_text("a\nsetenv mp2s_left 1\nsetenv bootcount 1\n")] == [3])
    with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as d:
        ok_f = os.path.join(d, "ok.sh")
        bad_f = os.path.join(d, "bad.sh")
        with open(ok_f, "w") as f:
            f.write("\n".join(CLEAN) + "\n")
        with open(bad_f, "w") as f:
            f.write("setenv bootcmd x\n")
        so, se = sys.stdout, sys.stderr
        try:
            sys.stdout = sys.stderr = open(os.devnull, "w")
            rc = (scan([ok_f]), scan([bad_f]), scan([os.path.join(d, "missing")]))
        finally:
            sys.stdout, sys.stderr = so, se
        check("exit codes 0/3/2", rc == (0, 3, 2))
    real = default_files()
    check("default sources include the runner and the script",
          any(p.endswith("board-stm32mp2-standalone.sh") for p in real)
          and any(p.endswith("mp2-standalone.cmd") for p in real))
    check("the real sources are clean", scan(real) == 0)
    print("%d checks, %d failed" % (n, bad))
    return 1 if bad else 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if any(a.startswith("-") for a in argv):
        print("usage: tools/mp2s_protect_scan.py [FILE...] | --self-test", file=sys.stderr)
        return 2
    return scan(argv or default_files())


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
