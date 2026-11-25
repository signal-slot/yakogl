#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/imx8mp-glmark2-session-selftest.sh
#
# Self-test of imx8mp-glmark2-session.sh on a synthetic record, host only (no
# board, serial or camera). The synthetic capture is the one of
# imx8mp-glmark2-pipeline-selftest.sh (its generator is taken from that file),
# built under $TMPDIR (exit 2 when unset) as the console.txt of a record, with a
# synthetic llvmpipe reference. Cases: (a) a clean capture goes through every
# step, exit 0; (b) a capture with a dropped row exits 3 at the decode step;
# (c) a record with an unredacted fake address fails at the redaction step;
# (d) --dry-run prints the dumps run command and touches no board (BOARD_UART
# unset); (e) a missing reference fails at the compare step.
set -u
here=$(cd "$(dirname "$0")" && pwd)
base=${TMPDIR:-}
if [ -z "$base" ]; then echo "TMPDIR is not set" >&2; exit 2; fi
work=$(mktemp -d "$base/imx8mp-session.XXXXXX") || exit 2
trap '[ -n "${KEEP:-}" ] || rm -rf "$work"' EXIT
fail=0
session=$here/imx8mp-glmark2-session.sh

awk '/^cat >"\$work\/gen.py"/ { on = 1; next } on && /^PY$/ { exit } on { print }' \
    "$here/imx8mp-glmark2-pipeline-selftest.sh" > "$work/gen.py"
[ -s "$work/gen.py" ] || { echo "FAIL: no generator in the pipeline self-test"; exit 1; }
mkdir "$work/cap"
python3 -I "$work/gen.py" "$here" "$work/cap" || { echo "FAIL generating the synthetic capture"; exit 1; }

label=gm01-build
sha=0123456789abcdef0123456789abcdef01234567
slug=build-use-vbo-false
mkdir "$work/ref"
cp "$work/cap/ref-diff.png" "$work/ref/$slug.png"

# mkrec NAME CAPTURE: a record with console.txt ("HH:MM:SS line"), meta.txt
mkrec() {
    local d=$work/$1/20261125-1200-$label-${sha:0:8}
    mkdir -p "$d"
    tr -d '\r' < "$2" | sed 's/^/00:00:01 /' > "$d/console.txt"
    printf 'commit=%s\nimage_sha256=%s\n' "$sha" "$(printf 'x' | sha256sum | cut -d' ' -f1)" > "$d/meta.txt"
    echo "$d"
}

check() { # name, status
    if [ "$2" = 0 ]; then echo "ok   $1"; else echo "FAIL $1"; fail=$((fail + 1)); fi
}

# (a) clean
rec=$(mkrec a "$work/cap/clean.log")
bash "$session" 1 --record-dir "$rec" --ref-dir "$work/ref" > "$work/a.out" 2> "$work/a.err"
rc=$?
[ "$rc" = 0 ]; check "(a) clean capture exits 0 (got $rc)" $?
[ "$rc" = 0 ] || { sed 's/^/     a.err: /' "$work/a.err"; }
for s in resolve decode "redaction, record-check" compare judge; do
    grep -q "step $s" "$work/a.out"; check "(a) step $s ran" $?
done
grep -q "imx8mp-glmark2-note-fill.py --note" "$work/a.out"; check "(a) prints the note-fill command" $?
[ -s "$rec/$label.sidebyside.png" ] && [ -s "$rec/$label.picdiff.png" ]; check "(a) picdiff and side-by-side written" $?

# (b) dropped row
rec=$(mkrec b "$work/cap/bad.log")
bash "$session" 1 --record-dir "$rec" --ref-dir "$work/ref" > "$work/b.out" 2> "$work/b.err"
rc=$?
[ "$rc" = 3 ]; check "(b) dropped row exits 3 (got $rc)" $?
grep -q "step decode failed" "$work/b.err" && grep -q "re-run, never patch" "$work/b.err"; check "(b) names the decode step, re-run never patch" $?
! grep -q "step redaction\|step compare" "$work/b.out"; check "(b) nothing after decode ran" $?

# (c) unredacted fake address
rec=$(mkrec c "$work/cap/clean.log")
fake=192.0.2.$((70 + 7))
echo "board=$fake" >> "$rec/meta.txt"
bash "$session" 1 --record-dir "$rec" --ref-dir "$work/ref" > "$work/c.out" 2> "$work/c.err"
rc=$?
[ "$rc" != 0 ] && grep -q "step redaction failed" "$work/c.err"; check "(c) address fails at the redaction step (got $rc)" $?
! grep -qF "$fake" "$work/c.out" "$work/c.err"; check "(c) the address is never printed" $?

# (d) dry run, no board
BOARD_UART= bash "$session" 24 --dry-run > "$work/d.out" 2> "$work/d.err"
rc=$?
[ "$rc" = 0 ]; check "(d) dry run exits 0 (got $rc)" $?
grep -q "tools/board.sh imx8mp run .*gm24-shadow" "$work/d.out"; check "(d) prints the dumps run command" $?
! grep -q "step decode" "$work/d.out"; check "(d) nothing else ran" $?

# (e) missing reference
rec=$(mkrec e "$work/cap/clean.log")
bash "$session" 1 --record-dir "$rec" --ref-dir "$work/none" > "$work/e.out" 2> "$work/e.err"
rc=$?
[ "$rc" != 0 ] && grep -q "step compare failed" "$work/e.err"; check "(e) missing reference fails at compare (got $rc)" $?

# (f) unknown scene
bash "$session" 99 --dry-run > /dev/null 2> "$work/f.err"
rc=$?
[ "$rc" = 2 ] && grep -q "step resolve failed" "$work/f.err"; check "(f) unknown scene fails at resolve (got $rc)" $?

if command -v shellcheck > /dev/null; then
    shellcheck -s bash "$session" "$here/imx8mp-glmark2-session-selftest.sh"; check "shellcheck" $?
else
    echo "skip shellcheck (not installed)"
fi

if [ "$fail" = 0 ]; then echo "imx8mp-glmark2-session self-test: all cases passed"; exit 0; fi
echo "imx8mp-glmark2-session self-test: $fail case(s) FAILED" >&2
exit 1
