#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Console side of run.sh (README.md in this directory).

    drive.py --qemu QEMU --uboot U-BOOT.BIN --image STANDIN.BIN --work DIR
             --uboot-script tools/uboot_script.py --logs DIR
             [--uboot-redund U-BOOT.BIN] [--only CASE]

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
The env_* cases run --uboot-redund (the environment in two copies on the
disk, as on the DK), read both copies after each boot and break their CRCs
between boots. The prod_redund_* cases do the same with the real product
script (../mp2-standalone.cmd, built by ../build.sh) as a counted default of
2: a power cut at each of its saves, and both copies bad. --only prod_redund
runs them all (--only NAME: NAME or every NAME_*).
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
    """lines: the console up to the prompt; env: printenv; info: the output of
    ENV_INFO; store: the two environment copies on the disk after the boot
    (uboot_env.Copy), the last two for redund cases only."""
    def __init__(self, n, lines, env, info=None, store=None):
        self.n, self.lines, self.env, self.info, self.store = n, lines, env, info or [], store


class Case:
    """files: {bootfs path: .cmd under cases/}, each built to a script image;
    corrupt: the bootfs paths among them whose data CRC is then broken.
    redund: run the redund U-Boot (the environment in two copies on the disk's
    u-boot-env partition, as on the DK); between: {n: f(disk)} run on the disk
    image after boot n, f returns assertions; bad_crc: the boots whose console
    must say "bad CRC" (every other boot must not).
    product: stage ../build.sh's boot.scr.uimg of the product script as
    /yakogl/standalone/boot.scr.uimg and the stand-in image as its default.bin;
    the harness's per-boot "unchanged" check then compares the protected
    variables (boot_prefixes left out: the product restores it) with the
    environment the boot loaded (the stored copy, or the defaults when both
    are bad), and each Boot gets loaded/saved/defaults (redund only)."""
    def __init__(self, name, script, presets, boots, check, files=None, corrupt=(),
                 redund=False, between=None, bad_crc=(), product=False):
        self.name, self.script, self.presets = name, script, presets
        self.boots, self.check = boots, check
        self.files, self.corrupt = files or {}, corrupt
        self.redund, self.between, self.bad_crc = redund, between or {}, bad_crc
        self.product = product


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


# The env_* cases: the redund U-Boot (mp2-standalone-redund.config) keeps the
# environment as the DK does, two 8 KiB copies at the top of the u-boot-env
# partition (mkdisk.sh env): copy 1 at CONFIG_ENV_OFFSET, copy 2 at
# CONFIG_ENV_OFFSET_REDUND. Each saveenv writes the copy not in use with the
# flags one higher; a load takes the valid copy, of two the higher flags
# (env/mmc.c, env/common.c env_check_redund). They are read and broken here
# with tools/uboot_env.py's parser.
UENV = load(os.path.join(HERE, "..", "..", "..", "..", "tools", "uboot_env.py"), "uboot_env")
ENV_SIZE = 0x2000
ENV_COPIES = (("copy 1", 0x1e7e000), ("copy 2", 0x1e7c000))
# Typed at the prompt after each boot of an env_* case: which copy is in use,
# and env_check's own test (default environment? can it be persisted?).
ENV_INFO = ("env info", "env info -d -p")


def env_store(disk):
    out = []
    with open(disk, "rb") as f:
        for label, off in ENV_COPIES:
            f.seek(off)
            out.append(UENV.Copy(label, disk, off, f.read(ENV_SIZE), True))
    return out


def env_vars(copy):
    return {UENV.name_of(e).decode(): UENV.value_of(e).decode() for e in copy.entries}


def describe(copy):
    if not copy.crc_ok:
        return "%s bad CRC" % copy.label
    v = env_vars(copy)
    k = "mp2s_left" if "mp2s_left" in v and "mp2s_test" not in v else "mp2s_test"   # prod_redund_*
    return "%s flags %d %s=%s" % (copy.label, copy.flags, k, v.get(k))


def break_crc(disk, copy):
    """Flips the low byte of the copy's stored CRC, so its data no longer
    matches it, as a write stopped half way by a power cut leaves it."""
    with open(disk, "r+b") as f:
        f.seek(copy.offset)
        b = f.read(1)
        f.seek(copy.offset)
        f.write(bytes([b[0] ^ 0xff]))


def cut_newer(disk):
    """(a) A power cut during the last boot's saveenv: the copy it wrote, the
    newer one, is torn; the older one is as the boot before left it."""
    s = env_store(disk)
    out = [("before the cut: both copies valid (%s)" % ", ".join(map(describe, s)),
            all(c.crc_ok for c in s), "")]
    if not out[0][1]:
        return out
    new = s[UENV.active_index(*s)]
    break_crc(disk, new)
    t = env_store(disk)
    out.append(("cut: the newer %s now fails its CRC, the older one holds (%s)"
                % (new.label, ", ".join(map(describe, t))),
                [c.crc_ok for c in t] == [c is not new for c in s], ""))
    return out


def cut_both(disk):
    """(b) Both copies bad."""
    for c in env_store(disk):
        break_crc(disk, c)
    t = env_store(disk)
    return [("both copies now fail their CRC (%s)" % ", ".join(map(describe, t)),
             not any(c.crc_ok for c in t), "")]


def counts(b):
    return [ln for ln in b.lines if re.fullmatch(r"boots left \d+", ln)]


def stored(b):
    """The copy the next load takes, or None when both are bad."""
    ok = [c for c in b.store if c.crc_ok]
    if len(ok) == 2:
        return b.store[UENV.active_index(*b.store)]
    return ok[0] if ok else None


def has(name, b, text, where=None):
    lines = b.info if where == "info" else b.lines
    return ("boot %d: %s" % (b.n, name), text in lines, "%s: %r" % (where or "console", lines[-12:]))


def powercut_check(setup_env, boots):
    """(a) The older copy loads, the count goes on: one extra boot at most."""
    out = []
    for b, want in zip(boots, (2, 2, 1, 0)):
        out.append(("boot %d: console says boots left %d" % (b.n, want), counts(b) == ["boots left %d" % want],
                    "got %r" % counts(b)))
        s = stored(b)
        out.append(("boot %d: the stored environment the next boot loads has mp2s_test=%d (%s)"
                    % (b.n, want, ", ".join(map(describe, b.store))),
                    s is not None and env_vars(s).get("mp2s_test") == str(want), ""))
    b2 = boots[1]
    out += [absent(b2, "Warning"),
            has("the environment came from the card, not the defaults", b2,
                "Environment was loaded from persistent storage", "info"),
            ordered("the older copy's count, saved again", b2,
                    ["Loading Environment from MMC... OK", "Saving Environment to MMC... Writing to",
                     "boots left 2"]),
            ("boot 2: its saveenv rewrote the torn copy: both valid again",
             all(c.crc_ok for c in b2.store), ", ".join(map(describe, b2.store)))]
    return out


def both_bad_check(setup_env, boots):
    """(b) Both copies bad: the defaults, env_check saves them, the next boot
    loads them and env_check does nothing."""
    b1, b2, b3 = boots
    s2, s3 = stored(b2), stored(b3)
    saving = [ln for ln in b2.lines if ln.startswith("Saving Environment to MMC...")]
    return [("boot 1: console says boots left 2", counts(b1) == ["boots left 2"], "got %r" % counts(b1)),
            ordered("bad CRC, the defaults, env_check saves them, then the scan", b2, [
                "Loading Environment from MMC... *** Warning - bad CRC, using default environment",
                "Hit any key to stop autoboot", "Saving Environment to MMC... Writing to",
                "mp2s: mp2s_test is not set"]),
            ("boot 2: env_check's save succeeded", len(saving) == 1 and saving[0].endswith("OK"),
             "got %r" % saving),
            has("env_check's test held: the defaults were in use", b2, "Default environment is used", "info"),
            ("boot 2: mp2s_test is gone from the environment", "mp2s_test" not in b2.env,
             "is %r" % b2.env.get("mp2s_test")),
            ("boot 2: the saved copy is the default environment (env_check set, no mp2s_test; %s)"
             % ", ".join(map(describe, b2.store)),
             s2 is not None and "mp2s_test" not in env_vars(s2) and "env_check" in env_vars(s2), ""),
            ordered("the saved defaults load, the scan runs", b3, [
                "Loading Environment from MMC... OK", "mp2s: mp2s_test is not set"]),
            absent(b3, "Saving Environment"),
            has("the environment came from the card", b3,
                "Environment was loaded from persistent storage", "info"),
            ("boot 3: the stored environment is unchanged by boot 3",
             s3 is not None and s2 is not None and s3.raw == s2.raw, "")]


# The prod_redund_* cases: the real product script (../mp2-standalone.cmd,
# built by ../build.sh) as a counted default of 2 on the redund U-Boot, through
# the legacy loop (prefixes /yakogl/standalone/ then the stock /mmc0_), with
# ST's env_check run first as ST's bootcmd_stm32mp does. A power cut at each
# saveenv the script does (cut_newer after boot 1, 2, 3) and one with both
# copies bad. The claim (docs/notes/stm32mp257f-dk-standalone.md, section 8):
# after any single cut the next boots end in the stock path within the count
# that was left, and the count never grows. Console lines: section 7.
P_UNDO = "mp2s: undo: setenv mp2s_left 0; saveenv; boot   (or a PC: delete yakogl/standalone/boot.scr.uimg)"
P_LEFT = "mp2s: default /yakogl/standalone/default.bin, boots left %d"
P_USEDUP = "mp2s: default used up, booting Linux"
P_LOADED = "mp2s: loaded 1000 bytes at 0xa0000000, RAM footprint 0x2000 of the 0x10000000-byte window"
P_GO = "mp2s: go 0xa0000000"
P_RETURNED = "mp2s: the image returned, booting Linux"
P_IMAGE = "mp2q: image ran at its link address 0xa0000000"
P_STOCK = "mp2q: stock path /mmc0_ reached (on the DK: ST extlinux, OpenSTLinux)"
P_ROOT = "mp2q: stock root /boot.scr.uimg reached (default environment; on the DK: OpenSTLinux)"
P_FOUND = "Found U-Boot script /yakogl/standalone/boot.scr.uimg"
P_FOUND_STOCK = "Found U-Boot script /mmc0_boot.scr.uimg"
P_FAILED = "SCRIPT FAILED: continuing..."
P_STOCK_PREFIXES = "/mmc0_"
P_OURS = "/yakogl/standalone/ " + P_STOCK_PREFIXES
P_MP2S = ("mp2s_left", "mp2s_ver", "mp2s_prefixes")
# Never in a boot's saved-environment diff (the item's contract).
P_NEVER = ("bootcmd", "bootcount", "altbootcmd")
# What the script may change in the saved environment, and only that.
P_MAY = P_MP2S + ("boot_prefixes",)
# The variables the loop itself sets before it sources the script (the same
# on the DK); a saveenv of the script stores them with the count. fileaddr and
# filesize are set by boot_a_script's load of the script itself.
P_LOOP_VARS = ("devtype", "devnum", "distro_bootpart", "fileaddr", "filesize")
# The legacy loop as LEGACY_LOOP, with ST's env_check first; bootcount and
# altbootcmd get a value so "unchanged" compares a real byte string (QEMU has
# no bootcount driver, ST's bootcount lives outside the environment).
P_LOOP = LEGACY_LOOP[:-1] + [
    "setenv bootcmd 'run env_check; setenv devtype mmc; setenv devnum 0; setenv distro_bootpart 2; "
    "run scan_dev_for_boot; echo LOOP DONE'",
    "setenv bootcount 0", "setenv altbootcmd 'echo mp2q: altbootcmd ran'"]
# "default 2" as the runner saves it; the trailing saves make three from a
# blank partition, so both copies hold it before boot 1 (as env_* do).
P_PRESETS = P_LOOP + ["setenv boot_prefixes '%s'" % P_OURS, "setenv mp2s_prefixes '%s'" % P_STOCK_PREFIXES,
                      "setenv mp2s_ver 1", "setenv mp2s_left 2", "saveenv", "saveenv"]
P_FILES = {STOCK: "prod-stock.cmd", "boot.scr.uimg": "prod-root-stock.cmd"}


def default_env():
    """The redund U-Boot's compiled-in default environment (redund-default.env):
    what a boot starts from when both stored copies are bad."""
    env = {}
    with open(os.path.join(HERE, "redund-default.env")) as f:
        for ln in f:
            ln = ln.rstrip("\n")
            if ln and not ln.startswith("#") and "=" in ln:
                k, v = ln.split("=", 1)
                env[k] = v
    return env


def loaded_env(store):
    """The environment a boot starts from on this store: the copy a load takes,
    else (both bad) the defaults."""
    ok = [c for c in store if c.crc_ok]
    if not ok:
        return default_env()
    return env_vars(store[UENV.active_index(*store)] if len(ok) == 2 else ok[0])


def p_kind(b):
    """What a boot did: ("image", left), ("usedup",), ("stock",) (the script is
    no longer reached), ("root",) (the defaults' scan), or ("other",)."""
    left = [int(ln.rsplit(" ", 1)[1]) for ln in b.lines if re.fullmatch(re.escape(P_LEFT[:-2]) + r"\d+", ln)]
    if P_IMAGE in b.lines and len(left) == 1:
        return ("image", left[0])
    if P_USEDUP in b.lines:
        return ("usedup",)
    if P_ROOT in b.lines and not any(ln.startswith("mp2s:") for ln in b.lines):
        return ("root",)
    if P_STOCK in b.lines and not any(ln.startswith("mp2s:") for ln in b.lines):
        return ("stock",)
    return ("other",)


def p_boot(b, want):
    """The assertions of one boot of the expected kind."""
    k = want[0]
    if k == "image":
        pats = [P_FOUND, P_UNDO, P_LEFT % want[1], P_LOADED, P_GO, P_IMAGE, P_RETURNED, P_FAILED,
                P_FOUND_STOCK, P_STOCK, "LOOP DONE"]
    elif k == "usedup":
        pats = [P_FOUND, P_UNDO, P_USEDUP, P_FAILED, P_FOUND_STOCK, P_STOCK, "LOOP DONE"]
    elif k == "stock":
        pats = ["Scanning mmc 0:2...", P_FOUND_STOCK, P_STOCK, "LOOP DONE"]
    else:
        pats = ["Hit any key to stop autoboot", P_ROOT]
    out = [ordered("%s" % " ".join(map(str, want)), b, pats),
           ("boot %d: it did %r" % (b.n, want), p_kind(b) == want, "did %r" % (p_kind(b),))]
    if k != "image":
        out += [absent(b, P_GO), absent(b, P_IMAGE)]
    if k in ("stock", "root"):
        out.append(absent(b, P_FOUND))
    return out


def p_store_after(b, want):
    """What the next boot loads after this one (a cut, if any, is applied later)."""
    s = stored(b)
    e = env_vars(s) if s is not None else {}
    k = want[0]
    if k == "image":
        return [("boot %d: stored mp2s_left %d, boot_prefixes ours" % (b.n, want[1]),
                 e.get("mp2s_left") == str(want[1]) and e.get("boot_prefixes") == P_OURS,
                 "mp2s_left %r boot_prefixes %r" % (e.get("mp2s_left"), e.get("boot_prefixes")))]
    if k == "usedup":
        return [("boot %d: stored boot_prefixes back to %r, mp2s_* gone" % (b.n, P_STOCK_PREFIXES),
                 e.get("boot_prefixes") == P_STOCK_PREFIXES and not any(v in e for v in P_MP2S),
                 "boot_prefixes %r, %r" % (e.get("boot_prefixes"), [v for v in P_MP2S if v in e]))]
    if b.defaults:
        d = default_env()
        return [("boot %d: env_check saved the defaults: bootcmd the default one, no mp2s_*, "
                 "boot_prefixes not ours" % b.n,
                 s is not None and e.get("bootcmd") == d["bootcmd"] and not any(v in e for v in P_MP2S)
                 and e.get("boot_prefixes") != P_OURS, "%r" % sorted(e))]
    return [("boot %d: stored environment as the boot loaded it (nothing saved)" % b.n,
             s is not None and env_vars(s) == b.loaded, "")]


def p_left(env):
    """The count a boot loads: the number, or -1 once mp2s_left is gone."""
    v = env.get("mp2s_left")
    return int(v) if v is not None and v.isdigit() else -1


def prod_check(expect, extra):
    """expect: the kind of each boot; extra: the image boots beyond N=2 the
    cut may cost (0 or 1)."""
    def check(setup_env, boots):
        out = []
        for b, want in zip(boots, expect):
            out += p_boot(b, want) + p_store_after(b, want)
            diff = sorted(k for k in set(b.loaded) | set(b.saved)
                          if b.loaded.get(k) != b.saved.get(k))
            out.append(("boot %d: %s not in the saved environment diff (diff %s)"
                        % (b.n, ", ".join(P_NEVER), ", ".join(diff) or "empty"),
                        not set(diff) & set(P_NEVER), ""))
            if not b.defaults:
                out.append(("boot %d: the saves changed only %s and the loop's own variables"
                            % (b.n, ", ".join(P_MAY)),
                            set(diff) <= set(P_MAY + P_LOOP_VARS), "diff %r" % diff))
        loads = [p_left(b.loaded) for b in boots]
        out.append(("the count each boot loads never grows (%s)" % ", ".join(map(str, loads)),
                    all(x >= y for x, y in zip(loads, loads[1:])), ""))
        shown = [k[1] for k in map(p_kind, boots) if k[0] == "image"]
        out.append(("console boots left never grows (%s)" % ", ".join(map(str, shown)),
                    all(x >= y for x, y in zip(shown, shown[1:])), ""))
        out.append(("image boots %d, at most N + %d = %d" % (len(shown), extra, 2 + extra),
                    len(shown) <= 2 + extra, ""))
        last = p_kind(boots[-1])
        out.append(("the last boot ends in the stock path (%s)" % last[0], last[0] in ("stock", "root"), ""))
        return out
    return check


def cases(here):
    def c(f):
        return os.path.join(here, "cases", f)
    p = "/yakogl/standalone/ /mmc0_"
    pf = {d: c(f) for d, f in P_FILES.items()}
    i1, i0, u, s, r = ("image", 1), ("image", 0), ("usedup",), ("stock",), ("root",)
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
                 corrupt=(YAKOGL,)),
            # The setup saves three times from a blank partition: copy 1
            # twice (no copy is in use yet, then copy 2 counts as in use),
            # then copy 2, so both copies hold mp2s_test=3 before boot 1.
            Case("env_powercut_saveenv", os.path.join(here, "demo-counter.cmd"),
                 ["setenv mp2s_test 3", "saveenv", "saveenv"], 4, powercut_check,
                 redund=True, between={1: cut_newer}),
            Case("env_both_bad_default", os.path.join(here, "demo-counter.cmd"),
                 ["setenv mp2s_test 3", "saveenv", "saveenv"], 3, both_bad_check,
                 redund=True, between={1: cut_both}, bad_crc=(2,)),
            # The product's default 2 uncut: the baseline of the cuts below.
            Case("prod_redund_nocut", None, P_PRESETS, 4, prod_check([i1, i0, u, s], 0), pf,
                 redund=True, product=True),
            # A cut at boot 1's save (2 -> 1): boot 1 is done again.
            Case("prod_redund_cut_save1", None, P_PRESETS, 5, prod_check([i1, i1, i0, u, s], 1), pf,
                 redund=True, product=True, between={1: cut_newer}),
            # A cut at boot 2's save (1 -> 0).
            Case("prod_redund_cut_save2", None, P_PRESETS, 5, prod_check([i1, i0, i0, u, s], 1), pf,
                 redund=True, product=True, between={2: cut_newer}),
            # A cut at boot 3's save (the used-up branch's restore): the count
            # stays 0, boot 4 restores again, no image.
            Case("prod_redund_cut_restore", None, P_PRESETS, 5, prod_check([i1, i0, u, u, s], 0), pf,
                 redund=True, product=True, between={3: cut_newer}),
            # Both copies bad after boot 1: the defaults, env_check saves them,
            # our prefix is gone: the stock root script on boots 2 and 3.
            Case("prod_redund_both_bad", None, P_PRESETS, 3, prod_check([i1, r, r], 0), pf,
                 redund=True, product=True, between={1: cut_both}, bad_crc=(2,))]


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
    ap.add_argument("--uboot-redund", help="the redund U-Boot (build-uboot.sh redund), for env_* cases")
    a = ap.parse_args()
    us = load(a.uboot_script, "uboot_script")
    res = {"ok": 0, "failed": 0}

    def check(name, cond, detail=""):
        res["ok" if cond else "failed"] += 1
        print("%s %s%s" % ("ok  " if cond else "FAIL", name,
                           "" if cond or not detail else ": " + detail), flush=True)

    for c in cases(HERE):
        # --only NAME: the case NAME, or every case NAME_* (prod_redund).
        if a.only and a.only != c.name and not c.name.startswith(a.only + "_"):
            continue
        if c.redund and not a.uboot_redund:
            print("skip %s: no --uboot-redund" % c.name, flush=True)
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
        if c.product:                           # the real script, as build.sh builds it
            r = subprocess.run(["bash", os.path.join(HERE, "..", "build.sh"), os.path.join(w, "scr")],
                               check=True, capture_output=True, text=True)
            print("     %s: %s" % (c.name, r.stdout.strip().splitlines()[-1]), flush=True)
            os.makedirs(os.path.join(stage, "yakogl", "standalone"), exist_ok=True)
            for src, dest in ((os.path.join(w, "scr", "boot.scr.uimg"), YAKOGL),
                              (a.image, "yakogl/standalone/default.bin")):
                with open(src, "rb") as f, open(os.path.join(stage, dest), "wb") as g:
                    g.write(f.read())
        disk, flash = os.path.join(w, "sd.img"), os.path.join(w, "env.img")
        subprocess.run(["bash", os.path.join(HERE, "mkdisk.sh"), disk, stage] + (["env"] if c.redund else []),
                       check=True)
        with open(flash, "wb") as f:            # QEMU virt's second pflash: U-Boot's ENV_ADDR
            f.truncate(64 << 20)
        argv = [a.qemu, "-machine", "virt", "-cpu", "cortex-a57", "-m", "2G",
                "-display", "none", "-serial", "stdio", "-monitor", "none", "-no-reboot",
                "-bios", a.uboot_redund if c.redund else a.uboot,
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
            before = env_store(disk) if c.redund else None
            def body(q, n=n):
                q.wait_prompt()
                lines = q.lines()
                env = printenv(q)
                info = [ln for i in (ENV_INFO if c.redund else ()) for ln in q.cmd(i)]
                return Boot(n, lines, env, info)
            b = session("boot%d" % n, body)
            if b is None:
                break
            boots.append(b)
            bad = any("bad CRC" in ln for ln in b.lines)
            if n in c.bad_crc:
                check("%s: boot %d: bad CRC, the default environment (as the case wants)" % (c.name, n), bad)
            else:
                check("%s: boot %d: stored environment read (no bad CRC)" % (c.name, n), not bad)
            if c.redund:                        # QEMU has exited: the disk is as the boot left it
                b.store = env_store(disk)
            if c.product:
                b.defaults = not any(x.crc_ok for x in before)
                b.loaded, b.saved = loaded_env(before), loaded_env(b.store)
                for k in PROTECTED:
                    if k == "boot_prefixes":
                        continue
                    check("%s: boot %d: %s as the boot loaded it%s" % (c.name, n, k,
                          " (the defaults)" if b.defaults else ""), b.env.get(k) == b.loaded.get(k),
                          "%r -> %r" % (b.loaded.get(k), b.env.get(k)))
                    check("%s: boot %d: %s as the boot loaded it, in the saved environment"
                          % (c.name, n, k), b.saved.get(k) == b.loaded.get(k),
                          "%r -> %r" % (b.loaded.get(k), b.saved.get(k)))
                print("     %s: boot %d: %s" % (c.name, n, " / ".join(
                    ln for ln in b.lines if ln.startswith(("mp2s:", "mp2q:")))), flush=True)
            else:
                for k in PROTECTED:
                    check("%s: boot %d: %s unchanged" % (c.name, n, k), b.env.get(k) == setup_env.get(k),
                          "%r -> %r" % (setup_env.get(k), b.env.get(k)))
            if n in c.between:
                for name, ok, detail in c.between[n](disk):
                    check("%s: after boot %d: %s" % (c.name, n, name), ok, detail)
        if len(boots) == c.boots:
            for name, ok, detail in c.check(setup_env, boots):
                check("%s: %s" % (c.name, name), ok, detail)
    print("run.sh: %d ok, %d failed" % (res["ok"], res["failed"]))
    return 0 if res["failed"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
