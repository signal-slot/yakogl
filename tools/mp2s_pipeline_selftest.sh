#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_pipeline_selftest.sh - the c6 verdict chain on synthetic boots, no
# board (plan #362): tools/mp2s_script_sim.py (the product script's text over
# N boots) -> tools/mp2s_record_check.py (verdict over a console log) ->
# tools/mp2s_report.py (the c6 quotes from a record directory).
#
#   tools/mp2s_pipeline_selftest.sh
#
#   (a) the simulator's `default 2` transcript, no power cut, wrapped in
#       synthetic BL2 banner, 'Reset reason:', image and Linux lines (which
#       the simulator does not print), is a console.log; record_check must
#       say VERDICT: PASS;
#   (b) the same with a power cut (before / torn / after) at every saveenv,
#       checked against the simulator's own verdict (Linux within N+1 boots,
#       image runs <= N): record_check must never PASS a run the simulator
#       calls wrong. Each cut is made twice, with a 'System reset' and with
#       a 'Power-on reset' after the cut; the latter is a supply suspect and
#       must never PASS. A wrong-count record (default 3 checked as
#       --count 2) must not PASS either. Each case prints both verdicts;
#   (c) a fake record directory as tools/mp2s_session.sh writes it
#       (console.log, raw.log, record-verdict.txt, boot-before.txt,
#       boot-after.txt): mp2s_report.py exits 0 and quotes both boots-left
#       lines.
#
# Everything runs under $TMPDIR. Prints "N checks, M failed"; exit 1 if any.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/.." && pwd)
T=$(mktemp -d "${TMPDIR:-${here}}/mp2s-pipeline-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
SCRIPT=$root/driver/tools/mp2-standalone/mp2-standalone.cmd
n=0
failed=0
ok() { n=$((n + 1)); echo "ok $n - $1"; }
bad() { n=$((n + 1)); failed=$((failed + 1)); echo "not ok $n - $1"; }

[ -f "$SCRIPT" ] || { echo "no $SCRIPT" >&2; exit 2; }

# The generator: simulator transcripts -> console logs, and the (b) matrix.
cat >"$T/gen.py" <<'PY'
import argparse, importlib.util, os, re, subprocess, sys
sys.dont_write_bytecode = True  # the gate runs with a read-only tree

here, script, out = sys.argv[1], sys.argv[2], sys.argv[3]
spec = importlib.util.spec_from_file_location("sim", os.path.join(here, "mp2s_script_sim.py"))
sim = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sim)


def run(left, cut=None):
    ns = argparse.Namespace(
        mode="run", script=script, left=left, env=None, set=[], unset=[],
        no_script=False, no_image=False, image=None, load_fails=False,
        image_returns=False, saveenv_fails=False, start_flags="5,6",
        max_boots=20, until_restored=False, check_restored=False)
    built = sim.build(ns)
    s = sim.Sim(*built[:4], ns, cut=cut, quiet=True)
    s.run()
    return s


def wrap(lines, poweron_after_cut):
    """Console text from the simulator's lines. Boot K starts at its first
    'sim: boot K:' line; BL2's banner and reset reason come before it."""
    o, cur, prev_cut = [], 0, False
    for ln in lines:
        m = re.match(r"sim: boot (\d+):", ln)
        if m and int(m.group(1)) != cur:
            cur = int(m.group(1))
            if cur == 1:
                r = "Pin reset from NRST"
            elif prev_cut and poweron_after_cut:
                r = "Power-on reset (por_rstn)"
            else:
                r = "System reset (SYSRST)"
            o.append("NOTICE:  BL2: v2.10-stm32mp-r3.1(release):")
            o.append("NOTICE:  Reset reason: %s (0x4)." % r)
            prev_cut = False
        if "power cut" in ln:
            prev_cut = True
        if ln.startswith("sim: "):
            if "the image runs" in ln:
                o.append("mp2_hello: stm32mp257f_dk, console usart2 at 115200 baud")
                o.append("mp2: mp2_hello done: PSCI SYSTEM_RESET at 20000 ms")
            elif "booting Linux" in ln:
                o.append("Starting Kernel ...")
                o.append("stm32mp2 login: ")
            continue
        o.append(ln)
    return "\n".join(o) + "\n"


def verdict(path, count):
    p = subprocess.run([sys.executable, "-I", os.path.join(here, "mp2s_record_check.py"),
                        path, "--count", str(count)], capture_output=True, text=True)
    first = next((l for l in p.stdout.splitlines() if l.startswith("VERDICT:")), "no VERDICT line")
    return first.startswith("VERDICT: PASS"), first


def sim_safe(s, n):
    if not s.linux_boots:
        return False
    boots = s.linux_boots[0] - (1 if s.cut_boot else 0)
    return boots <= n + 1 and s.image_runs <= n


N = 2
clean = run(str(N))
with open(os.path.join(out, "a.log"), "w") as f:
    f.write(wrap(clean.lines, False))
res = []
for i in range(1, clean.saves + 1):
    for kind in ("before", "torn", "after"):
        s = run(str(N), (i, kind))
        for por in (False, True):
            name = "cut-%d-%s-%s" % (i, kind, "poweron" if por else "sysrst")
            p = os.path.join(out, name + ".log")
            with open(p, "w") as f:
                f.write(wrap(s.lines, por))
            okv, first = verdict(p, N)
            res.append((name, sim_safe(s, N), okv, first))
w = run("3")
p = os.path.join(out, "wrong.log")
with open(p, "w") as f:
    f.write(wrap(w.lines, False))
okv, first = verdict(p, N)
res.append(("wrong-count-3-as-2", False, okv, first))
with open(os.path.join(out, "matrix.txt"), "w") as f:
    for name, safe, okv, first in res:
        f.write("%s\t%d\t%d\t%s\n" % (name, safe, okv, first))
PY

python3 -I "$T/gen.py" "$here" "$SCRIPT" "$T" || { bad "generator failed"; echo "$n checks, $failed failed"; exit 1; }

# (a)
rc=0
python3 -I "$here/mp2s_record_check.py" "$T/a.log" --count 2 >"$T/a.out" 2>&1 || rc=$?
if [ "$rc" = 0 ] && grep -q '^VERDICT: PASS' "$T/a.out"; then ok "(a) default 2, no cut: VERDICT: PASS"; else bad "(a) default 2, no cut: rc=$rc: $(head -3 "$T/a.out")"; fi

# (b)
cases=0
while IFS=$'\t' read -r name safe pass first; do
	cases=$((cases + 1))
	if [ "$safe" = 0 ] && [ "$pass" = 1 ]; then
		bad "(b) $name: simulator says the count went wrong, record_check says PASS"
	elif [ "$name" = wrong-count-3-as-2 ] && [ "$pass" = 1 ]; then
		bad "(b) $name: PASS"
	elif [[ $name == *-poweron ]] && [ "$pass" = 1 ]; then
		bad "(b) $name: PASS despite a power-on reset in the middle"
	else
		ok "(b) $name: sim safe=$safe, record_check pass=$pass ($first)"
	fi
done <"$T/matrix.txt"
if [ "$cases" -ge 19 ]; then ok "(b) $cases cases ran"; else bad "(b) only $cases cases"; fi

# (c)
R=$T/records/stm32mp257f-dk/20260101-0000-standalone-abcd1234
mkdir -p "$R"
awk '{ print 1763200000 + NR, $0 }' "$T/a.log" >"$R/console.log"
sed 's/$/\r/' "$T/a.log" >"$R/raw.log"
python3 -I "$here/mp2s_record_check.py" "$R/console.log" --count 2 >"$R/record-verdict.txt" || true
printf '%s  /boot/Image\n%s  /boot/extlinux/x.conf\n' \
	1111111111111111111111111111111111111111111111111111111111111111 \
	2222222222222222222222222222222222222222222222222222222222222222 >"$R/boot-before.txt"
cp "$R/boot-before.txt" "$R/boot-after.txt"
rc=0
python3 -I "$here/mp2s_report.py" "$R" >"$T/c.out" 2>"$T/c.err" || rc=$?
if [ "$rc" = 0 ]; then ok "(c) report exits 0"; else bad "(c) report exit $rc: $(head -3 "$T/c.err")"; fi
if grep -q 'boots left 1' "$T/c.out"; then ok "(c) quotes 'boots left 1'"; else bad "(c) no 'boots left 1' in the report"; fi
if grep -q 'boots left 0' "$T/c.out"; then ok "(c) quotes 'boots left 0'"; else bad "(c) no 'boots left 0' in the report"; fi
if grep -q 'VERDICT: PASS' "$T/c.out"; then ok "(c) quotes the record_check verdict"; else bad "(c) no VERDICT: PASS in the report"; fi

echo "$n checks, $failed failed"
[ "$failed" = 0 ]
