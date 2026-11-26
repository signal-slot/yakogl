#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_person_gate_selftest.sh - the person gate of
# tools/board-stm32mp2-standalone.sh (acceptance criterion c4 of plan #362):
# `--dry-run default IMAGE forever` is refused for every value of
# BOARD_PERSON_PRESENT but exactly 1.
#
# For BOARD_PERSON_PRESENT unset, empty, 0, yes, true, ' 1', '1 ', 01 (and a
# few more: 2, 11, -1, on, 1x, a newline after 1) each run must exit
# non-zero, print a message naming BOARD_PERSON_PRESENT and print no change
# line (no "DRY board$" or "DRY host$" line: the refusal comes before any
# command is planned). With exactly 1 the person-gate message must be absent
# (the run may still refuse later, e.g. the test image has no product
# marker; that is not this test's concern). `default IMAGE 2` is not gated:
# no person-gate message, unset or 0. Every run is a --dry-run against the
# fake board (tools/mp2s_fakeboard.sh), whose writes.log must stay empty.
# No board, no network.
#
#   tools/mp2s_person_gate_selftest.sh
#
# Prints "mp2s_person_gate_selftest: N checks, M failed"; exit 0 only when M
# is 0.
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
R=$HERE/board-stm32mp2-standalone.sh
FAKE=$HERE/mp2s_fakeboard.sh
W=$(mktemp -d) || exit 2
trap 'rm -rf "$W"' EXIT
unset BOARD_HOST BOARD_PERSON_PRESENT MP2_NO_WATCHDOG MP2_SUDO MP2_KNOWN_HOSTS MP2S_BOOT_MANIFEST
export MP2S_STATE_DIR=$W/state BOARD_LOCK=$W/board.lock LOCK_WAIT=5

N=0
F=0
ok() { N=$((N + 1)); echo "ok   $1"; }
bad() { N=$((N + 1)); F=$((F + 1)); echo "FAIL $1: $2"; }

# the fake board: its shims first in PATH, every write logged
"$FAKE" init "$W/fb" >/dev/null || { echo "mp2s_person_gate_selftest: fake board init failed"; exit 2; }
eval "$("$FAKE" shell "$W/fb")"
export BOARD_HOST=u@fake   # a placeholder: the fake ssh ignores it

# a valid test image, as tools/board_stm32mp2_standalone_selftest.sh builds
# it: the AArch64 Image magic at byte 56 and the watchdog marker
IMG=$W/img.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero; printf 'gc_wdog_arm: armed'; } >"$IMG"

# the gate's message names the variable; the gate is the only place that
# names it in a refusal of `default`
GATE=BOARD_PERSON_PRESENT
CHANGE='^DRY (board|host)\$'

# run VALUE COUNT: the dry-run default; VALUE UNSET leaves the variable unset
run() {
    if [ "$1" = UNSET ]; then
        OUT=$(env -u BOARD_PERSON_PRESENT "$R" --dry-run default "$IMG" "$2" 2>&1)
    else
        OUT=$(env BOARD_PERSON_PRESENT="$1" "$R" --dry-run default "$IMG" "$2" 2>&1)
    fi
    RC=$?
}
show() { printf '%q' "$1"; }

# 1. every value but exactly 1 is refused by the gate
for v in UNSET '' 0 yes true ' 1' '1 ' 01 2 11 -1 on 1x $'1\n'; do
    if [ "$v" = UNSET ]; then lbl="forever, BOARD_PERSON_PRESENT unset"; else lbl="forever, BOARD_PERSON_PRESENT=$(show "$v")"; fi
    run "$v" forever
    if [ "$RC" != 0 ]; then ok "$lbl: exits non-zero ($RC)"; else bad "$lbl: exits non-zero" "rc 0: $OUT"; fi
    if grep -qF -e "$GATE" <<<"$OUT"; then ok "$lbl: the message names $GATE"; else bad "$lbl: the message names $GATE" "not in: $OUT"; fi
    if grep -Eq -e "$CHANGE" <<<"$OUT"; then bad "$lbl: no change line" "$(grep -E -e "$CHANGE" <<<"$OUT")"; else ok "$lbl: no change line"; fi
done

# 2. exactly 1 gets past the gate (a later refusal is allowed)
run 1 forever
if grep -qF -e "$GATE" <<<"$OUT"; then bad "forever, BOARD_PERSON_PRESENT=1: past the person gate" "gate message: $OUT"; else ok "forever, BOARD_PERSON_PRESENT=1: past the person gate (rc $RC)"; fi

# 3. a counted default is not gated
for v in UNSET 0; do
    run "$v" 2
    if grep -qF -e "$GATE" <<<"$OUT"; then bad "default 2, BOARD_PERSON_PRESENT $v: not gated" "gate message: $OUT"; else ok "default 2, BOARD_PERSON_PRESENT $v: not gated"; fi
    if [ "$RC" = 0 ]; then ok "default 2, BOARD_PERSON_PRESENT $v: the dry run succeeds"; else bad "default 2, BOARD_PERSON_PRESENT $v: the dry run succeeds" "rc $RC: $OUT"; fi
    # the pattern of section 1 does match the runner's planned changes
    if grep -Eq -e "$CHANGE" <<<"$OUT"; then ok "default 2, BOARD_PERSON_PRESENT $v: prints change lines"; else bad "default 2, BOARD_PERSON_PRESENT $v: prints change lines" "none in: $OUT"; fi
done

# 4. nothing was written on the fake board
if [ -s "$W/fb/writes.log" ]; then bad "no write on the fake board" "$(cat "$W/fb/writes.log")"; else ok "no write on the fake board"; fi

echo "mp2s_person_gate_selftest: $N checks, $F failed"
[ "$F" = 0 ]
