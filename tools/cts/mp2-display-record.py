#!/usr/bin/env python3
"""Format a DK display session's scan-out record from a runner log.

usage: mp2-display-record.py RUNNER_LOG [JUDGE_OUTPUT]

RUNNER_LOG is what tools/board-stm32mp2.sh printed (or its INDEX.txt plus the
note lines); JUDGE_OUTPUT is display_check/colorbars.py's (or triangle.py's)
output.  Prints one Markdown record block in the shape of the bring-up
note's scan-out record rows: commit, probe, mode and its source, the probe's
PASS/FAIL line, the hold window, the judge's verdict, the runner's stock-boot-
file check and its return-to-Linux verdict.

Exit 0: block printed.  1: a required line is missing (the reason on stderr,
nothing on stdout).  2: usage or unreadable input.

Everything printed is redacted: IP addresses, host names (user@host and
host=/hostname=), /dev/tty* paths and USB serial numbers never reach a record.
Python 3 standard library only.
"""
import re
import sys

PROBES = ("mp2_display", "mp2_gpu_scanout")

REDACT = [
    (re.compile(r"/dev/tty[A-Za-z0-9_./-]*"), "<BOARD_UART>"),
    (re.compile(r"/dev/serial/by-id/\S+"), "<BOARD_UART>"),
    (re.compile(r"\busb-[A-Za-z0-9_.:-]+"), "<usb-serial>"),
    (re.compile(r"(?i)\b(serial(?:[ _-]?(?:number|no))?|ID_SERIAL(?:_SHORT)?|iSerial)(\s*[=:]\s*|\s+)(?!<)[A-Za-z0-9_.:-]{4,}"),
     r"\1\2<usb-serial>"),
    (re.compile(r"\b[\w.+-]+@[A-Za-z0-9_.-]+"), "<BOARD_HOST>"),
    (re.compile(r"(?i)\b(host(?:name)?|BOARD_HOST|ssh)(\s*[=:]\s*)(?!<)[A-Za-z0-9_.-]+"),
     r"\1\2<board-host>"),
    (re.compile(r"\b(?:\d{1,3}\.){3}\d{1,3}\b"), "<ip>"),
    (re.compile(r"(?i)(?<![0-9a-f:])(?:[0-9a-f]{1,4}:){2,7}[0-9a-f]{1,4}(?![0-9a-f:])"), "<ip>"),
    (re.compile(r"(?i)(?<![0-9a-f:])(?:[0-9a-f]{1,4}:){0,7}[0-9a-f]{0,4}::(?:[0-9a-f]{1,4}(?::[0-9a-f]{1,4}){0,6})?(?![0-9a-f:])"), "<ip>"),
    (re.compile(r"(?i)\b[a-z0-9-]+(?:\.[a-z0-9-]+)*\.(?:local|lan|internal|home|corp|ts\.net|net|com|org)\b"),
     "<board-host>"),
]


def redact(s):
    for rx, rep in REDACT:
        s = rx.sub(rep, s)
    return s


def die(msg, code=1):
    sys.stderr.write("mp2-display-record: %s\n" % msg)
    sys.exit(code)


def read(path):
    try:
        with open(path, errors="replace") as f:
            return f.read().splitlines()
    except OSError as e:
        die("cannot read %s: %s" % (path, e.strerror), 2)


def last(rx, lines):
    m = None
    for ln in lines:
        mm = rx.search(ln)
        if mm:
            m = (ln, mm)
    return m


def main(argv):
    if len(argv) not in (2, 3):
        die("usage: mp2-display-record.py RUNNER_LOG [JUDGE_OUTPUT]", 2)
    lines = [l.strip() for l in read(argv[1])]
    judge = [l.strip() for l in read(argv[2])] if len(argv) == 3 else None
    missing = []

    # commit: "...: <name>: build <describe> boot ..."
    c = last(re.compile(r"build (\S+) boot (\S+)"), lines)
    if not c:
        missing.append("the runner's 'build <commit> boot <backend>' line")
    commit, boot = (c[1].group(1), c[1].group(2)) if c else ("", "")

    # probe result line
    pr = last(re.compile(r"\b(%s): (PASS|FAIL)\b(.*)" % "|".join(PROBES)), lines)
    if not pr:
        missing.append("the probe's '<probe>: PASS|FAIL' result line")
    probe = pr[1].group(1) if pr else None
    verdict = pr[1].group(2) if pr else None
    result_line = pr[0] if pr else ""
    pre = re.escape(probe) if probe else "(?:%s)" % "|".join(PROBES)

    mode = last(re.compile(r"\b%s: mode (\d+)x(\d+) (\d+) Hz(?: h [\d/]+ v [\d/]+ flags \S+, (\w+) vic (\d+))?" % pre), lines)
    chain = last(re.compile(r"\b%s: chain step (\S+) err (-?\d+), hpd (\d+), edid (\d+)" % pre), lines)
    if not mode:
        missing.append("the probe's 'mode WxH N Hz' line")
    if not chain:
        missing.append("the probe's 'chain step ... edid N' line (the mode's source)")
    source = ""
    if chain:
        source = "EDID" if chain[1].group(4) == "1" else "fallback (no valid EDID)"
    mode_s = ""
    if mode:
        m = mode[1]
        mode_s = "%sx%s, pixel clock %s Hz" % (m.group(1), m.group(2), m.group(3))
        if m.group(5):
            mode_s += ", %s vic %s" % (m.group(4), m.group(5))

    hold = last(re.compile(r"hold: result seen at (\S+), holding (\d+)s"), lines)
    if not hold:
        missing.append("the runner's 'hold: result seen at <time>, holding <s>s' line (run with MP2_HOLD_S)")
    frames = last(re.compile(r"\bhold: (\d+) frames ok, (\d+) failed"), lines)

    stock = last(re.compile(r"stock boot files: (.*)"), lines)
    if not stock:
        missing.append("the runner's 'stock boot files: ...' line")

    vl = last(re.compile(r"verdict=(\S+) .*?boot_verdict=(\S+) result=(\S+) back_after=(\S+)"), lines)
    if not vl:
        missing.append("the runner's verdict line (verdict=... back_after=...: the return to Linux)")

    if missing:
        die("refused, required line(s) missing: " + "; ".join(missing))

    if judge is None:
        j = "not judged (no judge output given)"
    else:
        jm = last(re.compile(r"^(\w+): (PASS|FAIL)\b.*"), judge)
        if not jm:
            die("refused, the judge output has no '<name>: PASS|FAIL' line")
        j = jm[0]

    back = vl[1]
    held = "%ss from the result line, seen at %s" % (hold[1].group(2), hold[1].group(1))
    if frames:
        held += "; %s frames ok, %s failed" % (frames[1].group(1), frames[1].group(2))
    out = [
        "### DK scan-out record: %s" % probe,
        "",
        "| item | value |",
        "| --- | --- |",
        "| commit | `%s` |" % commit,
        "| probe | `%s` (boot %s) |" % (probe, boot),
        "| mode | %s |" % mode_s,
        "| mode source | %s |" % source,
        "| probe line | `%s` |" % result_line,
        "| probe verdict | %s |" % verdict,
        "| hold | %s |" % held,
        "| judge | `%s` |" % j,
        "| stock boot files | %s |" % stock[1].group(1),
        "| return to Linux | verdict=%s, boot_verdict=%s, result=%s, back after %s |" % (
            back.group(1), back.group(2), back.group(3), back.group(4)),
        "",
    ]
    sys.stdout.write(redact("\n".join(out) + "\n"))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
