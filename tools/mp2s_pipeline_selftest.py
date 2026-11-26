#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2s_pipeline_selftest.py - the DK standalone c6 verdict chain end to end on
the host (plan #362): tools/mp2s_script_sim.py's boots of
driver/tools/mp2-standalone/mp2-standalone.cmd, rendered as a console record,
through tools/mp2s_record_check.py, and a record directory in
tools/mp2s_session.sh's layout through tools/mp2s_report.py.

    python3 -I tools/mp2s_pipeline_selftest.py

(a) script_sim's `run --left 2` transcript (no power cut), with the console
    lines the simulator does not print put in where its "sim:" lines say they
    happen (TF-A's "Reset reason:" and "BL2: v" lines at each boot, the
    image's lines at "the image runs", the kernel and a login prompt at
    "booting Linux"), is a console.log: record_check --count 2 must say
    VERDICT: PASS, its boots-left lines must be the simulator's, and
    --count 1 / --count 3 on the same log must say FAIL.
(b) the same for a power cut at each saveenv (before the write, torn, after
    the write), each transcript taken from script_sim's own Sim class (the
    tool is imported, not changed). The cut boot ends where the simulator says
    so. Two renderings: the boot after the cut with BL2's "Power-on reset"
    (what a cut shows) must be a FAIL naming the supply; with a warm reset (a
    cut BL2 would not see) record_check must never say PASS where the
    simulator says the count went wrong (image runs not N, or Linux not at
    boot N+1 without the cut boot), and where both say the boot is safe they
    agree on the image boots. script_sim's own `faults` verdict per case must
    match the safe/unsafe this file computes.
(c) a record directory as tools/mp2s_session.sh writes it (raw.log,
    console.log with mp2s_console_watch.py's "<unix time> " stamps,
    console-verdict.txt, record-verdict.txt from record_check on that log,
    env-before/after.txt, boot-before/after.txt, session.txt) under
    <stamp>-standalone-<sha8>: mp2s_report.py must exit 0 and quote the
    "boots left 1" and "boots left 0" lines and the byte-identical stock boot
    files; with "boots left 0" gone from the logs it must exit 3.

No board, no network, nothing written outside $TMPDIR (the tools run under
python3 -I -B, and script_sim is imported with bytecode writing off: no
__pycache__ next to the tools). Prints "ok:"/"FAIL:" per check and
"N checks, M failed" last; exit 1 if any failed. This is a .py so that the
scope map of tools/check.sh keeps it in `tools`.
"""
import argparse
import importlib.util
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SCRIPT = os.path.join(ROOT, "driver", "tools", "mp2-standalone", "mp2-standalone.cmd")
SIM = os.path.join(HERE, "mp2s_script_sim.py")
CHECK = os.path.join(HERE, "mp2s_record_check.py")
REPORT = os.path.join(HERE, "mp2s_report.py")
N = 2

WARM = "System reset (SYSRST)"
POWER_ON = "Power-on reset (por_rstn)"
BOOT_RX = re.compile(r"^sim: boot (\d+): (env copy|both env copies bad)")
IMAGE_RX = re.compile(r"^sim: boot \d+: the image runs")
LINUX_RX = re.compile(r"^sim: boot \d+: prefix \S+: ST's extlinux, booting Linux")
CUT_RX = re.compile(r"^sim: boot (\d+): saveenv \d+: power cut")
BOOTS_LEFT_RX = re.compile(r"boots left [0-9]*")


class Tally:
    def __init__(self):
        self.checks = 0
        self.failed = 0

    def ok(self, msg):
        self.checks += 1
        print("ok: " + msg)

    def bad(self, msg, detail=None):
        self.checks += 1
        self.failed += 1
        print("FAIL: " + msg)
        if detail:
            for ln in detail.splitlines():
                print("    " + ln)

    def check(self, cond, good, wrong, detail=None):
        if cond:
            self.ok(good)
        else:
            self.bad(wrong, detail)
        return cond


def render(lines, cut_reason=WARM):
    """The console a DK would print for these simulator lines."""
    out = ["[  OK  ] Reached target System Reboot.", "reboot: Restarting system"]
    after_cut = False
    for ln in lines:
        m = BOOT_RX.match(ln)
        if m:
            reason = cut_reason if after_cut else WARM
            after_cut = False
            out += ["NOTICE:  Reset reason: %s (0x2074). Warm boot." % reason,
                    "NOTICE:  BL2: v2.10-stm32mp2-r2.0(release):",
                    "NOTICE:  BL31: v2.10-stm32mp2-r2.0(release):",
                    "I/TC: OP-TEE version: 4.0.0",
                    "U-Boot 2023.10-stm32mp-r2 (Oct 01 2023 - 00:00:00 +0000)",
                    "Hit any key to stop autoboot:  0"]
            if m.group(2).startswith("both"):
                out.append("*** Warning - bad CRC, using default environment")
            out.append("switch to partitions #0, OK")
            continue
        if IMAGE_RX.match(ln):
            out += ["## Starting application at 0xa0000000 ...",
                    "*** Booting Zephyr OS build v4.2.0 ***",
                    "mp2_hello: stm32mp257f_dk, console usart2 at 115200 baud",
                    "mp2: mp2_hello done: PSCI SYSTEM_RESET at 20000 ms"]
            continue
        if LINUX_RX.match(ln):
            out += ["Retrieving file: /extlinux/extlinux.conf",
                    "Starting kernel ...",
                    "[    0.000000] Linux version 6.6.48 (oe-user@oe-host)",
                    "[  OK  ] Reached target Multi-User System.",
                    "stm32mp2 login: "]
            continue
        if CUT_RX.match(ln):
            after_cut = True  # the console stops here: power gone
            continue
        if ln.startswith("mp2s: ") or ln.startswith("SCRIPT FAILED"):
            out.append(ln)
    return "\r\n".join(out) + "\r\n"


def load_sim(path):
    sys.dont_write_bytecode = True
    spec = importlib.util.spec_from_file_location("mp2s_script_sim", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def sim_args(script, left):
    return argparse.Namespace(
        mode="run", script=script, left=str(left), env=None, set=[], unset=[],
        no_script=False, no_image=False, image=None, load_fails=False,
        image_returns=False, saveenv_fails=False, start_flags="5,6",
        max_boots=20, until_restored=False, check_restored=False)


def verdicts(sim, n):
    """(safe, count_ok, linux_boot_without_cut) as script_sim's faults judges."""
    linux = sim.linux_boots[0] - (1 if sim.cut_boot else 0) if sim.linux_boots else None
    env, _, envbad = sim.env_report()
    safe = (linux is not None and linux <= n + 1 and sim.image_runs <= n
            and not envbad and sim.restored(env))
    count_ok = safe and sim.image_runs == n and linux == n + 1
    return safe, count_ok, linux


def tool(*argv):
    """(exit code, stdout+stderr) of a tool run under python3 -I -B."""
    p = subprocess.run([sys.executable, "-I", "-B", *argv], stdout=subprocess.PIPE,
                       stderr=subprocess.STDOUT, text=True, errors="replace")
    return p.returncode, p.stdout


def write(path, text):
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def read(path):
    with open(path, encoding="utf-8", errors="replace", newline="") as f:
        return f.read()


def image_boots(check_out):
    """The image boots record_check's table marks "yes", one per line."""
    lines = check_out.splitlines()[1:]
    return sum(1 for ln in lines if not ln.startswith("VERDICT") and " yes  " in ln)


def boots_left_seq(text):
    return " ".join(BOOTS_LEFT_RX.findall(text))


def part_a(t, tmp):
    print("== (a) script_sim run --left %d through record_check" % N)
    rc, sim_out = tool(SIM, "run", SCRIPT, "--left", str(N), "--check-restored")
    want = "sim: result: Linux at boot %d, boots %d, image runs %d," % (N + 1, N + 1, N)
    t.check(rc == 0 and any(ln.startswith(want) for ln in sim_out.splitlines()),
            "(a) script_sim: Linux at boot %d after %d image runs, env restored" % (N + 1, N),
            "(a) script_sim run --left %d: rc %d" % (N, rc), sim_out)
    console = os.path.join(tmp, "a.console.log")
    write(console, render(sim_out.splitlines()))

    rc, chk = tool(CHECK, console, "--count", str(N))
    t.check(rc == 0 and re.search(r"^VERDICT: PASS %d image boot\(s\) then Linux" % N, chk, re.M),
            "(a) record_check --count %d: VERDICT: PASS" % N,
            "(a) record_check --count %d: rc %d, want VERDICT: PASS" % (N, rc), chk)
    want_seq = boots_left_seq("\n".join(ln for ln in sim_out.splitlines() if ln.startswith("mp2s: ")))
    got_seq = boots_left_seq(chk)
    t.check(want_seq and want_seq == got_seq,
            "(a) record_check's boots left lines are the simulator's: " + want_seq,
            "(a) boots left: simulator '%s', record_check '%s'" % (want_seq, got_seq))
    got = image_boots(chk)
    t.check(got == N,
            "(a) record_check counts %d image boots, as the simulator's image runs" % N,
            "(a) record_check counts '%d' image boots, the simulator %d" % (got, N))
    for c in (N - 1, N + 1):
        rc, out = tool(CHECK, console, "--count", str(c))
        t.check(rc == 1 and re.search(r"^VERDICT: FAIL", out, re.M),
                "(a) the same log with --count %d: VERDICT: FAIL" % c,
                "(a) the same log with --count %d: rc %d, want VERDICT: FAIL" % (c, rc), out)
    return console


def make_cases(outdir, n):
    """Every power-cut case of script_sim's Sim: rows of
    (name, i, kind, image_runs, linux, safe, count_ok)."""
    mod = load_sim(SIM)
    args = sim_args(SCRIPT, n)
    ast, slen, stock, start = mod.build(args)
    clean = mod.Sim(ast, slen, stock, start, args, quiet=True)
    clean.run()
    rows = []
    for i in range(1, clean.saves + 1):
        for kind in ("before", "torn", "after"):
            sim = mod.Sim(ast, slen, stock, start, args, cut=(i, kind), quiet=True)
            sim.run()
            safe, count_ok, linux = verdicts(sim, n)
            name = "cut%d-%s" % (i, kind)
            for variant, reason in (("warm", WARM), ("poweron", POWER_ON)):
                write(os.path.join(outdir, "%s-%s.log" % (name, variant)), render(sim.lines, reason))
            write(os.path.join(outdir, name + ".sim.txt"), "\n".join(sim.lines) + "\n")
            rows.append((name, i, kind, sim.image_runs, linux, int(bool(safe)), int(bool(count_ok))))
    return clean.saves, rows


def part_b(t, tmp):
    print("== (b) a power cut at each saveenv of --left %d" % N)
    bdir = os.path.join(tmp, "b")
    os.makedirs(bdir)
    try:
        saves, rows = make_cases(bdir, N)
    except Exception as e:  # the simulator's API moved: one failure, not a traceback
        t.bad("(b) the power-cut transcripts were not made: %r" % e)
        return
    if not t.check(rows, "(b) script_sim's Sim ran every cut: saves %d, cases %d" % (saves, len(rows)),
                   "(b) the power-cut transcripts were not made: no saveenv in the clean run"):
        return
    # script_sim's own faults verdict per case, to hold this file's reading of it
    fault_rc, faults = tool(SIM, "faults", SCRIPT, "--left", str(N))
    t.check(re.search(r"^sim: faults: N=%d, " % N, faults, re.M),
            "(b) script_sim faults --left %d ran (exit %d)" % (N, fault_rc),
            "(b) script_sim faults --left %d: no summary, exit %d" % (N, fault_rc), faults)
    conservative = []
    for name, i, kind, runs, linux, safe, count_ok in rows:
        linux_s = "-" if linux is None else str(linux)
        m = re.search(r"^sim: fault: saveenv %d of [0-9]*, cut %s: (.*)$" % (i, kind), faults, re.M)
        line = m.group(0) if m else ""
        tool_safe = None
        if line.endswith(": ok"):
            tool_safe = 1
        elif ": FAIL" in line:
            tool_safe = 0
        t.check(tool_safe == safe,
                "(b) %s: script_sim faults and this test read the case alike (safe=%d, image runs %d, "
                "Linux at boot %s without the cut)" % (name, safe, runs, linux_s),
                "(b) %s: script_sim faults says '%s', this test safe=%d" % (name, line or "nothing", safe))

        # the boot after the cut as BL2 shows a power cut
        rc, out = tool(CHECK, os.path.join(bdir, name + "-poweron.log"), "--count", str(N))
        t.check(rc == 1 and re.search(r"^VERDICT: FAIL .*supply suspect", out, re.M),
                "(b) %s, Power-on reset after the cut: VERDICT: FAIL, supply suspect" % name,
                "(b) %s, Power-on reset after the cut: rc %d, want VERDICT: FAIL ... supply suspect"
                % (name, rc), out)

        # the cut not seen by BL2 (a warm reset): never PASS where the count went wrong
        rc, out = tool(CHECK, os.path.join(bdir, name + "-warm.log"), "--count", str(N))
        vm = re.search(r"^VERDICT: .*$", out, re.M)
        verdict = vm.group(0) if vm else ""
        if verdict.startswith("VERDICT: PASS"):
            got = image_boots(out)
            if count_ok and got == runs:
                t.ok("(b) %s, warm reset: both say the boot is safe and agree on %d image boots" % (name, runs))
            elif count_ok:
                t.bad("(b) %s, warm reset: both PASS, record_check counts %d image boots, the simulator %d"
                      % (name, got, runs))
            else:
                t.bad("(b) %s, warm reset: record_check PASS where the simulator says the count went wrong "
                      "(image runs %d, Linux at boot %s without the cut, safe=%d)" % (name, runs, linux_s, safe),
                      read(os.path.join(bdir, name + ".sim.txt")))
        elif verdict.startswith("VERDICT: FAIL"):
            if rc == 1:
                t.ok("(b) %s, warm reset: record_check FAIL (simulator: image runs %d, count_ok=%d): %s"
                     % (name, runs, count_ok, verdict[len("VERDICT: FAIL "):]))
                if count_ok:
                    conservative.append(name)
            else:
                t.bad("(b) %s, warm reset: VERDICT: FAIL with exit %d" % (name, rc))
        else:
            t.bad("(b) %s, warm reset: no verdict (rc %d)" % (name, rc), out)
    if conservative:
        print("note: record_check fails, the simulator's count is right (the record shows the cut boot): "
              + " ".join(conservative))


def part_c(t, tmp, console):
    print("== (c) a record directory in mp2s_session.sh's layout through mp2s_report.py")
    rec = os.path.join(tmp, "records", "stm32mp257f-dk", "20261125-1200-standalone-28c094e9")
    os.makedirs(rec)
    raw = read(console)
    write(os.path.join(rec, "raw.log"), raw)
    stamped = ["%d.%03d %s\n" % (1764064800 + n, n % 1000, ln.rstrip("\r"))
               for n, ln in enumerate(raw.split("\n")[:-1], start=1)]
    write(os.path.join(rec, "console.log"), "".join(stamped))
    write(os.path.join(rec, "console-verdict.txt"),
          "cycle 1: boots left 1, image, reset\ncycle 2: boots left 0, image, reset\n"
          "cycle 3: Linux\nverdict: ok (3 cycle(s))\n")
    rc, out = tool(CHECK, os.path.join(rec, "console.log"), "--count", str(N))
    write(os.path.join(rec, "record-verdict.txt"), out)
    if rc != 0:
        t.bad("(c) record_check on the stamped console.log did not pass", out)
    env = "bootcmd=run bootcmd_stm32mp\nboot_prefixes=/mmc0_\nscriptaddr=0x88000000\n"
    write(os.path.join(rec, "env-before.txt"), env)
    write(os.path.join(rec, "env-after.txt"), env)
    boot = "".join("%s  %s\n" % (d * 64, p) for d, p in
                   (("1", "./boot.scr.uimg"), ("2", "./extlinux/extlinux.conf"), ("3", "./Image.gz")))
    write(os.path.join(rec, "boot-before.txt"), boot)
    write(os.path.join(rec, "boot-after.txt"), boot)
    steps = ("precheck", "capture", "console", "runner", "comeback", "record", "undo", "env", "manifest")
    write(os.path.join(rec, "session.txt"),
          "".join("mp2s-session: step %s: ok\n" % s for s in steps) + "mp2s-session: PASS\n")

    rc, rep = tool(REPORT, rec)
    t.check(rc == 0, "(c) mp2s_report.py exits 0 on the record",
            "(c) mp2s_report.py: exit %d, want 0" % rc, rep)
    for k in (1, 0):
        t.check(re.search(r"mp2s: default /yakogl/standalone/default\.bin, boots left %d" % k, rep),
                "(c) the report quotes 'boots left %d'" % k,
                "(c) the report does not quote 'boots left %d'" % k)
    t.check("login:" in rep and "VERDICT: PASS" in rep,
            "(c) the report quotes the login prompt and record_check's VERDICT: PASS",
            "(c) the report lacks the login prompt or the VERDICT: PASS line")
    t.check("stock boot files byte-identical: yes" in rep,
            "(c) the report: stock boot files byte-identical: yes",
            "(c) the report does not say the stock boot files are byte-identical")
    for f in ("raw.log", "console.log"):
        p = os.path.join(rec, f)
        kept = [ln for ln in read(p).splitlines(keepends=True) if "boots left 0" not in ln]
        write(p, "".join(kept))
    rc, rep = tool(REPORT, rec)
    missing = [ln for ln in rep.splitlines() if "missing" in ln.lower()]
    t.check(rc == 3 and re.search(r"missing: .*boots left 0", rep),
            "(c) without 'boots left 0' in the logs the report exits 3 and names it missing",
            "(c) without 'boots left 0': exit %d, want 3 and a missing line" % rc, "\n".join(missing))


def main():
    t = Tally()
    for f in (SCRIPT, SIM, CHECK, REPORT):
        if not os.path.isfile(f):
            t.bad("missing " + os.path.relpath(f, ROOT))
            print("%d checks, %d failed" % (t.checks, t.failed))
            return 1
    # the report redacts these when set; the test's text holds none of them
    for v in ("BOARD_HOST", "BOARD_UART", "BOARD_SERIAL"):
        os.environ.pop(v, None)
    tmp = tempfile.mkdtemp(prefix="mp2s_pipeline.")
    try:
        console = part_a(t, tmp)
        part_b(t, tmp)
        part_c(t, tmp, console)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print("%d checks, %d failed" % (t.checks, t.failed))
    return 1 if t.failed else 0


if __name__ == "__main__":
    sys.exit(main())
