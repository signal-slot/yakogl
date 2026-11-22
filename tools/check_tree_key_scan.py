#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Every path a literal in the test code names is under the test tree key.

usage: tools/check_tree_key_scan.py --keyed <path>... --scan <dir>...

tools/check.sh runs it (the row "tree key: every path a test names is under
TEST_TREE_PATHS", and `tools/check.sh --tree-key-scan`) with its own lists:
--keyed is TEST_TREE_PATHS, the files the test binaries read at run time and
the test cache keys on; --scan is TREE_KEY_SCAN_DIRS, the directories whose
.rs files are read here. A <dir> may hold a `*`.

A file a test reads that is not under the key leaves the cache blind to it:
a binary that passed is not run again when that file changes. So every
string literal in the scanned code that names a path under the tree (src/,
tests/, tools/, driver/, ...) must be under a keyed path, or be in a file
listed in NOT_READ. Comments and the arguments of include_str!,
include_bytes!, include! and #[path] are left out: the compiler reads those,
and the binary's bytes already hold them.

Prints each literal outside the key as `<file>:<line> <literal>`, under one
line that says so, and exits 1; prints nothing and exits 0 when there is
none. tools/check_testcache_selftest.sh checks that a probe test reading a
file outside the key is named.
"""
import argparse
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Keyed outside git: the glmark2 corpus a glmark2 row generates
# (test_tree_key in tools/check.sh hashes it by content).
KEYED_OUTSIDE_GIT = ["build/glmark2-host/shaders"]
# Literals that tests never read, by file: yakogl-tools commands (their
# tests do not run them: strace of the test binary, 2026-10-12) and the
# coverage documents the round trips write under YAKOGL_WRITE_ISA_COVERAGE.
NOT_READ = {
    "tools/yakogl-tools/src/abilayout.rs",
    "tools/yakogl-tools/src/glconsts.rs",
    "tools/yakogl-tools/src/csbgen.rs",
    "src/shaderc/online/powervr/roundtrip.rs",
    "src/shaderc/online/vivante/roundtrip.rs",
}
LITERAL = re.compile(
    r'"((?:\.\./)*(?:src|tests|tools|driver|third_party|include|capi|build|docs|zephyr)/[^"\s]*)"'
)
COMPILER_READS = re.compile(r"(include_str!|include_bytes!|include!)\(\s*$|#\[path\s*=\s*$")


def scanned_files(dirs):
    # Tracked and untracked (not ignored) files: a new test file is scanned
    # before it is added. With `:(glob)`, `*` stays inside one path component
    # and `**/` is any number of directories.
    specs = [":(glob)%s/**/*.rs" % d for d in dirs]
    out = subprocess.run(
        ["git", "ls-files", "-co", "--exclude-standard", "--"] + specs,
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout
    return sorted(set(f for f in out.split() if f.endswith(".rs") and f not in NOT_READ))


def outside(keyed, files):
    for f in files:
        with open(os.path.join(ROOT, f), errors="replace") as src:
            for n, line in enumerate(src, 1):
                if line.strip().startswith(("//", "*", "/*")):
                    continue
                for m in LITERAL.finditer(line):
                    if COMPILER_READS.search(line[: m.start()]):
                        continue
                    p = re.sub(r"^(\.\./)+", "", m.group(1)).split("{")[0].rstrip("/")
                    if not any(p == k or p.startswith(k + "/") or k.startswith(p + "/") for k in keyed):
                        yield "%s:%d %s" % (f, n, m.group(1))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--keyed", nargs="+", required=True, help="TEST_TREE_PATHS")
    ap.add_argument("--scan", nargs="+", required=True, help="TREE_KEY_SCAN_DIRS")
    args = ap.parse_args()
    found = list(outside(args.keyed + KEYED_OUTSIDE_GIT, scanned_files(args.scan)))
    if found:
        print("a test names a path outside the tree key (TEST_TREE_PATHS in tools/check.sh):")
        for line in found:
            print("    " + line)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
