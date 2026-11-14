#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_console_watch_selftest.sh - tools/mp2s_console_watch.py without a board
# (plan #362, item console-watch).
#
#   tools/mp2s_console_watch_selftest.sh
#
# Scan verdicts on synthetic logs (ok for 2 boots; a missing image; a
# missing Linux; boots left not counting down; too few and too many
# cycles), and the passive "watch" on a pseudo-terminal: a feeder writes a
# 3-cycle console to the pty master, the watcher reads it, and the feeder
# checks that the watcher wrote nothing back.
#
# A tools/check.sh row for this selftest waits for plan #341, so it is run
# by hand: tools/mp2s_console_watch_selftest.sh
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=$(mktemp -d "${TMPDIR:-$here}/mp2s-console-watch-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
W="$here/mp2s_console_watch.py"
pass=0
fail=0

ok() { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; }

# cyc LEFT IMAGE LINUX: one boot cycle on stdout
cyc() {
    echo "NOTICE:  BL2: v2.10"
    echo "U-Boot 2023.10"
    [ -z "$1" ] || echo "mp2z: boots left $1"
    if [ "$2" = y ]; then
        echo "*** Booting Zephyr OS ***"
        echo "mp2: done, PSCI SYSTEM_RESET at 1234 ms"
    fi
    if [ "$3" = y ]; then echo "Starting kernel ..."; fi
}

# expect NAME WANT_EXIT WANT_VERDICT BOOTS (the log is in $T/in)
expect() {
    local name=$1 want=$2 v=$3 boots=$4 rc=0
    cp "$T/in" "$T/log"
    "$W" scan "$T/log" --boots "$boots" >"$T/out" 2>"$T/err" || rc=$?
    if [ "$rc" = "$want" ] && grep -q "^verdict: $v " "$T/out"; then ok "$name"
    else bad "$name (exit $rc, $(tail -1 "$T/out"))"; fi
}

{ cyc 1 y n; cyc 0 y n; cyc "" n y; } >"$T/in"; expect "2 boots then Linux" 0 ok 2
{ cyc "" n y; } >"$T/in"; expect "0 boots, Linux" 0 ok 0
{ cyc 1 y n; cyc 0 n n; cyc "" n y; } >"$T/in"; expect "image missing" 1 bad_boots 2
{ cyc 1 y n; cyc 0 y n; cyc "" n n; } >"$T/in"; expect "Linux missing" 1 no_linux 2
{ cyc 0 y n; cyc 1 y n; cyc "" n y; } >"$T/in"; expect "count not down" 1 bad_count 2
{ cyc 1 y n; cyc "" n y; } >"$T/in"; expect "too few cycles" 1 bad_boots 2
{ cyc 1 y n; cyc 0 y n; cyc "" y y; cyc "" n y; } >"$T/in"; expect "too many cycles" 1 bad_boots 2
{ cyc 1 y n; cyc 0 y n; cyc "" n y; } >"$T/in"; sed -i 's/^/1700000000.000 /' "$T/in"; expect "stamped log" 0 ok 2

rc=0
"$W" scan "$T/missing" >/dev/null 2>&1 || rc=$?
[ "$rc" = 2 ] && ok "missing log: exit 2" || bad "missing log: exit $rc"
rc=0
"$W" watch /nonexistent/tty "$T/r" "$T/s" >/dev/null 2>&1 || rc=$?
[ "$rc" = 4 ] && ok "unreachable console: exit 4" || bad "unreachable console: exit $rc"

# Passive watch on a pty: the watcher must read all of it and send nothing.
python3 -I - "$W" "$T" <<'PY' && ok "watch on a pty reads everything and sends nothing" || bad "watch on a pty"
import os, subprocess, sys, select, time, pty, tty
w, t = sys.argv[1:3]
master, slave = pty.openpty()
tty.setraw(slave)
name = os.ttyname(slave)
p = subprocess.Popen([w, "watch", name, t + "/raw", t + "/st", "--boots", "2", "--linger", "1",
                      "--timeout", "30"], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
time.sleep(1)
text = b""
for left in (1, 0):
    text += (b"NOTICE:  BL2: v2.10\r\nU-Boot 2023.10\r\nmp2z: boots left %d\r\n"
             b"*** Booting Zephyr OS ***\r\nmp2: PSCI SYSTEM_RESET at 99 ms\r\n" % left)
text += b"NOTICE:  BL2: v2.10\r\nU-Boot 2023.10\r\nStarting kernel ...\r\n"
os.write(master, text)
back = b""
end = time.time() + 20
while p.poll() is None and time.time() < end:
    r, _, _ = select.select([master], [], [], 0.2)
    if r:
        back += os.read(master, 4096)
out, err = p.communicate(timeout=5)
raw = open(t + "/raw", "rb").read()
assert p.returncode == 0, (p.returncode, out, err)
assert raw == text, raw
assert b"verdict: ok" in out, out
# the pty echoes nothing in raw mode, so anything on the master came from the watcher
assert back == b"", back
PY

echo "$pass passed, $fail failed"
[ "$fail" = 0 ]
