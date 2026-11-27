#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""one-commit-docs.py - the six glmark2 doc edits from three common records at one commit (#1196).

    tools/cts/one-commit-docs.py --root DIR --am62p REC --pi4 REC --imx8mp REC
                                 [--cite-am62p DIR] [--cite-pi4 DIR] [--cite-imx8mp DIR]
                                 [--record-commit SHA] [--dry-run]
    tools/cts/one-commit-docs.py --self-test

REC is one board's common record, records/<board>/common-<sha8>/ as
`tools/cts/board-stage.sh <board> common` wrote it. Each record is read with
tools/cts/common-row.py's analyse() (loaded with importlib, not copied).
--cite-<board> DIR names the dated record directory
records/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/ the RESULTS row cites (as
common-row.py --cite does); readme-glmark2-check.py --scenes accepts only that
form, so a row without one fails its record-directory check (a warning says so).

It refuses (exit 1, the reason on stderr, nothing written) when the three
records name different sha8s, or when a record has other than 33 scenes, a
failed or unsupported scene, a GL error (or none recorded) or an out-of-memory
scene. terrain, shadow and refract are recorded, never judged. Anything else
common-row.py's verdict lists (a picture state, a gl_test case) is printed as
a warning and does not stop the edit.

Under --root (required, so the real tree is never edited by accident) it edits:

  README.md                 the '| Board | Score |' glmark2 rows of the three
                            boards (common-row.py readme_cell()), the sentence
                            that introduces the table, and the Pi 4 terrain
                            figure 'about <n> ms a frame at main `<sha>`';
  tools/cts/RESULTS.md      the three 'Common set' rows (results_row(), a new row
                            replaces its board's row), with an explicit terrain
                            and refract picture verdict appended;
  tools/cts/STATUS.md       one Japanese line '- **glmark2 の一本化（#1196）...'
                            naming the sha8, each board label directly before
                            its score (replaced on a re-run, otherwise put
                            before the 共通セット line), the date line, the
                            README clause of the 共通セット line and the Pi 4
                            terrain figure;
  website/data.js           the glmark2.<board> and gltest.<board> figures
                            (data_js(): value, unit, commit, onMain, date,
                            status, note; provenance kind 'repository', and
                            recordCommit with --record-commit);
  docs/release/checklist.md row '| 9 |': the 'left to #1196' sentence becomes
                            the commit and the figures, the rest of the row stays;
  ROADMAP.md                stage 2's State paragraph and the #1196 bullet name
                            the commit and leave the i.MX8MP by-eye check (#17)
                            open; the Pi 4 terrain figure '<n> ms at `<sha>`' is
                            updated, the rest of the Pi 4 wording stays.

  tools/cts/RESULTS.md      also the bullet 'The README glmark2 table's runs:'
                            (the 'README columns' passage above the per-scene
                            table): it names the one commit and the three rows
                            instead of the old per-board commits;
  tools/website-check.sh    PRE_PUBLICATION_BUILDS: the six glmark2.<board> /
                            gltest.<board> lines are dropped. The registry is
                            read only for provenance kind 'pre-publication';
                            this edit makes every one of the six figures
                            'repository' (the commit is an ancestor of HEAD,
                            checked against git), and a pre-publication entry
                            cannot be moved to the new commit: it must name a
                            hash in the first public snapshot's section and a
                            date before 2026-10-27. Dropping is the only
                            correct edit, and it keeps stale entries out.

An edit whose anchor is not found is reported as a warning (exit 0 still);
--dry-run prints the warnings and writes nothing. Paths printed are relative
to records/; no host, address or credential is read.

--self-test builds three synthetic records at the current HEAD's sha8 with
common-row.py's synth(), applies them to a temporary copy of the six real
files and runs readme-glmark2-check.py (--ref HEAD --scenes
terrain,shadow,refract --verdicts --status --checklist --roadmap) and the
website-check.sh glmark2 comparison against the copy, and the full
website-check.sh on a git clone of HEAD with the edited files copied in; it also checks that the
refusals fire on mismatched sha8s and on a GL error.
"""
import argparse
import datetime
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
ORDER = ("am62p", "pi4", "imx8mp")
LONG = {"am62p": "AM62P", "pi4": "Raspberry Pi 4", "imx8mp": "i.MX8MP"}
SHORT = {"am62p": "AM62P", "pi4": "Pi 4", "imx8mp": "i.MX8MP"}
# normalised first table cell -> board key
CELL = {"am62p": "am62p", "raspberrypi4": "pi4", "pi4": "pi4", "imx8mp": "imx8mp"}
FILES = {
    "readme": "README.md",
    "results": os.path.join("tools", "cts", "RESULTS.md"),
    "status": os.path.join("tools", "cts", "STATUS.md"),
    "datajs": os.path.join("website", "data.js"),
    "checklist": os.path.join("docs", "release", "checklist.md"),
    "roadmap": "ROADMAP.md",
    "webcheck": os.path.join("tools", "website-check.sh"),
}
STATUS_MARK = "- **glmark2 の一本化（#1196）"
CHECKLIST_LEAD = "The glmark2 figures in `website/data.js` follow the README table at main"
DATED_RE = re.compile(r"^\d{8}-\d{4}-[A-Za-z0-9._-]+-([0-9a-f]{8})$")


X2_NOTE_EN = "measured on the Armadillo-X2"
X2_NOTE_LONG = ("measured on the Armadillo-X2 (same i.MX8MP SoC and GC7000UL GPU; the Verdin i.MX8MP "
                "is off the farm)")
X2_NOTE_JA = "Armadillo-X2 で測定（同じ SoC・GPU）"
X2_CITE_RE = re.compile(r"records/(?:x2|imx8mp)/\d{8}-\d{4}-[A-Za-z0-9._-]+-[0-9a-f]{8}/?$")
BY_EYE_OPEN = "(not looked at by eye)"


class Refused(Exception):
    pass


def on_x2(d):
    """True when the record carries the marker: a file `measured-on` holding `armadillo-x2`
    (also `measured-on: armadillo-x2`) in the record directory. Only the i.MX8MP is ever X2."""
    if d.get("board_key") != "imx8mp":
        return False
    try:
        with open(os.path.join(d["record"], "measured-on"), encoding="utf-8") as f:
            return re.search(r"(?i)\barmadillo-x2\b", f.read()) is not None
    except OSError:
        return False


def load_common_row():
    spec = importlib.util.spec_from_file_location("common_row", os.path.join(HERE, "common-row.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["common_row"] = mod
    spec.loader.exec_module(mod)
    return mod


def norm(cell):
    return re.sub(r"[^a-z0-9]", "", cell.lower())


def ms(v):
    return "%.2f" % v


# ---------------------------------------------------------------- reading
def cite_text(cr, path):
    """the --cite-<board> value as the row prints it: records/<board>/<dir>/"""
    if path is None:
        return None
    return cr.rel(path).rstrip("/") + "/"


def read_records(cr, recs, cites):
    """{board: analyse() dict}; raises Refused with every reason found"""
    ct = cr.load_tally()
    ds, why = {}, []
    for b in ORDER:
        try:
            d = cr.analyse(recs[b], b, ct, cite_text(cr, cites.get(b)))
        except (ValueError, OSError) as e:
            why.append("%s: %s" % (SHORT[b], e))
            continue
        d["board_key"] = b
        ds[b] = d
    if why:
        raise Refused("; ".join(why))
    shas = {b: ds[b]["sha8"] for b in ORDER}
    if len(set(shas.values())) != 1:
        raise Refused("the records name different commits: " + ", ".join(
            "%s %s" % (SHORT[b], shas[b]) for b in ORDER))
    for b in ORDER:
        d = ds[b]
        if d["nscenes"] != cr.WANT_SCENES:
            why.append("%s: %d scenes, want %d" % (SHORT[b], d["nscenes"], cr.WANT_SCENES))
        nofps = [s["name"] for s in d["scenes"] if s["fps"] is None]
        if d["failed"] or d["unsupported"] or nofps:
            why.append("%s: %d failed and %d unsupported scene(s)%s" % (
                SHORT[b], max(d["failed"], len(nofps)), d["unsupported"],
                " (" + ", ".join(nofps) + ")" if nofps else ""))
        if d["gl_errors"] is None:
            why.append("%s: GL errors not recorded" % SHORT[b])
        elif d["gl_errors"]:
            why.append("%s: %d GL error line(s)" % (SHORT[b], d["gl_errors"]))
        if d["oom"] or d["oom_outside"]:
            why.append("%s: out-of-memory in %s" % (SHORT[b], ", ".join(d["oom"]) or "no scene"))
        if d["score"] is None:
            why.append("%s: no glmark2 score" % SHORT[b])
    if why:
        raise Refused("; ".join(why))
    return ds


def warnings_for(cr, ds, cites):
    out = []
    for b in ORDER:
        d = ds[b]
        refused_kinds = ("scenes, want", "failed/unsupported", "GL error", "out-of-memory", "no glmark2 score")
        for f in d["fails"]:
            if not any(k in f for k in refused_kinds):
                out.append("%s: common-row.py verdict: %s (not refused)" % (SHORT[b], f))
        if not cites.get(b):
            hint = ""
            parent = os.path.dirname(os.path.abspath(d["record"]))
            try:
                dated = sorted(n for n in os.listdir(parent)
                               if (m := DATED_RE.match(n)) and m.group(1) == d["sha8"])
            except OSError:
                dated = []
            if dated:
                hint = "; candidates: " + ", ".join(cr.rel(os.path.join(parent, n)) + "/" for n in dated)
            out.append("%s: no --cite-%s: the RESULTS row cites only %s/, which readme-glmark2-check.py "
                       "--scenes does not accept (it wants records/%s/<YYYYMMDD-HHMM>-<name>-%s/)%s" % (
                           SHORT[b], b, cr.rel(d["record"]), b, d["sha8"], hint))
        elif not (cr.CITE_RE.search(cite_text(cr, cites[b]))
                  or (on_x2(d) and X2_CITE_RE.search(cite_text(cr, cites[b])))):
            out.append("%s: --cite-%s %s is not records/%s/<YYYYMMDD-HHMM>-<name>-<sha8>/" % (
                SHORT[b], b, cite_text(cr, cites[b]), b))
    return out


# ---------------------------------------------------------------- text pieces
def pic_state(d, scene):
    for info, (state, _) in d["pic"].items():
        if info.startswith("[%s]" % scene):
            return state
    return None


def results_line(cr, d):
    """common-row.py's row with an explicit terrain/refract picture verdict in the glmark2 cell"""
    row = cr.results_row(d)
    said = []
    for sc in ("terrain", "refract"):
        st = pic_state(d, sc)
        if st in ("match", "mismatch"):
            said.append("%s picture a %s" % (sc, st))
    if d["board_key"] == "imx8mp":
        # the i.MX8MP pictures are checked by value only; the by-eye check is #17
        said.append("pictures by value only " + BY_EYE_OPEN)
    if on_x2(d):
        said.append(X2_NOTE_LONG)
    if not said:
        return row
    cells = row.split(" | ")
    # cells: '| <board>', glmark2, gl_test, resets ' |'
    cells[1] += "; " + "; ".join(said)
    return " | ".join(cells)


def dates(ds):
    ds_ = sorted({ds[b]["date"] for b in ORDER})
    return ds_[0] if len(ds_) == 1 else "%s to %s" % (ds_[0], ds_[-1])


def scores_en(ds):
    return ", ".join("%s %s%s" % (LONG[b], ds[b]["score"], " (%s)" % X2_NOTE_EN if on_x2(ds[b]) else "")
                     for b in ORDER)


def slow_en(cr, ds):
    return "; ".join("%s %s" % (SHORT[b], cr.slow_text(ds[b])) for b in ORDER)


def slow_ja(ds):
    out = []
    for b in ORDER:
        parts = []
        for k in ("terrain", "shadow", "refract"):
            sc = ds[b]["slow"][k]
            parts.append("%s %s ms" % (k, ms(sc["ms"])) if sc and sc["ms"] is not None else "%s 無し" % k)
        out.append("%s は %s" % (SHORT[b], "、".join(parts)))
    return "、".join(out)


def gt_ja(cr, d):
    if d["gt_passed"] is None:
        return "結果無し"
    s = "%d/%d" % (d["gt_passed"], d["gt_total"])
    if d["skipped"]:
        s += "（%d スキップ）" % d["skipped"]
    return s


def pics_ja(d):
    st = d["pic_states"]
    s = "%d 中 %d 一致" % (sum(st.values()), st.get("match", 0))
    for k, ja in (("mismatch", "不一致"), ("black", "黒"), ("absent", "欠け"), ("unreadable", "読めない"),
                  ("no-reference", "基準無し"), ("not-in-record", "記録無し")):
        if st.get(k):
            s += "・%s %d" % (ja, st[k])
    return s


# ---------------------------------------------------------------- edits
def table_span(lines, header_re, start=0):
    """(first row index, end index) of the first table at or after start whose header matches"""
    for i in range(start, len(lines)):
        if lines[i].startswith("|") and re.search(header_re, lines[i]):
            j = i + 1
            while j < len(lines) and lines[j].startswith("|"):
                j += 1
            return i + 1, j
    return None


def replace_rows(lines, span, rows, warn, what):
    """replace (or append) one row per board inside the table span"""
    lo, hi = span
    seen = set()
    for i in range(lo, hi):
        cells = [c.strip() for c in lines[i].strip().strip("|").split("|")]
        b = CELL.get(norm(cells[0])) if cells else None
        if b in rows and b not in seen:
            lines[i] = rows[b]
            seen.add(b)
    add = [rows[b] for b in ORDER if b in rows and b not in seen]
    for b in ORDER:
        if b in rows and b not in seen:
            warn.append("%s: no %s row; added one at the end of the table" % (what, SHORT[b]))
    lines[hi:hi] = add


def readme_cell(cr, d):
    cell = cr.readme_cell(d)
    if on_x2(d):
        cell = cell[:-len(" |")].rstrip()
        assert cell.endswith(")")
        cell = "%s; %s) |" % (cell[:-1], X2_NOTE_EN)
    return cell


def edit_readme(text, cr, ds, sha8, warn):
    lines = text.split("\n")
    span = None
    for i, ln in enumerate(lines):
        if ln.strip() == "| Board | Score |":
            span = table_span(lines, r"^\|\s*Board\s*\|\s*Score\s*\|$", i)
            break
    if span is None:
        warn.append("README: no '| Board | Score |' table; nothing edited")
        return text
    lo = span[0]
    replace_rows(lines, (lo + 1, span[1]), {b: readme_cell(cr, ds[b]) for b in ORDER}, warn, "README")
    # the paragraph that introduces the table
    hdr = lo - 1
    j = hdr - 1
    if j >= 0 and lines[j] == "":
        j -= 1
    k = j
    while k > 0 and lines[k - 1] != "":
        k -= 1
    if j >= 0 and lines[k].startswith("glmark2-es2 at 800x600") and lines[j].endswith(":"):
        lines[k:j + 1] = [
            "glmark2-es2 at 800x600 runs all 33 scenes on every board, with 0 failed",
            "scenes, 0 GL errors and 0 out-of-memory scenes. All three scores are from",
            "main `%s` (%s; `tools/cts/RESULTS.md`, \"Common set\"):" % (sha8, dates(ds)),
        ]
    else:
        warn.append("README: the paragraph before the glmark2 table does not start with "
                    "'glmark2-es2 at 800x600' and end with ':'; left as it is")
    text = "\n".join(lines)
    t = ds["pi4"]["slow"]
    new, n = re.subn(
        r"(Pi\s+4\s+terrain\s+takes\s+about\s+)\d+(?:\.\d+)?(\s+ms\s+a\s+frame\s+at\s+main\s+)`[0-9a-f]{7,40}`"
        r"(\s+\(shadow\s+about\s+)\d+(?:\.\d+)?(\s+ms,\s+refract\s+about\s+)\d+(?:\.\d+)?(\s+ms\))",
        lambda m: "%s%s%s`%s`%s%s%s%s%s" % (
            m.group(1), ms(t["terrain"]["ms"]), m.group(2), sha8, m.group(3), ms(t["shadow"]["ms"]),
            m.group(4), ms(t["refract"]["ms"]), m.group(5)), text)
    if n:
        text = new
    else:
        warn.append("README: no 'Pi 4 terrain takes about <n> ms a frame at main `<sha>` (shadow about ..., "
                    "refract about ...)' figure; left as it is")
    return text


def edit_results(text, cr, ds, warn):
    lines = text.split("\n")
    start = next((i for i, ln in enumerate(lines) if re.match(r"##\s+Common set", ln)), None)
    if start is None:
        warn.append("RESULTS: no '## Common set' section; nothing edited")
        return text
    span = table_span(lines, r"^\|\s*Board\s*\|\s*glmark2", start)
    if span is None:
        warn.append("RESULTS: no '| Board | glmark2-es2 | ...' table in 'Common set'; nothing edited")
        return text
    lo, hi = span
    replace_rows(lines, (lo + 1, hi), {b: results_line(cr, ds[b]) for b in ORDER}, warn, "RESULTS")
    text = "\n".join(lines)
    return edit_results_columns(text, ds, warn)


RESULTS_COLS_TAIL = "keeps the earlier runs its column heads name."
RESULTS_COLS_RE = re.compile(
    r"(?ms)^(?P<ind>[ \t]*)- The README glmark2 table's runs:.*?"
    r"(?:\(2026-10-22, below\)\.|" + re.escape(RESULTS_COLS_TAIL) + r")")


def edit_results_columns(text, ds, warn):
    m = RESULTS_COLS_RE.search(text)
    if not m:
        warn.append("RESULTS: no 'The README glmark2 table's runs:' passage; left out")
        return text
    ind = m.group("ind")
    para = ("- The README glmark2 table's runs: the GM rows of the common set "
            "from one main commit, `%s` (%s), on all three boards: the three "
            "rows above, %s. The per-scene table below %s"
            % (ds["am62p"]["sha8"], dates(ds), scores_en(ds), RESULTS_COLS_TAIL))
    new = "\n".join(ind + (ln if i == 0 else "  " + ln) for i, ln in enumerate(wrap(para)))
    return text[:m.start()] + new + text[m.end():]


REGISTRY_IDS = ["%s.%s" % (k, b) for k in ("gltest", "glmark2") for b in ORDER]


def edit_webcheck(text, warn):
    """drop the PRE_PUBLICATION_BUILDS lines of the six figures (see the header)"""
    out = []
    for ln in text.split("\n"):
        if any(re.match(r'\s*"%s":\s*\(' % re.escape(i), ln) for i in REGISTRY_IDS):
            continue
        out.append(ln)
    if "PRE_PUBLICATION_BUILDS = {" not in text:
        warn.append("website-check.sh: no PRE_PUBLICATION_BUILDS dict; left out")
        return text
    return "\n".join(out)


def status_line(cr, ds, sha8):
    x2 = "。i.MX8MP の値は " + X2_NOTE_JA if on_x2(ds["imx8mp"]) else ""
    gts = "、".join("%s %s" % (SHORT[b], gt_ja(cr, ds[b])) for b in ORDER)
    pics = "、".join("%s %s" % (SHORT[b], pics_ja(ds[b])) for b in ORDER)
    return ("%s: README の glmark2 表は 3 ボードとも main %s の値（%s）: AM62P %s、Pi 4 %s、i.MX8MP %s%s。** "
            "3 ボードとも glmark2-es2 800x600 で 33 シーン実行、失敗 0、GL エラー 0、メモリ不足のシーン 0。"
            "gl_test は %s。絵は glmark2-pictures.txt の各列と %s。遅いシーンは記録だけで判定しない: %s。"
            "記録は records/<board>/common-%s/（RESULTS.md の「Common set」）。i.MX8MP の絵を目で見る確認（#17）は残る。" % (
                STATUS_MARK, sha8, dates(ds), ds["am62p"]["score"], ds["pi4"]["score"], ds["imx8mp"]["score"], x2,
                gts, pics, slow_ja(ds), sha8))


def edit_status(text, cr, ds, sha8, warn, today):
    lines = text.split("\n")
    line = status_line(cr, ds, sha8)
    i = next((i for i, ln in enumerate(lines) if ln.startswith(STATUS_MARK)), None)
    if i is not None:
        lines[i] = line
    else:
        j = next((j for j, ln in enumerate(lines) if ln.startswith("- **共通セット")), None)
        if j is None:
            j = len(lines)
            warn.append("STATUS: no '- **共通セット' line; the glmark2 line goes at the end")
        lines[j:j] = [line]
    text = "\n".join(lines)
    text, n = re.subn(r"(?m)^\d{4}-\d{2}-\d{2} 更新。", today + " 更新。", text, count=1)
    if not n:
        warn.append("STATUS: no '<YYYY-MM-DD> 更新。' line; left as it is")
    text, n = re.subn(r"README の glmark2 表は(?! 3 ボードとも)[^*\n]*",
                      "README の glmark2 表は 3 ボードとも main %s の値（上の行）。" % sha8, text, count=1)
    t = ds["pi4"]["slow"]["terrain"]
    text, n = re.subn(r"(Pi 4 の terrain（)[0-9a-f]{7,40}( で )\d+(?:\.\d+)?( ms)",
                      lambda m: "%s%s%s%s%s" % (m.group(1), sha8, m.group(2), ms(t["ms"]), m.group(3)), text)
    if not n:
        warn.append("STATUS: no 'Pi 4 の terrain（<sha> で <n> ms' figure; left as it is")
    return text


def edit_checklist(text, ds, sha8, warn):
    lines = text.split("\n")
    i = next((i for i, ln in enumerate(lines) if ln.startswith("| 9 ")), None)
    if i is None:
        warn.append("CHECKLIST: no '| 9 ' row; nothing edited")
        return text
    row = lines[i]
    new = "%s `%s` (%s, %s; #1196)." % (CHECKLIST_LEAD, sha8, scores_en(ds), dates(ds))
    k = row.find("left to #1196")
    if k < 0:
        k = row.find(CHECKLIST_LEAD)
    if k < 0:
        warn.append("CHECKLIST: row 9 has neither 'left to #1196' nor an earlier edit; nothing edited")
        return text
    s = row.rfind(". ", 0, k)
    s = s + 2 if s >= 0 else row.rfind("| ", 0, k) + 2
    e = k
    while True:
        e = row.find(".", e)
        if e < 0 or e + 1 >= len(row) or row[e + 1] == " ":
            break
        e += 1
    e = len(row) if e < 0 else e + 1
    lines[i] = row[:s] + new + row[e:]
    return "\n".join(lines)


def roadmap_section(lines):
    lo = next((i for i, ln in enumerate(lines) if re.match(r"##\s+2\.", ln)), None)
    if lo is None:
        return None
    hi = next((i for i in range(lo + 1, len(lines)) if lines[i].startswith("## ")), len(lines))
    return lo, hi


def wrap(text, width=76):
    out, cur = [], ""
    for w in text.split(" "):
        if cur and len(cur) + 1 + len(w) > width:
            out.append(cur)
            cur = w
        else:
            cur = w if not cur else cur + " " + w
    if cur:
        out.append(cur)
    return out


def edit_roadmap(text, cr, ds, sha8, warn):
    lines = text.split("\n")
    sec = roadmap_section(lines)
    if sec is None:
        warn.append("ROADMAP: no '## 2.' section; nothing edited")
        return text
    lo, hi = sec
    black = sum(ds[b]["pic_states"].get("black", 0) for b in ORDER)
    state = ("**State.** glmark2-es2 runs all 33 scenes on every board with no failed scene, no GL error "
             "and no scene out of GPU memory (`tools/cts/RESULTS.md`, \"Common set\"). The `README.md` "
             "table holds one score per board from main `%s` (%s): %s. A frame time and a picture value "
             "are recorded for every scene on every board (`tools/cts/glmark2-pictures.txt`)%s." % (
                 sha8, dates(ds), scores_en(ds), ", and no scene is black" if black == 0 else ""))
    i = next((i for i in range(lo, hi) if lines[i].startswith("**State.**")), None)
    if i is None:
        warn.append("ROADMAP: no '**State.**' paragraph in stage 2; left as it is")
    else:
        j = i
        while j < hi and lines[j] != "":
            j += 1
        lines[i:j] = wrap(state)
        hi += len(wrap(state)) - (j - i)
    # the #1196 bullet
    b = None
    for k in range(lo, hi):
        if lines[k].startswith("- "):
            e = k + 1
            while e < hi and lines[e].startswith("  ") and not lines[e].startswith("- "):
                e += 1
            if "#1196" in " ".join(lines[k:e]):
                b = (k, e)
                break
    bullet = ("- One main commit for the three scores (#1196): main `%s`, with terrain and refract pictures "
              "checked by value against `tools/cts/glmark2-pictures.txt`. Of the criterion only the "
              "i.MX8MP by-eye check (#17, above) stays open." % sha8)
    if b is None:
        warn.append("ROADMAP: no stage-2 bullet naming #1196; left as it is")
    else:
        w = wrap(bullet[2:], 74)
        lines[b[0]:b[1]] = ["- " + w[0]] + ["  " + x for x in w[1:]]
    t = ds["pi4"]["slow"]["terrain"]
    lo, hi = roadmap_section(lines)
    body = "\n".join(lines[lo:hi])
    body, n = re.subn(r"(Pi 4 terrain,\s+)\d+(?:\.\d+)?(\s+ms\s+at\s+)`[0-9a-f]{7,40}`",
                      lambda m: "%s%s%s`%s`" % (m.group(1), ms(t["ms"]), m.group(2), sha8), body)
    if not n:
        warn.append("ROADMAP: no 'Pi 4 terrain, <n> ms at `<sha>`' figure in stage 2; left as it is")
    return "\n".join(lines[:lo] + body.split("\n") + lines[hi:])


def edit_datajs(text, cr, ds, warn, record_commit):
    for b in ORDER:
        src = cr.data_js(ds[b])
        src = re.sub(r",(\s*)$", r"\1", src.strip())
        figs = json.loads("{" + re.sub(r",(\s*})", r"\1", src) + "}")
        # data_js() gives gltest.<board> no note: the old one describes an older run
        d = ds[b]
        if on_x2(d):
            figs["glmark2." + b]["note"] = figs["glmark2." + b]["note"].rstrip() + " " + \
                X2_NOTE_EN[0].upper() + X2_NOTE_EN[1:] + " (same SoC and GPU)."
        gnote = "Run from main %s (the common set %s row; %s/)." % (d["sha8"], SHORT[b], cr.rel(d["record"]))
        if d["gt_failing"]:
            gnote += " Failing: %s." % ", ".join("case %s" % c for c in d["gt_failing"])
        if on_x2(d):
            gnote += " " + X2_NOTE_EN[0].upper() + X2_NOTE_EN[1:] + " (same SoC and GPU)."
        figs.setdefault("gltest." + b, {}).setdefault("note", gnote)
        for fid, fields in figs.items():
            m = re.search(r'(?m)^(\s*)"%s": \{\n' % re.escape(fid), text)
            if not m:
                warn.append("data.js: no figure %s; left out" % fid)
                continue
            ind = m.group(1)
            end = re.compile(r"(?m)^%s\},?$" % re.escape(ind)).search(text, m.end())
            if not end:
                warn.append("data.js: figure %s does not end; left out" % fid)
                continue
            block = text[m.end():end.start()]
            fields = dict(fields)
            for key, val in fields.items():
                pat = re.compile(r'(?m)^(\s*"%s": ).*?(,?)$' % re.escape(key))
                v = json.dumps(val, ensure_ascii=False)
                if pat.search(block):
                    block = pat.sub(lambda mm: mm.group(1) + v + mm.group(2), block, count=1)
                else:
                    warn.append("data.js: %s has no \"%s\" field; left out" % (fid, key))
            block = re.sub(r'(?m)^(\s*"kind": )"pre-publication"', r'\1"repository"', block)
            if record_commit:
                block = re.sub(r'(?m)^(\s*"recordCommit": )"[0-9a-f]*"', r'\1"%s"' % record_commit, block)
            text = text[:m.end()] + block + text[end.start():]
    return text


# ---------------------------------------------------------------- driver
def apply(root, recs, cites, record_commit=None, dry_run=False, today=None, out=sys.stdout):
    """edit the six files under root; returns the warnings; raises Refused"""
    cr = load_common_row()
    ds = read_records(cr, recs, cites)
    sha8 = ds["am62p"]["sha8"]
    warn = warnings_for(cr, ds, cites)
    today = today or datetime.date.today().isoformat()
    paths = {k: os.path.join(root, v) for k, v in FILES.items()}
    texts = {}
    for k, p in paths.items():
        try:
            with open(p, encoding="utf-8") as f:
                texts[k] = f.read()
        except OSError as e:
            raise Refused("%s: %s" % (FILES[k], e.strerror))
    new = {
        "readme": edit_readme(texts["readme"], cr, ds, sha8, warn),
        "results": edit_results(texts["results"], cr, ds, warn),
        "status": edit_status(texts["status"], cr, ds, sha8, warn, today),
        "datajs": edit_datajs(texts["datajs"], cr, ds, warn, record_commit),
        "checklist": edit_checklist(texts["checklist"], ds, sha8, warn),
        "roadmap": edit_roadmap(texts["roadmap"], cr, ds, sha8, warn),
        "webcheck": edit_webcheck(texts["webcheck"], warn),
    }
    for k in FILES:
        changed = new[k] != texts[k]
        print("%s %s" % ("edited" if changed and not dry_run else
                         "would edit" if changed else "unchanged", FILES[k]), file=out)
        if changed and not dry_run:
            with open(paths[k], "w", encoding="utf-8") as f:
                f.write(new[k])
    print("main %s: %s" % (sha8, scores_en(ds)), file=out)
    print("recorded, not judged: %s" % slow_en(cr, ds), file=out)
    for w in warn:
        print("warning: " + w, file=out)
    return warn


# ---------------------------------------------------------------- self test
def self_test():
    cr = load_common_row()
    fails = []

    def check(name, cond, got=None):
        if not cond:
            fails.append("%s%s" % (name, "" if got is None else ": got %s" % (got,)))

    head = subprocess.run(["git", "-C", REPO, "rev-parse", "--short=8", "HEAD"],
                          capture_output=True, text=True)
    if head.returncode != 0:
        print("FAIL: git rev-parse HEAD: %s" % head.stderr.strip())
        return 1
    sha8 = head.stdout.strip()[:8]
    tmp = tempfile.mkdtemp(prefix="one-commit-docs-", dir=os.environ.get("TMPDIR") or None)
    try:
        recs = {b: cr.synth(tmp, b, sha8) for b in ORDER}
        cites = {}
        for b in ORDER:
            # the dated copy board-stage.sh is to leave next to common-<sha8>/
            dated = os.path.join(tmp, "records", b, "20261125-1200-common-%s" % sha8)
            shutil.copytree(recs[b], dated)
            cites[b] = dated
        tree = os.path.join(tmp, "tree")
        for rel in FILES.values():
            os.makedirs(os.path.dirname(os.path.join(tree, rel)) or tree, exist_ok=True)
            shutil.copy(os.path.join(REPO, rel), os.path.join(tree, rel))
        log = open(os.devnull, "w")
        warn = apply(tree, recs, cites, today="2026-11-26", out=log)
        check("no warnings on the real files", not warn, warn)
        args = ["python3", "-I", os.path.join(HERE, "readme-glmark2-check.py"),
                "--readme", os.path.join(tree, FILES["readme"]),
                "--results", os.path.join(tree, FILES["results"]),
                "--status-file", os.path.join(tree, FILES["status"]),
                "--checklist-file", os.path.join(tree, FILES["checklist"]),
                "--roadmap-file", os.path.join(tree, FILES["roadmap"]),
                "--ref", "HEAD", "--scenes", "terrain,shadow,refract",
                "--verdicts", "--status", "--checklist", "--roadmap"]
        r = subprocess.run(args, capture_output=True, text=True)
        check("readme-glmark2-check.py passes the edited copy (exit %d)" % r.returncode,
              r.returncode == 0 and "ok:" in r.stdout, "\n" + r.stdout + r.stderr)
        # the website-check.sh glmark2 comparison on the edited README and data.js
        wd = os.path.join(tmp, "web")
        os.makedirs(wd)
        shutil.copy(os.path.join(tree, FILES["readme"]), os.path.join(wd, "README.md"))
        shutil.copy(os.path.join(tree, FILES["datajs"]), os.path.join(wd, "data.js"))
        r = subprocess.run(["bash", os.path.join(REPO, "tools", "website-check.sh")],
                           capture_output=True, text=True, env=dict(os.environ, WEBSITE_GLMARK2_DIR=wd))
        check("website-check.sh glmark2 comparison passes (exit %d)" % r.returncode,
              r.returncode == 0, "\n" + r.stdout + r.stderr)
        # the RESULTS passage and the registry
        res = open(os.path.join(tree, FILES["results"]), encoding="utf-8").read()
        check("RESULTS 'README columns' names the one commit",
              re.search(r"The README glmark2 table's runs: the GM rows of the common set\s+from\s+one\s+main\s+commit, `%s`" % sha8, res)
              is not None and "(2026-10-22, below)" not in res and "445c5653` (2026-10-20); the README" not in res)
        wc = open(os.path.join(tree, FILES["webcheck"]), encoding="utf-8").read()
        check("registry: no glmark2/gltest line left",
              not any(re.search(r'"%s":\s*\(' % re.escape(i), wc) for i in REGISTRY_IDS)
              and '"deqp2.am62p": (' in wc)
        # the full website-check.sh on a clone of HEAD with the edited files copied in
        full = os.path.join(tmp, "full")
        r = subprocess.run(["git", "clone", "-q", "--local", REPO, full], capture_output=True, text=True)
        if r.returncode == 0:
            subprocess.run(["git", "-C", full, "checkout", "-q", sha8], capture_output=True)
            for rel in FILES.values():
                shutil.copy(os.path.join(tree, rel), os.path.join(full, rel))
            r = subprocess.run(["bash", os.path.join(full, "tools", "website-check.sh"), full],
                               capture_output=True, text=True)
            check("full website-check.sh passes the edited clone (exit %d)" % r.returncode,
                  r.returncode == 0, "\n" + r.stdout + r.stderr)
        else:
            check("git clone for the full website-check.sh", False, r.stderr.strip())
        # a second run changes nothing
        snap = {k: open(os.path.join(tree, v), encoding="utf-8").read() for k, v in FILES.items()}
        apply(tree, recs, cites, today="2026-11-26", out=log)
        again = [k for k, v in FILES.items() if open(os.path.join(tree, v), encoding="utf-8").read() != snap[k]]
        check("a second run is a no-op", not again, again)
        text = snap["readme"]
        check("README AM62P cell", "| AM62P | 468 (main `%s`, 2026-11-25) |" % sha8 in text)
        check("checklist keeps the rest of row 9", "| 9 | website/ consistent |" in snap["checklist"]
              and "`bash tools/website-check.sh` exits 0" in snap["checklist"])
        check("ROADMAP Pi 4 terrain figure", "Pi 4 terrain, 83.06 ms at `%s`" % sha8 in snap["roadmap"])
        check("ROADMAP keeps the Pi 4 by-eye text", "docs/notes/pi4-glmark2-pictures-by-eye.md" in snap["roadmap"])
        check("data.js glmark2.pi4 commit", re.search(r'"glmark2\.pi4": \{[^}]*"commit": "%s"' % sha8,
                                                       snap["datajs"]) is not None)
        check("no absolute path written", not any(tmp in v for v in snap.values()))

        # refusals: different sha8s, a GL error (main() exits 1 on both, nothing written)
        other = "%08x" % ((int(sha8, 16) + 1) % (1 << 32))
        bad = dict(recs, pi4=cr.synth(tmp, "pi4", other))
        gle = dict(recs, imx8mp=cr.synth(tmp, "imx8mp", sha8, gl_error=True, name="gle"))
        for name, rs, word in (("mismatched sha8s", bad, "different commits"),
                               ("a GL error", gle, "GL error")):
            before = {k: open(os.path.join(tree, v), encoding="utf-8").read() for k, v in FILES.items()}
            cmd = [sys.executable, "-I", os.path.abspath(__file__), "--root", tree]
            for b in ORDER:
                cmd += ["--" + b, rs[b]]
            r = subprocess.run(cmd, capture_output=True, text=True)
            after = {k: open(os.path.join(tree, v), encoding="utf-8").read() for k, v in FILES.items()}
            check("refuses %s (exit %d)" % (name, r.returncode),
                  r.returncode == 1 and word in r.stderr and before == after, r.stderr.strip())
        # --root is required
        r = subprocess.run([sys.executable, "-I", os.path.abspath(__file__)] +
                           sum([["--" + b, recs[b]] for b in ORDER], []), capture_output=True, text=True)
        check("--root is required", r.returncode == 2, r.returncode)
        # an i.MX8MP record measured on the Armadillo-X2 (testdata/one-commit-docs/measured-on)
        x2recs = dict(recs, imx8mp=cr.synth(tmp, "imx8mp", sha8, name="x2run"))
        shutil.copy(os.path.join(HERE, "testdata", "one-commit-docs", "measured-on"), x2recs["imx8mp"])
        x2cites = dict(cites)
        x2cites["imx8mp"] = os.path.join(tmp, "records", "imx8mp", "20261125-1300-x2-%s" % sha8)
        shutil.copytree(x2recs["imx8mp"], x2cites["imx8mp"])
        tree2 = os.path.join(tmp, "tree2")
        for rel in FILES.values():
            os.makedirs(os.path.dirname(os.path.join(tree2, rel)) or tree2, exist_ok=True)
            shutil.copy(os.path.join(REPO, rel), os.path.join(tree2, rel))
        warn2 = apply(tree2, x2recs, x2cites, today="2026-11-26", out=log)
        check("X2 case: no warnings", not warn2, warn2)
        got = {k: open(os.path.join(tree2, v), encoding="utf-8").read() for k, v in FILES.items()}
        for ln in open(os.path.join(HERE, "testdata", "one-commit-docs", "x2-expect.txt"), encoding="utf-8"):
            if not ln.strip() or ln.startswith("#"):
                continue
            key, _, sub = ln.rstrip("\n").partition(" | ")
            check("X2 case: %s has %r" % (key, sub), sub in got[key.lower()], key)
        check("X2 case: the other boards' README cells carry no X2 note",
              "| AM62P | 468 (main `%s`, 2026-11-25) |" % sha8 in got["readme"]
              and got["readme"].count("Armadillo-X2") == 1)
        r = subprocess.run(args[:3] + ["--readme", os.path.join(tree2, FILES["readme"]),
                                       "--results", os.path.join(tree2, FILES["results"]),
                                       "--status-file", os.path.join(tree2, FILES["status"]),
                                       "--checklist-file", os.path.join(tree2, FILES["checklist"]),
                                       "--roadmap-file", os.path.join(tree2, FILES["roadmap"])] + args[11:],
                           capture_output=True, text=True)
        check("X2 case: readme-glmark2-check.py passes (exit %d)" % r.returncode,
              r.returncode == 0 and "ok:" in r.stdout, "\n" + r.stdout + r.stderr)
        wd2 = os.path.join(tmp, "web2")
        os.makedirs(wd2)
        shutil.copy(os.path.join(tree2, FILES["readme"]), os.path.join(wd2, "README.md"))
        shutil.copy(os.path.join(tree2, FILES["datajs"]), os.path.join(wd2, "data.js"))
        r = subprocess.run(["bash", os.path.join(REPO, "tools", "website-check.sh")],
                           capture_output=True, text=True, env=dict(os.environ, WEBSITE_GLMARK2_DIR=wd2))
        check("X2 case: website-check.sh glmark2 comparison passes (exit %d)" % r.returncode,
              r.returncode == 0, "\n" + r.stdout + r.stderr)
        check("no X2 note without the marker", "Armadillo-X2" not in text and "Armadillo-X2" not in snap["status"]
              and "Armadillo-X2" not in snap["results"] and "Armadillo-X2" not in snap["datajs"])
        log.close()
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    if fails:
        for f in fails:
            print("FAIL: " + f)
        return 1
    print("one-commit-docs.py self-test: ok (readme-glmark2-check.py --ref HEAD --scenes terrain,shadow,refract "
          "--verdicts --status --checklist --roadmap passes the edited copy at %s)" % sha8)
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", help="the tree to edit (required; the real tree only when named)")
    for b in ORDER:
        ap.add_argument("--" + b, metavar="REC", help="records/%s/common-<sha8>/" % b)
        ap.add_argument("--cite-" + b, metavar="DIR",
                        help="the dated record directory records/%s/<YYYYMMDD-HHMM>-<name>-<sha8>/" % b)
    ap.add_argument("--record-commit", metavar="SHA",
                    help="also set data.js provenance.recordCommit of the six figures (a full commit)")
    ap.add_argument("--dry-run", action="store_true", help="print what would change, write nothing")
    ap.add_argument("--self-test", action="store_true",
                    help="synthetic records at HEAD's sha8 against a copy of the six files")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.root:
        ap.error("--root DIR is required (the tree to edit)")
    missing = [b for b in ORDER if not getattr(a, b)]
    if missing:
        ap.error("missing " + ", ".join("--" + b for b in missing))
    if a.record_commit and not re.fullmatch(r"[0-9a-f]{40}", a.record_commit):
        ap.error("--record-commit wants a full 40-character commit")
    recs = {b: getattr(a, b) for b in ORDER}
    cites = {b: getattr(a, "cite_" + b) for b in ORDER if getattr(a, "cite_" + b)}
    try:
        apply(a.root, recs, cites, a.record_commit, a.dry_run)
    except Refused as e:
        print("one-commit-docs.py: refused: %s" % e, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
