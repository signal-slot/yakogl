#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Read-only guard: refuse a one-shot while a standalone default is set, and the reverse.

    tools/mp2s_guard.py --for oneshot    [--printenv FILE | --env-image FILE] [LAYOUT]
    tools/mp2s_guard.py --for standalone [--printenv FILE | --env-image FILE] [LAYOUT]

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

  Exit codes; a caller must treat every non-zero as a refusal:
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
import sys

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


def main(argv=None):
    ap = argparse.ArgumentParser(
        prog="tools/mp2s_guard.py",
        description="Read-only guard for the STM32MP257F-DK: a one-shot is refused "
        "while a standalone default is set, and the reverse "
        "(docs/notes/stm32mp257f-dk-standalone.md, section 7); see the source's "
        "docstring for the modes, inputs and exit codes.",
    )
    ap.add_argument("--for", dest="mode", required=True, choices=("oneshot", "standalone"),
                    help="oneshot: the check the one-shot runners make before they "
                    "arm or reboot; standalone: the check the standalone runner's "
                    "default makes before its environment write")
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
