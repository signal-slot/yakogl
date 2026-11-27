#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""readme-glmark2-check.py - the README's glmark2 table names one main commit.

README.md's "glmark2-es2 at 800x600" table gives a score and a commit per
board. A score is only comparable across boards when all three were measured
at the same main commit, and only checkable when tools/cts/RESULTS.md holds a
glmark2 row for each board at that commit. This checks exactly that:

  * every board (AM62P, Raspberry Pi 4, i.MX8MP) has a row in the table;
  * the rows name one commit (short hashes match by prefix);
  * the commit is an ancestor of --ref (git merge-base --is-ancestor; skipped
    with a warning when git or the ref is unavailable, and with --ref none);
  * RESULTS.md's glmark2 table has a row for each board whose own commit, its
    first "main `<sha>`" outside parentheses, is that commit (full hash or the
    first 8 characters, not a looser prefix);
  * with --scenes (off by default), each of those rows also gives a number of
    ms a frame for every named scene and cites a record directory;
  * with --verdicts (off by default), each of those rows also states the run's
    outcome and the terrain and refract picture verdicts.

--scenes terrain,shadow,refract reads a scene figure as "<scene> <n> ms" (the
row text, markdown bold ignored; figures of another commit, "<scene> <n> ms at
`<other>`" and parentheses holding "at `<other>`", are left out). Both row
styles match: glmark2-ms.py --cells prints "terrain 178.86 ms (6 FPS)", and RESULTS.md rows have "terrain 250 ms
(4 FPS)". A record directory is records/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/
with <board> pi4, imx8mp or am62p and <sha8> the common commit's first 8
characters (by prefix). The i.MX8MP row may instead cite records/x2/ when its
text says "measured on the Armadillo-X2" (or "measured on the X2"); without
that note a records/x2 cite does not count.

--verdicts reads the row text (markdown bold and backticks ignored, case
ignored) for each of these; a number must not be part of a longer number:

  '33 scenes'        "33 scenes" or "33 of 33 scenes" (not "32 of 33 scenes")
  '0 failed'         "0 failed"
  '0 GL errors'      "0 GL errors", "0 GL error" ("0 GL error lines")
  '0 out-of-memory'  "0 out-of-memory", "0 out of memory", "0 out of GPU
                     memory" (hyphens or spaces; "... scenes" after it is fine)
  picture verdicts   for terrain and for refract, one of:
    (a) the scene name, then within 80 characters with no ',' ';' or '|'
        between them, a verdict: match / matches / matched, mismatch /
        mismatches, differ / differs / differed, or "not looked at by eye"
        ("terrain a match", "terrain and refract not looked at by eye",
        "refract means equal ... (not looked at by eye)");
    (b) a verdict word, then within 60 characters with no '(' ';' or '|' a
        parenthesised list naming the scene ("4 differ in a probe (desktop
        blur, jellyfish, terrain, refract)", "mismatch 5, each one probe of
        an animated scene (..., terrain c off 10, refract c ...)").
    A match / mismatch / differ verdict counts only when the row also names
    glmark2-pictures.txt (the table it was checked against); "not looked at
    by eye" counts on its own.

Opt-in checks of the other documents, with the README's commit (each failure
prints "FAIL: STATUS: ...", "FAIL: CHECKLIST: ..." or "FAIL: ROADMAP: ...");
the default run does not read these files:

  --status     STATUS.md (--status-file) has a line that mentions glmark2, names
               the commit's first 8 characters and gives each board's score as
               "<AM62P|Pi 4|i.MX8MP> <score>" (a few non-digits between, as in
               "AM62P は 468") with the score of that board's RESULTS.md row
               (the first number of the row after the board name);
  --checklist  the row of docs/release/checklist.md (--checklist-file) that
               starts with '| 9 ' does not contain 'left to #1196';
  --roadmap    the stage-2 section of ROADMAP.md (--roadmap-file; from the
               '## 2.' heading to the next '## ') names the commit and #17, and
               no sentence in it says the Pi 4 pictures are checked "by value
               only" (a sentence with 'Pi 4' and 'by value only').
  --prose      the README's paragraphs that mention glmark2 (blank-line separated)
               have no sentence with 'Pi 4' and 'by value only' (the Pi 4
               pictures are checked by eye, docs/notes/pi4-glmark2-pictures-by-eye.md),
               and every 'terrain <n> ms' figure in a sentence with 'Pi 4'
               equals the terrain ms of RESULTS.md's Pi 4 row, if it has one
               (a figure followed by 'at [main] `<sha>`' of another commit is history
               and is skipped; "terrain takes about 34.5 ms a frame" is read, "about"
               and a few verbs between are allowed). Failures print "FAIL: README: ...".

Each item missing gives "FAIL: RESULTS: the <Board> glmark2 row does not state
<what>" and exit status 1.

The live README fails this by design until its boards are re-measured at one
commit, so the gate runs the check on fixtures only (cts-selftest.sh).

Usage:
  python3 tools/cts/readme-glmark2-check.py [--readme F] [--results F] [--ref R]
                                                [--scenes terrain,shadow,refract]
                                                [--verdicts] [--status] [--checklist]
                                                [--roadmap] [--prose]
Exit status: 0 passes, 1 fails, 2 a file or table could not be parsed.
"""
import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))

# normalised first cell (lowercase, letters and digits only) -> board
BOARDS = {
    "am62p": "AM62P",
    "raspberrypi4": "Raspberry Pi 4",
    "pi4": "Raspberry Pi 4",
    "imx8mp": "i.MX8MP",
}
RECORD_DIR = {"AM62P": "am62p", "Raspberry Pi 4": "pi4", "i.MX8MP": "imx8mp"}
# The i.MX8MP row may cite records/x2/ when it says it was measured on the
# Armadillo-X2 (same SoC and GPU; farm rule of 2026-11-27).
X2_BOARDS = {"i.MX8MP"}
X2_NOTE = re.compile(r"\bmeasured\s+on\s+(?:the\s+)?(?:armadillo[- ])?x2\b", re.I)
WANT = ["AM62P", "Raspberry Pi 4", "i.MX8MP"]
HASH = re.compile(r"`([0-9a-f]{7,40})`")


def norm(cell):
    return re.sub(r"[^a-z0-9]", "", cell.lower())


def table_rows(path, header_re):
    """Rows (lists of cells) of the first table whose header matches header_re."""
    try:
        with open(path, encoding="utf-8") as f:
            lines = f.read().splitlines()
    except OSError as e:
        print("error: %s: %s" % (path, e.strerror))
        sys.exit(2)
    start = None
    for i, ln in enumerate(lines):
        if ln.startswith("|") and re.search(header_re, ln, re.I):
            start = i
            break
    if start is None:
        print("error: %s: no table with a header matching %r" % (path, header_re))
        sys.exit(2)
    rows = []
    for ln in lines[start + 1:]:
        if not ln.startswith("|"):
            break
        cells = [c.strip() for c in ln.strip().strip("|").split("|")]
        if cells and not re.fullmatch(r"[-: ]+", cells[0]):
            rows.append(cells)
    return rows


def board_hashes(rows):
    out = {}
    for cells in rows:
        b = BOARDS.get(norm(cells[0]))
        if b:
            out[b] = HASH.findall(" ".join(cells[1:]))
    return out


def board_text(rows):
    out = {}
    for cells in rows:
        b = BOARDS.get(norm(cells[0]))
        if b:
            out[b] = " ".join(cells[1:]).replace("*", "")
    return out


def same(a, b):
    return a.startswith(b) or b.startswith(a)


def same_commit(a, b):
    """a and b name one commit: equal, or both >= 8 characters and equal in
    their first 8 (not a looser prefix)."""
    return a == b or (min(len(a), len(b)) >= 8 and a[:8] == b[:8])


ATTRIB = re.compile(r"\bmain\s+`([0-9a-f]{7,40})`", re.I)
AT_OTHER = re.compile(r"\bat\s+`([0-9a-f]{7,40})`", re.I)


def row_commit(text):
    """The row's own commit: its first 'main `<sha>`' outside parentheses."""
    depth, out = 0, ""
    for ch in text:
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth = max(0, depth - 1)
        out += ch if depth == 0 else " "
    m = ATTRIB.search(out)
    return m.group(1) if m else None


def own_run_text(text, commit):
    """text without the figures that belong to another commit: parenthesised
    groups holding 'at `<other>`' and '<scene> <n> ms [(..)] at `<other>`'."""
    def other(m):
        return not same_commit(m.group(1), commit)
    prev = None
    while prev != text:
        prev = text
        text = re.sub(r"\([^()]*\)", lambda m: " " if any(
            other(a) for a in AT_OTHER.finditer(m.group(0))) else m.group(0), text)
    return re.sub(r"\b[a-z][\w-]*\s+\S+\s+ms\b(?:\s*\([^()]*\))?\s+at\s+`([0-9a-f]{7,40})`",
                  lambda m: " " if other(m) else m.group(0), text, flags=re.I)


def scene_fails(board, text, scenes, commit):
    fails = []
    own = own_run_text(text, commit)
    for sc in scenes:
        m = re.search(r"\b%s\b\s+(\S+)\s+ms\b" % re.escape(sc), own, re.I)
        if not m:
            fails.append("RESULTS: the %s glmark2 row has no '%s <n> ms' figure" % (board, sc))
        elif not re.fullmatch(r"\d+(?:\.\d+)?", m.group(1)):
            fails.append("RESULTS: the %s glmark2 row has a non-numeric %s figure '%s'" % (
                board, sc, m.group(1)))
    dirs = [RECORD_DIR[board]]
    if board in X2_BOARDS and X2_NOTE.search(text):
        dirs.append("x2")
    recs = re.findall(r"records/(?:%s)/\d{8}-\d{4}-[A-Za-z0-9._-]+-([0-9a-f]{8})/?(?![\w-])" %
                      "|".join(dirs), text)
    if not recs:
        fails.append("RESULTS: the %s glmark2 row cites no record directory "
                     "records/%s/<YYYYMMDD-HHMM>-<name>-<sha8>/" % (board, RECORD_DIR[board]))
    elif not any(same(commit, r) for r in recs):
        fails.append("RESULTS: the %s glmark2 row's record directories (%s) are not at `%s`" % (
            board, ", ".join(recs), commit))
    return fails


# what a row must state with --verdicts: (label, pattern over the row text)
OUTCOMES = [
    ("'33 scenes'", r"(?<!of )(?<![\d.])33 (?:of 33 )?scenes\b"),
    ("'0 failed'", r"(?<![\d.])0 failed\b"),
    ("'0 GL errors'", r"(?<![\d.])0 GL errors?\b"),
    ("'0 out-of-memory'", r"(?<![\d.])0 out[- ]of[- ](?:GPU[- ])?memory\b"),
]
PICTURE_SCENES = ["terrain", "refract"]
VERDICT = r"(?:\b(?:mis)?match(?:es|ed)?\b|\bdiffer(?:s|ed)?\b|\bnot looked at by eye\b)"
BY_EYE = "not looked at by eye"


def picture_verdict(text, scene):
    """True when text states a picture verdict for scene (module docstring)."""
    sc = r"\b%s\b" % re.escape(scene)
    tables = "glmark2-pictures.txt" in text
    for m in re.finditer(sc + r"[^,;|]{0,80}?(" + VERDICT + ")", text):
        if tables or m.group(1) == BY_EYE:
            return True
    for m in re.finditer("(" + VERDICT + r")[^(;|]{0,60}\(([^()]*)\)", text):
        if re.search(sc, m.group(2)) and (tables or m.group(1) == BY_EYE):
            return True
    return False


def verdict_fails(board, text):
    text = text.replace("`", "").lower()
    fails = []
    for what, pat in OUTCOMES:
        if not re.search(pat, text, re.I):
            fails.append(what)
    for sc in PICTURE_SCENES:
        if not picture_verdict(text, sc):
            fails.append("a %s picture verdict" % sc)
    return ["RESULTS: the %s glmark2 row does not state %s" % (board, w) for w in fails]


STATUS_LABEL = [("AM62P", r"AM62P"), ("Raspberry Pi 4", r"Pi 4"), ("i.MX8MP", r"i\.MX8MP")]


def read_lines(path):
    try:
        with open(path, encoding="utf-8") as f:
            return f.read().splitlines()
    except OSError as e:
        print("error: %s: %s" % (path, e.strerror))
        sys.exit(2)


def status_fails(path, commit, rtext):
    scores = {}
    for b, _ in STATUS_LABEL:
        m = re.match(r"\s*(\d+)\b", rtext.get(b, ""))
        if m:
            scores[b] = m.group(1)
    sha8 = commit[:8]
    best = None
    for ln in read_lines(path):
        if "glmark2" not in ln.lower() or sha8 not in ln:
            continue
        miss = [b for b, lab in STATUS_LABEL if b not in scores or not re.search(
            r"%s[^\d\n]{0,6}%s(?!\d)" % (lab, scores[b]), ln)]
        if not miss:
            return []
        best = miss
    if best is None:
        return ["STATUS: no line mentions glmark2 and names `%s`" % sha8]
    return ["STATUS: the glmark2 line naming `%s` lacks the RESULTS score of %s" % (
        sha8, ", ".join(best))]


def checklist_fails(path):
    rows = [ln for ln in read_lines(path) if ln.startswith("| 9 ")]
    if not rows:
        return ["CHECKLIST: no row starting with '| 9 '"]
    if "left to #1196" in rows[0]:
        return ["CHECKLIST: row 9 still says 'left to #1196'"]
    return []


def roadmap_fails(path, commit):
    sec, inside = [], False
    for ln in read_lines(path):
        if re.match(r"##\s+2\.", ln):
            inside = True
        elif inside and ln.startswith("## "):
            break
        if inside:
            sec.append(ln)
    if not sec:
        return ["ROADMAP: no '## 2.' stage-2 section"]
    text = " ".join(" ".join(sec).split())
    fails = []
    if commit[:8] not in text:
        fails.append("ROADMAP: the stage-2 section does not name `%s`" % commit[:8])
    if not re.search(r"#17(?!\d)", text):
        fails.append("ROADMAP: the stage-2 section does not name #17")
    for sent in re.split(r"[.!?](?=\s)", text):
        if "Pi 4" in sent and "by value only" in sent:
            fails.append("ROADMAP: the stage-2 section says the Pi 4 pictures are checked by value only")
            break
    return fails


def prose_fails(path, commit, rtext):
    paras, cur = [], []
    for ln in read_lines(path) + [""]:
        if ln.strip():
            cur.append(ln)
        else:
            if cur:
                paras.append(" ".join(" ".join(cur).split()))
            cur = []
    text = " ".join(p for p in paras if "glmark2" in p.lower() and not p.startswith("|"))
    fails = []
    want = None
    m = re.search(r"\bterrain\s+(\d+(?:\.\d+)?)\s+ms\b",
                  own_run_text(rtext.get("Raspberry Pi 4", ""), commit), re.I)
    if m:
        want = m.group(1)
    for sent in re.split(r"[.!?](?=\s)", text):
        if "Pi 4" not in sent:
            continue
        if "by value only" in sent:
            fails.append("README: the glmark2 prose says the Pi 4 pictures are judged by value only "
                         "(docs/notes/pi4-glmark2-pictures-by-eye.md records 33 of 33 checked by eye)")
        if want is None:
            continue
        for t in re.finditer(r"\bterrain\s+(?:\w+\s+){0,2}?(\d+(?:\.\d+)?)\s*ms\b"
                             r"(?P<rest>[^;,]{0,40})", sent, re.I):
            o = re.search(r"\bat\s+(?:main\s+)?`([0-9a-f]{7,40})`", t.group("rest"))
            if o and not same_commit(commit, o.group(1)):
                continue
            if float(t.group(1)) != float(want):
                fails.append("README: the glmark2 prose gives Pi 4 terrain %s ms, "
                             "RESULTS.md's Pi 4 row has %s ms" % (t.group(1), want))
    return fails


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--readme", default=os.path.join(ROOT, "README.md"))
    ap.add_argument("--results", default=os.path.join(HERE, "RESULTS.md"))
    ap.add_argument("--ref", default="origin/main",
                    help="the commit must be its ancestor ('none' skips)")
    ap.add_argument("--scenes", default="",
                    help="comma list of scenes (e.g. terrain,shadow,refract): each "
                    "board's RESULTS row also needs '<scene> <n> ms' and a record dir")
    ap.add_argument("--verdicts", action="store_true",
                    help="each board's RESULTS row also states 33 scenes, 0 failed, "
                    "0 GL errors, 0 out-of-memory and terrain/refract picture verdicts")
    ap.add_argument("--status", action="store_true",
                    help="STATUS.md has a glmark2 line with the commit's sha8 and the RESULTS scores")
    ap.add_argument("--status-file", default=os.path.join(HERE, "STATUS.md"))
    ap.add_argument("--checklist", action="store_true",
                    help="checklist row 9 no longer says 'left to #1196'")
    ap.add_argument("--checklist-file", default=os.path.join(ROOT, "docs", "release", "checklist.md"))
    ap.add_argument("--roadmap", action="store_true",
                    help="ROADMAP.md's stage-2 section names the commit and #17, no Pi 4 'by value only'")
    ap.add_argument("--prose", action="store_true",
                    help="README's glmark2 prose: no Pi 4 'by value only', Pi 4 terrain ms as in RESULTS")
    ap.add_argument("--roadmap-file", default=os.path.join(ROOT, "ROADMAP.md"))
    a = ap.parse_args()
    scenes = [x.strip() for x in a.scenes.split(",") if x.strip()]

    readme = board_hashes(table_rows(a.readme, r"\|\s*Board\s*\|.*Score"))
    rrows = table_rows(a.results, r"\|\s*Board\s*\|.*glmark2")
    results = board_hashes(rrows)
    rtext = board_text(rrows)
    fails = []

    cited = {}
    for b in WANT:
        if b not in readme:
            fails.append("README: no glmark2 row for %s" % b)
        elif not readme[b]:
            fails.append("README: the %s row names no commit in backticks" % b)
        else:
            cited[b] = readme[b][0]
    commits = []
    for c in cited.values():
        if not any(same(c, k) for k in commits):
            commits.append(c)
    if len(commits) > 1:
        fails.append("README: the boards name %d commits, want one: %s" % (
            len(commits), ", ".join("%s `%s`" % (b, c) for b, c in cited.items())))

    if commits:
        commit = commits[0]
        if a.ref.lower() != "none":
            for c in commits:
                try:
                    r = subprocess.run(["git", "-C", ROOT, "merge-base",
                                        "--is-ancestor", c, a.ref],
                                       capture_output=True, text=True)
                except OSError:
                    print("warning: ancestor check skipped: git unavailable")
                    break
                if r.returncode == 1:
                    fails.append("`%s` is not an ancestor of %s" % (c, a.ref))
                elif r.returncode != 0:
                    print("warning: ancestor check skipped for `%s`: %s" % (
                        c, r.stderr.strip() or "git failed"))
        for b in WANT:
            if b not in results:
                fails.append("RESULTS: no glmark2 row for %s" % b)
            elif not (rc := row_commit(rtext[b])) or not same_commit(commit, rc):
                fails.append("RESULTS: the %s glmark2 row does not cite `%s` as its own "
                             "commit (want 'main `<sha>`', 8 characters or full)" % (b, commit))
            else:
                if scenes:
                    fails += scene_fails(b, rtext[b], scenes, commit)
                if a.verdicts:
                    fails += verdict_fails(b, rtext[b])

    if commits and a.status:
        fails += status_fails(a.status_file, commits[0], rtext)
    if a.checklist:
        fails += checklist_fails(a.checklist_file)
    if commits and a.roadmap:
        fails += roadmap_fails(a.roadmap_file, commits[0])
    if commits and a.prose:
        fails += prose_fails(a.readme, commits[0], rtext)

    for f in fails:
        print("FAIL: " + f)
    if fails:
        return 1
    print("ok: %s at `%s`, RESULTS.md has a row for each" % (", ".join(WANT), commits[0]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
