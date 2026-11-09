#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# End to end, without a board: the slices tools/cts/mali-deqp-run.sh writes
# against its fake board (tools/cts/testdata/mali-deqp-run/, used by path, not
# copied) must satisfy tools/cts/mali-deqp-tally.py. Three runs over a tiny
# tree and mustpass list (tools/cts/testdata/mali-deqp-e2e/: 7 cases in rows
# E1..E3): a clean one with a dEQP Fail, a hang (Lost chunk-hang) and a crash
# (Lost chunk-crash). The slices then go through --check-cover (passes; fails
# with a case removed from results.tsv, with a chunk row cut short and with
# kmsg/resets/ssh_after emptied), --tally (its counts against awk's over
# results.tsv) and --check-doc (a hand-written fixture section passes, a wrong
# count fails).
#
# chunks.tsv's ssh_after is yes|no on every chunk the runner ran, '-' on a row
# it never ran (status lost), as the runner's header and the tally's agree.
#
#   tools/cts/mali-deqp-e2e-selftest.sh    prints "mali-deqp-e2e-selftest: N checks passed"
#
# Host only, about 15 s (the hang waits for a 2 s timeout). Not in
# tools/check.sh yet; the gate row is
#   bash tools/cts/mali-deqp-e2e-selftest.sh
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
RUNFIX=$HERE/testdata/mali-deqp-run
FIX=$HERE/testdata/mali-deqp-e2e
RUN=$HERE/mali-deqp-run.sh
TALLY=$HERE/mali-deqp-tally.py
EXPECT=27
base=${TMPDIR:-$REPO/build}
mkdir -p "$base"
tmp=$(mktemp -d "$base/mali-deqp-e2e-selftest-XXXXXX")
cleanup() {
  pkill -f -- "$tmp/" 2>/dev/null
  if [ "${MALI_DEQP_SELFTEST_KEEP-}" = 1 ]; then echo "mali-deqp-e2e-selftest: kept $tmp"; else rm -rf "$tmp"; fi
}
trap cleanup EXIT
passed=0 failed=0
check() { # <what> <command...>
  local what=$1
  shift
  if "$@" > /dev/null 2>&1; then
    passed=$((passed + 1))
    echo "  ok    $what"
  else
    failed=$((failed + 1))
    echo "  FAIL  $what"
  fi
}

mkdir -p "$tmp/bin" "$tmp/board-bin"
cp "$RUNFIX"/bin/* "$tmp/bin/"
cp "$RUNFIX"/board-bin/* "$tmp/board-bin/"
chmod +x "$tmp"/bin/* "$tmp"/board-bin/*

# The throwaway repository the runner and the tally take "on main" from.
g() { git -C "$tmp/repo" -c user.name=selftest -c user.email=selftest -c commit.gpgsign=false "$@"; }
mkdir -p "$tmp/repo"
git -C "$tmp/repo" init -q
g commit -q --allow-empty -m c1
g branch -M main
c1=$(g rev-parse HEAD)

# run <name> <scenario> <rows> [VAR=value...]: one runner run on a fresh fake
# board into $tmp/slices/t6-gles2-e2e-<name>.
run() {
  local name=$1 scen=$2 rows=$3 d=$tmp/$1
  shift 3
  mkdir -p "$d/board/root" "$d/build" "$tmp/slices"
  cp -r "$RUNFIX/fixture-build/." "$d/build/"
  chmod +x "$d/build/deqp-gles2"
  sed -i "s/^commit=.*/commit=$c1/" "$d/build/build.txt"
  printf '[    0.000000] Booting Linux on physical CPU 0x0\n' > "$d/board/kmsg"
  echo 5a1e0000-0000-4000-8000-00000000b007 > "$d/board/boot_id"
  : > "$d/ssh.log"
  env PATH="$tmp/bin:$PATH" BOARD_HOST=root@board.invalid BOARD_LOCK="$tmp/board.lock" \
    FAKE_BOARD="$d/board" FAKE_BOARD_BIN="$tmp/board-bin" FAKE_LOG="$d/ssh.log" \
    FAKE_FIXTURES="$RUNFIX" FAKE_SCENARIO="$FIX/scenarios/$scen.tsv" \
    MALI_DEQP_MAIN_REPO="$tmp/repo" MALI_DEQP_MAIN_REF=main \
    MALI_DEQP_MUSTPASS="$FIX/mustpass.txt" RAW_DIR="$d/raw" \
    KMSG_SETTLE_S=0.3 STAGE_MARGIN_KB=0 HOLD_MAX=21600 "$@" \
    bash "$RUN" --build "$d/build" --out "$tmp/slices/t6-gles2-e2e-$name" --rows "$rows" \
    --tree "$FIX/tree.txt" --session smoke > "$d/out.txt" 2>&1
  echo $? > "$d/rc"
}

S=$tmp/slices
echo "mali-deqp-e2e-selftest: the runner over E1 (clean, one Fail), E2 (hang), E3 (crash)"
run clean clean 1-1
run hang hang 2-2 CHUNK_LIMIT=2
run crash crash 3-3
for n in clean hang crash; do
  check "runner $n exits 0" bash -c "[ \"\$(cat '$tmp/$n/rc')\" = 0 ]"
done
slice_files() {
  local n
  for n in clean:1 hang:2 crash:3; do
    local d=$S/t6-gles2-e2e-${n%:*} f
    for f in meta.txt chunks.tsv results.tsv "kmsg/E${n#*:}.txt"; do [ -f "$d/$f" ] || return 1; done
  done
}
check "the runner wrote meta.txt, chunks.tsv, results.tsv and kmsg/E<row>.txt for each" slice_files
check "chunks: E1 done, E2 hang, E3 crash" \
  bash -c "[ \"\$(cat '$S'/t6-gles2-e2e-{clean,hang,crash}/chunks.tsv | awk -F'\t' '\$1 ~ /^[0-9]+\$/ { print \$2, \$3, \$4 }' | paste -sd,)\" = 'E1 3 done,E2 2 hang,E3 2 crash' ]"
check "results: the Fail, Lost chunk-hang and Lost chunk-crash are there" \
  bash -c "cat '$S'/*/results.tsv > '$tmp/all.tsv'; grep -qxP 'dEQP-GLES2.info.version\tFail\t.*' '$tmp/all.tsv' &&
    grep -qxP 'dEQP-GLES2.functional.prerequisite.state_reset\tLost\tchunk-hang' '$tmp/all.tsv' &&
    grep -qxP 'dEQP-GLES2.functional.flush_finish.finish\tLost\tchunk-crash' '$tmp/all.tsv'"

# rule_fail <RULE name> <command...>: the command exits 1 and prints RULE <name>
# on stderr, as the tally's rule failures do.
rule_fail() {
  local rule=$1 rc err
  shift
  err=$("$@" 2>&1 > /dev/null)
  rc=$?
  [ "$rc" = 1 ] && grep -q "^RULE $rule: " <<< "$err"
}

# The tally, run in the throwaway repository so that its ancestor rule (git
# merge-base --is-ancestor <commit> main) is the real one.
tally() { (cd "$tmp/repo" && python3 -I "$TALLY" "$@"); }
cover() { tally --check-cover --main-ref main "$FIX/mustpass.txt" "$@"; }
SL=("$S/t6-gles2-e2e-clean" "$S/t6-gles2-e2e-crash" "$S/t6-gles2-e2e-hang")

echo "mali-deqp-e2e-selftest: --check-cover on the slices as the runner wrote them"
check "--check-cover passes on the three slices against the tiny mustpass" cover "${SL[@]}"
check "the meta.txt commit is the full 40-hex commit of the throwaway main" \
  bash -c "grep -qx 'commit=$c1' '${SL[0]}/meta.txt'"
cover "${SL[0]}" > /dev/null 2> "$tmp/one.err"
check "--check-cover fails on one slice alone: RULE cover-missing for the 4 cases of E2 and E3" \
  bash -c "[ \$(grep -c '^RULE cover-missing: ' '$tmp/one.err') = 4 ]"

# A case removed from results.tsv of the crash slice.
cp -r "${SL[1]}" "$tmp/minus-case"
grep -v '^dEQP-GLES2.functional.flush_finish.wait	' "${SL[1]}/results.tsv" > "$tmp/minus-case/results.tsv"
cover "${SL[0]}" "$tmp/minus-case" "${SL[2]}" > "$tmp/minus-case.out" 2> "$tmp/minus-case.err"
check "--check-cover fails when a case is removed from results.tsv" test $? = 1
check "...and names it: RULE cover-missing: dEQP-GLES2.functional.flush_finish.wait" \
  grep -qx 'RULE cover-missing: dEQP-GLES2.functional.flush_finish.wait is in no slice' "$tmp/minus-case.err"
check "...and only that one RULE line" bash -c "[ \$(wc -l < '$tmp/minus-case.err') = 1 ]"

# A case added twice (a duplicated results row).
cp -r "${SL[0]}" "$tmp/dup"
{ cat "${SL[0]}/results.tsv"; sed -n 2p "${SL[0]}/results.tsv"; } > "$tmp/dup/results.tsv"
check "--check-cover fails on a duplicated case (RULE cover-duplicate)" \
  rule_fail cover-duplicate cover "$tmp/dup" "${SL[1]}" "${SL[2]}"

# A chunk row lacking its kmsg/resets/ssh_after (and the rest) fields.
cp -r "${SL[0]}" "$tmp/short"
awk -F'\t' -v OFS='\t' 'NR > 1 { print $1, $2, $3, $4, $5; next } { print }' "${SL[0]}/chunks.tsv" > "$tmp/short/chunks.tsv"
cover "$tmp/short" "${SL[1]}" "${SL[2]}" > /dev/null 2> "$tmp/short.err"
check "--check-cover fails on a chunk row cut after seconds" test $? = 1
check "...with chunk-field errors for kmsg, resets and ssh_after" \
  bash -c "for c in kmsg resets ssh_after; do grep -q \"^RULE chunk-field: .* \$c is empty\" '$tmp/short.err' || exit 1; done"
cp -r "${SL[0]}" "$tmp/empty"
awk -F'\t' -v OFS='\t' 'NR > 1 { $6 = ""; $7 = ""; $8 = "" } { print }' "${SL[0]}/chunks.tsv" > "$tmp/empty/chunks.tsv"
cover "$tmp/empty" "${SL[1]}" "${SL[2]}" > /dev/null 2> "$tmp/empty.err"
check "--check-cover fails on empty kmsg/resets/ssh_after fields" test $? = 1
check "...one chunk-field line for each of the three" \
  bash -c "[ \$(grep -c '^RULE chunk-field: ' '$tmp/empty.err') = 3 ]"

echo "mali-deqp-e2e-selftest: --tally against the slices"
tally --tally "${SL[@]}" > "$tmp/tally.txt"
cat "${SL[@]/%//results.tsv}" | awk -F'\t' '$1 ~ /^dEQP/ { n[$2]++; t++ } END { for (s in n) print s, n[s]; print "TOTAL", t }' | sort > "$tmp/want.status"
awk '$1 ~ /^(Pass|Fail|Lost|TOTAL)$/ && NF == 2 { print $1, $2 }' "$tmp/tally.txt" | sort > "$tmp/got.status"
check "tally: the status counts and the total are awk's over results.tsv (4 Pass, 1 Fail, 2 Lost, 7)" \
  bash -c "diff '$tmp/want.status' '$tmp/got.status' && grep -qx 'Pass 4' '$tmp/got.status' && grep -qx 'TOTAL 7' '$tmp/got.status'"
cat "${SL[@]/%//results.tsv}" | awk -F'\t' '$2 == "Lost" { n[$3]++ } END { for (r in n) print "Lost", r, n[r] }' | sort > "$tmp/want.lost"
awk '$1 == "cause" && $2 == "Lost" { print "Lost", $4, $3 }' "$tmp/tally.txt" | sort > "$tmp/got.lost"
check "tally: each Lost reason counted as in results.tsv (chunk-crash 1, chunk-hang 1)" \
  bash -c "diff '$tmp/want.lost' '$tmp/got.lost' && [ \$(wc -l < '$tmp/got.lost') = 2 ]"
check "tally: the Fail is one cause line of 1 case, host/address/MAC as the runner redacted them" \
  grep -qE '^cause Fail +1  Image comparison failed on <host> at <addr> & <mac>$' "$tmp/tally.txt"
check "tally: the cause counts add up to the non-Pass cases (3)" \
  bash -c "[ \$(awk '\$1 == \"cause\" { s += \$3 } END { print s }' '$tmp/tally.txt') = 3 ]"
chunk_sums() {
  local d
  for d in "${SL[@]}"; do
    [ "$(awk -F'\t' 'NR > 1 { s += $3 } END { print s }' "$d/chunks.tsv")" = "$(($(wc -l < "$d/results.tsv") - 1))" ] || return 1
  done
}
check "chunks.tsv: per slice, the chunks' cases add up to its results rows (the tally splits by it)" chunk_sums

echo "mali-deqp-e2e-selftest: --check-doc"
sed "s/@SHA8@/${c1:0:8}/" "$FIX/doc.md.in" > "$tmp/doc-right.md"
sed 's/^| Pass | 4 |$/| Pass | 5 |/' "$tmp/doc-right.md" > "$tmp/doc-wrong.md"
sed 's/^| Lost | chunk-hang | 1 |$/| Lost | chunk-hang | 2 |/' "$tmp/doc-right.md" > "$tmp/doc-wrong-cause.md"
check "the wrong-count fixtures differ from the right one" \
  bash -c "! cmp -s '$tmp/doc-right.md' '$tmp/doc-wrong.md' && ! cmp -s '$tmp/doc-right.md' '$tmp/doc-wrong-cause.md'"
check "--check-doc passes on the hand-written section for these counts" \
  tally --check-doc "$tmp/doc-right.md" "${SL[@]}"
check "--check-doc fails on a wrong status count (RULE doc-stale)" \
  rule_fail doc-stale tally --check-doc "$tmp/doc-wrong.md" "${SL[@]}"
check "--check-doc fails on a wrong cause count (RULE doc-stale)" \
  rule_fail doc-stale tally --check-doc "$tmp/doc-wrong-cause.md" "${SL[@]}"
tally --tally --markdown "${SL[@]}" > "$tmp/md.out"
check "the hand-written section is byte for byte the tally's own --markdown" \
  bash -c "sed -n '/mali-deqp-tally begin/,/mali-deqp-tally end/p' '$tmp/doc-right.md' | cmp - '$tmp/md.out'"

total=$((passed + failed))
if [ "$failed" -ne 0 ] || [ "$total" -ne "$EXPECT" ]; then
  echo "mali-deqp-e2e-selftest: $failed of $total checks failed (the self-test pins $EXPECT checks)" >&2
  for n in clean hang crash; do echo "--- $n"; tail -12 "$tmp/$n/out.txt"; done >&2
  exit 1
fi
echo "mali-deqp-e2e-selftest: $passed checks passed"
