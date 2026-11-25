#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Print the '## Wrong scenes' entry for a wrong row of the i.MX8MP by-eye note.

    tools/cts/imx8mp-glmark2-wrongscene.py --note PATH --row N
        --record YYYYMMDD-HHMM-<name>-<sha8> --picdiff 'LINE' --what 'TEXT'
        [--route dump|both|photograph+probes]

Reads row N (1-33) of the note's '## Verdicts' table and prints, on stdout,
the entry to paste under '## Wrong scenes': a '### ' heading that is the row's
scene cell character for character (imx8mp-glmark2-verdicts.py matches on
it), the issue title 'i.MX8MP GC7000UL: glmark2 <scene> renders wrong' and
the issue body (board and GPU, commit, full records/imx8mp/<dir>/ path, route,
the picdiff line verbatim, what differs from llvmpipe, and how to reproduce
it with tools/cts/imx8mp-glmark2-dumps.sh and the scene's gmNN label). The
note is never written.

--record takes the record directory name (a leading records/imx8mp/ and a
trailing / are accepted). The commit is the row's commit cell; when that is
still `-`, the record's sha8. The route is the row's route cell; when that is
still `-`, --route, default dump (a picdiff line comes from a decoded dump).

Refuses (exit 1, message on stderr): a record name that is not
YYYYMMDD-HHMM-<name>-<sha8>, a record sha8 that is not the row's commit, a
row outside the table, a route that cannot carry a wrong verdict (photograph
alone; the routes are imx8mp-glmark2-verdicts.py's, whose table parser this
uses too), a scene the dump script does not list, an argument
with a newline, and any argument that matches the redaction pattern (an IP
address, a tty path, a tailnet name, a password).
"""
import argparse
import importlib.util
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DUMPS = os.path.join(HERE, "imx8mp-glmark2-dumps.sh")


def _verdicts():
    """imx8mp-glmark2-verdicts.py as a module: its table parser and routes."""
    spec = importlib.util.spec_from_file_location(
        "imx8mp_glmark2_verdicts", os.path.join(HERE, "imx8mp-glmark2-verdicts.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True  # no __pycache__ in the tree
    spec.loader.exec_module(mod)
    return mod


VERDICTS = _verdicts()
# verdicts.py's routes but photograph alone: that is 'seen without a value'.
ROUTES = tuple(r for r in VERDICTS.ROUTES if r != "photograph")
REDACT = re.compile(
    r"([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|ttyUSB|ttyACM|ts\.net|password")
RECORD = re.compile(r"[0-9]{8}-[0-9]{4}-[^ |/]+-([0-9a-f]{8})")


class Refuse(Exception):
    pass


def label_of(scene):
    """The gmNN-<name> label of a scene cell, from `imx8mp-glmark2-dumps.sh scenes`."""
    r = subprocess.run(["bash", DUMPS, "scenes"], capture_output=True, text=True)
    if r.returncode:
        raise Refuse(f"dumps.sh scenes failed: {r.stderr.strip()}")
    for line in r.stdout.splitlines():
        label, _, rest = line.partition(" ")
        if (rest.rpartition(" -b ")[0] or rest) == scene:
            return label
    raise Refuse(f"scene {scene!r} is not in `imx8mp-glmark2-dumps.sh scenes`")


def entry(a):
    for name, val in vars(a).items():
        val = str(val)
        if REDACT.search(val):
            raise Refuse(f"--{name} matches the redaction pattern; board "
                         "addresses, tty paths and credentials stay in the environment")
        if "\n" in val or "\r" in val:
            raise Refuse(f"--{name} has a newline")
    rec = a.record
    if rec.startswith("records/imx8mp/"):
        rec = rec[len("records/imx8mp/"):]
    rec = rec.rstrip("/")
    m = RECORD.fullmatch(rec)
    if not m:
        raise Refuse(f"record {a.record!r} is not YYYYMMDD-HHMM-<name>-<sha8>")
    sha8 = m.group(1)
    with open(a.note, encoding="utf-8") as f:
        rows = [cells for _, cells in VERDICTS.read_rows(f.read())[0]]
    if not 1 <= a.row <= len(rows):
        raise Refuse(f"row {a.row} is not in the table (1-{len(rows)})")
    c = rows[a.row - 1]
    if len(c) != 5:
        raise Refuse(f"row {a.row} has {len(c)} cells, want 5")
    scene, commit, _, route, _ = c
    if commit == "-":
        commit = sha8
    elif commit != sha8:
        raise Refuse(f"record sha8 {sha8} is not the row's commit {commit}")
    if route == "-":
        route = a.route or "dump"
    if route not in ROUTES:
        raise Refuse(f"route {route!r} is not one of {', '.join(ROUTES)} (a photograph "
                     "alone is 'seen without a value', never wrong)")
    label = label_of(scene)
    nn = label.split("-")[0]
    title = f"i.MX8MP GC7000UL: glmark2 {scene} renders wrong"
    return f"""### {scene}

**Title**: {title}

**Body**:

The glmark2-es2 scene `{scene}` renders wrong on the i.MX8MP (Verdin iMX8M
Plus, Vivante GC7000UL), judged by eye against the upstream glmark2 picture
on Mesa llvmpipe (tools/cts/glmark2-host-ref.sh).

- Board and GPU: Verdin iMX8M Plus (i.MX8MP), Vivante GC7000UL
- Commit: {commit} (origin/main)
- Record: `records/imx8mp/{rec}/`
- Route: {route}
- Scene: {nn} (`{label}`), row {a.row} of
  docs/notes/imx8mp-glmark2-pictures-by-eye.md

tools/cts/glmark2-picdiff.py against the llvmpipe reference:

```
{a.picdiff}
```

What differs from llvmpipe: {a.what}

To reproduce, at commit {commit}, with the environment of
tools/cts/imx8mp-glmark2-dumps.sh (BOARD_UART from the environment):

```
tools/cts/imx8mp-glmark2-dumps.sh build <img-dir> {nn}
tools/cts/imx8mp-glmark2-dumps.sh run <img-dir> <records-dir> {nn}
```

then compare `<record>/{label}.png` with the llvmpipe picture of the scene
(tools/cts/glmark2-host-ref.sh, tools/cts/glmark2-picdiff.py).
"""


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--note", required=True)
    p.add_argument("--row", required=True, type=int)
    p.add_argument("--record", required=True)
    p.add_argument("--picdiff", required=True)
    p.add_argument("--what", required=True)
    p.add_argument("--route", choices=ROUTES)
    a = p.parse_args(argv)
    try:
        sys.stdout.write(entry(a))
    except (Refuse, OSError) as e:
        print(f"imx8mp-glmark2-wrongscene: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
