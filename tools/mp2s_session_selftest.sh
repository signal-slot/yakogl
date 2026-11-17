#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# tools/mp2s_session_selftest.sh - tools/mp2s_session.sh end to end without a
# board: ssh, the runner, the console watch and mp2s_comeback.sh are stubs in
# $TMPDIR; the guard, the capacity check, the image check, the console scan,
# the record check and the env diff are the real tools, run on canned data.
# Covers PASS, each FAIL step, the undo after a failure, no undo for a board
# that did not come back (NEEDS A PERSON), --dry-run and the BOARD_HOST rule.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/mp2s-session.XXXXXX") || exit 1
trap 'rm -rf "$T"' EXIT
N_OK=0
N_BAD=0
check() { # name, status (0 = pass)
    if [ "$2" = 0 ]; then
        N_OK=$((N_OK + 1)); echo "ok   $1"
    else
        N_BAD=$((N_BAD + 1)); echo "FAIL $1"
    fi
}

# the driver with the real python helpers, in a scratch tools/
TOOLS=$T/tools
mkdir -p "$TOOLS" "$T/state"
cp "$HERE"/*.py "$TOOLS"/
cp "$HERE/mp2s_session.sh" "$TOOLS/"
chmod 755 "$TOOLS/mp2s_session.sh"
SESSION=$TOOLS/mp2s_session.sh

# a valid arm64 Image header: image_size at byte 16, magic at byte 56
python3 -I - "$T/image" <<'PY'
import struct, sys
h = bytearray(0x100)
h[16:24] = struct.pack("<Q", 0x2000)
h[56:60] = struct.pack("<I", 0x644d5241)
open(sys.argv[1], "wb").write(h)
PY

BASE_ENV='boot_prefixes=/
bootcmd=run bootcmd_stm32mp
bootcount=1
bootdelay=1'
printf '%s\n' "$BASE_ENV" > "$T/state/env.base"
cp "$T/state/env.base" "$T/state/env"
printf 'aaaa  /boot/Image\nbbbb  /boot/extlinux/extlinux.conf\n' > "$T/state/manifest"

# fake ssh: canned fw_printenv, df, /boot listing, reboot; arguments: ... host, command
cat > "$T/ssh" <<'SH'
#!/bin/bash
cmd=${*: -1}
echo "ssh $cmd" >> "$STATE/calls"
case "$cmd" in
*fw_printenv*) cat "$STATE/env" ;;
*"df -B1"*) echo "  35000000" ;;
*"find /boot"*) cat "$STATE/manifest" ;;
*reboot*) : ;;
*) exit 1 ;;
esac
SH
# fake runner: default sets mp2s variables, nodefault restores
cat > "$T/runner" <<'SH'
#!/bin/bash
echo "runner $* person=${BOARD_PERSON_PRESENT-unset}" >> "$STATE/calls"
case "$1" in
install) : ;;
default)
    [ "${FAKE_RUNNER_FAIL:-0}" = 1 ] && exit 3
    printf 'mp2s_ver=1\nmp2s_left=%s\nboot_prefixes=/yakogl/standalone/ /\n' "$3" >> "$STATE/env" ;;
nodefault)
    [ "${FAKE_NO_RESTORE:-0}" = 1 ] || cp "$STATE/env.base" "$STATE/env" ;;
uninstall) : ;;
*) exit 2 ;;
esac
SH
# fake watch: watch UART RAW STAMPED ...: writes a canned log to RAW and STAMPED
cat > "$T/watch" <<'SH'
#!/bin/bash
echo "watch $*" >> "$STATE/calls"
cp "$FAKE_LOG" "$3"
sed 's/^/1700000000 /' "$FAKE_LOG" > "$4"
exit "${FAKE_WATCH_RC:-0}"
SH
# fake comeback
cat > "$TOOLS/mp2s_comeback.sh" <<'SH'
#!/bin/bash
echo "comeback $*" >> "$STATE/calls"
[ "${FAKE_COMEBACK_FAIL:-0}" = 1 ] && { echo "comeback: NEEDS A PERSON"; exit 4; }
echo "comeback: ok"
SH
chmod 755 "$T/ssh" "$T/runner" "$T/watch" "$TOOLS/mp2s_comeback.sh"

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
         "Starting kernel ...\nstm32mp2 login: \n")
good = bl2() + left(1) + load + image + bl2() + left(0) + load + image + bl2() + linux
bad = bl2() + left(1) + load + image + bl2() + load + image + bl2() + linux
open(t + "/good.log", "w").write(good)
open(t + "/bad.log", "w").write(bad)
PY

export STATE=$T/state MP2S_SSH=$T/ssh MP2S_RUNNER=$T/runner MP2S_WATCH=$T/watch
export BOARD_HOST=stub@board BOARD_UART=stub:0 MP2S_TIMEOUT=5 MP2S_BACK=1
export BOARD_PERSON_PRESENT=1   # the driver must drop it

# scenario LOG [VAR=VAL...]: runs the driver, leaves its output in $T/out
scenario() {
    local log=$1
    shift
    cp "$T/state/env.base" "$T/state/env"
    : > "$T/state/calls"
    rm -rf "$T/rec"
    env FAKE_LOG="$T/$log" MP2S_RECORDS="$T/rec" "$@" "$SESSION" "$T/image" > "$T/out" 2>&1
}
last() { tail -n 1 "$T/out"; }
# t CMD...: status of a test, for check
t() { if "$@"; then echo 0; else echo 1; fi; }
last_is() { [ "$(last)" = "$1" ]; }
called() { grep -q -- "$1" "$T/state/calls"; }
in_out() { grep -q -- "$1" "$T/out"; }

scenario good.log
rc=$?
[ -z "${SHOW:-}" ] || cat "$T/out"
check "good run: exit 0" "$rc"
check "good run: final line PASS" "$(t last_is 'mp2s-session: PASS')"
check "good run: record directory pattern" "$(ls "$T/rec" | grep -Eq '^[0-9]{8}-[0-9]{4}-standalone-[0-9a-f]{8}$'; echo $?)"
check "good run: watch got the UART and --boots 2" "$(t called 'watch stub:0 .* --boots 2 ')"
check "good run: install, then default IMAGE 2" "$(called 'runner install' && called "runner default $T/image 2"; echo $?)"
check "good run: BOARD_PERSON_PRESENT never reaches the runner" "$(! called 'person=1'; echo $?)"
check "good run: reboot sent" "$(t called 'systemctl reboot')"
check "good run: comeback waited" "$(t called 'comeback --wait 1')"
check "good run: nodefault and uninstall once, at the end" "$([ "$(grep -c '^runner nodefault' "$T/state/calls")" = 1 ] && [ "$(grep -c '^runner uninstall' "$T/state/calls")" = 1 ]; echo $?)"
check "good run: every step ok" "$([ "$(grep -c '^mp2s-session: step .*: ok$' "$T/out")" = 7 ]; echo $?)"

# the precheck refuses before anything changes
printf 'mp2s_ver=1\nboot_prefixes=/yakogl/standalone/ /\n' > "$T/state/env.base"
scenario good.log
check "guard refusal: FAIL precheck" "$(t last_is 'mp2s-session: FAIL precheck')"
check "guard refusal: runner never called, no undo" "$(! called '^runner'; echo $?)"
printf '%s\n' "$BASE_ENV" > "$T/state/env.base"

python3 -I -c 'import sys; open(sys.argv[1], "wb").write(b"x" * 64)' "$T/short"
env FAKE_LOG="$T/good.log" MP2S_RECORDS="$T/rec" "$SESSION" "$T/short" > "$T/out" 2>&1
check "bad image: FAIL precheck" "$(t last_is 'mp2s-session: FAIL precheck')"

scenario good.log FAKE_RUNNER_FAIL=1
rc=$?
check "runner failure: exit 1" "$([ "$rc" = 1 ]; echo $?)"
check "runner failure: FAIL runner" "$(t last_is 'mp2s-session: FAIL runner')"
check "runner failure: the undo ran (board still in Linux)" "$(called 'runner nodefault' && called 'runner uninstall'; echo $?)"
check "runner failure: no reboot" "$(! called 'systemctl reboot'; echo $?)"

# a board that did not come back is left alone
scenario good.log FAKE_WATCH_RC=1
check "watch verdict: FAIL back" "$(t last_is 'mp2s-session: FAIL back')"
check "watch verdict: NEEDS A PERSON" "$(t in_out 'NEEDS A PERSON')"
check "watch verdict: no nodefault, no uninstall" "$(! called 'runner nodefault' && ! called 'runner uninstall'; echo $?)"
scenario good.log FAKE_COMEBACK_FAIL=1
check "no comeback: FAIL back" "$(t last_is 'mp2s-session: FAIL back')"
check "no comeback: no nodefault, no uninstall" "$(! called 'runner nodefault' && ! called 'runner uninstall'; echo $?)"
check "no comeback: names the card undo and the supply" "$(in_out 'mp2s_card_undo.sh' && in_out 'USB-C supply'; echo $?)"

# the board is back, the record is wrong: the undo runs
scenario bad.log
check "bad record: FAIL record" "$(t last_is 'mp2s-session: FAIL record')"
check "bad record: the undo ran" "$(called 'runner nodefault' && called 'runner uninstall'; echo $?)"

scenario good.log FAKE_NO_RESTORE=1
check "env not restored: FAIL undo" "$(t last_is 'mp2s-session: FAIL undo')"

# --dry-run: prints every command, touches nothing
: > "$T/state/calls"
rm -rf "$T/rec"
env -u BOARD_HOST -u BOARD_UART MP2S_RECORDS="$T/rec" "$SESSION" --dry-run "$T/nonexistent" > "$T/out" 2>&1
rc=$?
check "dry-run: exit 0" "$rc"
check "dry-run: no record directory" "$([ ! -e "$T/rec" ]; echo $?)"
check "dry-run: no ssh, runner or watch call" "$([ ! -s "$T/state/calls" ]; echo $?)"
check "dry-run: prints the runner's default" "$(grep -q 'default .* 2' "$T/out"; echo $?)"
check "dry-run: no PASS line" "$(! grep -q 'mp2s-session: PASS' "$T/out"; echo $?)"

# the board host comes from the environment only
env -u BOARD_HOST "$SESSION" "$T/image" > /dev/null 2>&1
rc=$?
check "no BOARD_HOST: exit 2" "$([ "$rc" = 2 ]; echo $?)"
"$SESSION" > /dev/null 2>&1
rc=$?
check "no IMAGE: exit 2" "$([ "$rc" = 2 ]; echo $?)"

bash -n "$HERE/mp2s_session.sh"
check "bash -n mp2s_session.sh" $?
bash -n "$HERE/mp2s_session_selftest.sh"
check "bash -n mp2s_session_selftest.sh" $?

echo "mp2s_session_selftest: $N_OK ok, $N_BAD failed"
[ "$N_BAD" = 0 ]
