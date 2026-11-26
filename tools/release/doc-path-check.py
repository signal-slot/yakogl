#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Path check of a document's backticked spans (release checklist lines 1, 5).

    python3 -I tools/release/doc-path-check.py FILE [--section '## [Unreleased]']
    python3 -I tools/release/doc-path-check.py --self-test

Every backticked span of FILE (or of the named section, up to the next heading
of the same level) becomes candidates: brace lists are expanded, a leading ./
is dropped, a trailing :LINE or :LINE-LINE is dropped. A candidate resolves
against `git ls-files` at HEAD: an exact tracked file, a tracked directory
prefix, a glob (`*`) matching a tracked file, or a bare file name matching
exactly one tracked file. Spans sort into
resolved, non-path or missing; missing ones fail the run (exit 1).

Non-path: no '/' and no file extension; a span with spaces (a command) unless
its first word is a path; a bare extension (`.a`); an all-caps identifier or
prefix (GLES_PRIM_); a span with <...> placeholders (a run-record directory
under tools/cts/log/ resolves if some tracked path matches the pattern).
"""
import fnmatch
import re
import subprocess
import sys

# Build outputs and runtime files that have an extension but are never tracked.
NON_PATHS = {
    "libyakogl.a",  # product archive, built by tools/check.sh
    "uart.log",  # the board's serial capture, written at run time
    "glTexSubImage2D/3D",  # two GL API names joined by a slash, not a path
}


def tracked():
    out = subprocess.run(["git", "ls-files"], capture_output=True, text=True, check=True).stdout
    return out.splitlines()


def expand(s):
    m = re.search(r"\{([^{}]*)\}", s)
    if not m:
        return [s]
    res = []
    for alt in m.group(1).split(","):
        res += expand(s[: m.start()] + alt + s[m.end():])
    return res


def classify(span, files, dirs, names):
    """Return 'resolved', 'nonpath' or 'missing'."""
    s = span.strip()
    if not s:
        return "nonpath"
    if " " in s:
        first = s.split()[0]
        if "/" not in first or first.startswith("-"):
            return "nonpath"
        s = first
    if s in NON_PATHS:
        return "nonpath"
    if re.fullmatch(r"\.[A-Za-z0-9]+", s) or re.fullmatch(r"[A-Z][A-Z0-9_]*_?", s):
        return "nonpath"
    if "<" in s and ">" in s:
        pat = re.sub(r"<[^>]*>", "*", s).rstrip("/")
        pat = re.sub(r"\*+", "*", pat)
        if "/" in pat and any(fnmatch.fnmatch(f, pat + "*") for f in files):
            return "resolved"
        return "nonpath"
    for c in expand(s):
        c = re.sub(r"^(\./)+", "", c)
        c = re.sub(r":\d+(-\d+)?$", "", c)
        if "/" not in c and not re.search(r"\.[A-Za-z0-9]+$", c):
            return "nonpath"
        cc = c.rstrip("/")
        if "*" in c:
            if any(fnmatch.fnmatch(f, c) for f in files):
                continue
            return "missing"
        if c in files or cc in files or cc in dirs:
            continue
        if "/" not in c and len(names.get(c, ())) == 1:
            continue
        return "missing"
    return "resolved"


def section_text(text, heading):
    lines = text.splitlines()
    level = len(heading) - len(heading.lstrip("#"))
    out, on = [], False
    for ln in lines:
        m = re.match(r"(#+)\s", ln)
        if on and m and len(m.group(1)) <= level:
            break
        if ln.strip() == heading:
            on = True
            continue
        if on:
            out.append(ln)
    return "\n".join(out) if on else None


def run(path, section):
    text = open(path, encoding="utf-8").read()
    if section:
        text = section_text(text, section)
        if text is None:
            print("section not found: " + section)
            return 2
    files = set(tracked())
    d2 = set()
    for f in files:
        parts = f.split("/")
        for i in range(1, len(parts)):
            d2.add("/".join(parts[:i]))
    dirs = d2
    names = {}
    for f in files:
        names.setdefault(f.rsplit("/", 1)[-1], []).append(f)
    spans = []
    for s in re.findall(r"`([^`\n]+)`", text):
        if s not in spans:
            spans.append(s)
    miss, non = [], []
    for s in spans:
        r = classify(s, files, dirs, names)
        if r == "missing":
            miss.append(s)
        elif r == "nonpath":
            non.append(s)
    for s in miss:
        print("missing: " + s)
    for s in non:
        print("non-path: " + s)
    print("%d checked, %d missing, %d non-path" % (len(spans), len(miss), len(non)))
    return 1 if miss else 0


def self_test():
    files = {"a/b.rs", "a/c.rs", "x/y/z.txt", "tools/cts/log/run-2026/rec.md", "only.md"}
    dirs = {"a", "x", "x/y", "tools", "tools/cts", "tools/cts/log", "tools/cts/log/run-2026"}
    names = {"b.rs": ["a/b.rs"], "only.md": ["only.md"], "dup.md": ["p/dup.md", "q/dup.md"]}
    cases = [
        ("a/{b,c}.rs", "resolved"),
        ("a/{b,d}.rs", "missing"),
        ("./a/b.rs:12-30", "resolved"),
        ("x/y", "resolved"),
        ("only.md", "resolved"),
        ("dup.md", "missing"),
        ("cargo test --test foo", "nonpath"),
        ("a/b.rs --flag", "resolved"),
        (".a", "nonpath"),
        ("GLES_PRIM_", "nonpath"),
        ("libyakogl.a", "nonpath"),
        ("nothere.rs", "missing"),
        ("tools/cts/log/<run>/rec.md", "resolved"),
        ("tools/cts/log/<run>/gone.md", "nonpath"),
        ("word", "nonpath"),
        ("a/*.rs", "resolved"),
        ("a/*.py", "missing"),
    ]
    bad = 0
    for span, want in cases:
        got = classify(span, files, dirs, names)
        if got != want:
            print("FAIL %r: got %s, want %s" % (span, got, want))
            bad += 1
    print("self-test: %d case(s), %d failed" % (len(cases), bad))
    return 1 if bad else 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    section = None
    if "--section" in argv:
        i = argv.index("--section")
        section = argv[i + 1]
        argv = argv[:i] + argv[i + 2:]
    if len(argv) != 1:
        print(__doc__)
        return 2
    return run(argv[0], section)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
