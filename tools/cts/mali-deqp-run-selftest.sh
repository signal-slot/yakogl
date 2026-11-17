#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Without a board: tools/cts/mali-deqp-run.sh writes refusals/<label>.txt (the
# chunk's stdout and stderr merged in print order) and mali-deqp-tally.py
# --per-case assigns each refusal line to the dEQP case it follows. One run of
# the runner on its fake board (tools/cts/testdata/mali-deqp-run/, by path)
# with a fake deqp-gles2 that prints "Test case" lines on stdout and
# yakogl-mali-refusal lines on stderr, interleaved
# (tools/cts/testdata/mali-deqp/runner-percase/), over row E1 of the e2e tree.
# Checked: the file exists with its .license, holds the lines in print order,
# the stage's .out and .err are unchanged (stdout / stderr only), the dry run
# names the merged stream, and --per-case gives info.version the refusal cause
# and info.vendor errno:-12, while --tally without --per-case is unchanged.
#
#   tools/cts/mali-deqp-run-selftest.sh    prints "mali-deqp-run-selftest: N checks passed"
#
# Host only, a few seconds. Gate row (not in tools/check.sh yet):
#   bash tools/cts/mali-deqp-run-selftest.sh
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
RUNFIX=$HERE/testdata/mali-deqp-run
E2E=$HERE/testdata/mali-deqp-e2e
FIX=$HERE/testdata/mali-deqp/runner-percase
RUN=$HERE/mali-deqp-run.sh
TALLY=$HERE/mali-deqp-tally.py
EXPECT=14
base=${TMPDIR:-$REPO/build}
mkdir -p "$base"
tmp=$(mktemp -d "$base/mali-deqp-run-selftest-XXXXXX")
cleanup() {
  pkill -f -- "$tmp/" 2>/dev/null
  if [ "${MALI_DEQP_SELFTEST_KEEP-}" = 1 ]; then echo "mali-deqp-run-selftest: kept $tmp"; else rm -rf "$tmp"; fi
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

mkdir -p "$tmp/bin" "$tmp/board-bin" "$tmp/board/root" "$tmp/build" "$tmp/slices"
cp "$RUNFIX"/bin/* "$tmp/bin/"
cp "$RUNFIX"/board-bin/* "$tmp/board-bin/"
chmod +x "$tmp"/bin/* "$tmp"/board-bin/*
cp -r "$RUNFIX/fixture-build/." "$tmp/build/"
cp "$FIX/deqp-gles2" "$tmp/build/deqp-gles2"
chmod +x "$tmp/build/deqp-gles2"

g() { git -C "$tmp/repo" -c user.name=selftest -c user.email=selftest -c commit.gpgsign=false "$@"; }
mkdir -p "$tmp/repo"
git -C "$tmp/repo" init -q
g commit -q --allow-empty -m c1
g branch -M main
c1=$(g rev-parse HEAD)
sed -i "s/^commit=.*/commit=$c1/" "$tmp/build/build.txt"
printf '[    0.000000] Booting Linux on physical CPU 0x0\n' > "$tmp/board/kmsg"
echo 5a1e0000-0000-4000-8000-00000000b007 > "$tmp/board/boot_id"
: > "$tmp/ssh.log"

runner_env() {
  env PATH="$tmp/bin:$PATH" BOARD_HOST=root@board.invalid BOARD_LOCK="$tmp/board.lock" \
    FAKE_BOARD="$tmp/board" FAKE_BOARD_BIN="$tmp/board-bin" FAKE_LOG="$tmp/ssh.log" \
    FAKE_FIXTURES="$RUNFIX" FAKE_SCENARIO="$FIX/scenario.tsv" \
    MALI_DEQP_MAIN_REPO="$tmp/repo" MALI_DEQP_MAIN_REF=main \
    MALI_DEQP_MUSTPASS="$E2E/mustpass.txt" RAW_DIR="$tmp/raw" \
    KMSG_SETTLE_S=0.3 STAGE_MARGIN_KB=0 HOLD_MAX=21600 "$@"
}
args=(--build "$tmp/build" --rows 1-1 --tree "$E2E/tree.txt" --session smoke)

echo "mali-deqp-run-selftest: the runner over E1 with interleaved stdout and stderr"
runner_env bash "$RUN" "${args[@]}" --out "$tmp/slices/t6-gles2-percase" > "$tmp/out.txt" 2>&1
echo $? > "$tmp/rc"
S=$tmp/slices/t6-gles2-percase
check "the runner exits 0" bash -c "[ \"\$(cat '$tmp/rc')\" = 0 ]"
check "refusals/E1.txt is written, with its .license" test -s "$S/refusals/E1.txt" -a -f "$S/refusals/E1.txt.license"
cat > "$tmp/want.mrg" << 'EOF'
Test case 'dEQP-GLES2.info.vendor'..
yakogl-mali-refusal: -12 errno
  ResourceError (ResourceError)
Test case 'dEQP-GLES2.info.renderer'..
  Pass (Pass)
Test case 'dEQP-GLES2.info.version'..
yakogl-mali-refusal: -95 NOT_BUILT_BLEND_SHADER: a blend shader other than a constant colour's is not built yet
  Fail (Fail)
EOF
grep -E "^(Test case|yakogl-mali-refusal|  )" "$S/refusals/E1.txt" > "$tmp/got.mrg"
check "refusals/E1.txt holds stdout and stderr lines in print order" cmp "$tmp/want.mrg" "$tmp/got.mrg"
check "refusals/E1.txt holds the library's loaded and exit lines too (stderr)" \
  bash -c "grep -q '^yakogl-mali: loaded' '$S/refusals/E1.txt' && grep -q '^yakogl-mali-exit: ' '$S/refusals/E1.txt'"
raw_out=$(ls "$tmp"/raw/log/E1.r0.out)
raw_err=$(ls "$tmp"/raw/log/E1.r0.err)
check "the raw .out holds stdout only (3 Test case lines, no refusal line)" \
  bash -c "[ \$(grep -c '^Test case' '$raw_out') = 3 ] && ! grep -q yakogl-mali '$raw_out'"
check "the raw .err holds stderr only (loaded, 2 refusals, exit; no Test case line)" \
  bash -c "[ \$(grep -c '^yakogl-mali' '$raw_err') = 4 ] && ! grep -q '^Test case' '$raw_err'"
check "the slice's other files are still written (meta, chunks, results, refusals.tsv, kmsg)" \
  bash -c "cd '$S' && test -f meta.txt -a -f chunks.tsv -a -f results.tsv -a -f refusals.tsv -a -f kmsg/E1.txt"
check "chunks.tsv: E1 done" bash -c "awk -F'\t' '\$2 == \"E1\" && \$4 == \"done\"' '$S/chunks.tsv' | grep -q ."
check "the slice keeps no board name or address" bash -c "! grep -rE 'board\.invalid|198\.51\.100' '$S/refusals'"

echo "mali-deqp-run-selftest: the dry run names the merged stream"
runner_env bash "$RUN" "${args[@]}" --dry-run > "$tmp/dry.txt" 2>&1
check "the dry run echoes the stdbuf probe and the merged .mrg stream" \
  bash -c "grep -q 'stdbuf -oL' '$tmp/dry.txt' && grep -q '\.mrg' '$tmp/dry.txt'"

echo "mali-deqp-run-selftest: mali-deqp-tally.py --per-case over the slice"
tally() { (cd "$tmp/repo" && python3 -I "$TALLY" "$@"); }
tally --tally --per-case "$S" > "$tmp/per.txt" 2> "$tmp/per.err"
check "--per-case: info.version gets the NOT_BUILT_BLEND_SHADER refusal as its cause" \
  grep -qE '^cause Fail +1  refused:NOT_BUILT_BLEND_SHADER' "$tmp/per.txt"
check "--per-case: info.vendor (ResourceError) splits as errno:-12" \
  grep -qE '^cause ResourceError +1  errno:-12$' "$tmp/per.txt"
tally --tally "$S" > "$tmp/plain.txt" 2> /dev/null
check "--tally without --per-case reads no refusals/: its Fail cause is the default one" \
  bash -c "! grep -q 'refused:NOT_BUILT' '$tmp/plain.txt' && grep -qE '^cause Fail ' '$tmp/plain.txt'"
# The test fails when the runner does not write the file.
rm -rf "$tmp/nofile" && cp -r "$S" "$tmp/nofile" && rm -r "$tmp/nofile/refusals"
tally --tally --per-case "$tmp/nofile" > "$tmp/nofile.txt" 2> /dev/null
check "without refusals/E1.txt --per-case finds no refusal (the check above needs the file)" \
  bash -c "! grep -q 'refused:NOT_BUILT' '$tmp/nofile.txt'"

total=$((passed + failed))
if [ "$failed" -ne 0 ] || [ "$total" -ne "$EXPECT" ]; then
  echo "mali-deqp-run-selftest: $failed of $total checks failed (the self-test pins $EXPECT checks)" >&2
  tail -12 "$tmp/out.txt" >&2
  tail -5 "$tmp/per.err" >&2
  exit 1
fi
echo "mali-deqp-run-selftest: $passed checks passed"
