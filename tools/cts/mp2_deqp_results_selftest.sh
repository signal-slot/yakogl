#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_deqp_results_selftest.sh
#
# Host-only check of tools/cts/mp2-deqp-results.py on small synthetic UART logs in
# $TMPDIR: the last status of a repeated case, an excluded name, a group with no
# logs, CR/NUL/time-stamp noise, and the output read back by mp2-p9-diff.py.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-.}/mp2-results-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
fail=0
ok() { echo "ok: $1"; }
bad() { echo "FAIL: $1"; fail=1; }
P='dEQP-GLES2.functional'

printf '%s\n' "$P.color_clear.*" "$P.fbo.*" "$P.clipping.*" > "$T/list"
printf '%s\n' "$P.color_clear.a" "$P.color_clear.b" "$P.color_clear.c" "$P.fbo.x" "$P.fbo.y" "$P.clipping.line.wide" "$P.clipping.line.thin" > "$T/must"
printf '%s\tpending\n' "$P.clipping.line.wide" > "$T/excl"
{
    printf 'deqp: Fail %s.color_clear.a\r\n' "$P"
    printf 'deqp: Pass %s.color_clear.b\r\n' "$P"
    printf '12:00:01 deqp: Pass %s.color_clear.a\r\n' "$P"
    printf '\0\0deqp: Crash %s.fbo.x\r\n' "$P"
    printf 'deqp: NotSupported %s.clipping.line.wide\n' "$P"
    printf 'deqp: QualityWarning %s.clipping.line.thin\n' "$P"
    printf 'deqp: Fail dEQP-GLES2.outside.nothing\n'
} > "$T/s1.log"
printf 'deqp: Pass %s.color_clear.c\ndeqp: Fail %s.color_clear.b\n' "$P" "$P" > "$T/s2.log"

run() { python3 -I "$HERE/mp2-deqp-results.py" --list "$T/list" --exclude "$T/excl" --mustpass "$T/must" "$@"; }
run "$T/s1.log" "$T/s2.log" > "$T/out.tsv" 2> "$T/err" || bad "tool exit"
row() { awk -F'\t' -v i="$1" '$1 == i' "$T/out.tsv"; }
# color_clear: a Pass (last), b Fail (last, from s2), c Pass; fbo: x Crash, y never ran
[ "$(row 0 | cut -f3-8)" = $'3\t2\t1\t0\t0\t0' ] && ok "repeated case keeps its last status" || bad "row 0: $(row 0)"
[ "$(row 0 | cut -f9)" = "$P.color_clear.b" ] && ok "failing name" || bad "failing name"
[ "$(row 1 | cut -f3-9)" = $'1\t0\t0\t0\t1\t1\t'"$P.fbo.x" ] && ok "other status and a mustpass case without a result" || bad "row 1: $(row 1)"
[ "$(row 2 | cut -f3-9)" = $'1\t0\t0\t0\t1\t0\t'"$P.clipping.line.thin" ] && ok "excluded name not counted" || bad "row 2: $(row 2)"
grep -q 'match no group' "$T/err" && ok "stray case reported on stderr" || bad "stray case"

# a group with no logs: begun 0, '-' names
run "$T/s2.log" > "$T/out2.tsv" || bad "tool exit 2"
[ "$(awk -F'\t' '$1 == 1' "$T/out2.tsv")" = $'1\t'"$P.fbo.*"$'\t0\t0\t0\t0\t0\t0\t-' ] && ok "a group with no logs shows begun 0" || bad "empty group"
[ "$(wc -l < "$T/out2.tsv")" -eq 4 ] && ok "header and one row per list entry" || bad "row count"

# noise-only: NULs, CRs and stamps give the same counts as clean text
tr -d '\r\0' < "$T/s1.log" | sed 's/^12:00:01 //' > "$T/clean.log"
run "$T/clean.log" "$T/s2.log" > "$T/out3.tsv" 2>/dev/null
cmp -s "$T/out.tsv" "$T/out3.tsv" && ok "CR/NUL/time-stamp noise dropped" || bad "noise changed the output"

# the output feeds mp2-p9-diff.py
python3 -I "$HERE/mp2-p9-diff.py" --report-only --baseline "$HERE/imx8mp-full-tree-29460f96.results.tsv" \
    --exclude "$HERE/mp2-deqp-gles2-exclude.txt" "$T/out.tsv" > "$T/diff.out" 2>&1
rc=$?
if [ $rc -eq 0 ] && grep -q '^mp2-p9-diff: ' "$T/diff.out"; then ok "mp2-p9-diff.py reads the output"; else bad "mp2-p9-diff rc=$rc: $(tail -2 "$T/diff.out")"; fi

[ $fail -eq 0 ] && echo "mp2_deqp_results_selftest: ok" || echo "mp2_deqp_results_selftest: FAILED"
exit $fail
