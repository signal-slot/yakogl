#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2-display-record.py - format a DK display session's scan-out record.

  tools/cts/mp2-display-record.py RUNNER_LOG [--judge JUDGE_OUT] [--commit SHA]

RUNNER_LOG is tools/board-stm32mp2.sh's output (its verdict, hold and
stock-boot-file lines) with the probe's console lines (the record
directory's INDEX.txt and console.txt concatenated is fine). JUDGE_OUT is
the output of display_check/colorbars.py (or triangle.py):
"<tag>: PASS|FAIL: ...".

Prints the Markdown record block for docs/notes/stm32mp257f-dk-display-bringup.md:
commit, probe, mode and its source (EDID or fallback), the PASS/FAIL line,
the hold window, the judge verdict, the stock boot files and the return to
Linux. Exit 0 with the block (a FAIL session is a valid record: the block
says FAIL); exit 1, saying why, when a required line is missing (the commit,
the probe's PASS/FAIL line, its chain and mode lines, the runner's hold,
stock-boot-file and verdict lines, and the judge's verdict when --judge is
given); exit 2 on usage or an unreadable file.

Addresses, host names, /dev/tty* paths and USB serials never go in a file:
everything printed, a refusal message included, is redacted. Standard
library only.
"""
import re
import sys

ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")

REDACT = [
    (re.compile(r"/dev/(?:tty\w*|serial/\S+)"), "<BOARD_UART>"),
    (re.compile(r"\busb-[\w.:-]+"), "<usb-id>"),
    (re.compile(r"(?i)\b(?:serial(?:number)?|iSerial|ID_SERIAL(?:_SHORT)?)\s*[=:]\s*\S+"), "<usb-serial>"),
    (re.compile(r"\b[\w.-]+@[\w.-]+\b"), "<BOARD_HOST>"),
    (re.compile(r"\b(?:\d{1,3}\.){3}\d{1,3}(?::\d+)?\b"), "<ip>"),
    (re.compile(r"(?i)\b(?:[0-9a-f]{1,4}:){2,7}[0-9a-f]{1,4}\b"), "<ip>"),
    (re.compile(r"\b[0-9a-fA-F]{2}(?::[0-9a-fA-F]{2}){5}\b"), "<mac>"),
    (re.compile(r"\b[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.(?:local|lan|home|internal|intranet|net|com|org|io|jp|dev)\b"),
     "<board-host>"),
]


def redact(s):
    for rx, rep in REDACT:
        s = rx.sub(rep, s)
    return s


def read(path):
    try:
        with open(path, "r", errors="replace") as f:
            text = f.read()
    except OSError as e:
        print("mp2-display-record: cannot read %s: %s" % (redact(path), redact(str(e))), file=sys.stderr)
        sys.exit(2)
    return [ANSI.sub("", ln).replace("\r", "").rstrip("\n") for ln in text.split("\n")]


class Refuse(Exception):
    pass


def find(lines, rx, what):
    """The last line matching rx, as (line, match); Refuse when none."""
    hit = None
    for ln in lines:
        m = rx.search(ln)
        if m:
            hit = (ln, m)
    if hit is None:
        raise Refuse("missing line: %s" % what)
    return hit


RE_RESULT = re.compile(r"^\s*(mp2_display|mp2_gpu_scanout): (PASS|FAIL)\b.*$")
RE_VERDICT = re.compile(r"board-stm32mp2\.sh \S+ \S+ rc=(\d+) verdict=(\S+) .*boot_verdict=(\S+) "
                        r"result=(\S+) back_after=(\S+)")
RE_HOLD = re.compile(r"^hold: result seen at (\S+), holding (\d+)s")
RE_FRAMES = re.compile(r"^\s*(?:mp2_display|mp2_gpu_scanout): hold: (\d+) frames ok, (\d+) failed")
RE_STOCK = re.compile(r"stock boot files: (?:unchanged since install|no record[^\n]*)|"
                      r"the stock boot files differ[^\n]*")
RE_CHAIN = re.compile(r"(?:mp2_display|mp2_gpu_scanout): chain step (\S+) err (-?\d+), hpd (\d+), "
                      r"edid (\d+) .*choice (-?\d+)")
RE_MODE = re.compile(r"(?:mp2_display|mp2_gpu_scanout): mode (\d+x\d+) (\d+) Hz .*?, (\w+) vic (\d+)")
RE_BUILD = re.compile(r"runner: tools/board-stm32mp2\.sh build (\S+)")
RE_JUDGE = re.compile(r"^\s*(\w+): (PASS|FAIL)(?:: (.*))?$")


def build(lines, jlines, commit):
    out = {}
    if not commit:
        try:
            commit = find(lines, RE_BUILD, "runner build line")[1].group(1)
        except Refuse:
            raise Refuse("missing: the commit (no 'runner: ... build <sha>' line and no --commit)")
    out["commit"] = commit
    passed = True

    results = [m for m in (RE_RESULT.search(ln) for ln in lines) if m]
    if not results:
        raise Refuse("missing line: the probe's 'mp2_display: PASS|FAIL' (or mp2_gpu_scanout) result line")
    fails = [m for m in results if m.group(2) == "FAIL"]
    res = fails[0] if fails else results[-1]
    out["probe"] = res.group(1)
    out["result"] = res.group(0).strip()
    if fails:
        passed = False

    m = find(lines, RE_CHAIN, "the probe's 'chain step ... edid ... choice' line")[1]
    edid_ok, choice = int(m.group(4)), int(m.group(5))
    out["chain"] = "chain step %s err %s, hpd %s, edid %s, choice %s" % (
        m.group(1), m.group(2), m.group(3), m.group(4), m.group(5))
    out["source"] = "EDID" if edid_ok and choice >= 0 else "fallback (no usable EDID choice)"
    m = find(lines, RE_MODE, "the probe's 'mode WxH N Hz ... vic N' line")[1]
    out["mode"] = "%s, pixel clock %s Hz, %s, VIC %s" % (m.group(1), m.group(2), m.group(3), m.group(4))

    m = find(lines, RE_HOLD, "the runner's 'hold: result seen at ..., holding Ns' line (MP2_HOLD_S unset?)")[1]
    out["hold"] = "result seen at %s, held %s s" % (m.group(1), m.group(2))
    fr = [x for x in (RE_FRAMES.search(ln) for ln in lines) if x]
    if fr:
        out["hold"] += "; probe frames ok %s, failed %s" % (fr[-1].group(1), fr[-1].group(2))
        if int(fr[-1].group(2)) > 0:
            passed = False

    st = find(lines, RE_STOCK, "the runner's 'stock boot files: ...' line")[1]
    out["stock"] = st.group(0)
    if "differ" in out["stock"]:
        passed = False

    v = find(lines, RE_VERDICT, "the runner's verdict line 'board-stm32mp2.sh <board> <name> rc=.. verdict=..'")[1]
    out["verdict"] = "rc=%s verdict=%s boot_verdict=%s result=%s" % (v.group(1), v.group(2), v.group(3), v.group(4))
    out["back"] = "verdict=%s, Linux answered %s after the reboot" % (v.group(2), v.group(5))
    if v.group(2) not in ("done", "returned"):
        passed = False

    if jlines is None:
        out["judge"] = "none (no judge output given)"
    else:
        j = [x for x in (RE_JUDGE.search(ln) for ln in jlines) if x]
        if not j:
            raise Refuse("missing line: the judge's '<tool>: PASS|FAIL: ...' verdict in the judge file")
        out["judge"] = j[-1].group(0).strip()
        if j[-1].group(2) == "FAIL":
            passed = False
    out["passed"] = passed
    return out


def render(r):
    judge = r["judge"] if r["judge"].startswith("none") else "`%s`" % r["judge"]
    rows = [
        ("commit", r["commit"]),
        ("probe", r["probe"]),
        ("mode", "%s (source: %s)" % (r["mode"], r["source"])),
        ("chain", r["chain"]),
        ("result line", "`%s`" % r["result"]),
        ("hold window", r["hold"]),
        ("judge verdict", judge),
        ("stock boot files", r["stock"]),
        ("return to Linux", r["back"]),
        ("runner verdict", "`%s`" % r["verdict"]),
    ]
    s = "### Scan-out record: %s, %s\n\n" % (r["probe"], "PASS" if r["passed"] else "FAIL")
    s += "| item | value |\n| --- | --- |\n"
    for k, v in rows:
        s += "| %s | %s |\n" % (k, v)
    return s


def main(argv):
    path = judge = commit = None
    it = iter(argv)
    for a in it:
        if a in ("--judge", "--commit"):
            val = next(it, None)
            if val is None:
                print("mp2-display-record: %s needs a value" % a, file=sys.stderr)
                return 2
            if a == "--judge":
                judge = val
            else:
                commit = val
        elif a.startswith("-") or path:
            print("usage: mp2-display-record.py RUNNER_LOG [--judge JUDGE_OUT] [--commit SHA]", file=sys.stderr)
            return 2
        else:
            path = a
    if not path:
        print("usage: mp2-display-record.py RUNNER_LOG [--judge JUDGE_OUT] [--commit SHA]", file=sys.stderr)
        return 2
    lines = read(path)
    jlines = read(judge) if judge else None
    try:
        r = build(lines, jlines, commit)
    except Refuse as e:
        print("mp2-display-record: refused: %s" % redact(str(e)), file=sys.stderr)
        return 1
    sys.stdout.write(redact(render(r)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
