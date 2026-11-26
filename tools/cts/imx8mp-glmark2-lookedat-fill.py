#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Fill board values into one scene's 'What was looked at' entry of the
i.MX8MP by-eye note.

    tools/cts/imx8mp-glmark2-lookedat-fill.py --note PATH
        --scene gmNN|NN|'<exact scene cell>' --set TOKEN=VALUE ... [--dry-run]

TOKEN is one of verdict, route, commit, record, crc32, picdiff (written
without the angle brackets); every occurrence of `<TOKEN>` in that scene's
`- **[` entry line is replaced, nothing else in the file changes (the lead
line names the tokens too and is never touched). The scene is resolved as
imx8mp-glmark2-note-fill.py does. Refuses (exit 1, nothing written) an
unknown token, a token not present in the entry, a VALUE containing `<`, `>`,
a newline or `|`, a VALUE with an IPv4 address, /dev/tty, ttyUSB, ttyACM,
*.ts.net or password. Prints the tokens still unfilled in the entry.
--dry-run prints the new line only. Exit 2: usage error.
"""
import argparse
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TOKENS = ("verdict", "route", "commit", "record", "crc32", "picdiff")
C5 = re.compile(r"([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|ttyUSB|ttyACM|ts\.net|password",
                re.I)
LOOKED = "What was looked at"


class Refuse(Exception):
    pass


def load_resolver():
    spec = importlib.util.spec_from_file_location(
        "note_fill", os.path.join(HERE, "imx8mp-glmark2-note-fill.py"))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def fill(a, nf):
    sets = {}
    for s in a.set:
        tok, eq, val = s.partition("=")
        if not eq:
            raise Refuse(f"--set {s!r} is not TOKEN=VALUE")
        if tok not in TOKENS:
            raise Refuse(f"unknown token {tok!r} (one of {', '.join(TOKENS)})")
        if re.search(r"[<>|\r\n]", val):
            raise Refuse(f"value of {tok} contains <, >, | or a newline")
        if C5.search(val):
            raise Refuse(f"value of {tok} looks like an address or credential")
        if tok in sets:
            raise Refuse(f"token {tok!r} given twice")
        sets[tok] = val
    try:
        cell = nf.resolve(a.scene)
    except nf.Refuse as e:
        raise Refuse(str(e))
    with open(a.note, "rb") as f:
        lines = f.read().decode().splitlines(keepends=True)
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
    if a.dry_run:
        sys.stdout.write(new if new.endswith("\n") else new + "\n")
    else:
        lines[i] = new
        with open(a.note, "wb") as f:
            f.write("".join(lines).encode())
        print(f"filled: {cell}")
    print("unfilled: " + (", ".join(left) if left else "none"), file=sys.stderr
          if a.dry_run else sys.stdout)


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
