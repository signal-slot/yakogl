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
    """files: {bootfs path: .cmd under cases/}, each built to a script image;
    corrupt: the bootfs paths among them whose data CRC is then broken."""
    def __init__(self, name, script, presets, boots, check, files=None, corrupt=()):
        self.name, self.script, self.presets = name, script, presets
        self.boots, self.check = boots, check
        self.files, self.corrupt = files or {}, corrupt


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


# ST's 2023.10 boots the DK with the legacy distro loop (include/config_distro_bootcmd.h:
# scan_dev_for_boot, scan_dev_for_scripts, boot_a_script, "SCRIPT FAILED:
# continuing..."); QEMU's U-Boot uses bootstd, which walks prefixes differently
# (it stops at the first script it finds). The loop cases preset that legacy
# text (the extlinux step left out) and a bootcmd that runs it on the bootfs,
# partition 2; the setup boot saves it, so it is the baseline of the checks.
LEGACY_LOOP = [
    "setenv boot_scripts boot.scr.uimg boot.scr",
    "setenv boot_a_script 'load ${devtype} ${devnum}:${distro_bootpart} ${scriptaddr} ${prefix}${script}; "
    "source ${scriptaddr}'",
    "setenv scan_dev_for_scripts 'for script in ${boot_scripts}; do "
    "if test -e ${devtype} ${devnum}:${distro_bootpart} ${prefix}${script}; then "
    "echo Found U-Boot script ${prefix}${script}; run boot_a_script; "
    "echo SCRIPT FAILED: continuing...; fi; done'",
    "setenv scan_dev_for_boot 'echo Scanning ${devtype} ${devnum}:${distro_bootpart}...; "
    "for prefix in ${boot_prefixes}; do run scan_dev_for_scripts; done'",
    "setenv bootcmd 'setenv devtype mmc; setenv devnum 0; setenv distro_bootpart 2; "
    "run scan_dev_for_boot; echo LOOP DONE'",
]
YAKOGL = "yakogl/standalone/boot.scr.uimg"      # the product's prefix /yakogl/standalone/
STOCK = "mmc0_boot.scr.uimg"                    # the stand-in stock prefix /mmc0_


def loop_presets(prefixes):
    return [MMC_ONLY, "setenv boot_prefixes '%s'" % prefixes] + LEGACY_LOOP


def seq(lines, pats):
    """The index of each pattern's first line after the previous one, or None."""
    at, out = 0, []
    for p in pats:
        i = next((j for j in range(at, len(lines)) if lines[j].startswith(p)), None)
        out.append(i)
        if i is None:
            return out
        at = i + 1
    return out


def ordered(name, b, pats):
    got = seq(b.lines, pats)
    return ("boot %d: %s, in this order: %s" % (b.n, name, " | ".join(pats)),
            None not in got, "console lines at %r" % got)


def absent(b, pat):
    got = [ln for ln in b.lines if pat in ln]
    return ("boot %d: no line with %r" % (b.n, pat), not got, "got %r" % got)


def exit_check(setup_env, boots):
    """(a) exit ends only the sourced script; the loop goes on to the next prefix."""
    b = boots[0]
    return [ordered("exit ends the script, the loop goes on", b, [
                "Found U-Boot script /yakogl/standalone/boot.scr.uimg", "TEST first-prefix script runs",
                "SCRIPT FAILED: continuing...", "Found U-Boot script /mmc0_boot.scr.uimg",
                "TEST stock-prefix script runs", "LOOP DONE"]),
            absent(b, "TEST after-exit")]


def snapshot_check(setup_env, boots):
    """(b) boot_prefixes changed by a script does not change the list the loop walks."""
    b = boots[0]
    return [ordered("the stock prefix is still reached", b, [
                "TEST first-prefix script runs, boot_prefixes was /yakogl/standalone/ /mmc0_",
                "TEST boot_prefixes now /new/", "SCRIPT FAILED: continuing...",
                "Found U-Boot script /mmc0_boot.scr.uimg",
                "TEST stock-prefix script runs, boot_prefixes is /new/", "LOOP DONE"]),
            absent(b, "/new/boot.scr.uimg"), absent(b, "TEST new-prefix")]


def saveenv_check(setup_env, boots):
    """(c) saveenv inside a sourced script persists across a reset."""
    b1, b2 = boots
    return [ordered("saves the probe", b1, ["TEST probe unset", "TEST saveenv done", "SCRIPT FAILED"]),
            ("setup: the probe was not in the environment before", "mp2s_probe" not in setup_env, ""),
            ordered("reads the saved probe", b2, ["TEST probe is saved-by-script", "SCRIPT FAILED"]),
            absent(b2, "TEST probe unset"),
            ("boot 2: mp2s_probe in the environment", b2.env.get("mp2s_probe") == "saved-by-script",
             "is %r" % b2.env.get("mp2s_probe"))]


def corrupt_check(setup_env, boots):
    """(d) a corrupt script and a missing one fall through to the next prefix."""
    b = boots[0]
    return [ordered("bad CRC, then the loop goes on", b, [
                "Found U-Boot script /yakogl/standalone/boot.scr.uimg", "Bad data crc",
                "SCRIPT FAILED: continuing...", "Found U-Boot script /mmc0_boot.scr.uimg",
                "TEST stock-prefix script runs", "LOOP DONE"]),
            absent(b, "TEST corrupt script ran"), absent(b, "/absent/")]


def cases(here):
    def c(f):
        return os.path.join(here, "cases", f)
    p = "/yakogl/standalone/ /mmc0_"
    return [Case("demo-counter", os.path.join(here, "demo-counter.cmd"),
                 [MMC_ONLY, "setenv mp2s_test 3"], 3, demo_check),
            Case("loop_exit_continues", None, loop_presets(p), 1, exit_check,
                 {YAKOGL: c("exit-first.cmd"), STOCK: c("stock-marker.cmd")}),
            Case("loop_prefix_snapshot", None, loop_presets(p), 1, snapshot_check,
                 {YAKOGL: c("prefix-change.cmd"), STOCK: c("prefix-stock.cmd"),
                  "new/boot.scr.uimg": c("prefix-new.cmd")}),
            Case("loop_saveenv_persists", None, loop_presets(p), 2, saveenv_check,
                 {YAKOGL: c("saveenv-probe.cmd"), STOCK: c("stock-marker.cmd")}),
            Case("loop_corrupt_script", None, loop_presets("/yakogl/standalone/ /absent/ /mmc0_"), 1,
                 corrupt_check, {YAKOGL: c("corrupt-first.cmd"), STOCK: c("stock-marker.cmd")},
                 corrupt=(YAKOGL,))]


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
        files = dict(c.files)
        if c.script:
            files["boot.scr.uimg"] = c.script
        for dest, src in files.items():
            with open(src, "rb") as f:
                scr = us.build(f.read(), name=c.name, arch="arm64", time=0)
            if dest in c.corrupt:               # the last data byte: the data CRC no longer matches
                scr = scr[:-1] + bytes([scr[-1] ^ 0xff])
            os.makedirs(os.path.dirname(os.path.join(stage, dest)), exist_ok=True)
            with open(os.path.join(stage, dest), "wb") as f:
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
