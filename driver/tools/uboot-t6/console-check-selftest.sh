#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# console-check-selftest.sh - console-check.sh against a made-up tty: a pty
# pair driven by a Python helper below (as t6-uart-rfc2217.py --self-test
# does), a made-up RFC 2217 adapter that answers "busy", and a stand-in ssh.
# Every exit code of console-check.sh, no board, no network beyond
# 127.0.0.1. Run: bash driver/tools/uboot-t6/console-check-selftest.sh
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
CC=$HERE/console-check.sh
tmp=$(mktemp -d)
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done; rm -rf "$tmp"; }
trap cleanup EXIT

fails=0
check() { # <name> <ok:0|1>
	if [ "$2" = 0 ]; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi
}

# The fake tty. fake.py <mode> <dir>: makes the pty, writes the slave's path
# to <dir>/tty, and when it reads the CR (or, for "hold", at once) it
# answers per mode. What it received is saved in <dir>/rx at the end.
cat >"$tmp/fake.py" <<'PY'
import os, select, subprocess, sys, time
mode, d = sys.argv[1], sys.argv[2]
m, s = os.openpty()
name = os.ttyname(s)
holder = None
if mode == "busy":
    holder = subprocess.Popen(["sleep", "30"], stdin=s)
os.close(s)
open(d + "/tty.tmp", "w").write(name)
os.rename(d + "/tty.tmp", d + "/tty")
rx, sent, end = b"", False, time.monotonic() + 12
ans = {
    "login": b"\r\nArmbian 24 nanopc-t6 ttyFIQ0\r\n\r\nnanopc-t6 login: ",
    "prompt": b"\r\n\x1b[01;32mroot@nanopc-t6\x1b[00m:\x1b[01;34m~\x1b[00m# ",
    "garbage": bytes([0xe0, 0x80, 0xfe, 0x00, 0xf8, 0x1c, 0xe6, 0x80, 0x9e, 0x00, 0xfc, 0x66] * 8),
    "text": b"\r\nsome text with no prompt in it\r\nmore text\r\n",
    "silent": b"",
}.get(mode, b"")
while time.monotonic() < end:
    try:
        r, _, _ = select.select([m], [], [], 0.1)
        if r:
            c = os.read(m, 256)
            rx += c
            if b"\r" in rx and not sent:
                sent = True
                os.write(m, ans)
    except OSError:
        time.sleep(0.05)
open(d + "/rx", "wb").write(rx)
if holder:
    holder.kill()
PY

# The fake adapter: accepts, says "busy", and shuts its write side.
cat >"$tmp/adapter.py" <<'PY'
import socket, sys, time
s = socket.socket()
s.bind(("127.0.0.1", 0))
s.listen(4)
open(sys.argv[1], "w").write(str(s.getsockname()[1]))
end = time.monotonic() + 15
s.settimeout(1)
while time.monotonic() < end:
    try:
        c, _ = s.accept()
    except OSError:
        continue
    c.sendall(b"busy\r\n")
    c.shutdown(socket.SHUT_WR)
PY

# Stand-in ssh: the board answers (up) or not, by FAKE_SSH.
mkdir "$tmp/bin"
cat >"$tmp/bin/ssh" <<'SH'
#!/usr/bin/env bash
[ "${FAKE_SSH:-up}" = up ] && { echo 1234.5; exit 0; }
exit 255
SH
chmod +x "$tmp/bin/ssh"
export PATH=$tmp/bin:$PATH
export BOARD_HOST=root@t6.invalid READ_S=1

# run_mode <mode> -> sets rc, out; the helper's rx in $tmp/<mode>/rx
run_mode() {
	local mode=$1 i
	mkdir "$tmp/$mode"
	python3 -I "$tmp/fake.py" "$mode" "$tmp/$mode" &
	pids+=($!)
	for i in $(seq 50); do [ -e "$tmp/$mode/tty" ] && break; sleep 0.1; done
	out=$(T6_UART=$(cat "$tmp/$mode/tty") bash "$CC" 2>&1)
	rc=$?
	echo "$out" | sed 's/^/     | /'
	wait "${pids[-1]}" 2>/dev/null
}
rx_is_cr() { [ "$(od -An -c <"$tmp/$1/rx" | tr -d ' \n')" = '\r' ]; }
no_path() { [[ $out != *"$(cat "$tmp/$1/tty")"* ]]; }

run_mode login
check "login prompt: exit 0" "$([ $rc = 0 ] && echo 0 || echo 1)"
check "login: exactly one CR written, nothing else" "$(rx_is_cr login && echo 0 || echo 1)"
check "login: the tty path is not printed" "$(no_path login && echo 0 || echo 1)"

run_mode prompt
check "shell prompt with colour codes: exit 0" "$([ $rc = 0 ] && echo 0 || echo 1)"
check "prompt: exactly one CR written" "$(rx_is_cr prompt && echo 0 || echo 1)"

FAKE_SSH=up run_mode silent
check "nothing received: exit 5" "$([ $rc = 5 ] && echo 0 || echo 1)"
check "silent: one CR written" "$(rx_is_cr silent && echo 0 || echo 1)"
check "silent: ssh line says Armbian is up" "$([[ $out == *"ssh sees Armbian up"* ]] && echo 0 || echo 1)"

run_mode garbage
check "garbage: exit 6" "$([ $rc = 6 ] && echo 0 || echo 1)"
check "garbage: the count of non-printable bytes is printed" "$([[ $out == *"96 bytes received, 96 of them"* || $out == *"non-printable"* ]] && echo 0 || echo 1)"
check "garbage: no raw byte printed" "$(printf %s "$out" | LC_ALL=C grep -qP '[\x80-\xff\x00]' && echo 1 || echo 0)"

run_mode text
check "text without a prompt: exit 7" "$([ $rc = 7 ] && echo 0 || echo 1)"

run_mode busy
check "tty held open by another process: exit 4" "$([ $rc = 4 ] && echo 0 || echo 1)"
check "busy: nothing written" "$([ ! -s "$tmp/busy/rx" ] && echo 0 || echo 1)"

# Invalid or unset T6_UART, unset BOARD_HOST.
out=$(env -u T6_UART bash "$CC" 2>&1); rc=$?
check "T6_UART unset: exit 3" "$([ $rc = 3 ] && echo 0 || echo 1)"
out=$(T6_UART=/dev/null/nope bash "$CC" 2>&1); rc=$?
check "T6_UART not a device: exit 3" "$([ $rc = 3 ] && echo 0 || echo 1)"
out=$(T6_UART=/etc/hostname bash "$CC" 2>&1); rc=$?
check "T6_UART a plain file: exit 3" "$([ $rc = 3 ] && echo 0 || echo 1)"
out=$(T6_UART=rfc2217://nonsense bash "$CC" 2>&1); rc=$?
check "T6_UART an invalid rfc2217 address: exit 3" "$([ $rc = 3 ] && echo 0 || echo 1)"
out=$(env -u BOARD_HOST T6_UART=/dev/null bash "$CC" 2>&1); rc=$?
check "BOARD_HOST unset: exit 3" "$([ $rc = 3 ] && echo 0 || echo 1)"
out=$(bash "$CC" extra 2>&1); rc=$?
check "an argument: exit 2" "$([ $rc = 2 ] && echo 0 || echo 1)"

# The adapter's one slot busy: exit 4, and the address is not printed.
python3 -I "$tmp/adapter.py" "$tmp/port" &
pids+=($!)
for i in $(seq 50); do [ -s "$tmp/port" ] && break; sleep 0.1; done
addr=rfc2217://127.0.0.1:$(cat "$tmp/port")
out=$(T6_UART=$addr bash "$CC" 2>&1); rc=$?
echo "$out" | sed 's/^/     | /'
check "adapter says busy: exit 4" "$([ $rc = 4 ] && echo 0 || echo 1)"
check "busy: the address is not printed" "$([[ $out != *127.0.0.1* && $out != *"$(cat "$tmp/port")"* ]] && echo 0 || echo 1)"

if [ "$fails" = 0 ]; then echo "console-check-selftest: all ok"; exit 0; fi
echo "console-check-selftest: $fails failed"
exit 1
