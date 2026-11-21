#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_scanout32/verdicts_selftest.sh
#
# Hermetic self-test of verdicts.sh on synthetic logs written to $TMPDIR
# (removed at the end; nothing is written into the repository):
#
#   pass       a complete log -> 'verdicts: PASS', 'q4: yes', exit 0; the
#              6-bit FRCNT wrap (62 -> 1) counts as advancing
#   notnamed   'u3 named: no' -> FAIL, q4 no, exit 3
#   stall      FRCNT equal before and after one swap -> FAIL naming the swap
#   noalt      the same buffer twice in a row -> FAIL
#   restore    'u3 restore: FAIL' -> FAIL; a PASS whose hash differs from the
#              snapshot -> FAIL
#   word       placement overlapping the firmware span at heade, or running
#              past 4096 -> FAIL
#   refused    'u3 dlist: refused ..' -> FAIL (placement refused), q4
#              unanswered, exit 3, swap/named/restore not required
#   oldsnap    a snapshot without heade= -> incomplete (line missing)
#   hrestore   restore heade differing from the snapshot's -> FAIL
#   few        fewer than the minimum of swaps -> incomplete
#   cut        a log cut after the swaps -> incomplete, stop point named
#   gap        lines skipped between seen ones -> incomplete, gap named
#   redact     IPv4, host:port, IPv6, MAC, device path and tty name in
#              printed lines never reach the output; 0x values do
#   usage      argument count, unreadable file -> exit 2; empty log -> 1
# `sh verdicts_selftest.sh` works too: it re-runs itself under bash.
[ -n "${BASH_VERSION:-}" ] || exec bash "$0" "$@"
set -u
unset LC_ALL LANGUAGE
export LANG=C
HERE=$(cd "$(dirname "$0")" && pwd)
V=$HERE/verdicts.sh
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
check() { case "$out" in *"$1"*) ok "$2" ;; *) bad "$2: '$1' not in output" ;; esac; }
nope()  { case "$out" in *"$1"*) bad "$2: '$1' appeared in output" ;; *) ok "$2" ;; esac; }
want_rc() { [ "$rc" -eq "$1" ] && ok "$2: exit $1" || bad "$2: rc=$rc, out: $out"; }

T=$(mktemp -d "${TMPDIR:-/tmp}/verdicts32-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
[ -r "$V" ] || { echo "verdicts_selftest: $V not found" >&2; exit 2; }

bash -n "$V" && ok "verdicts.sh parses (bash -n)" || bad "verdicts.sh: bash -n"
bash -n "$0" && ok "verdicts_selftest.sh parses (bash -n)" || bad "selftest: bash -n"

# ---- fixture logs ----------------------------------------------------------
SNAP='u3 snapshot: list-ptr=0x00000000 heade=672 words=40 hash=0xdeadbeef'
DL='u3 dlist: placed word 32 len 12'  # fw at 672..711, ours at 32
PL='u3 plane: addr=0x3d000000 fmt=XRGB8888 pitch=7680 w=1920 h=1080'
S1='u3 swap 1: buf A list-ptr=0x00000020 frcnt 10 -> 11'
S2='u3 swap 2: buf B list-ptr=0x00000020 frcnt 11 -> 12'
S3='u3 swap 3: buf A list-ptr=0x00000020 frcnt 12 -> 13'
S4='u3 swap 4: buf B list-ptr=0x00000020 frcnt 62 -> 1'
NM='u3 named: yes (channel 0 list pointer 0x20 after swap 1)'
RS='u3 restore: PASS (32 words and pointer byte-equal, hash=0xdeadbeef)'
TM='rpi5_scanout32: PASS (4 swaps, list restored)'
FS='rpi5_scanout32: feeding stopped, reset expected'

mk() { # mk <file> <lines...>
    f=$1; shift
    printf '%s\n' "$@" > "$f"
}
mk "$T/pass.log" '--- Zephyr rpi5_scanout32 ---' 'get_throttled 0x50020 noise' \
    "$SNAP" "$DL" "$PL" "$S1" "$S2" "$S3" "$S4" "$NM" "$RS" "$TM" "$FS"
mk "$T/notnamed.log" "$SNAP" "$DL" "$PL" "$S1" "$S2" "$S3" "$S4" \
    'u3 named: no (channel 0 list pointer stayed 0x0)' "$RS" 'rpi5_scanout32: FAIL (our list not named)'
mk "$T/stall.log" "$SNAP" "$DL" "$PL" "$S1" 'u3 swap 2: buf B list-ptr=0x00000020 frcnt 11 -> 11' \
    "$S3" "$S4" "$NM" "$RS" 'rpi5_scanout32: FAIL (FRCNT stalled)'
mk "$T/noalt.log" "$SNAP" "$DL" "$PL" "$S1" "$S2" 'u3 swap 3: buf B list-ptr=0x00000020 frcnt 12 -> 13' \
    "$S4" "$NM" "$RS" "$TM"
mk "$T/restorefail.log" "$SNAP" "$DL" "$PL" "$S1" "$S2" "$S3" "$S4" "$NM" \
    'u3 restore: FAIL (word 7 differs)' 'rpi5_scanout32: FAIL (restore)'
mk "$T/restorehash.log" "$SNAP" "$DL" "$PL" "$S1" "$S2" "$S3" "$S4" "$NM" \
    'u3 restore: PASS (hash=0x12345678)' "$TM"
mk "$T/word.log" "$SNAP" 'u3 dlist: placed word 660 len 20' "$PL" "$S1" "$S2" "$S3" "$S4" "$NM" "$RS" "$TM"
mk "$T/past.log" "$SNAP" 'u3 dlist: placed word 4090 len 12' "$PL" "$S1" "$S2" "$S3" "$S4" "$NM" "$RS" "$TM"
mk "$T/after.log" "$SNAP" 'u3 dlist: placed word 712 len 12' "$PL" "$S1" "$S2" "$S3" "$S4" "$NM" "$RS" "$TM"
mk "$T/refused.log" "$SNAP" 'u3 dlist: refused no unused span of 12 words proven' "$TM"
mk "$T/oldsnap.log" 'u3 snapshot: list-ptr=0x00000000 words=32 hash=0xdeadbeef' "$DL" "$PL" "$S1" "$S2" "$S3" "$S4" "$NM" "$RS" "$TM"
mk "$T/hrestore.log" "$SNAP" "$DL" "$PL" "$S1" "$S2" "$S3" "$S4" "$NM" \
    'u3 restore: PASS (heade=668 hash=0xdeadbeef)' "$TM"
mk "$T/hrestoreok.log" "$SNAP" "$DL" "$PL" "$S1" "$S2" "$S3" "$S4" "$NM" \
    'u3 restore: PASS (heade=672 hash=0xdeadbeef)' "$TM"
mk "$T/few.log" "$SNAP" "$DL" "$PL" "$S1" "$S2" "$S3" "$NM" "$RS" "$TM"
mk "$T/cut.log" "$SNAP" "$DL" "$PL" "$S1" "$S2" "$S3" "$S4"
mk "$T/gap.log" "$SNAP" "$PL" "$S1" "$S2" "$S3" "$S4" "$RS" "$TM"
mk "$T/redact.log" 'uart bridge arm.example.invalid:40001 on /dev/''tty''AMA0 for 100.64''.7.9' \
    "$SNAP" "$DL" "$PL" "$S1" "$S2" "$S3" "$S4" "$NM" \
    'u3 restore: FAIL (word 7 differs; 100.64''.7.9:443 over /dev/''tty''USB3, link local fe80::1, eth de:ad:be:ef:12:34, nic aa-bb-cc-dd-ee-ff, unit serial-getty@ttyS2.service)' \
    'rpi5_scanout32: FAIL (arm.example.invalid:40001 timed out)'
: > "$T/empty.log"

run() { out=$(bash "$V" "$1" 2>&1); rc=$?; }

# ---- pass -------------------------------------------------------------------
run "$T/pass.log"
want_rc 0 "pass log"
check "verdicts: PASS (4 swaps alternating" "PASS named"
check "q4: yes" "q4 yes"
check "7 of 7 expected verdict lines" "all seven keys counted"
check "missing: none" "nothing missing"
check "[8] rpi5_scanout32: feeding stopped, reset expected" "feeding stopped printed next to PASS"
check "frcnt 62 -> 1" "the wrapped FRCNT swap prints"
nope "get_throttled" "noise lines are not verdict lines"
nope "FAIL" "no FAIL in the pass output"

# ---- our list not named ---------------------------------------------------
run "$T/notnamed.log"
want_rc 3 "not named"
check "verdicts: FAIL (our list was not named" "FAIL names the cause"
check "q4: no (channel 0 did not name our list" "q4 no"

# ---- FRCNT stall on one swap ----------------------------------------------
run "$T/stall.log"
want_rc 3 "stall"
check "FRCNT stalled at swap 2" "the stalled swap is named"
check "q4: no (FRCNT stalled at swap 2 (11 -> 11))" "q4 names the stall"
nope "stalled at swap 1" "only swap 2 stalled"

# ---- buffers do not alternate ----------------------------------------------
run "$T/noalt.log"
want_rc 3 "no alternation"
check "buffers did not alternate at swap 3" "alternation failure named"
check "q4: no (buffers did not alternate" "q4 no"

# ---- restore ---------------------------------------------------------------
run "$T/restorefail.log"
want_rc 3 "restore FAIL"
check "restore FAIL" "restore failure named"
check "q4: yes" "q4 is judged from the swaps, not the restore"
run "$T/restorehash.log"
want_rc 3 "restore hash differs"
check "restore PASS carries hash=0x12345678 but the snapshot has hash=0xdeadbeef" "hash mismatch named"

# ---- placement ----------------------------------------------------------------
run "$T/word.log"
want_rc 3 "overlap"
check "placed [660,680) overlaps the firmware list [672,712)" "overlap named"
run "$T/past.log"
want_rc 3 "past 4096"
check "placed [4090,4102) runs past word 4096" "past 4096 named"
run "$T/after.log"
want_rc 0 "placed right after the firmware span"
check "q4: yes" "adjacent placement is valid"
run "$T/refused.log"
want_rc 3 "refused"
check "verdicts: FAIL (placement refused: no unused span of 12 words proven)" "refusal is a FAIL"
check "q4: unanswered (placement refused)" "refusal leaves q4 unanswered"
nope "missing: u3 swap" "swap lines not required after a refusal"
run "$T/oldsnap.log"
want_rc 1 "old snapshot"
check "missing: u3 snapshot" "snapshot without heade= is missing"
run "$T/hrestore.log"
want_rc 3 "restore heade mismatch"
check "restore carries heade=668 but the snapshot has heade=672" "heade mismatch named"
run "$T/hrestoreok.log"
want_rc 0 "restore heade equal"

# ---- truncated, too few swaps, gap ------------------------------------------
run "$T/cut.log"
want_rc 1 "cut log"
check "verdicts: incomplete" "cut: incomplete"
check "q4: unanswered" "cut: q4 unanswered"
check "4 of 7 expected verdict lines" "cut: four keys seen"
check "missing: u3 named u3 restore rpi5_scanout32 terminal" "cut: missing lines named"
check "[4] u3 swap 4: buf B" "cut: the stop point is the last swap"
check "the image stopped there" "cut: stop claimed"
if grep -q '^verdicts: PASS\|^verdicts: FAIL' <<< "$out"; then bad "cut names a verdict"; else ok "cut names no verdict"; fi
run "$T/few.log"
want_rc 1 "too few swaps"
check "u3 swap (need 4, saw 3)" "few: the swap shortfall named"
check "gap: missing between seen lines: u3 swap" "few: the shortfall is not a tail"
run "$T/gap.log"
want_rc 1 "gap log"
check "gap: missing between seen lines: u3 dlist" "gap named"
nope "q4: yes" "gap: no q4 answer"

# ---- redaction --------------------------------------------------------------
run "$T/redact.log"
want_rc 3 "redact log"
check "0x" "0x values print unredacted"
nope "100.64" "no IPv4"
nope "arm.example" "no host"
nope "fe80" "no IPv6"
nope "de:ad:be:ef" "no MAC (colon)"
nope "aa-bb-cc-dd-ee-ff" "no MAC (dash)"
nope "/dev/""tty""USB3" "no device path"
nope "tty""USB3" "no UART name"
nope "ttyS2" "no tty name in a unit"
check "<host-port>" "host:port marker"
check "<ipv6/mac>" "IPv6/MAC marker"
check "<uart>" "UART marker"

# ---- usage ------------------------------------------------------------------
out=$(bash "$V" 2>&1); rc=$?; want_rc 2 "no arguments"
out=$(bash "$V" "$T/pass.log" extra 2>&1); rc=$?; want_rc 2 "two arguments"
out=$(bash "$V" "$T/no-such.log" 2>&1); rc=$?; want_rc 2 "unreadable log"
run "$T/empty.log"
want_rc 1 "empty log"
check "no expected verdict line was seen at all" "empty log says nothing was seen"

if [ $fail -eq 0 ]; then
    echo "verdicts_selftest: all checks passed"
else
    echo "verdicts_selftest: $fail failed"
    exit 1
fi
