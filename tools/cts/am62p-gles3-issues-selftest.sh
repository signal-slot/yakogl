#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of am62p-gles3-issues.py on synthetic TSVs, tree and mustpass list.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
d=$(mktemp -d "${TMPDIR:?TMPDIR must be set}/am62p-issues.XXXXXX")
trap 'rm -rf "$d"' EXIT
P=dEQP-GLES3.functional
F=$P.texture.filtering.3d
for i in $(seq 1 20); do echo "$P.a.b.c$i"; done > "$d/must.txt"
for i in 1 2 3; do echo "$F.c$i"; done >> "$d/must.txt"   # 23 cases
printf '# images\nA\nB\n\n' > "$d/tree.txt"
rows() {
  printf 'Fail\t%s.a.b.c1\tm1\tRC1-x\n' "$P"
  printf 'Fail\t%s.a.b.c2\tm2\tRC1-x\n' "$P"
  printf 'NotSupported\t%s.a.b.c4\tm\tNS-z\n' "$P"
  printf 'QualityWarning\t%s.a.b.c5\tq\tQW-w\n' "$P"
}
{ echo '# build abcdef12, run'; rows; printf 'NotSupported\t%s.c1\tm\tNS-z\n' "$F"; } > "$d/clean.tsv"
{ echo '# build abcdef12, run'; rows; printf 'NoResult\t%s.c1\tm\t#1285\n' "$F"; } > "$d/noresult.tsv"
{ echo '# rows 0-1: build abcdef12'; echo '# rows 2-3: build 1234567'; rows; } > "$d/mixed.tsv"
{ echo '# build abcdef12, run'; rows; for i in $(seq 1 25); do printf 'NotSupported\t%s.x%d\tm\tNS-z\n' "$P" "$i"; done; } > "$d/over.tsv"
run() { local t=$1; shift; python3 "$here/am62p-gles3-issues.py" "$@" --tsv "$d/$t" --tree "$d/tree.txt" --mustpass "$d/must.txt"; }
chk() { printf '%s' "$1" | grep -qF -- "$2" || { echo "FAIL: $3"; exit 1; }; }
rc() { local r; set +e; "$@" >/dev/null 2>&1; r=$?; set -e; echo "$r"; }

out=$(run clean.tsv roadmap)
echo "$out"
chk "$out" 'build abcdef12, 2 images' roadmap-commit
chk "$out" '23 mustpass cases 18 Pass / 2 Fail / 2 NotSupported / 1 QualityWarning / 0 NoResult' roadmap-counts

out=$(run clean.tsv issue-1285 --note "$d/note.md")
chk "$out" '1 NotSupported' c-outcome
chk "$out" 'can be closed' c-closable
chk "$out" "$d/note.md" c-note
out=$(run noresult.tsv issue-1285)
chk "$out" 'unfinished' n-unfinished
chk "$out" 'Not closable yet: texture.filtering.3d still has 1 NoResult' n-open
if printf '%s' "$out" | grep -qF 'can be closed'; then echo "FAIL: NoResult closable"; exit 1; fi

out=$(run clean.tsv new-issues)
chk "$out" 'Title: AM62P dEQP-GLES3: RC1-x 2 Fail' title-fail
chk "$out" 'Title: AM62P dEQP-GLES3: QW-w 1 QualityWarning' title-qw
chk "$out" "$P.a.b.c1: m1" body-case
printf '# tracked\nRC1-x\nQW-w\n' > "$d/known.txt"
out=$(run clean.tsv new-issues --known "$d/known.txt")
[ -z "$out" ] || { echo "FAIL: known family printed"; exit 1; }
# a TSV without a family column groups by <group>.<subgroup>
{ echo '# build abcdef12'; printf 'Fail\t%s.a.b.c1\tm1\n' "$P"; } > "$d/nofam.tsv"
out=$(run nofam.tsv new-issues)
chk "$out" 'Title: AM62P dEQP-GLES3: a.b 1 Fail' fallback-family

[ "$(rc python3 "$here/am62p-gles3-issues.py" roadmap --tsv "$d/mixed.tsv" --tree "$d/tree.txt" --mustpass "$d/must.txt")" = 2 ] \
  || { echo "FAIL: mixed builds not refused with 2"; exit 1; }
out=$(run mixed.tsv roadmap --allow-mixed)
chk "$out" 'builds abcdef12, 1234567' mixed-allowed
[ "$(rc python3 "$here/am62p-gles3-issues.py" roadmap --commit abcdef12 --tsv "$d/mixed.tsv" --tree "$d/tree.txt" --mustpass "$d/must.txt")" = 0 ] \
  || { echo "FAIL: --commit not accepted"; exit 1; }
[ "$(rc run over.tsv roadmap)" = 1 ] || { echo "FAIL: counts that do not add up not exit 1"; exit 1; }
echo "am62p-gles3-issues selftest: ok"
