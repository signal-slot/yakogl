#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of tools/mp2s_undo_order.py: no board, python3 only. It runs
# the runner's own --dry-run for every verb, then breaks the transcripts.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/mp2s_undo_order.py"
runner="$here/board-stm32mp2-standalone.sh"
tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/mp2s-undo.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
expect() { # name want-exit file
    python3 -I "$tool" "$3" >"$tmp/out" 2>"$tmp/err"
    local rc=$?
    if [ "$rc" = "$2" ]; then ok "$1"; else bad "$1: exit $rc, want $2"; cat "$tmp/out" "$tmp/err"; fi
}

head -c 100 /dev/zero >"$tmp/img"
printf 'ARMd' | dd of="$tmp/img" bs=1 seek=56 conv=notrunc status=none
export MP2_NO_WATCHDOG=1
r() { "$runner" --dry-run "$@" 2>&1; }
r install >"$tmp/install.txt"
r uninstall >"$tmp/uninstall.txt"
r nodefault >"$tmp/nodefault.txt"
r default "$tmp/img" 2 >"$tmp/default2.txt"
BOARD_PERSON_PRESENT=1 r default "$tmp/img" forever >"$tmp/forever.txt"
for v in install uninstall nodefault default2 forever; do expect "runner $v" 0 "$tmp/$v.txt"; done
grep -q 'fw_setenv' "$tmp/default2.txt" && grep -q 'tee' "$tmp/default2.txt" || bad "default2 has no change line"

# the change lines moved ahead of their undo
for v in install uninstall nodefault default2; do
    { grep '^DRY board\$' "$tmp/$v.txt"; grep -v '^DRY board\$' "$tmp/$v.txt"; } >"$tmp/late-$v.txt"
    expect "late $v" 1 "$tmp/late-$v.txt"
done
# the undo of the wrong verb
sed 's/standalone.sh uninstall/standalone.sh status/' "$tmp/install.txt" >"$tmp/wrong.txt"
expect "wrong undo for install" 1 "$tmp/wrong.txt"
# default without its undo lines
grep -v 'UNDO' "$tmp/default2.txt" >"$tmp/noundo.txt"
expect "default without undo" 1 "$tmp/noundo.txt"
# an unknown change fails
{ cat "$tmp/install.txt"; echo 'DRY board$ sudo -n mv -- /boot/a /boot/b'; } >"$tmp/unknown.txt"
expect "unknown change" 1 "$tmp/unknown.txt"
# reads alone pass
printf 'DRY board$ sudo -n fw_printenv mp2s_left\nDRY board$ ls -l -- /boot/yakogl/standalone\n' >"$tmp/status.txt"
expect "reads only" 0 "$tmp/status.txt"
# an empty transcript is a usage error, not a pass
: >"$tmp/empty.txt"
expect "empty transcript" 2 "$tmp/empty.txt"

[ "$fail" = 0 ] && echo "mp2s_undo_order selftest: ok" || echo "mp2s_undo_order selftest: FAILED"
exit "$fail"
