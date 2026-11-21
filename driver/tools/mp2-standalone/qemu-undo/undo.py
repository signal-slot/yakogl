#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Console side of run.sh (README.md in this directory).

Imports ../qemu-product/product.py (its constants and console lines) and,
through it, ../qemu/drive.py; neither is changed. Prints the console lines of
both boots, the undo's own output, and one verdict line:
"qemu-undo: ok" or "qemu-undo: FAIL <first failed check>".
"""
import argparse
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.join(HERE, "..", "qemu"))
import drive  # noqa: E402

P = drive.load(os.path.join(HERE, "..", "qemu-product", "product.py"), "product")

failed = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, "" if cond or not detail else ": " + detail),
          flush=True)
    if not cond:
        failed.append(name)
    return cond


def partition(disk, name):
    out = subprocess.run(["sfdisk", "-J", disk], check=True, capture_output=True, text=True).stdout
    for p in json.loads(out)["partitiontable"]["partitions"]:
        if p.get("name") == name:
            return p["start"], p["size"]
    raise SystemExit("qemu-undo: FAIL no partition %r in %s" % (name, disk))


def undo_on_disk(a, disk, work):
    """The card in a PC: the bootfs partition is cut out of the disk image
    (the tool refuses a whole disk), undone with --yes, and put back."""
    start, size = partition(disk, "bootfs")
    part = os.path.join(work, "bootfs.img")
    subprocess.run(["dd", "if=" + disk, "of=" + part, "bs=512", "skip=%d" % start,
                    "count=%d" % size, "status=none"], check=True)
    r = subprocess.run(["bash", a.undo, "--yes", part], capture_output=True, text=True)
    print("---- mp2s_card_undo.sh --yes (bootfs partition of the disk image) ----")
    sys.stdout.write(r.stdout)
    sys.stdout.write(r.stderr)
    print("----")
    check("undo: exit 0", r.returncode == 0, "exit %d" % r.returncode)
    if r.returncode == 0:
        subprocess.run(["dd", "if=" + part, "of=" + disk, "bs=512", "seek=%d" % start,
                        "conv=notrunc", "status=none"], check=True)
    return r.stdout


def main():
    ap = argparse.ArgumentParser()
    for k in ("qemu", "uboot", "image", "script", "fixture", "undo", "work", "logs", "uboot-script"):
        ap.add_argument("--" + k, required=True)
    a = ap.parse_args()
    us = drive.load(a.uboot_script, "uboot_script")
    fixture = P.read_fixture(a.fixture)
    st = ["setenv %s '%s'" % (k, fixture[k]) for k in P.ST_VARS]

    w = a.work
    stage = os.path.join(w, "bootfs")
    os.makedirs(os.path.join(stage, "yakogl", "standalone"), exist_ok=True)
    os.makedirs(os.path.join(stage, "mmc0_extlinux"), exist_ok=True)
    with open(a.script, "rb") as f, open(os.path.join(stage, drive.YAKOGL), "wb") as g:
        g.write(f.read())
    with open(a.image, "rb") as f, \
            open(os.path.join(stage, "yakogl", "standalone", "default.bin"), "wb") as g:
        g.write(f.read())
    stock = us.build(P.STOCK_CMD.encode(), name="stock-standin", arch="arm64", time=0)
    # ST's /mmc0_boot.scr.uimg stand-in (the loop's stock script) and the two
    # things mp2s_card_undo.sh insists on: a root boot.scr.uimg, mmc0_extlinux/.
    for p in (drive.STOCK, "boot.scr.uimg"):
        with open(os.path.join(stage, p), "wb") as g:
            g.write(stock)
    with open(os.path.join(stage, "mmc0_extlinux", "README"), "w") as g:
        g.write("stand-in for ST's extlinux directory (no .conf: the stock step is the script)\n")
    disk, flash = os.path.join(w, "sd.img"), os.path.join(w, "env.img")
    subprocess.run(["bash", os.path.join(HERE, "..", "qemu", "mkdisk.sh"), disk, stage, "env"], check=True)
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
        q = drive.Qemu(argv, os.path.join(a.logs, "%s.log" % label))
        try:
            return body(q)
        except (TimeoutError, EOFError) as e:
            check("%s: console" % label, False, "%s (log %s)" % (e, q.log.name))
            return None
        finally:
            q.close()

    def setup(q):
        q.stop_autoboot()
        for p in st + P.QEMU_VARS + P.default_presets(2):
            q.cmd(p)
        q.cmd("saveenv")
        return True

    def boot(q):
        q.wait_prompt(180)
        return q.lines()

    def show(label, lines):
        print("console %s: %s" % (label, " / ".join(
            ln for ln in lines if ln.startswith(("mp2s:", "mp2q:")) or "Found U-Boot script" in ln
            or "SCRIPT FAILED" in ln)), flush=True)

    if not session("setup", setup):
        print("qemu-undo: FAIL setup boot")
        return 1
    l1 = session("boot1", boot)
    if l1 is None:
        print("qemu-undo: FAIL boot 1")
        return 1
    show("boot 1", l1)
    check("boot 1: boots left 1, then the image, then stock",
          None not in drive.seq(l1, [P.FOUND_OURS, P.LEFT % 1, P.LOADED, P.GO, P.IMAGE, P.RETURNED,
                                     P.FOUND_STOCK, P.STOCK]))

    out = undo_on_disk(a, disk, w)
    # What the design note's section 5 says the tool prints and does.
    check("undo: names the renamed path as the only path written",
          "ours:     /yakogl/standalone/boot.scr.uimg (the only path this tool writes)" in out)
    check("undo: directory read back has boot.scr.uimg.off and no boot.scr.uimg",
          re.search(r"^  \d+\tboot\.scr\.uimg\.off$", out, re.M) is not None
          and re.search(r"^  \d+\tboot\.scr\.uimg$", out, re.M) is None)
    check("undo: stock files sha256 unchanged", "stock files: sha256 unchanged" in out)
    check("undo: filesystem state clean", "filesystem state: clean" in out)
    check("undo: says what to do next (nodefault from Linux)",
          "tools/board-stm32mp2-standalone.sh nodefault" in out)

    l2 = session("boot2", boot)
    if l2 is None:
        print("qemu-undo: FAIL boot 2")
        return 1
    show("boot 2", l2)
    check("boot 2: no mp2s line", not [ln for ln in l2 if "mp2s:" in ln])
    check("boot 2: our script not found", not [ln for ln in l2 if P.FOUND_OURS in ln])
    check("boot 2: the image did not run", P.IMAGE not in l2)
    check("boot 2: stock path reached", None not in drive.seq(l2, [P.FOUND_STOCK, P.STOCK]))
    check("boot 2: no bad CRC (stored environment read)", not [ln for ln in l2 if "bad CRC" in ln])
    # The note says the loop prints SCRIPT FAILED: continuing...; with no
    # script found it may not. Informational, not a verdict.
    if P.FAILED not in l2:
        print("note: boot 2 has no %r line (design note section 5 says the loop prints it)" % P.FAILED)
    if failed:
        print("qemu-undo: FAIL %s" % failed[0])
        return 1
    print("qemu-undo: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
