# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# who_remote - the remote sessions on the machine this runs on, without the
# session it runs in. POSIX sh, no shebang: nanopc-t6.sh sends this text to
# the board ahead of its own commands, and board_lock_selftest.sh runs it on
# made-up `who -u` output.
#
# It prints "user@line(host) " for every `who -u` line on a pts or on
# "sshd", except the session this shell belongs to: the line whose pid is
# this shell or one of its ancestors.
#
# Why the exception: a board without utmp (Armbian trixie: coreutils 9.7
# `who` reads the sessions from systemd-logind) lists every ssh command as a
# session of its own, on line "sshd" with the pid of its sshd. The runner
# reads the board over ssh, so it listed its own command as a remote session
# and --go refused every run (2026-11-11, with nobody else on the board).
who_remote() {
	wr_p=$$
	wr_own=" "
	while [ "${wr_p:-0}" -gt 1 ]; do
		wr_own="$wr_own$wr_p "
		wr_p=$(ps -o ppid= -p "$wr_p" | tr -d ' ')
	done
	# who -u: user, line, date, time, idle, pid, then (host) when there is one.
	who -u | awk -v own="$wr_own" '
		$2 ~ /^(pts|sshd)/ {
			host = ($NF ~ /^\(/) ? $NF : "(no host)"
			pid = ($NF ~ /^\(/) ? $(NF - 1) : $NF
			if (index(own, " " pid " ") == 0) print $1 "@" $2 host
		}' | tr '\n' ' '
}
