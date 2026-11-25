#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of am62p-gles3-report.py on synthetic records made here (mktemp -d in $TMPDIR):
# a clean row, a row with a GPU reset, a stopped row (the 600 s watchdog), a row no record
# ran, a row run again in a second record of the same build, and two records of different
# builds (exit 1). No board, no network; no board address, host name or UART path in them.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/am62p-report.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "am62p-gles3-report-selftest: FAIL: $*" >&2; exit 1; }
sha=abcdef12
F=dEQP-GLES3.functional.texture.filtering.3d
cat >"$tmp/mustpass.txt" <<EOF
$F.formats.rgba8_nearest
$F.formats.rgba8_linear
$F.formats.rgba8_linear_mipmap_linear
$F.sizes.4x8x8_linear_mipmap_linear
$F.sizes.3x7x5_linear_mipmap_linear
$F.wrap.clamp_clamp_clamp
EOF
cat >"$tmp/tree.txt" <<EOF
# a four-row table in the form of tools/cts/am62p-gles3-tree.txt
$F.formats.rgba8_nearest*,$F.formats.rgba8_linear
$F.formats.rgba8_linear_mipmap_linear
$F.sizes.4x8x8_linear_mipmap_linear
$F.sizes.3x7x5_linear_mipmap_linear,$F.wrap.*
EOF

# run REC N STAMP VERDICT RESETS BACK: a run directory of row N; console.txt from stdin
run() {
  local rec=$1 n=$2 st=$3 v=$4 rs=$5 ba=$6 lab d
  lab=img$((n + 1))-$sha
  d=$rec/$st-$lab-0badc0de
  mkdir -p "$d"
  printf 'name=%s\n' "$lab" >"$d/meta.txt"
  printf 'board: am62p\nname: %s\nverdict: board.sh am62p %s rc=0 verdict=%s boot=uboot-mmc row boot_verdict=complete result=legacy:deqp resets=%s record=absent back_after=%ss\n' \
    "$lab" "$lab" "$v" "$rs" "$ba" >"$d/INDEX.txt"
  cat >"$d/console.txt"
}
rec() { # rec NAME: a record directory with tree.txt for the four rows
  local d=$tmp/$1
  mkdir -p "$d"
  grep -v '^#' "$tmp/tree.txt" | awk -v s="$sha" '{ printf "img%d-%s\tbuild/img%d/zephyr/zephyr.bin\t-\t%s\n", NR, s, NR, $0 }' >"$d/tree.txt"
  echo "$d"
}
a=$(rec deqp-gles3-$sha)
# row 0: clean
run "$a" 0 20261125-1000 done 0 212 <<EOF
deqp: case filter '$F.formats.rgba8_nearest*,$F.formats.rgba8_linear'
deqp: Pass $F.formats.rgba8_nearest
deqp: Fail $F.formats.rgba8_linear
deqp: summary: 2 cases begun, 2 with a result, 0 without
deqp: done: 1 passed, 1 failed, 0 not supported, 0 warnings, complete=1
EOF
# row 1: done, but the PowerVR firmware recovered once
run "$a" 1 20261125-1010 done_with_resets 1 333 <<EOF
render: HWR during fence 12 (counter 0 -> 1)
deqp: QualityWarning $F.formats.rgba8_linear_mipmap_linear
deqp: summary: 1 cases begun, 1 with a result, 0 without
deqp: done: 0 passed, 0 failed, 0 not supported, 1 warnings, complete=1
EOF
# row 2: the 600 s watchdog stopped it in the middle of its only case
run "$a" 2 20261125-1020 watchdog_end absent '?' <<EOF
rti_arm: armed
qpa: #beginTestCaseResult $F.sizes.4x8x8_linear_mipmap_linear
RTI feeder lifetime over
EOF
# row 3: no run in this record
python3 -I "$here/am62p-gles3-report.py" --table "$tmp/tree.txt" --mustpass "$tmp/mustpass.txt" --note-section 1-3 "$a" \
  >"$tmp/one.md" 2>"$tmp/one.err" || { cat "$tmp/one.err" >&2; fail "one record: exit $?"; }
line() { grep -E "^\| $1 \|" "$2" || true; }
want() { # want ROW FILE TEXT: the row's table line holds TEXT
  line "$1" "$2" | grep -qF -- "$3" || fail "row $1: no '$3' in: $(line "$1" "$2")"
}
want 0 "$tmp/one.md" "| img1-$sha | $sha | done | 0 | 212 | 2 | 1 | 1 | 0 | 0 | 0 | 0 |"
want 1 "$tmp/one.md" "| done_with_resets | 1 | 333 | 1 | 0 | 0 | 0 | 1 | 0 | 0 |"
want 2 "$tmp/one.md" "| watchdog_end | absent | ? | 0+ | 0 | 0 | 0 | 0 | 0 | 1 |"
want 3 "$tmp/one.md" "| $F.sizes.3x7x5_linear_mipmap_linear | - | not run | - | - | - | 0 | 0 | 0 | 0 | 0 | 2 |"
want all "$tmp/one.md" "| 3 of 4 rows run | $sha | 1 done | 1 |"
grep -qx 'mustpass 6: Pass 1, Fail 1, NotSupported 0, QualityWarning 1, other 0, no result 3.' "$tmp/one.md" \
  || fail "totals line: $(grep '^mustpass' "$tmp/one.md")"
grep -q "^## Outcome on the board, build $sha" "$tmp/one.md" || fail "no note heading"
grep -qF -- "- row 1 (img2-$sha): \`texture.filtering.3d.formats.rgba8_linear_mipmap_linear\`, verdict=done_with_resets, resets=1, QualityWarning, back after 333 s." "$tmp/one.md" \
  || fail "note row 1: $(grep -- '- row 1 ' "$tmp/one.md")"
grep -qF -- "- row 2 (img3-$sha): \`texture.filtering.3d.sizes.4x8x8_linear_mipmap_linear\`, verdict=watchdog_end, resets=absent, no result." "$tmp/one.md" \
  || fail "note row 2: $(grep -- '- row 2 ' "$tmp/one.md")"
grep -qxF -- "- row 3: \`texture.filtering.3d.sizes.3x7x5_linear_mipmap_linear\`, \`texture.filtering.3d.wrap.*\` (2 mustpass cases), verdict=not run, resets=-." "$tmp/one.md" \
  || fail "note row 3: $(grep -- '- row 3 ' "$tmp/one.md")"
grep -q '^Not every row ended clean: 0 of 3 rows ended verdict=done with resets 0; rows 1-3 did not' "$tmp/one.md" \
  || fail "closing sentence: $(tail -1 "$tmp/one.md")"

# a second record of the same build reruns rows 1-3 later: all clean now
b=$(rec rerun)   # no build in the name: it comes from the labels of tree.txt
run "$b" 1 20261126-0900 done 0 301 <<EOF
deqp: Pass $F.formats.rgba8_linear_mipmap_linear
deqp: summary: 1 cases begun, 1 with a result, 0 without
deqp: done: 1 passed, 0 failed, 0 not supported, 0 warnings, complete=1
EOF
run "$b" 2 20261126-0910 done 0 290 <<EOF
deqp: Pass $F.sizes.4x8x8_linear_mipmap_linear
deqp: summary: 1 cases begun, 1 with a result, 0 without
deqp: done: 1 passed, 0 failed, 0 not supported, 0 warnings, complete=1
EOF
run "$b" 3 20261126-0920 done 0 250 <<EOF
deqp: Pass $F.sizes.3x7x5_linear_mipmap_linear
deqp: NotSupported $F.wrap.clamp_clamp_clamp
deqp: Pass $F.wrap.not_in_mustpass
deqp: summary: 3 cases begun, 3 with a result, 0 without
deqp: done: 2 passed, 0 failed, 1 not supported, 0 warnings, complete=1
EOF
python3 -I "$here/am62p-gles3-report.py" --table "$tmp/tree.txt" --mustpass "$tmp/mustpass.txt" --note-section 1-3 "$a" "$b" \
  >"$tmp/two.md" 2>"$tmp/two.err" || { cat "$tmp/two.err" >&2; fail "two records of one build: exit $?"; }
want 1 "$tmp/two.md" "| done | 0 | 301 | 1 | 1 | 0 | 0 | 0 | 0 | 0 |"
want 3 "$tmp/two.md" "| img4-$sha | $sha | done | 0 | 250 | 3 | 1 | 0 | 1 | 0 | 0 | 0 |"
grep -qxF -- "- row 3 (img4-$sha): \`texture.filtering.3d.sizes.3x7x5_linear_mipmap_linear\`, \`texture.filtering.3d.wrap.*\` (2 mustpass cases), verdict=done, resets=0, 1 Pass / 0 Fail / 1 NotSupported / 0 QualityWarning / 0 other / 0 no result, back after 250 s." "$tmp/two.md" \
  || fail "note row 3, two records: $(grep -- '- row 3 ' "$tmp/two.md")"
grep -qF "Rows 1-3 of \`tree.txt\`, numbered from 0:" "$tmp/two.md" || fail "the note names the table"
grep -qx 'Every one of the 3 rows ended verdict=done with resets 0.' "$tmp/two.md" \
  || fail "clean closing sentence: $(tail -1 "$tmp/two.md")"

# two records of different builds: exit 1, naming both
c=$tmp/deqp-gles3-12345678
mkdir -p "$c"
set +e
python3 -I "$here/am62p-gles3-report.py" --table "$tmp/tree.txt" --mustpass "$tmp/mustpass.txt" "$a" "$c" >/dev/null 2>"$tmp/mix.err"
rc=$?
set -e
[ "$rc" = 1 ] || fail "different builds: exit $rc, want 1"
grep -q 'different builds: 12345678 .*; abcdef12 ' "$tmp/mix.err" || fail "different builds: $(cat "$tmp/mix.err")"
# an -n3d1 record of the same commit is another build too
mkdir -p "$tmp/deqp-gles3-$sha-n3d1"
set +e
python3 -I "$here/am62p-gles3-report.py" --table "$tmp/tree.txt" --mustpass "$tmp/mustpass.txt" "$a" "$tmp/deqp-gles3-$sha-n3d1" >/dev/null 2>&1
rc=$?
set -e
[ "$rc" = 1 ] || fail "native-3D arm beside the default: exit $rc, want 1"

if [ "${1:-}" = -v ]; then cat "$tmp/one.md"; echo; cat "$tmp/two.md"; fi
echo "am62p-gles3-report-selftest: ok"
