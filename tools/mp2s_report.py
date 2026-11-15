#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Report builder for a STM32MP257F-DK standalone board record (plan #362).

    python3 -I tools/mp2s_report.py RECORD_DIR

RECORD_DIR is records/stm32mp257f-dk/<stamp>-standalone-...-<sha8>/ (the
name shape of tools/board.sh's record directories, <date>-<time>-<name>-
<sha8>). The report goes to stdout as Markdown:

  (a) the record name and the commit sha8 taken from it;
  (b) the verdict lines of every *verdict*.txt and *.verdict file: each line
      that starts with "verdict:", "PASS", "FAIL", "ok " or "not ok ";
  (c) the console lines of docs/notes/stm32mp257f-dk-standalone.md section 7
      ("mp2s: ..." lines), ST's "SCRIPT FAILED: continuing..." and the first
      Linux login prompt after "boots left 0", each with its file and line
      number, read from the record's console log (*.log). "boots left 1",
      "boots left 0" and the login prompt are required;
  (d) the sha256 comparison of the stock boot files from a manifest-before /
      manifest-after pair (sha256sum format) or else a *manifest*diff* file
      (a unified diff of sha256sum lines; no "-"/"+" entry means identical):
      "stock boot files byte-identical: yes/no" and the files that differ.
      Our own files (under yakogl/standalone/) are listed apart, not counted;
  (e) draft comment texts for issues #1281 and #7. Nothing is posted.

The record layout is not final: every file-name pattern is a constant below.

Redaction: IP addresses (v4 and v6), MAC addresses, host names (the login
prompt's host, user@host, dotted names with a network suffix), /dev/tty* and
/dev/serial/* paths, USB serial numbers ("serial: ...", "SerialNumber=...")
and the literal values of BOARD_HOST, BOARD_UART and BOARD_SERIAL from the
environment are replaced with <redacted>; the report's last line counts the
replacements.

Exit: 0 for a complete record (also when it shows a failure: the verdict is
in the text), 3 when a part is missing (each printed as "missing: <what>"),
64 for a usage error.
"""
import glob
import ipaddress
import os
import re
import sys

# Record layout (not final: plan #362's session-driver fixes it).
VERDICT_GLOBS = ("*verdict*.txt", "*.verdict")
CONSOLE_GLOBS = ("*.log",)
MANIFEST_BEFORE_GLOBS = ("*manifest-before*",)
MANIFEST_AFTER_GLOBS = ("*manifest-after*",)
MANIFEST_DIFF_GLOBS = ("*manifest*diff*",)
# Files the runner itself puts under /boot: not stock boot files.
OWN_FILE_RX = re.compile(r"(^|/)yakogl/standalone(/|$)")
# <stamp>-standalone[-...]-<sha8>
NAME_RX = re.compile(r"^(?P<stamp>.+?)-standalone(?:-.*)?-(?P<sha8>[0-9a-f]{8})$")

VERDICT_PREFIXES = ("verdict:", "PASS", "FAIL", "ok ", "not ok ")

# Section 7 "Console lines" of docs/notes/stm32mp257f-dk-standalone.md, with
# the wording of driver/tools/mp2-standalone/mp2-standalone.cmd.
CONTRACT_RX = [
    re.compile(p)
    for p in (
        r"mp2s: undo: setenv mp2s_left 0; saveenv; boot",
        r"mp2s: no default set, back to the distro loop",
        r"mp2s: refused: .*, booting Linux",
        r"mp2s: default /yakogl/standalone/default\.bin, boots left \d+",
        r"mp2s: default /yakogl/standalone/default\.bin, forever",
        r"mp2s: saveenv failed, booting Linux",
        r"mp2s: default used up, booting Linux",
        r"mp2s: loaded \S+ bytes at \S+, RAM footprint 0x[0-9a-fA-F]+ of the \S+-byte window",
        r"mp2s: go \S+",
        r"mp2s: the image returned, booting Linux",
        r"SCRIPT FAILED: continuing\.\.\.",
    )
]
LEFT1_RX = re.compile(r"mp2s: default \S+, boots left 1\b")
LEFT0_RX = re.compile(r"mp2s: default \S+, boots left 0\b")
# A console line may carry mp2s_console_watch.py's "<unix time> " stamp.
STAMP = r"(?:\d+(?:\.\d+)?\s+)?"
LOGIN_RX = re.compile(r"^\s*" + STAMP + r"(\S+) login:")

SHA_LINE_RX = re.compile(r"^([0-9a-fA-F]{64}) [ *](.+)$")

REDACTED = "<redacted>"
ANSI_RX = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|[\x00-\x08\x0b-\x1f\x7f]")
IPV4_RX = re.compile(r"(?<![\d.])(?:\d{1,3}\.){3}\d{1,3}(?!\d)(?!\.\d)")
IPV6_RX = re.compile(r"(?<![\w:])[0-9A-Fa-f:]*::?[0-9A-Fa-f:.]*[0-9A-Fa-f](?![\w:])")
MAC_RX = re.compile(r"(?<![\w:])(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}(?![\w:])")
TTY_RX = re.compile(r"/dev/(?:tty|serial/|cu\.)\S*")
SERIAL_RX = re.compile(r"(?i)(\b(?:i?serial(?:[ _]?(?:number|no))?|sn)\s*[:=]\s*)([^\s,;]+)")
USERHOST_RX = re.compile(r"\b([A-Za-z_][\w.-]*)@([A-Za-z0-9][\w.-]*)")
FQDN_RX = re.compile(
    r"\b(?:[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?\.)+"
    r"(?:local|lan|home|internal|localdomain|ts\.net|com|net|org|io|dev|cloud|jp)\b"
)
ENV_SECRETS = ("BOARD_HOST", "BOARD_UART", "BOARD_SERIAL")

USAGE = "usage: python3 -I tools/mp2s_report.py RECORD_DIR"


class Redactor:
    def __init__(self, env):
        self.count = 0
        self.literals = sorted(
            {env[k] for k in ENV_SECRETS if env.get(k) and len(env[k]) >= 3},
            key=len,
            reverse=True,
        )

    def _sub(self, rx, text, repl=None):
        def f(m):
            out = repl(m) if repl else REDACTED
            if out != m.group(0):
                self.count += 1
            return out

        return rx.sub(f, text)

    def _ip(self, m):
        try:
            ipaddress.ip_address(m.group(0))
        except ValueError:
            return m.group(0)
        return REDACTED

    def line(self, text, console=False):
        text = ANSI_RX.sub("", text)
        for lit in self.literals:
            n = text.count(lit)
            if n:
                self.count += n
                text = text.replace(lit, REDACTED)
        if console:
            m = LOGIN_RX.match(text)
            if m and m.group(1) != REDACTED:
                self.count += 1
                text = text[: m.start(1)] + REDACTED + text[m.end(1):]
        text = self._sub(TTY_RX, text)
        text = self._sub(MAC_RX, text)
        text = self._sub(IPV4_RX, text, self._ip)
        text = self._sub(IPV6_RX, text, self._ip)
        text = self._sub(SERIAL_RX, text, lambda m: m.group(1) + REDACTED)
        text = self._sub(USERHOST_RX, text, lambda m: m.group(1) + "@" + REDACTED)
        text = self._sub(FQDN_RX, text)
        return text


def find(d, patterns, exclude=None):
    out = []
    for p in patterns:
        for f in sorted(glob.glob(os.path.join(glob.escape(d), p))):
            if os.path.isfile(f) and f not in out and not (exclude and exclude(f)):
                out.append(f)
    return out


def read_lines(path):
    with open(path, "rb") as f:
        data = f.read()
    return data.decode("utf-8", "replace").replace("\r\n", "\n").replace("\r", "\n").split("\n")


def read_manifest(path):
    entries = {}
    for ln in read_lines(path):
        m = SHA_LINE_RX.match(ln.strip())
        if m:
            entries[m.group(2)] = m.group(1).lower()
    return entries


def compare(before, after):
    """(differing stock paths with a reason, differing own paths)."""
    stock, own = [], []
    for p in sorted(set(before) | set(after)):
        if p not in after:
            why = "removed"
        elif p not in before:
            why = "added"
        elif before[p] != after[p]:
            why = "changed"
        else:
            continue
        (own if OWN_FILE_RX.search(p) else stock).append((p, why))
    return stock, own


def diff_entries(path):
    before, after = {}, {}
    for ln in read_lines(path):
        if ln.startswith(("---", "+++")) or not ln[:1] in ("-", "+"):
            continue
        m = SHA_LINE_RX.match(ln[1:].strip())
        if m:
            (before if ln[0] == "-" else after)[m.group(2)] = m.group(1).lower()
    # A path on one side only of a diff was added or removed; on both, changed.
    return before, after


def build(d, env):
    red = Redactor(env)
    missing = []
    out = []
    rel = lambda f: os.path.basename(f)  # noqa: E731

    # (a)
    name = os.path.basename(os.path.normpath(d))
    m = NAME_RX.match(name)
    sha8 = m.group("sha8") if m else None
    out.append("# DK standalone record: `%s`" % red.line(name))
    out.append("")
    out.append("## Record")
    out.append("")
    out.append("- record: `%s`" % red.line(name))
    if sha8:
        out.append("- commit: `%s`" % sha8)
    else:
        missing.append("commit sha8 in the record name (<stamp>-standalone-...-<sha8>)")
        out.append("- missing: commit sha8 in the record name")
    out.append("")

    # (b)
    out.append("## Verdicts")
    out.append("")
    vfiles = find(d, VERDICT_GLOBS)
    verdicts = []
    for f in vfiles:
        for ln in read_lines(f):
            s = ln.strip()
            if s.startswith(VERDICT_PREFIXES):
                verdicts.append((rel(f), red.line(s)))
    if not vfiles:
        missing.append("verdict file (%s)" % ", ".join(VERDICT_GLOBS))
        out.append("- missing: verdict file (%s)" % ", ".join(VERDICT_GLOBS))
    elif not verdicts:
        missing.append("verdict lines in %s" % ", ".join(rel(f) for f in vfiles))
        out.append("- missing: verdict lines in %s" % ", ".join(rel(f) for f in vfiles))
    for f, s in verdicts:
        out.append("- `%s`: `%s`" % (f, s))
    out.append("")

    # (c)
    out.append("## Console (section 7 contract)")
    out.append("")
    logs = find(d, CONSOLE_GLOBS)
    hits = []  # (file, lineno, raw)
    logins = []
    for f in logs:
        for i, ln in enumerate(read_lines(f), 1):
            clean = ANSI_RX.sub("", ln)
            if any(rx.search(clean) for rx in CONTRACT_RX):
                hits.append((rel(f), i, clean))
            if LOGIN_RX.match(clean):
                logins.append((rel(f), i, clean))
    pos = {}
    if not logs:
        missing.append("console log (%s)" % ", ".join(CONSOLE_GLOBS))
        out.append("- missing: console log (%s)" % ", ".join(CONSOLE_GLOBS))
    else:
        order = {f: k for k, f in enumerate(rel(x) for x in logs)}
        key = lambda h: (order[h[0]], h[1])  # noqa: E731
        for label, rx in (("boots left 1", LEFT1_RX), ("boots left 0", LEFT0_RX)):
            h = next((h for h in hits if rx.search(h[2])), None)
            if h:
                pos[label] = h
        after0 = [h for h in logins if "boots left 0" in pos and key(h) > key(pos["boots left 0"])]
        login = (after0 or logins or [None])[0]
        if login:
            pos["Linux login"] = login
        for f, i, ln in sorted(hits + ([login] if login else []), key=key):
            out.append("- `%s:%d`: `%s`" % (f, i, red.line(ln.strip(), console=True)))
        for label in ("boots left 1", "boots left 0", "Linux login"):
            if label not in pos:
                missing.append("console line '%s'" % label)
                out.append("- missing: console line '%s'" % label)
        if len(pos) == 3:
            ordered = key(pos["boots left 1"]) < key(pos["boots left 0"]) < key(pos["Linux login"])
            out.append("- order boots left 1, boots left 0, Linux login: %s" % ("yes" if ordered else "no"))
    out.append("")

    # (d)
    out.append("## Stock boot files (sha256)")
    out.append("")
    diffs = find(d, MANIFEST_DIFF_GLOBS)
    befores = find(d, MANIFEST_BEFORE_GLOBS, lambda f: f in diffs)
    afters = find(d, MANIFEST_AFTER_GLOBS, lambda f: f in diffs)
    pair = None
    for b in befores:
        twin = os.path.join(os.path.dirname(b), rel(b).replace("before", "after"))
        if twin in afters:
            pair = (b, twin)
            break
    if pair is None and len(befores) == 1 and len(afters) == 1:
        pair = (befores[0], afters[0])
    identical = None
    if pair:
        stock, own = compare(read_manifest(pair[0]), read_manifest(pair[1]))
        src = "`%s` against `%s`" % (rel(pair[0]), rel(pair[1]))
    elif diffs:
        stock, own = compare(*diff_entries(diffs[0]))
        src = "`%s`" % rel(diffs[0])
    if pair or diffs:
        identical = not stock
        out.append("- from %s" % red.line(src))
        out.append("- stock boot files byte-identical: %s" % ("yes" if identical else "no"))
        for p, why in stock:
            out.append("  - differs (%s): `%s`" % (why, red.line(p)))
        for p, why in own:
            out.append("  - ours, not counted (%s): `%s`" % (why, red.line(p)))
    else:
        what = "sha256 manifest (%s with %s, or %s)" % (
            ", ".join(MANIFEST_BEFORE_GLOBS),
            ", ".join(MANIFEST_AFTER_GLOBS),
            ", ".join(MANIFEST_DIFF_GLOBS),
        )
        missing.append(what)
        out.append("- missing: %s" % what)
    out.append("")

    # (e)
    failed = [s for _, s in verdicts if s.startswith(("FAIL", "not ok "))]
    failed += [
        s for _, s in verdicts
        if s.startswith("verdict:") and s.split(":", 1)[1].strip().split()[:1] not in (["ok"], ["pass"], ["PASS"])
    ]
    problems = list(failed)
    if identical is False:
        problems.append("stock boot files differ: " + ", ".join("`%s`" % red.line(p) for p, _ in stock))
    if len(pos) == 3 and not (
        key(pos["boots left 1"]) < key(pos["boots left 0"]) < key(pos["Linux login"])
    ):
        problems.append("console lines out of order")
    met = not missing and not problems
    result = "met" if met else ("incomplete record" if missing else "not met")
    where = lambda k: "`%s:%d`" % pos[k][:2] if k in pos else "missing"  # noqa: E731

    c1281 = [
        "DK standalone boot (plan #362, board-run): `default mp2_hello 2` from the",
        "DK's own microSD, record `%s` at commit `%s`." % (red.line(name), sha8 or "missing"),
        "",
        "- console: `boots left 1` at %s, `boots left 0` at %s, OpenSTLinux login at %s"
        % (where("boots left 1"), where("boots left 0"), where("Linux login")),
        "- stock boot files byte-identical: %s"
        % ({True: "yes", False: "no", None: "missing"}[identical]),
    ]
    c1281 += ["- verdict: `%s`" % s for _, s in verdicts]
    c1281 += ["- problem: %s" % p for p in problems]
    c1281 += ["- missing: %s" % w for w in missing]
    c1281 += ["", "Result: %s." % result]
    c7 = [
        "Stage 5, STM32MP257F-DK: criterion 'a standalone boot from its own storage'",
        "(docs/standalone-boot.md): %s. Record `%s` (commit `%s`) of `default mp2_hello 2`"
        % (result, red.line(name), sha8 or "missing"),
        "(two counted boots, then OpenSTLinux by itself); stock boot files byte-identical: %s."
        % ({True: "yes", False: "no", None: "missing"}[identical]),
        "Details in #1281.",
    ]
    out.append("## Draft comment for #1281 (not posted)")
    out.append("")
    out += ["> " + ln if ln else ">" for ln in c1281]
    out.append("")
    out.append("## Draft comment for #7 (not posted)")
    out.append("")
    out += ["> " + ln if ln else ">" for ln in c7]
    out.append("")

    # Safety net over the whole text (generic patterns only).
    text = "\n".join(red.line(ln) for ln in out)
    text += "\nredactions: %d (replaced with %s)\n" % (red.count, REDACTED)
    return text, (3 if missing else 0)


def main(argv):
    if len(argv) == 2 and argv[1] in ("-h", "--help"):
        print(USAGE)
        return 0
    if len(argv) != 2:
        print(USAGE, file=sys.stderr)
        return 64
    d = argv[1]
    if not os.path.isdir(d):
        print("mp2s_report: not a directory: %s" % d, file=sys.stderr)
        print(USAGE, file=sys.stderr)
        return 64
    text, rc = build(d, os.environ)
    sys.stdout.write(text)
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv))
