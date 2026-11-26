#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The `[[test]]` required-features entries of Cargo.toml, from the tests' cfg gates.

Every integration test is its own binary, statically linking the whole library, and a test
row builds one per `tests/` file for every feature set of its row. A file whose file-level
`#![cfg(...)]` gate cannot hold under that feature set compiles to an empty crate: the row
still links and runs a 0-test copy of the library (a husk). Measured 2026-11-26 (docs/ci.md
3.9): 5,876 of the 8,920 integration binaries of a full gate run were husks.

Cargo already knows how to skip such a target: a `[[test]]` entry with `required-features`
is not built when one of its features is off (`cargo test --features ...` skips it; naming
it with `--test` names it in error). The entry's features are exactly the feature atoms the
file's cfg gate requires at its top level: the same feature set can satisfy the gate only
when all of them are on, so cargo skips the target only where the gate is false anyway.
A gate whose shape is not a top-level `all(...)` of plain `feature = "..."` atoms (an
`any(...)`, a `not(...)`, a cfg beyond features) contributes the atoms it has and keeps
the rest of its behaviour as today; atoms alone are necessary conditions, never a claim
that the gate holds. Files with nothing beyond `host-test` get no entry: every test row
has `host-test`, so an entry there would skip nothing.

usage: tools/check_test_features.py --check
  Compare Cargo.toml's `[[test]]` entries with the cfg gates of the `tests/` files, the
  way the gate compares a generated file with its source. Red, with the fix named, when
  an entry's features differ from its file's gate atoms, an entry names no tests/ file,
  a gated file has no entry, or two entries share a name. An edit that widens or narrows
  a file's gate must carry its Cargo.toml entry with it: this is what makes a drift
  between the two a red row rather than a silent skip.

usage: tools/check_test_features.py --emit
  Print the `[[test]]` entries Cargo.toml should hold, in order (one per gated file with
  atoms beyond `host-test`, names sorted, features sorted). The tail block between the
  two marker lines is this output; regenerate it the same way a generated table is.

usage: tools/check_test_features.py --live <feature set> [name...]
  Which of the named integration test targets are live under the comma-separated feature
  set (the set `cargo test --no-default-features --features <set>` would build): a name
  with no entry is live (cargo builds what Cargo.toml does not name), and an entry is
  live when every required feature is in the set. tools/check.sh filters a `--changed`
  tests-scope pick through this, so naming a target that is not built under the row's
  features is not an error (cargo errors on such a name).
"""
import re
import sys

ROOT = __import__("os").path.dirname(__import__("os").path.dirname(__import__("os").path.abspath(__file__)))

MARK_BEGIN = "# The [[test]] entries begin here (tools/check_test_features.py --emit)."
MARK_END = "# The [[test]] entries end here."

FEATURE = re.compile(r'feature\s*=\s*"([A-Za-z0-9_-]+)"')


def cfg_gate(src):
    """The whole-file gate: a first `#![cfg(...)]` with its inner expression, or None.

    The first inner cfg attribute of the file gates the whole crate (a later inner attribute
    cannot appear after an item), and a file's tests exist only where it holds. A file with
    several takes the first: an entry built from a subset is a smaller skip, never a wrong
    one, and --check compares entries with the same reading, so the two cannot disagree.
    """
    i = src.find("#![cfg(")
    if i < 0:
        return None
    k = i + len("#![cfg(")
    depth = 1
    while k < len(src) and depth:
        if src[k] == "(":
            depth += 1
        elif src[k] == ")":
            depth -= 1
        k += 1
    return src[i + len("#![cfg("):k - 1].strip()


def gate_atoms(expr):
    """The plain `feature = "..."` atoms of a whole-file cfg gate, a set.

    A top-level `all(...)` splits into parts at its own commas; only a part that is a whole
    feature test is an atom. A top-level gate that is not an `all(...)`, and a part that is
    an `any(...)` or a `not(...)`, is not necessary on its own and adds nothing: cargo's
    required-features are necessary conditions, and these are what is necessary. Feature
    atoms of a part carry their name; anything else (a `target_os`, a `not`) is left out,
    which makes the entry skip less than the gate would, never more.
    """
    if expr is None:
        return set()
    m = re.fullmatch(r"all\((.*)\)", expr, re.S)
    inner = m.group(1) if m else expr
    parts, depth, cur = [], 0, ""
    for c in inner:
        if c == "," and depth == 0:
            parts.append(cur)
            cur = ""
        else:
            if c in "([":
                depth += 1
            elif c in ")]":
                depth -= 1
            cur += c
    parts.append(cur)
    return {FEATURE.fullmatch(p.strip()).group(1)
            for p in parts if FEATURE.fullmatch(p.strip())}


def expected_entries():
    """target name -> sorted required features, for every tests/ file whose gate asks
    for features beyond host-test. No entry for a file with no gate or whose atoms are
    host-test alone: every test row of tools/check.sh has host-test, so such an entry
    would skip nothing."""
    out = {}
    for name in sorted(e[:-3] for e in _root_test_files()):
        atoms = gate_atoms(cfg_gate(_read("tests/" + name + ".rs")))
        if atoms and atoms != {"host-test"}:
            out[name] = sorted(atoms)
    return out


def _root_test_files():
    import os
    return [e for e in os.listdir(os.path.join(ROOT, "tests")) if e.endswith(".rs")]


def _read(rel):
    with open(ROOT + "/" + rel, encoding="utf-8", errors="replace") as f:
        return f.read()


def manifest_entries():
    """The `[[test]]` entries Cargo.toml declares: name -> (sorted features, line), and
    the problems of the manifest itself (duplicate names, a nameless entry).

    All entries live in the region between MARK_BEGIN and MARK_END (the tail block
    --emit prints): a `[[test]]` entry written anywhere else, or the markers gone,
    is a problem, the way a generated table written outside its generator's region is."""
    entries = {}
    problems = []
    lines = _read("Cargo.toml").split("\n")
    begin = next((i for i, l in enumerate(lines) if l == MARK_BEGIN), None)
    end = next((i for i, l in enumerate(lines) if l == MARK_END), None)
    if begin is None or end is None or begin > end:
        problems.append("Cargo.toml: the [[test]] entries belong between the two lines\n"
                        f"  {MARK_BEGIN}\n  {MARK_END}\n"
                        "  (the block tools/check_test_features.py --emit prints; docs/ci.md 3.9)")
    i = 0
    while i < len(lines):
        if lines[i].strip() == "[[test]]":
            if begin is not None and end is not None and not begin < i < end:
                problems.append(f"Cargo.toml:{i + 1}: a [[test]] entry outside the marked region")
            at = i + 1
            name = None
            feats = None
            while at < len(lines) and not lines[at].strip().startswith("[[") and not (
                    lines[at].startswith("[") and not lines[at].startswith("[[")):
                s = lines[at].strip()
                m = re.fullmatch(r'name\s*=\s*"([A-Za-z0-9_-]+)"', s)
                if m and name is None:
                    name = m.group(1)
                m = re.fullmatch(r'required-features\s*=\s*\[([^\]]*)\]', s)
                if m and feats is None:
                    feats = sorted(x.strip().strip('"') for x in m.group(1).split(",") if x.strip())
                if s and not s.startswith("#"):
                    m2 = re.fullmatch(r'(path|test|bench|doctest|crate|required-features)\s*=.*', s)
                    if not m2 and not re.fullmatch(r'name\s*=.*', s):
                        problems.append(f"Cargo.toml:{at + 1}: a [[test]] key this tool does not read: {s}")
                at += 1
            if name is None:
                problems.append(f"Cargo.toml:{i + 1}: a [[test]] entry with no name")
            elif name in entries:
                problems.append(f"Cargo.toml:{i + 1}: a second [[test]] entry named {name}")
            else:
                entries[name] = (feats or [], i + 1)
            i = at
        else:
            i += 1
    return entries, problems


def emit_block(expected):
    out = []
    for name in sorted(expected):
        out.append("[[test]]")
        out.append(f'name = "{name}"')
        out.append("required-features = [" + ", ".join(f'"{f}"' for f in expected[name]) + "]")
    return "\n".join(out)


def check():
    expected = expected_entries()
    entries, problems = manifest_entries()
    bad = list(problems)
    for name, (feats, at) in sorted(entries.items()):
        want = expected.get(name)
        if want is None:
            gate = gate_atoms(cfg_gate(_read(f"tests/{name}.rs"))) if _root_test_exists(name) else None
            if not _root_test_exists(name):
                bad.append(f"Cargo.toml:{at}: [[test]] {name} names no tests/{name}.rs")
            elif not gate:
                bad.append(f"Cargo.toml:{at}: [[test]] {name}: tests/{name}.rs has no whole-file cfg gate; "
                           "an entry there builds the target everywhere, exactly what cargo does without one")
            elif set(gate) == {"host-test"}:
                bad.append(f"Cargo.toml:{at}: [[test]] {name}: its gate needs host-test alone; "
                           "every test row has host-test, so drop the entry (it skips nothing)")
            else:
                bad.append(f"Cargo.toml:{at}: [[test]] {name}: required-features {feats} is not the "
                           f"gate's atoms {sorted(gate)}")
        elif feats != want:
            bad.append(f"Cargo.toml:{at}: [[test]] {name}: required-features {feats}, the gate asks {want}")
    for name in sorted(expected):
        if name not in entries:
            bad.append(f"tests/{name}.rs: its cfg gate needs {expected[name]}; Cargo.toml has no [[test]] "
                       f"{name} entry, so every feature set without those features links a 0-test copy of it")
    if bad:
        print("\n".join(bad))
        print("fix: tools/check_test_features.py --emit prints the entries; a changed cfg gate moves its")
        print("entry in the same commit (docs/ci.md 3.9).")
        return 1
    print(f"{len(entries)} [[test]] entries, one per gated tests/ file: every required-features list is")
    print(f"its cfg gate's feature atoms ({expected_lines(entries)} entry features in all).")
    return 0


def expected_lines(entries):
    return sum(len(v) for v in entries.values())


def _root_test_exists(name):
    import os
    return os.path.isfile(os.path.join(ROOT, "tests", name + ".rs"))


def live(features, names):
    entries, _ = manifest_entries()
    fs = set(features.split(","))
    for n in names:
        req = entries.get(n)
        if req is None or all(f in fs for f in req[0]):
            print(n)


def main(argv):
    mode = argv[1] if len(argv) > 1 else ""
    if mode == "--check":
        return check()
    if mode == "--emit":
        print(emit_block(expected_entries()))
        return 0
    if mode == "--live":
        if len(argv) < 3:
            print(__doc__)
            return 2
        names = argv[3:] or sys.stdin.read().split()
        live(argv[2], names)
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
