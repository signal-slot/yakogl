#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Which integration test targets a change confined to tests/ reaches.

usage: tools/check_tests_reach.py <changed path>...

Prints one line, for tools/check.sh (the `tests` scope, docs/ci.md):

  ALL             cannot tell: run every test target of the test rows
  WIDEN           something outside tests/ reads a changed file: every row
  <target> ...    the targets (tests/<target>.rs) the paths reach

The reading is textual and conservative: a path selects more targets, never
fewer. A target is a root tests/<name>.rs (Cargo.toml has no [[test]]
entries). Its sources are the file and everything it pulls in with `mod`,
`#[path]`, `include!`, `include_str!` or `include_bytes!`, transitively. A
changed root tests/<name>.rs reaches <name> and every target that pulls it
in. A changed file under a tests/ subdirectory reaches the targets that pull
it in and those whose sources mention its path, name or directory (a fixture
is read by name). A fixture that anything outside tests/ also mentions
(tools/, src/, driver/, ...) is WIDEN: that reader is a row of its own.
"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TESTS = "tests"

MOD = re.compile(r'^\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;', re.M)
PATH_ATTR = re.compile(r'#\[\s*path\s*=\s*"([^"]+)"\s*\]')
INCLUDE = re.compile(r'\binclude(?:_str|_bytes)?!\s*\(\s*"([^"]+)"')


def read(rel):
    try:
        with open(os.path.join(ROOT, rel), encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def exists(rel):
    return os.path.isfile(os.path.join(ROOT, rel))


def pulled_in(rel, text):
    """The repo-relative files `text` (the source of `rel`) pulls in."""
    here = os.path.dirname(rel)
    # a root file's `mod x;` is tests/x.rs; a mod.rs's is next to it
    base = here if os.path.basename(rel) in ("mod.rs", "main.rs") or here == TESTS else os.path.join(here, os.path.splitext(os.path.basename(rel))[0])
    out = set()
    for m in MOD.finditer(text):
        for c in (os.path.join(base, m.group(1) + ".rs"), os.path.join(base, m.group(1), "mod.rs"),
                  os.path.join(here, m.group(1) + ".rs"), os.path.join(here, m.group(1), "mod.rs")):
            if exists(c):
                out.add(os.path.normpath(c))
    for rx in (PATH_ATTR, INCLUDE):
        for m in rx.finditer(text):
            out.add(os.path.normpath(os.path.join(here, m.group(1))))
    return out


def closure(target):
    seen, todo, texts = set(), [target], []
    while todo:
        f = todo.pop()
        if f in seen:
            continue
        seen.add(f)
        t = read(f)
        texts.append(t)
        if f.endswith(".rs"):
            todo.extend(pulled_in(f, t))
    return seen, "\n".join(texts)


def keys_of(rel):
    base = os.path.basename(rel)
    stem = os.path.splitext(base)[0]
    ks = {rel, base}
    if base in ("mod.rs", "main.rs"):
        ks = {rel, os.path.basename(os.path.dirname(rel))}
    else:
        ks.add(stem)
    parts = rel.split("/")
    # a file inside a fixture directory is read as part of that directory
    if len(parts) > 3:
        ks.add(parts[2])
    return {k for k in ks if k}


def outside_reads(rel):
    base = os.path.basename(rel)
    try:
        out = subprocess.run(
            ["git", "grep", "-lF", "-e", base, "--", "tools", "src", "driver", "capi", "examples", "build.rs",
             "Cargo.toml", ".github", ":!*.md"],
            cwd=ROOT, capture_output=True, text=True).stdout
    except OSError:
        return True
    return bool(out.strip())


def main(paths):
    targets = sorted(f[:-3] for f in os.listdir(os.path.join(ROOT, TESTS))
                     if f.endswith(".rs") and os.path.isfile(os.path.join(ROOT, TESTS, f)))
    if not targets:
        return "ALL"
    info = {t: closure("%s/%s.rs" % (TESTS, t)) for t in targets}
    hit = set()
    for p in paths:
        p = os.path.normpath(p)
        if p == "tools/release/redaction-scan.allow":
            hit.add("redaction_scan")
            continue
        parts = p.split("/")
        if parts[0] != TESTS or len(parts) < 2:
            return "ALL"
        if len(parts) == 2 and p.endswith(".rs"):
            t = parts[1][:-3]
            if t in info:
                hit.add(t)
            for u, (files, _) in info.items():
                if p in files:
                    hit.add(u)
            # a deleted or renamed-away file: whoever names it
            if t not in info:
                for u, (_, text) in info.items():
                    if re.search(r'\b%s\b' % re.escape(t), text):
                        hit.add(u)
            continue
        if parts[1] == "fixtures" or not p.endswith(".rs"):
            if outside_reads(p):
                return "WIDEN"
        found = set()
        ks = keys_of(p)
        for u, (files, text) in info.items():
            if p in files or any(re.search(r'(?<![\w])%s(?![\w])' % re.escape(k), text) for k in ks):
                found.add(u)
        if not found:
            return "ALL"
        hit |= found
    return " ".join(sorted(hit)) if hit else "ALL"


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("usage: tools/check_tests_reach.py <changed path>...", file=sys.stderr)
        sys.exit(2)
    print(main(sys.argv[1:]))
