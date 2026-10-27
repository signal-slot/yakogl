#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Build one yakogl-mali-panthor binary for the NanoPC-T6 (aarch64, static
# against the host cross toolchain's glibc, so the board's glibc version does
# not matter), copy it to the board and run it there.
#
#   BOARD_HOST=root@<t6> tools/mali/panthor/run-on-board.sh panthor-hello [args...]
#
# BOARD_SSH_OPTS (optional) is passed to ssh and scp (e.g. a known_hosts file).
# The board runs one GL client at a time: the script refuses to start while a
# process opens the render node, and names it. A dmesg -w capture of the run
# is copied back (DMESG_LOG names the local file) and the kernel messages
# between the run's markers are counted from it.
#
# Instruments for a run whose effect may come late (both optional):
#   DMESG_TAIL_S=<s>  keep the dmesg -w capture open <s> seconds after the
#                     end marker and count the kernel messages there apart
#   UART_DEV=<tty>    capture the board's UART on this host, read-only, for
#   UART_LOG=<file>   the whole run (1500000 baud, raw); refused when any
#                     process has the device open (the Z1 recipe uses it)
# The board's boot_id is read before and after: a run across a reboot exits
# 7 whatever the binary said. /proc/cmdline is printed once.
#
# The whole session holds the board lock the kexec recipe takes
# (driver/tools/kexec-zephyr/board_lock.sh, sourced as nanopc-t6.sh sources
# it: one lock file per BOARD_HOST string, so spell the board the same in
# both). A jump cannot start while a GL run holds the board, and a GL run
# refuses while a jump holds it, naming the holder (exit 1). On 2026-10-19 a
# jump rebooted the T6 between two gl_test runs: the board had been handed
# over by chat only. LOCK_WAIT defaults to 0 here (refuse, never queue);
# HOLD_MAX to 1800 s (the build, a sweep and DMESG_TAIL_S fit well inside).
set -euo pipefail
bin=${1:?usage: BOARD_HOST=<user@host> $0 <binary> [args...]}
shift
host=${BOARD_HOST:?set BOARD_HOST=<user@host>}
read -r -a opts <<< "${BOARD_SSH_OPTS-}"
cd "$(dirname "$0")/../../.."
LOCK_WAIT=${LOCK_WAIT:-0}
HOLD_MAX=${HOLD_MAX:-1800}
# shellcheck source=../../../driver/tools/kexec-zephyr/board_lock.sh
. driver/tools/kexec-zephyr/board_lock.sh
echo "run-on-board.sh: holding the board lock $BOARD_LOCK (pid $$)"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C target-feature=+crt-static"
cargo build -q -p yakogl-mali-panthor --release --target aarch64-unknown-linux-gnu --bin "$bin"
exe="${CARGO_TARGET_DIR:-target}/aarch64-unknown-linux-gnu/release/$bin"
# The binary prints its build (build.rs: git describe --always --dirty). It
# must be the tree being run: refuse when the two differ at run time.
want=$(git describe --always --dirty)
echo "run-on-board.sh: tree $want"
LC_ALL=C grep -qaP -- "\\x00yakogl-build=\\Q${want}\\E\\x00" "$exe" ||
  { echo "run-on-board.sh: the binary does not carry the tree's build '$want': stale build, not running" >&2; exit 6; }
remote=/root/yakogl-mali/$bin
# Who holds the render node? fuser exits 0 when a process has it open, 1 when
# none has; a missing fuser or a failed ssh is its own refusal, never "free".
ssh "${opts[@]}" "$host" 'command -v fuser >/dev/null' || {
  echo "run-on-board.sh: fuser is missing on the board (or ssh failed): cannot tell who holds the render node, not starting" >&2
  exit 4
}
rc=0
users=$(ssh "${opts[@]}" "$host" 'fuser -v /dev/dri/renderD128 2>&1') || rc=$?
case $rc in
  0)
    echo "run-on-board.sh: /dev/dri/renderD128 is in use, not starting:" >&2
    echo "$users" >&2
    exit 3
    ;;
  1) ;;
  *)
    echo "run-on-board.sh: fuser failed with $rc, not starting: $users" >&2
    exit 4
    ;;
esac
ssh "${opts[@]}" "$host" "mkdir -p /root/yakogl-mali"
scp -q "${opts[@]}" "$exe" "$host:$remote"
boot0=$(ssh "${opts[@]}" "$host" 'cat /proc/sys/kernel/random/boot_id')
echo "run-on-board.sh: boot_id before $boot0"
echo "run-on-board.sh: cmdline $(ssh "${opts[@]}" "$host" 'cat /proc/cmdline')"
uart_pid=
if [ -n "${UART_DEV-}" ]; then
  : "${UART_LOG:?UART_DEV needs UART_LOG (the local file of the capture)}"
  [ -e "$UART_DEV" ] || { echo "run-on-board.sh: no UART device $UART_DEV" >&2; exit 4; }
  if fuser "$UART_DEV" >/dev/null 2>&1; then
    echo "run-on-board.sh: $UART_DEV is open by another process, not starting:" >&2
    fuser -v "$UART_DEV" >&2 || true
    exit 3
  fi
  # read-only: the capture never writes to the board's console
  stty -F "$UART_DEV" 1500000 raw -echo -echoe -echok -echoctl -echoke
  cat "$UART_DEV" > "$UART_LOG" &
  uart_pid=$!
  echo "run-on-board.sh: UART $UART_DEV captured read-only to $UART_LOG (pid $uart_pid)"
fi

# dmesg -w goes to a file on the board before the run and stops after it; the
# kernel messages of the run are counted from that file (the ring wraps), not
# from a grep at capture time. The markers bound the run inside the capture.
stamp=$(date +%Y%m%d-%H%M%S)
rlog=/root/yakogl-mali/dmesg-$bin-$stamp.log
llog=${DMESG_LOG:-$(mktemp "${TMPDIR:-/tmp}/dmesg-$bin-XXXXXX.log")}
cpid=$(ssh "${opts[@]}" "$host" "nohup dmesg -w > $rlog 2>&1 < /dev/null & echo \$!")
[ -n "$cpid" ] || { echo "run-on-board.sh: could not start dmesg -w on the board" >&2; exit 4; }
stop_capture() {
  ssh "${opts[@]}" "$host" "kill $cpid" || echo "run-on-board.sh: could not stop dmesg -w (pid $cpid) on the board" >&2
}
ssh "${opts[@]}" "$host" "echo 'yakogl-run-start $bin $stamp' > /dev/kmsg"
sleep 1
status=0
ssh "${opts[@]}" "$host" "$remote" "$@" || status=$?
ssh "${opts[@]}" "$host" "echo 'yakogl-run-end $bin $stamp' > /dev/kmsg"
sleep "${DMESG_TAIL_S:-1}"
stop_capture
if [ -n "$uart_pid" ]; then
  kill "$uart_pid" 2>/dev/null || true
  wait "$uart_pid" 2>/dev/null || true
  echo "run-on-board.sh: UART capture $UART_LOG, $(wc -l < "$UART_LOG") line(s)"
fi
boot1=$(ssh "${opts[@]}" "$host" 'cat /proc/sys/kernel/random/boot_id') || boot1=unreachable
echo "run-on-board.sh: boot_id after $boot1"
[ "$boot1" = "$boot0" ] ||
  echo "run-on-board.sh: the board's boot_id changed during the run ($boot0 -> $boot1); the run will exit 7" >&2

scp -q "${opts[@]}" "$host:$rlog" "$llog"
if ! grep -q "yakogl-run-start $bin $stamp" "$llog" || ! grep -q "yakogl-run-end $bin $stamp" "$llog"; then
  echo "run-on-board.sh: the dmesg capture $llog lacks a run marker: the kernel-message count is unknown" >&2
  exit 5
fi
during=$(sed -n "/yakogl-run-start $bin $stamp/,/yakogl-run-end $bin $stamp/p" "$llog" | sed '1d;$d')
count=$(printf '%s' "$during" | grep -c . || true)
echo "run-on-board.sh: $count kernel message(s) between the run markers, capture $llog (board: $rlog)"
[ "$count" -eq 0 ] || printf '%s\n' "$during"
tail_lines=$(sed -n "/yakogl-run-end $bin $stamp/,\$p" "$llog" | sed '1d')
tcount=$(printf '%s' "$tail_lines" | grep -c . || true)
echo "run-on-board.sh: $tcount kernel message(s) in the ${DMESG_TAIL_S:-1} s after the end marker"
[ "$tcount" -eq 0 ] || printf '%s\n' "$tail_lines"
if [ "$boot1" != "$boot0" ]; then
  echo "run-on-board.sh: the board's boot_id changed during the run ($boot0 -> $boot1): the run is not a result" >&2
  exit 7
fi
exit "$status"
