# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# prompt-lib.sh - the parts prompt-reads.sh and prompt-go.sh share: the ssh
# command, the board facts both read, the UART set-up, and the run around
# the Python tool on the UART (start it, reboot the board, wait for it and
# for Armbian). Sourced, not run.
#
# The caller defines, before it calls these: BOARD_HOST, SSH_OPTS (array),
# refuse() and die(); for the run also T6_UART, T6_HAND, tmp, REC, rec(),
# old_boot, and starts its tool in the background with its output in
# $tmp/py.out and its pid in py_pid, and with $tmp/linux-back as the tool's
# last argument (the file reboot_and_wait_tool makes when Armbian is back).
# TOOL is the caller's name for its messages (prompt-reads or prompt-go).

# One ssh command to the board, 30 s at most, no stdin.
board() { timeout 30 ssh "${SSH_OPTS[@]}" -o BatchMode=yes -o ConnectTimeout=10 "$BOARD_HOST" "$@" </dev/null; }

# The facts both runners read (each adds its own lines). who_remote is
# defined ahead of them on the board (../kexec-zephyr/who_remote.sh): the
# remote sessions without this ssh command's own.
# shellcheck disable=SC2034  # used by the scripts that source this
T6_STATE_COMMON='
printf "compatible=%s\n" "$(tr "\0" " " </proc/device-tree/compatible)"
printf "uid=%s\n" "$(id -u)"
printf "uptime=%s\n" "$(cut -d" " -f1 /proc/uptime)"
printf "system=%s\n" "$(systemctl is-system-running 2>/dev/null || true)"
printf "boot_id=%s\n" "$(cat /proc/sys/kernel/random/boot_id)"
printf "kexec_loaded=%s\n" "$(cat /sys/kernel/kexec_loaded)"
printf "gpu_users=%s\n" "$(fuser /dev/dri/* 2>/dev/null | tr -s " " || true)"
printf "who_remote=%s\n" "$(who_remote)"'

# T6_UART: a character device this user can read and write, open by no
# other process, set raw with no echo before the tool opens it (with echo
# on, the tty would send every received byte back to the board;
# nanopc-t6.sh does the same), and read back.
uart_ready() {
	local uart_mode w
	[ -n "${T6_UART:-}" ] || refuse "--go needs T6_UART=<the host tty on the T6's debug UART, or rfc2217://<host>:<port>>"
	[ -c "$T6_UART" ] && [ -r "$T6_UART" ] && [ -w "$T6_UART" ] ||
		refuse "T6_UART $T6_UART is not a character device this user can read and write"
	if fuser "$T6_UART" >/dev/null 2>&1; then
		refuse "T6_UART $T6_UART is already open (pids $(fuser "$T6_UART" 2>/dev/null | tr -s " " | cut -c1-80)): a second reader would split the bytes; stop the other capture first"
	fi
	stty -F "$T6_UART" 1500000 raw -echo -icrnl -ixon cs8 -cstopb -parenb clocal || refuse "stty on $T6_UART failed"
	uart_mode=$(stty -F "$T6_UART" -a | tr "\n;" "  ") || refuse "stty -a on $T6_UART failed"
	for w in 'speed 1500000' ' -echo ' ' -icrnl ' ' -ixon ' ' cs8 ' ' -cstopb ' ' -parenb ' ' clocal ' ' -opost ' ' -icanon '; do
		[[ " $uart_mode " == *"$w"* ]] || refuse "$T6_UART does not read back '${w# }' after stty: $uart_mode"
	done
}

# uart_dry: the dry run's line for an RFC 2217 address in T6_UART (not
# opened in a dry run; the address is not printed).
uart_dry() {
	[[ ${T6_UART:-} == rfc2217://* ]] || return 0
	echo "  T6_UART is an RFC 2217 address (not opened in a dry run): --go would open it with"
	echo "  ../kexec-zephyr/t6-uart-rfc2217.py, set 1500000 8N1, and give $TOOL.py the pty it makes."
}

# uart_record: the record's line when T6_UART is t6-uart-rfc2217.py's pty.
uart_record() {
	[ -z "${T6_UART_RFC2217:-}" ] ||
		rec "uart: the debug UART over RFC 2217 (t6-uart-rfc2217.py set 1500000 8N1 and read it back), through its pty $T6_UART"
}

# Wait until the tool says it listens (10 s); otherwise the board is not
# rebooted: exit 3.
wait_listening() {
	local n=0
	until grep -q 'listening, nothing written' "$tmp/py.out" 2>/dev/null; do
		n=$((n + 1))
		if [ "$n" -gt 50 ] || ! kill -0 "$py_pid" 2>/dev/null; then
			sed 's/^/  /' "$tmp/py.out" >&2
			rec "$TOOL.py did not start listening: the board was not rebooted" >&2
			exit 3
		fi
		sleep 0.2
	done
	rec "$TOOL.py listens on the UART (pid $py_pid, nothing written)"
}

# Reboot the board over ssh and wait for the tool: sets t0 and py_rc, and
# copies the tool's lines into the record.
reboot_and_wait_tool() {
	local line now i
	t0=$(date +%s)
	rec "rebooting the board over ssh (systemctl reboot)"
	# The connection may drop before the command's status comes back.
	board 'systemctl reboot' >/dev/null 2>&1 || true
	# While the tool listens, ask the other witness of Linux every
	# BACK_POLL_S seconds (issue #11943). The UART's stream can stop in the
	# kernel's boot output (issue #11874); the tool would then wait its
	# whole time for a login it cannot read, minutes after Armbian is up.
	# A new boot_id over ssh makes $tmp/linux-back, and the tool ends such
	# a wait once the UART has also been silent for a while.
	while kill -0 "$py_pid" 2>/dev/null; do
		now=$(board 'cat /proc/sys/kernel/random/boot_id' 2>/dev/null || true)
		if [ -n "$now" ] && [ "$now" != "$old_boot" ]; then
			: >"$tmp/linux-back"
			rec "Armbian answers over ssh with a new boot_id $(($(date +%s) - t0)) s after the reboot command; $TOOL.py is told, and ends a wait for the UART once that is silent"
			break
		fi
		for ((i = 0; i < ${BACK_POLL_S:-5}; i++)); do
			kill -0 "$py_pid" 2>/dev/null || break
			sleep 1
		done
	done
	py_rc=0
	wait "$py_pid" || py_rc=$?
	py_pid=
	while IFS= read -r line; do rec "$line"; done <"$tmp/py.out"
}

# Whatever the tool's result, the board is looked for: a command that made
# U-Boot reset the board, or an image that reset it, ends in Armbian too.
#
# exit_when_back [<code>:<code when back> ...]: the tool's codes that say
# "no Linux login on the UART" (prompt-go 6 and 10, prompt-reads 6), each
# with the code the tool gives when the login came. The UART is not the
# only witness of Linux: the network serial adapter can stop its stream for
# good in the kernel's boot output, and a login printed then is never read
# (issue #11874). So Armbian with a new boot_id over ssh is Linux back all
# the same, and the exit is the second code; the record says so, and the
# tool's last line in uart.log says where its stream went silent.
#
# Exits with the tool's code (or the second code above) when Armbian is
# back with a new boot_id. When it is not back within BACK_WAIT_S (default
# 180; the self-test sets 0): 5 when the tool stopped at the prompt; the
# tool's own code when that is one of the codes given (neither the UART nor
# ssh shows Linux: the board needs T6_HAND); otherwise 1.
exit_when_back() {
	local deadline now_boot= code=$py_rc pair
	deadline=$(( $(date +%s) + ${BACK_WAIT_S:-180} ))
	while [ "$(date +%s)" -lt "$deadline" ]; do
		now_boot=$(board 'cat /proc/sys/kernel/random/boot_id' 2>/dev/null || true)
		if [ -n "$now_boot" ] && [ "$now_boot" != "$old_boot" ]; then
			rec "back in Armbian $(( $(date +%s) - t0 )) s after the reboot command (boot_id $now_boot, up $(board 'cut -d" " -f1 /proc/uptime') s)"
			for pair in "$@"; do
				[ "${pair%%:*}" = "$py_rc" ] || continue
				code=${pair#*:}
				rec "$TOOL.py saw no Linux login on the UART (exit $py_rc), but Armbian answers over ssh with a new boot_id: Linux is back; exit $code, as with the login seen (uart.log's last line says where the UART went silent)"
			done
			echo "$TOOL.sh: record $REC"
			exit "$code"
		fi
		sleep 5
	done
	if [ -n "$now_boot" ]; then
		rec "the same Linux still runs (boot_id unchanged): the reboot did not happen" >&2
	elif [ "$py_rc" = 5 ]; then
		rec "the board is at its U-Boot prompt and stays there: it needs $T6_HAND at its power switch" >&2
		echo "$TOOL.sh: record $REC"
		exit 5
	else
		for pair in "$@"; do
			[ "${pair%%:*}" = "$py_rc" ] || continue
			rec "no Linux login on the UART (exit $py_rc) and no Armbian over ssh $(( $(date +%s) - t0 )) s after the reboot command: no sign of Linux; the board needs $T6_HAND at its power switch" >&2
			echo "$TOOL.sh: record $REC"
			exit "$py_rc"
		done
		rec "no Armbian $(( $(date +%s) - t0 )) s after the reboot command: the board needs $T6_HAND at its power switch" >&2
	fi
	echo "$TOOL.sh: record $REC"
	exit 1
}
