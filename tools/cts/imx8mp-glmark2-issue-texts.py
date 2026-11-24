#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Issue comments and one-line changes from the filled i.MX8MP by-eye note.

    tools/cts/imx8mp-glmark2-issue-texts.py --note PATH

Parses the verdict table of the note and prints six labelled blocks on stdout:
the comment for #17, the comment for #1196, and the one-line changes for
RESULTS.md ("Common set"), STATUS.md (Japanese), docs/notes/stage2-bench-gap.md
and ROADMAP.md stage 2 ("Pictures by eye", i.MX8MP part). Refuses (exit 1,
message on stderr) a note with other than 33 rows or any row that still reads
`not yet looked at` or whose verdict, route, commit or record is malformed.
Writes no file. The texts are handed to the plan that owns those files.
"""
import argparse
import re
import sys

TOTAL = 33
VERDICTS = ("right", "wrong", "seen without a value")
ROUTES = ("dump", "photograph", "both")
RECORD = re.compile(r"records/imx8mp/([0-9]{8})-([0-9]{4})-[^ |/]+-[0-9a-f]{8}/?")
SHA = re.compile(r"[0-9a-f]{7,40}")
NOTE = "docs/notes/imx8mp-glmark2-pictures-by-eye.md"


def die(msg):
    print(f"imx8mp-glmark2-issue-texts: {msg}", file=sys.stderr)
    sys.exit(1)


def section(text, name):
    m = re.search(r"^## " + re.escape(name) + r"\s*$", text, re.M)
    if not m:
        return ""
    rest = text[m.end():]
    n = re.search(r"^## ", rest, re.M)
    return rest[:n.start()] if n else rest


def parse_rows(text):
    rows = []
    for line in section(text, "Verdicts").splitlines():
        line = line.strip()
        if not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if len(cells) != 5 or cells[0] == "scene" or set(cells[0]) <= set("-"):
            continue
        rows.append(cells)
    return rows


def check(rows):
    if len(rows) != TOTAL:
        die(f"{len(rows)} verdict rows, {TOTAL} needed")
    for scene, commit, record, route, verdict in rows:
        if "not yet looked at" in verdict:
            die(f"not complete: {scene!r} reads 'not yet looked at'")
        if verdict not in VERDICTS:
            die(f"{scene!r}: verdict {verdict!r} is none of {', '.join(VERDICTS)}")
        if route not in ROUTES:
            die(f"{scene!r}: route {route!r} is none of {', '.join(ROUTES)}")
        if not SHA.fullmatch(commit):
            die(f"{scene!r}: commit {commit!r} is not hex")
        if not RECORD.fullmatch(record):
            die(f"{scene!r}: record {record!r} is not a full records/imx8mp/ path")


def wrong_titles(text, wrong):
    """Issue title of each wrong scene: the `**Title**:` lines of 'Wrong scenes'."""
    titles = []
    for m in re.finditer(r"\*\*Title\*\*:\s*(.+)", section(text, "Wrong scenes")):
        t = m.group(1).strip().strip("`").strip()
        if not t.startswith("<scene>"):
            titles.append(t)
    res = []
    for scene in wrong:
        key = scene.split("]")[0].strip("[").strip()
        hit = next((t for t in titles if t.startswith("[" + key + "]") or t.startswith(key)), None)
        res.append((scene, hit or "(issue title not in the note)"))
    return res


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--note", required=True)
    a = ap.parse_args()
    try:
        with open(a.note, encoding="utf-8") as f:
            text = f.read()
    except OSError as e:
        die(f"cannot read the note: {e.strerror}")
    rows = parse_rows(text)
    check(rows)
    right = sum(r[4] == "right" for r in rows)
    wrong = [r[0] for r in rows if r[4] == "wrong"]
    seen = sum(r[4] == "seen without a value" for r in rows)
    commits = sorted({r[1][:8] for r in rows})
    stamps = sorted("%s-%s" % RECORD.fullmatch(r[2]).groups() for r in rows)
    dates = sorted({s[:8] for s in stamps})
    if len(dates) == 1:
        span = f"{dates[0]} ({stamps[0][9:]} to {stamps[-1][9:]})"
    else:
        span = f"{dates[0]} to {dates[-1]}"
    routes = ", ".join(sorted({r[3] for r in rows}))
    counts = f"{right} right, {len(wrong)} wrong, {seen} seen without a value, of {TOTAL}"
    wl = wrong_titles(text, wrong)
    if wl:
        wtxt = "wrong scenes: " + "; ".join(f'{s} -> issue "{t}"' for s, t in wl)
    else:
        wtxt = "no wrong scene"
    where = (f"origin/main commit(s) {', '.join(commits)}; records "
             f"`records/imx8mp/<YYYYMMDD-HHMM>-<name>-<sha8>/` of {span} "
             f"(kept on the session machine, gitignored); route(s): {routes}")

    print("== 1. Comment for issue #17 ==")
    print("i.MX8MP (Verdin iMX8M Plus, GC7000UL) glmark2-es2 pictures looked at by "
          f"eye: {counts}. {wtxt}. Built from {where}. Method and per-scene "
          f"table: `{NOTE}`.")
    print()
    print("== 2. Comment for issue #1196 ==")
    print("The i.MX8MP part of 'Pictures by eye' (glmark2-es2, 33 scenes) is "
          f"done: {counts}. {wtxt}. Compared with the llvmpipe reference "
          "(`tools/cts/glmark2-host-ref.sh`, `tools/cts/glmark2-picdiff.py`). "
          f"{where[0].upper() + where[1:]}. Verdicts and records per scene: `{NOTE}`.")
    print()
    print("== 3. RESULTS.md 'Common set', i.MX8MP row: replace '(not looked at by eye)' ==")
    print(f"looked at by eye ({counts}; {wtxt}; `{NOTE}`)")
    print()
    print("== 4. STATUS.md (Japanese): replace '絵は Pi 4 と i.MX8MP をまだ目で見ていない（カメラ無し）' ==")
    jw = "間違い 0" if not wrong else "間違いのシーン: " + ", ".join(wrong)
    print(f"i.MX8MP の絵は 33 シーンを目で見た（正しい {right}、間違い {len(wrong)}、"
          f"値なしで見た {seen}。{jw}。{NOTE}）。Pi 4 は docs/notes/pi4-glmark2-pictures-by-eye.md")
    print()
    print("== 5. docs/notes/stage2-bench-gap.md: row 'every scene's picture checked', i.MX8MP cell ==")
    print(f"by value, 33 of 33 (column `imx8mp`); by eye: {counts} (`{NOTE}`)")
    print()
    print("== 6. ROADMAP.md stage 2, 'Pictures by eye', i.MX8MP part ==")
    print(f"The i.MX8MP pictures were looked at by eye ({counts}; {wtxt}; "
          f"`{NOTE}`, #17, #1196).")


if __name__ == "__main__":
    main()
