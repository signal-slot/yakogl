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
# Needs bash, flock(1) and strings(1). tools/check.sh runs it ("tools   board_lock.sh: ...").
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
# first in PATH prints $tmp/facts whatever it is asked, so no board is
# contacted. The facts are the T6's under Armbian: one System RAM bank
# 0x00200000-0xe143ffff, below 0x20000000 only the kernel's entries under
# it, the GIC's ITS/LPI tables above 4 GiB. An image TOP 13 passes the image
# checks. t6_board_run <iomem lines under the bank> <gic_tables> [--go]
mkdir "$tmp/fakebin"
printf '#!/bin/sh\ncat "%s/facts"\n' "$tmp" > "$tmp/fakebin/ssh"
chmod +x "$tmp/fakebin/ssh"
t6_image "$tmp/top13.bin" 'board: watchdog armed'
sed -i 's/; top 9; /; top 13; /' "$tmp/top13.bin"
t6_board_run() {
	{
		printf '%s\n' 'compatible=friendlyarm,nanopc-t6 rockchip,rk3588 ' kernel=6.18.55 \
			kexec_load_disabled=0 kexec_loaded=0 config_kexec=CONFIG_KEXEC=y uid=0 \
			wdt_identity=dw_wdt wdt_state=inactive wdt_timeout=44 wdt_nowayout=0 wdt_users= \
			"gic_tables=$2" 'iomem<<' '00200000-e143ffff : System RAM'
		printf '%s\n' "$1" '>>'
	} > "$tmp/facts"
	PATH=$tmp/fakebin:$PATH BOARD_HOST=root@board.invalid bash "$t6" "$tmp/top13.bin" ${3:+"$3"} 2>&1
}
kernel='  02000000-0432ffff : Kernel code'
gic_out='100220000 100230000 1002f0000 '
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

# The window is decided by nanopc_t6.dts (zephyr,sram) and held again where an
# image is loaded into it: nanopc-t6.sh checks /proc/iomem over WINDOW_BASE
# ..+WINDOW_SIZE, and the U-Boot script loads to t6z_load and refuses above
# t6z_window. A window grown in the dts alone would leave the dry run
# checking less RAM than the image takes as its own; one shrunk there would
# leave U-Boot loading past it. Each must equal the node's reg.
dts=$here/../../boards/friendlyarm/nanopc_t6/nanopc_t6.dts
ub=$here/../uboot-t6/t6-zephyr.cmd
reg=$(awk '/^\tzephyr_ram: memory@/ { n = 1 } n && /reg = / { print; exit }' "$dts")
win_base= win_size=
if [[ "$reg" =~ reg\ =\ \<(0x[0-9a-fA-F]+)\ DT_SIZE_M\(([0-9]+)\)\>\; ]]; then
	win_base=$((BASH_REMATCH[1])) win_size=$((BASH_REMATCH[2] << 20))
fi
# NAME=$((<number> [<< <number>])) in nanopc-t6.sh, evaluated only in that form.
script_value() {
	local e
	e=$(sed -n "s/^$1=\\\$((\\(.*\\)))\$/\\1/p" "$t6")
	[[ "$e" =~ ^(0x[0-9a-fA-F]+|[0-9]+)(\ \<\<\ [0-9]+)?$ ]] && echo $((e))
}
got="$(script_value WINDOW_BASE) $(script_value WINDOW_SIZE)"
check "nanopc-t6.sh: WINDOW_BASE WINDOW_SIZE ($got) are nanopc_t6.dts's zephyr,sram ($win_base $win_size: '${reg#"${reg%%[! 	]*}"}')" \
	"$([ -n "$win_size" ] && [ "$got" = "$win_base $win_size" ]; echo $?)"
uboot_value() {
	local v
	v=$(sed -n "s/^setenv $1 \\(0x[0-9a-fA-F]*\\)\$/\\1/p" "$ub")
	[ -n "$v" ] && echo $((v))
}
got="$(uboot_value t6z_load) $(uboot_value t6z_entry) $(uboot_value t6z_window)"
check "t6-zephyr.cmd: t6z_load t6z_entry t6z_window ($got) are nanopc_t6.dts's zephyr,sram ($win_base $win_base $win_size)" \
	"$([ -n "$win_size" ] && [ "$got" = "$win_base $win_base $win_size" ]; echo $?)"

# The test leaves no process behind: the sweep ends every marked process,
# and a known one (the holder, still running here) shows that the search
# finds what it looks for.
found=$(marked | grep -cx "$holder_pid")
sweep
left=$(marked)
left=${left//$'\n'/ }
check "no process of the test is left (the holder found by the search: $found; left: ${left:-none})" \
	"$([ "$found" = 1 ] && [ -z "$left" ]; echo $?)"

not_run=
while IFS=: read -r line _; do
	[ -n "${ran[$line]:-}" ] || not_run+=" $line"
done < <(grep -nE '^([^#]*[[:space:];&|(])?check "' "$0")
echo "board_lock_selftest: $run checks run, $failed failed${not_run:+, check lines not run:$not_run}"
[ "$failed" = 0 ] && [ "$run" -gt 0 ] && [ -z "$not_run" ]
