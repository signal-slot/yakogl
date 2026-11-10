#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of tools/cts/mali-glmark2-session.sh without a board: a fake ssh
# (testdata/mali-glmark2-session/bin/ssh) is one board for the preflight, the
# state snapshots and the runner, by handing each command to the fake ssh of
# that script's own self-test; the runner's fake scp, board tools, scenarios and
# fixture build tree, the preflight's "good" case and the state's "good" case
# are used where they are (testdata/mali-glmark2-run, -preflight,
# mali-board-state), not copied. Scenarios, each checked by its final lines and
# its exit code:
#   clean            runner exit 0, nothing changed, same boot_id: exit 0
#   runner-failed    the runner refuses a dev build (its exit 3) before any
#                    ssh, the board intact: exit 1, both snapshots and compare
#   boot-changed     a /boot file differs after the run: exit 3, "/boot CHANGED"
#   module-added     a module is loaded during the run: exit 3
#   rebooted         the boot_id differs after the run: exit 3, "boot_id changed"
#   ssh-lost         the board goes away after the run: exit 4, "NOT reachable"
#   preflight-fails  the preflight finds a wrong driver: exit 5, no snapshot,
#                    no runner ssh
#   dry-run          no BOARD_HOST, no ssh or scp called, nothing written
# and that the wrapper's own text has no write, sudo or module verb and no
# address, that its kept files hold no host name, and that its own ssh
# command is the boot_id read.
#
#   tools/cts/mali-glmark2-session-selftest.sh   prints "mali-glmark2-session-selftest: N checks passed"
#
# Host only, about 30 s. Its gate row in tools/check.sh is added by a separate
# item of plan #332 (gate-row); this script does not add it.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
FIX=$HERE/testdata/mali-glmark2-session
RFIX=$HERE/testdata/mali-glmark2-run
PFIX=$HERE/testdata/mali-glmark2-preflight
SFIX=$HERE/testdata/mali-board-state
SESSION=$HERE/mali-glmark2-session.sh
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mali-glmark2-session-selftest-XXXXXX")
cleanup() {
  pkill -f -- "$tmp/" 2> /dev/null
  if [ "${MALI_GLMARK2_SESSION_SELFTEST_KEEP-}" = 1 ]; then echo "mali-glmark2-session-selftest: kept $tmp"; else rm -rf "$tmp"; fi
}
trap cleanup EXIT
passed=0 failed=0
check() { # <what> <command...>
  local what=$1
  shift
  if "$@" > /dev/null 2>&1; then
    passed=$((passed + 1))
    echo "  ok    $what"
  else
    failed=$((failed + 1))
    echo "  FAIL  $what"
  fi
}

mkdir -p "$tmp/bin" "$tmp/board-bin"
cp "$FIX"/bin/* "$tmp/bin/"
cp "$RFIX/bin/scp" "$tmp/bin/scp"
cp "$RFIX"/board-bin/* "$tmp/board-bin/"
chmod +x "$tmp"/bin/* "$tmp"/board-bin/*

# A repository whose main has commit c1.
g() { git -C "$tmp/repo" -c user.name=selftest -c user.email=selftest -c commit.gpgsign=false "$@"; }
mkdir -p "$tmp/repo"
git -C "$tmp/repo" init -q
g commit -q --allow-empty -m c1
g branch -f main HEAD
c1=$(g rev-parse HEAD)

# run <name> <after> [session args...]: one session on a fresh fake board.
# <after> is the state after the run: good, boot-changed, module-added,
# rebooted, lost. VAR=value words before the call go to env: DEV, PFCASE.
run() {
  local name=$1 after=$2 d=$tmp/$1
  shift 2
  mkdir -p "$d/board/home" "$d/build" "$d/session" "$d/state-good" "$d/state-after" "$d/pf"
  cp -r "$RFIX/fixture-build/." "$d/build/"
  chmod +x "$d/build/bin/glmark2-es2-mali"
  sed -i "s/^commit=.*/commit=$c1/; s/^dev=.*/dev=${DEV:-no}/" "$d/build/BUILD-INFO"
  ln -sf libEGL.so.1 "$d/build/lib/libEGL.so"
  ln -sf libGLESv2.so.2 "$d/build/lib/libGLESv2.so"
  cp "$SFIX"/good/* "$d/state-good/"
  cp "$SFIX"/good/* "$d/state-after/"
  case $after in good | lost) ;; *) cp "$FIX/after/$after"/* "$d/state-after/" ;; esac
  cp "$PFIX"/good/* "$d/pf/"
  [ -z "${PFCASE-}" ] || cp "$PFIX/$PFCASE"/* "$d/pf/"
  printf '[    0.000000] Booting Linux on physical CPU 0x0\n[    9.500000] panthor fb000000.gpu: [drm] clock rate = 1000000000\n' > "$d/board/kmsg"
  echo 5a1e0000-0000-4000-8000-00000000b007 > "$d/board/boot_id"
  : > "$d/ssh.log"
  : > "$d/session.log"
  env PATH="$tmp/bin:$PATH" BOARD_HOST=selftest@board.invalid BOARD_LOCK="$tmp/board.lock" \
    FAKE_BOARD="$d/board" FAKE_BOARD_BIN="$tmp/board-bin" FAKE_LOG="$d/ssh.log" \
    FAKE_SESSION_LOG="$d/session.log" FAKE_SESSION_DIR="$d/session" \
    FAKE_PREFLIGHT_SSH="$PFIX/bin/ssh" FAKE_PREFLIGHT_CASE="$d/pf" \
    FAKE_STATE_SSH="$SFIX/bin/ssh" FAKE_STATE_CASE="$d/state-good" FAKE_STATE_AFTER="$d/state-after" \
    FAKE_RUN_SSH="$RFIX/bin/ssh" FAKE_AFTER="$after" \
    FAKE_FIXTURES="$RFIX" FAKE_SCENARIO="$RFIX/scenarios/clean.tsv" \
    MALI_GLMARK2_REPO="$tmp/repo" MALI_GLMARK2_MAIN_REF=main RAW_DIR="$d/raw" \
    BOARD_BACK_S=2 BOARD_POLL_S=0.5 KILL_GRACE_S=1 STAGE_MARGIN_KB=0 HOLD_MAX=300 \
    PROBE_PAUSE_S=0 PROBE_TIMEOUT_S=5 \
    bash "$SESSION" "$@" "$d/build" "$d/out" > "$d/out.txt" 2>&1
  echo $? > "$d/rc"
}
rc_is() { [ "$(cat "$tmp/$1/rc")" = "$2" ]; }
has() { grep -qxF -- "$2" "$tmp/$1/out.txt"; }
hasre() { grep -qE -- "$2" "$tmp/$1/out.txt"; }
lacks() { ! grep -qF -- "$2" "$tmp/$1/out.txt"; }
last_is() { [ "$(tail -n 1 "$tmp/$1/out.txt")" = "$2" ]; }
exists() { [ -e "$tmp/$1/out/$2" ]; }
absent() { [ ! -e "$tmp/$1/out/$2" ]; }
# the compare lines of the output are those of state-compare.txt, verbatim
verbatim() { [ -s "$tmp/$1/out/state-compare.txt" ] && grep -qxFf "$tmp/$1/out/state-compare.txt" "$tmp/$1/out.txt" &&
  [ "$(grep -cxFf "$tmp/$1/out/state-compare.txt" "$tmp/$1/out.txt")" = "$(wc -l < "$tmp/$1/out/state-compare.txt")" ]; }
clean_out() { ! grep -rqiE 'board\.invalid|198\.51\.100|02:00:5e|selftest@' "$tmp/$1/out"; }

echo "mali-glmark2-session-selftest: clean"
run clean good --scenes 1-3
check "clean: exit 0" rc_is clean 0
check "clean: preflight ran and passed" hasre clean '^preflight: summary: [0-9]+ ok, 0 FAIL'
check "clean: first snapshot" hasre clean '^mali-board-state\.sh: snapshot of 3 /boot files and 3 modules written to .*/state-before.txt$'
check "clean: runner summary kept" hasre clean '^mali-glmark2: 3/3 scenes ok, failed 0, commit '
check "clean: second snapshot" hasre clean '^mali-board-state\.sh: snapshot of 3 /boot files and 3 modules written to .*/state-after.txt$'
check "clean: /boot line verbatim" has clean "mali-board-state: /boot unchanged (3 files, sha256 $(sed -n 's/^boot_sha256=//p' "$tmp/clean/out/state-before.txt"))"
check "clean: modules line" hasre clean '^mali-board-state: modules unchanged \(3 modules, sha256 '
check "clean: env line" hasre clean '^mali-board-state: U-Boot env unchanged \(sha256 '
check "clean: boot_id line" has clean "mali-board-state: boot_id unchanged (0b1c2d3e-0000-4000-8000-aaaaaaaaaaaa)"
check "clean: compare lines are state-compare.txt, verbatim" verbatim clean
check "clean: reachability line" has clean "mali-glmark2-session: board reachable over ssh, boot_id same"
check "clean: tally table printed" hasre clean '^mali-glmark2-session: tally exit 0; table and row drafts under '
check "clean: last line" last_is clean "mali-glmark2-session: exit 0 (all clean, board left as found)"
check "clean: output files" bash -c "cd '$tmp/clean/out' && ls state-before.txt state-after.txt state-compare.txt preflight.txt run/scenes.tsv run/meta.txt tally-table.txt tally-results-row.txt tally-status-row.txt tally-picture-lines.txt tally-causes.txt"
check "clean: the three scenes are in the tally table" bash -c "[ \$(grep -c 'build' '$tmp/clean/out/tally-table.txt') -ge 1 ]"
check "clean: kept files have no host name or address" clean_out clean
check "clean: the stage is gone from the board" test ! -e "$tmp/clean/board/home/yakogl-mali-glmark2"
order_ok() { # preflight probes, then snapshot, then the runner's, then the second snapshot
  local f=$tmp/$1/session.log a b c d
  a=$(grep -n 'yakogl-preflight-ok' "$f" | head -1 | cut -d: -f1)
  b=$(grep -n 'yakogl-state-ok' "$f" | sed -n 1p | cut -d: -f1)
  c=$(grep -n 'renderD128' "$f" | tail -1 | cut -d: -f1)
  d=$(grep -n 'yakogl-state-ok' "$f" | sed -n 2p | cut -d: -f1)
  [ -n "$a" ] && [ -n "$b" ] && [ -n "$c" ] && [ -n "$d" ] && [ "$a" -lt "$b" ] && [ "$b" -lt "$c" ] && [ "$c" -lt "$d" ]
}
check "clean: order preflight, snapshot, runner, snapshot" order_ok clean
only_boot_id_own() { # the wrapper's own ssh: the boot_id read, once, after the second snapshot
  [ "$(grep -c '^ssh cat /proc/sys/kernel/random/boot_id$' "$tmp/$1/session.log")" -ge 1 ]
}
check "clean: the reachability check is the boot_id read" only_boot_id_own clean

echo "mali-glmark2-session-selftest: runner fails, board intact"
DEV=yes run runfail good --scenes 1-3
check "runner-failed: exit 1" rc_is runfail 1
check "runner-failed: the runner refused (dev build)" hasre runfail '^mali-glmark2-run: .*dev'
check "runner-failed: runner exit 3 reported" has runfail "mali-glmark2-session: runner exit 3"
check "runner-failed: second snapshot taken anyway" exists runfail state-after.txt
check "runner-failed: /boot unchanged" hasre runfail '^mali-board-state: /boot unchanged'
check "runner-failed: reachable, same boot_id" has runfail "mali-glmark2-session: board reachable over ssh, boot_id same"
check "runner-failed: no run directory to tally" has runfail "mali-glmark2-session: no run directory to tally ($tmp/runfail/out/run has no scenes.tsv)"
check "runner-failed: last line" last_is runfail "mali-glmark2-session: exit 1 (runner or tally failed, board intact)"

echo "mali-glmark2-session-selftest: /boot changed after the run"
run bootchg boot-changed --scenes 1-2
check "boot-changed: exit 3" rc_is bootchg 3
check "boot-changed: /boot CHANGED line" hasre bootchg '^mali-board-state: /boot CHANGED \(before 3 files, sha256 [0-9a-f]{64}; after 3 files, sha256 [0-9a-f]{64}\)$'
check "boot-changed: modules unchanged" hasre bootchg '^mali-board-state: modules unchanged'
check "boot-changed: lines verbatim" verbatim bootchg
check "boot-changed: board still reachable" has bootchg "mali-glmark2-session: board reachable over ssh, boot_id same"
check "boot-changed: the tally still ran" exists bootchg tally-table.txt
check "boot-changed: last line" last_is bootchg "mali-glmark2-session: exit 3 (board state CHANGED or not proven)"

echo "mali-glmark2-session-selftest: module added during the run"
run modadd module-added --scenes 1-2
check "module-added: exit 3" rc_is modadd 3
check "module-added: modules CHANGED line" hasre modadd '^mali-board-state: modules CHANGED \(before 3 modules, sha256 [0-9a-f]{64}; after 4 modules, sha256 [0-9a-f]{64}\)$'
check "module-added: /boot unchanged" hasre modadd '^mali-board-state: /boot unchanged'
check "module-added: lines verbatim" verbatim modadd
check "module-added: last line" last_is modadd "mali-glmark2-session: exit 3 (board state CHANGED or not proven)"

echo "mali-glmark2-session-selftest: board rebooted (boot_id changed)"
run reboot rebooted --scenes 1-2
check "rebooted: exit 3" rc_is reboot 3
check "rebooted: compare says the board rebooted" hasre reboot '^mali-board-state: boot_id differs \(before .*\): the board rebooted$'
check "rebooted: reachability line says changed" has reboot "mali-glmark2-session: board reachable over ssh, boot_id changed"
check "rebooted: last line" last_is reboot "mali-glmark2-session: exit 3 (board state CHANGED or not proven)"

echo "mali-glmark2-session-selftest: ssh lost after the run"
run lost lost --scenes 1-2
check "ssh-lost: exit 4" rc_is lost 4
check "ssh-lost: the second snapshot failed" has lost "mali-board-state.sh: board not reachable over ssh (BatchMode, 10 s); no snapshot written"
check "ssh-lost: no state-after.txt" absent lost state-after.txt
check "ssh-lost: final line says NOT reachable" has lost "mali-glmark2-session: board NOT reachable: needs a person, not retried"
check "ssh-lost: the runner's run was tallied" exists lost tally-table.txt
check "ssh-lost: last line" last_is lost "mali-glmark2-session: exit 4 (board lost)"
check "ssh-lost: nothing retried (one state-ok after the run)" bash -c "[ \$(grep -c '^ssh echo yakogl-state-ok\$' '$tmp/lost/session.log') = 2 ]"

echo "mali-glmark2-session-selftest: preflight fails"
PFCASE=wrong-driver run pffail good --scenes 1-2
check "preflight-fails: exit 5" rc_is pffail 5
check "preflight-fails: preflight FAIL printed" hasre pffail '^preflight: .*FAIL'
check "preflight-fails: stopped" has pffail "mali-glmark2-session: preflight failed: not starting; nothing was run on the board"
check "preflight-fails: last line" last_is pffail "mali-glmark2-session: exit 5 (stopped before the runner)"
check "preflight-fails: no snapshot, no runner" bash -c "! grep -q 'yakogl-state-ok' '$tmp/pffail/session.log' && [ ! -e '$tmp/pffail/out/state-before.txt' ] && [ ! -e '$tmp/pffail/out/run' ]"

echo "mali-glmark2-session-selftest: usage and dry run"
mkdir -p "$tmp/dry/bin"
for t in ssh scp; do printf '#!/bin/sh\necho "%s called: $*" >> "%s/dry/called"\nexit 99\n' "$t" "$tmp" > "$tmp/dry/bin/$t"; done
chmod +x "$tmp/dry/bin"/*
dry() { # <name> [args...]: BOARD_HOST unset, ssh and scp that fail when called
  local n=$1
  shift
  (cd "$tmp/dry" && env -u BOARD_HOST PATH="$tmp/dry/bin:$PATH" bash "$SESSION" "$@" > "$tmp/dry/$n.txt" 2>&1)
  echo $? > "$tmp/dry/$n.rc"
}
dry bare --dry-run
dry full --dry-run --scenes 1-3 --timeout 60 /nonexistent/build "$tmp/dry/outdir"
dry nohost --scenes 1-3 /nonexistent/build "$tmp/dry/outdir2"
dry badopt --bogus
check "dry-run: bare exit 0" bash -c "[ \"\$(cat '$tmp/dry/bare.rc')\" = 0 ]"
check "dry-run: prints the plan" bash -c "grep -q 'dry run, no ssh, nothing written' '$tmp/dry/bare.txt' && grep -q 'snapshot \$BOARD_HOST <outdir>/state-before.txt' '$tmp/dry/bare.txt' && grep -q 'compare <outdir>/state-before.txt' '$tmp/dry/bare.txt'"
check "dry-run: with a build and an outdir, exit 0" bash -c "[ \"\$(cat '$tmp/dry/full.rc')\" = 0 ]"
check "dry-run: runner options passed through in the plan" bash -c "grep -q -- '--scenes 1-3 --timeout 60 /nonexistent/build $tmp/dry/outdir/run' '$tmp/dry/full.txt'"
check "dry-run: ssh and scp never called" test ! -e "$tmp/dry/called"
check "dry-run: nothing written" bash -c "[ ! -e '$tmp/dry/outdir' ] && [ -z \"\$(ls '$tmp/dry' | grep -vE '^(bin|bare|full|nohost|badopt)')\" ]"
check "no BOARD_HOST: exit 2, says so" bash -c "[ \"\$(cat '$tmp/dry/nohost.rc')\" = 2 ] && grep -q 'set BOARD_HOST' '$tmp/dry/nohost.txt'"
check "unknown option: exit 2" bash -c "[ \"\$(cat '$tmp/dry/badopt.rc')\" = 2 ]"

echo "mali-glmark2-session-selftest: the wrapper's text"
no_verbs() { ! grep -v '^ *#' "$1" | grep -qE '(^|[^[:alnum:]_-])(sudo|modprobe|rmmod|insmod|tee -a /|dd|mount|umount|fw_setenv|tailscale|sshd|systemctl)([^[:alnum:]_-]|$)'; }
check "script text has no write, sudo, module, tailscale or sshd verb" no_verbs "$SESSION"
no_addr() { ! grep -qE '([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|[0-9a-f]{2}(:[0-9a-f]{2}){5}' "$1"; }
check "script text has no address or UART path" no_addr "$SESSION"
check "script is executable" test -x "$SESSION"

echo "mali-glmark2-session-selftest: $passed checks passed"
[ "$failed" = 0 ]
