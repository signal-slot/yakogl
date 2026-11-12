# board_lock.sh - take the shared board lock, bounded at both ends.
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# SOURCE this, do not run it. Sourcing acquires the lock, so it replaces the
# two lines every kexec runner used to carry:
#
#   -exec 9>"$LOCK"
#   -flock 9
#   +. "$REPO/tools/kexec-zephyr/board_lock.sh"
#
# and the runner keeps its own $LOCK, which is picked up as the default.
#
# WHY BOTH ENDS ARE BOUNDED. A shared board produces two different failures,
# and a plain `flock 9` prevents neither:
#
#   Waiting for ever. Eight runs queue behind one board; each takes its turn
#   long after the session that started it stopped caring, and kexecs a board
#   somebody else is now using. Measured 2026-09-21: eight processes queued on
#   one lock, the oldest waiting 25 minutes.
#
#   Holding for ever. One run wedges -- an ssh that never returns, a board
#   that never comes back -- and every other session stops. Measured the same
#   night: one runner held the lock 30 minutes despite a 560 s cap on its own
#   wait loop, which means the cap did not fire.
#
# So: give up waiting after LOCK_WAIT rather than queueing, and once the lock
# IS held, a watchdog takes the whole script down after HOLD_MAX however
# wedged it is. The second is the one that matters to everybody else.
#
# Environment, all optional:
#   LOCK / BOARD_LOCK  the lock file. LOCK is read too, because that is what
#                      the existing runners already call it; BOARD_LOCK wins
#                      when both are set. Default: one lock per board, named
#                      from BOARD_HOST (below).
#   BOARD_HOST         user@host of the board; the default lock is
#                      board-<first 12 hex digits of sha256(BOARD_HOST)>.lock.
#                      Required when neither LOCK nor BOARD_LOCK is set.
#   LOCK_WAIT          seconds to wait for the lock before giving up (2700).
#                      0 means do not wait at all.
#   HOLD_MAX           seconds the run may hold the lock before the watchdog
#                      kills it (900). 0 disables the watchdog, which should
#                      be a deliberate act and not a default.
#
#                      900 suits a runner that builds BEFORE taking the lock
#                      and then only kexecs. A runner that builds while
#                      holding it needs more: the gl_test runners take the
#                      lock first, and a regression set at DEADLINE=400 holds
#                      for 90 s of build plus 400 + 70 + 6*code of sample,
#                      approaching 850 s with nothing wrong. Set it from what
#                      YOUR runner does while holding the lock:
#                          HOLD_MAX=${HOLD_MAX:-1800}
#                          . .../board_lock.sh
#                      The bound is the point, not the number.
#   BOARD_LOCK_NO_ACQUIRE  set to 1 to define the helpers without taking the
#                      lock, for a script that only reads the board.
#
# It also defines three helpers, because the calls they wrap are exactly the
# ones that turn a vanishing board into a wedge: bssh, bscp and
# board_lock_holder.
#
# Deliberately does NOT set -e or -u: this is sourced, and changing the
# caller's shell options from underneath it would be its own kind of bug.

# The lock lives outside the repository on purpose: sessions from several
# checkouts and worktrees drive the one board, and they have to meet on a path
# none of them owns. It is a zero-length file used only for flock(2).
#
# One lock per board: the default name is keyed on BOARD_HOST, so runs on two
# boards do not wait for each other, and two runs on one board meet on one
# file. The name is a hash, not the address, so no address ends up in a file
# name or a log. The key is the BOARD_HOST string exactly as the runner has
# it: two spellings of one board (an alias and an address) are two locks, so
# keep one spelling per board. (Until 2026-10-03 every board shared one file,
# board79.lock, named after one board's address.)
#
# The directory is /run/user/<uid>/yakogl-locks. Every session of this user
# on this host gets the same path, whichever checkout, worktree or agent
# session it runs from. It is the user's runtime directory (tmpfs, mode 700),
# which systemd keeps while the user has any login session, so it outlives
# any one session. The path is built from the uid, not from
# $XDG_RUNTIME_DIR, so that an environment that sets that variable
# differently cannot split the lock. There is no fallback: a lock in a
# different place is no lock, so a directory that cannot be made is a
# refusal. (Until 2026-10-03 the directory was one agent session's
# scratchpad under /tmp.)
BOARD_LOCK_DIR=/run/user/$(id -u)/yakogl-locks
if [ -z "${BOARD_LOCK:-}" ] && [ -z "${LOCK:-}" ]; then
	if [ -z "${BOARD_HOST:-}" ]; then
		echo "board_lock: set BOARD_HOST (the lock is per board) or LOCK" >&2
		if [ "${BOARD_LOCK_NO_ACQUIRE:-0}" = 1 ]; then return 1; fi
		exit 1
	fi
	if ! mkdir -p -m 700 "$BOARD_LOCK_DIR" 2>/dev/null || [ ! -w "$BOARD_LOCK_DIR" ]; then
		echo "board_lock: cannot use $BOARD_LOCK_DIR (no runtime directory for uid $(id -u)?)" >&2
		if [ "${BOARD_LOCK_NO_ACQUIRE:-0}" = 1 ]; then return 1; fi
		exit 1
	fi
	BOARD_LOCK=$BOARD_LOCK_DIR/board-$(printf '%s' "$BOARD_HOST" | sha256sum | cut -c1-12).lock
fi
BOARD_LOCK=${BOARD_LOCK:-$LOCK}
LOCK_WAIT=${LOCK_WAIT:-2700}
HOLD_MAX=${HOLD_MAX:-900}

# ssh and scp to the board, bounded. Every call these wrap has hung in
# practice: ConnectTimeout covers a board that is already gone, and `timeout`
# covers one that leaves mid-call, which ConnectTimeout does not.
#
#   bssh [-t SECONDS] <ssh args...>      default 20 s
#   bscp [-t SECONDS] <scp args...>      default 180 s
bssh() {
	_t=20
	if [ "$1" = -t ]; then _t=$2; shift 2; fi
	timeout "$_t" ssh -o BatchMode=yes -o ConnectTimeout=8 \
		-o ServerAliveInterval=5 -o ServerAliveCountMax=3 "$@"
}
bscp() {
	_t=180
	if [ "$1" = -t ]; then _t=$2; shift 2; fi
	timeout "$_t" scp -o BatchMode=yes -o ConnectTimeout=10 \
		-o ServerAliveInterval=5 -o ServerAliveCountMax=3 "$@"
}

# Who holds the lock: the holder writes a record of itself into the lock file
# right after taking it (`board_lock_record`), and a refused taker reads it
# back. Until 2026-10-19 the holder was found with lsof, which a gate host
# may not have (runner A has none: the self-test could not run there) and which
# took about 6 s a refusal.
#
# The record: the holder's pid, its start time in clock ticks since boot
# (/proc/<pid>/stat field 22: a pid the kernel has given to another process
# since does not match), the start time as a date, and its command line with
# the directories of every path word dropped, so a script run by its
# absolute path is named by its script (2026-10-19: the cut line read
# "/bin/bash /home/.../tools/mali/panthor/").

# The start time of pid $1 in clock ticks since boot, or nothing.
board_lock_start_ticks() {
	_stat=$(cat "/proc/$1/stat" 2>/dev/null) || return 0
	# field 22; the comm in field 2 is in parentheses and may hold spaces
	printf '%s' "${_stat##*) }" | awk '{ print $20 }'
}

# The command line of pid $1, directories dropped, at most 100 characters.
board_lock_cmdline() {
	tr '\0' ' ' < "/proc/$1/cmdline" 2>/dev/null | sed -E 's#(^| )[./~][^ ]*/#\1#g; s/ $//' | cut -c1-100
}

# Write this shell's record into the lock file (the lock is held).
board_lock_record() {
	{
		echo "pid=$$"
		echo "ticks=$(board_lock_start_ticks $$)"
		echo "started=$(date '+%F %T')"
		echo "cmd=$(board_lock_cmdline $$)"
	} > "$BOARD_LOCK"
}

# A field of the record in the lock file.
board_lock_field() {
	sed -n "s/^$1=//p" "$BOARD_LOCK" 2>/dev/null | head -1
}

# Whether pid $1 is still the process that started at $2 clock ticks. A
# pid the kernel has given to another process since has other ticks; empty
# ticks match nothing.
board_lock_same_process() {
	[ -n "$2" ] && [ "$(board_lock_start_ticks "$1")" = "$2" ]
}

# Whether the record's process is alive: its pid runs with the start time
# the record holds.
board_lock_record_alive() {
	_rp=$(board_lock_field pid)
	[ -n "$_rp" ] && board_lock_same_process "$_rp" "$(board_lock_field ticks)"
}

# The watchdog: board_lock_watchdog <pid> <ticks> <seconds>. It looks at the
# runner (pid AND start ticks) once a second and ends as soon as the runner
# is gone, however it ended: a runner killed with SIGKILL runs no EXIT trap
# to stop it. If the runner still holds the board after <seconds>, it is
# sent TERM, and KILL 10 s later if it is still there.
#
# Until 2026-11-12 (issue #4499) this was one `sleep HOLD_MAX` and then
# `kill -0 $$`. After a run that ended normally, the EXIT trap killed the
# subshell and its `sleep HOLD_MAX` ran on to the end (55 sleeps of 1800
# and 21600 s on one host; now it is a `sleep 1`). After a SIGKILLed run
# the watchdog itself stayed for the rest of HOLD_MAX, and then killed
# whatever process had the dead runner's pid by then (the pid counter
# went round its whole range in about six minutes during gate runs). The
# ticks check makes the kill go only to the runner; between the last
# check and the kill there is no guard, a window of one fork rather than
# of HOLD_MAX.
board_lock_watchdog() {
	_wd_end=$(( $(date +%s) + $3 ))
	while board_lock_same_process "$1" "$2"; do
		if [ "$(date +%s)" -ge "$_wd_end" ]; then
			echo "board_lock: $(date '+%F %T') pid $1 held the board past ${3}s; killing the run" >&2
			echo "board_lock: if that was a working run, HOLD_MAX is too low for what" >&2
			echo "board_lock: this runner does while holding the lock -- set it higher" >&2
			echo "board_lock: before sourcing, e.g. HOLD_MAX=\${HOLD_MAX:-1800}" >&2
			kill -TERM "$1" 2>/dev/null
			_wd_n=0
			while [ "$_wd_n" -lt 10 ] && board_lock_same_process "$1" "$2"; do
				sleep 1
				_wd_n=$((_wd_n + 1))
			done
			if board_lock_same_process "$1" "$2"; then
				kill -KILL "$1" 2>/dev/null
			fi
			return 0
		fi
		sleep 1
	done
	return 0
}

# Name the holder (for a refusal): the record's process when it lives;
# when it is gone, the processes of this user that still have the lock file
# open (a child the holder left with its descriptor), found in /proc.
board_lock_holder() {
	_rp=$(board_lock_field pid)
	if [ -z "$_rp" ]; then
		echo "  no record in the lock file (its holder has ended, or took the lock before records were written); the processes that have it open:"
	elif board_lock_record_alive; then
		printf '  pid %s  started %s\n    %s\n' "$_rp" "$(board_lock_field started)" "$(board_lock_field cmd)"
		return
	else
		echo "  stale record: pid $_rp ($(board_lock_field cmd)) is gone; a process it left still holds the lock:"
	fi
	_found=0
	_lock=$(readlink -f "$BOARD_LOCK")
	for _fd in /proc/[0-9]*/fd/*; do
		[ "$(readlink "$_fd" 2>/dev/null)" = "$_lock" ] || continue
		_p=${_fd#/proc/}
		_p=${_p%%/*}
		[ "$_p" = "$$" ] && continue
		printf '  pid %s\n    %s\n' "$_p" "$(board_lock_cmdline "$_p")"
		_found=1
	done
	[ "$_found" = 1 ] || echo "  (no process of this user has it open; whoever held it has just let go)"
}

if [ "${BOARD_LOCK_NO_ACQUIRE:-0}" != 1 ]; then
	mkdir -p "$(dirname "$BOARD_LOCK")" 2>/dev/null
	# append, never truncate: a taker that is refused must not erase the
	# holder's record
	exec 9>>"$BOARD_LOCK"
	if [ "$LOCK_WAIT" -eq 0 ]; then
		_flock_args=-n
	else
		_flock_args="-w $LOCK_WAIT"
	fi
	# shellcheck disable=SC2086  # deliberately word-split
	if ! flock $_flock_args 9; then
		echo "board_lock: busy for ${LOCK_WAIT}s; giving up rather than queueing. The holder:" >&2
		board_lock_holder >&2
		exit 1
	fi
	if [ -n "$(board_lock_field pid)" ] && ! board_lock_record_alive; then
		echo "board_lock: stale record, pid $(board_lock_field pid) gone ($(board_lock_field cmd)); taking the lock" >&2
	fi
	board_lock_record

	if [ "$HOLD_MAX" -gt 0 ]; then
		# $$ stays the sourcing shell inside the subshell, which is
		# what this has to watch and kill: the runner, not the watchdog.
		#
		# `exec 9>&-` FIRST, and it is not optional. The subshell
		# inherits fd 9 from the `exec 9>` above, and so does the
		# `sleep` it forks. An flock is released only when every
		# duplicated descriptor is closed -- so when the EXIT trap
		# kills the subshell, the orphaned `sleep` goes on holding the
		# board for the REST of HOLD_MAX, long after the run it was
		# guarding has finished. With HOLD_MAX=1800 that is a
		# half-hour dead hold after every single run.
		#
		# Closing the watchdog's copy does not release the parent's
		# lock -- they share one open file description, and the parent
		# still has its own fd -- so a run holds the board exactly as
		# long as it did before. Only the orphan loses the ability to.
		#
		# (Since 2026-11-12 the watchdog sleeps one second at a time and
		# ends with its runner, which shortens such a hold to about a
		# second; the fd is closed all the same.) Killing the `sleep` as
		# well as the subshell in the trap would also work, but this
		# holds even when the trap does not run at all: a SIGKILLed run,
		# or one that dies before the trap is installed. Found by
		# vivante-gltest2, 2026-09-22, with sixteen runs queued behind a
		# lock no live run held.
		#
		# The caller's stdin, stdout and stderr go too, for the same
		# reason. The subshell and its `sleep` inherit them, so a caller
		# whose output is piped (`runner | tail`) saw the pipe stay open
		# after the run ended, until HOLD_MAX had passed: 60 s for a run
		# of 0.01 s with HOLD_MAX=60, ten minutes a run with the 600 of
		# tools/mali/mesa-board-ref.sh (2026-10-19). The lock was released
		# at once; only the caller's pipe waited. A watchdog that fires
		# writes its lines to <lock>.watchdog.log beside the lock (the
		# lock's own hashed name, so no address in it), emptied when the
		# lock is taken; the run's EXIT trap prints them to the run's own
		# stderr, so a killed run still says why.
		#
		# The watchdog names the runner by its pid and start ticks
		# (board_lock_watchdog above): it ends within a second or two of
		# the runner, so after a SIGKILLed run nothing is left but a
		# `sleep 1` about to end, and it never kills a process that got
		# the runner's pid. Without the ticks (no /proc) it could tell
		# neither, so the run is refused rather than left unbounded.
		_wd_ticks=$(board_lock_start_ticks $$)
		if [ -z "$_wd_ticks" ]; then
			echo "board_lock: no start time for pid $$ in /proc: the watchdog could not tell its run; refusing" >&2
			: > "$BOARD_LOCK"
			exit 1
		fi
		_wd_log=$BOARD_LOCK.watchdog.log
		: > "$_wd_log"
		( exec 9>&- </dev/null >/dev/null 2>>"$_wd_log"
		  board_lock_watchdog $$ "$_wd_ticks" "$HOLD_MAX" ) &
		BOARD_LOCK_WATCHDOG=$!
		# Keep whatever the caller already had on EXIT: a runner that
		# cleans up after itself must not lose that by sourcing this.
		_prev_exit_trap=$(trap -p EXIT 2>/dev/null |
			sed "s/^trap -- '//; s/' EXIT\$//")
		trap ": > \"\$BOARD_LOCK\"; kill \"\$BOARD_LOCK_WATCHDOG\" 2>/dev/null; [ -s \"\$_wd_log\" ] && cat \"\$_wd_log\" >&2; ${_prev_exit_trap}" EXIT
	else
		# no watchdog: the record still goes when the holder ends
		_prev_exit_trap=$(trap -p EXIT 2>/dev/null |
			sed "s/^trap -- '//; s/' EXIT\$//")
		trap ": > \"\$BOARD_LOCK\"; ${_prev_exit_trap}" EXIT
	fi
fi
