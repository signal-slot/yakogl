#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Selftest of the person gate of tools/board-stm32mp2-standalone.sh: 'default
# <image> forever' is refused (rc 2, nothing sent, no writes) for every value
# of BOARD_PERSON_PRESENT but exactly 1. Only --dry-run and the fake board
# (tools/mp2s_fakeboard.sh); no board, no network.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
RUNNER=$HERE/board-stm32mp2-standalone.sh
W=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_person_gate_st.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
FB=$W/fb
N=0 F=0
ok()  { N=$((N + 1)); echo "ok   $1"; }
bad() { N=$((N + 1)); F=$((F + 1)); echo "FAIL $1: $2"; }

unset BOARD_PERSON_PRESENT MP2_NO_WATCHDOG MP2_SUDO MP2_KNOWN_HOSTS MP2S_BOOT_MANIFEST
export MP2S_STATE_DIR=$W/state BOARD_LOCK=$W/board.lock LOCK_WAIT=5
"$HERE/mp2s_fakeboard.sh" init "$FB" >/dev/null || { echo "fake board init failed"; exit 1; }
eval "$("$HERE/mp2s_fakeboard.sh" shell "$FB")"
export BOARD_HOST=user@host

IMG=$W/img.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero; printf 'gc_wdog_arm: armed'; } >"$IMG"

REFUSAL="makes the image every boot's"

# every value but exactly 1: unset, empty, 0, 2, 01, blanks, words
run_forever() { # VALUE (UNSET = variable not set)
  if [ "$1" = UNSET ]; then
    env -u BOARD_PERSON_PRESENT "$RUNNER" --dry-run default "$IMG" forever 2>&1
  else
    BOARD_PERSON_PRESENT=$1 "$RUNNER" --dry-run default "$IMG" forever 2>&1
  fi
}
for v in UNSET "" 0 2 01 " 1" "1 " 11 -1 true yes on 1x; do
  out=$(run_forever "$v"); rc=$?
  lbl="forever refused with BOARD_PERSON_PRESENT=[$v]"
  if [ "$rc" != 2 ]; then bad "$lbl" "rc $rc, wanted 2"
  elif ! printf '%s' "$out" | grep -q "$REFUSAL"; then bad "$lbl" "no person-gate refusal text"
  else ok "$lbl"; fi
done

# exactly 1: the person gate does not refuse (any other outcome is fine)
out=$(run_forever 1)
if printf '%s' "$out" | grep -q "$REFUSAL"; then
  bad "forever with BOARD_PERSON_PRESENT=1" "person gate refused"
else ok "forever with BOARD_PERSON_PRESENT=1 passes the person gate"; fi

# a counted default needs no person
out=$(env -u BOARD_PERSON_PRESENT "$RUNNER" --dry-run default "$IMG" 2 2>&1)
if printf '%s' "$out" | grep -q "$REFUSAL"; then
  bad "counted default without a person" "person gate refused"
else ok "counted default 2 needs no person"; fi

# the runs wrote nothing on the fake board
w=$("$HERE/mp2s_fakeboard.sh" writes "$FB" 2>/dev/null | wc -l)
if [ "$w" = 0 ]; then ok "no write on the fake board"; else bad "no write on the fake board" "$w line(s) in writes.log"; fi

echo "$N checks, $F failed"
[ "$F" = 0 ]
