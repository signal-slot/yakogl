#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Which integration test targets hold any code in one feature set.

usage: tools/check_live_tests.py --split [--no-default-features] <features>
       tools/check_live_tests.py --count [--no-default-features] <features>...

<features> is one cargo feature list ("host-test,gpu-vivante"; "" for none).
--split prints one line per root target (tests/<name>.rs), `live <name>` or
`empty <name>`: empty are the targets whose file starts with a crate-level
`#![cfg(...)]` that is false for the feature set, live every other one. A
whole-suite test row builds the live ones (tools/check.sh,
live_test_targets). --count prints one line per feature set.

Why (docs/ci.md, section 6): a test file is its own crate, and most of them
open with `#![cfg(all(feature = "gpu-v3d", ...))]`. In a feature set where
that is false the crate is empty: rustc strips every item, the binary holds
no test, and running it proves nothing. A whole-suite row still compiled,
linked, hashed and ran one such binary per file (on 2026-11-27, 615 of the
967 in `host-test,gpu-vivante,shaderc-online`).

The reading can only err towards building a target:

- Only the file's head is read: blank lines, comments and inner attributes
  (`#![...]`) up to the first other line. A `cfg` further down, in a
  module or in a string, is never seen.
- A predicate is `all`, `any`, `not` and `feature = "..."` over the feature
  set, closed over Cargo.toml's [features] (a feature that names others
  turns them on). Anything else (`target_os`, `unix`, `debug_assertions`,
  a `cfg_attr`, a name this tool has never seen) is unknown, and a target
  is left out only when its predicate is false whatever the unknowns are.
- A file that cannot be read or parsed is built.

A file with a syntax error is still named by `cargo fmt --check`, which
parses every target's file whatever its cfg.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TOKEN = re.compile(r'"(?:[^"\\]|\\.)*"|[A-Za-z_][A-Za-z0-9_]*|[(),=]|\S')


def features_table(root):
    """Cargo.toml's [features]: name -> the features it turns on."""
    try:
        import tomllib
        with open(os.path.join(root, "Cargo.toml"), "rb") as f:
            table = tomllib.load(f).get("features", {})
    except (ImportError, OSError, ValueError):
        return None
    return {k: [x for x in v if isinstance(x, str)] for k, v in table.items()}


def closure(names, table):
    """The feature set cargo builds for `names`; None when it cannot be told."""
    out, todo = set(), list(names)
    while todo:
        n = todo.pop()
        if n in out:
            continue
        # `dep:x`, `x/y` and `x?/y` name dependencies: not this crate's shape
        if n.startswith("dep:") or "/" in n:
            return None
        if n not in table:
            return None
        out.add(n)
        todo.extend(table[n])
    return out


def head_attributes(text):
    """The inner attributes (`#![...]`, their text between the brackets) of
    the file's head: what comes before the first line that is neither blank,
    a comment nor an inner attribute."""
    out, i, n = [], 0, len(text)
    while i < n:
        c = text[i]
        if c in " \t\r\n﻿":
            i += 1
        elif text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j + 1
        elif text.startswith("/*", i):
            depth, i = 1, i + 2
            while i < n and depth:
                if text.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif text.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
            if depth:
                break
        elif text.startswith("#!", i) and not text.startswith("#!/", i):
            j = i + 2
            while j < n and text[j] in " \t\r\n":
                j += 1
            if j >= n or text[j] != "[":
                break
            depth, k, in_str = 0, j, False
            while k < n:
                ch = text[k]
                if in_str:
                    if ch == "\\":
                        k += 1
                    elif ch == '"':
                        in_str = False
                elif ch == '"':
                    in_str = True
                elif ch == "[":
                    depth += 1
                elif ch == "]":
                    depth -= 1
                    if depth == 0:
                        break
                k += 1
            if k >= n:
                break
            out.append(text[j + 1:k])
            i = k + 1
        else:
            break
    return out


class Unreadable(Exception):
    pass


def parse(tokens, i):
    """One cfg predicate from tokens[i:]: (tree, next index)."""
    if i >= len(tokens):
        raise Unreadable
    t = tokens[i]
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", t):
        raise Unreadable
    if i + 1 < len(tokens) and tokens[i + 1] == "(":
        args, i = [], i + 2
        while True:
            if i >= len(tokens):
                raise Unreadable
            if tokens[i] == ")":
                return (t, args), i + 1
            a, i = parse(tokens, i)
            args.append(a)
            if i < len(tokens) and tokens[i] == ",":
                i += 1
            elif i >= len(tokens) or tokens[i] != ")":
                raise Unreadable
    if i + 1 < len(tokens) and tokens[i + 1] == "=":
        if i + 2 >= len(tokens) or not tokens[i + 2].startswith('"'):
            raise Unreadable
        return ("=", t, tokens[i + 2][1:-1]), i + 3
    return ("name", t), i + 1


def value(tree, feats):
    """True, False, or None for "not decided by the features"."""
    op = tree[0]
    if op == "=":
        return (tree[2] in feats) if tree[1] == "feature" and "\\" not in tree[2] else None
    if op == "all":
        vals = [value(a, feats) for a in tree[1]]
        return False if False in vals else (None if None in vals else True)
    if op == "any":
        vals = [value(a, feats) for a in tree[1]]
        return True if True in vals else (None if None in vals else False)
    if op == "not" and len(tree[1]) == 1:
        v = value(tree[1][0], feats)
        return None if v is None else not v
    return None


def dead(path, feats):
    """True only when a crate-level cfg of the file is false for `feats`."""
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except (OSError, ValueError):
        return False
    for attr in head_attributes(text):
        tokens = TOKEN.findall(attr)
        if not tokens or tokens[0] != "cfg":
            continue
        try:
            tree, end = parse(tokens, 0)
        except Unreadable:
            continue
        if end != len(tokens) or len(tree[1]) != 1:
            continue
        if value(tree[1][0], feats) is False:
            return True
    return False


def targets(root):
    d = os.path.join(root, "tests")
    return sorted(f[:-3] for f in os.listdir(d) if f.endswith(".rs") and os.path.isfile(os.path.join(d, f)))


def other_targets(root):
    """True when the package may have targets besides its library and
    tests/*.rs, or names its tests itself: `--lib --test ...` would then not
    be what a bare `cargo test` builds."""
    try:
        import tomllib
        with open(os.path.join(root, "Cargo.toml"), "rb") as f:
            manifest = tomllib.load(f)
    except (ImportError, OSError, ValueError):
        return True
    if any(k in manifest for k in ("bin", "example", "test", "bench")):
        return True
    package = manifest.get("package", {})
    if any(k in package for k in ("autobins", "autoexamples", "autotests", "autobenches")):
        return True
    if os.path.exists(os.path.join(root, "src", "main.rs")) or os.path.isdir(os.path.join(root, "src", "bin")):
        return True
    ex = os.path.join(root, "examples")
    if os.path.isdir(ex):
        for e in os.listdir(ex):
            if e.endswith(".rs") or os.path.isfile(os.path.join(ex, e, "main.rs")):
                return True
    d = os.path.join(root, "tests")
    # a directory with a main.rs is a target too, and this tool reads files only
    return any(os.path.isfile(os.path.join(d, e, "main.rs")) for e in os.listdir(d))


def split(root, features, default):
    """(live, empty) target names for one feature list."""
    names = targets(root)
    table = features_table(root)
    asked = [f for f in re.split(r"[,\s]+", features) if f]
    if default:
        asked.append("default")
    feats = None if table is None else closure(asked, table if "default" in table else dict(table, default=[]))
    if feats is None or other_targets(root):
        return names, []
    gone = {t for t in names if dead(os.path.join(root, "tests", t + ".rs"), feats)}
    return [t for t in names if t not in gone], sorted(gone)


def main(argv):
    args = argv[1:]
    default = "--no-default-features" not in args
    count = "--count" in args
    sets = [a for a in args if not a.startswith("--")]
    if any(a.startswith("--") and a not in ("--no-default-features", "--split", "--count") for a in args) or \
            ("--split" in args) == count or (not count and len(sets) != 1) or not sets:
        sys.stderr.write(__doc__)
        return 2
    root = os.environ.get("YAKOGL_LIVE_TESTS_ROOT", ROOT)
    for s in sets:
        live, gone = split(root, s, default)
        if count:
            print("%s: %d live, %d empty" % (s, len(live), len(gone)))
        else:
            for t in sorted(live + gone):
                print("%s %s" % ("empty" if t in gone else "live", t))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
