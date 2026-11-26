#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2s_person_gate_selftest.py - the person gate of
tools/board-stm32mp2-standalone.sh (acceptance criterion c4 of plan #362):
`--dry-run default IMAGE forever` is refused for every value of
BOARD_PERSON_PRESENT but exactly 1.

For BOARD_PERSON_PRESENT unset, empty, 0, yes, true, ' 1', '1 ', 01 (and a
few more: 2, 11, -1, on, 1x, a newline after 1) each run must exit non-zero,
print a message naming BOARD_PERSON_PRESENT and print no change line (no
"DRY board$" or "DRY host$" line: the refusal comes before any command is
planned). With exactly 1 the person-gate message must be absent (the run may
still refuse later, e.g. the test image has no product marker; that is not
this test's concern). `default IMAGE 2` is not gated: with the variable unset
or 0 the run exits 0 and prints its planned changes, with no person-gate
message. Every run is a --dry-run against the fake board
(tools/mp2s_fakeboard.sh), whose writes.log must stay empty. No board, no
network, nothing written outside this run's temporary directory.

    tools/mp2s_person_gate_selftest.py

Prints "ok NAME" / "FAIL NAME: why" per check, then
"mp2s_person_gate_selftest: N checks, M failed"; exit 0 only when M is 0,
exit 2 on a setup error (a missing runner or fake board, the fake board
failing to init).

Why .py and not .sh: the path-to-scope map of tools/check.sh names
tools/*.py but no tools/mp2s_*.sh, so a pull request adding a .sh here
reaches scope `all` and runs every gate row; such a run does not finish
inside gate.yml's 300-minute job timeout (pull requests #11804 and #14240 -
the first attempt at this very file as a .sh - were both cancelled at exactly
five hours). As a .py the path reaches the `tools` scope, and
tools/check_selftests.py gives the file its own gate row from the commit
that adds it all the same (mp2s_tree_redact_selftest.py: the same choice,
for the same reason).
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
RUNNER = os.path.join(HERE, "board-stm32mp2-standalone.sh")
FAKE = os.path.join(HERE, "mp2s_fakeboard.sh")
RUN_TIMEOUT = 120  # one dry run is normally well under a second

GATE = "BOARD_PERSON_PRESENT"
# what a planned change looks like in a dry run (the runner's b()/host() lines)
CHANGE = re.compile(r"^DRY (board|host)\$", re.M)
# every value but exactly "1" must be refused for `forever` (None: unset)
VALUES = [None, "", "0", "yes", "true", " 1", "1 ", "01", "2", "11", "-1",
          "on", "1x", "1\n"]

N = 0
F = 0
W = None      # this run's temporary directory
IMG = None    # the test image, built in main()
FB_ENV = {}   # the fake board's PATH entry and MP2S_FAKE_ROOT


def label(value):
    if value is None:
        return "forever, BOARD_PERSON_PRESENT unset"
    return "forever, BOARD_PERSON_PRESENT=%s" % (repr(value),)


def ok(name):
    global N
    N += 1
    print("ok   %s" % name)


def bad(name, why):
    global N, F
    N += 1
    F += 1
    print("FAIL %s: %s" % (name, why))


def check(name, cond, why):
    if cond:
        ok(name)
    else:
        bad(name, why)


def die(msg):
    """Setup error: say what failed, exit 2 (a failed check exits 1)."""
    sys.stderr.write(msg + "\n")
    sys.exit(2)


def shell_env(fb):
    """The environment mp2s_fakeboard.sh `shell DIR` exports: the shims' bin/
    first in PATH and MP2S_FAKE_ROOT. Decoded through bash, the shell the
    snippet (a pair of `export NAME=%q...`) is written for."""
    try:
        snippet = subprocess.run([FAKE, "shell", fb], capture_output=True,
                                 text=True, check=True).stdout
        out = subprocess.run(["bash", "-c",
                              'eval -- "$1"\nprintf \'%s\\0%s\' "$PATH" "$MP2S_FAKE_ROOT"',
                              "-", snippet],
                             capture_output=True, text=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError) as err:
        die("mp2s_person_gate_selftest: %s shell failed: %s" % (FAKE, err))
    path, root = out.split("\0")
    return {"PATH": path, "MP2S_FAKE_ROOT": root}


def base_env():
    """The runner's environment: a clean one (no BOARD_* or MP2_* inherited
    from the calling shell - the dry-run rows of the gate run this way too),
    the fake board's shims, and a state directory of this run."""
    env = {}
    for keep in ("PATH", "HOME", "TMPDIR"):
        if keep in os.environ:
            env[keep] = os.environ[keep]
    env["LC_ALL"] = "C"
    env.update(FB_ENV)
    env["MP2S_STATE_DIR"] = os.path.join(W, "state")
    env["BOARD_LOCK"] = os.path.join(W, "board.lock")
    env["LOCK_WAIT"] = "5"
    env["BOARD_HOST"] = "u@fake"  # a placeholder: the fake ssh ignores it
    return env


def run_default(value, count):
    """--dry-run default IMAGE <count> with BOARD_PERSON_PRESENT set to VALUE
    (None: left unset). Returns (exit code, stdout and stderr together)."""
    env = base_env()
    if value is not None:
        env[GATE] = value
    try:
        r = subprocess.run([RUNNER, "--dry-run", "default", IMG, count],
                           env=env, capture_output=True, text=True,
                           errors="replace", timeout=RUN_TIMEOUT)
    except subprocess.TimeoutExpired:
        die("mp2s_person_gate_selftest: %s --dry-run default IMAGE %s did not "
            "finish in %d s" % (os.path.basename(RUNNER), count, RUN_TIMEOUT))
    return r.returncode, r.stdout + r.stderr


def main():
    global W, IMG, FB_ENV
    for tool, why in ((RUNNER, "the runner under test"),
                      (FAKE, "the fake board")):
        if not os.path.isfile(tool):
            die("mp2s_person_gate_selftest: missing %s (%s)" % (tool, why))
    W = tempfile.mkdtemp(prefix="mp2s_person_gate.")
    try:
        # the fake board: its shims first in PATH, every write logged
        fb = os.path.join(W, "fb")
        r = subprocess.run([FAKE, "init", fb], capture_output=True, text=True)
        if r.returncode != 0:
            die("mp2s_person_gate_selftest: fake board init failed: %s"
                % (r.stdout + r.stderr).strip())
        FB_ENV = shell_env(fb)

        # a valid test image, as tools/board_stm32mp2_standalone_selftest.sh
        # builds it: the AArch64 Image magic at byte 56 and the watchdog marker
        IMG = os.path.join(W, "img.bin")
        with open(IMG, "wb") as f:
            f.write(b"\0" * 56 + b"ARMd" + b"\0" * 4000
                    + b"gc_wdog_arm: armed")

        # 1. every value but exactly 1 is refused by the gate
        for v in VALUES:
            lbl = label(v)
            rc, out = run_default(v, "forever")
            check("%s: exits non-zero" % lbl, rc != 0, "rc 0: %s" % out)
            check("%s: the message names %s" % (lbl, GATE), GATE in out,
                  "not in: %s" % out)
            check("%s: no change line" % lbl, CHANGE.search(out) is None,
                  "found in: %s" % out)

        # 2. exactly 1 gets past the gate (a later refusal is allowed)
        rc, out = run_default("1", "forever")
        check("forever, BOARD_PERSON_PRESENT='1': past the person gate",
              GATE not in out, "gate message: %s" % out)

        # 3. a counted default is not gated
        for v in (None, "0"):
            lbl = "default 2, BOARD_PERSON_PRESENT %s" % \
                  ("unset" if v is None else "'%s'" % v)
            rc, out = run_default(v, "2")
            check("%s: not gated" % lbl, GATE not in out, "gate message: %s" % out)
            check("%s: the dry run succeeds" % lbl, rc == 0, "rc %d: %s" % (rc, out))
            check("%s: prints change lines" % lbl, CHANGE.search(out) is not None,
                  "none in: %s" % out)

        # 4. nothing was written on the fake board
        r = subprocess.run([FAKE, "writes", fb], capture_output=True, text=True)
        check("no write on the fake board",
              r.returncode == 0 and r.stdout == "",
              "rc %d: %s" % (r.returncode, r.stdout.strip()))
    finally:
        shutil.rmtree(W, ignore_errors=True)

    print("mp2s_person_gate_selftest: %d checks, %d failed" % (N, F))
    return 1 if F else 0


if __name__ == "__main__":
    sys.exit(main())
