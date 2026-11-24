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
# An eighth, found on 2026-11-12 (issue #4499): the watchdog was one
# `sleep HOLD_MAX` and then `kill -0 $$`. A runner killed with SIGKILL
# never ran the EXIT trap that stops the watchdog, so the watchdog and its
# `sleep 1800` stayed for the rest of HOLD_MAX (six pairs after six gate
# runs), and when the sleep ended it killed whatever process had the dead
# runner's pid by then. The watchdog now looks at its runner every second
# and ends when the runner is gone, and it names its runner by pid AND start
# time (board_lock_watchdog <pid> <ticks> <seconds>), so a pid given to
# another process is never killed. The checks: a runner killed with SIGKILL
# leaves no process of its watchdog after 5 s, and a watchdog given a live
# pid with other start ticks ends without killing it, while one given the
# right ticks kills it after its seconds. A normal exit left a process too:
# the EXIT trap killed the watchdog's subshell, and its `sleep HOLD_MAX`
# went on to the end (55 such sleeps of 1800 and 21600 s on one host). Now
# what is left is a `sleep 1`, so a holder that returns leaves no process
# of its watchdog after 5 s either.
#
# A ninth, found on 2026-11-12 in a review of the T6 GL images: when Linux's
# log named no GIC ITS/LPI table (its ring had wrapped), nanopc-t6.sh printed
# "not checked" and --go went on, though a table in the window would be
# overwritten by the image. It now refuses. The checks run nanopc-t6.sh on
# made-up board facts (an `ssh` that prints them): the 256 MiB window on the
# T6's /proc/iomem, an entry and a GIC table in the window's last part, and
# --go with and without the tables named.
#
# The T6 display dump (t6-display-dump.sh) takes this lock too and reads
# registers of the running Linux. Its own self-test, on a made-up board, runs
# from here as one check: the read list stays inside its windows, a display
# that is off or a held lock refuses before any read, and a read that fails
# stops the run. The crash route's interrupt steps in kexec_zephyr
# (crash_irq.h: the PMIC's power key unbound, /proc/interrupts checked, the
# power key bound again on a refusal) have a self-test of their own too,
# crash_irq_selftest.sh, run from here as one check the same way.
#
# A tenth, for the crash route (nanopc-t6.sh --crash, D2 of
# driver/docs/nanopc-t6-display.md): on made-up facts, --crash refuses
# without a crashkernel= reservation that holds the window, with one that
# holds only part of it, with a crash image already loaded, with another
# entry in the window, and with an image whose segment leaves no room for the
# kernel's control page; with the reservation it passes the dry run and
# reaches its T6_HAND refusal; the normal route says "route: normal", jumps
# without --crash, and is refused, naming --crash, when the reservation lies
# in the window. t6-crashkernel.sh, the tool that makes the reservation, runs
# against a fake board (a directory, T6CK_ROOT, and an `ssh` that runs the
# command here): status before and after, the set/undo round trip byte for
# byte on a file with an extraargs= line and on one without, and its
# refusals (a crashkernel= already there, two extraargs= lines, undo without
# set, a file changed after set, another board).
#
# An eleventh, for the crash route's mode (issues #10520, #11001): the image
# adopts 1024x768 and 1920x1080 (ADOPT_MODES), and an HDMI capture unit
# gives no picture after a size change. nanopc-t6.sh --crash --go starts
# t6-kms-mode.py, which holds VP0's own mode when it is adopted and else
# sets 1024x768, reads VP0's mode back from the DRM state and jumps only on
# the mode held. On the fake board: Linux's 1920x1080 and 1024x768 are held
# with no size change, and 1280x720, a 1920x1080 with the wrong syncs, VP0
# off and an unread VP0 get 1024x768; the holder is started before the
# jump; a refusal of the helper, an answer without its pid, and each wrong
# read-back (the other adopted mode, a clock off by 1 kHz, the wrong syncs,
# interlace, VP0 off, nothing) stop the run before the jump; every refusal
# after a holder was started stops it by the pid its line named, also a
# jump the board refuses; the normal route never starts it; the dry run
# names the mode it would hold, and refuses --crash without python3 or
# without VP0 in the DRM state. The board side of the stop runs here on a
# stand-in holder: it kills that pid and no other, and nothing in another
# Linux. vp0_mode reads VP0's lines of a DRM state in Linux's format.
# ADOPT_MODES are the Rust MODES. The helper's own self-test (the uapi
# numbers on arm64, the mode rule, the set and give-back order on a
# made-up card, the holder's stop) runs from here as one check.
#
# A twelfth (issue #10591): on 2026-11-22 a --go run jumped although the
# debug UART delivered nothing (uart.log 0 bytes). nanopc-t6.sh --go now
# has the board write one marker line to its console and reads it back from
# T6_UART before it copies anything. Here T6_UART is a pseudo terminal
# (wire.py) whose other end the fake board's console writes into, not
# /dev/null. The checks: the marker is written before anything is staged,
# read back and recorded, and is not in uart.log; each run has its own
# nonce; no byte, a failed write and bytes without the marker are each
# refused (rc 3, no record directory, no scp, no capture left on the tty);
# a board without a console=ttyS entry is refused; the host never writes a
# byte into the tty; the dry run names the console and writes nothing.
#
# A thirteenth (issue #11022): the screen path. nanopc-t6.sh --go takes
# T6_SCREEN, the capture unit on the board's HDMI output, as well as or
# instead of T6_UART. Here `connected` is a fake unit that answers with the
# screen of two requests ago (a PPM the holder's own draw_nonce() drew, or
# black for Linux's screen), gives no picture while 1024x768 is held, and
# `djpeg` passes the PPM through. The checks: T6_SCREEN without --crash and
# a run with neither path are refused before anything is copied; a run with
# T6_SCREEN only is proven at the third picture (the first two stale), keeps
# the proof pictures, has no uart.log and says so, and its capture loop
# takes pictures across the jump and is stopped by its pid, a request in
# flight with it; a unit that gives no picture, pictures of another nonce,
# or none at 1024x768 is refused in the mode step: no jump, the holder
# stopped by its pid, Linux's mode back. The record never names the unit.
# t6-screen-nonce.py's own self-test (the holder's drawing decoded back,
# the pictures that must not pass, a JPEG round trip) runs as one check.
#
# A fourteenth (issue #11468): the image runs at the CPU clock Linux's
# governor left at the jump. nanopc-t6.sh records the cpufreq facts of the
# CPU the jump runs on, and T6_CPU_KHZ holds that policy at one step
# (t6-cpufreq.sh). Its functions run here on a made-up sysfs, and the fake
# board's ssh runs them there too. The checks: the jump CPU (CPU 0 on the
# crash route, reboot_cpu on the normal one, the first online CPU when
# reboot_cpu is offline, every policy when it is not known); min and max
# written in the order that keeps min <= max for a step above, below and
# inside the range; a value outside the steps refused with the steps listed,
# before any write (dry run and --go); a hold whose scaling_cur_freq never
# reads the step undone by the board, and no jump; a jump the board refuses
# after the hold undone from the host; no undo after a jump, nor in another
# Linux; without T6_CPU_KHZ the record lines and no write.
#
# A fifteenth (issue #11611): the T6's debug UART is now a network serial
# adapter that speaks RFC 2217, and T6_UART=rfc2217://<host>:<port> runs the
# runner through t6-uart-rfc2217.py, which sets 1500000 8N1 and gives the
# runner a pty. Its self-test (a made-up adapter on 127.0.0.1) runs here as
# one check; then nanopc-t6.sh's dry run and one --go through it against
# that adapter (see the checks for what they show).
#
# The T6's other runners on the lock, driver/tools/uboot-t6/prompt-reads.sh
# (the reads at the U-Boot prompt) and prompt-go.sh (an image started with
# U-Boot's go), are held to the same rule: LOCK_WAIT 0 before they source
# board_lock.sh. The tools they start on the UART, prompt-reads.py and
# prompt-go.py, have self-tests against a fake board on a pty (what they
# type and when, that they never type into Linux's root shell, and for
# prompt-go.py nothing after go and every refusal before it); and
# uboot-t6/selftest.sh runs stage.sh and prompt-go.sh's dry run against a
# fake ssh. The checks here run all three, so the gate does too.
#
# Needs bash, flock(1), strings(1) and python3. tools/check.sh runs it ("tools   board_lock.sh: ...").
set -u
here=$(cd "$(dirname "$0")" && pwd)
run=0
failed=0
# The lines of this file whose check has run. Every line that calls check
# must be in it at the end: a check that a condition or an early return
# skipped fails the test, and no count is kept by hand.
declare -A ran=()
check() { # <what> <0 = ok>
	run=$((run + 1))
	ran[${BASH_LINENO[0]}]=1
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
# marked [marker]: the pids under /proc whose environment carries the marker
# (default $tmp; a check may give its processes a marker of their own), this
# shell excepted (the command substitution's subshell has it too, and is
# excepted by reading BASHPID). Builtins only: a tr or grep started here
# would carry the marker and could find itself.
marked() {
	local self=$BASHPID mark=${1:-$tmp} e p v
	local -a env
	for e in /proc/[0-9]*/environ; do
		p=${e#/proc/}
		p=${p%/environ}
		{ [ "$p" = "$$" ] || [ "$p" = "$self" ]; } && continue
		{ mapfile -d '' env <"$e"; } 2>/dev/null || continue
		for v in "${env[@]}"; do
			[ "$v" = "BOARD_LOCK_SELFTEST_RUN=$mark" ] && { echo "$p"; break; }
		done
	done
}
# sweep [marker]: TERM each marked pid, then wait up to 5 s for them to go.
sweep() {
	local p
	for p in $(marked "$@"); do kill "$p" 2>/dev/null; done
	for _ in $(seq 50); do [ -z "$(marked "$@")" ] && return 0; sleep 0.1; done
	return 1
}
trap 'sweep "$tmp/killed"; sweep "$tmp/normal"; sweep; rm -rf "$tmp"' EXIT

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

# prompt-reads.sh and prompt-go.sh, the same: LOCK_WAIT 0 before they
# source board_lock.sh.
for ubr in "$here/../uboot-t6/prompt-reads.sh" "$here/../uboot-t6/prompt-go.sh"; do
	src=$(grep -n '^\. "\$K/board_lock.sh"$' "$ubr" | cut -d: -f1)
	def=$(grep -n '^LOCK_WAIT=\${LOCK_WAIT:-0}$' "$ubr" | cut -d: -f1)
	later=$(awk -v a="${def:-0}" -v b="${src:-0}" 'NR > a && NR < b && /LOCK_WAIT=/' "$ubr")
	check "$(basename "$ubr"): LOCK_WAIT defaults to 0 before it sources board_lock.sh (lines ${def:-none} < ${src:-none})" \
		"$([ -n "$src" ] && [ -n "$def" ] && [ "$def" -lt "$src" ] && [ -z "$later" ]; echo $?)"
done

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

# A holder that returns (its EXIT trap stops the watchdog) leaves no process
# of its watchdog behind; its processes carry a marker of their own.
cat > "$tmp/normal.sh" <<EOF
#!/bin/bash
LOCK_WAIT=0 HOLD_MAX=1800
. "$here/board_lock.sh"
echo "\$BOARD_LOCK_WATCHDOG" > "$tmp/normal.wd"
EOF
chmod +x "$tmp/normal.sh"
BOARD_LOCK_SELFTEST_RUN=$tmp/normal BOARD_LOCK=$tmp/normal.lock timeout 60 "$tmp/normal.sh"
rc=$?
for _ in $(seq 50); do [ -z "$(marked "$tmp/normal")" ] && break; sleep 0.1; done
left=$(marked "$tmp/normal")
left=${left//$'\n'/ }
check "a holder that returns (rc $rc) leaves no watchdog process after 5 s (left: ${left:-none})" \
	"$([ "$rc" = 0 ] && [ -s "$tmp/normal.wd" ] && [ -z "$left" ]; echo $?)"

# A runner killed with SIGKILL (its EXIT trap never runs) leaves no process
# of its watchdog behind. Its processes carry a marker of their own, so the
# holder above does not count. The runner's own `sleep` is killed by its pid
# once the runner is gone; what is left after that is the watchdog's.
cat > "$tmp/killed.sh" <<EOF
#!/bin/bash
LOCK_WAIT=0 HOLD_MAX=1800
. "$here/board_lock.sh"
sleep 60 &
echo \$! > "$tmp/killed.sleep"
wait
EOF
chmod +x "$tmp/killed.sh"
BOARD_LOCK_SELFTEST_RUN=$tmp/killed BOARD_LOCK=$tmp/killed.lock "$tmp/killed.sh" &
killed_pid=$!
for _ in $(seq 100); do [ -s "$tmp/killed.sleep" ] && break; sleep 0.1; done
kill -KILL "$killed_pid"
wait "$killed_pid" 2>/dev/null
kill "$(cat "$tmp/killed.sleep")" 2>/dev/null
for _ in $(seq 50); do [ -z "$(marked "$tmp/killed")" ] && break; sleep 0.1; done
left=$(marked "$tmp/killed")
left=${left//$'\n'/ }
check "a runner killed with SIGKILL leaves no watchdog process after 5 s (left: ${left:-none})" \
	"$([ -s "$tmp/killed.sleep" ] && [ -z "$left" ]; echo $?)"

# The watchdog kills only its own runner: a live pid whose start ticks are
# not the runner's (the pid given to another process) is left alone, and the
# watchdog ends; the same pid with its own ticks is killed after the seconds.
sleep 30 &
victim=$!
vticks=$(BOARD_LOCK_NO_ACQUIRE=1 BOARD_LOCK=$tmp/wd.lock bash -c ". '$here/board_lock.sh'; board_lock_start_ticks $victim")
t0=$(date +%s)
BOARD_LOCK_NO_ACQUIRE=1 BOARD_LOCK=$tmp/wd.lock timeout 30 \
	bash -c ". '$here/board_lock.sh' && board_lock_watchdog $victim $((vticks + 1)) 1" 2>/dev/null
rc=$?
t1=$(date +%s)
check "a watchdog whose pid has other start ticks ends (rc $rc) within 5 s ($((t1 - t0)) s)" \
	"$([ "$rc" = 0 ] && [ $((t1 - t0)) -le 5 ]; echo $?)"
check "... and does not kill that process (pid $victim)" "$(kill -0 "$victim" 2>/dev/null; echo $?)"
err=$(BOARD_LOCK_NO_ACQUIRE=1 BOARD_LOCK=$tmp/wd.lock timeout 30 \
	bash -c ". '$here/board_lock.sh' && board_lock_watchdog $victim $vticks 1" 2>&1)
wait "$victim" 2>/dev/null
vrc=$?
check "a watchdog with its runner's start ticks kills it after its seconds (exit $vrc) and says so" \
	"$([ "$vrc" = 143 ] && grep -q "pid $victim held the board past 1s" <<<"$err"; echo $?)"

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

# nanopc-t6.sh's checks of the board's facts, on made-up facts: an `ssh`
# first in PATH answers from $tmp/facts (below), so no board is contacted.
# The facts are the T6's under Armbian: one System RAM bank
# 0x00200000-0xe143ffff, below 0x20000000 only the kernel's entries under
# it, the GIC's ITS/LPI tables above 4 GiB. An image TOP 13 passes the image
# checks. t6_board_run <iomem lines under the bank> <gic_tables> [--go]
# The board's `ssh`: it logs each command and answers by what it asks. The
# facts and the read under the lock come from $tmp/facts and $tmp/fresh;
# the jump (the command that runs kexec_zephyr) drops the connection and
# the board then comes back with another boot_id; anything else prints
# nothing. stty, fuser and scp stand in for --go's UART checks and copies.
# The crash route's mode step (t6-kms-mode.py): the holder's start answers
# T6_FAKE_HOLD_OUT and exits T6_FAKE_HOLD_RC (default: the --mode it was
# given, held by pid 4242); VP0's read-back answers T6_FAKE_VP0_HELD
# (default the mode held) while a holder runs and Linux's own mode
# otherwise (T6_FAKE_VP0_LINUX, default 1920x1080); the stop takes the
# holder away and answers T6_FAKE_VP0_AFTER (default Linux's own mode) as
# VP0's mode. A jump with T6_FAKE_JUMP_RC set is refused on the board with
# that code (the linux-wdt check) and does not happen.
vp0_1080='enable=1 active=1 mode: "1920x1080": 60 148500 1920 2008 2052 2200 1080 1084 1089 1125 0x48 0x5'
vp0_adopt='enable=1 active=1 mode: "1024x768": 60 65000 1024 1048 1184 1344 768 771 777 806 0x40 0xa'
vp0_720='enable=1 active=1 mode: "1280x720": 60 74250 1280 1390 1430 1650 720 725 730 750 0x40 0x5'
mkdir "$tmp/fakebin"
# The board's cpufreq (t6-cpufreq.sh), a made-up /sys and /proc under
# $tmp/cfroot: the T6's three policies (CPUs 0-3, 4-5, 6-7) and their steps,
# reboot_cpu 0, all eight CPUs online, the boot_id the fake board reports
# before the jump. cf_root [min max cur]: made again, policy0's
# scaling_min_freq, scaling_max_freq and scaling_cur_freq as given (default
# Linux's 408000 1800000 408000).
cfroot=$tmp/cfroot
cf_steps0='408000 600000 816000 1008000 1200000 1416000 1608000 1800000'
cf_steps4='408000 600000 816000 1008000 1200000 1416000 1608000 1800000 2016000 2208000 2256000'
cf_root() {
	local c p
	rm -rf "$cfroot"
	mkdir -p "$cfroot/sys/kernel/reboot" "$cfroot/sys/devices/system/cpu/cpufreq" "$cfroot/proc/sys/kernel/random"
	echo 0 > "$cfroot/sys/kernel/reboot/cpu"
	echo 0-7 > "$cfroot/sys/devices/system/cpu/online"
	echo boot-before > "$cfroot/proc/sys/kernel/random/boot_id"
	for p in 0:"0 1 2 3":"$cf_steps0" 4:"4 5":"$cf_steps4" 6:"6 7":"$cf_steps4"; do
		local n=${p%%:*} cpus steps d
		cpus=${p#*:}; cpus=${cpus%%:*}; steps=${p##*:}
		d=$cfroot/sys/devices/system/cpu/cpufreq/policy$n
		mkdir "$d"
		echo "$cpus" > "$d/related_cpus"
		echo cpufreq-dt > "$d/scaling_driver"
		echo ondemand > "$d/scaling_governor"
		echo "$steps " > "$d/scaling_available_frequencies"
		echo 408000 > "$d/scaling_min_freq"
		echo "${steps##* }" > "$d/scaling_max_freq"
		echo 408000 > "$d/scaling_cur_freq"
		echo 408000 > "$d/cpuinfo_cur_freq"
		for c in $cpus; do
			mkdir -p "$cfroot/sys/devices/system/cpu/cpu$c"
			ln -s ../cpufreq/policy$n "$cfroot/sys/devices/system/cpu/cpu$c/cpufreq"
		done
	done
	d=$cfroot/sys/devices/system/cpu/cpufreq/policy0
	echo "${1:-408000}" > "$d/scaling_min_freq"
	echo "${2:-1800000}" > "$d/scaling_max_freq"
	echo "${3:-408000}" > "$d/scaling_cur_freq"
}
cf_root
printf '%s\n' "$vp0_1080" > "$tmp/vp0-1920x1080"
printf '%s\n' "$vp0_adopt" > "$tmp/vp0-1024x768"
printf '%s\n' 't6-kms-mode: HDMI-A-1 on CRTC 81: "1920x1080" 148500 kHz h 1920 2008 2052 2200 v 1080 1084 1089 1125 +hsync +vsync set (fb 60), read back; was fb 50, "1920x1080" 148500 kHz; held by pid 4242 for at most 600 s' > "$tmp/held-1920x1080"
printf '%s\n' 't6-kms-mode: HDMI-A-1 on CRTC 81: "1024x768" 65000 kHz h 1024 1048 1184 1344 v 768 771 777 806 -hsync -vsync set (fb 60), read back; was fb 50, "1920x1080" 148500 kHz; held by pid 4242 for at most 600 s' > "$tmp/held-1024x768"
cat > "$tmp/fakebin/ssh" <<EOS
#!/bin/bash
for a; do last=\$a; done
printf '%s\n----\n' "\$last" >> "$tmp/ssh.log"
linux_mode() { if [ -n "\${T6_FAKE_VP0_LINUX+set}" ]; then printf '%s\n' "\$T6_FAKE_VP0_LINUX"; else cat "$tmp/vp0-1920x1080"; fi; }
case "\$last" in
*'printf "compatible='*) cat "$tmp/facts" ;;
*'printf "kexec_loaded='*) cat "$tmp/fresh" ;;
*'cf_facts '* | *'cf_hold '* | *'cf_restore '*) T6CF_ROOT=$cfroot exec sh -c "\$last" ;;
*'kill -TERM'*)
	p=\$(sed -n 's/^set -- \([0-9]*\) .*/\1/p' <<<"\$last")
	rm -f "$tmp/mode-held" "$tmp/screen-nonce"
	printf 'pid %s ended after SIGTERM\nt6-kms-mode: pid %s: stop (signal SIGTERM): CRTC 81 back on fb 50; card closed\nvp0=' "\$p" "\$p"
	if [ -n "\${T6_FAKE_VP0_AFTER+set}" ]; then printf '%s\n' "\$T6_FAKE_VP0_AFTER"; else linux_mode; fi ;;
*'t6-kms-mode.py hold --mode '*)
	m=\$(sed -n 's/.* hold --mode \([0-9x]*\) .*/\1/p' <<<"\$last")
	[ "\${T6_FAKE_HOLD_RC:-0}" != 0 ] || printf '%s\n' "\$m" > "$tmp/mode-held"
	sed -n 's/.* --nonce \([0-7]*\)\$/\1/p' <<<"\$last" > "$tmp/screen-nonce"
	if [ -n "\${T6_FAKE_HOLD_OUT+set}" ]; then printf '%s\n' "\$T6_FAKE_HOLD_OUT"; else cat "$tmp/held-\$m"; fi
	exit "\${T6_FAKE_HOLD_RC:-0}" ;;
*vp0_mode)
	if [ ! -e "$tmp/mode-held" ]; then linux_mode
	elif [ -n "\${T6_FAKE_VP0_HELD+set}" ]; then printf '%s\n' "\$T6_FAKE_VP0_HELD"
	else cat "$tmp/vp0-\$(cat "$tmp/mode-held")"; fi ;;
*'yakogl-uart-proof '*)
	case "\${T6_FAKE_UART:-marker}" in
	marker) printf '\r\n%s\r\n' "\$(grep -o 'yakogl-uart-proof [0-9a-f]*' <<<"\$last")" > "\${T6_FAKE_WIRE:-$tmp/wire.fifo}" ;;
	noise) cat "$tmp/noise" > "\${T6_FAKE_WIRE:-$tmp/wire.fifo}" ;;
	fail) exit 255 ;;
	esac ;;
*sha256sum*) printf '%s' "\$T6_FAKE_SHAS" ;;
*'/kexec_zephyr '*)
	if [ -n "\${T6_FAKE_JUMP_RC:-}" ]; then
		echo 'linux-wdt: WDT0 CR 0x00000000 TORR 0x0000000e after the open: WDT_EN is not 1, not jumping'
		exit "\$T6_FAKE_JUMP_RC"
	fi
	touch "$tmp/jumped"
	# the image's first line, on the RFC 2217 adapter's wire only
	[ -z "\${T6_FAKE_WIRE:-}" ] || printf 'T6:A image line\r\n' > "\$T6_FAKE_WIRE"
	exit 255 ;;
'cat /proc/sys/kernel/random/boot_id') if [ -e "$tmp/jumped" ]; then echo boot-after; else echo boot-before; fi ;;
esac
exit 0
EOS
printf '#!/bin/sh\ncase " $* " in *" -a "*) echo "speed 1500000 baud; line = 0; -parenb cs8 -cstopb clocal -icrnl -ixon -opost -icanon -echo" ;; esac\n' > "$tmp/fakebin/stty"
printf '#!/bin/sh\nexit 1\n' > "$tmp/fakebin/fuser"
printf '#!/bin/sh\necho "$*" >> "%s/scp.log"\n' "$tmp" > "$tmp/fakebin/scp"
# The capture unit: `connected screenshot <unit> -o <file>` writes the screen
# as it was two requests ago (a PPM, which the fake `djpeg -pnm` passes
# through): the holder's bars for the nonce it was given while it holds
# 1920x1080, nothing while it holds 1024x768 (the unit gives no picture of
# that mode), black while no holder runs. T6_FAKE_SCREEN: none (every
# request fails with the unit's error text), other (every picture shows
# another nonce), slow (each request takes 2 s). Each call's unit is logged.
cat > "$tmp/screen.py" <<'EOS'
import importlib.util, sys
what, out = sys.argv[1:3]
w, h = 1920, 1080
if what == "black":
    rgb = bytes(3 * w * h)
else:
    spec = importlib.util.spec_from_file_location("holder", sys.argv[3])
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    row = bytearray(4 * w)
    mod.draw_nonce(row, w, 1, 4 * w, what)
    line = bytearray(3 * w)
    line[0::3], line[1::3], line[2::3] = row[2::4], row[1::4], row[0::4]
    rgb = bytes(line) * h
with open(out, "wb") as f:
    f.write(b"P6\n%d %d\n255\n" % (w, h) + rgb)
EOS
cat > "$tmp/fakebin/connected" <<EOS
#!/bin/bash
[ "\$1 \$3" = "screenshot -o" ] || { echo "fake connected: usage" >&2; exit 2; }
echo "\$2" >> "$tmp/connected.log"
[ "\${T6_FAKE_SCREEN:-}" != slow ] || sleep 2
if [ "\${T6_FAKE_SCREEN:-}" = none ]; then echo "cube_screenshot failed: failed to capture screenshot (\$2)" >&2; exit 1; fi
# what the screen shows now, then the answer: what it showed two requests ago
if [ "\$(cat "$tmp/mode-held" 2>/dev/null)" = 1024x768 ]; then now=nosignal
elif [ -e "$tmp/mode-held" ] && [ -s "$tmp/screen-nonce" ]; then now=\$(cat "$tmp/screen-nonce")
else now=black; fi
if [ "\${T6_FAKE_SCREEN:-}" = other ]; then
	now=70615243
	[ "\$(cat "$tmp/screen-nonce" 2>/dev/null)" != 70615243 ] || now=36104527
fi
echo "\$now" >> "$tmp/screen-history"
shown=\$(tail -n 3 "$tmp/screen-history" | head -n 1)
[ "\$(wc -l < "$tmp/screen-history")" -ge 3 ] || shown=black
if [ "\$shown" = nosignal ]; then echo "cube_screenshot failed: failed to capture screenshot" >&2; exit 1; fi
python3 -I "$tmp/screen.py" "\$shown" "\$4" "$here/t6-kms-mode.py"
EOS
printf '#!/bin/sh\n[ "$1" = -pnm ] && [ -s "$2" ] && exec cat "$2"\necho "fake djpeg: no picture" >&2; exit 1\n' > "$tmp/fakebin/djpeg"
chmod +x "$tmp/fakebin/ssh" "$tmp/fakebin/stty" "$tmp/fakebin/fuser" "$tmp/fakebin/scp" "$tmp/fakebin/connected" "$tmp/fakebin/djpeg"
t6_image "$tmp/top13.bin" 'board: watchdog armed'
sed -i 's/; top 9; /; top 13; /' "$tmp/top13.bin"
t6_board_run() {
	{
		printf '%s\n' 'compatible=friendlyarm,nanopc-t6 rockchip,rk3588 ' kernel=6.18.55 \
			kexec_load_disabled=0 kexec_loaded=0 config_kexec=CONFIG_KEXEC=y uid=0 \
			wdt_identity=dw_wdt wdt_state=inactive wdt_timeout=44 wdt_nowayout=0 wdt_users= \
			"config_crash_dump=${t6_crash_dump:-CONFIG_CRASH_DUMP=y}" "kexec_crash_loaded=${t6_crash_loaded:-0}" \
			"kexec_crash_size=${t6_crash_size:-0}" "sysrq_trigger=${t6_sysrq:-writable}" \
			"python3=${t6_python3-/usr/bin/python3}" "vp0=${t6_vp0-$vp0_1080}" "console=${t6_console-ttyS2}" \
			"gic_tables=$2"
		T6CF_ROOT=$cfroot sh -c ". '$here/t6-cpufreq.sh'; cf_facts $(case " ${3:-} ${4:-} " in *" --crash "*) echo crash ;; *) echo normal ;; esac)"
		printf '%s\n' 'iomem<<' '00200000-e143ffff : System RAM' "$1" '>>'
	} > "$tmp/facts"
	PATH=$tmp/fakebin:$PATH BOARD_HOST=root@board.invalid bash "$t6" "${t6_img:-$tmp/top13.bin}" ${3:+"$3"} ${4:+"$4"} 2>&1
}
kernel='  02000000-0432ffff : Kernel code'
gic_out='100220000 100230000 1002f0000 '
# The UART: a pseudo terminal whose other end stands for the wire to the
# board's debug header. wire.py holds both ends; what the fake board writes
# to its console (the fake ssh writes it into wire.fifo) comes out of the
# tty, and any byte the host writes into the tty lands in wire.host, which
# must stay empty. The tty is raw with no echo, as the fake stty would leave
# the real one.
cat > "$tmp/wire.py" <<'EOS'
import os, pty, select, sys, tty
fifo, host, name = sys.argv[1:4]
m, s = pty.openpty()
tty.setraw(s)
f = os.open(fifo, os.O_RDWR)
with open(name + ".new", "w") as o:
    o.write(os.ttyname(s) + "\n")
os.rename(name + ".new", name)
while True:
    r, _, _ = select.select([f, m], [], [])
    if f in r:
        os.write(m, os.read(f, 4096))
    if m in r:
        d = os.read(m, 4096)
        with open(host, "ab") as o:
            o.write(d)
EOS
mkfifo "$tmp/wire.fifo"
python3 -I "$tmp/wire.py" "$tmp/wire.fifo" "$tmp/wire.host" "$tmp/wire.name" &
wire_pid=$!
for _ in $(seq 50); do [ -s "$tmp/wire.name" ] && break; sleep 0.1; done
wire_tty=$(cat "$tmp/wire.name" 2>/dev/null)
check "the stand-in UART is a pseudo terminal ($wire_tty, pid $wire_pid)" "$([ -c "$wire_tty" ]; echo $?)"
# wire_users: the processes of the test, the wire's excepted, that hold the
# stand-in UART open (a capture left behind).
wire_users() {
	local p f
	for p in $(marked); do
		[ "$p" = "$wire_pid" ] && continue
		for f in /proc/"$p"/fd/*; do
			[ "$(readlink "$f" 2>/dev/null)" = "$wire_tty" ] && { printf '%s ' "$p"; break; }
		done
	done
}
printf 'bytes from another console\r\n' > "$tmp/noise"
# t6_go_run <iomem lines> [--crash]: --go to the end on the fake board, with
# T6_HAND, a record directory, the stand-in UART (the fake stty and fuser
# pass it; t6_uart= leaves T6_UART empty), T6_SCREEN $t6_screen (default
# empty), SCREEN_PROOF_S $t6_proof_s (default 90) and stand-ins for the two
# tools. The read under the lock gives
# kexec_crash_loaded $t6_fresh_crash (default 0). The jump command reaches
# the fake ssh's log; the fake board writes the UART proof's marker line
# back on the wire unless T6_FAKE_UART says otherwise (none: nothing; noise:
# other bytes; fail: the ssh command fails).
t6_go_run() {
	rm -rf "$tmp/rec" "$tmp/jumped" "$tmp/ssh.log" "$tmp/scp.log" "$tmp/mode-held" "$tmp/screen-nonce" \
		"$tmp/screen-history" "$tmp/connected.log"
	mkdir "$tmp/rec"
	printf 'kexec_loaded=0\nkexec_crash_loaded=%s\nboot_id=boot-before\ngpu_users=\nwdt_users=\nwho_remote=\n' \
		"${t6_fresh_crash:-0}" > "$tmp/fresh"
	: > "$tmp/kz"
	: > "$tmp/mr"
	local staged=("$tmp/top13.bin" "$tmp/kz")
	[ "${2:-}" != --crash ] || staged+=("$here/t6-kms-mode.py")
	T6_FAKE_SHAS=$(sha256sum "${staged[@]}" | cut -d' ' -f1 | tr '\n' ' ') \
		T6_HAND=selftest T6_RECORD_DIR=$tmp/rec T6_UART=${t6_uart-$wire_tty} T6_SCREEN=${t6_screen:-} \
		SCREEN_PROOF_S=${t6_proof_s:-90} KEXEC_ZEPHYR=$tmp/kz MMIO_RD=$tmp/mr \
		BOARD_LOCK=$tmp/t6go.lock t6_board_run "$1" "$gic_out" --go ${2:+"$2"}
}
out=$(t6_board_run "$kernel" "$gic_out")
rc=$?
check "nanopc-t6.sh: the dry run on the T6's facts finds the window 0x10000000-0x1fffffff free (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'window 0x10000000-0x1fffffff is inside System RAM and overlaps no /proc/iomem entry' <<<"$out"; echo $?)"
out=$(t6_board_run "$kernel
  1ff00000-1fffffff : reserved" "$gic_out")
rc=$?
check "nanopc-t6.sh: an entry under the bank at the window's last MiB is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: the window overlaps: 1ff00000-1fffffff : reserved' <<<"$out"; echo $?)"
out=$(t6_board_run "$kernel" '100220000 1fff0000 ')
rc=$?
check "nanopc-t6.sh: a GIC table in the window's last 64 KiB is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: a GIC ITS/LPI table at 0x1fff0000' <<<"$out"; echo $?)"
# When dmesg names no table (its ring wrapped), where the tables are is
# unknown: --go and the dry run refuse, they do not say "not checked" and go on.
out=$(t6_board_run "$kernel" '' --go)
rc=$?
check "nanopc-t6.sh: --go refuses when Linux's log names no GIC table (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: Linux'"'"'s log names no GIC ITS/LPI table' <<<"$out"; echo $?)"
out=$(t6_board_run "$kernel" "$gic_out" --go)
rc=$?
check "nanopc-t6.sh: --go with the tables named gets past the board checks to its own T6_HAND refusal (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: --go needs T6_HAND' <<<"$out"; echo $?)"

# t6-display-dump.sh: its self-test, with its own made-up board and its own
# count of check lines.
out=$(bash "$here/t6-display-dump-selftest.sh" 2>&1)
rc=$?
[ "$rc" = 0 ] || grep ' FAIL$' <<<"$out"
check "t6-display-dump.sh: its self-test passes (rc $rc: $(tail -n 1 <<<"$out"))" "$rc"
out=$(sh "$here/crash_irq_selftest.sh" 2>&1)
rc=$?
[ "$rc" = 0 ] || grep -E ' FAIL$|error' <<<"$out"
check "kexec_zephyr's crash_irq.h: its self-test passes (rc $rc: $(tail -n 1 <<<"$out"))" "$rc"

# The crash route. The reservation t6-crashkernel.sh makes is
# 0x10000000-0x1fffffff, under the bank after the kernel.
ck='  10000000-1fffffff : Crash kernel'
out=$(t6_board_run "$kernel" "$gic_out")
check "nanopc-t6.sh: without --crash the dry run names the normal route and no reservation" \
	"$(grep -qF 'route: normal: kexec_load, then reboot(LINUX_REBOOT_CMD_KEXEC)' <<<"$out" && grep -qF 'crashkernel= reservation: none)' <<<"$out"; echo $?)"
check "nanopc-t6.sh: the normal route's plan jumps without --crash" \
	"$(grep -qE 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin 0x10000000$' <<<"$out"; echo $?)"
out=$(t6_board_run "$kernel" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: --crash without a reservation is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF "REFUSED: --crash needs a crashkernel= reservation that holds the window 0x10000000-0x1fffffff; the board has: Crash kernel 'none'" <<<"$out"; echo $?)"
out=$(t6_crash_size=134217728 t6_board_run "$kernel
  08000000-17ffffff : Crash kernel" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: --crash with a reservation holding half the window is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF "the board has: Crash kernel '08000000-17ffffff : Crash kernel'" <<<"$out"; echo $?)"
out=$(t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: --crash with the reservation passes the dry run and names the crash route (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'route: crash (--crash)' <<<"$out" && grep -qF 'inside the crashkernel= reservation 0x10000000-0x1fffffff' <<<"$out" && grep -qE 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin 0x10000000 --crash$' <<<"$out"; echo $?)"
out=$(t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash --go)
rc=$?
check "nanopc-t6.sh: --crash --go with the reservation reaches its T6_HAND refusal (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: --go needs T6_HAND' <<<"$out"; echo $?)"
out=$(t6_crash_loaded=1 t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: --crash with a crash image already loaded is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: --crash: a crash image is already loaded' <<<"$out"; echo $?)"
# The normal route while the reservation holds the window: the kernel
# limits only crash segments to the reservation (kernel/kexec_core.c:
# 200-210), so it is allowed while no crash image is loaded there.
out=$(t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out")
rc=$?
check "nanopc-t6.sh: the normal route is allowed with the reservation in the window and no crash image (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'route: normal:' <<<"$out" && grep -qF 'the window lies in the crashkernel= reservation 10000000-1fffffff : Crash kernel, and no crash image is loaded' <<<"$out"; echo $?)"
out=$(t6_crash_loaded=1 t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out")
rc=$?
check "nanopc-t6.sh: the normal route is refused when a crash image is loaded in that reservation (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: a crash image is loaded in the crashkernel= reservation the window lies in' <<<"$out"; echo $?)"
# --go to the end on the fake board: the jump command each route sends.
out=$(t6_crash_size=268435456 t6_go_run "$kernel
$ck" --crash)
rc=$?
jump=$(grep -m1 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin' "$tmp/ssh.log")
# After the links are down a returned kexec_zephyr must end in a WDT0
# reset: no magic close, and WDT0 kept open (unfed) past the ssh session.
after=$(sed -n '/kexec_zephyr \/tmp\/yakogl-kexec\/zephyr.bin/,/^----$/p' "$tmp/ssh.log")
check "nanopc-t6.sh: on the crash route a returned kexec_zephyr writes no magic close and keeps WDT0 open" \
	"$(! grep -qF 'printf V' <<<"$after" && grep -qF 'setsid sleep' <<<"$after"; echo $?)"
check "nanopc-t6.sh: --crash --go sends the jump with --crash and records the route (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qE 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin 0x10000000 --crash; rc=' <<<"$jump" && grep -qF 'route crash (reservation 0x10000000-0x1fffffff)' "$tmp"/rec/*/record.txt; echo $?)"
out=$(t6_go_run "$kernel")
rc=$?
jump=$(grep -m1 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin' "$tmp/ssh.log")
after=$(sed -n '/kexec_zephyr \/tmp\/yakogl-kexec\/zephyr.bin/,/^----$/p' "$tmp/ssh.log")
check "nanopc-t6.sh: on the normal route a returned kexec_zephyr is followed by the magic close" \
	"$(grep -qF 'printf V >&3' <<<"$after"; echo $?)"
check "nanopc-t6.sh: --go without --crash sends the jump without it (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qE 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin 0x10000000; rc=' <<<"$jump" && grep -qF 'route normal' "$tmp"/rec/*/record.txt; echo $?)"
check "nanopc-t6.sh: the normal route neither copies nor starts t6-kms-mode.py" \
	"$(! grep -qF 't6-kms-mode.py' "$tmp/ssh.log"; echo $?)"

# The UART proof (issue #10591), on the run above: before anything was
# copied, the board wrote one marker line with a per-run nonce to the console
# its kernel command line names, the capture read it back, and the record
# says so. The marker line is not in uart.log (the proof's capture had it),
# no capture holds the tty after the run, and the host wrote no byte to it.
rec=$(cat "$tmp"/rec/*/record.txt 2>/dev/null)
proof_re="uart: proven before anything was copied: the marker line 'yakogl-uart-proof ([0-9a-f]{16})' the board wrote to /dev/ttyS2 was read back from $wire_tty [0-9]+ ms after the write began; that capture read [0-9]+ bytes in [0-9]+\.[0-9] s \(not part of uart.log\)"
nonce1=
[[ "$rec" =~ $proof_re ]] && nonce1=${BASH_REMATCH[1]}
mark_at=$(grep -n "^printf '.r.n%s.r.n' 'yakogl-uart-proof $nonce1' >/dev/ttyS2\$" "$tmp/ssh.log" | head -1 | cut -d: -f1)
stage_at=$(grep -n '^rm -rf /tmp/yakogl-kexec && mkdir -p /tmp/yakogl-kexec$' "$tmp/ssh.log" | head -1 | cut -d: -f1)
check "nanopc-t6.sh: --go writes the marker to the board's console before it stages anything, and records it read back (nonce ${nonce1:-none}, lines ${mark_at:-none} < ${stage_at:-none})" \
	"$([ "$rc" = 0 ] && [ -n "$nonce1" ] && [ -n "$mark_at" ] && [ -n "$stage_at" ] && [ "$mark_at" -lt "$stage_at" ] && [ -s "$tmp/scp.log" ]; echo $?)"
users=$(wire_users)
check "nanopc-t6.sh: the marker is not in uart.log, no capture holds the tty after the run (${users:-none}), the host wrote nothing to it" \
	"$(! grep -qaF yakogl-uart-proof "$tmp"/rec/*/uart.log && [ -z "$users" ] && [ ! -s "$tmp/wire.host" ]; echo $?)"
out=$(t6_go_run "$kernel")
rc=$?
rec=$(cat "$tmp"/rec/*/record.txt 2>/dev/null)
nonce2=
[[ "$rec" =~ $proof_re ]] && nonce2=${BASH_REMATCH[1]}
check "nanopc-t6.sh: each run's marker has its own nonce (${nonce1:-none}, ${nonce2:-none}; rc $rc)" \
	"$([ "$rc" = 0 ] && [ -n "$nonce2" ] && [ "$nonce1" != "$nonce2" ]; echo $?)"
# uart_refused <case>: rc 3 with refuse's second line, no record
# directory, nothing copied or staged, no jump, no capture left on the tty
# and no byte written to it by the host.
uart_refused() {
	local users
	users=$(wire_users)
	[ "$rc" = 3 ] && grep -qF 'nanopc-t6.sh: nothing was copied, loaded or changed.' <<<"$out" &&
		[ -z "$(ls -A "$tmp/rec")" ] && [ ! -e "$tmp/scp.log" ] &&
		! grep -qF 'mkdir -p /tmp/yakogl-kexec' "$tmp/ssh.log" && ! grep -qF '/kexec_zephyr ' "$tmp/ssh.log" &&
		[ -z "$users" ] && [ ! -s "$tmp/wire.host" ]
}
t0=$(date +%s)
out=$(T6_FAKE_UART=none t6_go_run "$kernel")
rc=$?
t1=$(date +%s)
check "nanopc-t6.sh: no byte on the UART is refused within $((t1 - t0)) s: 0 bytes, the board's write ok, the wire first (rc $rc)" \
	"$(uart_refused && grep -qE "REFUSED: UART path not proven: the capture of $wire_tty read 0 bytes in [5-9]\.[0-9] s, and the board wrote one marker line to its console /dev/ttyS2 \(ssh rc 0\)\. Check first the wire between the board's debug header" <<<"$out" && [ $((t1 - t0)) -le 15 ]; echo $?)"
out=$(T6_FAKE_UART=fail t6_go_run "$kernel")
rc=$?
check "nanopc-t6.sh: a failed write on the board is named in the refusal (rc $rc)" \
	"$(uart_refused && grep -qF 'read 0 bytes in' <<<"$out" && grep -qF "the ssh command that writes one marker line to the board's console /dev/ttyS2 failed (rc 255), so the line may not have been written" <<<"$out"; echo $?)"
out=$(T6_FAKE_UART=noise t6_go_run "$kernel")
rc=$?
check "nanopc-t6.sh: bytes without the marker are refused with their count and the speed and board named (rc $rc)" \
	"$(uart_refused && grep -qF "REFUSED: UART path not proven: the capture of $wire_tty read $(wc -c <"$tmp/noise") bytes in " <<<"$out" && grep -qF 'but not the marker line, and the board wrote one marker line to its console /dev/ttyS2 (ssh rc 0). The bytes are not that line' <<<"$out" && grep -qF '1500000 baud' <<<"$out"; echo $?)"
out=$(t6_console= t6_go_run "$kernel")
rc=$?
check "nanopc-t6.sh: a board without a console=ttyS entry is refused, nothing written to it (rc $rc)" \
	"$(uart_refused && grep -qF "REFUSED: the board's kernel command line has no console=ttyS entry" <<<"$out" && ! grep -qF yakogl-uart-proof "$tmp/ssh.log"; echo $?)"
rm -f "$tmp/ssh.log"
out=$(t6_board_run "$kernel" "$gic_out")
rc=$?
check "nanopc-t6.sh: the dry run names the board's console and writes no marker (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF "uart: the board's serial console (its kernel command line): /dev/ttyS2; --go has the board write one marker line" <<<"$out" && ! grep -qF yakogl-uart-proof "$tmp/ssh.log"; echo $?)"

# The debug UART over RFC 2217 (issue #11611): T6_UART=rfc2217://<host>:<port>
# is a network serial adapter, and --go runs nanopc-t6.sh again through
# t6-uart-rfc2217.py, which sets the line and gives it a pty. Here the
# adapter is the tool's own made-up one (FakeAdapter) on 127.0.0.1, and the
# fake board's console writes go to it (T6_FAKE_WIRE) instead of wire.py.
# The checks: the tool's self-test; the dry run says T6_UART is an RFC 2217
# address and does not connect; --go through the tool proves the UART path
# on the pty, records it as "the debug UART over RFC 2217", gets the image's
# line into uart.log, opens one session, writes no byte to the board, and
# no message or record names the address.
out=$(python3 -I "$here/t6-uart-rfc2217.py" --self-test 2>&1)
got=$(tail -n 1 <<<"$out")
check "t6-uart-rfc2217.py --self-test ('$got'$(grep -E '^FAIL' <<<"$out" | sed 's/^/; /' | tr -d '\n'))" \
	"$(grep -qE '^t6-uart-rfc2217 self-test: [1-9][0-9]* checks run, 0 failed$' <<<"$got"; echo $?)"
cat > "$tmp/adapter.py" <<'EOS'
import importlib.util, os, select, sys
tool, fifo, portfile, state = sys.argv[1:5]
spec = importlib.util.spec_from_file_location("t6uart", tool)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
srv = mod.FakeAdapter()
srv.start()
f = os.open(fifo, os.O_RDWR)
with open(portfile + ".new", "w") as o:
    o.write("%d\n" % srv.port)
os.rename(portfile + ".new", portfile)
while True:
    if select.select([f], [], [], 0.1)[0]:
        srv.board(os.read(f, 4096))
    with open(state + ".new", "w") as o:
        o.write("conns=%d host=%d\n" % (srv.conns, len(srv.got) + len(srv.garbled)))
    os.rename(state + ".new", state)
EOS
mkfifo "$tmp/adapter.fifo"
python3 -I "$tmp/adapter.py" "$here/t6-uart-rfc2217.py" "$tmp/adapter.fifo" "$tmp/adapter.port" "$tmp/adapter.state" &
adapter_pid=$!
for _ in $(seq 50); do [ -s "$tmp/adapter.port" ] && break; sleep 0.1; done
aport=$(cat "$tmp/adapter.port" 2>/dev/null)
aurl=rfc2217://127.0.0.1:${aport:-0}
# no_address <text>: the adapter's address is not in it
no_address() { ! grep -qF "127.0.0.1" <<<"$1" && ! grep -qF ":$aport" <<<"$1"; }
out=$(T6_UART=$aurl t6_board_run "$kernel" "$gic_out")
rc=$?
sleep 0.3
check "nanopc-t6.sh: the dry run with an RFC 2217 address says so, prints no address and does not connect ($(cat "$tmp/adapter.state" 2>/dev/null); rc $rc)" \
	"$([ "$rc" = 0 ] && [ -n "$aport" ] && grep -qF 'uart: T6_UART is an RFC 2217 address (not opened in a dry run; --go opens it with t6-uart-rfc2217.py' <<<"$out" &&
		no_address "$out" && grep -qx 'conns=0 host=0' "$tmp/adapter.state"; echo $?)"
out=$(T6_FAKE_WIRE=$tmp/adapter.fifo t6_uart=$aurl t6_go_run "$kernel")
rc=$?
rec=$(cat "$tmp"/rec/*/record.txt 2>/dev/null)
sleep 0.3
check "nanopc-t6.sh --go through t6-uart-rfc2217.py: the line set, the UART proven on its pty and recorded as the debug UART over RFC 2217 (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 't6-uart-rfc2217: the debug UART over RFC 2217: 1500000 8N1, no flow control' <<<"$out" &&
		grep -qF 'paths: UART (the debug UART over RFC 2217) (proven before anything was copied' <<<"$rec" &&
		grep -qE "uart: proven before anything was copied: the marker line 'yakogl-uart-proof [0-9a-f]{16}' the board wrote to /dev/ttyS2 was read back from the debug UART over RFC 2217 \(t6-uart-rfc2217.py's pty /dev/pts/[0-9]+\)" <<<"$rec" &&
		grep -qF 'back in Armbian' <<<"$rec"; echo $?)"
check "nanopc-t6.sh --go through t6-uart-rfc2217.py: uart.log holds the image's line and not the marker; one session, no byte to the board ($(cat "$tmp/adapter.state" 2>/dev/null))" \
	"$(grep -qaF 'T6:A image line' "$tmp"/rec/*/uart.log && ! grep -qaF yakogl-uart-proof "$tmp"/rec/*/uart.log &&
		grep -qx 'conns=1 host=0' "$tmp/adapter.state"; echo $?)"
check "nanopc-t6.sh --go through t6-uart-rfc2217.py: neither its output nor the record names the address" \
	"$(no_address "$out" && no_address "$rec"; echo $?)"
kill "$adapter_pid" 2>/dev/null
wait "$adapter_pid" 2>/dev/null

# The crash route's mode step (issues #10520, #11001): t6-kms-mode.py holds
# VP0's own mode when it is one the image adopts (no size change), else it
# sets 1024x768; VP0's mode is read back from Linux, and the jump happens
# only when it is the mode held. Every refusal after a holder was started
# stops it by the pid its line named (4242 on the fake board).
ck_go() { # t6_go_run --crash on the reservation; sets out, rc, rec
	out=$(t6_crash_size=268435456 t6_go_run "$kernel
$ck" --crash)
	rc=$?
	rec=$(cat "$tmp"/rec/*/record.txt 2>/dev/null)
}
stopped_4242() { grep -qF "set -- 4242 boot-before /tmp/yakogl-kexec" "$tmp/ssh.log" && grep -qF 'kill -TERM' "$tmp/ssh.log"; }
# held_and_jumped <mode> <VP0's line>: the holder of that mode started
# before the jump, and VP0 read back as that mode
held_and_jumped() {
	local hold_at jump_at
	hold_at=$(grep -n "t6-kms-mode.py hold --mode $1 --max-hold 600 --log /tmp/yakogl-kexec/kms-mode.log" "$tmp/ssh.log" | head -1 | cut -d: -f1)
	jump_at=$(grep -n 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin 0x10000000 --crash; rc=' "$tmp/ssh.log" | head -1 | cut -d: -f1)
	[ "$rc" = 0 ] && [ -n "$hold_at" ] && [ -n "$jump_at" ] && [ "$hold_at" -lt "$jump_at" ] &&
		[ "$(grep -c 't6-kms-mode.py hold' "$tmp/ssh.log")" = 1 ] &&
		grep -qF "mode: VP0 reads back '$2': the adopt's mode $1, held by pid 4242 across the jump" <<<"$rec"
}
ck_go
check "nanopc-t6.sh: --crash --go on Linux's 1920x1080, an adopted mode: it is held, no size change, and read back before the jump (rc $rc)" \
	"$(held_and_jumped 1920x1080 "$vp0_1080" && grep -qF 'mode: VP0 runs an adopted mode: hold 1920x1080, no size change' <<<"$rec"; echo $?)"
check "nanopc-t6.sh: after a jump that came back, the holder is not looked for (it ended with that Linux)" \
	"$(! grep -qF 'kill -TERM' "$tmp/ssh.log" && grep -qF 'mode: holder pid 4242 ended with the Linux that ran it' <<<"$rec"; echo $?)"
T6_FAKE_VP0_LINUX=$vp0_adopt ck_go
check "nanopc-t6.sh: --crash --go on Linux's 1024x768, an adopted mode: it is held, no size change (rc $rc)" \
	"$(held_and_jumped 1024x768 "$vp0_adopt" && grep -qF 'mode: VP0 runs an adopted mode: hold 1024x768, no size change' <<<"$rec"; echo $?)"
T6_FAKE_VP0_LINUX=$vp0_720 ck_go
check "nanopc-t6.sh: --crash --go on Linux's 1280x720, not an adopted mode: 1024x768 is set (rc $rc)" \
	"$(held_and_jumped 1024x768 "$vp0_adopt" && grep -qF 'mode: VP0 runs none of the adopted modes: set 1024x768' <<<"$rec"; echo $?)"
for vp in "${vp0_1080/0x5/0xa}" "${vp0_1080/enable=1 active=1/enable=0 active=0}" ''; do
	T6_FAKE_VP0_LINUX=$vp ck_go
	check "nanopc-t6.sh: --crash --go on VP0 '${vp:-unread}', not an adopted mode: 1024x768 is set (rc $rc)" \
		"$(held_and_jumped 1024x768 "$vp0_adopt" && grep -qF 'mode: VP0 runs none of the adopted modes: set 1024x768' <<<"$rec"; echo $?)"
done
T6_FAKE_HOLD_RC=3 T6_FAKE_HOLD_OUT="t6-kms-mode: REFUSED: HDMI-A-1 lists no mode with the adopt's 1920x1080 timing (9 modes; of size 1920x1080: none); nothing was changed" ck_go
check "nanopc-t6.sh: a refusal of t6-kms-mode.py stops the run before the jump, and no stop is sent (no holder) (rc $rc)" \
	"$([ "$rc" = 1 ] && grep -qF "mode: REFUSED: t6-kms-mode.py did not set and hold the adopt's mode (rc 3); not jumping" <<<"$out" && grep -qF 'lists no mode with the adopt' <<<"$rec" && ! grep -q 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin' "$tmp/ssh.log" && ! grep -qF 'kill -TERM' "$tmp/ssh.log"; echo $?)"
T6_FAKE_HOLD_OUT='t6-kms-mode: something else' ck_go
check "nanopc-t6.sh: an answer without the holder's pid is refused and says the holder ends by itself (rc $rc)" \
	"$([ "$rc" = 1 ] && grep -qF 'did not set and hold the adopt' <<<"$out" && grep -qF 'mode: no holder pid in the answer' <<<"$rec" && ! grep -q 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin' "$tmp/ssh.log" && ! grep -qF 'kill -TERM' "$tmp/ssh.log"; echo $?)"
# The read-back is the runner's own rule: with 1920x1080 held, each of
# these is refused (the other adopted mode too: it is not the mode held),
# and the holder is stopped and Linux's own mode is back.
for vp in "$vp0_adopt" \
	'enable=1 active=1 mode: "1920x1080": 60 148501 1920 2008 2052 2200 1080 1084 1089 1125 0x48 0x5' \
	'enable=1 active=1 mode: "1920x1080": 60 148500 1920 2008 2052 2200 1080 1084 1089 1125 0x48 0xa' \
	'enable=1 active=1 mode: "1920x1080": 60 148500 1920 2008 2052 2200 1080 1084 1089 1125 0x48 0x15' \
	'enable=0 active=0 mode: "1920x1080": 60 148500 1920 2008 2052 2200 1080 1084 1089 1125 0x48 0x5' \
	''; do
	T6_FAKE_VP0_HELD=$vp ck_go
	check "nanopc-t6.sh: VP0 reading back '${vp:-nothing}' is refused, the holder stopped by its pid, Linux's mode back (rc $rc)" \
		"$([ "$rc" = 1 ] && grep -qF "mode: REFUSED: VP0 does not run the mode held (1920x1080 148500 1920 2008 2052 2200 1080 1084 1089 1125 0x5) after the set: '${vp:-unread}'; not jumping" <<<"$out" && ! grep -q 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin' "$tmp/ssh.log" && stopped_4242 && grep -qF 'mode: stop: pid 4242 ended after SIGTERM' <<<"$rec" && grep -qF "mode: Linux's own mode is back on VP0" <<<"$rec"; echo $?)"
done
T6_FAKE_JUMP_RC=98 ck_go
check "nanopc-t6.sh: a jump the board refuses (linux-wdt) stops the holder and Linux's mode comes back (rc $rc)" \
	"$([ "$rc" = 1 ] && [ ! -e "$tmp/jumped" ] && grep -qF 'linux-wdt: REFUSED on the board, no jump' <<<"$rec" && stopped_4242 && grep -qF "mode: Linux's own mode is back on VP0" <<<"$rec"; echo $?)"
# What the stop records about VP0 afterwards: "back" only for a read that
# shows the mode from before the set.
T6_FAKE_JUMP_RC=98 T6_FAKE_VP0_AFTER= ck_go
check "nanopc-t6.sh: a stop after which VP0 cannot be read does not record that Linux's mode is back" \
	"$(stopped_4242 && ! grep -qF "mode: Linux's own mode is back on VP0" <<<"$rec" && grep -qF "mode: VP0 was not read (now 'unread', before the set '$vp0_1080'): nothing says that Linux's own mode is back" <<<"$rec"; echo $?)"
T6_FAKE_JUMP_RC=98 T6_FAKE_VP0_AFTER=$vp0_adopt ck_go
check "nanopc-t6.sh: a stop after which VP0 runs another mode records that it does NOT run Linux's" \
	"$(stopped_4242 && ! grep -qF "mode: Linux's own mode is back on VP0" <<<"$rec" && grep -qF "mode: VP0 does NOT run the mode it ran before the set: now '$vp0_adopt', before '$vp0_1080'" <<<"$rec"; echo $?)"
out=$(t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: the --crash dry run names VP0's adopted mode and the plan holds it (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF "board: VP0 runs an adopted mode, 1920x1080 ($vp0_1080); --go holds it with t6-kms-mode.py, no size change" <<<"$out" && grep -qF 'then the adopt'"'"'s mode: python3 -I /tmp/yakogl-kexec/t6-kms-mode.py hold --mode 1920x1080 --max-hold 600' <<<"$out"; echo $?)"
out=$(t6_vp0=$vp0_720 t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: the --crash dry run on a mode not adopted plans to set 1024x768 (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF "board: VP0 runs '$vp0_720', none of the adopted modes; --go sets 1024x768 first" <<<"$out" && grep -qF 'then the adopt'"'"'s mode: python3 -I /tmp/yakogl-kexec/t6-kms-mode.py hold --mode 1024x768 --max-hold 600' <<<"$out"; echo $?)"
out=$(t6_python3=absent t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: --crash without python3 on the board is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: --crash: the board has no python3' <<<"$out"; echo $?)"
out=$(t6_vp0= t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: --crash without VP0 in the DRM state is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: --crash: no video_port0 in /sys/kernel/debug/dri/0/state' <<<"$out"; echo $?)"
# The screen path (issue #11022). unit is the capture unit's name: it must
# reach `connected` and never the record.
unit=selftest-capture-unit
# no_go: rc 3, refuse's second line, no record, nothing copied or staged,
# no picture asked for.
no_go() {
	[ "$rc" = 3 ] && grep -qF 'nanopc-t6.sh: nothing was copied, loaded or changed.' <<<"$out" &&
		[ -z "$(ls -A "$tmp/rec")" ] && [ ! -e "$tmp/scp.log" ] && ! grep -qF 'mkdir -p /tmp/yakogl-kexec' "$tmp/ssh.log" &&
		[ ! -e "$tmp/connected.log" ]
}
out=$(t6_uart='' t6_screen=$unit t6_go_run "$kernel")
rc=$?
check "nanopc-t6.sh: T6_SCREEN without --crash is refused by name before anything is copied (rc $rc)" \
	"$(no_go && grep -qF 'REFUSED: T6_SCREEN needs --crash: the screen proof runs through the mode holder' <<<"$out"; echo $?)"
out=$(t6_uart='' t6_crash_size=268435456 t6_go_run "$kernel
$ck" --crash)
rc=$?
check "nanopc-t6.sh: --go with neither T6_UART nor T6_SCREEN is refused (rc $rc)" \
	"$(no_go && grep -qF "REFUSED: --go needs T6_UART=<the host tty on the T6's debug UART, or rfc2217://<host>:<port>> (read only), T6_SCREEN=<the capture unit" <<<"$out"; echo $?)"
# sc_go: --crash --go with T6_SCREEN and no T6_UART, the proof bounded by
# $t6_proof_s (default 90); sets out, rc, rd (the record directory), rec.
sc_go() {
	out=$(t6_uart='' t6_screen=$unit t6_crash_size=268435456 t6_go_run "$kernel
$ck" --crash)
	rc=$?
	rd=$(find "$tmp/rec" -mindepth 1 -maxdepth 1 -type d | head -n 1)
	rec=$(cat "$rd/record.txt" 2>/dev/null)
}
# loop_gone: the capture loop's pid (its record line) no longer runs, and
# no fake `connected` of the test is left.
loop_gone() {
	local p q
	p=$(sed -n 's/.* screen: capture loop started (pid \([0-9]*\)).*/\1/p' <<<"$rec")
	[ -n "$p" ] && ! kill -0 "$p" 2>/dev/null || return 1
	for q in $(marked); do
		[[ "$(tr '\0' ' ' <"/proc/$q/cmdline" 2>/dev/null)" == *fakebin/connected* ]] && return 1
	done
	return 0
}
t0=$(date +%s)
sc_go
t1=$(date +%s)
proof_re="screen: proven: picture proof-03.jpg \(request 3, picture 3 that came\) shows this run's nonce ([0-7]{8}) \(proof-03.jpg: 1920x1080: bars ([0-7]{8})\), [0-9]+\.[0-9] s after the hold began"
nonce1= bars1=
if [[ "$rec" =~ $proof_re ]]; then nonce1=${BASH_REMATCH[1]} bars1=${BASH_REMATCH[2]}; fi
check "nanopc-t6.sh: T6_SCREEN only: the holder draws the nonce, the first two pictures are stale, the third proves it, before the jump ($((t1 - t0)) s; nonce ${nonce1:-none}; rc $rc)" \
	"$([ "$rc" = 0 ] && [ -n "$nonce1" ] && [ "$nonce1" = "$bars1" ] && held_and_jumped 1920x1080 "$vp0_1080" &&
		grep -qF "t6-kms-mode.py hold --mode 1920x1080 --max-hold 600 --log /tmp/yakogl-kexec/kms-mode.log --nonce $nonce1" "$tmp/ssh.log" &&
		[ -s "$rd/proof-01.jpg" ] && [ -s "$rd/proof-02.jpg" ] && [ -s "$rd/proof-03.jpg" ] && [ ! -e "$rd/proof-04.jpg" ] &&
		[ "$(grep -n '^.* screen: proven: ' <<<"$rec" | cut -d: -f1)" -lt "$(grep -n ' jumping (linux_wdt' <<<"$rec" | cut -d: -f1)" ]; echo $?)"
check "nanopc-t6.sh: T6_SCREEN only: the record's first lines name the paths, no uart.log and no line about one" \
	"$(sed -n 2p "$rd/record.txt" | grep -qF "paths: no UART (T6_UART not set: no uart.log); screen (T6_SCREEN: proven in the mode step before the load, else no jump; nonce $nonce1)" &&
		grep -qF 'paths proven before the jump: screen' <<<"$rec" && [ ! -e "$rd/uart.log" ] &&
		! grep -q 'uart: proven\|UART capture started\|uart.log: ' <<<"$rec" && grep -qF 'no uart.log (T6_UART not set)' <<<"$rec"; echo $?)"
pics=$(find "$rd" -maxdepth 1 -name 'screen-*.jpg' | wc -l)
check "nanopc-t6.sh: the capture loop takes pictures from the jump on (screen-SSS.jpg: $pics), is stopped by its pid and leaves no request behind" \
	"$([ "$pics" -ge 1 ] && find "$rd" -maxdepth 1 -name 'screen-*.jpg' | grep -qE '/screen-[0-9]{3}\.jpg$' &&
		! find "$rd" -maxdepth 1 -name 'screen-*.jpg' | grep -qvE '/screen-[0-9]{3}\.jpg$' &&
		grep -qE "screen: capture loop pid [0-9]+ stopped: $pics pictures, 0 failed requests" <<<"$rec" && loop_gone; echo $?)"
check "nanopc-t6.sh: every request named the unit, and the record does not ($(sort -u "$tmp/connected.log" 2>/dev/null | tr '\n' ' '))" \
	"$([ "$(sort -u "$tmp/connected.log")" = "$unit" ] && ! grep -rqF "$unit" "$rd"; echo $?)"
T6_FAKE_SCREEN=slow sc_go
check "nanopc-t6.sh: a request in flight when the loop is stopped is stopped with it (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qE 'screen: capture loop pid [0-9]+ stopped: ' <<<"$rec" && loop_gone; echo $?)"
# Not proven: the "mode: REFUSED ... not jumping" path, the holder stopped
# by its pid, Linux's mode back. The proof is bounded at 4 s here.
screen_refused() {
	[ "$rc" = 1 ] && ! grep -q 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin' "$tmp/ssh.log" && stopped_4242 &&
		grep -qF "mode: Linux's own mode is back on VP0" <<<"$rec" && ! grep -qF 'capture loop' <<<"$rec"
}
T6_FAKE_SCREEN=none t6_proof_s=4 sc_go
check "nanopc-t6.sh: a unit that gives no picture: refused before the load, its error text named (rc $rc)" \
	"$(screen_refused && grep -qE "mode: REFUSED: screen path not proven: 0 of [0-9]+ requests to the capture unit \(T6_SCREEN\) gave a picture in [0-9.]+ s since the hold began, none with this run's nonce [0-7]{8}; the unit's last error: rc 1: cube_screenshot failed: failed to capture screenshot \(<T6_SCREEN>\); the last picture: no picture came; not jumping" <<<"$out" &&
		grep -qE '^[0-9-]+ [0-9:]+ proof-01\.jpg: request failed \(rc 1\): cube_screenshot failed' "$rd/screen.log" && ! grep -rqF "$unit" "$rd"; echo $?)"
T6_FAKE_SCREEN=other t6_proof_s=4 sc_go
check "nanopc-t6.sh: pictures of another nonce: refused, the last one's bars named (rc $rc)" \
	"$(screen_refused && grep -qE "mode: REFUSED: screen path not proven: ([0-9]+) of \1 requests .* none with this run's nonce [0-7]{8}; the unit's last error: none; the last picture: proof-[0-9]{2}\.jpg: 1920x1080: bars (00000000|70615243|36104527); not jumping" <<<"$out"; echo $?)"
T6_FAKE_VP0_LINUX=$vp0_720 t6_proof_s=4 sc_go
check "nanopc-t6.sh: 1024x768 set (Linux ran 1280x720), the unit gives no picture of it: refused by the same rule (rc $rc)" \
	"$(screen_refused && grep -qF 'mode: VP0 runs none of the adopted modes: set 1024x768' <<<"$rec" &&
		grep -qE "screen path not proven: 2 of [0-9]+ requests .* the last picture: proof-02\.jpg: 1920x1080: bars 00000000; not jumping" <<<"$out"; echo $?)"
# Both paths: each proven, both named.
out=$(t6_screen=$unit t6_crash_size=268435456 t6_go_run "$kernel
$ck" --crash)
rc=$?
rec=$(cat "$tmp"/rec/*/record.txt 2>/dev/null)
check "nanopc-t6.sh: T6_UART and T6_SCREEN: both proven and named, uart.log kept (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'paths: UART (proven before anything was copied, below); screen (T6_SCREEN:' <<<"$rec" &&
		grep -qF 'uart: proven before anything was copied' <<<"$rec" && grep -qF 'screen: proven: picture proof-03.jpg' <<<"$rec" &&
		grep -qF 'paths proven before the jump: UART and screen' <<<"$rec" && [ -e "$(dirname "$(ls "$tmp"/rec/*/record.txt)")/uart.log" ]; echo $?)"
out=$(T6_SCREEN=$unit t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: the --crash dry run with T6_SCREEN names the proof and the holder's nonce (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'screen: T6_SCREEN is set: --go has the mode holder show this run'"'"'s nonce' <<<"$out" &&
		grep -qF -- '--max-hold 600 --nonce <this run'"'"'s>' <<<"$out" && ! grep -qF 'would refuse' <<<"$out"; echo $?)"
# The CPU clock (issue #11468): t6-cpufreq.sh on the made-up sysfs
# ($cfroot), then nanopc-t6.sh with T6_CPU_KHZ on the fake board, whose ssh
# runs the cf_ commands on that sysfs. cfsh <commands>: t6-cpufreq.sh's
# functions run there, as the board runs them.
cfsh() { T6CF_ROOT=$cfroot sh -c ". '$here/t6-cpufreq.sh'; $1" 2>&1; }
p0=$cfroot/sys/devices/system/cpu/cpufreq/policy0
p0_now() { echo "$(cat "$p0/scaling_min_freq") $(cat "$p0/scaling_max_freq")"; }
p0_line="policy0 (CPUs 0 1 2 3): driver cpufreq-dt, governor ondemand, scaling_cur_freq 408000, scaling_min_freq 408000, scaling_max_freq 1800000, scaling_available_frequencies $cf_steps0, cpuinfo_cur_freq 408000"
# cf_writes <output>: its write lines as "min|max <value>", in order.
cf_writes() { sed -n 's/^cpufreq: write scaling_\(min\|max\)_freq \([0-9]*\)$/\1 \2/p' <<<"$1" | tr '\n' ' ' | sed 's/ $//'; }
# cf_replay <min> <max> <output>: the writes applied in order from min and
# max keep min <= max after each one.
cf_replay() {
	local lo=$1 hi=$2 w v
	while read -r w v; do
		[ -n "$w" ] || continue
		if [ "$w" = min ]; then lo=$v; else hi=$v; fi
		[ "$lo" -le "$hi" ] || return 1
	done < <(sed -n 's/^cpufreq: write scaling_\(min\|max\)_freq \([0-9]*\)$/\1 \2/p' <<<"$3")
}
# A stand-in for the kernel applying new limits: policy0's scaling_cur_freq
# follows its scaling_max_freq every 0.05 s, from gov_start to gov_stop
# (by its pid). Without it scaling_cur_freq stays where it is: a hold that
# does not take.
gov_pid=
gov_start() {
	( while :; do
		cat "$p0/scaling_max_freq" > "$p0/cur.new" 2>/dev/null && mv "$p0/cur.new" "$p0/scaling_cur_freq" 2>/dev/null
		sleep 0.05
	done ) &
	gov_pid=$!
}
gov_stop() {
	[ -n "$gov_pid" ] || return 0
	kill "$gov_pid" 2>/dev/null
	wait "$gov_pid" 2>/dev/null
	gov_pid=
	rm -f "$p0/cur.new"
}
cf_root
out=$(cfsh 'cf_facts crash')
check "t6-cpufreq.sh: the crash route jumps on CPU 0; its policy policy0 and each of its facts are printed" \
	"$(grep -qx 'cpufreq_cpu=0' <<<"$out" && grep -qx 'cpufreq_policy=policy0' <<<"$out" && grep -qxF "cpufreq_steps=$cf_steps0" <<<"$out" &&
		grep -qx 'cpufreq_min=408000' <<<"$out" && grep -qx 'cpufreq_max=1800000' <<<"$out" && grep -qxF "cpufreq_line=$p0_line" <<<"$out"; echo $?)"
echo 6 > "$cfroot/sys/kernel/reboot/cpu"
out=$(cfsh 'cf_facts normal')
check "t6-cpufreq.sh: the normal route jumps on reboot_cpu (6, online): policy6" \
	"$(grep -qx 'cpufreq_cpu=6' <<<"$out" && grep -qx 'cpufreq_policy=policy6' <<<"$out" && grep -qxF 'cpufreq_why=normal route: reboot_cpu 6 (/sys/kernel/reboot/cpu), online (0-7)' <<<"$out" && grep -qF 'cpufreq_line=policy6 (CPUs 6 7): ' <<<"$out"; echo $?)"
out=$(cfsh 'cf_facts crash')
check "t6-cpufreq.sh: the crash route stays on CPU 0 whatever reboot_cpu is" \
	"$(grep -qx 'cpufreq_cpu=0' <<<"$out" && grep -qx 'cpufreq_policy=policy0' <<<"$out"; echo $?)"
echo 0-3 > "$cfroot/sys/devices/system/cpu/online"
out=$(cfsh 'cf_facts normal')
check "t6-cpufreq.sh: reboot_cpu offline: the first online CPU, as migrate_to_reboot_cpu takes it" \
	"$(grep -qx 'cpufreq_cpu=0' <<<"$out" && grep -qxF 'cpufreq_why=normal route: reboot_cpu 6 is offline, so the first online CPU (0-3)' <<<"$out"; echo $?)"
rm "$cfroot/sys/kernel/reboot/cpu"
out=$(cfsh 'cf_facts normal')
check "t6-cpufreq.sh: no /sys/kernel/reboot/cpu: the CPU is not known, every policy is printed" \
	"$(grep -qx 'cpufreq_cpu=' <<<"$out" && grep -qx 'cpufreq_policy=absent' <<<"$out" &&
		grep -qE '^cpufreq_line=every policy: policy0 \(CPUs 0 1 2 3\): .*; policy4 \(CPUs 4 5\): .*; policy6 \(CPUs 6 7\): .*cpuinfo_cur_freq 408000$' <<<"$out"; echo $?)"
out=$(cfsh 'cf_hold normal 1800000 3')
rc=$?
check "t6-cpufreq.sh: cf_hold refuses when the jump CPU is not known, before any write (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'cpufreq: REFUSED: the jump CPU or its policy is not known' <<<"$out" && [ -z "$(cf_writes "$out")" ] && [ "$(p0_now)" = '408000 1800000' ]; echo $?)"
cf_root 600000 1008000 600000
out=$(cfsh "cf_set '$p0' 1800000 1800000")
rc=$?
check "t6-cpufreq.sh: a step above the range: scaling_max_freq first, min <= max after each write ($(cf_writes "$out"); rc $rc)" \
	"$([ "$rc" = 0 ] && [ "$(cf_writes "$out")" = 'max 1800000 min 1800000' ] && cf_replay 600000 1008000 "$out" && [ "$(p0_now)" = '1800000 1800000' ]; echo $?)"
cf_root 600000 1008000 600000
out=$(cfsh "cf_set '$p0' 408000 408000")
rc=$?
check "t6-cpufreq.sh: a step below the range: scaling_min_freq first, min <= max after each write ($(cf_writes "$out"); rc $rc)" \
	"$([ "$rc" = 0 ] && [ "$(cf_writes "$out")" = 'min 408000 max 408000' ] && cf_replay 600000 1008000 "$out" && [ "$(p0_now)" = '408000 408000' ]; echo $?)"
cf_root 600000 1008000 600000
out=$(cfsh "cf_set '$p0' 816000 816000")
check "t6-cpufreq.sh: a step inside the range: scaling_min_freq first ($(cf_writes "$out"))" \
	"$([ "$(cf_writes "$out")" = 'min 816000 max 816000' ] && cf_replay 600000 1008000 "$out"; echo $?)"
cf_root
out=$(cfsh 'cf_hold normal 1234 3')
rc=$?
check "t6-cpufreq.sh: cf_hold refuses a value outside the steps and lists them, before any write (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qxF "cpufreq: REFUSED: 1234 kHz is not one of policy0's steps ($cf_steps0)" <<<"$out" && [ -z "$(cf_writes "$out")" ] && [ "$(p0_now)" = '408000 1800000' ]; echo $?)"
cf_root 600000 1008000 600000
out=$(cfsh 'cf_hold normal 1800000 3')
rc=$?
check "t6-cpufreq.sh: a hold whose read-back never comes: the old min and max written back, min <= max throughout (rc $rc; $(cf_writes "$out"))" \
	"$([ "$rc" = 1 ] && grep -qxF "cpufreq: scaling_cur_freq reads '600000', not 1800000, 3 tenths of a second after the writes" <<<"$out" &&
		[ "$(cf_writes "$out")" = 'max 1800000 min 1800000 min 600000 max 1008000' ] && cf_replay 600000 1008000 "$out" &&
		grep -qx 'cpufreq_restored=600000 1008000' <<<"$out" && [ "$(p0_now)" = '600000 1008000' ]; echo $?)"
cf_root 600000 1008000 600000
gov_start
out=$(cfsh 'cf_hold normal 408000 50')
rc=$?
gov_stop
check "t6-cpufreq.sh: a hold that takes: the read-back named, min and max at the step (rc $rc; $(grep '^cpufreq_held=' <<<"$out"))" \
	"$([ "$rc" = 0 ] && grep -qE '^cpufreq_held=408000 [0-9]+ 408000$' <<<"$out" && grep -qx 'cpufreq_old=600000 1008000' <<<"$out" &&
		[ "$(cf_writes "$out")" = 'min 408000 max 408000' ] && [ "$(p0_now)" = '408000 408000' ]; echo $?)"
out=$(cfsh 'cf_restore policy0 600000 1008000 boot-another')
rc=$?
check "t6-cpufreq.sh: the undo writes nothing in another Linux (another boot_id; rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'another Linux runs' <<<"$out" && [ -z "$(cf_writes "$out")" ] && [ "$(p0_now)" = '408000 408000' ]; echo $?)"
out=$(cfsh 'cf_restore policy0 600000 1008000 boot-before')
rc=$?
check "t6-cpufreq.sh: the undo in the Linux that made the hold writes the old values back (rc $rc; $(cf_writes "$out"))" \
	"$([ "$rc" = 0 ] && [ "$(cf_writes "$out")" = 'max 1008000 min 600000' ] && cf_replay 408000 408000 "$out" && [ "$(p0_now)" = '600000 1008000' ]; echo $?)"

# nanopc-t6.sh: the dry run.
cf_root
rm -f "$tmp/ssh.log"
out=$(t6_board_run "$kernel" "$gic_out")
rc=$?
check "nanopc-t6.sh: the dry run names the jump CPU and its policy's facts, and without T6_CPU_KHZ writes nothing (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qxF "cpufreq: the jump runs on CPU 0 (normal route: reboot_cpu 0 (/sys/kernel/reboot/cpu), online (0-7)); $p0_line" <<<"$out" &&
		grep -qF 'cpufreq: T6_CPU_KHZ is not set: --go records the clock before the jump and writes nothing' <<<"$out" &&
		! grep -qE '^cf_hold ' "$tmp/ssh.log" && [ "$(p0_now)" = '408000 1800000' ]; echo $?)"
echo 6 > "$cfroot/sys/kernel/reboot/cpu"
out=$(t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash)
check "nanopc-t6.sh: the --crash dry run names CPU 0 and policy0 whatever reboot_cpu is" \
	"$(grep -qxF "cpufreq: the jump runs on CPU 0 (crash route: kexec_zephyr panics only with CPU 0 the one online CPU); $p0_line" <<<"$out"; echo $?)"
cf_root
for v in 1234 1.8GHz '408000 600000'; do
	out=$(T6_CPU_KHZ=$v t6_board_run "$kernel" "$gic_out")
	rc=$?
	check "nanopc-t6.sh: T6_CPU_KHZ='$v' is refused before any change, the steps listed (rc $rc)" \
		"$([ "$rc" = 3 ] && grep -qF "REFUSED: T6_CPU_KHZ=$v is not one of policy0's steps (scaling_available_frequencies, kHz): $cf_steps0" <<<"$out" &&
			grep -qF 'nanopc-t6.sh: nothing was copied, loaded or changed.' <<<"$out" && ! grep -qE '^cf_hold ' "$tmp/ssh.log" && [ "$(p0_now)" = '408000 1800000' ]; echo $?)"
done
rm "$cfroot/sys/kernel/reboot/cpu"
out=$(T6_CPU_KHZ=1800000 t6_board_run "$kernel" "$gic_out")
rc=$?
check "nanopc-t6.sh: T6_CPU_KHZ with the jump CPU not known is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: T6_CPU_KHZ=1800000: the cpufreq policy of the CPU the jump runs on is not known: the jump CPU is not known (normal route: /sys/kernel/reboot/cpu unreadable' <<<"$out"; echo $?)"
cf_root
out=$(T6_CPU_KHZ=1800000 t6_board_run "$kernel" "$gic_out")
rc=$?
check "nanopc-t6.sh: the dry run with T6_CPU_KHZ a step names the hold and the plan holds it (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'cpufreq: T6_CPU_KHZ=1800000: --go holds policy0 at 1800000 kHz right before the jump, and writes its scaling_min_freq 408000 and scaling_max_freq 1800000 back when there is no jump' <<<"$out" &&
		grep -qF "then the CPU clock: policy0's cpufreq facts into the record; policy0 held at" <<<"$out" && [ "$(p0_now)" = '408000 1800000' ]; echo $?)"

# nanopc-t6.sh --go on the fake board. cf_go: the normal route; sets out,
# rc, rec.
cf_go() {
	out=$(t6_go_run "$kernel")
	rc=$?
	rec=$(cat "$tmp"/rec/*/record.txt 2>/dev/null)
}
# line_of <pattern> <text>: the number of the first line of text that has it.
line_of() { grep -nF -- "$1" <<<"$2" | head -n 1 | cut -d: -f1; }
cf_root
cf_go
before_at=$(line_of 'cpufreq: before the jump: ' "$rec")
jump_at=$(line_of ' jumping (linux_wdt' "$rec")
check "nanopc-t6.sh: --go without T6_CPU_KHZ records the clock before the jump and writes nothing (rc $rc; lines ${before_at:-none} < ${jump_at:-none})" \
	"$([ "$rc" = 0 ] && grep -qF "cpufreq: before the jump: the jump runs on CPU 0 (normal route: reboot_cpu 0 (/sys/kernel/reboot/cpu), online (0-7)); $p0_line" <<<"$rec" &&
		[ -n "$before_at" ] && [ -n "$jump_at" ] && [ "$before_at" -lt "$jump_at" ] &&
		! grep -qE '^cf_(hold|restore) ' "$tmp/ssh.log" && ! grep -qF 'cpufreq: hold' <<<"$rec" && [ "$(p0_now)" = '408000 1800000' ]; echo $?)"
# The jump command: the fake ssh's log entry (up to its ---- line) that
# runs kexec_zephyr.
jcmd=$(awk 'BEGIN { RS = "\n----\n" } /kexec_zephyr \/tmp\/yakogl-kexec\/zephyr.bin/ { print; exit }' "$tmp/ssh.log")
at_jump=$(line_of 'echo "cpufreq: at the jump: policy0 scaling_cur_freq $(cat /sys/devices/system/cpu/cpufreq/policy0/scaling_cur_freq 2>/dev/null || echo unread)"' "$jcmd")
kz_at=$(line_of 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin' "$jcmd")
check "nanopc-t6.sh: the jump command reads the jump CPU's scaling_cur_freq right before kexec_zephyr (its lines ${at_jump:-none} < ${kz_at:-none})" \
	"$([ -n "$at_jump" ] && [ -n "$kz_at" ] && [ "$at_jump" -lt "$kz_at" ]; echo $?)"
cf_root 600000 1008000 600000
gov_start
T6_CPU_KHZ=1800000 cf_go
gov_stop
hold_at=$(grep -nx 'cf_hold normal 1800000 50' "$tmp/ssh.log" | head -n 1 | cut -d: -f1)
kz_at=$(grep -n 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin' "$tmp/ssh.log" | head -n 1 | cut -d: -f1)
check "nanopc-t6.sh: T6_CPU_KHZ above the range: max then min, read back, recorded, held before the jump (rc $rc; ssh lines ${hold_at:-none} < ${kz_at:-none})" \
	"$([ "$rc" = 0 ] && grep -qF 'cpufreq: hold 1800000 kHz on policy0 (scaling_min_freq and scaling_max_freq were 600000 1008000): write scaling_max_freq 1800000; write scaling_min_freq 1800000 (rc 0)' <<<"$rec" &&
		grep -qE "cpufreq: held: policy0's scaling_cur_freq reads 1800000 kHz [0-9]+\.[0-9] s after the writes, cpuinfo_cur_freq 408000; scaling_min_freq and scaling_max_freq go back to 600000 1008000 if there is no jump" <<<"$rec" &&
		[ -n "$hold_at" ] && [ -n "$kz_at" ] && [ "$hold_at" -lt "$kz_at" ] && [ -e "$tmp/jumped" ]; echo $?)"
check "nanopc-t6.sh: after a jump the hold is not undone: the Linux that made it is gone" \
	"$(grep -qF 'cpufreq: the hold of policy0 ended with the Linux that made it (the board came back with a new boot_id)' <<<"$rec" && ! grep -qE '^cf_restore ' "$tmp/ssh.log" && [ "$(p0_now)" = '1800000 1800000' ]; echo $?)"
cf_root 600000 1008000 600000
gov_start
T6_CPU_KHZ=408000 cf_go
gov_stop
check "nanopc-t6.sh: T6_CPU_KHZ below the range: min then max (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'cpufreq: hold 408000 kHz on policy0 (scaling_min_freq and scaling_max_freq were 600000 1008000): write scaling_min_freq 408000; write scaling_max_freq 408000 (rc 0)' <<<"$rec" && [ "$(p0_now)" = '408000 408000' ]; echo $?)"
cf_root 600000 1008000 600000
T6_CPU_KHZ=1800000 cf_go
check "nanopc-t6.sh: a hold whose read-back never comes: no jump, the board wrote the old min and max back (rc $rc)" \
	"$([ "$rc" = 1 ] && grep -qF 'cpufreq: REFUSED: policy0 did not take 1800000 kHz; the board wrote scaling_min_freq and scaling_max_freq back (600000 1008000); not jumping' <<<"$rec" &&
		grep -qF "scaling_cur_freq reads '600000', not 1800000, 50 tenths of a second after the writes" <<<"$rec" &&
		! grep -q 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin' "$tmp/ssh.log" && [ "$(p0_now)" = '600000 1008000' ]; echo $?)"
cf_root 600000 1008000 600000
gov_start
T6_CPU_KHZ=1800000 T6_FAKE_JUMP_RC=98 cf_go
gov_stop
check "nanopc-t6.sh: a jump the board refuses after the hold: the old min and max written back (rc $rc)" \
	"$([ "$rc" = 1 ] && grep -qF 'linux-wdt: REFUSED on the board, no jump' <<<"$rec" && grep -qx 'cf_restore policy0 600000 1008000 boot-before' "$tmp/ssh.log" &&
		grep -qF 'cpufreq: undo: cpufreq: write scaling_min_freq 600000 cpufreq: write scaling_max_freq 1008000 cpufreq_restored=600000 1008000' <<<"$rec" && [ "$(p0_now)" = '600000 1008000' ]; echo $?)"
cf_root
T6_CPU_KHZ=1234 cf_go
check "nanopc-t6.sh: --go with T6_CPU_KHZ outside the steps is refused before the lock, nothing copied (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: T6_CPU_KHZ=1234 is not one of' <<<"$out" && [ -z "$(ls -A "$tmp/rec")" ] && [ ! -e "$tmp/scp.log" ] && ! grep -qE '^cf_hold ' "$tmp/ssh.log"; echo $?)"
cf_root
# t6-screen-nonce.py's own self-test. Its JPEG round trip needs cjpeg and
# djpeg on this host; without them it is skipped, and the last line says so.
got=$(python3 -I "$here/t6-screen-nonce.py" --self-test 2>&1)
rc=$?
[ "$rc" = 0 ] || grep -E ' FAIL$' <<<"$got"
check "t6-screen-nonce.py --self-test (rc $rc: $(tail -n 1 <<<"$got"))" \
	"$([ "$rc" = 0 ] && tail -n 1 <<<"$got" | grep -qE '^t6-screen-nonce self-test: [0-9]+ checks run, 0 failed(, skipped: the JPEG round trip \(no [a-z ]+ on this host\))?$'; echo $?)"
# vp0_mode, the board's read of VP0's mode, on a DRM state in the format of
# linux v6.18 drm_atomic.c:443-455 (planes first, then the CRTCs by id, then
# the connectors): VP0's lines only, also with VP1 printed before it.
vp0_fn=$(awk '/^VP0_FN=\$\(cat <<.EOF.$/ { n = 1; next } n && /^EOF$/ { exit } n' "$t6" | sed "s|/sys/kernel/debug/dri/0/state|$tmp/drm-state|")
drm_crtc() { # <id> <port> <on> <mode line>
	printf 'crtc[%s]: video_port%s\n\tenable=%s\n\tactive=%s\n\tself_refresh_active=0\n\tplanes_changed=0\n\tmode_changed=0\n' "$1" "$2" "$3" "$3"
	printf '\tactive_changed=0\n\tconnectors_changed=0\n\tcolor_mgmt_changed=0\n\tplane_mask=%s\n\tconnector_mask=%s\n\tencoder_mask=%s\n\tmode: %s\n' "$3" "$3" "$3" "$4"
}
drm_state() { # <crtc blocks>
	printf 'plane[33]: Cluster0-win0\n\tcrtc=video_port0\n\tfb=60\n\tmode: not a crtc line\n%s\nconnector[83]: HDMI-A-1\n\tcrtc=video_port0\n\toutput_bpc=8\n' "$1" > "$tmp/drm-state"
}
off='"": 0 0 0 0 0 0 0 0 0 0 0x0 0x0'
drm_state "$(drm_crtc 81 0 1 '"1024x768": 60 65000 1024 1048 1184 1344 768 771 777 806 0x40 0xa'; drm_crtc 84 1 0 "$off")"
got=$(bash -c "$vp0_fn
vp0_mode")
check "nanopc-t6.sh: vp0_mode reads VP0's lines of the DRM state ('$got')" \
	"$([ -n "$vp0_fn" ] && [ "$got" = "$vp0_adopt" ]; echo $?)"
drm_state "$(drm_crtc 80 1 1 '"1024x768": 60 65000 1024 1048 1184 1344 768 771 777 806 0x40 0xa'; drm_crtc 81 0 0 "$off")"
got=$(bash -c "$vp0_fn
vp0_mode")
check "nanopc-t6.sh: vp0_mode reads VP0, not VP1 printed before it ('$got')" \
	"$([ "$got" = 'enable=0 active=0 mode: "": 0 0 0 0 0 0 0 0 0 0 0x0 0x0' ]; echo $?)"
# The holder's stop as the board runs it (MODE_STOP of nanopc-t6.sh; the
# fake ssh above only logs it), here, on a stand-in holder: a python3 with
# the holder's command line that writes its stop line to the log on SIGTERM.
# It kills that pid and no other, waits for it (a zombie counts as ended),
# and does nothing in another Linux.
mode_stop_sh=$(awk '/^MODE_STOP=\$\(cat <<.EOF.$/ { n = 1; next } n && /^EOF$/ { exit } n' "$t6")
stage=$tmp/stage
mkdir "$stage"
cat > "$stage/t6-kms-mode.py" <<'EOS'
import signal, sys, time
def stop(*_):
    with open(sys.argv[5], "a") as f:
        f.write("t6-kms-mode: stand-in: stop (signal SIGTERM)\n")
    sys.exit(0)
signal.signal(signal.SIGTERM, stop)
time.sleep(60)
EOS
boot=$(cat /proc/sys/kernel/random/boot_id)
run_stop() { bash -c "vp0_mode() { echo stand-in; }
set -- $1 $2 $stage
$mode_stop_sh" 2>&1; }
python3 -I "$stage/t6-kms-mode.py" hold --max-hold 600 --log "$stage/kms-mode.log" &
sp=$!
for _ in $(seq 50); do [[ "$(tr '\0' ' ' <"/proc/$sp/cmdline" 2>/dev/null)" == *t6-kms-mode.py* ]] && break; sleep 0.1; done
sleep 60 &
other=$!
out=$(run_stop "$other" "$boot")
check "nanopc-t6.sh: the board's stop kills no pid that is not the holder ($(head -n 1 <<<"$out"))" \
	"$([ -n "$mode_stop_sh" ] && grep -qF "pid $other is not the holder any more" <<<"$out" && kill -0 "$other" 2>/dev/null; echo $?)"
kill "$other" 2>/dev/null
wait "$other" 2>/dev/null
out=$(run_stop "$sp" other-boot-id)
check "nanopc-t6.sh: the board's stop in another Linux kills nothing ($(head -n 1 <<<"$out"))" \
	"$(grep -qF "another Linux runs (boot_id changed): pid $sp ended with the one that ran it" <<<"$out" && kill -0 "$sp" 2>/dev/null; echo $?)"
out=$(run_stop "$sp" "$boot")
check "nanopc-t6.sh: the board's stop ends the holder by its pid and prints its last line and VP0 ($(tr '\n' ';' <<<"$out"))" \
	"$(grep -qx "pid $sp ended after SIGTERM" <<<"$out" && grep -qx 't6-kms-mode: stand-in: stop (signal SIGTERM)' <<<"$out" && grep -qx 'vp0=stand-in' <<<"$out"; echo $?)"
kill "$sp" 2>/dev/null
wait "$sp" 2>/dev/null
# ADOPT_MODES are the Rust MODES the image adopts, in their order: the
# name (the mode's size), h = active, +front, +sync, +back; the same for v;
# the clock in kHz; the sync flags. Each Rust mode is a VideoMode literal
# in the same file.
rs=$here/../../rust/src
want=
for mode_name in $(sed -n 's/^pub const MODES: \[VideoMode; [0-9]*\] = \[\(.*\)\];$/\1/p' "$rs/board/nanopc_t6_display.rs" | tr -d ,); do
	mode_rs=$(awk -v first="pub const $mode_name: VideoMode = VideoMode {" '$0 == first { n = 1; next } n && /^\};/ { exit } n { gsub(/[ ,]/, ""); print }' "$rs/board/nanopc_t6_display.rs")
	rsv() { sed -n "s/^$1://p" <<<"$mode_rs" | tr -d _; }
	if [ -z "$(rsv vback)" ]; then
		want=
		break
	fi
	hflag=2 vflag=8 # DRM_MODE_FLAG_NHSYNC, _NVSYNC; 1 and 4 are the positive ones
	if [ "$(rsv hsync_high)" = true ]; then hflag=1; fi
	if [ "$(rsv vsync_high)" = true ]; then vflag=4; fi
	want+="${want:+
}$(rsv hactive)x$(rsv vactive) $(($(rsv pixel_clock_hz) / 1000)) $(rsv hactive) $(($(rsv hactive) + $(rsv hfront))) $(($(rsv hactive) + $(rsv hfront) + $(rsv hsync))) $(($(rsv hactive) + $(rsv hfront) + $(rsv hsync) + $(rsv hback)))"
	want+=" $(rsv vactive) $(($(rsv vactive) + $(rsv vfront))) $(($(rsv vactive) + $(rsv vfront) + $(rsv vsync))) $(($(rsv vactive) + $(rsv vfront) + $(rsv vsync) + $(rsv vback))) $(printf '0x%x' $((hflag | vflag)))"
done
got=$(awk '/^ADOPT_MODES="/ { n = 1; sub(/^ADOPT_MODES="/, "") } n { if (sub(/"$/, "")) { print; exit } print }' "$t6")
check "nanopc-t6.sh: ADOPT_MODES ($(tr '\n' ';' <<<"$got")) are board::nanopc_t6_display::MODES ($(tr '\n' ';' <<<"${want:-unread}"))" \
	"$([ "$(wc -l <<<"$want")" = 2 ] && [ "$got" = "$want" ]; echo $?)"
# t6-kms-mode.py's own self-test: the uapi numbers, the mode rule, the hold
# and give-back order on a made-up card, the holder's stop.
got=$(python3 -I "$here/t6-kms-mode.py" --self-test 2>&1)
rc=$?
[ "$rc" = 0 ] || grep -E ' FAIL$' <<<"$got"
check "t6-kms-mode.py --self-test (rc $rc: $(tail -n 1 <<<"$got"))" \
	"$([ "$rc" = 0 ] && tail -n 1 <<<"$got" | grep -qE '^t6-kms-mode self-test: [0-9]+ checks run, 0 failed$'; echo $?)"
# The read under the lock: a crash image loaded between the dry checks and
# the lock stops the jump, on the crash route and on the normal route in
# the reservation.
out=$(t6_fresh_crash=1 t6_crash_size=268435456 t6_go_run "$kernel
$ck" --crash)
rc=$?
check "nanopc-t6.sh: --crash --go refuses a crash image loaded before the lock was taken (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: under the lock: a crash image is already loaded' <<<"$out" && ! grep -q 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin' "$tmp/ssh.log"; echo $?)"
out=$(t6_fresh_crash=1 t6_crash_size=268435456 t6_go_run "$kernel
$ck")
rc=$?
check "nanopc-t6.sh: --go on the normal route in the reservation refuses the same under the lock (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: under the lock: a crash image is already loaded' <<<"$out" && ! grep -q 'kexec_zephyr /tmp/yakogl-kexec/zephyr.bin' "$tmp/ssh.log"; echo $?)"
out=$(t6_crash_size=268435456 t6_board_run "$kernel
$ck
  1ff00000-1fffffff : reserved" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: --crash still refuses another entry in the window (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: the window overlaps: 1ff00000-1fffffff : reserved' <<<"$out"; echo $?)"
out=$(t6_crash_dump=absent t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: --crash on a kernel without CONFIG_CRASH_DUMP=y is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: --crash: the kernel 6.18.55 has absent, so it has no crash route' <<<"$out"; echo $?)"
out=$(t6_crash_size=266338304 t6_board_run "$kernel
  10200000-1fffffff : Crash kernel" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: --crash with a reservation that starts 2 MiB into the window is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF "the board has: Crash kernel '10200000-1fffffff : Crash kernel'" <<<"$out"; echo $?)"
# The trigger is checked before the jump: kexec_zephyr takes every link down
# before it writes it, so a trigger that is missing would leave the board
# without network.
out=$(t6_sysrq=absent t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: --crash without a writable /proc/sysrq-trigger is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: --crash: /proc/sysrq-trigger is absent' <<<"$out"; echo $?)"
# --crash without Linux's watchdog: if the crash kexec does not happen,
# panic() waits in its timeout loop (kernel/panic.c:508-509, :557), and
# with panic_timeout 0 nothing ever resets the board.
out=$(t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash --no-linux-wdt)
rc=$?
check "nanopc-t6.sh: --crash with --no-linux-wdt is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: --crash needs the Linux watchdog' <<<"$out"; echo $?)"
# An image whose RAM footprint (header bytes 16..23) is the whole window
# leaves no page of the reservation for the control page.
cp "$tmp/top13.bin" "$tmp/full.bin"
printf '\x00\x00\x00\x10\x00\x00\x00\x00' | dd of="$tmp/full.bin" bs=1 seek=16 conv=notrunc status=none
out=$(t6_img=$tmp/full.bin t6_crash_size=268435456 t6_board_run "$kernel
$ck" "$gic_out" --crash)
rc=$?
check "nanopc-t6.sh: --crash refuses a segment that leaves no room for the control page (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF "REFUSED: --crash: the image's segment (268435456 bytes at 0x10000000)" <<<"$out"; echo $?)"

# t6-crashkernel.sh on a fake board: a directory with the board's files, and
# an `ssh` (and a `dmesg`) that run here.
ckt=$here/t6-crashkernel.sh
fb=$tmp/ckboard
mkdir -p "$tmp/fakessh" "$fb/boot" "$fb/proc/device-tree" "$fb/proc/sys/kernel" "$fb/sys/kernel" "$fb/root"
printf '#!/bin/sh\nfor a; do last=$a; done\nexec sh -c "$last"\n' > "$tmp/fakessh/ssh"
printf '#!/bin/sh\ncat "%s/dmesg"\n' "$fb" > "$tmp/fakessh/dmesg"
# sync: counts its calls, logs whether set's new file is still beside the
# file and the file's sha256, and at call $CK_KILL_AT kills the board
# script that called it (a set or undo cut there).
cat > "$tmp/fakessh/sync" <<'EOS'
#!/bin/sh
n=$(( $(cat "$CK_SYNC_N" 2>/dev/null || echo 0) + 1 ))
echo "$n" > "$CK_SYNC_N"
t=no
[ -e "$CK_FILE.yakogl-new" ] && t=yes
echo "sync $n: new-file $t, file $(sha256sum < "$CK_FILE" | cut -d' ' -f1)" >> "$CK_SYNC_LOG"
[ "$n" = "${CK_KILL_AT:-0}" ] && kill -9 "$PPID"
exit 0
EOS
chmod +x "$tmp/fakessh/ssh" "$tmp/fakessh/dmesg" "$tmp/fakessh/sync"
export CK_SYNC_N=$tmp/ck-sync.n CK_SYNC_LOG=$tmp/ck-sync.log CK_FILE=$fb/boot/armbianEnv.txt
printf 'friendlyarm,nanopc-t6\0rockchip,rk3588\0' > "$fb/proc/device-tree/compatible"
echo 6.18.55 > "$fb/proc/sys/kernel/osrelease"
printf 'CONFIG_KEXEC=y\nCONFIG_CRASH_RESERVE=y\nCONFIG_CRASH_DUMP=y\n' > "$fb/boot/config-6.18.55"
# The root UUID is on the board's command line, in dmesg's "Kernel command
# line" and in armbianEnv.txt: no output may show it (the end checks).
uuid=deadbeef-0000-4000-8000-0123456789ab
printf 'root=UUID=%s rootwait cma=256M\n' "$uuid" > "$fb/proc/cmdline"
echo 0 > "$fb/sys/kernel/kexec_crash_size"
echo 0 > "$fb/sys/kernel/kexec_crash_loaded"
printf '00200000-e143ffff : System RAM\n  02000000-0432ffff : Kernel code\n' > "$fb/proc/iomem"
printf '%s\n' "Kernel command line: root=UUID=$uuid rootwait cma=256M" \
	'cma: Reserved 256 MiB at 0x00000000d1400000' > "$fb/dmesg"
ckrun() { # <command>; every output is kept for the UUID check at the end
	PATH=$tmp/fakessh:$PATH T6CK_ROOT=$fb BOARD_LOCK=$tmp/ck.lock BOARD_HOST=root@board.invalid bash "$ckt" "$1" 2>&1 |
		tee -a "$tmp/ck-all.out"
	return "${PIPESTATUS[0]}"
}
env_with="verbosity=1
rootdev=UUID=$uuid
bootlogo=false
overlay_prefix=rockchip-rk3588
extraargs=cma=256M
usbstoragequirks=0x2537:0x1066:u
"
printf '%s' "$env_with" > "$fb/boot/armbianEnv.txt"
before=$(sha256sum < "$fb/boot/armbianEnv.txt")
out=$(ckrun status)
rc=$?
check "t6-crashkernel.sh: status without a reservation exits 1 and says so (rc $rc)" \
	"$([ "$rc" = 1 ] && grep -qF 'is NOT reserved (no crashkernel=)' <<<"$out" && grep -qF 'cma: Reserved 256 MiB' <<<"$out"; echo $?)"
# The words of the extraargs= line include its first one (extraargs=cma=...).
check "t6-crashkernel.sh: status prints the first word of the extraargs= line ($(grep -F 'extraargs line' <<<"$out"))" \
	"$(grep -qF 'extraargs line: line 5: cma=256M' <<<"$out"; echo $?)"
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: set prints the words of the line before and after, the first one included" \
	"$(grep -qx 'board: extraargs before (crashkernel=, cma= words): cma=256M ' <<<"$out" && grep -qx 'board: extraargs after  (crashkernel=, cma= words): cma=256M crashkernel=256M@0x10000000 ' <<<"$out"; echo $?)"
check "t6-crashkernel.sh: set appends the token to the extraargs= line only (rc $rc)" \
	"$([ "$rc" = 0 ] && [ "$(grep -v '^extraargs=' "$fb/boot/armbianEnv.txt")" = "$(grep -v '^extraargs=' <<<"$env_with")" ] && grep -qx 'extraargs=cma=256M crashkernel=256M@0x10000000' "$fb/boot/armbianEnv.txt" && grep -qF 'next: reboot the board' <<<"$out"; echo $?)"
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: a second set changes nothing (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'already set by this tool' <<<"$out" && [ "$(grep -c crashkernel= "$fb/boot/armbianEnv.txt")" = 1 ]; echo $?)"
out=$(ckrun status)
rc=$?
check "t6-crashkernel.sh: status after set, before the reboot, says reboot (rc $rc)" \
	"$([ "$rc" = 1 ] && grep -qF 'the file has crashkernel= but the running kernel was not given it: reboot' <<<"$out"; echo $?)"
printf 'root=UUID=%s rootwait cma=256M crashkernel=256M@0x10000000\n' "$uuid" > "$fb/proc/cmdline"
echo "Kernel command line: root=UUID=$uuid rootwait cma=256M crashkernel=256M@0x10000000" >> "$fb/dmesg"
printf '00200000-e143ffff : System RAM\n  02000000-0432ffff : Kernel code\n  10000000-1fffffff : Crash kernel\n' > "$fb/proc/iomem"
out=$(ckrun status)
rc=$?
check "t6-crashkernel.sh: status after the reboot finds the window in the reservation (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'is inside the reservation: nanopc-t6.sh --crash can run' <<<"$out"; echo $?)"
printf '00200000-e143ffff : System RAM\n  02000000-0432ffff : Kernel code\n  10200000-2fffffff : Crash kernel\n' > "$fb/proc/iomem"
out=$(ckrun status)
rc=$?
check "t6-crashkernel.sh: status does not take a reservation that starts 2 MiB into the window as holding it (rc $rc)" \
	"$([ "$rc" = 1 ] && grep -qF 'is NOT reserved although the kernel was given' <<<"$out"; echo $?)"
printf '00200000-e143ffff : System RAM\n' > "$fb/proc/iomem"
echo 'crashkernel reservation failed - memory is in use.' >> "$fb/dmesg"
out=$(ckrun status)
rc=$?
check "t6-crashkernel.sh: status names a reservation the kernel could not make (rc $rc)" \
	"$([ "$rc" = 1 ] && grep -qF 'NOT reserved although the kernel was given' <<<"$out" && grep -qF 'memory is in use' <<<"$out"; echo $?)"
out=$(ckrun undo)
rc=$?
check "t6-crashkernel.sh: undo restores the file byte for byte and removes its copy (rc $rc)" \
	"$([ "$rc" = 0 ] && [ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$before" ] && [ ! -e "$fb/root/yakogl-crashkernel" ] && grep -qF 'byte for byte' <<<"$out"; echo $?)"
# No extraargs= line: set adds one at the end; undo takes it away again.
printf 'verbosity=1\nrootdev=UUID=0\n' > "$fb/boot/armbianEnv.txt"
before=$(sha256sum < "$fb/boot/armbianEnv.txt")
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: set on a file without extraargs= adds the line at the end (rc $rc)" \
	"$([ "$rc" = 0 ] && [ "$(cat "$fb/boot/armbianEnv.txt")" = "verbosity=1
rootdev=UUID=0
extraargs=crashkernel=256M@0x10000000" ]; echo $?)"
check "t6-crashkernel.sh: ... and status names the crashkernel= of the line set added (the line's first word)" \
	"$(ckrun status | grep -qF 'crashkernel= in it: line 3: crashkernel=256M@0x10000000'; echo $?)"
out=$(ckrun undo)
rc=$?
check "t6-crashkernel.sh: undo on it is byte for byte the old file (rc $rc)" \
	"$([ "$rc" = 0 ] && [ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$before" ]; echo $?)"
# Refusals leave the file as it was.
printf 'extraargs=root=UUID=%s cma=256M crashkernel=512M\n' "$uuid" > "$fb/boot/armbianEnv.txt"
before=$(sha256sum < "$fb/boot/armbianEnv.txt")
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: set refuses a file that already has a crashkernel= (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'already has a crashkernel=' <<<"$out" && [ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$before" ] && [ ! -e "$fb/root/yakogl-crashkernel" ]; echo $?)"
printf 'extraargs=a\nextraargs=b\n' > "$fb/boot/armbianEnv.txt"
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: set refuses two extraargs= lines (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'has 2 extraargs= lines' <<<"$out"; echo $?)"
# U-Boot's env import skips blanks before a name (lib/hashtable.c:902-904,
# v2026.07), so " extraargs=" is the live line: set edits it, keeping its
# blanks, and adds no second line (a later one would replace it).
printf 'verbosity=1\n \textraargs=cma=256M\nrootdev=UUID=%s\n' "$uuid" > "$fb/boot/armbianEnv.txt"
before=$(sha256sum < "$fb/boot/armbianEnv.txt")
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: set edits a live extraargs= line with leading blanks and adds none (rc $rc)" \
	"$([ "$rc" = 0 ] && [ "$(grep -c 'extraargs=' "$fb/boot/armbianEnv.txt")" = 1 ] && grep -qx ' 	extraargs=cma=256M crashkernel=256M@0x10000000' "$fb/boot/armbianEnv.txt"; echo $?)"
out=$(ckrun undo)
rc=$?
check "t6-crashkernel.sh: undo after it is byte for byte the old file (rc $rc)" \
	"$([ "$rc" = 0 ] && [ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$before" ]; echo $?)"
printf 'verbosity=1\r\nextraargs=cma=256M\r\n' > "$fb/boot/armbianEnv.txt"
before=$(sha256sum < "$fb/boot/armbianEnv.txt")
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: set refuses CR line ends and leaves the file (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'has CR line ends' <<<"$out" && [ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$before" ] && [ ! -e "$fb/root/yakogl-crashkernel" ]; echo $?)"
printf 'verbosity=1\nextraargs=cma=256M' > "$fb/boot/armbianEnv.txt"
before=$(sha256sum < "$fb/boot/armbianEnv.txt")
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: set refuses a file without a final newline and leaves it (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'does not end with a newline' <<<"$out" && [ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$before" ]; echo $?)"
printf ' extraargs=a\nextraargs=b\n' > "$fb/boot/armbianEnv.txt"
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: set counts a blank-led extraargs= line with the others (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'has 2 extraargs= lines' <<<"$out"; echo $?)"
out=$(ckrun undo)
rc=$?
check "t6-crashkernel.sh: undo without a set refuses (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'nothing to undo' <<<"$out"; echo $?)"
# A set cut short (S2). set writes new.sha256 before it renames the new
# file over the old one, and changes nothing else after that, so each cut
# leaves a state the next set or undo finishes from. Sync 1 is between the
# new file and the rename, sync 2 after the rename.
ck_sync() { rm -f "$CK_SYNC_N" "$CK_SYNC_LOG"; }
printf '%s' "$env_with" > "$fb/boot/armbianEnv.txt"
before=$(sha256sum < "$fb/boot/armbianEnv.txt")
ck_sync
out=$(CK_KILL_AT=1 ckrun set)
check "t6-crashkernel.sh: a set killed before the rename leaves the file as it was (rc $?)" \
	"$(grep -qF 'set did not complete' <<<"$out" && [ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$before" ] && [ -d "$fb/root/yakogl-crashkernel" ]; echo $?)"
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: the next set finishes it (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qx 'extraargs=cma=256M crashkernel=256M@0x10000000' "$fb/boot/armbianEnv.txt" && [ ! -e "$fb/boot/armbianEnv.txt.yakogl-new" ]; echo $?)"
out=$(ckrun undo)
rc=$?
check "t6-crashkernel.sh: ... and undo after it is byte for byte the old file (rc $rc)" \
	"$([ "$rc" = 0 ] && [ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$before" ] && [ ! -e "$fb/root/yakogl-crashkernel" ]; echo $?)"
ck_sync
out=$(CK_KILL_AT=1 ckrun set)
out=$(ckrun undo)
rc=$?
check "t6-crashkernel.sh: undo after a set killed before the rename clears its state, file untouched (rc $rc)" \
	"$([ "$rc" = 0 ] && [ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$before" ] && [ ! -e "$fb/root/yakogl-crashkernel" ] && [ ! -e "$fb/boot/armbianEnv.txt.yakogl-new" ]; echo $?)"
ck_sync
killed=$(CK_KILL_AT=2 ckrun set)
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: after a set killed past the rename, set says it is set (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'set did not complete' <<<"$killed" && grep -qF 'already set by this tool' <<<"$out"; echo $?)"
out=$(ckrun undo)
rc=$?
check "t6-crashkernel.sh: ... and undo restores it byte for byte (rc $rc)" \
	"$([ "$rc" = 0 ] && [ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$before" ]; echo $?)"
mkdir "$fb/root/yakogl-crashkernel"
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: set finishes from an empty state directory (killed after its mkdir) (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qx 'extraargs=cma=256M crashkernel=256M@0x10000000' "$fb/boot/armbianEnv.txt"; echo $?)"
ckrun undo >/dev/null
# The order on disk: a sync while the new file is still beside the old one
# (it is on disk before the rename), then one after the rename.
ck_sync
ckrun set >/dev/null
new_sha=$(sha256sum < "$fb/boot/armbianEnv.txt" | cut -d' ' -f1)
old_sha=${before%% *}
check "t6-crashkernel.sh: set syncs the new file before the rename and the rename after it ($(tr '\n' ';' < "$CK_SYNC_LOG"))" \
	"$(grep -qx "sync 1: new-file yes, file $old_sha" "$CK_SYNC_LOG" && grep -qx "sync 2: new-file no, file $new_sha" "$CK_SYNC_LOG"; echo $?)"
ckrun undo >/dev/null
printf '%s' "$env_with" > "$fb/boot/armbianEnv.txt"
ckrun set >/dev/null
echo 'fdtfile=other.dtb' >> "$fb/boot/armbianEnv.txt"
changed=$(sha256sum < "$fb/boot/armbianEnv.txt")
out=$(ckrun undo)
rc=$?
check "t6-crashkernel.sh: undo refuses a file changed after set and leaves it (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'changed after set' <<<"$out" && [ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$changed" ] && [ -e "$fb/root/yakogl-crashkernel/armbianEnv.txt.orig" ]; echo $?)"
# set on the same state (the file is neither the one set found nor the one it
# wrote) refuses too, and keeps the copy undo needs.
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: set refuses a file changed after set and keeps the copy (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'neither the file set found nor the one it wrote' <<<"$out" && [ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$changed" ] && [ -e "$fb/root/yakogl-crashkernel/armbianEnv.txt.orig" ] && [ -e "$fb/root/yakogl-crashkernel/new.sha256" ]; echo $?)"
rm -rf "$fb/root/yakogl-crashkernel"
# A root UUID inside extraargs= itself: set's before and after lines show
# only the words (the UUID check at the end reads them).
printf 'verbosity=1\nextraargs=root=UUID=%s cma=256M\n' "$uuid" > "$fb/boot/armbianEnv.txt"
before=$(sha256sum < "$fb/boot/armbianEnv.txt")
out=$(ckrun set)
rc=$?
check "t6-crashkernel.sh: set on an extraargs= that holds a root UUID prints only its words (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qx 'board: extraargs after  (crashkernel=, cma= words): cma=256M crashkernel=256M@0x10000000 ' <<<"$out"; echo $?)"
ckrun undo >/dev/null
check "t6-crashkernel.sh: ... and undo is byte for byte" \
	"$([ "$(sha256sum < "$fb/boot/armbianEnv.txt")" = "$before" ]; echo $?)"
printf 'friendlyarm,other\0' > "$fb/proc/device-tree/compatible"
out=$(ckrun status)
rc=$?
check "t6-crashkernel.sh: another board is refused (rc $rc)" \
	"$([ "$rc" != 0 ] && grep -qF 'not a NanoPC-T6' <<<"$out"; echo $?)"
# The board's command line holds the root UUID (in /proc/cmdline, dmesg's
# "Kernel command line" and armbianEnv.txt's rootdev=): status, set and undo
# print only the crashkernel= and cma= tokens and the reservation lines.
check "t6-crashkernel.sh: no output of status, set or undo shows the root UUID ($(grep -c "$uuid" "$tmp/ck-all.out") lines do)" \
	"$([ -s "$tmp/ck-all.out" ] && ! grep -qF "$uuid" "$tmp/ck-all.out"; echo $?)"

# The window is decided by nanopc_t6.dts (zephyr,sram) and held again where an
# image is loaded into it: nanopc-t6.sh checks /proc/iomem over WINDOW_BASE
# ..+WINDOW_SIZE; prompt-go.sh refuses an image larger than its WINDOW_SIZE
# or linked elsewhere than its WINDOW_BASE, and prompt-go.py has U-Boot load
# to LOAD and `go` there; tools/board.sh refuses an image whose image_size
# runs past LOAD_END before it calls nanopc-t6.sh. A window grown in the dts
# alone would leave the dry run checking less RAM than the image takes as its
# own, and board.sh refusing an image that fits; one shrunk there would leave
# U-Boot loading past it. Each must equal the node's reg.
dts=$here/../../boards/friendlyarm/nanopc_t6/nanopc_t6.dts
ugo=$here/../uboot-t6/prompt-go.sh
upy=$here/../uboot-t6/prompt-go.py
reg=$(awk '/^\tzephyr_ram: memory@/ { n = 1 } n && /reg = / { print; exit }' "$dts")
win_base= win_size=
if [[ "$reg" =~ reg\ =\ \<(0x[0-9a-fA-F]+)\ DT_SIZE_M\(([0-9]+)\)\>\; ]]; then
	win_base=$((BASH_REMATCH[1])) win_size=$((BASH_REMATCH[2] << 20))
fi
# NAME=$((<number> [<< <number>])) in a runner, evaluated only in that form.
script_value() { # <name> [file, default nanopc-t6.sh]
	local e
	e=$(sed -n "s/^$1=\\\$((\\(.*\\)))\$/\\1/p" "${2:-$t6}")
	[[ "$e" =~ ^(0x[0-9a-fA-F]+|[0-9]+)(\ \<\<\ [0-9]+)?$ ]] && echo $((e))
}
got="$(script_value WINDOW_BASE) $(script_value WINDOW_SIZE)"
check "nanopc-t6.sh: WINDOW_BASE WINDOW_SIZE ($got) are nanopc_t6.dts's zephyr,sram ($win_base $win_size: '${reg#"${reg%%[! 	]*}"}')" \
	"$([ -n "$win_size" ] && [ "$got" = "$win_base $win_size" ]; echo $?)"
# YAKO_ADDR=<hex>; LOAD_END=<hex> in the nanopc-t6) case of tools/board.sh.
bsh=$here/../../../tools/board.sh
got=$(awk '/^nanopc-t6\)/ { n = 1 } n && /YAKO_ADDR=0x[0-9a-fA-F]+; LOAD_END=0x[0-9a-fA-F]+$/ { print $1, $2; exit }' "$bsh")
if [[ "$got" =~ ^YAKO_ADDR=(0x[0-9a-fA-F]+)\;\ LOAD_END=(0x[0-9a-fA-F]+)$ ]]; then
	got="$((BASH_REMATCH[1])) $((BASH_REMATCH[2] - BASH_REMATCH[1]))"
fi
check "tools/board.sh: nanopc-t6's YAKO_ADDR and LOAD_END - YAKO_ADDR ($got) are nanopc_t6.dts's zephyr,sram ($win_base $win_size)" \
	"$([ -n "$win_size" ] && [ "$got" = "$win_base $win_size" ]; echo $?)"
got="$(script_value WINDOW_BASE "$ugo") $(script_value WINDOW_SIZE "$ugo")"
check "prompt-go.sh: WINDOW_BASE WINDOW_SIZE ($got) are nanopc_t6.dts's zephyr,sram ($win_base $win_size)" \
	"$([ -n "$win_size" ] && [ "$got" = "$win_base $win_size" ]; echo $?)"
got=$(sed -n 's/^LOAD = \(0x[0-9a-fA-F]*\)$/\1/p' "$upy")
check "prompt-go.py: LOAD (${got:-none}), where U-Boot loads the image and goes, is nanopc_t6.dts's zephyr,sram base ($win_base)" \
	"$([ -n "$got" ] && [ -n "$win_base" ] && [ $((got)) = "$win_base" ]; echo $?)"

# prompt-reads.py's and prompt-go.py's own self-tests (a fake board on a
# pty), before the process check below, so that it covers what they start.
# The last line names the checks run and how many failed, and every check
# line of the tool that did not run (none kept by hand: a skipped check line
# fails it). The check line here also carries the tool's own FAIL lines: the
# gate's log is all a CI run leaves, and the count alone does not say which
# check failed.
fail_lines() { grep -E '^FAIL|FAIL$' <<<"$1" | sed 's/^/; /' | tr -d '\n' || true; }
for t in prompt-reads prompt-go; do
	out=$(python3 -I "$here/../uboot-t6/$t.py" --self-test 2>&1)
	got=$(tail -n 1 <<<"$out")
	check "$t.py --self-test ('$got'$(fail_lines "$out"))" \
		"$(grep -qE "^$t self-test: [1-9][0-9]* checks run, 0 failed\$" <<<"$got"; echo $?)"
done
# stage.sh and prompt-go.sh's dry run against a fake ssh.
out=$(bash "$here/../uboot-t6/selftest.sh" 2>&1)
got=$(tail -n 1 <<<"$out")
check "uboot-t6/selftest.sh ('$got'$(fail_lines "$out"))" \
	"$(grep -qE '^uboot-t6 selftest: [1-9][0-9]* checks run, 0 failed$' <<<"$got"; echo $?)"

# The stand-in UART's wire, by its pid.
kill "$wire_pid" 2>/dev/null
wait "$wire_pid" 2>/dev/null

# The test leaves no process behind: the sweep ends every marked process,
# and a known one, started here, shows that the search finds what it looks
# for. (Not the holder: it ends by itself 60 s after its start, so a run
# that takes longer than that would not find it.)
sleep 300 &
known_pid=$!
found=$(marked | grep -cx "$known_pid")
sweep
left=$(marked)
left=${left//$'\n'/ }
check "no process of the test is left (a known one found by the search: $found; left: ${left:-none})" \
	"$([ "$found" = 1 ] && [ -z "$left" ]; echo $?)"

not_run=
while IFS=: read -r line _; do
	[ -n "${ran[$line]:-}" ] || not_run+=" $line"
done < <(grep -nE '^([^#]*[[:space:];&|(])?check "' "$0")
echo "board_lock_selftest: $run checks run, $failed failed${not_run:+, check lines not run:$not_run}"
[ "$failed" = 0 ] && [ "$run" -gt 0 ] && [ -z "$not_run" ]
