#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Fill board values into one scene's 'What was looked at' entry of the
i.MX8MP by-eye note.

    tools/cts/imx8mp-glmark2-lookedat-fill.py --note PATH
        --scene gmNN|NN|'<exact scene cell>' --set TOKEN=VALUE ... [--dry-run]

TOKEN is one of verdict, route, commit, record, crc32, picdiff (written
without the angle brackets); every occurrence of `<TOKEN>` after the scene
cell of that scene's `- **[` entry line is replaced with VALUE, and nothing
else in the file changes: the lead line above the entries names the tokens
too and is never touched, and the scene cell in the bold stays byte for
byte. The scene is resolved as imx8mp-glmark2-note-fill.py's resolver does
(gmNN or the exact cell), and a plain number NN resolves to gmNN (so 2, 02
and gm2 all name gm02). Refuses (exit 1, message on stderr, nothing
written): a note that cannot be read, an unknown scene, an unknown token,
a token given twice, an empty VALUE, a token not present in that entry, a
VALUE containing `<`, `>`, a newline or `|`, a VALUE matching the c5 pattern
of imx8mp-glmark2-accept.sh (an IPv4 address, /dev/tty, ttyUSB, ttyACM, a
.ts.net name or a password; case-insensitive, as imx8mp-glmark2-wrongscene.py
carries the same pattern). Prints the tokens of the six still unfilled in
the entry after the fill. --dry-run writes nothing and prints the new line
only, on stdout (the unfilled list then goes to stderr). Exit 2: usage
error.
"""
import argparse
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TOKENS = ("verdict", "route", "commit", "record", "crc32", "picdiff")
# The c5 classes (imx8mp-glmark2-accept.sh, and the same pattern as
# imx8mp-glmark2-wrongscene.py's REDACT), matched case-insensitively.
C5 = re.compile(r"([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|ttyUSB|ttyACM|ts\.net|password",
                re.I)
LOOKED = "What was looked at"


class Refuse(Exception):
    pass


def load_resolver():
    """imx8mp-glmark2-note-fill.py as a module: its scene resolver."""
    sys.dont_write_bytecode = True  # no __pycache__ beside the tools
    spec = importlib.util.spec_from_file_location(
        "note_fill", os.path.join(HERE, "imx8mp-glmark2-note-fill.py"))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def resolve(nf, scene):
    """note-fill.py's resolver, plus a plain NN: 2, 02 and gm2 all mean gm02."""
    try:
        return nf.resolve(scene)
    except nf.Refuse:
        pass
    m = re.fullmatch(r"(?:gm)?([0-9]{1,2})", scene)
    if m and 1 <= int(m.group(1)) <= 33:
        try:
            return nf.resolve("gm%02d" % int(m.group(1)))
        except nf.Refuse:
            pass
    raise Refuse(f"unknown scene: {scene!r}")


def fill(a, nf):
    sets = {}
    for s in a.set:
        tok, eq, val = s.partition("=")
        if not eq:
            raise Refuse(f"--set {s!r} is not TOKEN=VALUE")
        if tok not in TOKENS:
            raise Refuse(f"unknown token {tok!r} (one of {', '.join(TOKENS)})")
        if not val:
            raise Refuse(f"value of {tok} is empty")
        if re.search(r"[<>|\r\n]", val):
            hint = (" (picdiff prints '|diff|'; write 'abs diff' for it,"
                    " verdicts.py splits rows on every '|')"
                    if "|" in val else "")
            raise Refuse(f"value of {tok} contains <, >, | or a newline{hint}")
        if C5.search(val):
            raise Refuse(f"value of {tok} looks like an address or credential")
        if tok in sets:
            raise Refuse(f"token {tok!r} given twice")
        sets[tok] = val
    cell = resolve(nf, a.scene)
    try:
        with open(a.note, "rb") as f:
            lines = f.read().decode().splitlines(keepends=True)
    except OSError as e:
        raise Refuse(f"note {a.note!r}: {e.strerror or e}")
    sect, hit = None, []
    for i, line in enumerate(lines):
        if line.startswith("## "):
            sect = line[3:].strip()
        elif sect == LOOKED and line.startswith("- **["):
            m = re.match(r"- \*\*(\[.*?)\*\*", line)
            if m and m.group(1).strip() == cell:
                hit.append(i)
    if len(hit) != 1:
        raise Refuse(f"scene {cell!r}: {len(hit)} entries under '{LOOKED}', need exactly 1")
    i = hit[0]
    line = lines[i]
    bold_end = re.match(r"- \*\*\[.*?\*\*", line).end()
    head, tail = line[:bold_end], line[bold_end:]
    for tok, val in sets.items():
        if f"<{tok}>" not in tail:
            raise Refuse(f"token <{tok}> is not in the entry of {cell!r}")
        tail = tail.replace(f"<{tok}>", val)
    new = head + tail
    left = [t for t in TOKENS if f"<{t}>" in tail]
    unfilled = "unfilled: " + (", ".join(left) if left else "none")
    if a.dry_run:
        sys.stdout.write(new if new.endswith("\n") else new + "\n")
        print(unfilled, file=sys.stderr)
    else:
        lines[i] = new
        with open(a.note, "wb") as f:
            f.write("".join(lines).encode())
        print(f"filled: {cell}")
        print(unfilled)


class Parser(argparse.ArgumentParser):
    def error(self, msg):
        self.print_usage(sys.stderr)
        print(f"lookedat-fill: {msg}", file=sys.stderr)
        sys.exit(2)


def main(argv):
    p = Parser(description=__doc__.split("\n\n")[0])
    p.add_argument("--note", required=True)
    p.add_argument("--scene", required=True)
    p.add_argument("--set", action="append", default=[], required=True)
    p.add_argument("--dry-run", action="store_true")
    a = p.parse_args(argv)
    try:
        fill(a, load_resolver())
    except Refuse as e:
        print(f"lookedat-fill: refused: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
