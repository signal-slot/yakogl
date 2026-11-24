#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2s_tree_redact_selftest.py - no committed DK standalone file carries a
board address, host name, UART path or serial (plan #362, item
tree-redact-selftest).

    tools/mp2s_tree_redact_selftest.py [-v]        (-v: also the allowed count)

Runs tools/mp2s_redact_check.py over every tracked file under the DK
standalone paths (PATHSPECS below: the design notes, driver/tools/
mp2-standalone, the tools/mp2s_ family, the standalone runner and its
selftest, the env and EFI fixtures, the comeback tools). Binary files are
scanned through their printable-character runs (what `strings -n 4` prints):
the checker and the release redaction scan both skip binary files, and a
padded env fixture is exactly where a copied serverip hides, so this scan
looks there too. For a binary file the reported line numbers are of that
string view, not of the file's bytes.

A finding fails the run unless its file is in ALLOW below (fixture inputs
whose job is to hold fake secrets, ST's stock env defaults, public upstream
URLs). Every allowed path was read by hand. A real leak is never whitelisted
here: the run fails and prints path:line:kind (the checker masks the match
itself, so the finding line carries no value) for a new issue.

Self-check: fake values of every guarded kind (an address in the CGNAT
range, a host name, a UART path, a serial# value) planted in a scratch file
must come back as findings of those kinds. The planted values are assembled
at run time: this file holds no needle literally, and scanning it must stay
clean.

Why .py and not .sh: tools/check.sh's path-to-scope map names tools/*.py
but no tools/mp2s_*.sh, so a pull request adding a .sh here reaches scope
`all` and runs every gate row; pull request #11804 showed such a run does
not finish inside gate.yml's 300-minute job timeout (its check job was
cancelled at exactly five hours, as an earlier attempt's had been). As a
.py the path reaches the `tools` scope, and a selftest.py that no row names
still gets a gate row through tools/check_selftests.py all the same: the
file is in the gate from the commit that adds it, and its pull request runs
the tools rows.

Summary: "mp2s_tree_redact_selftest: N files, F findings". Exit 0 clean,
1 findings, 2 setup error (no checker, no git, no file matched). Writes
only under TMPDIR (/var/tmp when TMPDIR is unset).
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CHK = "tools/mp2s_redact_check.py"

PATHSPECS = (
    "docs/notes/stm32mp257f-dk-standalone.md",
    "docs/notes/stm32mp257f-dk-standalone-boot.md",
    "driver/tools/mp2-standalone/**",
    "tools/mp2s_*",
    "tools/board-stm32mp2-standalone.sh",
    "tools/board_stm32mp2_standalone_selftest.sh",
    "tools/testdata/mp2s-*/**",
    "tools/testdata/efi_varfile*",
    "tools/fwu_metadata*",
    "tools/efi_varfile*",
)

# path :: why its findings are not board identity. Read by hand at writing
# time; a finding in a path not listed here fails the run.
ALLOW = (
    ("driver/tools/mp2-standalone/st-env/stm32mp25-dk-initial-env.txt",
     "ST stock serverip= factory default, a public default, not a lab address"),
    ("driver/tools/mp2-standalone/qemu/README.md",
     "U-Boot's canonical git host, a public upstream URL"),
    ("driver/tools/mp2-standalone/qemu/build-uboot.sh",
     "U-Boot's canonical git host, a public upstream URL"),
    ("driver/tools/mp2-standalone/qemu-card/README.md",
     "U-Boot's canonical git host, a public upstream URL"),
    ("driver/tools/mp2-standalone/qemu-product/README.md",
     "U-Boot's canonical git host, a public upstream URL"),
    ("tools/mp2s_redact_check.py",
     "the checker's own docstring names the tty path patterns it scans for"),
    ("tools/mp2s_redact_check_selftest.sh",
     "deliberate fake addresses, hosts, tty, serial# and secrets: the "
     "redaction checker's own test inputs"),
    ("tools/mp2s_report.py",
     "docstring names tty path patterns"),
    ("tools/mp2s_report_selftest.sh",
     "deliberate fake host, tty, serial# and IPv6 values: the report "
     "redaction test's inputs"),
    ("tools/mp2s_env_diff_selftest.sh",
     "fake env dumps on documentation-range addresses and invented MACs: "
     "the env diff test's inputs"),
    ("tools/mp2s_fakeboard_selftest.sh",
     "the bare tailnet-suffix needle it searches captured storage for"),
    ("tools/testdata/mp2s-env/both-bad.bin", "ST stock env serverip default"),
    ("tools/testdata/mp2s-env/counting1.bin", "ST stock env serverip default"),
    ("tools/testdata/mp2s-env/counting2.bin", "ST stock env serverip default"),
    ("tools/testdata/mp2s-env/installed.bin", "ST stock env serverip default"),
    ("tools/testdata/mp2s-env/left0.bin", "ST stock env serverip default"),
    ("tools/testdata/mp2s-env/one-copy-bad.bin", "ST stock env serverip default"),
    ("tools/testdata/mp2s-env/pristine.bin", "ST stock env serverip default"),
)
ALLOWED = {p for p, _ in ALLOW}

# checker lines are "path:line:kind:masked", kind one of ipv4 ipv6 mac
# tailnet fqdn tty serial secret env:BOARD_<NAME>
FINDING = re.compile(r"^(.*?):(\d+):([a-z][a-z0-9]*(?::BOARD_[A-Z_]+)?):(.*)$")


def die(msg):
    """Setup error: say what failed, exit 2 (a finding exits 1)."""
    sys.stderr.write(msg + "\n")
    sys.exit(2)


def tracked_files():
    try:
        out = subprocess.run(["git", "ls-files", "--", *PATHSPECS],
                             cwd=ROOT, check=True, capture_output=True, text=True)
    except (OSError, subprocess.CalledProcessError):
        die("mp2s_tree_redact_selftest: git ls-files failed")
    return sorted(f for f in out.stdout.split("\n") if f)


def strings_view(path):
    """The printable-character runs of length 4 or more, like `strings -n 4`.

    The checker skips a file whose head holds a NUL byte; a padded env
    fixture is scanned through these runs instead. Bytes are read as
    latin-1 so every byte maps to one character.
    """
    with open(path, "rb") as f:
        text = f.read().decode("latin-1")
    runs, run = [], []
    for ch in text:
        if ch == "\t" or " " <= ch <= "~":
            run.append(ch)
        else:
            if len(run) >= 4:
                runs.append("".join(run))
            run = []
    if len(run) >= 4:
        runs.append("".join(run))
    return "\n".join(runs) + ("\n" if runs else "")


def run_checker(paths):
    """[(path, line, kind, masked)] for the checker over those paths."""
    res = subprocess.run([sys.executable, "-I", CHK, *paths],
                         cwd=ROOT, capture_output=True, text=True, errors="replace")
    if res.stderr.strip():
        sys.stderr.write(res.stderr)
    if res.returncode not in (0, 1):
        die("mp2s_tree_redact_selftest: %s exited %d" % (CHK, res.returncode))
    out = []
    for line in res.stdout.splitlines():
        m = FINDING.match(line)
        if not m:
            sys.stderr.write("mp2s_tree_redact_selftest: unparsable checker "
                             "line: %s\n" % line)
            continue
        path, n, kind, masked = m.group(1), int(m.group(2)), m.group(3), m.group(4)
        out.append((path, n, kind, masked))
    return out


def main(argv):
    verbose = False
    for a in argv:
        if a == "-v":
            verbose = True
        elif a in ("-h", "--help"):
            sys.stdout.write(__doc__)
            return 0
        else:
            die("usage: tools/mp2s_tree_redact_selftest.py [-v]")
    if not os.path.isfile(os.path.join(ROOT, CHK)):
        die("mp2s_tree_redact_selftest: no %s" % CHK)

    files = tracked_files()
    if not files:
        die("mp2s_tree_redact_selftest: no files matched")
    n_files = len(files)

    try:  # a scratch dir for the binary string views and the self-check file
        scratch = tempfile.mkdtemp(prefix="mp2s_tree_redact.",
                                   dir=os.environ.get("TMPDIR") or "/var/tmp")
    except OSError as e:
        die("mp2s_tree_redact_selftest: no scratch directory: %s" % e)
    try:
        paths, svmap = [], {}
        for i, f in enumerate(files):
            p = os.path.join(ROOT, f)
            if not os.path.isfile(p):  # tracked but not on disk: nothing to read
                continue
            with open(p, "rb") as fh:
                binary = b"\0" in fh.read(8192)
            if binary:
                sv = os.path.join(scratch, "sv%04d.txt" % i)
                with open(sv, "w", encoding="latin-1", newline="") as fh:
                    fh.write(strings_view(p))
                svmap[os.path.abspath(sv)] = f
                paths.append(sv)
            else:
                paths.append(f)

        findings, allowed = 0, 0
        for path, line, kind, masked in run_checker(paths):
            ap = svmap.get(path, path)
            if ap in ALLOWED:
                allowed += 1
                continue
            findings += 1
            print("FINDING %s:%d:%s:%s" % (ap, line, kind, masked))

        # Self-check: a fake of every guarded kind, planted in scratch, must
        # come back as a finding of that kind. Assembled at run time so this
        # file holds no value and its own scan stays clean.
        v4 = "".join(["100.64.", str(12), ".34"])       # a CGNAT-range address
        host = "".join(["dk-selftest", ".", "lan"])     # a host name
        tty = "".join(["/dev/", "ttyMP2SELFTEST"])     # a UART path
        serial = "".join(["iSerial", "=", "SELFTESTFAKE"])  # a serial# value
        planted = os.path.join(scratch, "selfcheck.txt")
        with open(planted, "w", encoding="utf-8") as fh:
            fh.write("addr %s here\nhost %s here\nconsole %s open\ndevice %s\n"
                     % (v4, host, tty, serial))
        want = {"ipv4", "fqdn", "tty", "serial"}
        got = {kind for _, _, kind, _ in run_checker([os.path.abspath(planted)])}
        if want <= got:
            print("self-check ok: planted address, host, UART path and serial# "
                  "all found")
        else:
            print("self-check FAILED: planted values not found for "
                  + ", ".join(sorted(want - got)))
            findings += 1

        if verbose:
            print("allowed by list: %d" % allowed)
        print("mp2s_tree_redact_selftest: %d files, %d findings" % (n_files, findings))
        return 1 if findings else 0
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
