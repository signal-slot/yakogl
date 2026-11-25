#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""common-row.py - RESULTS.md / README.md / website/data.js text from one common record.

    tools/cts/common-row.py [--board B] [--cite DIR] RECORD
    tools/cts/common-row.py --self-test

RECORD is one board's common record directory, what `tools/cts/board-stage.sh
<board> common` wrote: records/<board>/common-<sha8>/ with tree.txt and one run
directory per row, <YYYYMMDD-HHMM>-GM-<sha8> (glmark2) and -GT-<sha8> (gl_test),
each with meta.txt, INDEX.txt (the verdict line, the decoded records) and, on
the AM62P and the Pi 4, console.txt. The i.MX8MP has no console: its glmark2
speed and pictures come from a glmark2-pages.txt (`tools/board.sh imx8mp gm ...
pictures`, `gm-lines`) found in the GM run directory, in RECORD, or in
RECORD's parent; its score, GL errors and gl_test count come from INDEX.txt.
The board is read from the path (records/<board>/...) or --board (am62p, pi4,
imx8mp); the commit (sha8) from the directory name common-<sha8>.

It prints (a) a verdict block ending in one line 'VERDICT: pass' or 'VERDICT:
fail <reasons>', (b) the RESULTS.md 'Common set' row, (c) the README table
cell, (d) the website/data.js figure fields. The verdict needs 33 scenes, 0
failed, 0 GL errors, 0 out-of-memory scenes, every scene's picture present,
not black and with a reference in tools/cts/glmark2-pictures.txt (a probe off
by more than the tolerance is counted and listed, not failed: animated
scenes), both rows verdict=done, and gl_test with every run case passed.
terrain, shadow and refract are recorded, never judged. Exit 0 on pass, 1 on
fail, 2 on a record it cannot read.

The row cites the record as records/<board>/<...>/. tools/cts/readme-glmark2-check.py
--scenes only accepts records/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/ directly
under the board: a common-<sha8>/ record is cited as it is and a note says so;
--cite DIR adds a directory of that form (the i.MX8MP per-scene record).
Paths printed are relative to records/; no host, address or credential is read.
"""
import argparse
import importlib.util
import os
import re
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
BOARDS = {"am62p": "AM62P", "imx8mp": "i.MX8MP", "pi4": "Pi 4"}
SLOW = ("terrain", "shadow", "refract")
WANT_SCENES = 33
RUN_RE = re.compile(r"^(\d{8})-(\d{4})-(GM|GT)-([0-9a-f]{8}|probe)$")
CITE_RE = re.compile(r"records/(am62p|pi4|imx8mp)/\d{8}-\d{4}-[A-Za-z0-9._-]+-[0-9a-f]{8}/?$")


def load_tally():
    spec = importlib.util.spec_from_file_location("common_tally", os.path.join(HERE, "common-tally.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["common_tally"] = mod
    spec.loader.exec_module(mod)
    return mod


def rd(path):
    try:
        with open(path, "rb") as f:
            return f.read().decode("utf-8", errors="replace")
    except OSError:
        return None


def rel(path):
    """path from its 'records' component on (never an absolute path)"""
    parts = os.path.abspath(path).split(os.sep)
    if "records" in parts:
        i = len(parts) - 1 - parts[::-1].index("records")
        return "/".join(parts[i:])
    return "/".join(["records"] + parts[-2:])


def kv(line):
    out = {}
    for tok in line.split():
        k, sep, v = tok.partition("=")
        if sep and k not in out:
            out[k] = v
    return out


def index_verdict(rundir):
    """(verdict, resets) from INDEX.txt's 'verdict:' line; None when absent"""
    text = rd(os.path.join(rundir, "INDEX.txt")) or ""
    m = re.search(r"^verdict: (.*)$", text, re.M)
    if not m:
        return None, None, text
    v = kv(m.group(1))
    return v.get("verdict"), v.get("resets"), text


def fmt_ms(v):
    return "%.2f" % v


def analyse(record, board, ct, cite=None):
    """everything the output needs, from a record directory; raises ValueError for an unreadable one"""
    record = record.rstrip("/")
    m = re.match(r"^(?:\d{8}-\d{4}-)?[A-Za-z0-9._]*?-?([0-9a-f]{8})$", os.path.basename(record))
    sha8 = m.group(1) if m else None
    if sha8 is None:
        raise ValueError("%s: the directory name has no <sha8> (common-<sha8>)" % os.path.basename(record))
    runs = {}
    for n in sorted(os.listdir(record)):
        r = RUN_RE.match(n)
        if r and os.path.isdir(os.path.join(record, n)):
            runs[r.group(3)] = os.path.join(record, n)  # the last run of a row
    for lab in ("GM", "GT"):
        if lab not in runs:
            raise ValueError("%s has no %s run directory" % (rel(record), lab))
    d = {"board": board, "sha8": sha8, "record": record, "fails": [], "notes": [],
         "gm": runs["GM"], "gt": runs["GT"]}
    date = RUN_RE.match(os.path.basename(runs["GM"])).group(1)
    d["date"] = "%s-%s-%s" % (date[:4], date[4:6], date[6:])
    fails = d["fails"]

    # ---- glmark2
    gv, gres, gidx = index_verdict(runs["GM"])
    d["gm_verdict"], d["resets"] = gv, gres
    if gv != "done":
        fails.append("GM verdict=%s" % gv)
    console = rd(os.path.join(runs["GM"], "console.txt"))
    pages = None
    if console is None:
        for p in (runs["GM"], record, os.path.dirname(record)):
            t = rd(os.path.join(p, "glmark2-pages.txt"))
            if t is not None:
                pages = t
                d["pages"] = rel(os.path.join(p, "glmark2-pages.txt"))
                break
    text = console if console is not None else pages
    if text is None:
        raise ValueError("%s has neither console.txt nor glmark2-pages.txt" % rel(runs["GM"]))
    sp = ct.glmark2_speed(text)
    scenes = sp["scenes"]
    d["scenes"] = scenes
    d["nscenes"] = len(scenes)
    if len(scenes) != WANT_SCENES:
        fails.append("%d scenes, want %d" % (len(scenes), WANT_SCENES))
    score = sp["score"]
    sm = re.search(r"glmark2: score (\d+): (\d+) scenes ran, (\d+) failed, (\d+) unsupported", text or "")
    im = ct.GLMARK2_RE.search(gidx) if hasattr(ct, "GLMARK2_RE") else None
    if score is None and im:
        score = int(im.group(1))
    d["score"] = score
    if score is None:
        fails.append("no glmark2 score")
    if sm:
        failed, unsup = int(sm.group(3)), int(sm.group(4))
    elif im:
        failed, unsup = int(im.group(5)), int(im.group(6))
    else:
        failed = unsup = None
    nofps = [s["name"] for s in scenes if s["fps"] is None]
    d["failed"] = failed if failed is not None else len(nofps)
    d["unsupported"] = unsup or 0
    if d["failed"] or nofps or d["unsupported"]:
        fails.append("%d failed/unsupported scene(s)%s" % (max(d["failed"], len(nofps)) + d["unsupported"],
                                                           ": " + ", ".join(nofps) if nofps else ""))
    if console is not None:
        gs = ct.glmark2_scenes(console)
        cls = {}
        for counts in list(gs["scenes"].values()) + [gs["outside"]]:
            for k, v in counts.items():
                cls[k] = cls.get(k, 0) + v
        d["gl_errors"] = cls.get("gl-error", 0)
        d["oom"] = sorted(s for s, c in gs["scenes"].items()
                          if c.get("out-of-memory") or c.get("tiled-copy"))
        d["oom_outside"] = gs["outside"].get("out-of-memory", 0) + gs["outside"].get("tiled-copy", 0)
        d["other_bad"] = {k: v for k, v in cls.items() if k not in ("gl-error", "out-of-memory", "tiled-copy")}
    else:
        em = re.search(r"GL errors raised (\d+)", gidx)
        d["gl_errors"] = int(em.group(1)) if em else None
        d["oom"], d["oom_outside"], d["other_bad"] = [], 0, {}
        d["notes"].append("no console: GL errors and out-of-memory come from INDEX.txt, not per scene")
        if d["gl_errors"] is None:
            fails.append("GL errors not recorded (no console, none in INDEX.txt)")
    if d["gl_errors"]:
        fails.append("%d GL error line(s)" % d["gl_errors"])
    if d["oom"] or d["oom_outside"]:
        fails.append("out-of-memory in %s" % (", ".join(d["oom"]) or "no scene"))
    if d["other_bad"]:
        d["notes"].append("other known-bad console lines (not judged): %s" % ", ".join(
            "%s %d" % kvp for kvp in sorted(d["other_bad"].items())))

    # ---- pictures
    table = ct.read_picture_table()
    judged = ct.judge_pictures(ct.glmark2_pictures(text), table, board)
    d["pic"] = {}
    if judged is None:
        d["pic_states"] = {}
        fails.append("no picture lines")
    else:
        d["pic_states"] = ct.picture_states(judged)
        d["pic"] = judged["scenes"]
        npic = len(judged["order"])
        bad = {k: v for k, v in d["pic_states"].items()
               if k in ("black", "absent", "unreadable", "no-reference", "not-in-record")}
        if npic != WANT_SCENES:
            fails.append("%d picture lines, want %d" % (npic, WANT_SCENES))
        if bad:
            fails.append("pictures: " + ", ".join("%s %d" % kvp for kvp in sorted(bad.items())))
        d["mismatch"] = sorted(s for s, (st, _) in judged["scenes"].items() if st == "mismatch")

    # ---- slow scenes (recorded, not judged)
    d["slow"] = {}
    for key in SLOW:
        sc = next((s for s in scenes if s["name"].startswith("[%s]" % key)), None)
        d["slow"][key] = sc
        if sc is None or sc["ms"] is None:
            fails.append("%s has no ms/frame" % key)

    # ---- gl_test
    tv, _, tidx = index_verdict(runs["GT"])
    d["gt_verdict"] = tv
    if tv != "done":
        fails.append("GT verdict=%s" % tv)
    tcon = rd(os.path.join(runs["GT"], "console.txt"))
    passed = total = None
    failing = []
    if tcon is not None:
        g = ct.gl_test_from_console(tcon)
        passed, total, failing = g["passed"], g["total"], g["failing"]
    sm2 = re.search(r"set (?:passed|failed)\) passed (\d+) of (\d+) run; skipped (\d+).*?failing (\d+)", tidx)
    d["skipped"] = None
    if passed is None and sm2:
        passed, total = int(sm2.group(1)), int(sm2.group(2))
        d["skipped"] = int(sm2.group(3))
        if int(sm2.group(4)):
            failing = ["?"]
    d["gt_passed"], d["gt_total"], d["gt_failing"] = passed, total, failing
    if passed is None:
        fails.append("no gl_test result")
    elif passed != total or failing:
        fails.append("gl_test %d of %d passed%s" % (passed, total, ", failing " + ",".join(map(str, failing)) if failing else ""))
    d["cite"] = cite
    return d


def slow_text(d):
    out = []
    for k in SLOW:
        sc = d["slow"][k]
        if sc is None or sc["ms"] is None:
            out.append("%s absent" % k)
        else:
            out.append("%s %s ms (%d FPS)" % (k, fmt_ms(sc["ms"]), sc["fps"]))
    return ", ".join(out)


def gt_text(d):
    if d["gt_passed"] is None:
        return "no result"
    s = "%d of %d" % (d["gt_passed"], d["gt_total"])
    if d["skipped"]:
        s += " run passed, %d skipped" % d["skipped"]
    return s


def results_row(d):
    pics = d["pic_states"]
    npic = sum(pics.values())
    ptxt = "pictures against `tools/cts/glmark2-pictures.txt` column `%s`: %d scenes, match %d" % (
        d["board_key"], npic, pics.get("match", 0))
    for k in ("mismatch", "black", "absent", "unreadable", "no-reference", "not-in-record"):
        if pics.get(k):
            ptxt += ", %s %d" % (k, pics[k])
    if d["mismatch"]:
        ptxt += " (%s)" % ", ".join(s.split("]")[0].lstrip("[") for s in d["mismatch"])
    errs = "0 GL errors" if d["gl_errors"] == 0 else "%s GL errors" % d["gl_errors"]
    cite = rel(d["record"]) + "/" + os.path.basename(d["gm"]) + "/"
    gm = "%s, %d scenes, %d failed, %s, %d out-of-memory scenes; main `%s`, %s (`%s`, GM `verdict=%s`, resets %s)" % (
        d["score"], d["nscenes"], d["failed"], errs, len(d["oom"]), d["sha8"], d["date"], cite,
        d["gm_verdict"], d["resets"] if d["resets"] is not None else "absent")
    if d["cite"]:
        gm += "; per-scene record `%s`" % d["cite"]
    gm += "; **%s**; %s" % (slow_text(d), ptxt)
    gt = "%s (GT `%s`, `verdict=%s`)" % (gt_text(d), rel(d["record"]) + "/" + os.path.basename(d["gt"]) + "/",
                                         d["gt_verdict"])
    return "| %s | %s | %s | %s |" % (BOARDS[d["board_key"]], gm, gt, d["resets"] if d["resets"] is not None else "absent")


def readme_cell(d):
    return "| %s | %s (main `%s`, %s) |" % (
        {"am62p": "AM62P", "pi4": "Raspberry Pi 4", "imx8mp": "i.MX8MP"}[d["board_key"]], d["score"], d["sha8"], d["date"])


def js(v):
    return '"%s"' % v.replace("\\", "\\\\").replace('"', '\\"')


def data_js(d):
    b = d["board_key"]
    status = "pass" if not d["fails"] and not d["mismatch"] else "warn"
    note = "Main %s. Per frame: %s." % (d["sha8"], slow_text(d).replace(" ms (", " ms (").replace(")", ")", 1))
    if d["mismatch"]:
        note += " %d of %d pictures differ in a probe from the reference (%s)." % (
            len(d["mismatch"]), sum(d["pic_states"].values()),
            ", ".join(s.split("]")[0].lstrip("[") for s in d["mismatch"]))
    gstat = "pass" if not d["fails"] and not d["gt_failing"] else "warn"
    return "\n".join([
        '    "glmark2.%s": {' % b,
        '      "value": %s,' % d["score"],
        '      "unit": %s,' % js("score at 800x600; %d scenes, %d failed, %d GL errors" % (
            d["nscenes"], d["failed"], d["gl_errors"] or 0)),
        '      "commit": %s,' % js(d["sha8"]),
        '      "onMain": true,',
        '      "date": %s,' % js(d["date"]),
        '      "status": %s,' % js(status),
        '      "note": %s,' % js(note),
        '    },',
        '    "gltest.%s": {' % b,
        '      "value": %s,' % js(gt_text(d).replace(" run passed, ", " / ", 0) if d["gt_passed"] is None else
                                  "%d / %d" % (d["gt_passed"], d["gt_total"])),
        '      "unit": %s,' % js("cases passed" + ("; %d skipped" % d["skipped"] if d["skipped"] else "")),
        '      "commit": %s,' % js(d["sha8"]),
        '      "onMain": true,',
        '      "date": %s,' % js(d["date"]),
        '      "status": %s,' % js(gstat),
        '    },'])


def verdict_block(d):
    pics = d["pic_states"]
    L = ["== common record %s (%s, main %s) ==" % (rel(d["record"]), BOARDS[d["board_key"]], d["sha8"]),
         "scenes: %d (want %d)" % (d["nscenes"], WANT_SCENES),
         "failed scenes: %d" % d["failed"],
         "GL errors: %s" % ("not recorded" if d["gl_errors"] is None else d["gl_errors"]),
         "out-of-memory scenes: %d%s" % (len(d["oom"]), " (%s)" % ", ".join(d["oom"]) if d["oom"] else ""),
         "glmark2 score: %s; GM verdict=%s, resets %s" % (d["score"], d["gm_verdict"], d["resets"]),
         "gl_test: %s; GT verdict=%s" % (gt_text(d), d["gt_verdict"]),
         "pictures (glmark2-pictures.txt column %s): %s" % (
             d["board_key"], ", ".join("%s %d" % kvp for kvp in sorted(pics.items())) or "none")]
    for s in d["mismatch"]:
        L.append("  picture mismatch: %s: %s" % (s, d["pic"][s][1]))
    L.append("recorded, not judged: " + slow_text(d))
    if "pages" in d:
        L.append("per-scene source: %s (ms = 1000 / integer FPS)" % d["pages"])
    for n in d["notes"]:
        L.append("note: " + n)
    cites = [d["cite"]] if d["cite"] else [rel(d["record"]) + "/"]
    if not any(CITE_RE.search(c) for c in cites):
        L.append("note: readme-glmark2-check.py --scenes wants a record directory records/%s/<YYYYMMDD-HHMM>-<name>-%s/ "
                 "under the board; %s is not of that form (see --cite)" % (d["board_key"], d["sha8"], cites[0]))
    L.append("VERDICT: pass" if not d["fails"] else "VERDICT: fail " + "; ".join(d["fails"]))
    return "\n".join(L)


def render(record, board=None, cite=None):
    ct = load_tally()
    parts = os.path.abspath(record).split(os.sep)
    key = board
    if key is None:
        key = next((p for p in parts if p in BOARDS), None)
    if key not in BOARDS:
        raise ValueError("the board is not in the path (records/<board>/...): pass --board am62p|pi4|imx8mp")
    d = analyse(record, key, ct, cite)
    d["board_key"] = key
    out = [verdict_block(d), "",
           "-- RESULTS.md 'Common set' row --", results_row(d), "",
           "-- README.md glmark2 table row --", readme_cell(d), "",
           "-- website/data.js --", data_js(d)]
    return "\n".join(out), d


# ---------------------------------------------------------------- self test
def synth(root, board, sha8, scenes=33, gl_error=False, name="common"):
    base = os.path.join(root, "records", board, "20261125-1200-%s-%s" % (name, sha8)
                        if name != "common" else "common-%s" % sha8)
    gm = os.path.join(base, "20261125-1200-GM-aaaaaaaa")
    gt = os.path.join(base, "20261125-1210-GT-bbbbbbbb")
    os.makedirs(gm)
    os.makedirs(gt)
    ref = {}
    for line in open(os.path.join(HERE, "glmark2-pictures.txt"), encoding="utf-8"):
        p = [x.strip() for x in line.rstrip("\n").split(" | ", 2)]
        if len(p) == 3 and p[0] == board:
            ref[p[1]] = p[2]
    infos = list(ref)[:WANT_SCENES]
    assert len(infos) == WANT_SCENES, "glmark2-pictures.txt has %d scenes for %s" % (len(ref), board)
    infos = infos[:scenes]
    con = []
    for i, info in enumerate(infos):
        sc = info.split("]")[0].lstrip("[")
        fps = {"terrain": 13, "shadow": 95, "refract": 29}.get(sc, 100)
        ms = {"terrain": "83.060", "shadow": "10.620", "refract": "35.090"}.get(sc, "10.000")
        con.append("glmark2-mem: entry %d before setup: x" % i)
        if gl_error and i == 3:
            con.append("GL error 0x502 in glGenerateMipmap")
        con.append("glmark2-mem: entry %d end: x" % i)
        con.append("%s: FPS: %d FrameTime: %s ms" % (info, fps, ms))
        con.append("glmark2-picture: %s: %s" % (info, ref[info]))
    con.append("glmark2 Score: 468")
    con.append("glmark2: score 468: %d scenes ran, 0 failed, 0 unsupported" % len(infos))
    with open(os.path.join(gm, "console.txt"), "w") as f:
        f.write("\n".join(con) + "\n")
    v = "board.sh %s verdict=done rc=0 result=x record=a,b,c,d resets=0" % board
    for d in (gm, gt):
        with open(os.path.join(d, "meta.txt"), "w") as f:
            f.write("name=x\n")
        with open(os.path.join(d, "INDEX.txt"), "w") as f:
            f.write("verdict: %s\nrecords:\n  GLMARK2 score 468 status 0 (ok) scenes ran 33 failed 0 unsupported 0\n" % v)
    with open(os.path.join(gt, "console.txt"), "w") as f:
        f.write("gl_test: all: 0 of 50 cases failed (50 passed; 0 timed out; 0 skipped)\n")
    return base


def self_test():
    ct = load_tally()
    fails = []

    def check(name, cond, got=None):
        if not cond:
            fails.append("%s%s" % (name, "" if got is None else ": got %r" % (got,)))

    root = tempfile.mkdtemp(prefix="common-row-", dir=os.environ.get("TMPDIR") or None)
    try:
        # a pass, in the dated layout readme-glmark2-check.py reads, and in board-stage.sh's own
        base = synth(root, "am62p", "abcdef12", name="common")
        out, d = render(base)
        check("verdict pass", out.rstrip().split("\n")[-1] == "VERDICT: pass" or
              "\nVERDICT: pass" in out, [l for l in out.split("\n") if "VERDICT" in l])
        for want in ("scenes: 33 (want 33)", "failed scenes: 0", "GL errors: 0", "out-of-memory scenes: 0",
                     "gl_test: 50 of 50", "match 33"):
            check("block has %r" % want, want in out)
        check("slow figures", "terrain 83.06 ms (13 FPS), shadow 10.62 ms (95 FPS), refract 35.09 ms (29 FPS)" in out)
        check("no absolute path", os.sep + "records" not in out.replace("records/", "") and root not in out)
        check("note for common-<sha8>", "is not of that form" in out)
        check("score cell", "| AM62P | 468 (main `abcdef12`, 2026-11-25) |" in out)
        check("data.js", '"commit": "abcdef12"' in out and '"value": 468,' in out and '"value": "50 / 50",' in out)
        # the row passes readme-glmark2-check.py's rules: --scenes figures, a record dir at the commit
        dated = synth(root, "pi4", "abcdef12", name="dated")
        cite = "records/pi4/20261125-1200-dated-abcdef12/"
        out2, d2 = render(dated, cite=cite)
        row = next(l for l in out2.split("\n") if l.startswith("| Pi 4 |"))
        spec = importlib.util.spec_from_file_location("rgc", os.path.join(HERE, "readme-glmark2-check.py"))
        rgc = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(rgc)
        cells = [c.strip() for c in row.strip().strip("|").split("|")]
        check("row board parses", rgc.BOARDS.get(rgc.norm(cells[0])) == "Raspberry Pi 4", cells[0])
        text = " ".join(cells[1:]).replace("*", "")
        check("row has the commit hash", [h for h in rgc.HASH.findall(text) if h == "abcdef12"])
        check("row passes scene_fails", rgc.scene_fails("Raspberry Pi 4", text, ["terrain", "shadow", "refract"],
                                                         "abcdef12") == [],
              rgc.scene_fails("Raspberry Pi 4", text, ["terrain", "shadow", "refract"], "abcdef12"))
        check("--cite silences the note", "is not of that form" not in out2)
        # 32 scenes, a GL error, a failed GT
        o3, _ = render(synth(root, "am62p", "00000032", scenes=32, name="s32"))
        check("32 scenes fails", "VERDICT: fail" in o3 and "32 scenes, want 33" in o3)
        o4, _ = render(synth(root, "am62p", "00000042", gl_error=True, name="gle"))
        check("GL error fails", "VERDICT: fail" in o4 and "1 GL error line(s)" in o4)
        b5 = synth(root, "am62p", "00000052", name="gt")
        with open(os.path.join(b5, "20261125-1210-GT-bbbbbbbb", "console.txt"), "w") as f:
            f.write("gl_test: all: 1 of 50 cases failed (49 passed; 0 timed out; 0 skipped)\n"
                    "gl_test: case 20 etc: FAIL\n")
        o5, _ = render(b5)
        check("gl_test failure fails", "VERDICT: fail" in o5 and "49 of 50" in o5)
        # no console: INDEX.txt and glmark2-pages.txt (the i.MX8MP)
        b6 = synth(root, "imx8mp", "00000062", name="imx")
        con = rd(os.path.join(b6, "20261125-1200-GM-aaaaaaaa", "console.txt"))
        os.remove(os.path.join(b6, "20261125-1200-GM-aaaaaaaa", "console.txt"))
        os.remove(os.path.join(b6, "20261125-1210-GT-bbbbbbbb", "console.txt"))
        pages = "glmark2-pages: x\n" + "\n".join(
            l.replace("FPS: %s FrameTime: " % l.split("FPS: ")[1].split()[0], "FPS: 4 FrameTime: ")
            if "FPS: " in l and "glmark2-picture" not in l else l
            for l in con.split("\n") if "glmark2-mem" not in l and "GL error" not in l)
        with open(os.path.join(b6, "glmark2-pages.txt"), "w") as f:
            f.write(pages)
        with open(os.path.join(b6, "20261125-1200-GM-aaaaaaaa", "INDEX.txt"), "a") as f:
            f.write("  GL errors raised 0\n")
        with open(os.path.join(b6, "20261125-1210-GT-bbbbbbbb", "INDEX.txt"), "a") as f:
            f.write("  CASE (1) seq 9: set status 0 (set passed) passed 49 of 49 run; skipped 1, first skip case 16; "
                    "failing 0: first case none, second case none\n")
        o6, _ = render(b6)
        check("i.MX8MP record", "VERDICT: pass" in o6 and "49 of 49 run passed, 1 skipped" in o6
              and "terrain 250.00 ms (4 FPS)" in o6, o6)
    finally:
        shutil.rmtree(root, ignore_errors=True)
    if fails:
        for f in fails:
            print("FAIL: " + f)
        return 1
    print("common-row.py self-test: ok")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0],
                                 epilog="RECORD: records/<board>/common-<sha8>/ as board-stage.sh wrote it.")
    ap.add_argument("record", nargs="?", metavar="RECORD",
                    help="one board's common record directory, records/<board>/common-<sha8>/")
    ap.add_argument("--board", choices=sorted(BOARDS), help="the board, when the path does not say")
    ap.add_argument("--cite", help="also cite this directory (records/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/)")
    ap.add_argument("--self-test", action="store_true", help="build a synthetic record and check the output")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.record:
        ap.error("RECORD is required (the common record directory)")
    try:
        out, d = render(a.record, a.board, a.cite)
    except (ValueError, OSError) as e:
        print("common-row.py: %s" % e, file=sys.stderr)
        return 2
    print(out)
    return 0 if not d["fails"] else 1


if __name__ == "__main__":
    sys.exit(main())
