#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board_lock.sh without a board: a second taker refuses at once and its
# refusal names the holder by its script, and nanopc-t6.sh never waits for
# the lock.
#
# On 2026-10-19 a kexec jump rebooted the T6 under a gl_test session: the
# board had been handed over by chat, and nothing refused. Two things made
# the lock no help there, and this pins both:
#
#  - The holder line cut the holder's command at 100 characters, and a
#    script run by its absolute path filled them with directories: the
#    refusal read "/bin/bash /home/.../tools/mali/panthor/", never the
#    script's name. board_lock_holder now drops the directories of every
#    path word before the cut (the pid stays).
#  - nanopc-t6.sh took board_lock.sh's default LOCK_WAIT, 2700 s: a jump
#    started while a GL session held the board waited, then jumped onto
#    whatever session held the board by then. It now refuses at once
#    (LOCK_WAIT 0) unless the caller sets LOCK_WAIT. The recipe takes the
#    lock only after it has read the board over ssh, so its refusal cannot
#    be run without a board: the check is on the value it sources, which
#    the timed refusal below then shows the effect of.
#
# A third, found the same day: the watchdog's subshell and its `sleep`
# inherited the caller's stdout and stderr, so a runner whose output was
# piped (`runner | tail`) kept the pipe open until HOLD_MAX had passed
# after the run ended (60 s for a 0.01 s run with HOLD_MAX=60; ten minutes
# a run of tools/mali/mesa-board-ref.sh). The watchdog now leaves the
# caller's stdio, and a watchdog that fires still says so on the run's
# stderr (through <lock>.watchdog.log and the EXIT trap).
#
# A fourth: the holder was found with lsof, which a gate host may lack (runner A
# has none, and the self-test stopped there). The holder now writes a record
# of itself into the lock file and a refusal reads it; a record whose
# process is gone is named stale. Nothing here or in board_lock.sh runs lsof.
#
# A fifth, found on 2026-10-19 in a CI log ("Terminate orphan process ...
# run-on-board.sh ... sleep"): the test left four processes behind it, the
# holder's `sleep 60` and its watchdog's, the stuck run's `sleep 30` and the
# fired watchdog's `sleep 10`. Killing the holder's children missed the
# grandchildren, and the runs that had ended left theirs reparented, out of
# any tree. Every process the test starts now carries its own marker in the
# environment; the end sweeps the pids that carry it, found under /proc
# (never by name or process group), and checks that none is left.
#
# A sixth, found on 2026-11-11 when the T6 had no utmp (Armbian trixie:
# `who` lists the sessions of systemd-logind): every ssh command was a
# session on line "sshd", so nanopc-t6.sh listed the command it read the
# board with as a remote session and --go refused every run. The session
# the check runs in is now left out (who_remote.sh); any other remote
# session is still named.
#
# A seventh, found on 2026-11-12 with the first t6_fw image (434272 bytes,
# the firmware in it): nanopc-t6.sh refused it as lacking the "board:
# watchdog armed" line, which it carries. The check was `strings | grep -q`
# under `set -o pipefail`: grep left at the match, strings was still
# writing and died of SIGPIPE, and the pipeline's status was 141, read as
# "no such line". The smaller t6_mmu image had passed because strings had
# finished before grep left. The runner now reads the image's strings once
# and matches them from a here-string; the same shape on /proc/iomem went
# with it. The check here is an image whose line is followed by 4 MiB of
# text: it must get past that check (to the refusal of its TOP 9), and an
# image without the line must still be refused for it.
#
# Needs bash, flock(1) and strings(1). tools/check.sh runs it ("tools   board_lock.sh: ...").
set -u
here=$(cd "$(dirname "$0")" && pwd)
planned=15
run=0
failed=0
check() { # <what> <0 = ok>
	run=$((run + 1))
	if [ "$2" = 0 ]; then
		echo "$1 ok"
	else
		echo "$1 FAIL"
		failed=$((failed + 1))
	fi
}
for t in flock strings; do
	command -v "$t" >/dev/null || { echo "board_lock_selftest: $t is missing: the lock and the runner's image checks cannot be tested"; exit 1; }
done

tmp=$(mktemp -d)
# Every process started below inherits this; no other process has it.
export BOARD_LOCK_SELFTEST_RUN=$tmp
# marked: the pids under /proc whose environment carries the marker, this
# shell excepted (the command substitution's subshell has it too, and is
# excepted by reading BASHPID). Builtins only: a tr or grep started here
# would carry the marker and could find itself.
marked() {
	local self=$BASHPID e p v
	local -a env
	for e in /proc/[0-9]*/environ; do
		p=${e#/proc/}
		p=${p%/environ}
		{ [ "$p" = "$$" ] || [ "$p" = "$self" ]; } && continue
		{ mapfile -d '' env <"$e"; } 2>/dev/null || continue
		for v in "${env[@]}"; do
			[ "$v" = "BOARD_LOCK_SELFTEST_RUN=$tmp" ] && { echo "$p"; break; }
		done
	done
}
# sweep: TERM each marked pid, then wait up to 5 s for them to go.
sweep() {
	local p
	for p in $(marked); do kill "$p" 2>/dev/null; done
	for _ in $(seq 50); do [ -z "$(marked)" ] && return 0; sleep 0.1; done
	return 1
}
trap 'sweep; rm -rf "$tmp"' EXIT

# A holder run by an absolute path longer than the holder line's 100
# characters, named as the two runners are.
deep=$tmp/a-directory-long-enough/that-the-holder-line/is-cut-before/the-script-name/of/the/holder
mkdir -p "$deep"
cat > "$deep/run-on-board.sh" <<EOF
#!/bin/bash
LOCK_WAIT=0 HOLD_MAX=60
. "$here/board_lock.sh"
: > "$tmp/held"
sleep 60
EOF
chmod +x "$deep/run-on-board.sh"
BOARD_LOCK=$tmp/board.lock "$deep/run-on-board.sh" panthor-gltest --set &
holder_pid=$!
for _ in $(seq 100); do [ -e "$tmp/held" ] && break; sleep 0.1; done
check "the holder holds the lock" "$([ -e "$tmp/held" ]; echo $?)"

# The second taker, as nanopc-t6.sh takes it now (LOCK_WAIT 0).
t0=$(date +%s)
err=$(BOARD_LOCK=$tmp/board.lock LOCK_WAIT=0 timeout 60 bash -c ". '$here/board_lock.sh'; echo went-past" 2>&1)
rc=$?
t1=$(date +%s)
check "the second taker refuses (exit 1, rc $rc)" "$([ "$rc" = 1 ] && ! grep -q went-past <<<"$err"; echo $?)"
# the record is read, nothing is searched: 5 s (lsof took about 6 s; the
# wait this replaces was 2700 s)
check "it refuses within 5 s ($((t1 - t0)) s), not after a wait" "$([ $((t1 - t0)) -le 5 ]; echo $?)"
check "the refusal names the holder's pid $holder_pid" "$(grep -q "pid $holder_pid " <<<"$err"; echo $?)"
check "the refusal names the holder's script, not its directories" \
	"$(grep -q "run-on-board.sh panthor-gltest --set" <<<"$err"; echo $?)"
[ "$failed" = 0 ] || printf '%s\n' "$err"

# nanopc-t6.sh sources board_lock.sh with LOCK_WAIT 0 unless the caller set
# one: the default is assigned before the line that sources the file, and
# not changed between them.
t6=$here/nanopc-t6.sh
src=$(grep -n '^\. "\$HERE/board_lock.sh"$' "$t6" | cut -d: -f1)
def=$(grep -n '^LOCK_WAIT=\${LOCK_WAIT:-0}$' "$t6" | cut -d: -f1)
later=$(awk -v a="${def:-0}" -v b="${src:-0}" 'NR > a && NR < b && /LOCK_WAIT=/' "$t6")
check "nanopc-t6.sh: LOCK_WAIT defaults to 0 before it sources board_lock.sh (lines ${def:-none} < ${src:-none})" \
	"$([ -n "$src" ] && [ -n "$def" ] && [ "$def" -lt "$src" ] && [ -z "$later" ]; echo $?)"

# A holder whose output is piped returns when it ends, not after HOLD_MAX.
cat > "$tmp/piped.sh" <<EOF
#!/bin/bash
LOCK_WAIT=0 HOLD_MAX=60
. "$here/board_lock.sh"
echo held
EOF
chmod +x "$tmp/piped.sh"
t0=$(date +%s)
out=$(BOARD_LOCK=$tmp/piped.lock timeout 90 "$tmp/piped.sh" | cat)
t1=$(date +%s)
check "a piped holder returns within 5 s of its end, not after HOLD_MAX 60 ($((t1 - t0)) s)" \
	"$([ "$out" = held ] && [ $((t1 - t0)) -le 5 ]; echo $?)"

# A watchdog that fires kills the run and says so on the run's stderr.
cat > "$tmp/stuck.sh" <<EOF
#!/bin/bash
LOCK_WAIT=0 HOLD_MAX=2
. "$here/board_lock.sh"
sleep 30 </dev/null >/dev/null 2>&1
EOF
chmod +x "$tmp/stuck.sh"
t0=$(date +%s)
err=$(BOARD_LOCK=$tmp/stuck.lock timeout 60 "$tmp/stuck.sh" 2>&1 >/dev/null </dev/null)
t1=$(date +%s)
check "a watchdog that fires ends the run within 15 s and names HOLD_MAX on its stderr ($((t1 - t0)) s)" \
	"$([ $((t1 - t0)) -le 15 ] && grep -q 'held the board past 2s' <<<"$err"; echo $?)"

# A record left by a holder that is gone (killed before its EXIT trap ran):
# the next taker says so and takes the lock.
bash -c 'exit 0' &
dead=$!
wait "$dead"
printf 'pid=%s\nticks=1\nstarted=2026-10-19 00:00:00\ncmd=bash run-on-board.sh gone\n' "$dead" > "$tmp/stale.lock"
err=$(BOARD_LOCK=$tmp/stale.lock LOCK_WAIT=0 HOLD_MAX=0 timeout 30 bash -c ". '$here/board_lock.sh'; echo went-past" 2>&1)
check "a stale record (pid $dead gone) is named and the lock is taken" \
	"$(grep -q "stale record, pid $dead gone" <<<"$err" && grep -q went-past <<<"$err"; echo $?)"

# who_remote.sh, as nanopc-t6.sh runs it on the board, on made-up `who -u`
# output. The session it runs in is the line that carries the pid of one of
# its ancestors (this shell here, the ssh command's sshd on the board); the
# other session's pid belongs to a process that is gone.
bash -c 'exit 0' &
other=$!
wait "$other"
console="root     ttyS2        2026-11-09 15:00  old          829
root     seat0        2026-11-09 15:00   ?           828
root     tty1         2026-11-09 15:00  old          828"
own_line="root     sshd         2026-11-11 04:07   ?       $$ (203.0.113.7)"
who_remote_on() { # <the who -u output>
	WR_WHO=$1 sh -c 'who() { printf "%s\n" "$WR_WHO"; }; . "$0"; who_remote' "$here/who_remote.sh"
}
got=$(who_remote_on "$own_line
$console")
check "who_remote: its own ssh command and the console logins are no remote session ('$got')" \
	"$([ -z "$got" ]; echo $?)"
got=$(who_remote_on "$own_line
user     sshd         2026-11-11 04:01   ?       $other (203.0.113.9)
$console")
check "who_remote: another ssh command beside its own is named ('$got')" \
	"$([ "$got" = "user@sshd(203.0.113.9) " ]; echo $?)"
got=$(who_remote_on "user     pts/0        2026-11-11 04:01   .       $other (203.0.113.9)
$console")
check "who_remote: a login on a pts is named ('$got')" \
	"$([ "$got" = "user@pts/0(203.0.113.9) " ]; echo $?)"

# nanopc-t6.sh's image checks, which run on this host before anything
# reaches the board (BOARD_HOST is never contacted: both images are refused
# first). t6_image <file> <line after the marker>: an AArch64 Image header,
# a marker with TOP 9 (outside 11..15), the line, then 4 MiB of text.
t6_image() {
	{
		head -c 56 /dev/zero
		printf 'ARM\x64'
		head -c 4 /dev/zero
		printf 'yakogl-wdt: nanopc_t6 WDT0 armed at boot; top 9; lifetime_s 60; alive_s 5; proof 0; load 0x10000000\0'
		printf '%s\0' "$2"
		yes 'text after the line, longer than twenty characters' | head -c $((4 << 20))
	} > "$1"
}
t6_image "$tmp/armed.bin" 'board: watchdog armed'
t6_image "$tmp/unarmed.bin" 'board: watchdog is not armed'
err=$(BOARD_HOST=root@board.invalid bash "$t6" "$tmp/armed.bin" 2>&1 >/dev/null)
rc=$?
check "nanopc-t6.sh: an image whose 'board: watchdog armed' line is followed by 4 MiB of text gets past that check (rc $rc: $(head -n 1 <<<"$err"))" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: WDT0 TOP 9 outside 11..15' <<<"$err"; echo $?)"
err=$(BOARD_HOST=root@board.invalid bash "$t6" "$tmp/unarmed.bin" 2>&1 >/dev/null)
rc=$?
check "nanopc-t6.sh: an image without the line is refused for it (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF "lacks the 'board: watchdog armed' line" <<<"$err"; echo $?)"

# The test leaves no process behind: the sweep ends every marked process,
# and a known one (the holder, still running here) shows that the search
# finds what it looks for.
found=$(marked | grep -cx "$holder_pid")
sweep
left=$(marked)
left=${left//$'\n'/ }
check "no process of the test is left (the holder found by the search: $found; left: ${left:-none})" \
	"$([ "$found" = 1 ] && [ -z "$left" ]; echo $?)"

echo "board_lock_selftest: $run checks run (planned $planned), $failed failed"
[ "$failed" = 0 ] && [ "$run" = "$planned" ]
