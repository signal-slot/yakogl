#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# console-check.sh - prove that the NanoPC-T6's debug UART reads at 1500000
# on this host before a board session reboots anything.
#
#   BOARD_HOST=root@<t6> T6_UART=<tty> driver/tools/uboot-t6/console-check.sh
#   BOARD_HOST=root@<t6> T6_UART=rfc2217://<host>:<port> driver/tools/uboot-t6/console-check.sh
#
# Why: three board sessions of plan #408 (docs/notes/nanopc-t6-uboot-handover.md
# section 5, "Board sessions") stopped after their preparation because the
# console reached this host at the adapter's stored 115200 while the T6's
# console runs at 1500000 (the bytes came as garbage), or because the
# adapter's one client slot was busy. prompt-reads.sh and prompt-go.sh find
# that out only after they took the lock and started; this finds it first.
#
# What it does, with Armbian running on the board:
#   1. One ssh command reads the board: compatible (a NanoPC-T6), uid 0, and
#      `stty -F /dev/ttyS2 speed` (Armbian's console tty, which must be at
#      1500000).
#   2. It opens T6_UART the way the runners do: a local tty is set to
#      1500000 raw with no echo and read back (prompt-lib.sh's uart_ready);
#      an rfc2217:// address goes through ../kexec-zephyr/t6-uart-rfc2217.py,
#      which must have the adapter confirm 1500000 8N1 before this script
#      runs again inside it, on the pty that tool makes.
#   3. With a reader open on T6_UART, one ssh command writes one line,
#      "#### console-check <16 random hex digits>", to /dev/ttyS2 on the
#      board. That is output of the board's console only.
#   4. The line must be read back on T6_UART, byte for byte, within
#      CONSOLE_CHECK_READ_S seconds (default 5, 1 to 60).
#
# It never writes a byte to T6_UART: Armbian runs an autologin root shell
# on that line, and every byte written there is typed into it. Echo is off
# on T6_UART before the reader opens it (uart_ready, or the RFC 2217 tool's
# pty), so the line written in step 3 is not sent back to the board. It
# takes no board lock, reboots nothing, and writes nothing on the board
# but the one line to its console. It prints no address: neither T6_UART
# nor BOARD_HOST is in its output.
#
# Exit, one meaning each (the session goes on only after 0):
#    0  the line written on the board was read back verbatim: the console
#       reads at 1500000 on this host
#    1  this script or the RFC 2217 tool failed (a missing tool, the tool's
#       exit 41)
#    2  usage: an argument, or BOARD_HOST or T6_UART unset
#    3  the board over ssh: no answer, not a NanoPC-T6, not root, or the
#       line could not be written to /dev/ttyS2
#    4  the board's /dev/ttyS2 is not at 1500000
#    5  T6_UART is a local tty this user cannot use: not a character device
#       it can read and write, open by another process, or stty did not
#       take 1500000 raw
#    6  the adapter's one client slot is busy (t6-uart-rfc2217.py exit 40,
#       answered "busy"): another client, such as the farm's console server,
#       holds it
#    7  the RFC 2217 session could not be opened for another reason
#       (t6-uart-rfc2217.py exit 40 without "busy")
#    8  the adapter did not confirm 1500000 (t6-uart-rfc2217.py exit 42)
#    9  no byte at all was read on T6_UART within the time
#   10  bytes were read but not the line: the console reaches this host at
#       another rate (garbage), or the bytes are not the board's
# The last line is "console-check: <verdict> (exit <n>)".
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
K=$HERE/../kexec-zephyr
ME=console-check

# scrub: stdin with BOARD_HOST, T6_UART and their host parts replaced, so
# that no message (ssh's own, uart_ready's, the tool's) names an address.
scrub() {
	local w text
	text=$(cat)
	for w in "${BOARD_HOST:-}" "${BOARD_HOST#*@}" "${T6_UART:-}"; do
		[ -n "$w" ] || continue
		text=${text//"$w"/<address>}
	done
	if [[ ${T6_UART:-} == rfc2217://* ]]; then
		w=${T6_UART#rfc2217://}
		w=${w%:*}
		[ -z "$w" ] || text=${text//"$w"/<address>}
	fi
	[ -z "$text" ] || printf '%s\n' "$text"
}
say() { echo "$ME: $*" | scrub >&2; }
finish() { # <code> <verdict>
	echo "$ME: $2 (exit $1)"
	exit "$1"
}
die() { say "$*"; finish 2 "usage error"; }

INNER=0
for a in "$@"; do
	case "$a" in
	--inner) INNER=1 ;; # internal: the run inside t6-uart-rfc2217.py
	-h | --help)
		sed -n '2,/^set -uo/{/^set -uo/d;s/^# \{0,1\}//;p}' "$0"
		exit 0
		;;
	*) die "usage: console-check.sh (see the header)" ;;
	esac
done
[ -n "${BOARD_HOST:-}" ] || die "set BOARD_HOST=root@<the T6>"
[ -n "${T6_UART:-}" ] || die "set T6_UART=<the host tty on the T6's debug UART, or rfc2217://<host>:<port>>"
READ_S=${CONSOLE_CHECK_READ_S:-5}
[[ $READ_S =~ ^[0-9]+$ ]] && [ "$READ_S" -ge 1 ] && [ "$READ_S" -le 60 ] ||
	die "CONSOLE_CHECK_READ_S must be a whole number of seconds from 1 to 60"
if [ "$INNER" = 1 ]; then
	[ -n "${T6_UART_RFC2217:-}" ] || die "--inner is only for the run inside t6-uart-rfc2217.py"
elif [ -n "${T6_UART_RFC2217:-}" ]; then
	die "T6_UART_RFC2217 is set outside t6-uart-rfc2217.py; unset it"
fi
for t in ssh timeout python3 stty od; do
	command -v "$t" >/dev/null || { say "$t is not on this host"; finish 1 "a tool is missing"; }
done

# shellcheck disable=SC2206  # deliberately word-split, as the runners do
SSH_OPTS=(${BOARD_SSH_OPTS:-})
refuse() { say "$*"; finish 3 "the board did not answer as a NanoPC-T6 with root"; }
# shellcheck source=prompt-lib.sh
. "$HERE/prompt-lib.sh" # board(), uart_ready()

tmp=$(mktemp -d "${TMPDIR:-/tmp}/console-check.XXXXXX") || finish 1 "no temporary directory"
trap 'rm -rf "$tmp"' EXIT

# ---- 1. the board, read only (once, before the UART is opened) -------------
if [ "$INNER" = 0 ]; then
	facts=$(board 2>"$tmp/ssh.err" 'printf "compatible=%s\n" "$(tr "\0" " " </proc/device-tree/compatible)"
printf "uid=%s\n" "$(id -u)"
printf "speed=%s\n" "$(stty -F /dev/ttyS2 speed 2>/dev/null)"') ||
		refuse "cannot read the board's facts over ssh: $(tail -n 3 "$tmp/ssh.err")"
	fact() { printf '%s\n' "$facts" | sed -n "s/^$1=//p" | head -1; }
	case "$(fact compatible)" in *friendlyarm,nanopc-t6*) ;; *) refuse "the board is not a NanoPC-T6: compatible '$(fact compatible)'" ;; esac
	[ "$(fact uid)" = 0 ] || refuse "the ssh user on the board is not root (writing /dev/ttyS2 needs it)"
	if [ "$(fact speed)" != 1500000 ]; then
		say "the board's /dev/ttyS2 is at '$(fact speed)', not 1500000"
		finish 4 "the board's console tty is not at 1500000"
	fi
	echo "$ME: board: NanoPC-T6, root, /dev/ttyS2 at 1500000"
fi

# ---- 2. T6_UART -------------------------------------------------------------
if [ "$INNER" = 0 ] && [[ $T6_UART == rfc2217://* ]]; then
	echo "$ME: T6_UART is an RFC 2217 address: opening it with t6-uart-rfc2217.py (1500000 8N1)"
	rc=0
	python3 -I "$K/t6-uart-rfc2217.py" run -- bash "$HERE/$ME.sh" --inner 2>"$tmp/tool.err" || rc=$?
	# The tool names no address in its messages; this script's own lines
	# from inside it are on stdout, which passes straight through.
	sed 's/^/  /' "$tmp/tool.err" | scrub >&2
	case "$rc" in
	40)
		if grep -qi busy "$tmp/tool.err"; then
			finish 6 "the adapter's client slot is busy (another client holds it)"
		fi
		finish 7 "the RFC 2217 session could not be opened"
		;;
	41) finish 1 "t6-uart-rfc2217.py failed during the check" ;;
	42) finish 8 "the adapter did not confirm 1500000" ;;
	esac
	# Otherwise the code is the inner run's, whose last line said it.
	exit "$rc"
fi
if [ "$INNER" = 0 ]; then
	ready_out=$( (
		refuse() { echo "$*"; exit 1; }
		uart_ready
	) 2>&1) || {
		say "$ready_out"
		finish 5 "T6_UART is not a tty this user can use at 1500000"
	}
	echo "$ME: T6_UART is a tty set to 1500000 raw, no echo (read back)"
else
	echo "$ME: T6_UART is t6-uart-rfc2217.py's pty; the adapter confirmed 1500000 8N1"
fi

# ---- 3. the line, read back -------------------------------------------------
token=$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')
[[ $token =~ ^[0-9a-f]{16}$ ]] || finish 1 "no random token"
line="#### console-check $token"
# The reader: opens T6_UART for reading only, says so in $tmp/ready, keeps
# every byte in $tmp/got, and ends at the line or the deadline. Exit 0 the
# line was read, 9 no byte, 10 bytes but not the line, 1 an error.
python3 -I -c '
import os, select, sys, time
path, want, secs, got, ready = sys.argv[1], sys.argv[2].encode(), float(sys.argv[3]), sys.argv[4], sys.argv[5]
try:
    fd = os.open(path, os.O_RDONLY | os.O_NOCTTY | os.O_NONBLOCK)
except OSError as e:
    print("cannot open T6_UART: %s" % e.strerror, file=sys.stderr)
    sys.exit(1)
open(ready, "w").close()
data = b""
end = time.monotonic() + secs
with open(got, "wb") as out:
    while want not in data:
        left = end - time.monotonic()
        if left <= 0:
            break
        r, _, _ = select.select([fd], [], [], left)
        if not r:
            continue
        try:
            b = os.read(fd, 4096)
        except BlockingIOError:
            continue
        except OSError:
            time.sleep(0.05)
            continue
        data += b
        out.write(b)
        out.flush()
sys.exit(0 if want in data else 10 if data else 9)
' "$T6_UART" "$line" "$READ_S" "$tmp/got" "$tmp/ready" &
reader=$!
n=0
until [ -e "$tmp/ready" ]; do
	n=$((n + 1))
	if [ "$n" -gt 50 ] || ! kill -0 "$reader" 2>/dev/null; then
		wait "$reader" 2>/dev/null
		finish 1 "the reader did not open T6_UART"
	fi
	sleep 0.1
done
if ! board "printf '\\r\\n%s\\r\\n' '$line' >/dev/ttyS2" 2>"$tmp/ssh.err"; then
	kill "$reader" 2>/dev/null
	wait "$reader" 2>/dev/null
	say "the line could not be written to /dev/ttyS2 on the board over ssh: $(tail -n 3 "$tmp/ssh.err")"
	finish 3 "the board did not take the line on its console"
fi
rc=0
wait "$reader" || rc=$?
bytes=$(wc -c <"$tmp/got" 2>/dev/null || echo 0)
case "$rc" in
0) finish 0 "the line written on the board was read back verbatim: the console reads at 1500000" ;;
9) finish 9 "no byte read on T6_UART in $READ_S s" ;;
10)
	printable=$(LC_ALL=C tr -cd '\11\12\15\40-\176' <"$tmp/got" | wc -c)
	say "read $bytes bytes in $READ_S s, $printable of them printable ASCII, without the line; the first ones:"
	head -c 32 "$tmp/got" | od -An -tx1 | sed 's/^/  /' >&2
	finish 10 "bytes but not the line: the console reaches this host at another rate"
	;;
*) finish 1 "the reader failed (exit $rc)" ;;
esac
