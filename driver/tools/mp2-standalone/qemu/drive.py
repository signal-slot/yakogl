#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Console side of run.sh (README.md in this directory).

    drive.py --qemu QEMU --uboot U-BOOT.BIN --image STANDIN.BIN --work DIR
             --uboot-script tools/uboot_script.py --logs DIR

A case is: a boot script (.cmd), environment presets, N boots. For each case
drive.py builds boot.scr.uimg (uboot_script.py), a GPT disk with a bootfs
partition holding it (mkdisk.sh), a fresh environment flash, and then

  setup  one QEMU boot that stops autoboot, types the presets, saves the
         environment ("saveenv") and prints it;
  boot 1..N  one QEMU boot each on the same flash and disk, untouched: U-Boot's
         own bootcmd (the distro scan) runs boot.scr.uimg; when it is back at
         the prompt the boot reads its environment and powers off.

The console log of every boot is in --logs. A case's check gets the setup
environment and a Boot per boot (lines, environment) and returns the
(name, ok, detail) of its assertions; the harness adds its own for each boot
(no bad CRC, bootcmd and the other protected variables as the setup left them).
Prints one line per assertion and "run.sh: N ok, M failed" last; exit 0 only
when nothing failed. More cases: add a Case to cases() (qemu-run's job).
"""
import argparse
import importlib.util
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
# The variables of ST's boot flow that no script of the product may change
# (docs/notes/stm32mp257f-dk-standalone.md); the harness checks them after
# every boot. bootcmd is the one that matters on QEMU, the rest are listed so
# a case that presets them is checked too.
PROTECTED = ("bootcmd", "boot_targets", "boot_prefixes", "bootcount", "bootlimit",
             "altbootcmd", "upgrade_available")


# QEMU's default boot_targets (qfw usb scsi virtio nvme dhcp) has no mmc and
# dhcp would try QEMU's TFTP: scan the microSD only, as the DK's loop reaches
# its bootfs. The setup boot saves it, so it is the baseline of the checks.
MMC_ONLY = "setenv boot_targets mmc0"


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# The console driver of ../../uboot-mp2/qemu/drive.py (spawn, expect, cmd).
BASE = load(os.path.join(HERE, "..", "..", "uboot-mp2", "qemu", "drive.py"), "uboot_mp2_drive")


class Qemu(BASE.Qemu):
    def stop_autoboot(self):
        self.expect(BASE.STOP, 120)
        self.send(" ")
        self.expect(BASE.PROMPT, 20)

    def wait_prompt(self, timeout=120):
        """An untouched boot: U-Boot's autoboot runs, then the prompt."""
        self.expect(BASE.PROMPT, timeout)

    def lines(self):
        return self.buf.decode(errors="replace").replace("\r", "").split("\n")


class Boot:
    def __init__(self, n, lines, env):
        self.n, self.lines, self.env = n, lines, env


class Case:
    def __init__(self, name, script, presets, boots, check):
        self.name, self.script, self.presets = name, script, presets
        self.boots, self.check = boots, check


def demo_check(setup_env, boots):
    """The counter script: boots left 2, 1, 0; the saved variable follows."""
    out = []
    left = []
    for b in boots:
        got = [ln for ln in b.lines if re.fullmatch(r"boots left \d+", ln)]
        left.append(got)
        want = ["boots left %d" % (len(boots) - b.n)]
        out.append(("boot %d: console says %s" % (b.n, want[0]), got == want, "got %r" % got))
        out.append(("boot %d: mp2s_test is %d in the environment after it" % (b.n, len(boots) - b.n),
                    b.env.get("mp2s_test") == str(len(boots) - b.n), "is %r" % b.env.get("mp2s_test")))
    out.append(("env persisted across QEMU restarts: boots left %s"
                % ", ".join(g[0].split()[-1] for g in left if g),
                [g[0] for g in left if g] == ["boots left %d" % (len(boots) - i)
                                              for i in range(1, len(boots) + 1)], ""))
    return out


def cases(here):
    return [Case("demo-counter", os.path.join(here, "demo-counter.cmd"),
                 [MMC_ONLY, "setenv mp2s_test 3"], 3, demo_check)]


def printenv(q):
    env = {}
    for ln in q.cmd("printenv"):
        if ln.startswith("Environment size:") or "=" not in ln:
            continue
        k, v = ln.split("=", 1)
        env[k] = v
    return env


def main():
    ap = argparse.ArgumentParser()
    for a in ("qemu", "uboot", "image", "work", "uboot-script", "logs"):
        ap.add_argument("--" + a, required=True)
    ap.add_argument("--only", help="run only the case of this name")
    a = ap.parse_args()
    us = load(a.uboot_script, "uboot_script")
    res = {"ok": 0, "failed": 0}

    def check(name, cond, detail=""):
        res["ok" if cond else "failed"] += 1
        print("%s %s%s" % ("ok  " if cond else "FAIL", name,
                           "" if cond or not detail else ": " + detail), flush=True)

    for c in cases(HERE):
        if a.only and a.only != c.name:
            continue
        w = os.path.join(a.work, c.name)
        stage = os.path.join(w, "bootfs")
        os.makedirs(os.path.join(stage, "yakogl"), exist_ok=True)
        with open(c.script, "rb") as f:
            scr = us.build(f.read(), name=c.name, arch="arm64", time=0)
        with open(os.path.join(stage, "boot.scr.uimg"), "wb") as f:
            f.write(scr)
        with open(a.image, "rb") as f, open(os.path.join(stage, "yakogl", "mp2z.bin"), "wb") as g:
            g.write(f.read())
        disk, flash = os.path.join(w, "sd.img"), os.path.join(w, "env.img")
        subprocess.run(["bash", os.path.join(HERE, "mkdisk.sh"), disk, stage], check=True)
        with open(flash, "wb") as f:            # QEMU virt's second pflash: U-Boot's ENV_ADDR
            f.truncate(64 << 20)
        argv = [a.qemu, "-machine", "virt", "-cpu", "cortex-a57", "-m", "2G",
                "-display", "none", "-serial", "stdio", "-monitor", "none", "-no-reboot",
                "-bios", a.uboot,
                "-drive", "if=pflash,unit=1,format=raw,file=" + flash,
                "-device", "sdhci-pci",
                "-drive", "if=none,id=sd0,format=raw,file=" + disk,
                "-device", "sd-card,drive=sd0"]

        def session(label, body):
            q = Qemu(argv, os.path.join(a.logs, "%s-%s.log" % (c.name, label)))
            try:
                return body(q)
            except (TimeoutError, EOFError) as e:
                check("%s: %s: console" % (c.name, label), False, "%s (log %s)" % (e, q.log.name))
                return None
            finally:
                q.close()

        def setup(q):
            q.stop_autoboot()
            for p in c.presets:
                q.cmd(p)
            q.cmd("saveenv")
            return printenv(q)

        setup_env = session("setup", setup)
        if setup_env is None:
            continue
        boots = []
        for n in range(1, c.boots + 1):
            def body(q, n=n):
                q.wait_prompt()
                lines = q.lines()
                return Boot(n, lines, printenv(q))
            b = session("boot%d" % n, body)
            if b is None:
                break
            boots.append(b)
            check("%s: boot %d: stored environment read (no bad CRC)" % (c.name, n),
                  not any("bad CRC" in ln for ln in b.lines))
            for k in PROTECTED:
                check("%s: boot %d: %s unchanged" % (c.name, n, k), b.env.get(k) == setup_env.get(k),
                      "%r -> %r" % (setup_env.get(k), b.env.get(k)))
        if len(boots) == c.boots:
            for name, ok, detail in c.check(setup_env, boots):
                check("%s: %s" % (c.name, name), ok, detail)
    print("run.sh: %d ok, %d failed" % (res["ok"], res["failed"]))
    return 0 if res["failed"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
