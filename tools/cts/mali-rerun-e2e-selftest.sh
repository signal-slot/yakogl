#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The rerun path end to end, without a board: the table mali-rerun-tree.py
# makes of the Lost cases of two fixture slices (tools/cts/testdata/
# mali-rerun-e2e/) must be accepted by check-gles3-tree.py, planned by
# mali-deqp-run.sh --dry-run --tree --session adhoc, and run by the runner on
# the fake board of tools/cts/testdata/mali-deqp-run/ (used by path), whose
# slice then lists exactly the rerun cases with session=adhoc.
#
#   tools/cts/mali-rerun-e2e-selftest.sh    prints "mali-rerun-e2e-selftest: N checks passed"
#
# Host only. Not in tools/cts/cts-selftest.sh yet; the gate wiring is
#   bash tools/cts/mali-rerun-e2e-selftest.sh
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
RUNFIX=$HERE/testdata/mali-deqp-run
FIX=$HERE/testdata/mali-rerun-e2e
RUN=$HERE/mali-deqp-run.sh
base=${TMPDIR:-$REPO/build}
mkdir -p "$base"
tmp=$(mktemp -d "$base/mali-rerun-e2e-selftest-XXXXXX")
trap 'pkill -f -- "$tmp/" 2>/dev/null; rm -rf "$tmp"' EXIT
passed=0 failed=0
check() { # <what> <command...>
  local what=$1
  shift
  if "$@" > /dev/null 2>&1; then
    passed=$((passed + 1)); echo "  ok    $what"
  else
    failed=$((failed + 1)); echo "  FAIL  $what"
  fi
}

# (a) the table
MP=$FIX/mustpass.txt
python3 -I "$HERE/mali-rerun-tree.py" --mustpass "$MP" --max 2 "$FIX/slice-a" "$FIX/slice-b" > "$tmp/table.txt" 2> "$tmp/tree.err"
check "mali-rerun-tree.py exits 0 on the two slices" test $? -eq 0
# The Lost cases, in mustpass order, from the fixtures themselves.
awk -F'\t' '$2 == "Lost" { print $1 }' "$FIX"/slice-*/results.tsv | sort -u > "$tmp/lost.set"
grep -v '^#' "$MP" | grep -Fxf "$tmp/lost.set" > "$tmp/lost.txt"
grep -v '^#' "$tmp/table.txt" | tr ',' '\n' > "$tmp/table.cases"
check "the table holds exactly the 6 Lost cases, in mustpass order, once each" \
  bash -c "[ \$(wc -l < '$tmp/lost.txt') = 6 ] && cmp '$tmp/lost.txt' '$tmp/table.cases'"
check "no Pass or Fail case is in the table" \
  bash -c "! grep -qE 'info\.(vendor|renderer|version)|shaders\.basic\.pass' '$tmp/table.cases'"
check "the crash and hang cases stand alone in their rows" \
  bash -c "grep -qxF 'dEQP-GLES2.functional.prerequisite.state_reset' '$tmp/table.cases' &&
    grep -qxF 'dEQP-GLES2.functional.flush_finish.finish' '$tmp/table.cases' &&
    ! grep -v '^#' '$tmp/table.txt' | grep -E 'state_reset|flush_finish\.finish' | grep -q ,"
check "four rows, labelled T6RR001..T6RR004" \
  bash -c "[ \"\$(grep '^# image ' '$tmp/table.txt' | sed 's/^# image \\(T6RR[0-9]*\\):.*/\\1/' | paste -sd,)\" = T6RR001,T6RR002,T6RR003,T6RR004 ]"
# check-gles3-tree.py wants every mustpass case matched once, so it reads
# the Lost cases as the mustpass list (the Pass and Fail ones are not rerun).
check "check-gles3-tree.py accepts the table (--max 2) over the Lost cases as the list" \
  python3 -I "$HERE/check-gles3-tree.py" --mustpass "$tmp/lost.txt" --max 2 "$tmp/table.txt"

# (b) the dry run
: > "$tmp/ssh.log"
mkdir -p "$tmp/bin" "$tmp/board-bin"
cp "$RUNFIX"/bin/* "$tmp/bin/"
cp "$RUNFIX"/board-bin/* "$tmp/board-bin/"
chmod +x "$tmp"/bin/* "$tmp"/board-bin/*
env -u BOARD_HOST PATH="$tmp/bin:$PATH" FAKE_LOG="$tmp/ssh.log" FAKE_BOARD="$tmp/dryboard" \
  FAKE_BOARD_BIN="$tmp/board-bin" FAKE_FIXTURES="$RUNFIX" BOARD_LOCK="$tmp/dry.lock" \
  MALI_DEQP_MUSTPASS="$MP" TMPDIR="$tmp" \
  bash "$RUN" --dry-run --tree "$tmp/table.txt" --rows 1-4 --session adhoc > "$tmp/dry.out" 2>&1
check "the dry run exits 0" test $? -eq 0
check "...plans session adhoc, rows 1-4, 4 rows and 6 cases" \
  grep -q 'session adhoc, rows 1-4 of .*: 4 row(s), 6 case(s)' "$tmp/dry.out"
check "...per row: T6RR001 1, T6RR002 2, T6RR003 1, T6RR004 2 case(s)" \
  bash -c "[ \"\$(grep -oE 'row [0-9] T6RR00[0-9]: [0-9]+ case' '$tmp/dry.out' | sed 's/row [0-9] //' | paste -sd,)\" = 'T6RR001: 1 case,T6RR002: 2 case,T6RR003: 1 case,T6RR004: 2 case' ]"
check "...and touches no board: empty ssh log, no lock file" \
  bash -c "[ ! -s '$tmp/ssh.log' ] && [ ! -e '$tmp/dry.lock' ] && [ ! -e '$tmp/dryboard' ]"

# (c) the run on the fake board
g() { git -C "$tmp/repo" -c user.name=selftest -c user.email=selftest -c commit.gpgsign=false "$@"; }
mkdir -p "$tmp/repo" "$tmp/b/board/root" "$tmp/b/build"
git -C "$tmp/repo" init -q
g commit -q --allow-empty -m c1
g branch -M main
c1=$(g rev-parse HEAD)
cp -r "$RUNFIX/fixture-build/." "$tmp/b/build/"
chmod +x "$tmp/b/build/deqp-gles2"
sed -i "s/^commit=.*/commit=$c1/" "$tmp/b/build/build.txt"
printf '[    0.000000] Booting Linux on physical CPU 0x0\n' > "$tmp/b/board/kmsg"
echo 5a1e0000-0000-4000-8000-00000000b007 > "$tmp/b/board/boot_id"
: > "$tmp/b/ssh.log"
env PATH="$tmp/bin:$PATH" BOARD_HOST=root@board.invalid BOARD_LOCK="$tmp/board.lock" \
  FAKE_BOARD="$tmp/b/board" FAKE_BOARD_BIN="$tmp/board-bin" FAKE_LOG="$tmp/b/ssh.log" \
  FAKE_FIXTURES="$RUNFIX" FAKE_SCENARIO="$FIX/scenarios/clean.tsv" \
  MALI_DEQP_MAIN_REPO="$tmp/repo" MALI_DEQP_MAIN_REF=main MALI_DEQP_MUSTPASS="$MP" \
  RAW_DIR="$tmp/b/raw" KMSG_SETTLE_S=0.3 STAGE_MARGIN_KB=0 HOLD_MAX=21600 \
  bash "$RUN" --build "$tmp/b/build" --out "$tmp/slice" --rows 1-4 --tree "$tmp/table.txt" \
  --session adhoc > "$tmp/b/out.txt" 2>&1
check "the runner exits 0 on the rerun table" test $? -eq 0
check "the slice has meta.txt, chunks.tsv, results.tsv and kmsg/T6RR001..4.txt" \
  bash -c "for f in meta.txt chunks.tsv results.tsv kmsg/T6RR001.txt kmsg/T6RR002.txt kmsg/T6RR003.txt kmsg/T6RR004.txt; do [ -f '$tmp/slice'/\$f ] || exit 1; done"
check "meta.txt says session=adhoc" grep -qx 'session=adhoc' "$tmp/slice/meta.txt"
tail -n +2 "$tmp/slice/results.tsv" | cut -f1 | sort > "$tmp/got.cases"
sort "$tmp/lost.txt" > "$tmp/want.cases"
check "results.tsv lists exactly the 6 rerun cases, once each" \
  bash -c "cmp '$tmp/want.cases' '$tmp/got.cases' && [ \$(wc -l < '$tmp/got.cases') = 6 ]"
check "...all Pass (the Lost cases are no longer lost)" \
  bash -c "[ \"\$(tail -n +2 '$tmp/slice/results.tsv' | cut -f2 | sort -u)\" = Pass ]"
check "chunks.tsv: four chunks, all done" \
  bash -c "[ \"\$(awk -F'\t' 'NR > 1 { print \$2, \$3, \$4 }' '$tmp/slice/chunks.tsv' | paste -sd,)\" = 'T6RR001 1 done,T6RR002 2 done,T6RR003 1 done,T6RR004 2 done' ]"

total=$((passed + failed))
if [ "$failed" -ne 0 ]; then
  echo "mali-rerun-e2e-selftest: $failed of $total checks failed" >&2
  cat "$tmp/tree.err" >&2
  tail -15 "$tmp/dry.out" "$tmp/b/out.txt" >&2
  exit 1
fi
echo "mali-rerun-e2e-selftest: $passed checks passed"
