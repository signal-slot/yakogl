#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Fill one row of the verdict table of the i.MX8MP by-eye note.

    tools/cts/imx8mp-glmark2-note-fill.py --note PATH --scene SCENE|gmNN
        --commit SHA --record records/imx8mp/YYYYMMDD-HHMM-<name>-<sha8>/
        --route dump|photograph|both
        --verdict right|wrong|'seen without a value' [--check-main]

Replaces exactly the row of SCENE under '## Verdicts' in place; every other
byte of the file stays. The commit cell is the first 8 hex digits (lowercase).
Refuses (exit 1, message on stderr) an unknown scene, a commit that is not 7
to 40 hex digits, a record path outside the full pattern, a record sha8 that
is not a prefix of the commit, a route or verdict outside the allowed values,
photograph with anything but `seen without a value` and dump/both with it
(the rules of imx8mp-glmark2-verdicts.py). --check-main also requires
`git merge-base --is-ancestor <commit> origin/main`.
"""
import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
DUMPS = os.path.join(HERE, "imx8mp-glmark2-dumps.sh")
ROUTES = ("dump", "photograph", "both")
VERDICTS = ("right", "wrong", "seen without a value")
HEX = re.compile(r"[0-9a-fA-F]{7,40}")
RECORD = re.compile(r"records/imx8mp/[0-9]{8}-[0-9]{4}-[^ |/]+-([0-9a-f]{8})/?")


class Refuse(Exception):
    pass


def scene_list():
    """(label, scene cell) pairs, as `imx8mp-glmark2-dumps.sh scenes` lists them."""
    out = subprocess.run(["bash", DUMPS, "scenes"], capture_output=True,
                         text=True, check=True).stdout
    res = []
    for line in out.splitlines():
        if not line.strip():
            continue
        label, _, rest = line.partition(" ")
        res.append((label, rest.rpartition(" -b ")[0] or rest))
    return res


def resolve(scene):
    for label, cell in scene_list():
        if scene in (label, label.split("-")[0], cell):
            return cell
    raise Refuse(f"unknown scene: {scene!r}")


def validate(a):
    if not HEX.fullmatch(a.commit):
        raise Refuse(f"commit {a.commit!r} is not 7 to 40 hex digits")
    commit = a.commit.lower()
    m = RECORD.fullmatch(a.record)
    if not m:
        raise Refuse(f"record {a.record!r} does not match "
                     "records/imx8mp/YYYYMMDD-HHMM-<name>-<sha8>/")
    if not commit.startswith(m.group(1)):
        raise Refuse(f"record sha8 {m.group(1)} is not a prefix of commit {commit}")
    if a.route not in ROUTES:
        raise Refuse(f"route {a.route!r} not one of {', '.join(ROUTES)}")
    if a.verdict not in VERDICTS:
        raise Refuse(f"verdict {a.verdict!r} not one of {', '.join(VERDICTS)}")
    if (a.route == "photograph") != (a.verdict == "seen without a value"):
        raise Refuse("photograph goes only with `seen without a value`, "
                     "dump and both never do")
    if a.check_main:
        r = subprocess.run(["git", "-C", ROOT, "merge-base", "--is-ancestor",
                            commit, "origin/main"], capture_output=True)
        if r.returncode != 0:
            raise Refuse(f"commit {commit} is not an ancestor of origin/main")
    return commit


def fill(a):
    cell = resolve(a.scene)
    commit = validate(a)
    with open(a.note, "rb") as f:
        lines = f.read().decode().splitlines(keepends=True)
    sect, hit = None, []
    for i, line in enumerate(lines):
        if line.startswith("## "):
            sect = line[3:].strip()
        elif sect == "Verdicts" and line.startswith("| ["):
            if line.strip().strip("|").split("|")[0].strip() == cell:
                hit.append(i)
    if len(hit) != 1:
        raise Refuse(f"scene {cell!r}: {len(hit)} rows in the note, need exactly 1")
    i = hit[0]
    eol = lines[i][len(lines[i].rstrip("\r\n")):]
    lines[i] = (f"| {cell} | {commit[:8]} | {a.record} | {a.route} | "
                f"{a.verdict} |{eol}")
    with open(a.note, "wb") as f:
        f.write("".join(lines).encode())
    print(f"filled: {cell}")


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    for k in ("note", "scene", "commit", "record", "route", "verdict"):
        p.add_argument("--" + k, required=True)
    p.add_argument("--check-main", action="store_true")
    try:
        a = p.parse_args(argv)
        fill(a)
    except Refuse as e:
        print(f"note-fill: refused: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
