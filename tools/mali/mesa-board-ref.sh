#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Run one GLES2 C program through the board's own Mesa (panfrost on
# panthor) with a pandecode dump, and fetch the dump: the descriptor
# reference a frame-builder change is pinned to.
#
#   BOARD_HOST=root@<t6> tools/mali/mesa-board-ref.sh <prog.c> <outdir> <name> [args...]
#
# The program is compiled on the board (gcc, -lEGL -lGLESv2) and run as
#   PAN_MESA_DEBUG=linear,nocrc,dump,trace,sync PANDECODE_DUMP_FILE=<name>.pandecode
# (the flags of 0dce654a8:tools/mali/board-ref/2026-10-18/linear-dump: a linear target,
# no CRC, the trace plus the raw memory after the job ran). Into <outdir>:
# <name>.pandecode (Mesa writes one <file>.ctx-0.<NNNN> per frame it
# submits; here every one, in order, concatenated), <name>.stdout
# (stdout and stderr, then the rc line) and <name>.dmesg (the kernel messages
# the run added). Prints the sha256 of each.
#
# BOARD_SSH_OPTS (optional) is passed to ssh and scp. Like run-on-board.sh it
# takes the board lock (driver/tools/kexec-zephyr/board_lock.sh; never
# waits: LOCK_WAIT 0) and refuses while a process holds the render node.
set -euo pipefail
src=${1:?usage: BOARD_HOST=<user@host> $0 <prog.c> <outdir> <name> [args...]}
out=${2:?outdir}
name=${3:?name}
shift 3
host=${BOARD_HOST:?set BOARD_HOST=<user@host>}
read -r -a opts <<< "${BOARD_SSH_OPTS-}"
case $name in *[!A-Za-z0-9_-]*) echo "mesa-board-ref.sh: name '$name': letters, digits, - and _ only" >&2; exit 2 ;; esac
src=$(realpath "$src")
mkdir -p "$out"
out=$(realpath "$out")
cd "$(dirname "$0")/../.."
LOCK_WAIT=0
HOLD_MAX=${HOLD_MAX:-600}
# shellcheck source=../../driver/tools/kexec-zephyr/board_lock.sh
. driver/tools/kexec-zephyr/board_lock.sh
echo "mesa-board-ref.sh: holding the board lock $BOARD_LOCK (pid $$)"
rc=0
users=$(bssh "${opts[@]}" "$host" 'fuser -v /dev/dri/renderD128 2>&1') || rc=$?
case $rc in
  0) echo "mesa-board-ref.sh: /dev/dri/renderD128 is in use, not starting:" >&2; echo "$users" >&2; exit 3 ;;
  1) ;;
  *) echo "mesa-board-ref.sh: fuser failed with $rc, not starting: $users" >&2; exit 4 ;;
esac
dir=/root/mref-yakogl
bssh "${opts[@]}" "$host" "mkdir -p $dir && rm -f $dir/$name.pandecode.ctx-*"
bscp "${opts[@]}" "$src" "$host:$dir/$name.c"
bssh -t 120 "${opts[@]}" "$host" "gcc -O1 -Wall -Werror -o $dir/$name $dir/$name.c -lEGL -lGLESv2"
boot0=$(bssh "${opts[@]}" "$host" 'cat /proc/sys/kernel/random/boot_id')
lines0=$(bssh "${opts[@]}" "$host" 'dmesg | wc -l')
echo "mesa-board-ref.sh: boot_id before $boot0"
args=$(printf ' %q' "$@")
rc=0
bssh -t 120 "${opts[@]}" "$host" \
  "cd $dir && PAN_MESA_DEBUG=linear,nocrc,dump,trace,sync PANDECODE_DUMP_FILE=$dir/$name.pandecode ./$name$args > $name.stdout 2>&1; echo \"rc=\$?\" >> $name.stdout" || rc=$?
boot1=$(bssh "${opts[@]}" "$host" 'cat /proc/sys/kernel/random/boot_id')
echo "mesa-board-ref.sh: boot_id after $boot1"
bssh "${opts[@]}" "$host" "dmesg | tail -n +$((lines0 + 1))" > "$out/$name.dmesg"
bscp "${opts[@]}" "$host:$dir/$name.stdout" "$out/$name.stdout"
frames=$(bssh "${opts[@]}" "$host" "cd $dir && ls $name.pandecode.ctx-* | sort")
[ -n "$frames" ] || { echo "mesa-board-ref.sh: no pandecode dump was written" >&2; exit 6; }
: > "$out/$name.pandecode"
for f in $frames; do
  bscp "${opts[@]}" "$host:$dir/$f" "$out/$name.frame.tmp"
  cat "$out/$name.frame.tmp" >> "$out/$name.pandecode"
done
rm -f "$out/$name.frame.tmp"
echo "mesa-board-ref.sh: $(echo "$frames" | wc -l) dump file(s): $(echo $frames)"
cat "$out/$name.stdout"
echo "mesa-board-ref.sh: kernel messages during the run: $(wc -l < "$out/$name.dmesg")"
(cd "$out" && sha256sum "$name.pandecode" "$name.stdout" "$name.dmesg")
[ "$boot0" = "$boot1" ] || { echo "mesa-board-ref.sh: the board rebooted during the run" >&2; exit 7; }
[ "$rc" = 0 ] || { echo "mesa-board-ref.sh: ssh exited $rc" >&2; exit 5; }
