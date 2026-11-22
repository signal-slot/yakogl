#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# tools/mp2s_session_e2e_selftest.sh - tools/mp2s_session.sh end to end on the
# fake board (tools/mp2s_fakeboard.sh), through the report, without a board.
# The console the watch stub replays is not canned: it is what
# tools/mp2s_script_sim.py prints for the real mp2-standalone.cmd under
# `default <image> 2` (the "mp2s:" lines and "SCRIPT FAILED: continuing..."),
# with a BL2 banner before every boot, the image's lines after each `go`, then
# stock Linux login lines. The runner and the watch are stubs over the fake
# (as in mp2s_session_selftest.sh); the guard, image, bootfs, fwenv, FWU,
# comeback, record, env and manifest checks are the real tools.
# Case 1 (pass): in a temporary records root the record directory has its
# parts, mp2s_record_check.py passes, mp2s_report.py exits 0 and
# mp2s_redact_check.py is clean on the record. Case 2: the same console
# without its 'boots left 0' line must end in `FAIL record`, a failing
# verdict, with the report exiting 3 (missing).
# KEEP=1 keeps the scratch directory.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
[ -n "${TMPDIR:-}" ] || { echo "mp2s_session_e2e_selftest: set TMPDIR" >&2; exit 2; }
T=$(mktemp -d "$TMPDIR/mp2s-e2e.XXXXXX") || exit 1
trap '[ -n "${KEEP:-}" ] || rm -rf "$T"' EXIT
N_OK=0
N_BAD=0
check() { # name, status (0 = pass)
    if [ "$2" = 0 ]; then
        N_OK=$((N_OK + 1)); echo "ok   $1"
    else
        N_BAD=$((N_BAD + 1)); echo "FAIL $1"
        [ -z "${OUT_SHOWN:-}" ] && { sed 's/^/    | /' "$T/out"; OUT_SHOWN=1; }
    fi
}
t() { if "$@"; then echo 0; else echo 1; fi; }

mkdir -p "$T/tools"
cp "$HERE"/*.py "$HERE/mp2s_comeback.sh" "$HERE/mp2s_session.sh" "$T/tools"/
chmod 755 "$T/tools"/*
SESSION=$T/tools/mp2s_session.sh
SCRIPT=$ROOT/driver/tools/mp2-standalone/mp2-standalone.cmd
[ -r "$SCRIPT" ] || { echo "mp2s_session_e2e_selftest: no $SCRIPT" >&2; exit 2; }

# a valid arm64 Image header: image_size at byte 16, magic at byte 56
python3 -I - "$T/image" <<'PY'
import struct, sys
h = bytearray(0x100)
h[16:24] = struct.pack("<Q", 0x2000)
h[56:60] = struct.pack("<I", 0x644d5241)
open(sys.argv[1], "wb").write(h)
PY

# the console of `default IMAGE 2`, from the simulator's own output
python3 -I "$HERE/mp2s_script_sim.py" run "$SCRIPT" --left 2 --until-restored > "$T/sim.txt" 2>&1
check "script_sim: default 2 reaches Linux at boot 3" "$(grep -q '^sim: result: Linux at boot 3' "$T/sim.txt"; echo $?)"
python3 -I - "$T" <<'PY'
import re, sys
t = sys.argv[1]
bl2 = ("NOTICE:  BL2: v2.10-stm32mp-r3.1(release):\n"
       "NOTICE:  Reset reason: System reset (SYSRST) (0x4).\n")
image = ("*** Booting Zephyr OS build v4.0 ***\n"
         "mp2_hello: stm32mp257f_dk, console usart2 at 115200 baud\n"
         "mp2: mp2_hello done: PSCI SYSTEM_RESET at 20000 ms\n")
out = ""
for line in open(t + "/sim.txt").read().splitlines():
    if re.match(r"sim: boot \d+: env copy", line):
        out += bl2
    elif line.startswith("mp2s: ") or line.startswith("SCRIPT FAILED"):
        out += line + "\n"
        if line.startswith("mp2s: go "):
            out += image
    elif "booting Linux (OpenSTLinux)" in line:
        out += "Starting kernel ...\n\nstm32mp2 login: "
crlf = out.replace("\n", "\r\n")
open(t + "/good.log", "w").write(crlf)
open(t + "/noleft0.log", "w").write(
    "".join(l for l in crlf.splitlines(True) if "boots left 0" not in l))
PY

# the stub runner: logs its call; default and nodefault edit the fake's env
cat > "$T/runner" <<'SH'
#!/bin/bash
echo "runner $* person=${BOARD_PERSON_PRESENT-unset} nowd=${MP2_NO_WATCHDOG-unset}" >> "$STATE/calls"
fb() { "$FB/bin/ssh" stub "$@"; }
case "$1" in
install | uninstall) echo "runner: $1 done" ;;
# the real runner reads the card's u-boot-env partition; the fake's env is
# what its fw_printenv shim prints
printenv) fb "sudo -n fw_printenv" ;;
default)
    pfx=$(fb "sudo -n fw_printenv -n boot_prefixes") || exit 1
    printf '%s' "$pfx" > "$STATE/pfx"
    printf 'mp2s_ver=1\nmp2s_left=%s\nmp2s_prefixes=%s\nboot_prefixes=/yakogl/standalone/ %s\n' "$3" "$pfx" "$pfx" |
        fb "sudo -n fw_setenv -s -" || exit 1
    echo "default set for $3 boots; nothing was rebooted." ;;
nodefault)
    if fb "sudo -n fw_printenv -n mp2s_ver" > /dev/null 2>&1; then
        printf 'boot_prefixes=%s\nmp2s_ver=\nmp2s_left=\nmp2s_prefixes=\n' "$(cat "$STATE/pfx")" |
            fb "sudo -n fw_setenv -s -" || exit 1
    fi
    echo "runner: nodefault done" ;;
*) exit 2 ;;
esac
SH
# the fake ssh: the fake board's shim; a reboot is marked once its call is over
cat > "$T/ssh" <<'SH'
#!/bin/bash
echo "${*: -1}" >> "$STATE/ssh-calls"
"$FB/bin/ssh" "$@"
rc=$?
case "${*: -1}" in *"systemctl reboot"*) : > "$STATE/rebooted" ;; esac
exit "$rc"
SH
# the stub watch: watch UART RAW STAMPED --boots N ...; it waits for the fake's
# reboot, does what the last boot of the count does to the environment and the
# boot id, replays the console and ends with the real scan's exit (or 0 with
# FAKE_WATCH=lenient: the session's record check is then the verdict)
cat > "$T/watch" <<'SH'
#!/bin/bash
echo "$*" >> "$STATE/watch-calls"
end=$((SECONDS + 600))
until [ -e "$STATE/rebooted" ]; do
    if ! kill -0 "$PPID" 2>/dev/null || [ "$SECONDS" -ge "$end" ]; then echo "no reboot seen"; exit 1; fi
    sleep 0.1
done
pfx=$(cat "$STATE/pfx")
printf 'boot_prefixes=%s\nmp2s_ver=\nmp2s_left=\nmp2s_prefixes=\n' "$pfx" | "$FB/bin/ssh" stub "sudo -n fw_setenv -s -"
echo 00000000-0000-4000-8000-0000000000b2 > "$FB/proc/sys/kernel/random/boot_id"
cp "$FAKE_LOG" "$3"
tr -d '\r' < "$FAKE_LOG" | sed 's/^/1700000000 /' > "$4"
python3 -I "$REPO_TOOLS/mp2s_console_watch.py" scan "$4" --boots "$6"
rc=$?
[ "$FAKE_WATCH" = lenient ] && exit 0
exit "$rc"
SH
chmod 755 "$T/runner" "$T/ssh" "$T/watch"

export STATE=$T/state REPO_TOOLS=$HERE MP2S_RUNNER=$T/runner MP2S_WATCH=$T/watch
export BOARD_HOST=stubhost BOARD_UART=stubuart MP2S_TIMEOUT=30 MP2S_BACK=120 MP2S_WATCH_SETTLE=1
unset MP2_NO_WATCHDOG MP2_SUDO BOARD_PERSON_PRESENT
BASE_PATH=$PATH

scenario() { # LOG WATCH: a fresh fake board, then the driver; output in $T/out
    rm -rf "$T/fb" "$T/rec" "$STATE"
    mkdir -p "$STATE"
    : > "$STATE/calls"
    "$HERE/mp2s_fakeboard.sh" init "$T/fb" > /dev/null || { echo "fakeboard init failed"; return 99; }
    echo 00000000-0000-4000-8000-0000000000b1 > "$T/fb/proc/sys/kernel/random/boot_id"
    OUT_SHOWN=
    env FB="$T/fb" PATH="$T/fb/bin:$BASE_PATH" MP2S_FAKE_ROOT="$T/fb" MP2S_SSH="$T/ssh" \
        FAKE_LOG="$T/$1" FAKE_WATCH="$2" MP2S_RECORDS="$T/rec" \
        "$SESSION" "$T/image" > "$T/out" 2>&1
}
last_is() { [ "$(tail -n 1 "$T/out")" = "$1" ]; }

# case 1: pass
scenario good.log ok
rc=$?
check "pass: session exit 0, PASS" "$([ "$rc" = 0 ] && last_is 'mp2s-session: PASS'; echo $?)"
RECS=("$T"/rec/*)
REC=${RECS[0]}
check "pass: one record directory" "$([ "${#RECS[@]}" = 1 ] && [ -d "$REC" ]; echo $?)"
check "pass: the record has its parts" "$(for f in env-before.txt env-default.txt env-after.txt boot-before.txt \
    boot-after.txt raw.log console.log console-verdict.txt record-verdict.txt default.txt nodefault.txt session.txt; do
    [ -s "$REC/$f" ] || { echo "missing $f" >&2; exit 1; }; done; echo 0)"
check "pass: console.log has both counts and the image" \
    "$(grep -q 'boots left 1' "$REC/console.log" && grep -q 'boots left 0' "$REC/console.log" && grep -q mp2_hello "$REC/console.log"; echo $?)"
python3 -I "$HERE/mp2s_record_check.py" "$REC/console.log" --count 2 > "$T/out" 2>&1
check "pass: mp2s_record_check passes" $?
python3 -I "$HERE/mp2s_report.py" "$REC" > "$T/report.md" 2> "$T/out"
check "pass: mp2s_report exits 0 (complete)" $?
check "pass: the report quotes boots left 0" "$(t grep -q 'boots left 0' "$T/report.md")"
# the quoted parts: the console logs, the verdicts and the session summary (the
# env dumps hold ST's stock "ipv4:1***" text, which the check flags: see report)
python3 -I "$HERE/mp2s_redact_check.py" "$REC/raw.log" "$REC/console.log" "$REC/console-verdict.txt" \
    "$REC/record-verdict.txt" "$REC/session.txt" > "$T/out" 2>&1
check "pass: mp2s_redact_check is clean on the quoted record files" $?
python3 -I "$HERE/mp2s_redact_check.py" "$T/report.md" > "$T/out" 2>&1
check "pass: mp2s_redact_check is clean on the report" $?

# case 2: 'boots left 0' missing; the watch stub lets it through, so the
# session's own record check must fail it
scenario noleft0.log lenient
rc=$?
check "no 'boots left 0': session exit 1, FAIL record" "$([ "$rc" = 1 ] && last_is 'mp2s-session: FAIL record'; echo $?)"
RECS=("$T"/rec/*)
REC=${RECS[0]}
python3 -I "$HERE/mp2s_record_check.py" "$REC/console.log" --count 2 > "$T/out" 2>&1
rc=$?
check "no 'boots left 0': mp2s_record_check fails" "$([ "$rc" != 0 ]; echo $?)"
check "no 'boots left 0': the record verdict says FAIL" "$(grep -qi 'fail' "$REC/record-verdict.txt"; echo $?)"
python3 -I "$HERE/mp2s_report.py" "$REC" > "$T/report.md" 2> "$T/out"
rc=$?
check "no 'boots left 0': mp2s_report exits 3 (missing)" "$([ "$rc" = 3 ]; echo $?)"

bash -n "$HERE/mp2s_session_e2e_selftest.sh"
check "bash -n" $?
echo "mp2s_session_e2e_selftest: $N_OK ok, $N_BAD failed"
[ "$N_BAD" = 0 ]
