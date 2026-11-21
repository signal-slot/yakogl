#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# tools/mp2s_session_selftest.sh - tools/mp2s_session.sh end to end without a
# board. Everything lives in $TMPDIR: the board is tools/mp2s_fakeboard.sh
# (MP2S_SSH is a stub over its ssh shim, which is also first in PATH for
# tools/mp2s_comeback.sh),
# the runner is a stub (MP2S_RUNNER) that edits the fake's environment as
# default and nodefault would and answers printenv from it (the session's only
# environment read: the real runner reads the card's partition, because the
# DK has no fw_env.config), and the console watch is a stub (MP2S_WATCH)
# that waits for the fake's reboot, does what the counted default's last boot
# does to the environment and the boot id, writes a canned console log and
# answers with the real `mp2s_console_watch.py scan` exit. The guard, image,
# bootfs, FWU, comeback, record, env and manifest checks are the real
# tools. Covers: PASS; FAIL record (no 'boots left 0'); FAIL env (not
# restored); FAIL console (the watch timed out: no Linux); FAIL console before
# the runner (busy console); FAIL precheck; --dry-run touches nothing; the
# skip line of every missing helper; no BOARD_HOST. shellcheck when installed.
# KEEP=1 keeps the scratch directory (each scenario's output under dbg/).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
[ -n "${TMPDIR:-}" ] || { echo "mp2s_session_selftest: set TMPDIR" >&2; exit 2; }
T=$(mktemp -d "$TMPDIR/mp2s-session.XXXXXX") || exit 1
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

# the driver and the real helpers in a scratch tools/ (a helper can go missing)
mk_tools() {
    mkdir -p "$1"
    cp "$HERE"/*.py "$HERE/mp2s_comeback.sh" "$HERE/mp2s_session.sh" "$1"/
    chmod 755 "$1"/*
}
mk_tools "$T/tools"
SESSION=$T/tools/mp2s_session.sh

# a valid arm64 Image header: image_size at byte 16, magic at byte 56
python3 -I - "$T/image" <<'PY'
import struct, sys
h = bytearray(0x100)
h[16:24] = struct.pack("<Q", 0x2000)
h[56:60] = struct.pack("<I", 0x644d5241)
open(sys.argv[1], "wb").write(h)
PY

# canned console logs: good, without 'boots left 0', and cut off (no Linux)
python3 -I - "$T" <<'PY'
import sys
t = sys.argv[1]
def bl2(r="System reset (SYSRST)"):
    return f"NOTICE:  BL2: v2.10-stm32mp-r3.1(release):\nNOTICE:  Reset reason: {r} (0x4).\n"
def left(n):
    return f"mp2s: default /yakogl/standalone/default.bin, boots left {n}\n"
load = ("mp2s: loaded 1234 bytes at 0xa0000000, RAM footprint 0x2000 of the 0x10000000-byte window\n"
        "mp2s: go 0xa0000000\n")
image = ("*** Booting Zephyr OS build v4.0 ***\n"
         "mp2_hello: stm32mp257f_dk, console usart2 at 115200 baud\n"
         "mp2: mp2_hello done: PSCI SYSTEM_RESET at 20000 ms\n")
linux = ("mp2s: default used up, booting Linux\nSCRIPT FAILED: continuing...\n"
         "Starting kernel ...\n\nstm32mp2 login: ")
for name, text in (
        ("good", bl2() + left(1) + load + image + bl2() + left(0) + load + image + bl2() + linux),
        ("noleft0", bl2() + left(1) + load + image + bl2() + load + image + bl2() + linux),
        ("cut", bl2() + left(1) + load + image + bl2() + left(0) + load + image + bl2())):
    open(f"{t}/{name}.log", "w").write(text.replace("\n", "\r\n"))
PY

# the stub runner: logs its call; default and nodefault edit the fake's env
cat > "$T/runner" <<'SH'
#!/bin/bash
echo "runner $* person=${BOARD_PERSON_PRESENT-unset} nowd=${MP2_NO_WATCHDOG-unset}" >> "$STATE/calls"
fb() { "$FB/bin/ssh" stub "$@"; }
case "$1" in
install | uninstall) echo "runner: $1 done" ;;
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
# the stub watch: watch UART RAW STAMPED --boots N ...; it ends with the real
# scan's exit, or 0 with FAKE_WATCH=lenient (a watch that let a bad log pass:
# the record check is the session's own verdict)
cat > "$T/watch" <<'SH'
#!/bin/bash
echo "$*" >> "$STATE/watch-calls"
[ "$FAKE_WATCH" = busy ] && { echo "console busy"; exit 4; }
# until the reboot, as long as the session that started it is there (a loaded
# gate host can take well over 30 s to get there); at most 600 s
end=$((SECONDS + 600))
until [ -e "$STATE/rebooted" ]; do
    if ! kill -0 "$PPID" 2>/dev/null || [ "$SECONDS" -ge "$end" ]; then echo "no reboot seen"; exit 1; fi
    sleep 0.1
done
# the last boot of the count: the used-up branch puts boot_prefixes back
pfx=$(cat "$STATE/pfx")
[ "$FAKE_WATCH" = badrestore ] && pfx="$pfx /extra/"
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
# MP2S_BACK only bounds a fake that does not answer: a loaded gate host can
# take more than a few seconds for one fake ssh round, and none of these
# scenarios has a board that stays away
export BOARD_HOST=stubhost BOARD_UART=stubuart MP2S_TIMEOUT=30 MP2S_BACK=120 MP2S_WATCH_SETTLE=1
export BOARD_PERSON_PRESENT=1   # the driver must drop it
unset MP2_NO_WATCHDOG MP2_SUDO
BASE_PATH=$PATH

# scenario TOOLS LOG WATCH: a fresh fake board, then the driver; output in $T/out
scenario() {
    rm -rf "$T/fb" "$T/rec" "$STATE"
    mkdir -p "$STATE"
    : > "$STATE/calls"
    "$HERE/mp2s_fakeboard.sh" init "$T/fb" > /dev/null || { echo "fakeboard init failed"; return 99; }
    echo 00000000-0000-4000-8000-0000000000b1 > "$T/fb/proc/sys/kernel/random/boot_id"
    OUT_SHOWN=
    env FB="$T/fb" PATH="$T/fb/bin:$BASE_PATH" MP2S_FAKE_ROOT="$T/fb" MP2S_SSH="$T/ssh" \
        FAKE_LOG="$T/$2" FAKE_WATCH="$3" MP2S_RECORDS="$T/rec" \
        "$1/mp2s_session.sh" "$T/image" > "$T/out" 2>&1
    local rc=$?
    [ -z "${KEEP:-}" ] || { mkdir -p "$T/dbg/$2-$3"; cp "$T/out" "$T/fb/writes.log" "$T/dbg/$2-$3/"; }
    return "$rc"
}
last_is() { [ "$(tail -n 1 "$T/out")" = "$1" ]; }
called() { grep -q -- "$1" "$STATE/calls"; }
in_out() { grep -q -- "$1" "$T/out"; }
step_is() { grep -qx "mp2s-session: step $1: $2" "$T/out"; }
no_violation() { ! grep -q VIOLATION "$T/fb/writes.log"; }
count_is() { [ "$(grep -c -- "$1" "$2")" = "$3" ]; }
absent() { local p; for p; do [ ! -e "$p" ] || return 1; done; }
empty() { local p; for p; do [ ! -s "$p" ] || return 1; done; }
rec_named() { local d; for d in "$T"/rec/*; do [[ ${d##*/} =~ ^[0-9]{8}-[0-9]{4}-standalone-[0-9a-f]{8}$ ]] || return 1; done; }

scenario "$T/tools" good.log ok
rc=$?
check "good run: exit 0" "$rc"
check "good run: final line PASS" "$(t last_is 'mp2s-session: PASS')"
check "good run: every step ok" "$(t count_is '^mp2s-session: step [a-z]*: ok$' "$T/out" 9)"
check "good run: record directory <YYYYMMDD-HHMM>-standalone-<sha8>" "$(t rec_named)"
RECS=("$T"/rec/*)
REC=${RECS[0]}
check "good run: env, /boot and console captured" \
    "$(for f in env-before.txt env-after.txt boot-before.txt boot-after.txt raw.log console.log session.txt; do [ -s "$REC/$f" ] || exit 1; done; echo 0)"
check "good run: watch on the UART with --boots 2" "$(grep -q '^watch stubuart .* --boots 2 ' "$STATE/watch-calls"; echo $?)"
check "good run: install, then default IMAGE 2" "$(called '^runner install' && called "^runner default $T/image 2 "; echo $?)"
check "good run: BOARD_PERSON_PRESENT never reaches the runner" "$(! called 'person=1'; echo $?)"
check "good run: MP2_NO_WATCHDOG=1 for mp2_hello" "$(t called '^runner default .*nowd=1')"
check "good run: one reboot, no write outside the fake's rules" "$([ "$(grep -cx reboot "$T/fb/writes.log")" = 1 ] && no_violation; echo $?)"
check "good run: the real bootfs, fwenv, FWU and comeback checks ran" \
    "$(in_out '^ok: ' && in_out '^comeback: ok' && ! in_out 'skip tools/'; echo $?)"
check "good run: no fw_printenv sent to the board by the session" "$(! grep -q fw_printenv "$STATE/ssh-calls"; echo $?)"
check "good run: the environment is read through the runner's printenv" "$(count_is '^runner printenv' "$STATE/calls" 4; echo $?)"
check "good run: nodefault and uninstall once, at the end" \
    "$(count_is '^runner nodefault' "$STATE/calls" 1 && count_is '^runner uninstall' "$STATE/calls" 1; echo $?)"
check "good run: env restored, /boot the same" "$(in_out '^mp2s-env: restored' && in_out '^mp2s-boot: same'; echo $?)"
check "good run: the board host is never printed" "$(! in_out stubhost; echo $?)"

scenario "$T/tools" noleft0.log lenient
check "no 'boots left 0': FAIL record" "$(t last_is 'mp2s-session: FAIL record')"
check "no 'boots left 0': the undo ran once" "$(called '^runner nodefault' && called '^runner uninstall' && step_is undo 'ok (after a failure)'; echo $?)"
check "no 'boots left 0': env and manifest not run" "$(step_is env 'not run' && step_is manifest 'not run'; echo $?)"

scenario "$T/tools" good.log badrestore
check "env not restored: FAIL env" "$(t last_is 'mp2s-session: FAIL env')"
check "env not restored: names boot_prefixes" "$(t in_out '^mp2s-env: NOT restored: .*boot_prefixes')"

scenario "$T/tools" cut.log ok
rc=$?
check "watch timeout (no Linux): exit 1, FAIL console" "$([ "$rc" = 1 ] && last_is 'mp2s-session: FAIL console'; echo $?)"
check "watch timeout: NEEDS A PERSON, supply first, card undo" \
    "$(in_out 'NEEDS A PERSON' && in_out 'USB-C supply' && in_out 'mp2s_card_undo.sh'; echo $?)"
check "watch timeout: the board is left alone (no nodefault)" "$(! called '^runner nodefault'; echo $?)"

# the busy watch must be seen gone within the settle time, also on a loaded
# host: the session sees it end at once, so a long settle costs nothing here
MP2S_WATCH_SETTLE=120 scenario "$T/tools" good.log busy
check "busy console: FAIL console" "$(t last_is 'mp2s-session: FAIL console')"
check "busy console: runner never called but its read-only printenv, no reboot" "$(! grep -v '^runner printenv' "$STATE/calls" &&! grep -q reboot "$T/fb/writes.log"; echo $?)"

# the guard refuses before anything changes: a default left behind in boot_prefixes
rm -rf "$T/fb" "$T/rec" "$STATE" && mkdir -p "$STATE" && "$HERE/mp2s_fakeboard.sh" init "$T/fb" > /dev/null
printf 'boot_prefixes=/yakogl/standalone/ /mmc0_\n' |
    env PATH="$T/fb/bin:$BASE_PATH" MP2S_FAKE_ROOT="$T/fb" "$T/fb/bin/ssh" stub "sudo -n fw_setenv -s -"
: > "$T/fb/writes.log"
env FB="$T/fb" PATH="$T/fb/bin:$BASE_PATH" MP2S_FAKE_ROOT="$T/fb" MP2S_SSH="$T/ssh" FAKE_LOG="$T/good.log" \
    FAKE_WATCH=ok MP2S_RECORDS="$T/rec" "$SESSION" "$T/image" > "$T/out" 2>&1
OUT_SHOWN=
check "guard refusal: FAIL precheck" "$(t last_is 'mp2s-session: FAIL precheck')"
check "guard refusal: no watch, no runner, no board write" \
    "$(absent "$STATE/watch-calls" && empty "$T/fb/writes.log" && ! grep -v '^runner printenv' "$STATE/calls"; echo $?)"

# a missing helper: its skip line, its check not reported, the run still passes
mk_tools "$T/tools-min"
for h in mp2s_boot_manifest.py mp2s_comeback.sh fwu_metadata.py mp2s_bootfs_check.py; do
    rm "$T/tools-min/$h"
done
scenario "$T/tools-min" good.log ok
check "missing helpers: PASS on the merged guards alone" "$(t last_is 'mp2s-session: PASS')"
check "missing helpers: one exact skip line each" \
    "$(for h in mp2s_boot_manifest.py mp2s_comeback.sh fwu_metadata.py mp2s_bootfs_check.py; do
        grep -qx "mp2s-session: skip tools/$h: not present" "$T/out" || exit 1; done; echo 0)"
check "missing helpers: no output of a skipped check" \
    "$(! in_out '^comeback: ' && ! in_out '^mp2s-boot: '; echo $?)"

# --dry-run: prints every command, touches nothing
rm -rf "$T/fb" "$T/rec" "$STATE" && mkdir -p "$STATE" && "$HERE/mp2s_fakeboard.sh" init "$T/fb" > /dev/null
: > "$T/fb/writes.log"
mk_tools "$T/dry/tools"
(cd "$T" && env -u BOARD_HOST -u BOARD_UART -u MP2S_RECORDS FB="$T/fb" PATH="$T/fb/bin:$BASE_PATH" \
    MP2S_FAKE_ROOT="$T/fb" MP2S_SSH="$T/ssh" "$T/dry/tools/mp2s_session.sh" --dry-run "$T/nonexistent") > "$T/out" 2>&1
rc=$?
OUT_SHOWN=
check "dry-run: exit 0" "$rc"
check "dry-run: no record directory" "$(t absent "$T/dry/records" "$T/rec")"
check "dry-run: no ssh, runner or watch call, no board write" \
    "$(absent "$STATE/ssh-calls" "$STATE/calls" "$STATE/watch-calls" "$STATE/rebooted" && empty "$T/fb/writes.log"; echo $?)"
check "dry-run: prints the guard, the watch, default IMAGE 2, the reboot and the undo" \
    "$(in_out 'mp2s_guard.py --for standalone' && in_out "watch \\\$BOARD_UART" && in_out "runner default $T/nonexistent 2" &&
        in_out 'systemctl reboot' && in_out 'runner nodefault' && in_out 'mp2s_env_diff.py'; echo $?)"
check "dry-run: no PASS line" "$(! in_out 'mp2s-session: PASS'; echo $?)"

# the board host and the console come from the environment only
env -u BOARD_HOST "$SESSION" "$T/image" > "$T/out" 2>&1
rc=$?
check "no BOARD_HOST: exit 2" "$(t test "$rc" = 2)"
"$SESSION" > "$T/out" 2>&1
rc=$?
check "no IMAGE: exit 2" "$(t test "$rc" = 2)"

bash -n "$HERE/mp2s_session.sh"
check "bash -n mp2s_session.sh" $?
bash -n "$HERE/mp2s_session_selftest.sh"
check "bash -n mp2s_session_selftest.sh" $?
if command -v shellcheck > /dev/null; then
    shellcheck "$HERE/mp2s_session.sh" "$HERE/mp2s_session_selftest.sh" > "$T/out" 2>&1
    check "shellcheck" $?
else
    echo "skip shellcheck: not installed"
fi

echo "mp2s_session_selftest: $N_OK ok, $N_BAD failed"
[ "$N_BAD" = 0 ]
