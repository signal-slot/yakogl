#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_dispatch_transcript_selftest.sh - the DK's standalone verbs through the
# tools/board.sh front door, on the fake board (no board, no ssh, no network).
#
# tools/board.sh dispatches
#   tools/board.sh [--dry-run] stm32mp257f-dk status | install | uninstall | nodefault
#   tools/board.sh [--dry-run] stm32mp257f-dk default <image> <1-9|forever>
# to tools/board-stm32mp2-standalone.sh (MP2S_STANDALONE_RUNNER overriding the
# runner path), exec'ing it with --dry-run passed first and its exit code
# kept. This checks that the front door is a pure pass-through, not a second
# implementation. For every verb, run once through board.sh and once through
# the runner directly with the same words in the same order, the two runs must
# agree:
#
#   - the exit codes are the same, and each one the runner's documented code
#     for that case (0 done, 2 refused before anything was changed);
#   - the transcripts are byte for byte the same, except the runner's own
#     scratch directory: its dry-run printer (the runner's host()) strips the
#     tree path first, so when TMPDIR lies inside the tree — as in a gate row
#     (tools/check.sh selftest_row sets it under build/) — a line keeps the
#     tree prefix and the per-run mktemp name. The two invocations take
#     different scratch directories, so the selftest hides the scratch path
#     (with and without the tree prefix, the mktemp directory and its parent)
#     behind <runner-scratch> in both transcripts before comparing.
#     undo_order and protect_scan read the raw board.sh transcript, never the
#     hidden one;
#   - tools/mp2s_undo_order.py accepts that transcript (every undo printed
#     before its change, docs/notes/stm32mp257f-dk-standalone.md section 5);
#   - tools/mp2s_protect_scan.py, given that transcript as a file, finds no
#     write of a protected U-Boot variable (bootcmd, bootcount, altbootcmd,
#     ...), of a protected partition (fsbla1/2, metadata1/2, fip-a/b) or of
#     the eMMC.
#
# 'forever' without BOARD_PERSON_PRESENT=1 must be refused through the front
# door exactly as the runner refuses it: the same exit 2, a message naming
# BOARD_PERSON_PRESENT, and no board line at all — nothing was about to
# change. 'default <image> forever' with BOARD_PERSON_PRESENT=1 is a dry run
# like any other verb: it touches no board, and its two transcripts must
# agree as well. After every case the fake board's writes.log is empty, holds
# no VIOLATION line, and its /boot still matches the manifest taken before
# the first run (tools/mp2s_boot_manifest.py).
#
# The board is tools/mp2s_fakeboard.sh: init, then eval its shell (its shims
# first in PATH, so even a stray ssh would reach the fake, never the network;
# dry-run sends none either). BOARD_HOST is the placeholder
# user@example.invalid (RFC 2606; the fake ssh ignores it).
# MP2S_STANDALONE_RUNNER is pinned to this checkout's runner, so an inherited
# override cannot point the front door somewhere else.
#
# A mismatch is a finding of the dispatch or of the runner, not of this file:
# report it (a new issue), never bend a check here.
#
#   tools/mp2s_dispatch_transcript_selftest.sh
#
# Prints one line per check and "mp2s_dispatch_transcript_selftest: N checks,
# F failed"; exit 0 only when F is 0, 1 with findings, 2 on a setup error.
# Writes only under TMPDIR (/var/tmp when TMPDIR is unset); MP2S_DT_KEEP names
# this run's work directory to keep.
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
BOARD=$HERE/board.sh
RUNNER=$HERE/board-stm32mp2-standalone.sh
FB=$HERE/mp2s_fakeboard.sh
UNDO=$HERE/mp2s_undo_order.py
PROTECT=$HERE/mp2s_protect_scan.py
IMG_CHECK=$HERE/mp2s_image_check.py
MANIFEST=$HERE/mp2s_boot_manifest.py
for f in "$BOARD" "$RUNNER" "$FB" "$UNDO" "$PROTECT" "$IMG_CHECK" "$MANIFEST"; do
    [ -f "$f" ] || { echo "mp2s_dispatch_transcript_selftest: no $f"; exit 2; }
done

W=$(mktemp -d "${TMPDIR:-/var/tmp}/mp2s_dispatch.XXXXXX") ||
    { echo "mp2s_dispatch_transcript_selftest: no work directory"; exit 2; }
D=$W/fakeboard
[ -n "${MP2S_DT_KEEP:-}" ] || trap 'rm -rf "$W"' EXIT
# the runner keeps its scratch directory in TMPDIR: keep it under W (nothing
# of a run is left behind) and under one known parent, so the transcript
# comparison has exactly the mktemp suffix to hide
export TMPDIR=$W

N=0
F=0
ok() { N=$((N + 1)); echo "ok   $1"; }
bad() { N=$((N + 1)); F=$((F + 1)); echo "FAIL $1: $2"; }
check() { if [ "$2" = 0 ]; then ok "$1"; else bad "$1" "${3:-}"; fi; }
checkrc() { if [ "$2" = "$3" ]; then ok "$1 ($2)"; else bad "$1" "rc $2, wanted $3: ${4:-}"; fi; }
short() { head -c 300 "$1" | tr '\n' ' '; }

# ---- the environment this selftest owns: nothing inherited, nothing kept
for v in $(compgen -e); do
    case $v in
    MP2S_DT_KEEP) ;;
    BOARD | BOARD_* | MP2_* | MP2S_* | LOCK | LOCK_WAIT | HOLD_MAX) unset "$v" ;;
    esac
done
export MP2S_STANDALONE_RUNNER=$RUNNER   # the front door runs this checkout's runner
export MP2S_STATE_DIR=$W/state BOARD_LOCK=$W/board.lock LOCK_WAIT=5

# ---- the fake board: init, then its shims first in PATH (the ssh shim: no
# real ssh is reachable from this run)
"$FB" init "$D" >"$W/init.log" 2>&1 ||
    { echo "mp2s_dispatch_transcript_selftest: fake board init failed: $(short "$W/init.log")"; exit 2; }
# shellcheck disable=SC1090
eval "$("$FB" shell "$D")"
export BOARD_HOST=user@example.invalid
if [ "$(command -v ssh)" = "$D/bin/ssh" ]; then
    ok "ssh resolves to the fake's shim (no board, no network reachable from here)"
else
    bad "ssh resolves to the fake's shim" "$(command -v ssh)"
fi

# ---- the test image: the arm64 Image magic (ARMd at byte 56), a footprint of
# 0 inside the go window, the board layer's watchdog marker and the product
# refresh marker (the same image thus serves 'default 2' and 'forever' with a
# person), and no 'yakogl-lifetime-s=' marker
IMG=$W/img.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero;
  printf 'gc_wdog_arm: armed'; head -c 256 /dev/zero;
  printf 'mp2_wdtr: product refresh armed'; } >"$IMG"
python3 -I "$IMG_CHECK" "$IMG" >"$W/ic.out" 2>&1 ||
    { echo "mp2s_dispatch_transcript_selftest: the test image fails mp2s_image_check.py: $(short "$W/ic.out")"; exit 2; }

# the fake /boot as init made it: every later compare must say 'same'
python3 -I "$MANIFEST" record "$D/boot" -o "$W/first.manifest" >"$W/rec.log" 2>&1 ||
    { echo "mp2s_dispatch_transcript_selftest: mp2s_boot_manifest.py record failed: $(short "$W/rec.log")"; exit 2; }

# ---- transcript normalization: hide the runner's scratch, in every form it
# can reach the output (the runner strips the tree prefix before its own
# scratch path, so both the absolute and the tree-relative form appear)
work_abs=$W
work_real=$(cd "$W" && pwd)
work_rel=${W#"$REPO"/}
work_rel2=${work_real#"$REPO"/}
esc() { printf '%s' "$1" | sed 's#[][\\.~^$*+?(){}|/&]#\\&#g'; }
norm() {   # IN OUT
    local base
    local -a ses=()
    for base in "$work_abs" "$work_real" "$work_rel" "$work_rel2"; do
        [ -n "$base" ] || continue
        ses+=(-e "s|$(esc "$base")/tmp\.[A-Za-z0-9]+|<runner-scratch>|g")
    done
    for base in "$work_abs" "$work_real" "$work_rel" "$work_rel2"; do
        [ -n "$base" ] || continue
        ses+=(-e "s|$(esc "$base")|<runner-work>|g")
    done
    sed -E "${ses[@]}" "$1" >"$2"
}

# one_case NAME WANT_RC VAR=VALUE... -- ARGS...: run ARGS (the runner's own
# arguments, --dry-run included) twice: directly on the runner, and through
# board.sh --dry-run stm32mp257f-dk with the same words in the same order.
DOUT=$W/direct.out
VOUT=$W/via.out
one_case() {
    local name=$1 want=$2 rc_d=0 rc_v=0
    shift 2
    local -a envs=()
    while [ $# -gt 0 ] && [ "$1" != -- ]; do envs+=("$1"); shift; done
    [ $# = 0 ] || shift
    env ${envs[@]+"${envs[@]}"} bash "$RUNNER" "$@" >"$DOUT" 2>&1; rc_d=$?
    env ${envs[@]+"${envs[@]}"} bash "$BOARD" --dry-run stm32mp257f-dk "$@" >"$VOUT" 2>&1; rc_v=$?
    checkrc "$name: direct exit" "$rc_d" "$want" "$(short "$DOUT")"
    checkrc "$name: board.sh keeps the runner's exit code" "$rc_v" "$rc_d" "direct $rc_d: $(short "$VOUT")"
    norm "$DOUT" "$W/direct.norm"
    norm "$VOUT" "$W/via.norm"
    if cmp -s "$W/direct.norm" "$W/via.norm"; then
        ok "$name: transcript identical through board.sh ($(wc -l <"$VOUT") lines)"
    else
        bad "$name: transcript" "$(diff "$W/direct.norm" "$W/via.norm" | head -6 | tr '\n' ' ')"
    fi
}

# transcript_checks NAME: the raw board.sh transcript passes the two checks
# the design gives a dry-run transcript
transcript_checks() {
    local name=$1 out
    if python3 -I "$UNDO" "$VOUT" >"$W/undo.log" 2>&1; then
        ok "$name: undo_order accepts the transcript ($(short "$W/undo.log"))"
    else
        bad "$name: undo_order" "$(short "$W/undo.log")"
    fi
    if out=$(python3 -I "$PROTECT" "$VOUT" 2>&1); then
        ok "$name: protect_scan finds no protected write in the transcript"
    else
        bad "$name: protect_scan" "$(short "$out")"
    fi
}

# ---- the five dispatched verbs, dry-run: identical doors, accepted transcripts
for verb in status install uninstall nodefault; do
    one_case "$verb" 0 -- --dry-run "$verb"
    transcript_checks "$verb"
done
one_case "default 2" 0 -- --dry-run default "$IMG" 2
transcript_checks "default 2"

# ---- forever with a person, dry-run: a verb like any other through the door
one_case "default forever (person present)" 0 BOARD_PERSON_PRESENT=1 -- --dry-run default "$IMG" forever
transcript_checks "default forever (person present)"

# ---- forever without a person: refused at both doors, naming the gate, with
# nothing about to change
one_case "default forever (no person)" 2 -- --dry-run default "$IMG" forever
if grep -q BOARD_PERSON_PRESENT "$VOUT"; then
    ok "the refusal through board.sh names BOARD_PERSON_PRESENT"
else
    bad "the refusal through board.sh names BOARD_PERSON_PRESENT" "$(short "$VOUT")"
fi
if grep -q BOARD_PERSON_PRESENT "$DOUT"; then
    ok "the direct refusal names BOARD_PERSON_PRESENT"
else
    bad "the direct refusal names BOARD_PERSON_PRESENT" "$(short "$DOUT")"
fi
if ! grep -q '^DRY board\$' "$VOUT" && ! grep -q '^DRY board\$' "$DOUT"; then
    ok "both refusals printed no board line: they refused before anything would change"
else
    bad "both refusals printed no board line" "$(grep -h '^DRY board\$' "$DOUT" "$VOUT" | head -2 | tr '\n' ' ')"
fi
python3 -I "$UNDO" "$VOUT" >"$W/undo.log" 2>&1
rc=$?
checkrc "the refusal is no dry-run transcript (undo_order's own verdict on it)" "$rc" 2 "$(short "$W/undo.log")"

# ---- the door is exactly these verbs: the runner's read-only printenv is not
# a board.sh verb, and board.sh says so itself, naming the runner, exit 2
"$BOARD" --dry-run stm32mp257f-dk printenv >"$W/door.log" 2>&1
rc=$?
if [ "$rc" = 2 ] && grep -q board-stm32mp2-standalone.sh "$W/door.log"; then
    ok "printenv is refused by board.sh itself (exit 2, the message names the runner)"
else
    bad "printenv is refused by board.sh itself" "rc $rc: $(short "$W/door.log")"
fi

# ---- nothing was written anywhere: a dry-run sent no change, to the fake or
# to its /boot
writes=$("$FB" writes "$D")
if [ -z "$writes" ]; then
    ok "the fake board's writes.log is empty (the dry-runs wrote nothing)"
else
    bad "the fake board's writes.log is empty" "$(printf '%s\n' "$writes" | head -3 | tr '\n' ' ')"
fi
if grep -q VIOLATION "$D/writes.log"; then
    bad "no VIOLATION line in the fake board" "$(grep VIOLATION "$D/writes.log" | head -3 | tr '\n' ' ')"
else
    ok "no VIOLATION line in the fake board"
fi
python3 -I "$MANIFEST" compare "$D/boot" "$W/first.manifest" >"$W/cmp.log" 2>&1
rc=$?
if [ "$rc" = 0 ] && grep -q '^mp2s-boot: same' "$W/cmp.log"; then
    ok "the fake board's /boot is byte for byte as init made it"
else
    bad "the fake board's /boot is byte for byte as init made it" "rc $rc: $(short "$W/cmp.log")"
fi

echo "mp2s_dispatch_transcript_selftest: $N checks, $F failed"
[ "$F" = 0 ]
