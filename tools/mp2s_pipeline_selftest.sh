#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_pipeline_selftest.sh - the DK standalone c6 verdict chain end to end on
# the host (plan #362): tools/mp2s_script_sim.py's boots of
# driver/tools/mp2-standalone/mp2-standalone.cmd, rendered as a console
# record, through tools/mp2s_record_check.py, and a record directory in
# tools/mp2s_session.sh's layout through tools/mp2s_report.py.
#
#   tools/mp2s_pipeline_selftest.sh
#
# (a) script_sim's `run --left 2` transcript (no power cut), with the console
#     lines the simulator does not print put in where its "sim:" lines say
#     they happen (TF-A's "Reset reason:" and "BL2: v" lines at each boot,
#     the image's lines at "the image runs", the kernel and a login prompt at
#     "booting Linux"), is a console.log: record_check --count 2 must say
#     VERDICT: PASS, its boots-left lines must be the simulator's, and
#     --count 1 / --count 3 on the same log must say FAIL.
# (b) the same for a power cut at each saveenv (before the write, torn, after
#     the write), each transcript taken from script_sim's own Sim class (the
#     tool is imported, not changed). The cut boot ends where the simulator
#     says so. Two renderings: the boot after the cut with BL2's
#     "Power-on reset" (what a cut shows) must be a FAIL naming the supply;
#     with a warm reset (a cut BL2 would not see) record_check must never say
#     PASS where the simulator says the count went wrong (image runs not N,
#     or Linux not at boot N+1 without the cut boot), and where both say the
#     boot is safe they agree on the image boots. script_sim's own `faults`
#     verdict per case must match the safe/unsafe this file computes.
# (c) a record directory as tools/mp2s_session.sh writes it (raw.log,
#     console.log with mp2s_console_watch.py's "<unix time> " stamps,
#     console-verdict.txt, record-verdict.txt from record_check on that log,
#     env-before/after.txt, boot-before/after.txt, session.txt) under
#     <stamp>-standalone-<sha8>: mp2s_report.py must exit 0 and quote the
#     "boots left 1" and "boots left 0" lines and the byte-identical stock
#     boot files; with "boots left 0" gone from the logs it must exit 3.
#
# No board, no network, nothing written outside $TMPDIR (python runs -I -B:
# no __pycache__ next to the tools). Prints "ok:"/"FAIL:" per check and
# "N checks, M failed" last; exit 1 if any failed.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
SCRIPT="$ROOT/driver/tools/mp2-standalone/mp2-standalone.cmd"
SIM="$HERE/mp2s_script_sim.py"
CHECK="$HERE/mp2s_record_check.py"
REPORT="$HERE/mp2s_report.py"
PY=(python3 -I -B)
N=2

tmp=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_pipeline.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
# the report redacts these when set; the test's text holds none of them
unset BOARD_HOST BOARD_UART BOARD_SERIAL

checks=0
failed=0
ok() { checks=$((checks + 1)); echo "ok: $1"; }
bad() { checks=$((checks + 1)); failed=$((failed + 1)); echo "FAIL: $1"; }

for f in "$SCRIPT" "$SIM" "$CHECK" "$REPORT"; do
    if [ ! -f "$f" ]; then
        bad "missing ${f#"$ROOT"/}"
        echo "$checks checks, $failed failed"
        exit 1
    fi
done

# The helper: renders a simulator transcript as a console log, and runs the
# power-cut cases through script_sim's Sim class.
cat > "$tmp/pipe.py" <<'PYEOF'
import argparse
import importlib.util
import os
import re
import sys

WARM = "System reset (SYSRST)"
POWER_ON = "Power-on reset (por_rstn)"
BOOT_RX = re.compile(r"^sim: boot (\d+): (env copy|both env copies bad)")
IMAGE_RX = re.compile(r"^sim: boot \d+: the image runs")
LINUX_RX = re.compile(r"^sim: boot \d+: prefix \S+: ST's extlinux, booting Linux")
CUT_RX = re.compile(r"^sim: boot (\d+): saveenv \d+: power cut")


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
                    "stm32mp2-e3-c0-a2 login: "]
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


def cmd_render(a):
    with open(a.transcript, encoding="utf-8") as f:
        lines = f.read().splitlines()
    with open(a.out, "w", encoding="utf-8", newline="") as f:
        f.write(render(lines))
    return 0


def cmd_faults(a):
    mod = load_sim(a.sim)
    n = a.left
    args = sim_args(a.script, n)
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
                with open(os.path.join(a.outdir, "%s-%s.log" % (name, variant)), "w",
                          encoding="utf-8", newline="") as f:
                    f.write(render(sim.lines, reason))
            with open(os.path.join(a.outdir, name + ".sim.txt"), "w", encoding="utf-8") as f:
                f.write("\n".join(sim.lines) + "\n")
            rows.append("%s\t%d\t%s\t%d\t%s\t%s\t%s" % (
                name, i, kind, sim.image_runs, linux if linux is not None else "-",
                int(safe), int(count_ok)))
    with open(os.path.join(a.outdir, "cases.tsv"), "w", encoding="utf-8") as f:
        f.write("\n".join(rows) + "\n")
    print("saves %d, cases %d" % (clean.saves, len(rows)))
    return 0


def main(argv):
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("render")
    r.add_argument("transcript")
    r.add_argument("out")
    f = sub.add_parser("faults")
    f.add_argument("sim")
    f.add_argument("script")
    f.add_argument("left", type=int)
    f.add_argument("outdir")
    a = ap.parse_args(argv)
    return cmd_render(a) if a.cmd == "render" else cmd_faults(a)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
PYEOF

# The image boots record_check's table marks "yes", one per line.
image_boots() { awk 'NR > 1 && $0 !~ /^VERDICT/ && / yes  /' "$1" | wc -l | tr -d ' '; }
boots_left_seq() { grep -o 'boots left [0-9]*' "$1" | tr '\n' ' '; }

# ---- (a) the clean default-2 run
echo "== (a) script_sim run --left $N through record_check"
"${PY[@]}" "$SIM" run "$SCRIPT" --left "$N" --check-restored > "$tmp/a.sim.txt" 2>&1
rc=$?
if [ "$rc" -eq 0 ] && grep -q "^sim: result: Linux at boot $((N + 1)), boots $((N + 1)), image runs $N," "$tmp/a.sim.txt"; then
    ok "(a) script_sim: Linux at boot $((N + 1)) after $N image runs, env restored"
else
    bad "(a) script_sim run --left $N: rc $rc"
    sed 's/^/    /' "$tmp/a.sim.txt"
fi
"${PY[@]}" "$tmp/pipe.py" render "$tmp/a.sim.txt" "$tmp/a.console.log" || bad "(a) render failed"
"${PY[@]}" "$CHECK" "$tmp/a.console.log" --count "$N" > "$tmp/a.check.txt" 2>&1
rc=$?
if [ "$rc" -eq 0 ] && grep -q "^VERDICT: PASS $N image boot(s) then Linux" "$tmp/a.check.txt"; then
    ok "(a) record_check --count $N: VERDICT: PASS"
else
    bad "(a) record_check --count $N: rc $rc, want VERDICT: PASS"
    sed 's/^/    /' "$tmp/a.check.txt"
fi
want_seq=$(grep '^mp2s: ' "$tmp/a.sim.txt" > "$tmp/a.mp2s.txt"; boots_left_seq "$tmp/a.mp2s.txt")
got_seq=$(boots_left_seq "$tmp/a.check.txt")
if [ -n "$want_seq" ] && [ "$want_seq" = "$got_seq" ]; then
    ok "(a) record_check's boots left lines are the simulator's: $want_seq"
else
    bad "(a) boots left: simulator '$want_seq', record_check '$got_seq'"
fi
got=$(image_boots "$tmp/a.check.txt")
if [ "$got" = "$N" ]; then
    ok "(a) record_check counts $N image boots, as the simulator's image runs"
else
    bad "(a) record_check counts '$got' image boots, the simulator $N"
fi
for c in $((N - 1)) $((N + 1)); do
    "${PY[@]}" "$CHECK" "$tmp/a.console.log" --count "$c" > "$tmp/a.check$c.txt" 2>&1
    rc=$?
    if [ "$rc" -eq 1 ] && grep -q '^VERDICT: FAIL' "$tmp/a.check$c.txt"; then
        ok "(a) the same log with --count $c: VERDICT: FAIL"
    else
        bad "(a) the same log with --count $c: rc $rc, want VERDICT: FAIL"
        sed 's/^/    /' "$tmp/a.check$c.txt"
    fi
done

# ---- (b) a power cut at each saveenv
echo "== (b) a power cut at each saveenv of --left $N"
mkdir -p "$tmp/b" || exit 1
if "${PY[@]}" "$tmp/pipe.py" faults "$SIM" "$SCRIPT" "$N" "$tmp/b" > "$tmp/b.gen.txt" 2>&1 && [ -s "$tmp/b/cases.tsv" ]; then
    ok "(b) script_sim's Sim ran every cut: $(cat "$tmp/b.gen.txt")"
else
    bad "(b) the power-cut transcripts were not made"
    sed 's/^/    /' "$tmp/b.gen.txt"
fi
# script_sim's own faults verdict per case, to hold this file's reading of it
"${PY[@]}" "$SIM" faults "$SCRIPT" --left "$N" > "$tmp/b.faults.txt" 2>&1
fault_rc=$?
if grep -q "^sim: faults: N=$N, " "$tmp/b.faults.txt"; then
    ok "(b) script_sim faults --left $N ran (exit $fault_rc)"
else
    bad "(b) script_sim faults --left $N: no summary, exit $fault_rc"
    sed 's/^/    /' "$tmp/b.faults.txt"
fi
conservative=""
while IFS=$'\t' read -r name i kind runs linux safe count_ok; do
    [ -n "$name" ] || continue
    line=$(grep "^sim: fault: saveenv $i of [0-9]*, cut $kind: " "$tmp/b.faults.txt")
    case "$line" in *": ok") tool_safe=1 ;; *": FAIL"*) tool_safe=0 ;; *) tool_safe=x ;; esac
    if [ "$tool_safe" = "$safe" ]; then
        ok "(b) $name: script_sim faults and this test read the case alike (safe=$safe, image runs $runs, Linux at boot $linux without the cut)"
    else
        bad "(b) $name: script_sim faults says '${line:-nothing}', this test safe=$safe"
    fi

    # the boot after the cut as BL2 shows a power cut
    "${PY[@]}" "$CHECK" "$tmp/b/$name-poweron.log" --count "$N" > "$tmp/b/$name-poweron.check" 2>&1
    rc=$?
    if [ "$rc" -eq 1 ] && grep -q '^VERDICT: FAIL .*supply suspect' "$tmp/b/$name-poweron.check"; then
        ok "(b) $name, Power-on reset after the cut: VERDICT: FAIL, supply suspect"
    else
        bad "(b) $name, Power-on reset after the cut: rc $rc, want VERDICT: FAIL ... supply suspect"
        sed 's/^/    /' "$tmp/b/$name-poweron.check"
    fi

    # the cut not seen by BL2 (a warm reset): never PASS where the count went wrong
    "${PY[@]}" "$CHECK" "$tmp/b/$name-warm.log" --count "$N" > "$tmp/b/$name-warm.check" 2>&1
    rc=$?
    verdict=$(grep '^VERDICT: ' "$tmp/b/$name-warm.check" | head -n 1)
    case "$verdict" in
    "VERDICT: PASS"*)
        if [ "$count_ok" = 1 ] && [ "$(image_boots "$tmp/b/$name-warm.check")" = "$runs" ]; then
            ok "(b) $name, warm reset: both say the boot is safe and agree on $runs image boots"
        elif [ "$count_ok" = 1 ]; then
            bad "(b) $name, warm reset: both PASS, record_check counts $(image_boots "$tmp/b/$name-warm.check") image boots, the simulator $runs"
        else
            bad "(b) $name, warm reset: record_check PASS where the simulator says the count went wrong (image runs $runs, Linux at boot $linux without the cut, safe=$safe)"
            sed 's/^/    /' "$tmp/b/$name.sim.txt"
        fi
        ;;
    "VERDICT: FAIL"*)
        if [ "$rc" -eq 1 ]; then
            ok "(b) $name, warm reset: record_check FAIL (simulator: image runs $runs, count_ok=$count_ok): ${verdict#VERDICT: FAIL }"
            [ "$count_ok" = 1 ] && conservative="$conservative $name"
        else
            bad "(b) $name, warm reset: VERDICT: FAIL with exit $rc"
        fi
        ;;
    *)
        bad "(b) $name, warm reset: no verdict (rc $rc)"
        sed 's/^/    /' "$tmp/b/$name-warm.check"
        ;;
    esac
done < "$tmp/b/cases.tsv"
if [ -n "$conservative" ]; then
    echo "note: record_check fails, the simulator's count is right (the record shows the cut boot):$conservative"
fi

# ---- (c) a record directory through the report builder
echo "== (c) a record directory in mp2s_session.sh's layout through mp2s_report.py"
rec="$tmp/records/stm32mp257f-dk/20261125-1200-standalone-28c094e9"
mkdir -p "$rec" || exit 1
cp "$tmp/a.console.log" "$rec/raw.log"
awk '{ sub(/\r$/, ""); printf "%d.%03d %s\n", 1764064800 + NR, NR % 1000, $0 }' "$rec/raw.log" > "$rec/console.log"
printf 'cycle 1: boots left 1, image, reset\ncycle 2: boots left 0, image, reset\ncycle 3: Linux\nverdict: ok (3 cycle(s))\n' \
    > "$rec/console-verdict.txt"
"${PY[@]}" "$CHECK" "$rec/console.log" --count "$N" > "$rec/record-verdict.txt" 2>&1 ||
    bad "(c) record_check on the stamped console.log did not pass"
printf 'bootcmd=run bootcmd_stm32mp\nboot_prefixes=/mmc0_\nscriptaddr=0x88000000\n' > "$rec/env-before.txt"
cp "$rec/env-before.txt" "$rec/env-after.txt"
{
    echo "1111111111111111111111111111111111111111111111111111111111111111  ./boot.scr.uimg"
    echo "2222222222222222222222222222222222222222222222222222222222222222  ./extlinux/extlinux.conf"
    echo "3333333333333333333333333333333333333333333333333333333333333333  ./Image.gz"
} > "$rec/boot-before.txt"
cp "$rec/boot-before.txt" "$rec/boot-after.txt"
{
    for s in precheck capture console runner comeback record undo env manifest; do
        echo "mp2s-session: step $s: ok"
    done
    echo "mp2s-session: PASS"
} > "$rec/session.txt"
"${PY[@]}" "$REPORT" "$rec" > "$tmp/c.report.md" 2>&1
rc=$?
if [ "$rc" -eq 0 ]; then
    ok "(c) mp2s_report.py exits 0 on the record"
else
    bad "(c) mp2s_report.py: exit $rc, want 0"
    sed 's/^/    /' "$tmp/c.report.md"
fi
for k in 1 0; do
    if grep -q "mp2s: default /yakogl/standalone/default\.bin, boots left $k" "$tmp/c.report.md"; then
        ok "(c) the report quotes 'boots left $k'"
    else
        bad "(c) the report does not quote 'boots left $k'"
    fi
done
if grep -q "login:" "$tmp/c.report.md" && grep -q "VERDICT: PASS" "$tmp/c.report.md"; then
    ok "(c) the report quotes the login prompt and record_check's VERDICT: PASS"
else
    bad "(c) the report lacks the login prompt or the VERDICT: PASS line"
fi
if grep -q "stock boot files byte-identical: yes" "$tmp/c.report.md"; then
    ok "(c) the report: stock boot files byte-identical: yes"
else
    bad "(c) the report does not say the stock boot files are byte-identical"
fi
for f in raw.log console.log; do
    grep -v 'boots left 0' "$rec/$f" > "$rec/$f.new" && mv "$rec/$f.new" "$rec/$f"
done
"${PY[@]}" "$REPORT" "$rec" > "$tmp/c.report-missing.md" 2>&1
rc=$?
if [ "$rc" -eq 3 ] && grep -q "missing: .*boots left 0" "$tmp/c.report-missing.md"; then
    ok "(c) without 'boots left 0' in the logs the report exits 3 and names it missing"
else
    bad "(c) without 'boots left 0': exit $rc, want 3 and a missing line"
    grep -i missing "$tmp/c.report-missing.md" | sed 's/^/    /'
fi

echo "$checks checks, $failed failed"
[ "$failed" -eq 0 ]
