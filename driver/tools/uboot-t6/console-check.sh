#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# console-check.sh - before a board session reboots the NanoPC-T6, prove
# that its debug UART is read at the console's 1500000 baud: send one CR to
# the running Armbian and look for its shell or login prompt as text.
# Sessions have stopped because the line was read at 115200 (a serial
# adapter's stored speed, or the farm's console server) and every byte was
# garbage, or because the adapter's one slot was busy
# (docs/notes/nanopc-t6-uboot-handover.md, section 5, "Board sessions").
#
#   BOARD_HOST=root@<t6> T6_UART=<tty> driver/tools/uboot-t6/console-check.sh
#   BOARD_HOST=root@<t6> T6_UART=rfc2217://<host>:<port> ...console-check.sh
#
# T6_UART (a local tty, or an RFC 2217 address, taken from the environment
# only) and BOARD_HOST (ssh) are the same as prompt-reads.sh's. For an
# rfc2217:// address this runs itself again through
# ../kexec-zephyr/t6-uart-rfc2217.py (set 1500000 8N1 and read back), as
# prompt-lib.sh's runners do. The address and the tty's path are never
# printed.
#
# What it writes to the UART: ONE carriage return, nothing else, never a
# command: Armbian's console is an autologin root shell. It does not reboot
# or change the board; the one ssh command (only after a failure) reads
# /proc/uptime. Run it before prompt-reads.sh --go or prompt-go.sh --go.
#
# Exit:
#   0   the answer holds Armbian's shell prompt or login prompt as text
#   2   usage
#   3   T6_UART or BOARD_HOST unset or invalid (not a character device this
#       user can read and write, or not rfc2217://<host>:<port>); nothing
#       was written
#   4   the line is busy: another process has the tty open, or the adapter
#       told the session "busy"; nothing was written
#   5   nothing received within READ_S seconds after the CR
#   6   bytes received, but not text (the usual sign of the wrong baud): the
#       count of non-printable bytes is printed, never the bytes
#   7   readable text received, but no shell or login prompt in it (the board
#       is not in Armbian's console, or the answer was cut)
#   40  t6-uart-rfc2217.py could not open the session or set the line (not
#       "busy"); 41 that tool failed during the run
# After 5, 6 and 7 one line says whether ssh sees Armbian up (read only).
# READ_S: seconds to read (default 4).
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
K=$HERE/../kexec-zephyr
TOOL=console-check
READ_S=${READ_S:-4}
SSH_OPTS=(${SSH_OPTS:-})

die() { echo "console-check.sh: $*" >&2; exit 2; }
refuse() {
	echo "REFUSED: $*" >&2
	echo "nothing was written to the UART." >&2
	exit 3
}
# shellcheck source=prompt-lib.sh
. "$HERE/prompt-lib.sh"

[ "$#" = 0 ] || die "usage: console-check.sh (no arguments; see the header)"
[[ $READ_S =~ ^[0-9]+$ ]] && [ "$READ_S" -ge 1 ] || die "READ_S is not a whole number of seconds"
[ -n "${BOARD_HOST:-}" ] || refuse "BOARD_HOST is not set (root@<t6>)"
[ -n "${T6_UART:-}" ] || refuse "T6_UART is not set (the host tty on the T6's debug UART, or rfc2217://<host>:<port>)"

# An rfc2217:// address: run again through the tool, which gives a pty as
# T6_UART. Its stderr is read here to tell "busy" from other refusals.
[[ $T6_UART != rfc2217://* ]] || [[ $T6_UART =~ ^rfc2217://[^:/]+:[0-9]+$ ]] || refuse "T6_UART is not rfc2217://<host>:<port>"
if [[ $T6_UART == rfc2217://* ]]; then
	[ -z "${T6_UART_RFC2217:-}" ] || die "T6_UART is still an rfc2217:// address inside t6-uart-rfc2217.py"
	err=$(mktemp)
	trap 'rm -f "$err"' EXIT
	rc=0
	python3 -I "$K/t6-uart-rfc2217.py" run -- bash "$HERE/$(basename "$0")" 2>"$err" || rc=$?
	cat "$err" >&2
	if [ "$rc" = 40 ]; then
		if grep -q "busy" "$err"; then
			echo "console-check.sh: the adapter's one slot is busy (another session holds it)" >&2
			exit 4
		fi
		exit 40
	fi
	exit "$rc"
fi

[ -c "$T6_UART" ] && [ -r "$T6_UART" ] && [ -w "$T6_UART" ] ||
	refuse "T6_UART is not a character device this user can read and write, nor rfc2217://<host>:<port>"
if fuser "$T6_UART" >/dev/null 2>&1; then
	echo "console-check.sh: the line is busy: another process has the tty open (pids $(fuser "$T6_UART" 2>/dev/null | tr -s ' ' | cut -c1-80))" >&2
	exit 4
fi
# 1500000 raw, no echo (an echo would send every byte back to the console);
# read back by uart_ready, which refuses (3) when the line does not hold.
uart_ready

# One CR out, then READ_S seconds in. Prints "bytes=<n> nonprint=<m> prompt=<0|1>".
res=$(python3 -I - "$T6_UART" "$READ_S" <<'PY'
import os, re, select, sys, time
path, secs = sys.argv[1], float(sys.argv[2])
fd = os.open(path, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
os.write(fd, b"\r")
buf = b""
end = time.monotonic() + secs
# CSI sequences (colours in the prompt) are text; a prompt is a line that
# ends in # or $ (root@host:~#), or "login:".
csi = re.compile(rb"\x1b\[[0-9;?]*[ -/]*[@-~]")
prompt = re.compile(rb"(login: ?$|[#$] ?$)", re.M)
def seen(b):
    t = csi.sub(b"", b).replace(b"\r", b"\n")
    return any(prompt.search(l.rstrip(b" \t")) or l.rstrip().endswith(b"login:") for l in t.split(b"\n") if l.strip())
while time.monotonic() < end:
    r, _, _ = select.select([fd], [], [], 0.2)
    if r:
        try:
            c = os.read(fd, 4096)
        except (BlockingIOError, InterruptedError):
            continue
        except OSError:
            break
        buf += c
        if seen(buf) and len(buf) > 1:
            break
t = csi.sub(b"", buf)
bad = sum(1 for x in t if not (x in (9, 10, 13) or 32 <= x < 127))
print("bytes=%d nonprint=%d prompt=%d" % (len(buf), bad, 1 if seen(buf) and bad * 5 <= max(len(t), 1) else 0))
PY
) || die "the read failed"

bytes=${res#bytes=}; bytes=${bytes%% *}
nonprint=${res#*nonprint=}; nonprint=${nonprint%% *}
prompt=${res##*prompt=}

ssh_line() {
	local up
	if up=$(board 'cut -d" " -f1 /proc/uptime' 2>/dev/null) && [ -n "$up" ]; then
		echo "console-check.sh: ssh sees Armbian up ($up s): the board is alive, the console is the problem"
	else
		echo "console-check.sh: ssh does not answer either"
	fi
}

if [ "$prompt" = 1 ]; then
	echo "console-check.sh: ok: Armbian's prompt read as text at 1500000 ($bytes bytes, $nonprint non-printable)"
	exit 0
elif [ "$bytes" = 0 ]; then
	echo "console-check.sh: nothing received in ${READ_S} s after one CR"
	ssh_line
	exit 5
elif [ "$nonprint" -gt 0 ] && [ $((nonprint * 5)) -gt "$bytes" ]; then
	echo "console-check.sh: $bytes bytes received, $nonprint of them non-printable: not text; the line is probably not at 1500000"
	ssh_line
	exit 6
fi
echo "console-check.sh: $bytes bytes of text received, with no shell or login prompt in them"
ssh_line
exit 7
