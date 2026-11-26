#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2s_dispatch_transcript_selftest.py - the DK's standalone verbs through the
tools/board.sh front door, on the fake board (no board, no ssh, no network).

tools/board.sh dispatches
  tools/board.sh [--dry-run] stm32mp257f-dk status | install | uninstall | nodefault
  tools/board.sh [--dry-run] stm32mp257f-dk default <image> <1-9|forever>
to tools/board-stm32mp2-standalone.sh (MP2S_STANDALONE_RUNNER overriding the
runner path), exec'ing it with --dry-run passed first and its exit code kept.
This checks that the front door is a pure pass-through, not a second
implementation. For every verb, run once through board.sh and once through the
runner directly with the same words in the same order; the two runs must agree:

  - the exit codes are the same, and each one the runner's documented code
    for that case (0 done, 2 refused before anything was changed);
  - the transcripts are byte for byte the same, except the runner's own
    scratch directory: its dry-run printer (the runner's host()) strips the
    tree path first, so when TMPDIR lies inside the tree - as in a gate row
    (tools/check.sh selftest_row sets it under build/) - a line keeps the
    tree prefix and the per-run mktemp name. The two invocations take
    different scratch directories, so this file hides the scratch path (with
    and without the tree prefix, the mktemp directory and its parent) behind
    <runner-scratch> in both transcripts before comparing.
    undo_order and protect_scan read the raw board.sh transcript, never the
    hidden one;
  - tools/mp2s_undo_order.py accepts that transcript (every undo printed
    before its change, docs/notes/stm32mp257f-dk-standalone.md section 5);
  - tools/mp2s_protect_scan.py, given that transcript as a file, finds no
    write of a protected U-Boot variable (bootcmd, bootcount, altbootcmd,
    ...), of a protected partition (fsbla1/2, metadata1/2, fip-a/b) or of
    the eMMC.

'forever' without BOARD_PERSON_PRESENT=1 must be refused through the front
door exactly as the runner refuses it: the same exit 2, a message naming
BOARD_PERSON_PRESENT, and no board line at all - nothing was about to change.
'default <image> forever' with BOARD_PERSON_PRESENT=1 is a dry run like any
other verb: it touches no board, and its two transcripts must agree as well.
After every case the fake board's writes.log is empty, holds no VIOLATION
line, and its /boot still matches the manifest taken before the first run
(tools/mp2s_boot_manifest.py).

The board is tools/mp2s_fakeboard.sh: init, then its shell environment (its
shims first in PATH, so even a stray ssh reaches the fake, never the network;
dry-run sends none either) is applied to every case. BOARD_HOST is the
placeholder user@example.invalid (RFC 2606; the fake ssh ignores it).
MP2S_STANDALONE_RUNNER is pinned to this checkout's runner, so an inherited
override cannot point the front door somewhere else.

Why this file is a .py and not a .sh. A tracked self-test that no row names
gets a gate row from tools/check_selftests.py (no row names this file, and
none is edited for it; #341 owns tools/check.sh), but the pull request that
adds it runs the rows its path reaches. In check.sh's path map a top-level
tools/*.sh that no entry lists falls to the `*` case and reaches everything:
its gate runs the whole plan, which on the shared runners takes from under
three hours to past the job's five-hour limit. Four attempts at this check as
tools/mp2s_dispatch_transcript_selftest.sh died there and none on its checks:
2026-11-25 saw one check job fail at 2 h 51 min (14fa7325), one at 5 h 0 min
(5e8d000c), one fail at 2 h 49 min (9ccd49a7) and pull request #13870
(2abe0e1d) cancel with "The job has exceeded the maximum execution time of
5h0m0s" - the full plan, twice run to exhaustion and twice run to a red row
that was main's, not this file's. A tools/*.py maps to the tools scope, so
this file's pull request runs the tools rows, as every merged standalone
self-test that is a .py (mp2s_boot_manifest_selftest.py,
mp2s_console_watch_selftest.py, mp2s_stage_selftest.py) already did.

A mismatch is a finding of the dispatch or of the runner, not of this file:
report it (a new issue), never bend a check here.

  tools/mp2s_dispatch_transcript_selftest.py

Prints one line per check and "mp2s_dispatch_transcript_selftest: N checks,
F failed"; exit 0 only when F is 0, 1 with findings, 2 on a setup error.
Writes only under TMPDIR (a /var/tmp stand-in when unset); MP2S_DT_KEEP set
to any value keeps this run's work directory (its path is printed).
"""
import atexit
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

NAME = "mp2s_dispatch_transcript_selftest"
HERE = Path(__file__).resolve().parent
HERE_LOGICAL = Path(__file__).absolute().parent
REPO = HERE.parent


def setup_fail(msg):
    print(f"{NAME}: {msg}", file=sys.stderr)
    sys.exit(2)


def run_tool(argv, env, out_file=None):
    """Run one command; merged stdout+stderr into out_file when given, else
    captured as text. Returns (exit code, output text)."""
    if out_file is not None:
        with open(out_file, "wb") as fh:
            p = subprocess.run([str(a) for a in argv], env=env,
                               stdout=fh, stderr=subprocess.STDOUT)
        data = open(out_file, "rb").read()
        return p.returncode, data.decode("utf-8", "replace")
    p = subprocess.run([str(a) for a in argv], env=env,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    return p.returncode, p.stdout.decode("utf-8", "replace")


def main():
    board = HERE / "board.sh"
    runner = HERE / "board-stm32mp2-standalone.sh"
    fakeboard = HERE / "mp2s_fakeboard.sh"
    undo_order = HERE / "mp2s_undo_order.py"
    protect_scan = HERE / "mp2s_protect_scan.py"
    image_check = HERE / "mp2s_image_check.py"
    manifest = HERE / "mp2s_boot_manifest.py"
    for f in (board, runner, fakeboard, undo_order, protect_scan,
              image_check, manifest):
        if not f.is_file():
            setup_fail(f"no {f}")

    # the runner keeps its scratch directory in TMPDIR: keep it under W
    # (nothing of a run is left behind) and under one known parent, so the
    # transcript comparison has exactly the mktemp name to hide
    tmp_base = os.environ.get("TMPDIR") or "/var/tmp"
    try:
        w = Path(tempfile.mkdtemp(prefix="mp2s_dispatch.", dir=tmp_base))
    except OSError:
        setup_fail("no work directory")
    keep_work = os.environ.get("MP2S_DT_KEEP") is not None

    @atexit.register
    def tidy():
        """This run's work directory goes when the run is over: on the clean
        way out, on setup_fail's exit 2, and on an unexpected error."""
        if keep_work:
            print(f"{NAME}: work directory kept: {w}")
            return
        subprocess.run(["bash", "-c", 'rm -rf -- "$1"', "_", str(w)],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    # ---- the environment this selftest owns: nothing inherited, nothing kept
    child_env = {}
    for k, v in os.environ.items():
        if k == "MP2S_DT_KEEP":
            continue
        if re.fullmatch(r"BOARD|BOARD_.*|MP2_.*|MP2S_.*|LOCK|LOCK_WAIT|HOLD_MAX", k):
            continue
        child_env[k] = v
    child_env["LC_ALL"] = "C"
    child_env["TMPDIR"] = str(w)
    child_env["MP2S_STANDALONE_RUNNER"] = str(runner)  # this checkout's runner
    child_env["MP2S_STATE_DIR"] = str(w / "state")
    child_env["BOARD_LOCK"] = str(w / "board.lock")
    child_env["LOCK_WAIT"] = "5"

    n = 0
    fails = 0

    def ok(name):
        nonlocal n
        n += 1
        print(f"ok   {name}")

    def bad(name, detail):
        nonlocal n, fails
        n += 1
        fails += 1
        print(f"FAIL {name}: {detail}")

    def checkrc(name, got, want, detail=""):
        if got == want:
            ok(f"{name} ({got})")
        else:
            bad(name, f"rc {got}, wanted {want}: {detail}")

    def short(text):
        return re.sub(r"\n", " ", text[:300])

    # ---- the fake board: init, then its shell environment applied to every
    # case (its shims first in PATH: no real ssh is reachable from this run)
    d = w / "fakeboard"
    init_log = w / "init.log"
    rc, out = run_tool([fakeboard, "init", d], child_env, init_log)
    if rc != 0:
        setup_fail(f"fake board init failed: {short(out)}")
    rc, snippet = run_tool([fakeboard, "shell", d], child_env)
    if rc != 0:
        setup_fail(f"fake board shell failed: {short(snippet)}")
    # the fakeboard's shell is `export PATH=<dir>/bin:$PATH` and
    # `export MP2S_FAKE_ROOT=<dir>`; let bash resolve both, exactly as a
    # caller's eval would
    r = subprocess.run(
        ["bash", "-c", 'eval "$1"; printf "%s\\0%s" "$PATH" "$MP2S_FAKE_ROOT"',
         "_", snippet],
        env=child_env, stdout=subprocess.PIPE)
    if r.returncode != 0:
        setup_fail("could not apply the fake board's shell environment")
    child_env["PATH"], fake_root = r.stdout.decode().split("\0")
    child_env["BOARD_HOST"] = "user@example.invalid"
    rc, where = run_tool(["bash", "-c", "command -v ssh"], child_env)
    if os.path.realpath(where.strip()) == os.path.realpath(str(d / "bin" / "ssh")):
        ok("ssh resolves to the fake's shim (no board, no network reachable from here)")
    else:
        bad("ssh resolves to the fake's shim", where.strip())

    # ---- the test image: the arm64 Image magic (ARMd at byte 56), a footprint
    # of 0 inside the go window, the board layer's watchdog marker and the
    # product refresh marker (the same image thus serves 'default 2' and
    # 'forever' with a person), and no 'yakogl-lifetime-s=' marker
    img = w / "img.bin"
    with open(img, "wb") as fh:
        fh.write(b"\0" * 56 + b"ARMd" + b"\0" * 4000
                 + b"gc_wdog_arm: armed" + b"\0" * 256
                 + b"mp2_wdtr: product refresh armed")
    rc, out = run_tool([sys.executable, "-I", image_check, img], child_env,
                       w / "ic.out")
    if rc != 0:
        setup_fail(f"the test image fails mp2s_image_check.py: {short(out)}")

    # the fake /boot as init made it: every later compare must say 'same'
    rc, out = run_tool([sys.executable, "-I", manifest, "record", d / "boot",
                        "-o", w / "first.manifest"], child_env, w / "rec.log")
    if rc != 0:
        setup_fail(f"mp2s_boot_manifest.py record failed: {short(out)}")

    # ---- transcript normalization: hide the runner's scratch, in every form
    # it can reach the output (the runner strips the tree prefix before its
    # own scratch path, so both the absolute and the tree-relative form appear)
    r = subprocess.run(["bash", "-c", 'cd "$1" && pwd', "_", str(w)],
                       env=child_env, stdout=subprocess.PIPE)
    w_forms = {str(w), r.stdout.decode().strip()}
    w_real = os.path.realpath(str(w))
    w_forms.add(w_real)
    rel_forms = set()
    for base in (HERE_LOGICAL.parent, Path.cwd().absolute()):
        prefix = str(base) + os.sep
        for form in list(w_forms):
            if form.startswith(prefix):
                rel_forms.add(form[len(prefix):])
    w_forms |= rel_forms
    forms = sorted((f for f in w_forms if f), key=len, reverse=True)
    alt = "|".join(re.escape(f) for f in forms)
    scratch_re = re.compile(f"(?:{alt})/tmp\\.[A-Za-z0-9]+")
    work_re = re.compile(f"(?:{alt})")

    def normalize(src, dst):
        text = open(src, "rb").read().decode("utf-8", "replace")
        text = scratch_re.sub("<runner-scratch>", text)
        text = work_re.sub("<runner-work>", text)
        with open(dst, "wb") as fh:
            fh.write(text.encode())

    # one_case NAME WANT_RC VAR=VALUE... -- ARGS...: run ARGS (the runner's
    # own arguments, --dry-run included) twice: directly on the runner, and
    # through board.sh --dry-run stm32mp257f-dk with the same words in the
    # same order.
    direct_out = w / "direct.out"
    via_out = w / "via.out"

    def one_case(name, want, overrides, args):
        env = dict(child_env)
        env.update(overrides)
        rc_direct, _ = run_tool(["bash", runner] + args, env, direct_out)
        rc_via, _ = run_tool(["bash", board, "--dry-run", "stm32mp257f-dk"] + args,
                             env, via_out)
        checkrc(f"{name}: direct exit", rc_direct, want, short(open(direct_out).read()))
        checkrc(f"{name}: board.sh keeps the runner's exit code", rc_via, rc_direct,
                f"direct {rc_direct}: {short(open(via_out).read())}")
        normalize(direct_out, w / "direct.norm")
        normalize(via_out, w / "via.norm")
        if open(w / "direct.norm", "rb").read() == open(w / "via.norm", "rb").read():
            lines = len(open(via_out, "rb").read().splitlines())
            ok(f"{name}: transcript identical through board.sh ({lines} lines)")
        else:
            direct_lines = open(w / "direct.norm").read().splitlines()
            via_lines = open(w / "via.norm").read().splitlines()
            diff = [f"-{a} +{b}" for a, b in zip(direct_lines, via_lines) if a != b]
            if len(direct_lines) != len(via_lines):
                diff.insert(0, f"line count {len(direct_lines)} vs {len(via_lines)}")
            bad(f"{name}: transcript", " ".join(diff[:6]))

    # transcript_checks NAME: the raw board.sh transcript passes the two
    # checks the design gives a dry-run transcript
    def transcript_checks(name):
        rc, out = run_tool([sys.executable, "-I", undo_order, via_out],
                           child_env, w / "undo.log")
        if rc == 0:
            ok(f"{name}: undo_order accepts the transcript ({short(out)})")
        else:
            bad(f"{name}: undo_order", short(out))
        rc, out = run_tool([sys.executable, "-I", protect_scan, via_out],
                           child_env)
        if rc == 0:
            ok(f"{name}: protect_scan finds no protected write in the transcript")
        else:
            bad(f"{name}: protect_scan", short(out))

    # ---- the five dispatched verbs, dry-run: identical doors, accepted
    # transcripts
    for verb in ("status", "install", "uninstall", "nodefault"):
        one_case(verb, 0, {}, ["--dry-run", verb])
        transcript_checks(verb)
    one_case("default 2", 0, {}, ["--dry-run", "default", str(img), "2"])
    transcript_checks("default 2")

    # ---- forever with a person, dry-run: a verb like any other through the
    # door
    one_case("default forever (person present)", 0,
             {"BOARD_PERSON_PRESENT": "1"},
             ["--dry-run", "default", str(img), "forever"])
    transcript_checks("default forever (person present)")

    # ---- forever without a person: refused at both doors, naming the gate,
    # with nothing about to change
    one_case("default forever (no person)", 2, {},
             ["--dry-run", "default", str(img), "forever"])
    via_text = open(via_out).read()
    direct_text = open(direct_out).read()
    if "BOARD_PERSON_PRESENT" in via_text:
        ok("the refusal through board.sh names BOARD_PERSON_PRESENT")
    else:
        bad("the refusal through board.sh names BOARD_PERSON_PRESENT", short(via_text))
    if "BOARD_PERSON_PRESENT" in direct_text:
        ok("the direct refusal names BOARD_PERSON_PRESENT")
    else:
        bad("the direct refusal names BOARD_PERSON_PRESENT", short(direct_text))
    dry_lines = [ln for ln in direct_text.splitlines() + via_text.splitlines()
                 if ln.startswith("DRY board$")]
    if not dry_lines:
        ok("both refusals printed no board line: they refused before anything would change")
    else:
        bad("both refusals printed no board line", " ".join(dry_lines[:2]))
    rc, out = run_tool([sys.executable, "-I", undo_order, via_out], child_env,
                       w / "undo.log")
    checkrc("the refusal is no dry-run transcript (undo_order's own verdict on it)",
            rc, 2, short(out))

    # ---- the door is exactly these verbs: the runner's read-only printenv is
    # not a board.sh verb, and board.sh says so itself, naming the runner,
    # exit 2
    rc, out = run_tool(["bash", board, "--dry-run", "stm32mp257f-dk", "printenv"],
                       child_env, w / "door.log")
    if rc == 2 and "board-stm32mp2-standalone.sh" in out:
        ok("printenv is refused by board.sh itself (exit 2, the message names the runner)")
    else:
        bad("printenv is refused by board.sh itself", f"rc {rc}: {short(out)}")

    # ---- nothing was written anywhere: a dry-run sent no change, to the fake
    # or to its /boot
    rc, writes = run_tool([fakeboard, "writes", d], child_env)
    if not writes.strip():
        ok("the fake board's writes.log is empty (the dry-runs wrote nothing)")
    else:
        bad("the fake board's writes.log is empty", " ".join(writes.splitlines()[:3]))
    violation_lines = [ln for ln in (d / "writes.log").read_text(errors="replace").splitlines()
                       if "VIOLATION" in ln]
    if not violation_lines:
        ok("no VIOLATION line in the fake board")
    else:
        bad("no VIOLATION line in the fake board", " ".join(violation_lines[:3]))
    rc, out = run_tool([sys.executable, "-I", manifest, "compare", d / "boot",
                        w / "first.manifest"], child_env, w / "cmp.log")
    if rc == 0 and "mp2s-boot: same" in out:
        ok("the fake board's /boot is byte for byte as init made it")
    else:
        bad("the fake board's /boot is byte for byte as init made it",
            f"rc {rc}: {short(out)}")

    print(f"{NAME}: {n} checks, {fails} failed")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
