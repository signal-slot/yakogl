#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_person_gate_selftest.sh - the person gate of the DK standalone runner:
# 'default <image> forever' is refused for every value of BOARD_PERSON_PRESENT
# but exactly 1 (unset, empty, 0, yes, true, ' 1', '1 ', 01), each time with
# exit non-zero, a message naming BOARD_PERSON_PRESENT and no change line
# ('DRY board$'). With =1 the person-gate message must be absent (the run may
# still refuse for another reason). A boot count is not gated.
# Only --dry-run, on the fake board (tools/mp2s_fakeboard.sh). No network, no board.
#
#   tools/mp2s_person_gate_selftest.sh
#
# Prints "mp2s_person_gate_selftest: N checks, M failed"; exit 0 only when M is 0.
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
R="$HERE/board-stm32mp2-standalone.sh"
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
unset BOARD_HOST BOARD_PERSON_PRESENT MP2_NO_WATCHDOG MP2_SUDO MP2_KNOWN_HOSTS MP2S_BOOT_MANIFEST
export MP2S_STATE_DIR=$W/state BOARD_LOCK=$W/board.lock LOCK_WAIT=5

N=0
F=0
ok() { N=$((N + 1)); echo "ok   $1"; }
bad() { N=$((N + 1)); F=$((F + 1)); echo "FAIL $1: $2"; }

if "$HERE/mp2s_fakeboard.sh" init "$W/fk" >/dev/null 2>&1; then
    eval "$("$HERE/mp2s_fakeboard.sh" shell "$W/fk")"
else
    echo "mp2s_person_gate_selftest: no fake board; running without one"
fi

# a valid test image: AArch64 Image magic at byte 56, product marker (as the runner selftest)
IMG=$W/prod.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero; printf 'lifetime %%u s (PSCI SYSTEM_RESET)\0mp2_wdtr: product refresh armed'; } >"$IMG"

# refused LABEL ENVCMD...: forever under that environment must be refused
refused() {
    local label=$1 out rc
    shift
    out=$("$@" "$R" --dry-run default "$IMG" forever 2>&1)
    rc=$?
    if [ "$rc" -ne 0 ]; then ok "$label: exit $rc"; else bad "$label: exit" "rc 0, wanted non-zero"; fi
    if grep -qF BOARD_PERSON_PRESENT <<<"$out"; then ok "$label: message names BOARD_PERSON_PRESENT"; else bad "$label: message" "no BOARD_PERSON_PRESENT in: $out"; fi
    if grep -qF 'DRY board$' <<<"$out"; then bad "$label: no change line" "$out"; else ok "$label: no change line"; fi
}
refused "forever, BOARD_PERSON_PRESENT unset" env -u BOARD_PERSON_PRESENT
for v in "" 0 yes true " 1" "1 " 01; do
    refused "forever, BOARD_PERSON_PRESENT='$v'" env BOARD_PERSON_PRESENT="$v"
done

# =1 passes the gate (may refuse later for another reason)
OUT=$(BOARD_PERSON_PRESENT=1 "$R" --dry-run default "$IMG" forever 2>&1)
if grep -qF "Set BOARD_PERSON_PRESENT=1" <<<"$OUT" || grep -qF "'forever' makes the image" <<<"$OUT"; then
    bad "forever, BOARD_PERSON_PRESENT=1 passes the person gate" "$OUT"
else ok "forever, BOARD_PERSON_PRESENT=1 passes the person gate"; fi

# a count is not gated, with or without a person
for v in unset 0 1; do
    if [ "$v" = unset ]; then
        OUT=$(env -u BOARD_PERSON_PRESENT "$R" --dry-run default "$IMG" 2 2>&1)
    else
        OUT=$(BOARD_PERSON_PRESENT=$v "$R" --dry-run default "$IMG" 2 2>&1)
    fi
    rc=$?
    if [ "$rc" = 0 ] && ! grep -qF "'forever' makes the image" <<<"$OUT"; then ok "default <image> 2 is not gated (BOARD_PERSON_PRESENT $v)"; else bad "default 2 not gated ($v)" "rc $rc: $OUT"; fi
done

echo "mp2s_person_gate_selftest: $N checks, $F failed"
[ "$F" = 0 ]
