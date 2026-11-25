#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# console-check-selftest.sh - console-check.sh against a fake board and a
# fake UART under $TMPDIR. No board, no network.
#
#   bash driver/tools/uboot-t6/console-check-selftest.sh
#
# console-check.sh and prompt-lib.sh are copied into a tree of their own,
# with a fake ../kexec-zephyr/t6-uart-rfc2217.py beside them. An `ssh` first
# in PATH plays the board: it logs every command, answers the facts from
# FAKE_* variables, and its /dev/ttyS2 is a file. The fake tool makes a real
# pty (raw, no echo) and runs the command on it, as the real one does; it
# copies what the board wrote to its /dev/ttyS2 into the pty, as is, garbled
# (a line at another rate) or not at all, and keeps every byte the command
# writes to the pty. Run as `tty`, it gives the command the pty as a local
# T6_UART; run as `run` (console-check's RFC 2217 path) it also answers
# busy (exit 40), no session (40), baud not confirmed (42) or failed (41).
#
# Each exit code of console-check.sh's header is reached once, and every
# run checks that nothing was written to T6_UART and that no output names
# T6_UART's address or BOARD_HOST.
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
W=$(mktemp -d "${TMPDIR:-/tmp}/console-check-st.XXXXXX") || exit 1
trap 'rm -rf "$W"' EXIT
fails=0
n=0
check() {
	n=$((n + 1))
	if [ "$2" = 0 ]; then echo "ok   $1"; else
		echo "FAIL $1"
		fails=$((fails + 1))
	fi
}

F=$W/tree/driver/tools
mkdir -p "$F/uboot-t6" "$F/kexec-zephyr" "$W/fakebin"
cp "$HERE/console-check.sh" "$HERE/prompt-lib.sh" "$F/uboot-t6/"
CC=$F/uboot-t6/console-check.sh
FAKE=$F/kexec-zephyr/t6-uart-rfc2217.py

cat >"$FAKE" <<'EOF'
import os, pty, select, subprocess, sys, time, tty
how = sys.argv[1]
cmd = sys.argv[sys.argv.index("--") + 1:]
mode = os.environ.get("FAKE_UART", "ok")
if how == "run":
    with open(os.environ["FAKE_TOOL_LOG"], "a") as f:
        f.write(" ".join(cmd) + "\n")
    if mode == "busy":
        print("t6-uart-rfc2217: not started: the adapter answered 'busy..'", file=sys.stderr)
        sys.exit(40)
    if mode == "noconn":
        # the real tool names no address; this one does, for the scrub
        host = os.environ["T6_UART"][len("rfc2217://"):].rsplit(":", 1)[0]
        print("t6-uart-rfc2217: not started: %s: connection refused" % host, file=sys.stderr)
        sys.exit(40)
    if mode == "baud":
        print("t6-uart-rfc2217: not started: server baud 115200, expected 1500000", file=sys.stderr)
        sys.exit(42)
    if mode == "fail":
        print("t6-uart-rfc2217: failed during the run", file=sys.stderr)
        sys.exit(41)
master, slave = pty.openpty()
tty.setraw(slave)
env = dict(os.environ, T6_UART=os.ttyname(slave))
env.pop("T6_UART_RFC2217", None)
if how == "run":
    env["T6_UART_RFC2217"] = "1"
p = subprocess.Popen(cmd, env=env)
os.close(slave)
tx, off = os.environ["FAKE_TTYS2"], 0
typed = open(os.environ["FAKE_TYPED"], "ab")
while p.poll() is None:
    r, _, _ = select.select([master], [], [], 0.05)
    if r:
        try:
            typed.write(os.read(master, 4096))
            typed.flush()
        except OSError:
            time.sleep(0.05)
    try:
        with open(tx, "rb") as f:
            data = f.read()
    except FileNotFoundError:
        data = b""
    new, off = data[off:], len(data)
    if new and mode != "silent":
        if mode == "garble":
            new = bytes((((b * 7) ^ 0xA5) & 0xFF) | 0x80 for b in new[::4])
        try:
            os.write(master, new)
        except OSError:
            pass
sys.exit(p.returncode)
EOF

cat >"$W/fakebin/ssh" <<'EOF'
#!/usr/bin/env bash
cmd=${!#}
printf '%s\n----\n' "$cmd" >>"$FAKE_SSH_LOG"
host=${*: -2:1}
case "${FAKE_SSH:-ok}" in down)
	echo "ssh: Could not resolve hostname ${host#*@}: Name or service not known" >&2
	exit 255
	;;
esac
case "$cmd" in *console-check*'/dev/ttyS2'*) if [ "${FAKE_SSH:-ok}" = nowrite ]; then
	echo "Connection to ${host#*@} closed by remote host." >&2
	exit 255
fi ;; esac
cmd=${cmd//\/dev\/ttyS2/$FAKE_TTYS2}
cmd=${cmd//\/proc\/device-tree\/compatible/$FAKE_DT}
pre='id() { echo "${FAKE_UID:-0}"; }
stty() { if [ "$1" = -F ] && [ "$3" = speed ]; then echo "${FAKE_SPEED:-1500000}"; else command stty "$@"; fi; }
'
exec bash -c "$pre$cmd"
EOF
chmod +x "$W/fakebin/ssh"

printf 'friendlyarm,nanopc-t6\0rockchip,rk3588\0' >"$W/dt-t6"
printf 'radxa,rock-5b\0rockchip,rk3588\0' >"$W/dt-other"
SECRET_UART=rfc2217://secret-uart.invalid:7777
SECRET_HOST=root@secret-board.invalid

# cc <how> [VAR=value...] [-- args...]: one run of console-check.sh; how is
# tty (the fake pty as a local T6_UART), rfc (T6_UART an rfc2217:// address)
# or plain (T6_UART as given). Sets out, rc, ssh_n (ssh commands sent) and
# typed (bytes written to T6_UART).
cc() {
	local how=$1 vars=() args=()
	shift
	while [ $# -gt 0 ] && [ "$1" != -- ]; do vars+=("$1"); shift; done
	[ "${1:-}" != -- ] || { shift; args=("$@"); }
	rm -f "$W/ttyS2" "$W/typed" "$W/ssh.log" "$W/tool.log"
	: >"$W/typed" >"$W/ssh.log" >"$W/tool.log"
	local base=(env -u T6_UART_RFC2217 -u CONSOLE_CHECK_READ_S -u BOARD_SSH_OPTS
		PATH="$W/fakebin:$PATH" TMPDIR="$W" BOARD_HOST="$SECRET_HOST" T6_UART="$SECRET_UART"
		FAKE_TTYS2="$W/ttyS2" FAKE_DT="$W/dt-t6" FAKE_TYPED="$W/typed"
		FAKE_SSH_LOG="$W/ssh.log" FAKE_TOOL_LOG="$W/tool.log")
	case "$how" in
	tty) out=$("${base[@]}" "${vars[@]+"${vars[@]}"}" python3 -I "$FAKE" tty -- bash "$CC" "${args[@]+"${args[@]}"}" 2>&1) ;;
	*) out=$("${base[@]}" "${vars[@]+"${vars[@]}"}" bash "$CC" "${args[@]+"${args[@]}"}" 2>&1) ;;
	esac
	rc=$?
	ssh_n=$(grep -c '^----$' "$W/ssh.log")
	typed=$(wc -c <"$W/typed")
	! grep -q 'secret-uart\|secret-board' <<<"$out"
	check "$how ${vars[*]-} ${args[*]-}: no address in the output" $?
}
expect() { # <what> <code>
	if [ "$rc" = "$2" ]; then check "$1: exit $2" 0; else
		check "$1: exit $2 (got $rc)" 1
		sed 's/^/    /' <<<"$out"
	fi
	[ "$typed" = 0 ]
	check "$1: nothing written to T6_UART" $?
}

# 0: the line read back, local tty and RFC 2217
cc tty
expect "local tty, line read back" 0
grep -q 'console-check: the line written on the board was read back verbatim' <<<"$out"
check "local tty: the verdict line" $?
[ "$ssh_n" = 2 ]
check "local tty: two ssh commands (facts, the line)" $?
grep -q "^printf '\\\\r\\\\n%s\\\\r\\\\n' '#### console-check [0-9a-f]\{16\}' >/dev/ttyS2$" "$W/ssh.log"
check "local tty: the line is one printf to /dev/ttyS2" $?
! grep -Eq 'reboot|kexec|dd |mkfs|/dev/mmc|fw_setenv' "$W/ssh.log"
check "local tty: no ssh command reboots or writes a block device" $?
grep -q '#### console-check [0-9a-f]\{16\}' "$W/ttyS2"
check "local tty: the board's console got the line" $?
cc rfc
expect "RFC 2217, line read back" 0
grep -q 'console-check.sh --inner$' "$W/tool.log"
check "RFC 2217: run again inside t6-uart-rfc2217.py with --inner" $?
grep -q 'adapter confirmed 1500000' <<<"$out"
check "RFC 2217: says the adapter confirmed the line" $?

# 1: the tool failed
cc rfc FAKE_UART=fail
expect "RFC 2217 tool failed" 1
# 2: usage
cc plain -- --bogus
expect "an unknown argument" 2
cc plain BOARD_HOST=
expect "BOARD_HOST unset" 2
cc plain T6_UART=
expect "T6_UART unset" 2
cc plain CONSOLE_CHECK_READ_S=0
expect "CONSOLE_CHECK_READ_S 0" 2
cc plain T6_UART_RFC2217=1
expect "T6_UART_RFC2217 set outside the tool" 2
cc plain -- --inner
expect "--inner outside the tool" 2
[ "$ssh_n" = 0 ]
check "usage errors: no ssh command" $?
# 3: the board
cc rfc FAKE_SSH=down
expect "ssh down" 3
grep -q 'Could not resolve hostname <address>' <<<"$out"
check "ssh down: ssh's message shown, host scrubbed" $?
[ -s "$W/tool.log" ]
check "ssh down: the UART not opened" $((!$?))
cc rfc FAKE_DT="$W/dt-other"
expect "not a NanoPC-T6" 3
cc rfc FAKE_UID=1000
expect "not root" 3
cc tty FAKE_SSH=nowrite
expect "the line not written" 3
# 4: the board's console tty
cc rfc FAKE_SPEED=115200
expect "board ttyS2 at 115200" 4
[ -s "$W/tool.log" ]
check "board ttyS2 at 115200: the UART not opened" $((!$?))
# 5: T6_UART no usable tty
: >"$W/notatty"
cc plain T6_UART="$W/notatty"
expect "T6_UART a regular file" 5
! grep -qF "$W/notatty" <<<"$out"
check "T6_UART a regular file: its path is not printed" $?
grep -q '<address>' <<<"$out"
check "T6_UART a regular file: uart_ready's message kept, path scrubbed" $?
[ "$ssh_n" = 1 ]
check "T6_UART a regular file: no line written on the board" $?
# 6, 7, 8: the RFC 2217 session
cc rfc FAKE_UART=busy
expect "adapter slot busy" 6
grep -q "busy" <<<"$out"
check "adapter slot busy: the tool's line is shown" $?
cc rfc FAKE_UART=noconn
expect "no RFC 2217 session" 7
grep -q '<address>: connection refused' <<<"$out"
check "no RFC 2217 session: the tool's line shown, adapter scrubbed" $?
cc rfc FAKE_UART=baud
expect "baud not confirmed" 8
[ "$ssh_n" = 1 ]
check "baud not confirmed: no line written on the board" $?
# 9: silence, 10: garbage
cc tty FAKE_UART=silent CONSOLE_CHECK_READ_S=1
expect "silent UART" 9
cc rfc FAKE_UART=garble CONSOLE_CHECK_READ_S=1
expect "garbled line (another rate)" 10
grep -q 'without the line' <<<"$out"
check "garbled line: the bytes are counted" $?

# preflight.sh's check e prints the command for an executable console-check.sh
[ -x "$HERE/console-check.sh" ]
check "console-check.sh is executable (preflight.sh check e)" $?

echo "console-check selftest: $n checks run, $fails failed"
[ "$fails" = 0 ]
