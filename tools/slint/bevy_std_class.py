#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Classify the crates a Cargo probe builds for a target by what they need from std.

usage: bevy_std_class.py <probe dir> <target> <out.tsv> [<failed.txt>]

<probe dir> holds a Cargo.toml with the dependencies to survey (for M9: the
dependencies of slint/examples/bevy/bevy-hosts-slint). <target> is a target with
no std, such as aarch64-unknown-none. <failed.txt> optionally lists the names of
crates that failed in `cargo check --keep-going -Z build-std=core,alloc`; they
count as needing std whatever their source says.

The crate list and the features come from `cargo tree` for the target (resolver
2 resolves features per target; `cargo metadata` would merge host and target).
Build dependencies and proc-macros run on the host and are left out.

One row per crate in <out.tsv>, tab-separated:
  name, version,
  mode   : "no_std-capable" (the crate root has #![no_std] or a cfg_attr no_std)
           or "std-only",
  build  : "needs-std" if this build needs std, else "-",
  class  : "ok"          no_std as built;
           "feature"     can be no_std, but this build turns std on;
           otherwise the heaviest std facility the code names (net, files,
           process, threads, time, io, os), or "alloc-only" if it names none,
  uses   : every facility the code names ("-": names std but none of these;
           "none": does not name std at all),
  source : the registry, or "path".

"uses" is a source scan with comments, strings and test/bench files removed. It
shows what the code names, not a proven call path, and code under
#[cfg(feature = "std")] still counts.
"""
import json
import os
import re
import subprocess
import sys
from collections import Counter

# std facilities, heaviest first; a crate's class is the first one its code names
FACILITIES = [
    ("net", r"std::net\b|std::os::(unix::net|fd)"),
    ("files", r"std::fs\b|std::path\b|std::os::unix::fs|std::os::windows::fs"),
    ("process", r"std::process\b|std::env\b"),
    ("threads", r"std::thread\b|thread_local!|std::sync::(\{[^}]*\b)?"
                r"(Mutex|RwLock|Condvar|Barrier|mpsc|Once\b|OnceLock|LazyLock|PoisonError|TryLockError)"),
    ("time", r"std::time::(\{[^}]*\b)?(Instant|SystemTime|UNIX_EPOCH)"),
    ("io", r"std::io\b|\b(e?println|e?print)!"),
    ("os", r"std::os\b|std::ffi::(OsStr|OsString)|std::panic::catch_unwind|std::backtrace"
           r"|std::alloc::System|std::(collections::hash_map|hash)::RandomState"
           r"|std::collections::(HashMap|HashSet)\b"),
]


def cargo(probe, *args):
    env = dict(os.environ, CARGO_TARGET_DIR="target")
    return subprocess.check_output(["cargo", "+nightly", *args], cwd=probe, env=env, text=True)


def built_crates(probe, target):
    """(name, version) -> (features, lib root, source) for what cargo tree builds for target."""
    # no-proc-macro: a proc-macro and what it depends on are built for the host
    tree = cargo(probe, "tree", "--target", target, "-e", "normal,no-proc-macro", "--prefix", "none",
                 "--no-dedupe", "-f", "{p}|{f}")
    feats = {}
    for line in tree.splitlines():
        pkg, _, f = line.partition("|")
        parts = pkg.split()
        if len(parts) >= 2:
            feats[(parts[0], parts[1].lstrip("v"))] = {x for x in f.split(",") if x}
    meta = json.loads(cargo(probe, "metadata", "--format-version", "1", "--filter-platform", target))
    root = meta["resolve"]["root"]
    out = {}
    for p in meta["packages"]:
        key = (p["name"], p["version"])
        if p["id"] == root or key not in feats:
            continue
        lib = next((t for t in p["targets"] if set(t["kind"]) & {"lib", "rlib"}), None)
        if lib:  # proc-macros have no lib target and are left out here
            out[key] = (feats[key], lib["src_path"], p["source"] or "path")
    return out


def strip(src):
    """Source text without comments and string contents."""
    src = re.sub(r"/\*.*?\*/", " ", src, flags=re.S)
    src = re.sub(r"//[^\n]*", " ", src)
    return re.sub(r'"(\\.|[^"\\])*"', '""', src)


def crate_code(src_path):
    """All .rs files under the crate root's directory, less tests and benches."""
    text = []
    for dirpath, _, files in os.walk(os.path.dirname(src_path)):
        if "/tests" in dirpath or "/benches" in dirpath:
            continue
        for f in files:
            if f.endswith(".rs") and not re.search(r"test|bench", f):
                with open(os.path.join(dirpath, f), errors="replace") as fh:
                    text.append(strip(fh.read()))
    return "\n".join(text)


def cfg_true(cond, feats):
    """Evaluate a cfg predicate over features; test and unknown predicates are false."""
    cond = cond.strip()
    m = re.fullmatch(r'feature\s*=\s*"([^"]+)"', cond)
    if m:
        return m.group(1) in feats
    m = re.fullmatch(r"(not|all|any)\s*\((.*)\)", cond, re.S)
    if not m:
        return False
    args, depth, cur = [], 0, ""
    for ch in m.group(2):
        depth += (ch == "(") - (ch == ")")
        if ch == "," and depth == 0:
            args.append(cur)
            cur = ""
        else:
            cur += ch
    if cur.strip():
        args.append(cur)
    vals = [cfg_true(a, feats) for a in args]
    if m.group(1) == "not":
        return not vals[0]
    return all(vals) if m.group(1) == "all" else any(vals)


def no_std_as_built(root_src, code, feats):
    """Whether the crate builds without std with these features."""
    if re.search(r"#!\[\s*no_std\s*\]", root_src):
        # lazy_static style: a no_std root that still says `extern crate std`
        # unless a feature picks another module
        return not (re.search(r"^\s*extern crate std\s*;", code, re.M)
                    and not re.search(r"#\[cfg[^\]]*\]\s*extern crate std", code))
    # [^\]]: stay inside one attribute (a crate may have other cfg_attr lines first)
    m = re.search(r"#!\[\s*cfg_attr\(([^\]]*?),\s*no_std\s*\)\s*\]", root_src)
    return bool(m) and cfg_true(m.group(1), feats)


def main():
    probe, target, out = sys.argv[1:4]
    failed = set(open(sys.argv[4]).read().split()) if len(sys.argv) > 4 else set()
    rows = []
    for (name, version), (feats, src_path, source) in sorted(built_crates(probe, target).items()):
        with open(src_path, errors="replace") as fh:
            # only whole-line comments go: strip() would read the "//" of a URL
            # in #![doc(html_root_url = "https://...")] as a comment start
            root_src = re.sub(r"^\s*//[^\n]*", "", fh.read(), flags=re.M)
        code = crate_code(src_path)
        capable = bool(re.search(r"#!\[\s*(cfg_attr\([^\]]*\bno_std\b|no_std\b)", root_src))
        needs_std = (not no_std_as_built(root_src, code, feats) or "std" in feats
                     or name in failed)
        uses = [f for f, rx in FACILITIES if re.search(rx, code)]
        if not needs_std:
            cls = "ok"
        elif capable:
            cls = "feature"
        else:
            cls = uses[0] if uses else "alloc-only"
        named = ",".join(uses) or ("-" if re.search(r"\bstd::", code) else "none")
        rows.append((name, version, "no_std-capable" if capable else "std-only",
                     "needs-std" if needs_std else "-", cls, named, source))
    with open(out, "w") as fh:
        for r in rows:
            fh.write("\t".join(r) + "\n")
    counts = Counter(r[4] for r in rows)
    print(len(rows), "crates:", ", ".join(f"{k} {v}" for k, v in counts.most_common()))


if __name__ == "__main__":
    main()
