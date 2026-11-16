#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Console side of run.sh (README.md in this directory): the product script
mp2-standalone.cmd in U-Boot v2023.10 under QEMU, through ST's distro loop.

    product.py --qemu QEMU --uboot U-BOOT.BIN --image STANDIN.BIN
               --script BOOT.SCR.UIMG --fixture ST-ENV.TXT --work DIR --logs DIR
               --uboot-script tools/uboot_script.py [--only NAME]

It reuses ../qemu/drive.py (the console driver, Boot, MMC_ONLY, printenv, seq)
and ../qemu/mkdisk.sh, and has its own case loop: drive.py's main() asserts
that boot_prefixes never changes, and the product restores it when the count
is used up. Per case:

  bootfs   /yakogl/standalone/boot.scr.uimg (build.sh's image of the product
           script), /yakogl/standalone/default.bin (the stand-in image,
           ../qemu/image.S), and /mmc0_boot.scr.uimg, a stand-in for ST's
           /mmc0_extlinux that only prints STOCK;
  setup    one boot that stops autoboot, types ST's distro-loop variables from
           the st-env fixture (bootcmd=run bootcmd_stm32mp, byte for byte) and
           the case's presets, saveenv;
  boot n   untouched boots: ST's bootcmd_stm32mp runs the loop, which sources
           our script; a "cut" boot is killed (SIGKILL, a power cut) on the
           console line that follows the counting saveenv.

After every QEMU run the saved environment is read from the flash image
itself (not printenv), so "unchanged" means the bytes U-Boot stored.
Prints "PASS|FAIL case: assertion" per assertion, one "case NAME: PASS|FAIL"
verdict per case, and "product.py: N ok, M failed" last.
"""
import argparse
import os
import re
import subprocess
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
QEMU_DIR = os.path.join(HERE, "..", "qemu")
sys.dont_write_bytecode = True      # no __pycache__ in ../qemu, which is not ours
sys.path.insert(0, QEMU_DIR)
import drive  # noqa: E402  ../qemu/drive.py

# The product script's lines (mp2-standalone.cmd; section 7 of
# docs/notes/stm32mp257f-dk-standalone.md).
UNDO = "mp2s: undo: setenv mp2s_left 0; saveenv; boot   (or a PC: delete yakogl/standalone/boot.scr.uimg)"
LEFT = "mp2s: default /yakogl/standalone/default.bin, boots left %d"
LEFT_RE = re.compile(rb"mp2s: default /yakogl/standalone/default\.bin, boots left \d+")
NODEF = "mp2s: no default set, back to the distro loop"
USEDUP = "mp2s: default used up, booting Linux"
LOADED = "mp2s: loaded 1000 bytes at 0xa0000000, RAM footprint 0x2000 of the 0x10000000-byte window"
GO = "mp2s: go 0xa0000000"
RETURNED = "mp2s: the image returned, booting Linux"
# The stand-in image's line (../qemu/image.S) and the stock stand-in's.
IMAGE = "mp2q: image ran at its link address 0xa0000000"
STOCK = "mp2q: stock path /mmc0_ reached (on the DK: ST extlinux, OpenSTLinux)"
STOCK_CMD = "echo " + STOCK + "\n"
FOUND_OURS = "Found U-Boot script /yakogl/standalone/boot.scr.uimg"
FOUND_STOCK = "Found U-Boot script /mmc0_boot.scr.uimg"
FAILED = "SCRIPT FAILED: continuing..."

PREFIXES = "/mmc0_"                         # the board's saved boot_prefixes (stock script)
OURS = "/yakogl/standalone/ " + PREFIXES    # what "default" saves

# ST's distro-loop variables, copied from the fixture byte for byte.
ST_VARS = ("bootcmd", "altbootcmd", "bootcmd_stm32mp", "env_check", "distro_bootcmd",
           "bootcmd_mmc0", "mmc_boot", "scan_dev_for_boot_part", "scan_dev_for_boot",
           "scan_dev_for_extlinux", "scan_dev_for_scripts", "boot_a_script", "boot_extlinux",
           "boot_scripts", "boot_syslinux_conf", "scriptaddr")
# QEMU has no stm32prog and its EFI loader would try bootefi bootmgr; the
# loop's EFI step is replaced by a marker. Its MMC sits on PCI and is probed
# by "mmc list", not by ST's "mmc dev": preboot does it (README.md, differences).
QEMU_VARS = ["setenv preboot 'pci enum; mmc list'",
             "setenv boot_device mmc", "setenv boot_instance 0", drive.MMC_ONLY,
             "setenv scan_dev_for_efi 'echo mp2q: efi step skipped'",
             # ST's bootcount lives outside the env on the DK; a value here makes
             # check (c) compare a real byte string, not two absences.
             "setenv bootcount 0"]
# Check (c): byte-identical in the saved environment before and after.
PROTECTED = ("bootcmd", "bootcount", "altbootcmd", "bootcmd_stm32mp", "distro_bootcmd",
             "bootlimit", "upgrade_available")
MP2S = ("mp2s_left", "mp2s_ver", "mp2s_prefixes")

ENV_SIZE = 0x40000    # ../qemu/mp2-standalone.config's ENV_SIZE (qemu_arm64)


def read_saved_env(path):
    """The environment U-Boot stored in the flash image: {name: value}, or None.
    Plain (crc32, data) at 0; a redundant pair (crc32, flags, data) at 0 and
    ENV_SIZE is read too, the valid copy with the higher flags winning."""
    with open(path, "rb") as f:
        raw = f.read(2 * ENV_SIZE)
    found = []
    for off, hdr in ((0, 4), (0, 5), (ENV_SIZE, 5)):
        blk = raw[off:off + ENV_SIZE]
        if len(blk) < ENV_SIZE:
            continue
        crc = int.from_bytes(blk[:4], "little")
        data = blk[hdr:]
        if zlib.crc32(data) & 0xffffffff == crc:
            found.append((blk[4] if hdr == 5 else 0, data))
    if not found:
        return None
    data = max(found)[1]
    env = {}
    for kv in data.split(b"\0\0", 1)[0].split(b"\0"):
        if b"=" in kv:
            k, v = kv.split(b"=", 1)
            env[k.decode()] = v.decode(errors="replace")
    return env


def read_fixture(path):
    env = {}
    with open(path) as f:
        for ln in f:
            ln = ln.rstrip("\n")
            if "=" in ln and not ln.startswith("#"):
                k, v = ln.split("=", 1)
                env.setdefault(k, v)
    return env


class Case:
    """boots: a list, one entry per boot: "full" (to the prompt) or "cut" (killed
    on the line after the counting saveenv)."""
    def __init__(self, name, proves, presets, boots, check):
        self.name, self.proves, self.presets = name, proves, presets
        self.boots, self.check = boots, check


def default_presets(left):
    return ["setenv boot_prefixes '%s'" % OURS, "setenv mp2s_prefixes '%s'" % PREFIXES,
            "setenv mp2s_ver 1", "setenv mp2s_left %s" % left]


def ordered(b, what, pats):
    got = drive.seq(b.lines, pats)
    return ("boot %d: %s: %s" % (b.n, what, " | ".join(pats)), None not in got,
            "console lines at %r" % got)


def absent(b, pat):
    got = [ln for ln in b.lines if pat in ln]
    return ("boot %d: no line with %r" % (b.n, pat), not got, "got %r" % got[:3])


def image_boot(b, left):
    """A counting boot: the count is saved before anything is loaded, then the
    image runs; the stand-in returns, so the loop goes on to the stock path."""
    return [ordered(b, "boots left %d, then the image" % left, [
                FOUND_OURS, UNDO, LEFT % left, LOADED, GO, IMAGE, RETURNED, FAILED,
                FOUND_STOCK, STOCK]),
            ("boot %d: saved mp2s_left is %d" % (b.n, left), b.saved.get("mp2s_left") == str(left),
             "is %r" % b.saved.get("mp2s_left")),
            ("boot %d: saved boot_prefixes still %r" % (b.n, OURS), b.saved.get("boot_prefixes") == OURS,
             "is %r" % b.saved.get("boot_prefixes"))]


def cut_boot(b, left):
    return [ordered(b, "cut after the counting saveenv", [FOUND_OURS, UNDO, LEFT % left]),
            absent(b, GO),
            ("boot %d: the cut kept mp2s_left %d in flash" % (b.n, left),
             b.saved.get("mp2s_left") == str(left), "is %r" % b.saved.get("mp2s_left"))]


def restored(b):
    """The used-up branch: boot_prefixes back to the recorded value, mp2s_* gone."""
    out = [("boot %d: saved boot_prefixes restored to %r" % (b.n, PREFIXES),
            b.saved.get("boot_prefixes") == PREFIXES, "is %r" % b.saved.get("boot_prefixes"))]
    for k in MP2S:
        out.append(("boot %d: %s cleared in the saved environment" % (b.n, k), k not in b.saved,
                    "is %r" % b.saved.get(k)))
    return out


def usedup_boot(b):
    return ([ordered(b, "used up, then the stock path", [FOUND_OURS, UNDO, USEDUP, FAILED,
                                                          FOUND_STOCK, STOCK]),
             absent(b, GO), absent(b, IMAGE)] + restored(b))


def stock_only(b):
    """Our prefix is gone from boot_prefixes: the script is not even found."""
    return [ordered(b, "stock path only", [FOUND_STOCK, STOCK]), absent(b, "mp2s:"),
            absent(b, FOUND_OURS), absent(b, IMAGE)]


def first_stock(boots):
    return next((b.n for b in boots if STOCK in b.lines and IMAGE not in b.lines), None)


def check_a(setup, boots):
    b1, b2, b3, b4 = boots
    return (image_boot(b1, 1) + image_boot(b2, 0) + usedup_boot(b3) + stock_only(b4)
            + [("the record reads boots left 1, image, boots left 0, image, stock",
                first_stock(boots) == 3, "first stock-only boot %r" % first_stock(boots))])


def check_b_absent(setup, boots):
    b = boots[0]
    return [ordered(b, "no default: back to the loop, stock on the first boot",
                    [FOUND_OURS, UNDO, NODEF, FAILED, FOUND_STOCK, STOCK]),
            absent(b, GO), absent(b, IMAGE),
            ("boot 1: the script saved nothing (saved env as setup left it)", b.saved == setup,
             "keys changed %r" % sorted(k for k in set(b.saved) | set(setup)
                                        if b.saved.get(k) != setup.get(k)))]


def check_b_zero(setup, boots):
    return usedup_boot(boots[0])


def check_b_removed(setup, boots):
    b = boots[0]
    return stock_only(b) + [("boot 1: saved env as setup left it", b.saved == setup, "")]


def check_d_every(setup, boots):
    b1, b2, b3 = boots
    return (cut_boot(b1, 1) + cut_boot(b2, 0) + usedup_boot(b3)
            + [("stock path within N+1 = 3 boots", first_stock(boots) == 3,
                "first stock-only boot %r" % first_stock(boots))])


def check_d_first(setup, boots):
    b1, b2, b3 = boots
    return (cut_boot(b1, 1) + image_boot(b2, 0) + usedup_boot(b3)
            + [("stock path within N+1 = 3 boots", first_stock(boots) == 3,
                "first stock-only boot %r" % first_stock(boots))])


def cases():
    return [
        Case("a_default_2", "(a) default 2: boots left 1, image; boots left 0, image; then stock, "
             "mp2s_* cleared; boot 4 stays stock", default_presets(2),
             ["full"] * 4, check_a),
        Case("b_nodefault_absent", "(b) mp2s_left absent, our prefix still in boot_prefixes: "
             "stock on boot 1", ["setenv boot_prefixes '%s'" % OURS], ["full"], check_b_absent),
        Case("b_nodefault_zero", "(b) mp2s_left 0: used up, prefix restored, stock on boot 1",
             default_presets(0), ["full"], check_b_zero),
        Case("b_nodefault_removed", "(b) after nodefault (boot_prefixes back, no mp2s_*): "
             "stock on boot 1", ["setenv boot_prefixes '%s'" % PREFIXES], ["full"], check_b_removed),
        Case("d_cut_every_count", "(d) default 2, power cut after the counting saveenv on boots 1 "
             "and 2: stock on boot 3", default_presets(2), ["cut", "cut", "full"], check_d_every),
        Case("d_cut_first_count", "(d) default 2, power cut after the counting saveenv on boot 1: "
             "stock on boot 3", default_presets(2), ["cut", "full", "full"], check_d_first),
    ]


def main():
    ap = argparse.ArgumentParser()
    for a in ("qemu", "uboot", "image", "script", "fixture", "work", "logs", "uboot-script"):
        ap.add_argument("--" + a, required=True)
    ap.add_argument("--only", help="run only the case of this name")
    a = ap.parse_args()
    us = drive.load(a.uboot_script, "uboot_script")
    fixture = read_fixture(a.fixture)
    st = []
    for k in ST_VARS:
        v = fixture[k]
        if "'" in v:
            sys.exit("product.py: fixture %s holds a single quote; cannot type it" % k)
        st.append("setenv %s '%s'" % (k, v))
    total = {"ok": 0, "failed": 0}
    verdicts = []

    for c in cases():
        if a.only and a.only != c.name:
            continue
        res = {"ok": 0, "failed": 0}

        def check(name, cond, detail="", res=res, c=c):
            res["ok" if cond else "failed"] += 1
            print("%s %s: %s%s" % ("PASS" if cond else "FAIL", c.name, name,
                                   "" if cond or not detail else ": " + detail), flush=True)

        print("== %s: %s" % (c.name, c.proves), flush=True)
        w = os.path.join(a.work, c.name)
        stage = os.path.join(w, "bootfs")
        os.makedirs(os.path.join(stage, "yakogl", "standalone"), exist_ok=True)
        with open(a.script, "rb") as f, open(os.path.join(stage, drive.YAKOGL), "wb") as g:
            g.write(f.read())
        with open(a.image, "rb") as f, \
                open(os.path.join(stage, "yakogl", "standalone", "default.bin"), "wb") as g:
            g.write(f.read())
        with open(os.path.join(stage, drive.STOCK), "wb") as g:
            g.write(us.build(STOCK_CMD.encode(), name="stock-standin", arch="arm64", time=0))
        disk, flash = os.path.join(w, "sd.img"), os.path.join(w, "env.img")
        subprocess.run(["bash", os.path.join(QEMU_DIR, "mkdisk.sh"), disk, stage], check=True)
        with open(flash, "wb") as f:
            f.truncate(64 << 20)
        argv = [a.qemu, "-machine", "virt", "-cpu", "cortex-a57", "-m", "2G",
                "-display", "none", "-serial", "stdio", "-monitor", "none", "-no-reboot",
                "-bios", a.uboot,
                "-drive", "if=pflash,unit=1,format=raw,file=" + flash,
                "-device", "sdhci-pci",
                "-drive", "if=none,id=sd0,format=raw,file=" + disk,
                "-device", "sd-card,drive=sd0"]

        def session(label, body):
            q = drive.Qemu(argv, os.path.join(a.logs, "%s-%s.log" % (c.name, label)))
            try:
                return body(q)
            except (TimeoutError, EOFError) as e:
                check("%s: console" % label, False, "%s (log %s)" % (e, q.log.name))
                return None
            finally:
                q.close()

        def setup(q):
            q.stop_autoboot()
            for p in st + QEMU_VARS + c.presets:
                q.cmd(p)
            q.cmd("saveenv")
            return True

        ok = session("setup", setup)
        setup_env = read_saved_env(flash) if ok else None
        if setup_env is not None:
            check("setup: ST's bootcmd from the fixture saved byte for byte",
                  setup_env.get("bootcmd") == fixture["bootcmd"], repr(setup_env.get("bootcmd")))
        else:
            check("setup: a saved environment in flash", False)
        boots = []
        for n, kind in enumerate(c.boots if setup_env is not None else [], 1):
            def body(q, n=n, kind=kind):
                if kind == "cut":
                    q.expect(LEFT_RE, 120)
                    q.cut()
                else:
                    q.wait_prompt(180)
                return drive.Boot(n, q.lines(), None)
            b = session("boot%d" % n, body)
            if b is None:
                break
            b.saved = read_saved_env(flash) or {}
            boots.append(b)
            print("     %s: boot %d (%s): %s" % (c.name, n, kind, " / ".join(
                ln for ln in b.lines if ln.startswith(("mp2s:", "mp2q:")))), flush=True)
            check("boot %d: saved environment valid (CRC)" % n, bool(b.saved))
            check("boot %d: stored environment read (no bad CRC)" % n,
                  not any("bad CRC" in ln for ln in b.lines))
            for k in PROTECTED:
                check("boot %d: (c) %s byte-identical in the saved env" % (n, k),
                      b.saved.get(k) == setup_env.get(k),
                      "%r -> %r" % (setup_env.get(k), b.saved.get(k)))
            extra = sorted(k for k in set(b.saved) | set(setup_env)
                           if k not in MP2S + ("boot_prefixes",)
                           and b.saved.get(k) != setup_env.get(k))
            if extra:
                print("     %s: boot %d: info: other variables the saves persisted: %s"
                      % (c.name, n, ", ".join(extra)), flush=True)
        if len(boots) == len(c.boots):
            for name, cond, detail in c.check(setup_env, boots):
                check(name, cond, detail)
        else:
            check("all %d boots ran" % len(c.boots), False, "%d ran" % len(boots))
        verdict = "PASS" if res["failed"] == 0 else "FAIL"
        verdicts.append("case %s: %s (%d ok, %d failed)" % (c.name, verdict, res["ok"], res["failed"]))
        total["ok"] += res["ok"]
        total["failed"] += res["failed"]
    print()
    for v in verdicts:
        print(v)
    print("product.py: %d ok, %d failed" % (total["ok"], total["failed"]))
    return 0 if total["failed"] == 0 and verdicts else 1


if __name__ == "__main__":
    sys.exit(main())
