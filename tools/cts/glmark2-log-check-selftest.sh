#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/glmark2-log-check-selftest.sh
#
# Host-only self-test of tools/cts/glmark2-log-check.sh on synthetic log
# directories: a clean one passes; one without state-compare.txt fails c1; one
# hit per redaction class (IPv4, MAC, home path, ttyUSB, ttyACM, login at a
# host) fails c2 with <file>:<line> and the class named and the matched text
# masked; hits in a subdirectory and in a binary file are found; both checks
# failing give FAIL 2; usage errors exit 2. No output may carry the scratch
# path or any text the redaction expression matches.
#
# The sensitive strings are built from pieces at run time, so this file holds
# none (tools/release/redaction-scan.py reads it). Documentation values only:
# 198.51.100.0/24 (RFC 5737), a locally administered MAC, .example hosts.
#
# Scratch: a directory under $TMPDIR, or under the gitignored target/ of the
# repository when TMPDIR is unset or under /tmp or /var/tmp (as
# tools/cts/cts-selftest.sh does); removed on exit.
#
# Run by tools/cts/cts-selftest.sh as check 62 with its ok count pinned; ends on
# "glmark2-log-check-selftest: N checks passed".
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
CHECK="$HERE/glmark2-log-check.sh"
NAME=glmark2-log-check-selftest

base=${TMPDIR:-}
case $( [ -n "$base" ] && [ -d "$base" ] && cd "$base" && pwd -P) in
    ''|/tmp|/tmp/*|/var/tmp|/var/tmp/*) base=$ROOT/target; mkdir -p "$base" ;;
esac
scratch=$(mktemp -d "$base/$NAME.XXXXXX") || { echo "  FAIL  mktemp -d under the scratch base"; exit 1; }
trap 'rm -rf "$scratch"' EXIT

pass=0
fail=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

# The checker's c2 expression, rebuilt here so a hit in the checker's output
# is caught (the checker must mask what it found).
RX='([0-9]{1,3}\.){3}[0-9]{1,3}|([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}|/home/|/dev/tty(USB|ACM)|@[a-z0-9-]+\.'

# Sensitive values, from pieces.
d=.
c=:
s=/
IP="198${d}51${d}100${d}7"
MAC="02${c}00${c}5e${c}10${c}00${c}01"
HOMEP="${s}home${s}someone${s}yakogl"
TTYUSB="${s}dev${s}ttyUSB0"
TTYACM="${s}dev${s}ttyACM1"
LOGIN="builder@board-1${d}example"

# clean <dir>: the three files of a session log, with look-alikes that the
# expression does not match (a three-part version, /homework, ttyS2, a@ sign).
clean() {
    mkdir -p "$1"
    printf 'scene\tran\tfps\nbuild:use-vbo=false\tyes\t312.5\n' > "$1/tally-table.txt"
    printf '| Mali: NanoPC-T6 glmark2-es2 | 16 of 33 | v1.2.3 |\n' > "$1/tally-results-row.txt"
    printf 'before == after\n/homework/notes\n/dev/ttyS2\nmail me @ noon\n' > "$1/state-compare.txt"
}

# run <dir>: the checker's output in $out, its exit status in $rc.
run() { out=$(bash "$CHECK" "$@" 2>&1); rc=$?; }

# leak <case>: no output line carries the scratch path or a c2 match.
leak() {
    if printf '%s\n' "$out" | grep -qF -- "$scratch"; then
        bad "$1: output carries the log directory's path"
    elif printf '%s\n' "$out" | grep -qE -- "$RX"; then
        bad "$1: output carries text the redaction expression matches"
    else
        ok "$1: output has no path and no matched text"
    fi
}

show() { printf '%s\n' "$out" | sed 's/^/        | /'; }

# ---- clean directory ----------------------------------------------------------
clean "$scratch/clean"
run "$scratch/clean"
if [ "$rc" -eq 0 ] && [ "$(printf '%s\n' "$out" | tail -1)" = "glmark2-log-check: ok" ] \
   && printf '%s\n' "$out" | grep -qx 'c1 layout: ok (tally-table.txt, tally-results-row.txt, state-compare.txt)' \
   && printf '%s\n' "$out" | grep -qx 'c2 redaction: ok (no match in 3 file(s))'; then
    ok "clean: exit 0, c1 ok, c2 ok over 3 files, ends on ok"
else
    bad "clean: rc $rc"; show
fi
leak clean

# ---- missing state-compare.txt -----------------------------------------------
clean "$scratch/nostate"
rm "$scratch/nostate/state-compare.txt"
run "$scratch/nostate"
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -qx 'c1 layout: FAIL missing state-compare.txt' \
   && printf '%s\n' "$out" | grep -q '^c2 redaction: ok' \
   && [ "$(printf '%s\n' "$out" | tail -1)" = "glmark2-log-check: FAIL 1" ]; then
    ok "no state-compare.txt: exit 1, c1 names it, c2 ok, FAIL 1"
else
    bad "no state-compare.txt: rc $rc"; show
fi

# ---- one hit per class ----------------------------------------------------------
# hit <case> <class> <value>: the value on line 5 of tally-table.txt (clean()
# writes 2 lines, then 3 more here).
hit() {
    local dir="$scratch/hit-$1"
    clean "$dir"
    printf 'first\nsecond\nboard at %s now\n' "$3" >> "$dir/tally-table.txt"
    run "$dir"
    if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -qx "  hit tally-table.txt:5 ($2; matched text masked)" \
       && printf '%s\n' "$out" | grep -q '^c1 layout: ok' \
       && printf '%s\n' "$out" | grep -qx 'c2 redaction: FAIL 1 line(s) in 1 file(s) match' \
       && [ "$(printf '%s\n' "$out" | tail -1)" = "glmark2-log-check: FAIL 1" ]; then
        ok "$1: exit 1, hit tally-table.txt:5 ($2), FAIL 1"
    else
        bad "$1: rc $rc"; show
    fi
    leak "$1"
}
hit ipv4 ipv4 "$IP"
hit mac mac "$MAC"
hit home home "$HOMEP"
hit ttyusb tty "$TTYUSB"
hit ttyacm tty "$TTYACM"
hit login login "$LOGIN"

# ---- a subdirectory, a binary file, two classes on one line -----------------------
dir="$scratch/deep"
clean "$dir"
mkdir -p "$dir/state/before"
printf 'ok\nconsole on %s from %s\n' "$TTYUSB" "$LOGIN" > "$dir/state/before/dmesg.txt"
printf 'ELF\000\001\002\nmac=%s\n' "$MAC" > "$dir/state/blob.bin"
run "$dir"
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -qx '  hit state/before/dmesg.txt:2 (tty,login; matched text masked)' \
   && printf '%s\n' "$out" | grep -qx '  hit state/blob.bin:2 (mac; matched text masked)' \
   && printf '%s\n' "$out" | grep -qx 'c2 redaction: FAIL 2 line(s) in 2 file(s) match' \
   && [ "$(printf '%s\n' "$out" | tail -1)" = "glmark2-log-check: FAIL 1" ]; then
    ok "subdirectory and binary file: both hits found, two classes on one line named"
else
    bad "subdirectory and binary file: rc $rc"; show
fi
leak deep

# ---- a file whose name matches is named masked ------------------------------------
dir="$scratch/name"
clean "$dir"
printf 'ssh to %s\n' "$IP" > "$dir/session-$IP.log"
run "$dir"
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -qx '  hit session-\[masked\].log:1 (ipv4; matched text masked)'; then
    ok "file name with an address: named with the address masked"
else
    bad "file name with an address: rc $rc"; show
fi
leak name

# ---- both checks fail ------------------------------------------------------------
dir="$scratch/both"
clean "$dir"
rm "$dir/tally-results-row.txt" "$dir/tally-table.txt"
printf 'uart %s\n' "$TTYACM" >> "$dir/state-compare.txt"
run "$dir"
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -qx 'c1 layout: FAIL missing tally-table.txt tally-results-row.txt' \
   && printf '%s\n' "$out" | grep -q '^c2 redaction: FAIL' \
   && [ "$(printf '%s\n' "$out" | tail -1)" = "glmark2-log-check: FAIL 2" ]; then
    ok "c1 and c2 both fail: FAIL 2, exit 1"
else
    bad "c1 and c2 both fail: rc $rc"; show
fi
leak both

# ---- usage ---------------------------------------------------------------------
run
if [ "$rc" -eq 2 ]; then ok "no argument: exit 2"; else bad "no argument: rc $rc"; show; fi
run "$scratch/clean" "$scratch/clean"
if [ "$rc" -eq 2 ]; then ok "two arguments: exit 2"; else bad "two arguments: rc $rc"; show; fi
run "$scratch/none"
if [ "$rc" -eq 2 ]; then ok "not a directory: exit 2"; else bad "not a directory: rc $rc"; show; fi
leak "not a directory"

if [ "$fail" -ne 0 ]; then
    echo "$NAME: $fail check(s) FAILED, $pass passed"
    exit 1
fi
echo "$NAME: $pass checks passed"
