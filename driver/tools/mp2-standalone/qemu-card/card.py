#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Console side of run.sh (README.md in this directory): boots the output of
tools/mp2s_product_card.py, the 'forever' product card, in U-Boot v2023.10
(../qemu/build-uboot.sh redund: the environment in two copies on the card's
u-boot-env partition, as on the DK) under QEMU.

    card.py --qemu QEMU --uboot U-BOOT.BIN (redund) --image STANDIN.BIN
            --script BOOT.SCR.UIMG --fixture ST-ENV.TXT --card-tool mp2s_product_card.py
            --uboot-script tools/uboot_script.py --work DIR --logs DIR [--boots 3]

1. The input environment: ST's distro-loop variables from the st-env fixture
   byte for byte, the board's saved boot_prefixes=/mmc0_, the QEMU adjustments
   of ../qemu-product (preboot, boot_device, boot_instance, boot_targets,
   scan_dev_for_efi, bootcount) and the addresses of ../qemu/redund-default.env.
2. The card: mp2s_product_card.py --env-text IN --env-size 0x2000 --script
   --image --out card, and once more with --no-linux; the two outputs must be
   byte-identical but for manifest.txt's note (the flag changes no card byte).
3. Per case, a GPT disk (../qemu/mkdisk.sh DISK BOOTFS env): bootfs is the
   card's yakogl/standalone/ tree, the card's u-boot-env-copy1.bin and
   -copy2.bin are written at ../qemu's copy 1 and copy 2 offsets
   (drive.ENV_COPIES). "forever" adds /mmc0_boot.scr.uimg, a stand-in for ST's
   mmc0_extlinux/ (Linux); "forever_no_linux" is the card after the person
   removed it (what --no-linux's note asks for).
4. N untouched QEMU boots on the same disk. Each must reach the product image
   under 'forever' and leave the disk, both environment copies included, byte
   for byte as the card was written.

Prints "PASS|FAIL case: assertion" per assertion, "case NAME: PASS|FAIL (n ok,
m failed)" per case and "card.py: N ok, M failed" last; exit 0 only when
nothing failed.
"""
import argparse
import hashlib
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
QEMU_DIR = os.path.join(HERE, "..", "qemu")
sys.dont_write_bytecode = True      # no __pycache__ in ../qemu, which is not ours
sys.path.insert(0, QEMU_DIR)
import drive  # noqa: E402  ../qemu/drive.py: Qemu, Boot, seq, ENV_COPIES, ENV_SIZE, UENV

# The product script's lines (../mp2-standalone.cmd; section 7 of
# docs/notes/stm32mp257f-dk-standalone.md) and the stand-ins'.
FOUND_OURS = "Found U-Boot script /yakogl/standalone/boot.scr.uimg"
UNDO = ("mp2s: undo: setenv mp2s_left 0; saveenv; boot   "
        "(or a PC: delete yakogl/standalone/boot.scr.uimg)")
FOREVER = "mp2s: default /yakogl/standalone/default.bin, forever"
LOADED = ("mp2s: loaded 1000 bytes at 0xa0000000, RAM footprint 0x2000 of the "
          "0x10000000-byte window")
GO = "mp2s: go 0xa0000000"
IMAGE = "mp2q: image ran at its link address 0xa0000000"     # ../qemu/image.S
RETURNED = "mp2s: the image returned, booting Linux"
FAILED = "SCRIPT FAILED: continuing..."
STOCK_FILE = "mmc0_boot.scr.uimg"
FOUND_STOCK = "Found U-Boot script /" + STOCK_FILE
STOCK = "mp2q: stock path /mmc0_ reached (on the DK: ST extlinux, OpenSTLinux)"
ENV_LOADED = "Loading Environment from MMC... OK"

PREFIXES = "/mmc0_"                          # the board's saved boot_prefixes
OURS = "/yakogl/standalone/ " + PREFIXES     # what the card saves

# ST's distro-loop variables, from the fixture byte for byte (as ../qemu-product).
ST_VARS = ("bootcmd", "altbootcmd", "bootcmd_stm32mp", "env_check", "distro_bootcmd",
           "bootcmd_mmc0", "mmc_boot", "scan_dev_for_boot_part", "scan_dev_for_boot",
           "scan_dev_for_extlinux", "scan_dev_for_scripts", "boot_a_script", "boot_extlinux",
           "boot_scripts", "boot_syslinux_conf", "scriptaddr")
# QEMU's differences (../qemu-product/README.md): its SD host is on PCI,
# boot_device/boot_instance come from the boot ROM on the DK, the EFI step
# would try bootefi bootmgr, and bootcount lives outside the env on the DK (a
# value here makes the "unchanged" check compare bytes, not two absences).
QEMU_VARS = (("preboot", "pci enum; mmc list"), ("boot_device", "mmc"), ("boot_instance", "0"),
             ("boot_targets", "mmc0"), ("scan_dev_for_efi", "echo mp2q: efi step skipped"),
             ("bootcount", "0"))
# The redund default env's lines that the fixture and QEMU_VARS do not replace
# are its addresses; these three are ST's or the card's.
REDUND_SKIP = ("bootcmd", "env_check", "boot_targets")
# The card's own edits (tools/mp2s_product_card.py).
CARD_EDITS = {"mp2s_ver": "1", "mp2s_left": "forever", "mp2s_prefixes": PREFIXES,
              "boot_prefixes": OURS}
CARD_FILES = ("u-boot-env-copy1.bin", "u-boot-env-copy2.bin",
              "yakogl/standalone/boot.scr.uimg", "yakogl/standalone/default.bin")


def read_kv(path):
    """name=value lines, comments and blanks skipped; the first value wins."""
    out = {}
    with open(path) as f:
        for ln in f:
            ln = ln.rstrip("\n")
            if "=" in ln and not ln.startswith("#"):
                k, v = ln.split("=", 1)
                out.setdefault(k, v)
    return out


def input_env(fixture):
    env = {k: v for k, v in read_kv(os.path.join(QEMU_DIR, "redund-default.env")).items()
           if k not in REDUND_SKIP}
    for k in ST_VARS:
        env[k] = fixture[k]
    env.update(QEMU_VARS)
    env["boot_prefixes"] = PREFIXES
    return env


def env_of(raw):
    """{name: value} of one redundant env copy (crc32, flags, data), or None
    when its CRC is bad."""
    c = drive.UENV.Copy("copy", "card", 0, raw, True)
    if not c.crc_ok:
        return None
    return drive.env_vars(c)


def sha(b):
    return hashlib.sha256(b).hexdigest()


def read(path):
    with open(path, "rb") as f:
        return f.read()


class Results:
    def __init__(self):
        self.total = {"ok": 0, "failed": 0}
        self.verdicts = []

    def case(self, name):
        res = {"ok": 0, "failed": 0}

        def check(what, cond, detail=""):
            res["ok" if cond else "failed"] += 1
            self.total["ok" if cond else "failed"] += 1
            print("%s %s: %s%s" % ("PASS" if cond else "FAIL", name, what,
                                   "" if cond or not detail else ": " + detail), flush=True)
            return cond

        def done():
            self.verdicts.append("case %s: %s (%d ok, %d failed)" % (
                name, "PASS" if res["failed"] == 0 else "FAIL", res["ok"], res["failed"]))
        return check, done


def build_card(a, check, env_txt):
    """Runs the card tool twice (without and with --no-linux); returns the card
    directory or None."""
    outs = {}
    for label, extra in (("card", []), ("card-no-linux", ["--no-linux"])):
        out = os.path.join(a.work, label)
        p = subprocess.run([sys.executable, "-I", a.card_tool, "--env-text", env_txt,
                            "--env-size", hex(drive.ENV_SIZE), "--script", a.script,
                            "--image", a.image, "--out", out] + extra,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        if not check("mp2s_product_card.py %s exits 0" % " ".join(extra or ["(no flags)"]),
                     p.returncode == 0, "rc %d: %s" % (p.returncode, p.stdout.strip())):
            return None
        outs[label] = out
    same = [f for f in CARD_FILES
            if read(os.path.join(outs["card"], f)) == read(os.path.join(outs["card-no-linux"], f))]
    check("--no-linux changes no card file (%s byte-identical)" % ", ".join(CARD_FILES),
          len(same) == len(CARD_FILES), "identical: %r" % same)
    m1 = read(os.path.join(outs["card"], "manifest.txt")).decode().splitlines()
    m2 = read(os.path.join(outs["card-no-linux"], "manifest.txt")).decode().splitlines()
    check("--no-linux only adds the manifest's NOTE line",
          m2[:len(m1)] == m1 and len(m2) == len(m1) + 1 and m2[-1].startswith("NOTE: --no-linux"),
          "%r" % m2[len(m1):])
    return outs["card"]


def check_card_env(check, card, env_in):
    c1 = read(os.path.join(card, "u-boot-env-copy1.bin"))
    c2 = read(os.path.join(card, "u-boot-env-copy2.bin"))
    check("env copies are %#x bytes each (the redund U-Boot's ENV_SIZE)" % drive.ENV_SIZE,
          len(c1) == len(c2) == drive.ENV_SIZE, "%d, %d" % (len(c1), len(c2)))
    e1, e2 = env_of(c1), env_of(c2)
    if not check("both env copies have a good CRC", e1 is not None and e2 is not None):
        return False
    check("copy 1 is the active one (flags 1 over 0)", (c1[4], c2[4]) == (1, 0),
          "flags %d, %d" % (c1[4], c2[4]))
    check("both copies hold the same variables", e1 == e2)
    want = dict(env_in)
    want.update(CARD_EDITS)
    diff = sorted(k for k in set(e1) | set(want) if e1.get(k) != want.get(k))
    return check("the card env is the input with only mp2s_ver, mp2s_left=forever, "
                 "mp2s_prefixes, boot_prefixes changed", not diff,
                 "differs: %r" % [(k, want.get(k), e1.get(k)) for k in diff])


def run_case(a, res, name, card, stock_bytes, fixture):
    check, done = res.case(name)
    w = os.path.join(a.work, name)
    stage = os.path.join(w, "bootfs")
    shutil.copytree(os.path.join(card, "yakogl"), os.path.join(stage, "yakogl"))
    if stock_bytes is not None:
        with open(os.path.join(stage, STOCK_FILE), "wb") as f:
            f.write(stock_bytes)
    disk, flash = os.path.join(w, "sd.img"), os.path.join(w, "flash.img")
    subprocess.run(["bash", os.path.join(QEMU_DIR, "mkdisk.sh"), disk, stage, "env"], check=True)
    copies = {"copy 1": read(os.path.join(card, "u-boot-env-copy1.bin")),
              "copy 2": read(os.path.join(card, "u-boot-env-copy2.bin"))}
    with open(disk, "r+b") as f:
        for label, off in drive.ENV_COPIES:
            f.seek(off)
            f.write(copies[label])
    with open(flash, "wb") as f:            # ../qemu's argv; the redund U-Boot does not use it
        f.truncate(64 << 20)
    base = sha(read(disk))
    print("     %s: card written at %s; disk sha256 %s" % (
        name, ", ".join("%s %#x" % c for c in drive.ENV_COPIES), base), flush=True)
    argv = [a.qemu, "-machine", "virt", "-cpu", "cortex-a57", "-m", "2G",
            "-display", "none", "-serial", "stdio", "-monitor", "none", "-no-reboot",
            "-bios", a.uboot,
            "-drive", "if=pflash,unit=1,format=raw,file=" + flash,
            "-device", "sdhci-pci",
            "-drive", "if=none,id=sd0,format=raw,file=" + disk,
            "-device", "sd-card,drive=sd0"]
    reached = 0
    for n in range(1, a.boots + 1):
        q = drive.Qemu(argv, os.path.join(a.logs, "%s-boot%d.log" % (name, n)))
        try:
            q.wait_prompt(180)
            lines = q.lines()
            env = drive.printenv(q)
        except (TimeoutError, EOFError) as e:
            check("boot %d: console" % n, False, "%s (log %s)" % (e, q.log.name))
            break
        finally:
            q.close()
        print("     %s: boot %d: %s" % (name, n, " / ".join(
            ln for ln in lines if ln.startswith(("mp2s:", "mp2q:")))), flush=True)
        check("boot %d: the card's environment loaded (%r, no bad CRC)" % (n, ENV_LOADED),
              any(ln.startswith(ENV_LOADED) for ln in lines)
              and not any("bad CRC" in ln for ln in lines))
        pats = [FOUND_OURS, UNDO, FOREVER, LOADED, GO, IMAGE, RETURNED]
        if stock_bytes is not None:     # the stand-in returns from go; a Zephyr image does not
            pats += [FAILED, FOUND_STOCK, STOCK]
        got = drive.seq(lines, pats)
        ok = check("boot %d: forever reaches the product image: %s" % (n, " | ".join(pats)),
                   None not in got, "console lines at %r" % got)
        reached += ok
        for bad in ("boots left", "Saving Environment", "used up", "refused"):
            check("boot %d: no line with %r" % (n, bad), not any(bad in ln for ln in lines),
                  "%r" % [ln for ln in lines if bad in ln][:3])
        if stock_bytes is None:
            check("boot %d: no stock path on a card without it" % n,
                  not any(FOUND_STOCK in ln or STOCK in ln for ln in lines))
        check("boot %d: printenv mp2s_left=forever, boot_prefixes=%r, ST's bootcmd" % (n, OURS),
              env.get("mp2s_left") == "forever" and env.get("boot_prefixes") == OURS
              and env.get("bootcmd") == fixture["bootcmd"],
              "%r" % {k: env.get(k) for k in ("mp2s_left", "boot_prefixes", "bootcmd")})
        raw = read(disk)
        for label, off in drive.ENV_COPIES:
            check("boot %d: env %s on the disk byte-identical to the card's" % (n, label),
                  raw[off:off + drive.ENV_SIZE] == copies[label])
        check("boot %d: the whole disk unchanged (sha256)" % n, sha(raw) == base, sha(raw))
    check("all %d boots reached the product image under forever" % a.boots, reached == a.boots,
          "%d of %d" % (reached, a.boots))
    done()


def main():
    ap = argparse.ArgumentParser()
    for x in ("qemu", "uboot", "image", "script", "fixture", "card-tool", "uboot-script",
              "work", "logs"):
        ap.add_argument("--" + x, required=True)
    ap.add_argument("--boots", type=int, default=3)
    a = ap.parse_args()
    us = drive.load(a.uboot_script, "uboot_script")
    fixture = read_kv(a.fixture)
    os.makedirs(a.work, exist_ok=True)
    res = Results()

    check, done = res.case("card")
    env_in = input_env(fixture)
    env_txt = os.path.join(a.work, "input-env.txt")
    with open(env_txt, "w") as f:
        f.write("".join("%s=%s\n" % kv for kv in env_in.items()))
    card = build_card(a, check, env_txt)
    card_ok = card is not None and check_card_env(check, card, env_in)
    done()
    if card_ok:
        stock = us.build(("echo " + STOCK + "\n").encode(), name="stock-standin",
                         arch="arm64", time=0)
        run_case(a, res, "forever", card, stock, fixture)
        run_case(a, res, "forever_no_linux", card, None, fixture)
    print()
    for v in res.verdicts:
        print(v)
    print("card.py: %d ok, %d failed" % (res.total["ok"], res.total["failed"]))
    return 0 if res.total["failed"] == 0 and card_ok else 1


if __name__ == "__main__":
    sys.exit(main())
