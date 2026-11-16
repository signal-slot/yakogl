#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# tools/mp2s_session_selftest.sh - tools/mp2s_session.sh end to end without a
# board: ssh, the runner and the console logger are stubs in $TMPDIR. The
# driver and the python tools it calls are copied into a scratch tools/ so
# that "a helper is not merged yet" is true whatever the real tree holds.
# Covers PASS, FAIL record, FAIL env, FAIL console, FAIL runner, FAIL precheck,
# --dry-run (touches nothing), the skip line and a present helper.
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

TOOLS=$T/tools
mkdir -p "$TOOLS" "$T/state"
cp "$HERE"/*.py "$TOOLS"/
cp "$HERE/mp2s_session.sh" "$TOOLS/"
chmod 755 "$TOOLS/mp2s_session.sh"
for h in mp2s_boot_manifest.py mp2s_comeback.sh fwu_metadata.py mp2s_bootfs_check.py mp2s_fwenv_check.py; do
    rm -f "$TOOLS/$h"
done
SESSION=$TOOLS/mp2s_session.sh

# A valid arm64 Image header: image_size at byte 16, magic at byte 56.
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

# fake ssh: canned fw_printenv, /boot listing, reboot; arguments: host, command
cat > "$T/ssh" <<'SH'
#!/bin/bash
cmd=$2
echo "ssh $cmd" >> "$STATE/calls"
case "$cmd" in
*fw_printenv*) cat "$STATE/env" ;;
*"find /boot"*) cat "$STATE/manifest" ;;
*reboot*) : ;;
*) exit 1 ;;
esac
SH
# fake runner: default sets mp2s variables, nodefault restores, uninstall no-op
cat > "$T/runner" <<'SH'
#!/bin/bash
echo "runner $*" >> "$STATE/calls"
case "$1" in
default)
    [ "${FAKE_RUNNER_FAIL:-0}" = 1 ] && exit 3
    printf 'mp2s_ver=1\nmp2s_left=%s\nboot_prefixes=/yakogl/standalone/ /\n' "$3" >> "$STATE/env" ;;
nodefault)
    [ "${FAKE_NO_RESTORE:-0}" = 1 ] || cp "$STATE/env.base" "$STATE/env" ;;
uninstall) : ;;
*) exit 2 ;;
esac
SH
# fake logger: watch UART RAW STAMPED ...: writes a canned log to RAW
cat > "$T/watch" <<'SH'
#!/bin/bash
echo "watch $*" >> "$STATE/calls"
cp "$FAKE_LOG" "$3"
exit "${FAKE_WATCH_RC:-0}"
SH
chmod 755 "$T/ssh" "$T/runner" "$T/watch"

python3 -I - "$T" <<'PY'
import sys
t = sys.argv[1]
def bl2(r="System reset (SYSRST)"):
    return f"NOTICE:  BL2: v2.10-stm32mp-r3.1(release):\r\nNOTICE:  Reset reason: {r} (0x4).\r\n"
def left(n):
    return f"mp2s: default /yakogl/standalone/default.bin, boots left {n}\r\n"
load = ("mp2s: loaded 1234 bytes at 0xa0000000, RAM footprint 0x2000 of the 0x10000000-byte window\r\n"
        "mp2s: go 0xa0000000\r\n")
image = ("mp2_hello: stm32mp257f_dk, console usart2 at 115200 baud\r\n"
         "mp2: mp2_hello done: PSCI SYSTEM_RESET at 20000 ms\r\n")
linux = ("mp2s: default used up, booting Linux\r\nSCRIPT FAILED: continuing...\r\n"
         "Starting Kernel\r\nstm32mp2 login: \r\n")
good = bl2() + left(1) + load + image + bl2() + left(0) + load + image + bl2() + linux
bad = bl2() + left(1) + load + image + bl2() + load + image + bl2() + linux
open(t + "/good.log", "w").write(good)
open(t + "/bad.log", "w").write(bad)
PY

export STATE=$T/state MP2S_SSH=$T/ssh MP2S_RUNNER=$T/runner MP2S_WATCH=$T/watch
export BOARD_HOST=stub@board BOARD_UART=stub:0 MP2S_TIMEOUT=5

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

scenario good.log
rc=$?
check "good run: exit 0" "$rc"
check "good run: final line PASS" "$(t last_is 'mp2s-session: PASS')"
check "good run: record directory pattern" "$(ls "$T/rec" | grep -Eq '^[0-9]{8}-[0-9]{4}-standalone-[0-9a-f]{8}$'; echo $?)"
check "good run: watch got the UART and --boots 2" "$(t called 'watch stub:0 .* --boots 2 ')"
check "good run: runner called default IMAGE 2" "$(t called "runner default $T/image 2")"
check "good run: a skip line per missing helper" "$([ "$(grep -c '^mp2s-session: skip .*: not present$' "$T/out")" = 5 ]; echo $?)"
check "good run: skip line text exact" "$(grep -qx 'mp2s-session: skip mp2s_comeback.sh: not present' "$T/out"; echo $?)"

scenario bad.log
rc=$?
check "missing 'boots left 0': exit 1" "$([ "$rc" = 1 ]; echo $?)"
check "missing 'boots left 0': FAIL record" "$(t last_is 'mp2s-session: FAIL record')"
check "FAIL record still ran nodefault and uninstall" "$(called 'runner nodefault' && called 'runner uninstall'; echo $?)"

scenario good.log FAKE_NO_RESTORE=1
rc=$?
check "env not restored: exit 1" "$([ "$rc" = 1 ]; echo $?)"
check "env not restored: FAIL env" "$(t last_is 'mp2s-session: FAIL env')"

scenario good.log FAKE_WATCH_RC=4
rc=$?
check "logger timeout: exit 1" "$([ "$rc" = 1 ]; echo $?)"
check "logger timeout: FAIL console" "$(t last_is 'mp2s-session: FAIL console')"
check "logger timeout: the undo ran" "$(t called 'runner uninstall')"

scenario good.log FAKE_RUNNER_FAIL=1
check "runner failure: FAIL runner" "$(t last_is 'mp2s-session: FAIL runner')"

# a present helper runs; a failing one stops the precheck
printf '#!/bin/bash\necho helper-ran "$@" >> "$STATE/calls"\n' > "$TOOLS/fwu_metadata.py"
chmod 755 "$TOOLS/fwu_metadata.py"
scenario good.log
rc=$?
check "present helper: ran, no skip line for it" "$(called helper-ran && ! grep -q 'skip fwu_metadata.py' "$T/out"; echo $?)"
check "present helper: still PASS" "$([ "$rc" = 0 ]; echo $?)"
printf '#!/bin/bash\nexit 1\n' > "$TOOLS/fwu_metadata.py"
scenario good.log
check "failing helper: FAIL precheck" "$(t last_is 'mp2s-session: FAIL precheck')"
check "failing helper: runner never called" "$(! called '^runner'; echo $?)"
rm -f "$TOOLS/fwu_metadata.py"

# a refusing guard (a default already set) stops before anything changes
printf 'mp2s_ver=1\nboot_prefixes=/yakogl/standalone/ /\n' > "$T/state/env.base"
scenario good.log
check "guard refusal: FAIL precheck" "$(t last_is 'mp2s-session: FAIL precheck')"
check "guard refusal: runner never called" "$(! called '^runner'; echo $?)"
printf '%s\n' "$BASE_ENV" > "$T/state/env.base"

# --dry-run: prints every command, touches nothing
cp "$T/state/env.base" "$T/state/env"
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

bash -n "$HERE/mp2s_session.sh"
check "bash -n mp2s_session.sh" $?
bash -n "$HERE/mp2s_session_selftest.sh"
check "bash -n mp2s_session_selftest.sh" $?
if command -v shellcheck > /dev/null 2>&1; then
    shellcheck -x -S warning "$HERE/mp2s_session.sh" "$HERE/mp2s_session_selftest.sh" > /dev/null 2>&1
    check "shellcheck" $?
else
    echo "skip shellcheck: not installed"
fi

echo "mp2s_session_selftest: $N_OK ok, $N_BAD failed"
[ "$N_BAD" = 0 ]
