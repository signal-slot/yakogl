#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Lint a U-Boot script (mp2-standalone.cmd) against ST U-Boot 2023.10 hush.

usage: mp2s_hush_lint.py [--config FILE] [--cbsize N] script.cmd

Exit 0 clean, 1 with file:line messages, 2 usage.  See
docs/notes/stm32mp257f-dk-standalone.md sections 2, 3 and 7.

Checks: every line (after backslash joining) fits CONFIG_SYS_CBSIZE; if/fi
and for/while/until do/done balance; no bash-only syntax; setexpr is the only
arithmetic; saveenv only in the counted path (right after the decrement of
mp2s_left, or after the restore of boot_prefixes); no setenv of bootcmd,
bootcount, altbootcmd; the file ends with a newline and has no CR.
"""
import os
import re
import sys

DEFAULT_CBSIZE = 1024  # stm32mp25 default
HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_CONFIG = os.path.join(HERE, "..", "driver", "tools", "mp2-standalone",
                              "qemu", "mp2-standalone.config")
PROTECTED = ("bootcmd", "bootcount", "altbootcmd")
NOT_HUSH = ("local", "declare", "typeset", "let", "expr", "readonly", "select",
            "function")
KEYWORDS = ("if", "elif", "while", "until", "for", "then", "else", "do", "fi",
            "done", "{", "}", "!")


def cbsize(config, override):
    """Return (value, source)."""
    if override is not None:
        return override, "--cbsize"
    try:
        with open(config) as f:
            for ln in f:
                m = re.match(r"\s*CONFIG_SYS_CBSIZE\s*=\s*(\w+)", ln)
                if m:
                    return int(m.group(1), 0), config
    except OSError:
        pass
    return DEFAULT_CBSIZE, "default %d (stm32mp25); %s sets none" % (
        DEFAULT_CBSIZE, os.path.basename(config))


def logical_lines(text):
    """Return [(first line number, joined text)] after continuation joining."""
    out, cur, start = [], None, 0
    for n, raw in enumerate(text.split("\n"), 1):
        if cur is None:
            cur, start = raw, n
        else:
            cur += raw
        if cur.endswith("\\"):
            cur = cur[:-1]
            continue
        out.append((start, cur))
        cur = None
    if cur is not None:
        out.append((start, cur))
    return out


def scan(line):
    """Return (code, nosq, open_quote). code: quoted text blanked, comment cut.
    nosq: only single-quoted text blanked (double quotes still expand)."""
    code, nosq = [], []
    q = None
    prev_space = True
    for ch in line:
        if q is None:
            if ch == "#" and prev_space:
                break
            if ch in "'\"":
                q = ch
            code.append(ch)
            nosq.append(ch)
            prev_space = ch in " \t;"
        else:
            if ch == q:
                q = None
                code.append(ch)
                nosq.append(ch)
            else:
                code.append(" ")
                nosq.append(" " if q == "'" else ch)
            prev_space = False
    return "".join(code), "".join(nosq), q


def commands(code):
    """Split blanked code into word lists; keywords stand alone."""
    cmds = []
    for part in re.split(r"[;&|]+", code):
        w = part.split()
        while w and w[0] in ("then", "else", "do", "{", "}", "!"):
            cmds.append([w[0]])
            w = w[1:]
        if w:
            cmds.append(w)
    return cmds


def lint(path, cb):
    msgs = []

    def err(n, m):
        msgs.append("%s:%d: %s" % (path, n, m))

    with open(path, "rb") as f:
        data = f.read()
    if b"\r" in data:
        err(data[:data.index(b"\r")].count(b"\n") + 1, "CR in file")
    if data and not data.endswith(b"\n"):
        err(data.count(b"\n") + 1, "file does not end with a newline")
    text = data.decode("utf-8", "replace").replace("\r", "")

    stack = []  # (keyword, line)
    prev = None  # previous executable command (words)
    for n, line in logical_lines(text):
        if len(line) > cb:
            err(n, "line is %d characters, CONFIG_SYS_CBSIZE is %d"
                % (len(line), cb))
        code, nosq, openq = scan(line)
        if openq:
            err(n, "unterminated %s quote" % openq)
        if re.search(r"\$\(|\$\[|`", nosq):
            err(n, "bash-only $(( )), $[ ], $( ) or backtick expansion")
        if "[[" in code or "]]" in code:
            err(n, "bash-only [[ ]]")
        if "<<" in code:
            err(n, "here-document: hush in U-Boot has none")
        if re.search(r"(^|[\s;&|])[A-Za-z_]\w*\+?=\(", code):
            err(n, "bash array assignment")
        if re.search(r"\$\{[^}]*[#%/^,:\[][^}]*\}", nosq):
            err(n, "bash-only ${...} parameter expansion")
        if re.search(r"^\s*(function\s+\w+|\w+\s*\(\s*\))", code):
            err(n, "function definition: hush in U-Boot has no functions")
        for w in commands(code):
            c = w[0]
            if c == "if":
                stack.append(("if", n))
            elif c in ("for", "while", "until"):
                stack.append((c, n))
            elif c == "fi":
                if stack and stack[-1][0] == "if":
                    stack.pop()
                else:
                    err(n, "fi without a matching if")
            elif c == "done":
                if stack and stack[-1][0] != "if":
                    stack.pop()
                else:
                    err(n, "done without a matching for/while/until")
            elif c in ("then", "elif", "else"):
                if not stack or stack[-1][0] != "if":
                    err(n, "%s outside an if" % c)
            elif c == "do":
                if not stack or stack[-1][0] == "if":
                    err(n, "do outside a loop")
            # an if/elif/while condition is itself a command
            cw = w[1:] if c in ("if", "elif", "while", "until") else w
            if not cw or cw[0] in KEYWORDS:
                continue
            if cw[0] in NOT_HUSH:
                err(n, "'%s' is not available in U-Boot hush (setexpr is the "
                    "only arithmetic)" % cw[0])
            if cw[0] == "setenv" and len(cw) > 1 and cw[1] in PROTECTED:
                err(n, "setenv of %s is forbidden" % cw[1])
            if cw[0] == "saveenv" and not (
                    prev is not None and (
                        prev[:2] == ["setexpr", "mp2s_left"] or
                        prev[:2] == ["setenv", "mp2s_ver"])):
                err(n, "saveenv outside the counted path (allowed only right "
                    "after the mp2s_left decrement or the restore of "
                    "boot_prefixes, design note section 3)")
            prev = cw
    for kw, n in stack:
        err(n, "%s is never closed" % kw)
    return msgs


def main(argv):
    config, override, path = DEFAULT_CONFIG, None, None
    a = argv[1:]
    while a:
        x = a.pop(0)
        if x == "--config" and a:
            config = a.pop(0)
        elif x == "--cbsize" and a:
            override = int(a.pop(0), 0)
        elif x.startswith("-") or path:
            sys.stderr.write(__doc__)
            return 2
        else:
            path = x
    if not path:
        sys.stderr.write(__doc__)
        return 2
    cb, src = cbsize(config, override)
    print("mp2s_hush_lint: CONFIG_SYS_CBSIZE %d (%s)" % (cb, src))
    msgs = lint(path, cb)
    for m in msgs:
        print(m)
    return 1 if msgs else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
