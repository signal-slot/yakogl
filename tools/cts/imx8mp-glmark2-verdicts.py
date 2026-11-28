#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check the verdict table of docs/notes/imx8mp-glmark2-pictures-by-eye.md.

    tools/cts/imx8mp-glmark2-verdicts.py [NOTE] [--complete] [--scenes FILE]
    tools/cts/imx8mp-glmark2-verdicts.py --self-test

NOTE defaults to the by-eye note. The '## Verdicts' table has the columns
`| scene | commit | record | route | verdict |`, one row per scene. Rules:

  (a) exactly 33 rows;
  (b) the scene cells are the scenes of `tools/cts/imx8mp-glmark2-dumps.sh
      scenes` (or of FILE, same line format), each once;
  (c) a row whose verdict is not `not yet looked at` has an 8-lowercase-hex
      commit, a record `records/imx8mp/YYYYMMDD-HHMM-<name>-<sha8>/`
      (sha8: 8 lowercase hex digits, the image's sha256 prefix as
      tools/board.sh names the record; it is not compared with the commit), a
      route of dump, photograph, both or photograph+probes
      (a camera photograph judged by content plus the same boot's
      `glmark2-picture:` value line), and a verdict of right, wrong or `seen
      without a value`; photograph goes only with `seen without a value`, dump,
      both and photograph+probes never do; a `wrong` row has an
      entry (its scene text) under '## Wrong scenes' if that heading exists;
  (d) every `- **[scene]**` bullet under '## What was looked at' names a scene
      of the table, at most once;
  (e) with --complete every row is filled in: no `not yet looked at`, no `-`,
      and acceptance criterion c2's grep counts 33; every scene of the table
      has exactly one bullet under '## What was looked at'; and the note holds
      none of the skeleton's placeholder texts (PLACEHOLDERS).

One line per problem, exit 1 if any; otherwise `ok: N rows, M filled`, exit 0.
"""
import io
import os
import re
import subprocess
import sys
from contextlib import redirect_stdout

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
NOTE = os.path.join(ROOT, "docs/notes/imx8mp-glmark2-pictures-by-eye.md")
DUMPS = os.path.join(HERE, "imx8mp-glmark2-dumps.sh")
TESTDATA = os.path.join(HERE, "testdata/imx8mp-glmark2-verdicts")
NROWS = 33
TODO = "not yet looked at"
VERDICTS = ("right", "wrong", "seen without a value", TODO)
ROUTES = ("dump", "photograph", "both", "photograph+probes")
RECORD = re.compile(r"records/imx8mp/[0-9]{8}-[0-9]{4}-[^ |]+-([0-9a-f]{8})/?")
LOOKED = "What was looked at"
BULLET = re.compile(r"^- \*\*(\[.*?)\*\*")
PLACEHOLDERS = ("Not yet established", "To fill in", "Skeleton: no scene")
C2 = re.compile(r"^\| \[[a-z0-9]+\] .*\| [0-9a-f]{8} \| "
                r"records/imx8mp/[0-9]{8}-[0-9]{4}-[^ |]+-[0-9a-f]{8}/? \|")


def scenes_from_lines(text):
    """`gm01-build [build] x -b build:x` lines -> the scene cells."""
    out = []
    for line in text.splitlines():
        if not line.strip():
            continue
        rest = line.split(" ", 1)[1] if " " in line else line
        out.append(rest.rpartition(" -b ")[0] or rest)
    return out


def read_rows(text):
    """The table rows under '## Verdicts' (line, cells), the Wrong scenes text,
    and the scenes of the '- **[' bullets under '## What was looked at' (None
    if there is no such section)."""
    rows, wrong, looked, sect = [], None, None, None
    for line in text.splitlines():
        if line.startswith("## "):
            sect = line[3:].strip()
            if sect == "Wrong scenes":
                wrong = ""
            elif sect == LOOKED and looked is None:
                looked = []
            continue
        if sect == "Wrong scenes":
            wrong += line + "\n"
        elif sect == "Verdicts" and line.startswith("| ["):
            rows.append((line, [c.strip() for c in line.strip().strip("|").split("|")]))
        elif sect == LOOKED and line.startswith("- **["):
            m = BULLET.match(line)
            looked.append(m.group(1).strip() if m else line[4:].strip())
    return rows, wrong, looked


def check(text, scenes, complete):
    """Return (problems, nrows, nfilled)."""
    rows, wrong, looked = read_rows(text)
    probs = []
    if len(rows) != NROWS:
        probs.append(f"rows: {len(rows)}, want {NROWS}")
    cells_of, filled, c2 = [], 0, 0
    for line, c in rows:
        if len(c) != 5:
            probs.append(f"{c[0]}: {len(c)} cells, want 5")
            continue
        scene, commit, record, route, verdict = c
        cells_of.append(scene)
        c2 += bool(C2.match(line))
        if verdict not in VERDICTS:
            probs.append(f"{scene}: verdict '{verdict}' is not one of "
                         + ", ".join(VERDICTS))
            continue
        if complete and (verdict == TODO or "-" in (commit, record, route, verdict)):
            probs.append(f"{scene}: not filled in (--complete)")
            continue
        if verdict == TODO:
            continue
        filled += 1
        if not re.fullmatch(r"[0-9a-f]{8}", commit):
            probs.append(f"{scene}: commit '{commit}' is not 8 lowercase hex digits")
        m = RECORD.fullmatch(record)
        if not m:
            probs.append(f"{scene}: record '{record}' is not records/imx8mp/"
                         "<YYYYMMDD-HHMM>-<name>-<sha8>/")
        if route not in ROUTES:
            probs.append(f"{scene}: route '{route}' is not one of " + ", ".join(ROUTES))
        elif route == "photograph" and verdict != "seen without a value":
            probs.append(f"{scene}: route photograph needs 'seen without a value', "
                         f"not '{verdict}'")
        elif route in ("dump", "both", "photograph+probes") and verdict == "seen without a value":
            probs.append(f"{scene}: route {route} has a value, not 'seen without a value'")
        if verdict == "wrong" and wrong is not None and scene not in wrong:
            probs.append(f"{scene}: wrong, but no entry under '## Wrong scenes'")
    seen = set()
    for s in cells_of:
        if s in seen:
            probs.append(f"{s}: scene appears twice")
        seen.add(s)
    for s in scenes:
        if s not in seen:
            probs.append(f"{s}: scene missing from the table")
    for s in sorted(seen - set(scenes)):
        probs.append(f"{s}: scene is not in the dump script's list")
    if complete and c2 != NROWS:
        probs.append(f"c2 grep counts {c2}, want {NROWS}")
    if looked is None:
        if complete:
            probs.append(f"no '## {LOOKED}' section (--complete)")
        looked = []
    count = {}
    for s in looked:
        count[s] = count.get(s, 0) + 1
    for s in count:
        if s not in seen:
            probs.append(f"{s}: '{LOOKED}' entry names no scene of the table")
        elif count[s] > 1:
            probs.append(f"{s}: '{LOOKED}' entry appears {count[s]} times")
    if complete:
        for s in cells_of:
            if s not in count:
                probs.append(f"{s}: no '{LOOKED}' entry (--complete)")
        for p in PLACEHOLDERS:
            for n, line in enumerate(text.splitlines(), 1):
                if p in line:
                    probs.append(f"line {n}: placeholder text '{p}' (--complete)")
    return probs, len(rows), filled


def run(note, scenes_file, complete):
    if scenes_file:
        with open(scenes_file, encoding="utf-8") as f:
            scenes = scenes_from_lines(f.read())
    else:
        r = subprocess.run(["bash", DUMPS, "scenes"], capture_output=True, text=True)
        if r.returncode:
            return [f"dumps.sh scenes failed: {r.stderr.strip()}"], 0, 0
        scenes = scenes_from_lines(r.stdout)
    with open(note, encoding="utf-8") as f:
        return check(f.read(), scenes, complete)


def report(probs, nrows, nfilled):
    for p in probs:
        print(p)
    if probs:
        return 1
    print(f"ok: {nrows} rows, {nfilled} filled")
    return 0


# fixture, --complete, exit status, text expected in the output
CASES = [
    ("skeleton.md", False, 0, "ok: 33 rows, 0 filled"),
    ("skeleton.md", True, 1, "not filled in (--complete)"),
    ("filled.md", True, 0, "ok: 33 rows, 33 filled"),
    ("partial.md", False, 0, "ok: 33 rows, 2 filled"),
    ("bad-rows.md", False, 1, "rows: 32, want 33"),
    ("bad-missing-scene.md", False, 1, "scene missing from the table"),
    ("bad-duplicate.md", False, 1, "scene appears twice"),
    ("bad-commit.md", False, 1, "is not 8 lowercase hex digits"),
    ("bad-record.md", False, 1, "is not records/imx8mp/"),
    ("partial-sha-differs.md", False, 0, "ok: 33 rows, 1 filled"),
    ("bad-sha8.md", False, 1, "is not records/imx8mp/"),
    ("bad-route.md", False, 1, "route 'camera' is not one of"),
    ("bad-verdict.md", False, 1, "verdict 'good' is not one of"),
    ("bad-photograph.md", False, 1, "route photograph needs 'seen without a value'"),
    ("x2-kvm.md", True, 0, "ok: 33 rows, 33 filled"),
    ("partial-probes.md", False, 0, "ok: 33 rows, 2 filled"),
    ("bad-probes-novalue.md", False, 1,
     "route photograph+probes has a value, not 'seen without a value'"),
    ("bad-dump-novalue.md", False, 1, "has a value, not 'seen without a value'"),
    ("bad-wrong-entry.md", False, 1, "wrong, but no entry under '## Wrong scenes'"),
    ("bad-dash.md", True, 1, "not filled in (--complete)"),
    ("skeleton.md", True, 1, "[build] use-vbo=false: no 'What was looked at' entry"),
    ("skeleton.md", True, 1, "placeholder text 'Not yet established' (--complete)"),
    ("partial-looked.md", False, 0, "ok: 33 rows, 2 filled"),
    ("bad-looked-missing.md", True, 1,
     "[shading] shading=phong: no 'What was looked at' entry (--complete)"),
    ("bad-looked-duplicate.md", False, 1,
     "[build] use-vbo=true: 'What was looked at' entry appears 2 times"),
    ("bad-looked-unknown.md", False, 1,
     "[build] use-vbo=maybe: 'What was looked at' entry names no scene of the table"),
    ("bad-looked-placeholder.md", True, 1, "placeholder text 'To fill in' (--complete)"),
    ("bad-looked-nosection.md", True, 1, "no '## What was looked at' section (--complete)"),
]


def self_test():
    scenes = os.path.join(TESTDATA, "scenes.txt")
    bad = 0
    for name, complete, rc, want in CASES:
        buf = io.StringIO()
        with redirect_stdout(buf):
            got = report(*run(os.path.join(TESTDATA, name), scenes, complete))
        out = buf.getvalue()
        good = got == rc and want in out
        print(f"  {'ok  ' if good else 'FAIL'}  {name}{' --complete' if complete else ''}"
              f" -> {got}, {want!r}")
        if not good:
            bad += 1
            print("        output: " + out.replace("\n", "\n                "))
    if bad:
        print(f"== {bad} self-test case(s) FAILED", file=sys.stderr)
        return 1
    print("== imx8mp-glmark2-verdicts self-test: all cases passed")
    return 0


def main(argv):
    complete, scenes, note = False, None, NOTE
    args = list(argv)
    if "--self-test" in args:
        return self_test()
    while args:
        a = args.pop(0)
        if a == "--complete":
            complete = True
        elif a == "--scenes" and args:
            scenes = args.pop(0)
        elif a.startswith("-"):
            print(__doc__, file=sys.stderr)
            return 2
        else:
            note = a
    return report(*run(note, scenes, complete))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
