#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_dispatch_transcript_selftest.sh - no-board selftest of the DK's
# standalone verbs through the tools/board.sh front door
# (docs/notes/stm32mp257f-dk-standalone.md, section 7; board.sh's dispatch of
# plan #362). board.sh exec's
#   [--dry-run] stm32mp257f-dk status | install | uninstall | nodefault
#   [--dry-run] stm32mp257f-dk default <image> <1-9|forever>
# to tools/board-stm32mp2-standalone.sh, MP2S_STANDALONE_RUNNER overriding the
# runner path. This checks that the front door is a pure pass-through, not a
# second implementation: for each verb, with tools/mp2s_fakeboard.sh as the
# board (its PATH shims in place; --dry-run sends no ssh either, so nothing
# can reach a network), the board.sh run must end with the exit code of the
# same direct runner call and print byte-for-byte the same transcript, and
# that transcript must pass tools/mp2s_undo_order.py (every undo before its
# change) and tools/mp2s_protect_scan.py (no write of a protected variable,
# partition or the eMMC). 'forever' without BOARD_PERSON_PRESENT=1 must be
# refused through the front door too, with a message naming
# BOARD_PERSON_PRESENT. The fake's writes.log must stay empty: dry-run writes
# nothing anywhere. BOARD_HOST is the placeholder user@example.invalid.
#
# A mismatch is a finding: the dispatch or the runner changed so the two
# entry points disagree; report it (new issue) rather than bending the checks.
#
#   tools/mp2s_dispatch_transcript_selftest.sh
#
# Prints one line per check and "mp2s_dispatch_transcript selftest: N ok, F failed";
# exit 0 only when F is 0.
set -u
export LC_ALL=C
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/.." && pwd)
cd "$repo" || exit 2
board=$here/board.sh
fb=$here/mp2s_fakeboard.sh
undo=$here/mp2s_undo_order.py
protect=$here/mp2s_protect_scan.py
runner=${MP2S_STANDALONE_RUNNER:-$here/board-stm32mp2-standalone.sh}
work=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_dispatch.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
D=$work/dk

# the environment the selftest owns: no leak in, nothing kept after
unset BOARD_HOST BOARD_PERSON_PRESENT MP2_NO_WATCHDOG MP2_SUDO MP2_KNOWN_HOSTS MP2S_BOOT_MANIFEST
export MP2S_STATE_DIR=$work/state BOARD_LOCK=$work/board.lock LOCK_WAIT=5
export MP2S_STANDALONE_RUNNER=$runner

npass=0
nfail=0
ok() { npass=$((npass + 1)); echo "ok   $1"; }
bad() { nfail=$((nfail + 1)); echo "FAIL $1${2:+: $2}"; }

# ---- the fake board: init, and its shims in PATH (the ssh shim, never ssh)
"$fb" init "$D" >"$work/init.log" 2>&1 || { echo "FAIL fakeboard init: $(tail -3 "$work/init.log")"; exit 1; }
# shellcheck disable=SC1090
eval "$("$fb" shell "$D")" || { echo "FAIL fakeboard shell"; exit 1; }
if [ "$(command -v ssh)" = "$D/bin/ssh" ]; then ok "ssh resolves to the fake's shim (no real ssh is reachable)"; else bad "ssh resolves to the fake's shim" "$(command -v ssh)"; fi

# ---- the images the default verb takes: the board layer's watchdog marker
# (a counted default), the same one for the forever refusal (the person gate
# fires before the image's markers are read)
IMG=$work/img.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero; printf 'gc_wdog_arm: armed'; } >"$IMG"

# one_case NAME WANT_RC ARGS...: run ARGS (the runner's arguments, starting
# with --dry-run) twice, once as the direct runner call and once as
# board.sh --dry-run stm32mp257f-dk ARGS; the two invocations are the same
# words with board.sh and the board name in front.
direct_out=$work/direct.out
via_out=$work/via.out
one_case() {
    local name=$1 want=$2
    shift 2
    local rc_direct rc_via
    "$runner" "$@" >"$direct_out" 2>&1
    rc_direct=$?
    "$board" --dry-run stm32mp257f-dk "$@" >"$via_out" 2>&1
    rc_via=$?
    if [ "$rc_direct" = "$want" ]; then ok "$name: direct exit $rc_direct"; else bad "$name: direct exit" "rc $rc_direct, wanted $want: $(head -c 300 "$direct_out")"; fi
    if [ "$rc_via" = "$rc_direct" ]; then ok "$name: board.sh keeps the exit code ($rc_via)"; else bad "$name: board.sh exit code" "via $rc_via, direct $rc_direct: $(head -c 300 "$via_out")"; fi
    if cmp -s "$direct_out" "$via_out"; then ok "$name: transcript identical through board.sh ($(wc -l <"$via_out") lines)"; else bad "$name: transcript" "$(diff "$direct_out" "$via_out" | head -6 | tr '\n' ' ')"; fi
}

# ---- the five verbs, dry-run: identical, and the transcript passes the checks
for verb in status install uninstall nodefault; do
    one_case "$verb" 0 --dry-run "$verb"
    if python3 -I "$undo" "$via_out" >"$work/undo.log" 2>&1; then
        ok "$verb: undo_order accepts the transcript ($(cat "$work/undo.log"))"
    else
        bad "$verb: undo_order" "$(head -c 300 "$work/undo.log")"
    fi
    if python3 -I "$protect" "$via_out" >"$work/protect.log" 2>&1; then
        ok "$verb: protect_scan finds no protected write"
    else
        bad "$verb: protect_scan" "$(head -c 300 "$work/protect.log")"
    fi
done
one_case "default 2" 0 --dry-run default "$IMG" 2
if python3 -I "$undo" "$via_out" >"$work/undo.log" 2>&1; then
    ok "default 2: undo_order accepts the transcript ($(cat "$work/undo.log"))"
else
    bad "default 2: undo_order" "$(head -c 300 "$work/undo.log")"
fi
if python3 -I "$protect" "$via_out" >"$work/protect.log" 2>&1; then
    ok "default 2: protect_scan finds no protected write"
else
    bad "default 2: protect_scan" "$(head -c 300 "$work/protect.log")"
fi

# ---- forever without a person: refused through the front door, rc 2
one_case "default forever (no person)" 2 --dry-run default "$IMG" forever
if grep -q BOARD_PERSON_PRESENT "$via_out"; then
    ok "the refusal names BOARD_PERSON_PRESENT"
else
    bad "the refusal names BOARD_PERSON_PRESENT" "$(head -c 300 "$via_out")"
fi

# ---- nothing was written: dry-run touched no board, and the fake saw no write
writes=$("$fb" writes "$D")
if [ -z "$writes" ]; then
    ok "the fake board's writes.log is empty (dry-run wrote nothing)"
else
    bad "the fake board's writes.log is empty" "$writes"
fi
if grep -q VIOLATION "$D/writes.log"; then
    bad "no VIOLATION in the fake" "$(grep VIOLATION "$D/writes.log" | head -3)"
else
    ok "no VIOLATION line in the fake"
fi

echo "mp2s_dispatch_transcript selftest: $npass ok, $nfail failed"
[ "$nfail" = 0 ]
