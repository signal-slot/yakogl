#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/deqp-es2-programs-selftest.sh
#
# Host-only check of the extract stage of tools/cts/deqp-es2-programs.sh over
# the hand-written run directory in tools/cts/testdata/deqp-es2-programs/run:
# three programs in two log forms (one used by two cases, one refused and used
# by a case of a resumed chunk, one in attribute_location's section form), a
# case with a lone vertex shader only, and a case that never began. index.tsv
# and cases-without-program.txt must equal expected/, the sources must come
# back verbatim, and a second extract must give byte-identical outputs.
# Nothing here is a real dEQP log.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
DATA=$HERE/testdata/deqp-es2-programs
D=$(mktemp -d "${TMPDIR:-.}/deqp-es2-programs.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }

# extract <log>: the extract stage over $D, its output in <log>
extract() { "$HERE/deqp-es2-programs.sh" --stage extract --out "$D" > "$1" 2>&1; }

cp -r "$DATA/run" "$D/run"
# a stale program of an earlier extract is removed
mkdir -p "$D/programs"
: > "$D/programs/0000000000000000.vert"

chk "extract stage exits 0" extract "$D/out1.txt"
chk "index.tsv equals expected" diff -u "$DATA/expected/index.tsv" "$D/index.tsv"
chk "cases-without-program.txt equals expected" \
    diff -u "$DATA/expected/cases-without-program.txt" "$D/cases-without-program.txt"
chk "summary line" grep -qxF 'deqp-es2-programs: 3 unique programs: 2 compiled, 1 refused at 4.2 (HOST-ONLY)' "$D/out1.txt"
chk "coverage line" grep -qxF 'deqp-es2-programs: 5 of 7 cases logged a program; 1 not run, 1 without a program' "$D/out1.txt"
chk "a stale program is removed" test ! -e "$D/programs/0000000000000000.vert"
chk "one .vert and one .frag per program" test "$(ls "$D/programs" | wc -l)" -eq 6

# The ids are sha256(vs + '\0' + fs) of the sources as written, and the
# sources are dEQP's text unescaped (&lt; &amp; &gt; back to < & >).
ids_ok=1
while IFS=$'\t' read -r id _; do
    [ "$id" = id ] && continue
    got=$( { cat "$D/programs/$id.vert"; printf '\0'; cat "$D/programs/$id.frag"; } | sha256sum | cut -c1-16)
    [ "$got" = "$id" ] || ids_ok=0
done < "$D/index.tsv"
chk "every id is sha256(vs NUL fs) of its files" test "$ids_ok" -eq 1
chk "sources unescaped verbatim" grep -qF 'gl_FragColor = vec4(v_x < 0.5 && v_x > 0.25);' "$D"/programs/*.frag
chk "sources keep their final newline" test "$(tail -c1 "$D/programs/$(sed -n 2p "$D/index.tsv" | cut -f1).vert" | od -An -c | tr -d ' ')" = '\n'

cp "$D/index.tsv" "$D/index1.tsv"
cp "$D/cases-without-program.txt" "$D/without1.txt"
(cd "$D/programs" && sha256sum ./* > "$D/programs1.sum")
chk "second extract exits 0" extract "$D/out2.txt"
chk "second index.tsv byte-identical" cmp -s "$D/index1.tsv" "$D/index.tsv"
chk "second cases-without-program.txt byte-identical" cmp -s "$D/without1.txt" "$D/cases-without-program.txt"
chk "second programs/ byte-identical" sh -c "cd '$D/programs' && sha256sum ./* | cmp -s - '$D/programs1.sum'"

chk "extract without a run directory fails" \
    sh -c "! '$HERE/deqp-es2-programs.sh' --stage extract --out '$D/none' > /dev/null 2>&1"
chk "an unknown stage is refused" sh -c "! '$HERE/deqp-es2-programs.sh' --stage nope > /dev/null 2>&1"

echo "deqp-es2-programs-selftest: $pass ok, $fail failed"
[ "$fail" -eq 0 ]
