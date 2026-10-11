#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Pins tools/check.sh's test cache (cargo_test). A binary that passed in a
# run with YAKOGL_CHECK_RECORD is skipped by the next run with the same key;
# a new environment key or a changed run-time input runs it again; a
# failing binary turns the row red, is named, and is never recorded. A
# wrong skip hides a failure, so the cache is pinned here and run in the
# gate (scope tools).
set -u
cd "$(dirname "$0")/.." || exit 2
fails=0
checks=0
mkdir -p build/check
cache=$(mktemp -d build/check/testcache-selftest.XXXXXX) || exit 2
probes=()
cleanup() {
  rm -rf "$cache"
  [ "${#probes[@]}" -eq 0 ] || rm -f "${probes[@]}"
}
trap cleanup EXIT
expect() {
  local label=$1 pattern=$2 out=$3
  checks=$((checks + 1))
  printf '%s\n' "$out" | grep -qE "$pattern" || {
    echo "FAIL $label: no line matches '$pattern'"
    printf '%s\n' "$out" | sed 's/^/    /' | tail -15
    fails=$((fails + 1))
  }
}
same() {
  local label=$1 a=$2 b=$3 want=$4
  checks=$((checks + 1))
  if { [ "$want" = same ] && [ "$a" != "$b" ]; } || { [ "$want" = differ ] && [ "$a" = "$b" ]; }; then
    echo "FAIL $label: expected the keys to $want ($a / $b)"
    fails=$((fails + 1))
  fi
}
# The binaries-concurrently arm first (YAKOGL_CHECK_NEXTEST=0); the nextest
# arm below when the host has cargo-nextest.
row() {
  env -u YAKOGL_CHECK_RECORD -u YAKOGL_TEST_ENV_EXTRA YAKOGL_TEST_OK="$cache" YAKOGL_CHECK_NEXTEST=0 "$@" 2>&1 |
    sed 's/\x1b\[[0-9;]*m//g'
}
nextest_row() {
  env -u YAKOGL_CHECK_RECORD -u YAKOGL_TEST_ENV_EXTRA -u YAKOGL_CHECK_NEXTEST YAKOGL_TEST_OK="$cache" "$@" 2>&1 |
    sed 's/\x1b\[[0-9;]*m//g'
}
key() { row tools/check.sh --test-keys | sed -n "s/^$1 //p"; }

# The row records, then skips every binary it recorded.
out=$(row YAKOGL_CHECK_RECORD=deadbeef tools/check.sh --cargo-test -p yakogl-tools)
expect "record: the row is green" '^check.sh: 1 ok, 0 failed' "$out"
n=$(printf '%s\n' "$out" | sed -nE 's/.*test binaries: ([0-9]+) run, 0 skipped.*/\1/p')
checks=$((checks + 1))
entries=$(find "$cache" -maxdepth 1 -type f ! -name '*.log' | wc -l)
if [ -z "$n" ] || [ "$n" -lt 1 ] || [ "$entries" -ne "$n" ]; then
  echo "FAIL record: ${n:-no} binaries run, $entries entries written"
  fails=$((fails + 1))
fi
for e in $(find "$cache" -maxdepth 1 -type f ! -name '*.log'); do
  checks=$((checks + 1))
  [ "$(cat "$e")" = deadbeef ] && [ -s "$e.log" ] ||
    { echo "FAIL record: $e does not hold the commit deadbeef, or has no log"; fails=$((fails + 1)); }
done
out=$(row tools/check.sh --cargo-test -p yakogl-tools)
expect "cached: the row is green" '^check.sh: 1 ok, 0 failed' "$out"
expect "cached: nothing ran" "test binaries: 0 run, ${n:-x} skipped" "$out"
checks=$((checks + 1))
k=$(printf '%s\n' "$out" | grep -cE '^skip   yakogl-tools \(.*\) \(passed on main deadbeef\)$')
[ "$k" = "${n:-x}" ] || { echo "FAIL cached: $k 'passed on main deadbeef' lines for ${n:-?} binaries"; fails=$((fails + 1)); }
# A run without YAKOGL_CHECK_RECORD writes nothing.
before=$(find "$cache" -type f | wc -l)
out=$(row YAKOGL_TEST_ENV_EXTRA=x tools/check.sh --cargo-test -p yakogl-tools)
expect "a new environment key: every binary runs" "test binaries: ${n:-x} run, 0 skipped" "$out"
checks=$((checks + 1))
[ "$(find "$cache" -type f | wc -l)" -eq "$before" ] || { echo "FAIL a run without YAKOGL_CHECK_RECORD wrote an entry"; fails=$((fails + 1)); }
# A failing binary: red, named, its log kept, not recorded.
out=$(row YAKOGL_CHECK_RECORD=deadbeef YAKOGL_TEST_ENV_EXTRA=y tools/check.sh --cargo-test -p yakogl-tools -- --no-such-harness-flag)
expect "failure: the row is red" '^check.sh: 0 ok, 1 failed' "$out"
expect "failure: the binary is named" '^  FAILED binary yakogl-tools \(.*\): exit [0-9]+, log build/check/tests-[0-9]+/logs/1/1\.log$' "$out"
checks=$((checks + 1))
[ "$(find "$cache" -type f | wc -l)" -eq "$before" ] || { echo "FAIL a failing binary was recorded"; fails=$((fails + 1)); }

# The nextest arm: a pass recorded by the other runner is not one for
# nextest (the runner is in the key); nextest records, then skips.
if nv=$(cargo nextest --version 2>/dev/null | head -1) && [ -n "$nv" ]; then
  out=$(nextest_row tools/check.sh --cargo-test -p yakogl-tools)
  expect "nextest: the runner is named" "runner: cargo-nextest " "$out"
  expect "nextest: the other runner's pass is not reused" "test binaries: ${n:-x} run, 0 skipped" "$out"
  out=$(nextest_row YAKOGL_CHECK_RECORD=cafe tools/check.sh --cargo-test -p yakogl-tools)
  expect "nextest: record, green" '^check.sh: 1 ok, 0 failed' "$out"
  out=$(nextest_row tools/check.sh --cargo-test -p yakogl-tools)
  expect "nextest: cached, nothing ran" "test binaries: 0 run, ${n:-x} skipped" "$out"
  expect "nextest: the skip line" '^skip   yakogl-tools \(.*\) \(passed on main cafe\)$' "$out"
else
  echo "check_testcache_selftest: no cargo-nextest on this host; its arm was not checked"
fi

# The run-time inputs: the tree key moves with a file the tests read
# (tools/shaders), never with a document; an integration test source moves
# the library's key only. Untracked probes, removed on exit.
env0=$(key env)
tree0=$(key tree)
lib0=$(key lib-tree)
names=$(key names)
for k in "$env0" "$tree0" "$lib0"; do
  checks=$((checks + 1))
  [[ $k =~ ^[0-9a-f]{64}$ ]] || { echo "FAIL --test-keys printed '$k', not a sha256"; fails=$((fails + 1)); }
done
for v in GLMARK2_CORPUS DEQP_PROGRAMS V3D_QPU_REF; do
  expect "the environment key reads $v" "(^| )$v( |$)" "$names"
done
probes+=(docs/.testcache-selftest-probe)
echo probe > docs/.testcache-selftest-probe
same "a document: the tree key" "$tree0" "$(key tree)" same
same "a document: the library's tree key" "$lib0" "$(key lib-tree)" same
probes+=(tests/testcache_selftest_probe.rs)
echo '// probe' > tests/testcache_selftest_probe.rs
same "an integration test source: the tree key" "$tree0" "$(key tree)" same
same "an integration test source: the library's tree key" "$lib0" "$(key lib-tree)" differ
rm -f tests/testcache_selftest_probe.rs
probes+=(tools/shaders/.testcache-selftest-probe)
echo probe > tools/shaders/.testcache-selftest-probe
same "a shader the tests read: the tree key" "$tree0" "$(key tree)" differ
same "a shader the tests read: the library's tree key" "$lib0" "$(key lib-tree)" differ
same "the environment key does not read the tree" "$env0" "$(key env)" same
echo "check_testcache_selftest: $checks checks run (planned $checks), $fails failed"
[ "$fails" -eq 0 ]
