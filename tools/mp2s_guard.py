#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Read-only guard: refuse a one-shot while a standalone default is set, and the reverse.

    tools/mp2s_guard.py --for oneshot    [--printenv FILE | --env-image FILE] [LAYOUT]
    tools/mp2s_guard.py --for standalone [--printenv FILE | --env-image FILE] [LAYOUT]
    tools/mp2s_guard.py --self-test

  Input, at most one; with neither, the fw_printenv text is read on stdin:
    --printenv FILE   the text of `fw_printenv` (NAME=VALUE lines): a board
                      runner pipes its fw_printenv into this tool or keeps
                      it in FILE
    --env-image FILE  the environment as an image (what the board runners
                      dd off the `u-boot-env` partition), read with the
                      functions of tools/uboot_env.py -- imported from this
                      file's own directory, that file not edited. The
                      variables seen are those of the copy env_import_redund
                      would make active (tools/uboot_env.py:194-201): a
                      stale copy's variables are not live and do not count.
  LAYOUT, only with --env-image (the same meaning as in tools/uboot_env.py;
  N decimal or 0x hex):
    --size N  --offset N  --offset2 N  --redundant  --second FILE
    the DK's partition as tools/board-stm32mp2.sh reads it:
    --redundant --size 0x2000 --offset 0x7e000 --offset2 0x7c000

Why (docs/notes/stm32mp257f-dk-standalone.md, section 7 :544-547): the
one-shot runners (tools/board-stm32mp2.sh run, tools/board-stm32mp2-uboot.sh)
must refuse while `mp2s_left` is set, because the boot after their image
would not be Linux: with the standalone default armed, `boot_prefixes`
carries `/yakogl/standalone/` first (the note's variable table :236-242),
ST's distro loop reaches our script before ST's extlinux, and the script's
`mp2s_left` branch loads `default.bin` instead. Those runners belong to
plans #357 and #349, so the wiring is their work; this tool is the read-only
check they call. It is reversible: the standalone runner's `default` calls
the same tool with --for standalone (the note's "Never touched" :248-251 and
its pre-check :344-345).

  --for oneshot    REFUSE (exit 3) while any `mp2s_*` variable is in the
    environment: `mp2s_left` names the armed default; a stray `mp2s_*`
    without `mp2s_left` refuses too, because the design writes and deletes
    those variables in one environment write (section 3 :244-246), so a
    leftover means an undo is owed. Exit 0 otherwise: a pending `yako_*` or
    `mp2z_*` is the caller's own state to check, and a leftover
    `/yakogl/standalone/` prefix without any `mp2s_*` is not a default --
    our script prints `no default set` and exits, or is missing or corrupt
    (`SCRIPT FAILED: continuing...`), and ST's extlinux boots (section 3
    step 2 :265-266, section 8 :556).

  --for standalone REFUSE (exit 3), naming the variables, while any
    `yako_*` (the old runner's hook, tools/board-stm32mp2.sh:285) or
    `mp2z_*` (the typed one-shot's route,
    docs/notes/stm32mp257f-dk-uboot-boot.md :415-418) is in the
    environment, or while `boot_prefixes` already contains
    `/yakogl/standalone/` (a default is armed or was left behind, section 5
    :391-395). Exit 0 otherwise -- including while `mp2s_*` are set: `default`
    refusing a second default is the runner's own check (the note's runner-
    verbs table :533).

  --self-test  the 30 pinned checks of these verdicts, without a board
    (fixtures in $TMPDIR, both input forms, exit codes and messages pinned):
    exit 0 when all 30 pass, 1 otherwise. It lives in the tool rather than in
    a tools/mp2s_guard_selftest.sh of its own because the gate's scope map
    sends an unknown tools/*.sh to `all` -- a pull request adding one runs
    every row of the nightly plan (tools/check.sh:176-204, 337-341) -- while
    tools/*.py maps to `tools`; the check is a `--self-test` on the tool, as
    tools/api_coverage.py, tools/tftp_oneshot.py and tools/vivante/
    gen_shader_table.py carry theirs. The gate row that runs it (plan #341)
    is `python3 tools/mp2s_guard.py --self-test`, scope tools.

  Exit codes; a caller must treat every non-zero as a refusal (--self-test
  uses 0 and 1, and a caller never passes it):
    0  allow: one `mp2s-guard: allow: ...` line on stdout, empty stderr
    3  refuse: exactly one `mp2s-guard: REFUSE: <what is set>: <what to run
       first>` line on stderr, empty stdout
    2  the environment could not be read -- unreadable input, a bad CRC,
       strings that do not end inside the data area, a layout that does not
       fit the file -- or a wrong command line: one `mp2s-guard: error: ...`
       line (or argparse's usage) on stderr, empty stdout. A caller maps it
       like its own "refused before the board was changed"
       (tools/board-stm32mp2.sh:176-181) and goes on to investigate.

The guard reads: it never writes a file, never edits an environment, never
touches a board. A name counts as set even with an empty value (the safe
direction: U-Boot's himport_r reads a saved `name=` as a deletion, so a
board's own fw_printenv would not show it, but an image that holds one is
not clean). Duplicated names take the last value, as himport_r does
(tools/uboot_env.py:127-133). Values are shown as read (bytes decoded with
backslashreplace); nothing here scrubs a host, an address or a path, because
the environment's own names and values are the subject.
"""
import argparse
import importlib.util
import os
import re
import subprocess
import sys
import tempfile
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
PREFIX_STANDALONE = b"mp2s_"
PREFIX_ONESHOT = (b"yako_", b"mp2z_")
STANDALONE_BOOT_PREFIX = "/yakogl/standalone/"
STANDALONE_RUNNER = "tools/board-stm32mp2-standalone.sh"


class Error(Exception):
    """The environment could not be read: exit 2; the caller must refuse."""


def load_uboot_env():
    """tools/uboot_env.py, from this file's own directory. importlib rather
    than a plain import so it also works under `python3 -I`, the way
    tools/board-stm32mp2.sh invokes its env_tool (board-stm32mp2.sh:382-388).
    The module is used read-only; the file is not edited."""
    path = os.path.join(HERE, "uboot_env.py")
    spec = importlib.util.spec_from_file_location("yakogl_uboot_env", path)
    if spec is None or spec.loader is None:
        raise Error("cannot load %s" % path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["yakogl_uboot_env"] = mod
    spec.loader.exec_module(mod)
    return mod


NAME_RE = re.compile(rb"[A-Za-z0-9_]+")


def txt(b):
    return b.decode("utf-8", "backslashreplace")


def pairs_text(pairs):
    return " ".join("%s=%s" % (txt(n), txt(v)) for n, v in pairs)


def normalize(pairs):
    """One entry per name, first occurrence's place, last occurrence's value
    (the order U-Boot's himport_r settles on)."""
    order, last = [], {}
    for n, v in pairs:
        if n not in last:
            order.append(n)
        last[n] = v
    return [(n, last[n]) for n in order]


def read_bytes(path):
    try:
        with open(path, "rb") as fh:
            return fh.read()
    except OSError as err:
        raise Error("%s is not a readable file (%s)" % (path, err.strerror or err))


def parse_printenv(raw, src):
    """fw_printenv's NAME=VALUE lines. Other lines (fw_printenv's own error
    lines, console chatter) are skipped; input with no NAME=VALUE line at all
    is not a fw_printenv print."""
    pairs = []
    for line in raw.split(b"\n"):
        line = line.rstrip(b"\r")
        at = line.find(b"=")
        if at < 1 or not NAME_RE.fullmatch(line[:at]):
            continue
        pairs.append((line[:at], line[at + 1:]))
    if not pairs:
        raise Error("unreadable input: no NAME=VALUE line in %s" % src)
    return normalize(pairs)


def parse_env_image(path, layout, ue):
    """The entries of the copy env_import_redund would make active. Any bad
    copy is an error: tools/board-stm32mp2.sh refuses to write when either
    copy's CRC fails (board-stm32mp2.sh:65-66), so a guard call over such an
    image is a refusal too."""
    try:
        img = ue.Image(path, layout, layout.second)
    except ue.Refused as err:
        raise Error("env image %s: %s" % (path, err))
    except OSError as err:
        raise Error("%s is not a readable file (%s)" % (path, err.strerror or err))
    bad = img.problems()
    if bad:
        raise Error("env image %s does not check: %s" % (path, "; ".join(bad)))
    entries = img.copies[img.active()].entries
    return normalize([(ue.name_of(e), ue.value_of(e)) for e in entries])


def verdict_oneshot(pairs, src):
    hits = [(n, v) for n, v in pairs if n.startswith(PREFIX_STANDALONE)]
    if not hits:
        return 0, "allow: no standalone default (no mp2s_* in %s)" % src
    left = next((v for n, v in hits if n == b"mp2s_left"), None)
    if left is not None:
        what = "a standalone default is set (mp2s_left=%s)" % txt(left)
    else:
        what = "standalone variables are set (%s)" % pairs_text(hits)
    return 3, "REFUSE: %s: run %s nodefault first" % (what, STANDALONE_RUNNER)


def verdict_standalone(pairs, src):
    parts = []
    yz = [(n, v) for n, v in pairs if any(n.startswith(p) for p in PREFIX_ONESHOT)]
    if yz:
        tips = []
        if any(n.startswith(b"yako_") for n, _ in yz):
            tips.append("run tools/board-stm32mp2.sh disarm (yako_once) or "
                        "uninstall (the hook's variables)")
        if any(n.startswith(b"mp2z_") for n, _ in yz):
            tips.append("undo the typed one-shot first "
                        "(stm32mp257f-dk-uboot-boot.md section 7)")
        parts.append("a one-shot is set (%s): %s" % (pairs_text(yz), "; ".join(tips)))
    boot_prefixes = next((v for n, v in pairs if n == b"boot_prefixes"), None)
    if boot_prefixes is not None and STANDALONE_BOOT_PREFIX.encode() in boot_prefixes:
        parts.append("boot_prefixes already contains %s (boot_prefixes=%s): "
                     "run %s nodefault first"
                     % (STANDALONE_BOOT_PREFIX, txt(boot_prefixes), STANDALONE_RUNNER))
    if not parts:
        return 0, ("allow: no one-shot state (no yako_*, no mp2z_*, boot_prefixes "
                   "free of %s) in %s" % (STANDALONE_BOOT_PREFIX, src))
    return 3, "REFUSE: " + "; ".join(parts)


# ---- the self-test (tools/mp2s_guard.py --self-test) --------------------
#
# The 30 pinned checks of this guard without a board (plan #362, item
# oneshot-guard; docs/notes/stm32mp257f-dk-standalone.md section 7 :544-547:
# the one-shot runners refuse while mp2s_left is set, and the standalone
# default refuses while yako_*/mp2z_* are set). Every check runs the tool as
# a caller would (`python3 -I`, real streams, real exit code) against a
# fixture in a temporary directory under $TMPDIR, removed at the end:
#   - fw_printenv texts: a clean ST environment, mp2s_left=2,
#     mp2s_left=forever, a stray mp2s_prefixes (no mp2s_left), yako_once,
#     mp2z_token, a doubled /yakogl/standalone/ prefix with no mp2s_* left,
#     yako_once plus the doubled prefix, and a garbage file;
#   - environment images (a tiny stdlib builder: CRC-32 LE, a flags byte per
#     copy, NUL-terminated strings, zero padding; one built with
#     tools/uboot_env.py's own set writer; one with the DK's real layout,
#     copies at 0x7e000/0x7c000 in a 512 KiB partition).
# Verdicts (30 checks):
#   - allow (exit 0, exactly one "mp2s-guard: allow:" line on stdout, empty
#     stderr): clean, both modes and both forms; a stray mp2s_* is not a
#     one-shot; yako_*/mp2z_* and a doubled prefix are not a standalone
#     default; mp2s_* are not one-shot state; the mp2s_left kept only in the
#     stale redundant copy (flags 0, the newer copy is clean) is not live;
#   - REFUSE (exit 3, exactly one line on stderr equal to the pinned message,
#     empty stdout): the canonical oneshot refusals for mp2s_left=2 (file and
#     stdin form) and mp2s_left=forever, the stray-variables refusal, the
#     standalone refusals for yako_once, mp2z_token, the doubled prefix, both
#     together, and the mp2z_load/mp2z_window left in the active copy of a
#     DK-layout image;
#   - error (exit 2, empty stdout, no REFUSE line, stderr naming it): a
#     flipped data byte (bad CRC, single copy), a bad CRC on the copy that is
#     not active, a missing --printenv file, a missing --env-image file,
#     garbage and an empty stdin ("unreadable input"), a missing --for, both
#     input files at once, a layout flag used with --printenv.
WANT_CHECKS = 30

ST_CLEAN = ["bootcmd=run bootcmd_stm32mp", "boot_targets=mmc0 mmc1 distro",
            "boot_prefixes=/mmc0_ /mmc1_", "bootdelay=1"]


def st_pe(path, *vars_):
    """A fw_printenv text: NAME=VALUE lines. A NAME written twice keeps the
    last value (U-Boot's himport_r), as the guard reads it."""
    with open(path, "wb") as fh:
        for v in vars_:
            fh.write(v.encode() + b"\n")
    return path


def st_envbin(path, size, mode, *lines):
    """An environment image, one line per copy, FLAGS|NAME=VALUE|... (values
    hold no '|'). CRC-32 LE, a flags byte with r, strings then zero padding
    to the copy size."""
    red = mode == "r"
    hdr = 5 if red else 4
    out = b""
    for line in lines:
        flag, *vars_ = line.split("|")
        data = b"".join(v.encode() + b"\0" for v in vars_) + b"\0"
        if len(data) + hdr > size:
            raise Error("st_envbin: the strings need %d bytes, the copy size is %d"
                        % (len(data) + hdr, size))
        area = data + b"\0" * (size - hdr - len(data))
        out += zlib.crc32(area).to_bytes(4, "little")
        out += bytes([int(flag, 0)]) if red else b""
        out += area
    with open(path, "wb") as fh:
        fh.write(out)
    return path


def st_dklayout(path):
    """The u-boot-env partition as board-stm32mp2.sh reads it: 512 KiB, copy 1
    at 0x7e000 (flags 0x08, the active one, armed with mp2z_load/mp2z_window)
    and copy 2 at 0x7c000 (flags 0x07, clean)."""
    armed = ST_CLEAN + ["mp2z_load=0xa0000000", "mp2z_window=0x10000000"]

    def copy(entries, flags):
        data = b"".join(v.encode() + b"\0" for v in entries) + b"\0"
        area = data + b"\0" * (0x2000 - 5 - len(data))
        return zlib.crc32(area).to_bytes(4, "little") + bytes([flags]) + area

    buf = bytearray(0x80000)
    buf[0x7c000:0x7c000 + 0x2000] = copy(ST_CLEAN, 0x07)
    buf[0x7e000:0x7e000 + 0x2000] = copy(armed, 0x08)
    with open(path, "wb") as fh:
        fh.write(buf)
    return path


def st_corrupt(path, off):
    """Flip one byte: an image whose CRC no longer matches its data."""
    b = bytearray(read_bytes(path))
    b[off] ^= 0xFF
    with open(path, "wb") as fh:
        fh.write(bytes(b))
    return path


def self_test():
    tmp = tempfile.TemporaryDirectory(prefix="mp2s-guard-selftest.")
    t = tmp.name
    passed, failed = [], []

    def run(args, stdin_file=None):
        argv = [sys.executable, "-I", os.path.abspath(__file__)] + list(args)
        if stdin_file is None:
            p = subprocess.run(argv, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        else:
            with open(stdin_file, "rb") as fh:
                p = subprocess.run(argv, stdin=fh,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        dec = lambda b: b.decode("utf-8", "backslashreplace")
        return p.returncode, dec(p.stdout), dec(p.stderr)

    def report(what, want, rc, out, err):
        failed.append(what)
        print("FAIL %s (want %s; got rc %d)" % (what, want, rc))
        for line in out.splitlines():
            print("  out| %s" % line)
        for line in err.splitlines():
            print("  err| %s" % line)

    def allow(what, args, needle=None, stdin_file=None):
        rc, out, err = run(args, stdin_file)
        allows = [ln for ln in out.splitlines() if ln.startswith("mp2s-guard: allow: ")]
        if rc == 0 and len(allows) == 1 and err == "" and (needle is None or needle in out):
            passed.append(what)
        else:
            report(what, "exit 0, one allow line matching %s, empty stderr"
                   % (needle if needle is not None else "anything",), rc, out, err)

    def refuse(what, line, args, stdin_file=None):
        rc, out, err = run(args, stdin_file)
        if rc == 3 and out == "" and err == line + "\n":
            passed.append(what)
        else:
            report(what, "exit 3 and exactly the pinned line on stderr, empty stdout", rc, out, err)

    def error(what, needles, args, stdin_file=None):
        rc, out, err = run(args, stdin_file)
        if (rc == 2 and out == "" and err != "" and "REFUSE" not in err
                and all(n in err for n in needles)):
            passed.append(what)
        else:
            report(what, "exit 2, empty stdout, no REFUSE line, a line naming each needle", rc, out, err)

    def fix(name, *vars_):
        return st_pe(os.path.join(t, name), *vars_)

    pe_clean = fix("pe-clean.txt", *ST_CLEAN)
    pe_mp2s2 = fix("pe-mp2s2.txt", *ST_CLEAN,
                   "mp2s_ver=1", "mp2s_left=2", "mp2s_prefixes=/mmc0_ /mmc1_",
                   "boot_prefixes=/yakogl/standalone/ /mmc0_ /mmc1_")
    pe_forever = fix("pe-forever.txt", *ST_CLEAN,
                     "mp2s_ver=1", "mp2s_left=forever", "mp2s_prefixes=/mmc0_ /mmc1_",
                     "boot_prefixes=/yakogl/standalone/ /mmc0_ /mmc1_")
    pe_stray = fix("pe-stray.txt", *ST_CLEAN, "mp2s_prefixes=/mmc0_ /mmc1_")
    pe_yako = fix("pe-yako.txt", *ST_CLEAN, "yako_once=tok123")
    pe_mp2z = fix("pe-mp2z.txt", *ST_CLEAN, "mp2z_token=deadbeef")
    pe_dup = fix("pe-dup.txt", *ST_CLEAN, "boot_prefixes=/yakogl/standalone/ /yakogl/standalone/ /mmc0_")
    pe_combo = fix("pe-combo.txt", *ST_CLEAN, "yako_once=tok123",
                   "boot_prefixes=/yakogl/standalone/ /yakogl/standalone/ /mmc0_")
    pe_garbage = fix("pe-garbage.txt", '## Error: "bootcmd" not defined', "hello world")

    img_clean = st_envbin(os.path.join(t, "img-clean.bin"), 0x200, "s",
                          "0|" + "|".join(ST_CLEAN))
    img_red_stale = st_envbin(os.path.join(t, "img-red-stale.bin"), 0x200, "r",
                              "0|" + "|".join(ST_CLEAN)
                              + "|mp2s_ver=1|mp2s_left=2|mp2s_prefixes=/mmc0_ /mmc1_",
                              "1|" + "|".join(ST_CLEAN))
    img_red_bad2 = st_corrupt(
        st_envbin(os.path.join(t, "img-red-bad2.bin"), 0x200, "r",
                  "0|" + "|".join(ST_CLEAN), "1|" + "|".join(ST_CLEAN)), 0x210)
    img_badcrc = st_corrupt(
        st_envbin(os.path.join(t, "img-badcrc.bin"), 0x200, "s",
                  "0|" + "|".join(ST_CLEAN)), 0x10)
    img_writer = os.path.join(t, "img-writer.bin")
    p = subprocess.run([sys.executable, "-I", os.path.join(HERE, "uboot_env.py"),
                        "set", img_clean, img_writer, "mp2s_left=2"],
                       stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if p.returncode != 0:
        print("FAIL the uboot_env.py set fixture failed (rc %d)" % p.returncode)
        for line in p.stderr.decode("utf-8", "backslashreplace").splitlines():
            print("  err| %s" % line)
        tmp.cleanup()
        return 1
    img_dk = st_dklayout(os.path.join(t, "img-dk.bin"))

    # ---- allow: clean, and each route's own state is not the other's --------
    allow("clean printenv text, --for oneshot",
          ["--for", "oneshot", "--printenv", pe_clean],
          needle="no standalone default (no mp2s_* in")
    allow("clean printenv text on stdin, --for standalone",
          ["--for", "standalone"], stdin_file=pe_clean,
          needle="no one-shot state (no yako_*, no mp2z_*, boot_prefixes free of "
                 "/yakogl/standalone/) in stdin")
    allow("stray mp2s_prefixes is not one-shot state (--for standalone)",
          ["--for", "standalone", "--printenv", pe_stray])
    allow("a pending yako_once is not a standalone default (--for oneshot)",
          ["--for", "oneshot", "--printenv", pe_yako], needle="no mp2s_* in")
    allow("a leftover mp2z_token is not a standalone default (--for oneshot)",
          ["--for", "oneshot", "--printenv", pe_mp2z])
    allow("a doubled /yakogl/standalone/ prefix with no mp2s_* is not a default "
          "(--for oneshot)", ["--for", "oneshot", "--printenv", pe_dup])
    allow("clean env image, --for oneshot", ["--for", "oneshot", "--env-image", img_clean])
    allow("clean env image, --for standalone", ["--for", "standalone", "--env-image", img_clean])
    allow("mp2s_left in the image alone is not one-shot state (--for standalone)",
          ["--for", "standalone", "--env-image", img_writer])
    allow("mp2s_left only in the stale copy (flags 0; the active copy, flags 1, is clean)",
          ["--for", "oneshot", "--env-image", img_red_stale, "--redundant", "--size", "0x200"])
    allow("mp2z_load in a DK-layout image is not a standalone default (--for oneshot)",
          ["--for", "oneshot", "--env-image", img_dk, "--redundant",
           "--size", "0x2000", "--offset", "0x7e000", "--offset2", "0x7c000"])

    # ---- REFUSE: the pinned messages ----------------------------------------
    refuse("oneshot refuses an armed default (mp2s_left=2)",
           "mp2s-guard: REFUSE: a standalone default is set (mp2s_left=2): "
           "run tools/board-stm32mp2-standalone.sh nodefault first",
           ["--for", "oneshot", "--printenv", pe_mp2s2])
    refuse("oneshot refuses the same default read on stdin",
           "mp2s-guard: REFUSE: a standalone default is set (mp2s_left=2): "
           "run tools/board-stm32mp2-standalone.sh nodefault first",
           ["--for", "oneshot"], stdin_file=pe_mp2s2)
    refuse("oneshot refuses mp2s_left=forever",
           "mp2s-guard: REFUSE: a standalone default is set (mp2s_left=forever): "
           "run tools/board-stm32mp2-standalone.sh nodefault first",
           ["--for", "oneshot", "--printenv", pe_forever])
    refuse("oneshot refuses a stray mp2s_* without mp2s_left",
           "mp2s-guard: REFUSE: standalone variables are set "
           "(mp2s_prefixes=/mmc0_ /mmc1_): run tools/board-stm32mp2-standalone.sh "
           "nodefault first",
           ["--for", "oneshot", "--printenv", pe_stray])
    refuse("standalone refuses yako_once",
           "mp2s-guard: REFUSE: a one-shot is set (yako_once=tok123): run "
           "tools/board-stm32mp2.sh disarm (yako_once) or uninstall (the hook's "
           "variables)",
           ["--for", "standalone", "--printenv", pe_yako])
    refuse("standalone refuses mp2z_token",
           "mp2s-guard: REFUSE: a one-shot is set (mp2z_token=deadbeef): undo the "
           "typed one-shot first (stm32mp257f-dk-uboot-boot.md section 7)",
           ["--for", "standalone", "--printenv", pe_mp2z])
    refuse("standalone refuses a doubled /yakogl/standalone/ prefix with no mp2s_* left",
           "mp2s-guard: REFUSE: boot_prefixes already contains /yakogl/standalone/ "
           "(boot_prefixes=/yakogl/standalone/ /yakogl/standalone/ /mmc0_): run "
           "tools/board-stm32mp2-standalone.sh nodefault first",
           ["--for", "standalone", "--printenv", pe_dup])
    refuse("standalone refuses yako_once and the doubled prefix together",
           "mp2s-guard: REFUSE: a one-shot is set (yako_once=tok123): run "
           "tools/board-stm32mp2.sh disarm (yako_once) or uninstall (the hook's "
           "variables); boot_prefixes already contains /yakogl/standalone/ "
           "(boot_prefixes=/yakogl/standalone/ /yakogl/standalone/ /mmc0_): run "
           "tools/board-stm32mp2-standalone.sh nodefault first",
           ["--for", "standalone", "--printenv", pe_combo])
    refuse("oneshot refuses an env image the uboot_env.py writer armed (mp2s_left=2)",
           "mp2s-guard: REFUSE: a standalone default is set (mp2s_left=2): "
           "run tools/board-stm32mp2-standalone.sh nodefault first",
           ["--for", "oneshot", "--env-image", img_writer])
    refuse("standalone refuses mp2z_load in the active copy of a DK-layout image",
           "mp2s-guard: REFUSE: a one-shot is set (mp2z_load=0xa0000000 "
           "mp2z_window=0x10000000): undo the typed one-shot first "
           "(stm32mp257f-dk-uboot-boot.md section 7)",
           ["--for", "standalone", "--env-image", img_dk, "--redundant",
            "--size", "0x2000", "--offset", "0x7e000", "--offset2", "0x7c000"])

    # ---- error (exit 2): a refusal the caller must also treat as a refusal --
    error("bad CRC in the single copy", ["bad CRC"],
          ["--for", "oneshot", "--env-image", img_badcrc])
    error("bad CRC in the copy that is not active names copy 2", ["copy 2", "bad CRC"],
          ["--for", "oneshot", "--env-image", img_red_bad2, "--redundant", "--size", "0x200"])
    error("missing --printenv file", ["not a readable file"],
          ["--for", "oneshot", "--printenv", os.path.join(t, "does-not-exist.txt")])
    error("missing --env-image file", ["not a readable file"],
          ["--for", "standalone", "--env-image", os.path.join(t, "does-not-exist.bin")])
    error("printenv text with no NAME=VALUE line", ["unreadable input"],
          ["--for", "oneshot", "--printenv", pe_garbage])
    error("empty stdin", ["unreadable input"], ["--for", "standalone"])
    error("--for missing", ["required: --for"], ["--printenv", pe_clean])
    error("--printenv and --env-image at once", ["not allowed with argument --printenv"],
          ["--for", "oneshot", "--printenv", pe_clean, "--env-image", img_clean])
    error("a layout flag used with --printenv", ["go with --env-image"],
          ["--for", "oneshot", "--printenv", pe_clean, "--redundant"])

    tmp.cleanup()
    print("mp2s_guard.py --self-test: %d checks, %d failed (pinned: %d)"
          % (len(passed), len(failed), WANT_CHECKS))
    return 0 if not failed and len(passed) == WANT_CHECKS else 1


def main(argv=None):
    ap = argparse.ArgumentParser(
        prog="tools/mp2s_guard.py",
        description="Read-only guard for the STM32MP257F-DK: a one-shot is refused "
        "while a standalone default is set, and the reverse "
        "(docs/notes/stm32mp257f-dk-standalone.md, section 7); see the source's "
        "docstring for the modes, inputs and exit codes.",
    )
    ap.add_argument("--for", dest="mode", choices=("oneshot", "standalone"),
                    help="oneshot: the check the one-shot runners make before they "
                    "arm or reboot; standalone: the check the standalone runner's "
                    "default makes before its environment write")
    ap.add_argument("--self-test", action="store_true",
                    help="run the 30 pinned verdict checks of this tool and exit "
                    "(0 when all pass, 1 otherwise)")
    src = ap.add_mutually_exclusive_group()
    src.add_argument("--printenv", metavar="FILE", help="fw_printenv output in FILE")
    src.add_argument("--env-image", metavar="FILE",
                     help="an environment image, read with the functions of tools/uboot_env.py")
    try:
        ue = load_uboot_env()
    except Error as err:
        print("mp2s-guard: error: %s" % err, file=sys.stderr)
        return 2
    ue.layout_args(ap, False)
    a = ap.parse_args(argv)
    if a.self_test:
        return self_test()
    if a.mode is None:
        ap.error("the following arguments are required: --for")
    if a.env_image is None and (a.size is not None or a.offset != 0 or a.offset2 is not None
                                or a.redundant or a.second is not None):
        ap.error("--size/--offset/--offset2/--redundant/--second go with --env-image")
    try:
        if a.env_image is not None:
            pairs, src_name = parse_env_image(a.env_image, a, ue), a.env_image
        elif a.printenv is not None:
            pairs, src_name = parse_printenv(read_bytes(a.printenv), a.printenv), a.printenv
        else:
            pairs, src_name = parse_printenv(sys.stdin.buffer.read(), "stdin"), "stdin"
        if a.mode == "oneshot":
            rc, msg = verdict_oneshot(pairs, src_name)
        else:
            rc, msg = verdict_standalone(pairs, src_name)
    except (Error, ue.Refused, OSError) as err:
        print("mp2s-guard: error: %s" % err, file=sys.stderr)
        return 2
    print("mp2s-guard: %s" % msg, file=None if rc == 0 else sys.stderr)
    return rc


if __name__ == "__main__":
    sys.exit(main())
