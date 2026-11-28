#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Issue comments and one-line changes from the filled i.MX8MP by-eye note.

    tools/cts/imx8mp-glmark2-issue-texts.py --note PATH [--board x2|verdin|TEXT]

Parses the verdict table of the note and prints six labelled blocks on stdout:
the comment for #17, the comment for #1196, and the one-line changes for
RESULTS.md ("Common set"), STATUS.md (Japanese), docs/notes/stage2-bench-gap.md
and ROADMAP.md stage 2 ("Pictures by eye", i.MX8MP part). Every generated text
names the board the pictures were measured on: --board takes x2 (the
Armadillo-X2 — the i.MX8MP board since the owner's rule of 2026-11-27 moved the
board work off the away Verdin iMX8M Plus), verdin, or free text naming the
board; without --board it is derived from the note (Armadillo-X2 when the note
measures on it, Verdin iMX8M Plus when the note names only the Verdin,
Armadillo-X2 otherwise, per that rule). The RESULTS.md and STATUS.md lines are
marked as readings of that board. A --board value carrying an address, a tty
path or a credential is refused. Refuses (exit 1, message on stderr) a note
with other than 33 rows or any row that still reads `not yet looked at` or
whose verdict, route, commit or record is malformed. Writes no file. The texts
are handed to the plan that owns those files.
"""
import argparse
import re
import sys

TOTAL = 33
VERDICTS = ("right", "wrong", "seen without a value")
ROUTES = ("dump", "photograph", "photograph+probes", "both")
RECORD = re.compile(r"records/imx8mp/([0-9]{8})-([0-9]{4})-[^ |/]+-[0-9a-f]{8}/?")
SHA = re.compile(r"[0-9a-f]{7,40}")
PI4 = ("#17's Pi 4 part stays by value only: there is no camera or KVM on the "
       "Pi 4, so its pictures are not looked at by eye.")
NOTE = "docs/notes/imx8mp-glmark2-pictures-by-eye.md"
REDACT = re.compile(
    r"([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|ttyUSB|ttyACM|ts\.net|password")


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


def derive_board(text):
    """The board the note measures on, for the default when --board is absent.

    The owner's rule of 2026-11-27: while the Verdin iMX8M Plus is away the
    i.MX8MP board work is done on the Armadillo-X2 (same SoC and GPU). A note
    that measures on the Armadillo-X2 (or whose records carry board=x2) is an
    X2 note; a note that names only the Verdin is a Verdin note; a note naming
    neither defaults to the X2."""
    if re.search(r"Armadillo|board=x2|board\.sh x2 run", text):
        return "x2"
    if "Verdin" in text:
        return "verdin"
    return "x2"


def board_words(board):
    """The wording slots that name the measured board in the six blocks.

    x2 and verdin have fixed wording (verdin keeps the texts of before the
    owner's rule of 2026-11-27); any other value names the board as given."""
    if board == "x2":
        return {
            "paren": "measured on the Armadillo-X2 (same i.MX8MP SoC and GC7000UL)",
            "where": ("board: Armadillo-X2 (measured on the X2, not on the away "
                      "Verdin iMX8M Plus; owner's rule of 2026-11-27)"),
            "eye": " on the Armadillo-X2",
            "by_eye": "by eye (measured on the Armadillo-X2)",
            "jp": "Armadillo-X2 で測定、",
        }
    if board == "verdin":
        return {
            "paren": "Verdin iMX8M Plus, GC7000UL",
            "where": "board: Verdin iMX8M Plus",
            "eye": "",
            "by_eye": "by eye",
            "jp": "",
        }
    return {
        "paren": f"measured on {board}",
        "where": f"board: {board}",
        "eye": f" on {board}",
        "by_eye": f"by eye (measured on {board})",
        "jp": f"{board} で測定、",
    }


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


def templates():
    """RESULTS.md / STATUS.md lines for plan #387, X2 wording, placeholders."""
    w = board_words("x2")
    print("== RESULTS.md 'Common set', i.MX8MP row: replace '(not looked at by eye)' ==")
    print(f"looked at by eye ({w['paren']}; commit <COMMIT>, record "
          f"<RECORD>; <N> right, <N> wrong, <N> seen without a value, of {TOTAL}; "
          f"wrong scenes: <WRONG-SCENES or none>; `{NOTE}`)")
    print()
    print("== STATUS.md (Japanese), the two imx8mp picture sentences ==")
    for head in ("imx8mp 列は", "i.MX8MP の絵は"):
        print(f"{head} 33 シーンを目で見た（{w['jp']}同じ i.MX8MP SoC・GC7000UL、"
              "commit <COMMIT>、record <RECORD>、正しい <N>、間違い <N>、"
              f"値なしで見た <N>。間違いのシーン: <WRONG-SCENES または なし>。{NOTE}）。")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--note")
    ap.add_argument("--templates", action="store_true",
                    help="print the RESULTS.md and STATUS.md line templates "
                    "with <COMMIT>, <RECORD> and <WRONG-SCENES> placeholders "
                    "(X2 wording; no note needed)")
    ap.add_argument("--board", default=None, metavar="BOARD",
                    help="the board the pictures were measured on: x2, verdin, "
                    "or free text naming it; without it the board is derived "
                    "from the note (default Armadillo-X2)")
    a = ap.parse_args()
    if a.templates:
        templates()
        return
    if not a.note:
        die("--note PATH is required (or --templates)")
    try:
        with open(a.note, encoding="utf-8") as f:
            text = f.read()
    except OSError as e:
        die(f"cannot read the note: {e.strerror}")
    rows = parse_rows(text)
    check(rows)
    board = a.board.strip() if a.board is not None else None
    if board is not None:
        if not board or "\n" in board:
            die("--board names one board, without a newline")
        if REDACT.search(board):
            die("--board matches the redaction pattern; board addresses, tty "
                "paths and credentials stay in the environment")
    else:
        board = derive_board(text)
    bw = board_words(board)
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
             f"(kept on the session machine, gitignored); {bw['where']}; "
             f"route(s): {routes}")

    print("== 1. Comment for issue #17 ==")
    print(f"i.MX8MP, {bw['paren']}: glmark2-es2 pictures looked at by "
          f"eye: {counts}. {wtxt}. Built from {where}. Method and per-scene "
          f"table: `{NOTE}`. {PI4}")
    print()
    print("== 2. Comment for issue #1196 ==")
    print("The i.MX8MP part of 'Pictures by eye' (glmark2-es2, 33 scenes) is "
          f"done: {counts}. {wtxt}. Compared with the llvmpipe reference "
          "(`tools/cts/glmark2-host-ref.sh`, `tools/cts/glmark2-picdiff.py`). "
          f"{where[0].upper() + where[1:]}. Verdicts and records per scene: `{NOTE}`.")
    print()
    print("== 3. RESULTS.md 'Common set', i.MX8MP row: replace '(not looked at by eye)' ==")
    print(f"looked at by eye{bw['eye']} ({counts}; {wtxt}; `{NOTE}`)")
    print()
    print("== 4. STATUS.md (Japanese), two places (line 11 and line 30): replace "
          "'imx8mp 列は値だけで判定している回帰用の基準で、目で見るのは #17 で開いている。' "
          "and 'i.MX8MP の絵はまだ値だけで判定しており（カメラ無し）、目で見るのは #17 で開いている。' ==")
    jw = "間違い 0" if not wrong else "間違いのシーン: " + ", ".join(wrong)
    print(f"imx8mp 列は 33 シーンを目で見た（{bw['jp']}正しい {right}、間違い {len(wrong)}、"
          f"値なしで見た {seen}。{jw}。{NOTE}）。")
    print(f"i.MX8MP の絵は 33 シーンを目で見た（{bw['jp']}正しい {right}、間違い {len(wrong)}、"
          f"値なしで見た {seen}。{jw}。{NOTE}）。")
    print()
    print("== 5. docs/notes/stage2-bench-gap.md: row 'every scene's picture checked', i.MX8MP cell ==")
    print(f"by value, 33 of 33 (column `imx8mp`); {bw['by_eye']}: {counts} (`{NOTE}`)")
    print()
    print("== 6. ROADMAP.md stage 2, 'Pictures by eye': replace the two lines "
          "'Only the i.MX8MP by-eye check stays open (#17); its' and "
          "'pictures are checked by value only.' ==")
    print(f"The i.MX8MP pictures were looked at by eye{bw['eye']} ({counts}; {wtxt}; "
          f"`{NOTE}`, #17, #1196).")


if __name__ == "__main__":
    main()
