#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Self-test of tools/check_live_tests.py and of the arguments tools/check.sh
makes from it (live_test_targets). No build: stand-in trees under TMPDIR for
the tool, and `tools/check.sh --live-test-args` on this tree for the gate.
No row names this file: the gate runs it as one of the self-tests
tools/check_selftests.py lists.
"""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import check_live_tests as live  # noqa: E402

fails = 0
checks = 0


def check(label, got, want):
    global fails, checks
    checks += 1
    if got != want:
        fails += 1
        print("FAIL %s: got %r, expected %r" % (label, got, want))


MANIFEST = """[package]
name = "x"
[features]
default = ["osal"]
osal = []
a = []
b = []
both = ["a", "b"]
"""

# file text -> is it empty for the features {a}, for {b}, for {a, b}
CASES = {
    "plain": ("fn f() {}\n", (False, False, False)),
    "feature_a": ('#![cfg(feature = "a")]\nfn f() {}\n', (False, True, False)),
    "not_a": ('#![cfg(not(feature = "a"))]\n', (True, False, True)),
    "all_ab": ('#![cfg(all(feature = "a", feature = "b"))]\n', (True, True, False)),
    "any_ab": ('#![cfg(any(feature = "a", feature = "b"))]\n', (False, False, False)),
    "a_not_b": ('#![cfg(all(feature = "a", not(feature = "b")))]\n', (False, True, True)),
    # comments, a licence header, other inner attributes and a doc comment first
    "after_head": ('// SPDX\n//! doc\n/* a /* nested */ block */\n#![allow(dead_code)]\n\n'
                   '#![cfg(feature = "a")]\nuse x::y;\n', (False, True, False)),
    "multi_line": ('#![cfg(all(\n    feature = "a",\n    not(feature = "b"),\n    target_os = "linux",\n))]\n',
                   (False, True, True)),
    # two attributes: empty when either is false
    "two_attrs": ('#![cfg(feature = "a")]\n#![cfg(feature = "b")]\n', (True, True, False)),
    # the host decides, the tool does not: built unless the features alone say no
    "host_only": ('#![cfg(target_os = "linux")]\n', (False, False, False)),
    "host_and_a": ('#![cfg(all(unix, feature = "a"))]\n', (False, True, False)),
    "host_or_a": ('#![cfg(any(windows, feature = "a"))]\n', (False, False, False)),
    "not_host": ('#![cfg(not(all(feature = "a", debug_assertions)))]\n', (False, False, False)),
    # never seen: an item first, a cfg in a module, in a comment, in a string, a cfg_attr
    "after_item": ('use x::y;\n#![cfg(feature = "b")]\n', (False, False, False)),
    "in_module": ('mod m {\n    #![cfg(feature = "b")]\n}\n', (False, False, False)),
    "in_comment": ('// #![cfg(feature = "b")]\n/* #![cfg(feature = "b")] */\nfn f() {}\n', (False, False, False)),
    "in_string": ('const S: &str = "\n#![cfg(feature = \\"b\\")]\n";\n', (False, False, False)),
    "cfg_attr": ('#![cfg_attr(feature = "b", allow(unused))]\n', (False, False, False)),
    "outer_attr": ('#[cfg(feature = "b")]\nfn f() {}\n', (False, False, False)),
    # not a predicate the tool reads: built
    "garbled": ('#![cfg(feature = )]\n', (False, False, False)),
    "two_preds": ('#![cfg(feature = "b", feature = "b")]\n', (False, False, False)),
    "unclosed": ('#![cfg(feature = "b")\nfn f() {}\n', (False, False, False)),
    "unclosed_comment": ('/* never closed\n#![cfg(feature = "b")]\n', (False, False, False)),
    # a feature that turns others on
    "wants_b": ('#![cfg(feature = "b")]\n', (True, False, False)),
    # the default feature, with and without --no-default-features
    "wants_osal": ('#![cfg(feature = "osal")]\n', (True, True, True)),
}


def tree(tmp, manifest=MANIFEST, extra=()):
    root = tempfile.mkdtemp(dir=tmp)
    os.makedirs(os.path.join(root, "tests", "common"))
    os.makedirs(os.path.join(root, "src"))
    with open(os.path.join(root, "Cargo.toml"), "w") as f:
        f.write(manifest)
    for name, (text, _) in CASES.items():
        with open(os.path.join(root, "tests", name + ".rs"), "w") as f:
            f.write(text)
    # a module of the tests, not a target
    with open(os.path.join(root, "tests", "common", "mod.rs"), "w") as f:
        f.write('#![cfg(feature = "b")]\n')
    for rel in extra:
        os.makedirs(os.path.dirname(os.path.join(root, rel)), exist_ok=True)
        open(os.path.join(root, rel), "w").close()
    return root


def empty_of(root, features, default=False):
    return live.split(root, features, default)[1]


with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR") or None) as tmp:
    root = tree(tmp)
    for col, feats in enumerate(("a", "b", "a,b")):
        got = set(empty_of(root, feats))
        for name, (_, want) in CASES.items():
            check("%s with %s: empty" % (name, feats), name in got, want[col])
    check("the targets are the root files", sorted(live.targets(root)), sorted(CASES))
    live_names, gone = live.split(root, "a", False)
    check("every target is live or empty, once", sorted(live_names + gone), sorted(CASES))
    # `both` turns a and b on
    check("a feature that names others", sorted(empty_of(root, "both")), sorted(empty_of(root, "a,b")))
    # the default features are on unless the row says --no-default-features
    check("wants_osal with the defaults", "wants_osal" in empty_of(root, "a", default=True), False)
    check("wants_osal without them", "wants_osal" in empty_of(root, "a", default=False), True)
    # cannot tell: every target is built
    check("a feature Cargo.toml lacks", empty_of(root, "a,nosuch"), [])
    check("a dependency's feature", empty_of(tree(tmp, MANIFEST + 'dep = ["dep:x"]\n'), "dep"), [])
    check("no Cargo.toml", empty_of(tree(tmp, "not toml ["), "a"), [])
    check("a [[test]] table", empty_of(tree(tmp, MANIFEST + '[[test]]\nname = "t"\n'), "a"), [])
    check("a [[bin]] table", empty_of(tree(tmp, MANIFEST + '[[bin]]\nname = "t"\n'), "a"), [])
    check("autotests", empty_of(tree(tmp, MANIFEST.replace('name = "x"', 'name = "x"\nautotests = false')), "a"), [])
    for rel in ("src/main.rs", "src/bin/t.rs", "examples/e.rs", "examples/e/main.rs", "tests/dir/main.rs"):
        check("another target, %s" % rel, empty_of(tree(tmp, extra=[rel]), "a"), [])
    # the control: the same tree without them does leave targets out
    check("the control leaves targets out", len(empty_of(root, "a")) > 0, True)

    # the command line
    env = dict(os.environ, YAKOGL_LIVE_TESTS_ROOT=root)
    tool = [sys.executable, "-I", os.path.join(HERE, "check_live_tests.py")]
    out = subprocess.run(tool + ["--split", "--no-default-features", "a"], env=env, capture_output=True, text=True)
    lines = out.stdout.split("\n")
    check("--split: exit", out.returncode, 0)
    check("--split: one line per target", len([x for x in lines if x]), len(CASES))
    check("--split: an empty target", "empty wants_b" in lines, True)
    check("--split: a live target", "live feature_a" in lines, True)
    out = subprocess.run(tool + ["--count", "--no-default-features", "a", "b"], env=env, capture_output=True, text=True)
    check("--count: one line per feature set", out.stdout.count("\n"), 2)
    for bad in ([], ["a"], ["--split"], ["--split", "a", "b"], ["--split", "--count", "a"], ["--nosuch", "a"]):
        check("usage error for %r" % bad, subprocess.run(tool + bad, env=env, capture_output=True).returncode, 2)


# The gate's arguments, on this tree (nothing is built).
def gate_args(*args, mode=None):
    env = dict(os.environ)
    env.pop("YAKOGL_CHECK_EMPTY_TARGETS", None)
    if mode:
        env["YAKOGL_CHECK_EMPTY_TARGETS"] = mode
    out = subprocess.run([os.path.join(HERE, "check.sh"), "--live-test-args"] + list(args),
                         env=env, capture_output=True, text=True)
    lines = [x for x in out.stdout.split("\n") if x and not x.startswith("check.sh: ")]
    return out.returncode, lines


real = sorted(live.targets(ROOT))
for feats in ("host-test,gpu-vivante,shaderc-online", "host-test,gpu-powervr"):
    names, gone = live.split(ROOT, feats, False)
    head = ["--no-default-features", "--features", feats]
    rc, got = gate_args(*head)
    want = head + ["--lib"] + [x for t in names for x in ("--test", t)]
    check("check.sh, a whole-suite row of %s: exit" % feats, rc, 0)
    check("check.sh, a whole-suite row of %s: the library and the live targets" % feats, got, want if gone else head)
    rc, got = gate_args(*head, "--tests")
    check("check.sh, --tests of %s: the live targets in its place" % feats, got,
          head + [x for t in names for x in ("--test", t)] if gone else head + ["--tests"])
    # on main: nothing is left out, and the empty ones are to be proved
    rc, got = gate_args(*head, mode="prove")
    check("check.sh, prove: the arguments as given", got[:3], head)
    check("check.sh, prove: the empty targets are named", got[3:], ["prove: " + " ".join(gone) + " "] if gone else [])
    check("live and empty are the targets of tests/", sorted(names + gone), real)
# This tree has feature-gated tests: a reading that left none out would pass
# every check above while the rows built everything.
check("this tree: a GPU's row leaves targets out",
      len(live.split(ROOT, "host-test,gpu-powervr", False)[1]) > 0, True)
# a row that names its package, its targets or every feature is left alone
for row in (["-p", "yakogl-tools"],
            ["--no-default-features", "--features", "host-test,gpu-powervr", "--test", "sha256"],
            ["--no-default-features", "--features", "host-test,gpu-powervr", "--lib"],
            ["--no-default-features", "--features", "host-test,gpu-powervr", "--lib", "--tests"],
            ["--no-default-features", "--features=host-test,gpu-powervr"],
            ["--all-features"],
            ["-p", "yakogl-drivers", "--all-features"]):
    rc, got = gate_args(*row)
    check("check.sh leaves %r as it is" % row, (rc, got), (0, row))

print("check_live_tests_selftest: %d checks run, %d failed" % (checks, fails))
sys.exit(1 if fails else 0)
