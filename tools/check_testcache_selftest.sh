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
#
# And the input keys of the other rows (rowkey): a keyed row records, is
# not run again with the same key, runs again when an input changes, and a
# red keyed row is never recorded. Every real row key is then checked
# against a known positive (a change to one input it names moves it) and a
# negative (a document, or the inputs of the other rows, leave it).
set -u
cd "$(dirname "$0")/.." || exit 2
fails=0
checks=0
mkdir -p build/check
cache=$(mktemp -d build/check/testcache-selftest.XXXXXX) || exit 2
probes=()
restore=()
cleanup() {
  local f
  for f in "${restore[@]}"; do cp "$cache/restore/$(echo "$f" | tr / _)" "$f"; done
  rm -rf "$cache"
  [ "${#probes[@]}" -eq 0 ] || rm -f "${probes[@]}"
}
trap cleanup EXIT
trap 'exit 2' INT TERM
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

# nextest_verdict: the verdict per binary from a nextest.log fragment. The
# shapes are nextest's own (LEAK captured from `cargo nextest run` 0.9.148 on
# a test that left a child holding its output: "2 passed (1 leaky)").
nlog=$cache/nextest.log
cat > "$nlog" <<'EOF2'
   Compiling yakogl v0.1.0 (/work/yakogl)
   Compiling yakogl-tools v0.1.0 (/work/tools)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.00s
────────────
 Nextest run ID 00000000-0000-0000-0000-000000000000 with nextest profile: default
        PASS [   0.012s] (   1/8) yakogl gl::ok
        LEAK [   0.214s] (   2/8) yakogl gl::leaky
        SLOW [> 60.000s] (   3/8) yakogl gl::slow
        PASS [  61.000s] (   3/8) yakogl gl::slow
        SKIP [   0.000s] (   4/8) yakogl gl::skipped
        PASS [   0.010s] (   5/8) yakogl-tools t::ok
        FAIL [   0.010s] (   6/8) yakogl-tools t::bad
        PASS [   0.010s] (   7/8) yakogl_extra x::ok
────────────
     Summary [   1.000s] 8 tests run: 6 passed (1 leaky), 1 failed, 0 skipped
EOF2
# check.sh prints its banner first; the verdict's lines are the ones read.
verdict() {
  out=$(tools/check.sh --nextest-verdict "$nlog" "$1" "${2:-1}" 2>&1)
  vrc=$?
  out=$(printf '%s\n' "$out" | grep -E '^ *[A-Z]+ +\[|^no (result line|tests selected)')
}
verdict yakogl
checks=$((checks + 1)); [ "$vrc" = 0 ] || { echo "FAIL verdict: PASS/LEAK/SLOW/SKIP binary is rc $vrc, expected 0"; fails=$((fails + 1)); }
expect "verdict: the binary's own lines, LEAK included" '^ *LEAK .* yakogl gl::leaky$' "$out"
checks=$((checks + 1)); [ "$(printf '%s\n' "$out" | wc -l)" = 5 ] || { echo "FAIL verdict: yakogl has 5 result lines, got: $out"; fails=$((fails + 1)); }
verdict yakogl-tools
checks=$((checks + 1)); [ "$vrc" = 1 ] || { echo "FAIL verdict: a FAIL line is rc $vrc, expected 1"; fails=$((fails + 1)); }
verdict yakogl_extra
checks=$((checks + 1)); [ "$vrc" = 0 ] || { echo "FAIL verdict: a binary whose id is not a prefix of another is rc $vrc, expected 0"; fails=$((fails + 1)); }
# "Compiling yakogl" and the summary name no binary: no result line.
verdict missing
checks=$((checks + 1)); [ "$vrc" = 3 ] || { echo "FAIL verdict: a binary with no line is rc $vrc, expected 3"; fails=$((fails + 1)); }
expect "verdict: the empty log names its cause" '^no result line for missing in nextest.log \(1 tests selected; ' "$out"
# A binary with no test selected (a feature set leaves its file empty) has no
# line and is not a failure; the log says it selected none.
verdict missing 0
checks=$((checks + 1)); [ "$vrc" = 0 ] || { echo "FAIL verdict: a binary with 0 tests selected is rc $vrc, expected 0"; fails=$((fails + 1)); }
expect "verdict: nothing selected is said" '^no tests selected in missing$' "$out"
: > "$nlog"
verdict yakogl
checks=$((checks + 1)); [ "$vrc" = 3 ] || { echo "FAIL verdict: an empty nextest.log is rc $vrc, expected 3"; fails=$((fails + 1)); }
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
rm -f tools/shaders/.testcache-selftest-probe
# The tree key holds what the tests read (TEST_TREE_PATHS), not the whole
# checkout: a file outside it moves neither key, one inside moves both, a
# library source the library's key only (src/test_rules.rs reads src/).
tprobe() {
  probes+=("$1")
  mkdir -p "$(dirname "$1")"
  echo probe > "$1"
  same "$2: the tree key" "$tree0" "$(key tree)" "$3"
  same "$2: the library's tree key" "$lib0" "$(key lib-tree)" "$4"
  rm -f "$1"
}
tprobe driver/zephyr/.testcache-selftest-probe "a driver file no test reads" same same
tprobe tools/cts/.testcache-selftest-probe "a tool no test reads" same same
tprobe src/gl/.testcache-selftest-probe "a library source" same differ
tprobe src/osal/.testcache-selftest-probe "an OSAL header a test compiles" differ differ
tprobe driver/src/v3d/.testcache-selftest-probe "a V3D source a test compiles" differ differ
tprobe tests/fixtures/.testcache-selftest-probe "a fixture" differ differ
tprobe tools/cts/wedge_probe/.testcache-selftest-probe "a wedge probe source" differ differ
had_corpus=0
[ -e build/glmark2-host/shaders ] && had_corpus=1
tprobe build/glmark2-host/shaders/.testcache-selftest-probe "the generated glmark2 corpus (outside git)" differ differ
[ "$had_corpus" = 1 ] || rmdir build/glmark2-host/shaders build/glmark2-host 2>/dev/null
# Every path a literal in the test code names at run time is under the tree
# key, or is listed below with the reason it is not read by a test. A new
# file read in a test that is not under TEST_TREE_PATHS fails here instead
# of leaving the key blind to it. The control: a probe test that reads a
# file outside them is named.
literal_paths() {
  python3 - "$(key paths)" <<'PY'
import re, subprocess, sys
keyed = sys.argv[1].split() + ["build/glmark2-host/shaders"]
# Literals that tests never read, by file: yakogl-tools commands (their
# tests do not run them: strace of the test binary, 2026-10-12) and the
# coverage documents the round trips write under YAKOGL_WRITE_ISA_COVERAGE.
not_read = {"tools/yakogl-tools/src/abilayout.rs", "tools/yakogl-tools/src/glconsts.rs",
            "tools/yakogl-tools/src/csbgen.rs", "src/shaderc/online/powervr/roundtrip.rs",
            "src/shaderc/online/vivante/roundtrip.rs"}
files = subprocess.run(["git", "ls-files", "-co", "--exclude-standard", "--", "src", "tests",
                        "tools/yakogl-tools/src", "tools/precompile/src", "driver/rust/src",
                        ":(glob)driver/samples/zephyr/*/check/src/**"],
                       capture_output=True, text=True, check=True).stdout.split()
lit = re.compile(r'"((?:\.\./)*(?:src|tests|tools|driver|third_party|include|capi|build|docs|zephyr)/[^"\s]*)"')
for f in sorted(x for x in files if x.endswith(".rs") and x not in not_read):
    for n, line in enumerate(open(f, errors="replace"), 1):
        s = line.strip()
        if s.startswith(("//", "*", "/*")):
            continue
        for m in lit.finditer(line):
            before = line[:m.start()]
            if re.search(r"(include_str!|include_bytes!|include!)\(\s*$|#\[path\s*=\s*$", before):
                continue
            p = re.sub(r"^(\.\./)+", "", m.group(1)).split("{")[0].rstrip("/")
            if not any(p == k or p.startswith(k + "/") or k.startswith(p + "/") for k in keyed):
                print("%s:%d %s" % (f, n, m.group(1)))
PY
}
out=$(literal_paths)
checks=$((checks + 1))
[ -z "$out" ] || { echo "FAIL a test names a path outside the tree key:"; printf '%s\n' "$out" | sed 's/^/    /'; fails=$((fails + 1)); }
probes+=(tests/testcache_selftest_probe.rs)
echo 'fn f() { let _ = std::fs::read("driver/zephyr/testcache-probe.txt"); }' > tests/testcache_selftest_probe.rs
expect "the control: a read outside the tree key is named" 'testcache_selftest_probe.rs:1 driver/zephyr/testcache-probe.txt' "$(literal_paths)"
rm -f tests/testcache_selftest_probe.rs
rm -f "${probes[@]}"
probes=()

# A keyed row end to end, on a real key function over a directory: record,
# not run again, run again when a file under the directory is added; a run
# without YAKOGL_CHECK_RECORD and a red row write no entry.
keyed() {
  row "$@" tools/check.sh --keyed-row key_python_row tools/cts -- python3 tools/cts/deqp-uart-tally.py --self-test
}
rows() { find "$cache/rows" -maxdepth 1 -type f ! -name '*.log' 2>/dev/null | wc -l; }
out=$(keyed YAKOGL_CHECK_RECORD=deadbeef)
expect "keyed: recorded, green" '^check.sh: 1 ok, 0 failed, .* 0 not run \(passed on main\)' "$out"
checks=$((checks + 1))
[ "$(rows)" -eq 1 ] || { echo "FAIL keyed: $(rows) row entries after one recorded row"; fails=$((fails + 1)); }
out=$(keyed)
expect "keyed: not run again" '^check.sh: 0 ok, 0 failed, .* 1 not run \(passed on main\)' "$out"
expect "keyed: the skip line" '^skip  keyed   python3 tools/cts/deqp-uart-tally.py --self-test \(passed on main deadbeef\)$' "$out"
probes+=(tools/cts/.testcache-selftest-probe)
echo probe > tools/cts/.testcache-selftest-probe
out=$(keyed)
expect "keyed: an input added, it runs" '^check.sh: 1 ok, 0 failed, .* 0 not run \(passed on main\)' "$out"
checks=$((checks + 1))
[ "$(rows)" -eq 1 ] || { echo "FAIL keyed: a run without YAKOGL_CHECK_RECORD wrote an entry"; fails=$((fails + 1)); }
out=$(row YAKOGL_CHECK_RECORD=deadbeef tools/check.sh --keyed-row key_python_row tools/cts -- false)
expect "keyed: a red row is red" '^check.sh: 0 ok, 1 failed' "$out"
checks=$((checks + 1))
[ "$(rows)" -eq 1 ] || { echo "FAIL keyed: a red row was recorded"; fails=$((fails + 1)); }
rm -f tools/cts/.testcache-selftest-probe

# Every row key of the gate, over its declared inputs (YAKOGL_CHECK_INPUT_KEYS:
# the keys printed, nothing run, the artefacts a key also holds not built).
row_keys() {
  env "$@" YAKOGL_CHECK_INPUT_KEYS=1 YAKOGL_CHECK_PATHS=src/gl/draw.rs tools/check.sh --changed HEAD 2>&1 |
    sed 's/\x1b\[[0-9;]*m//g' | sed -n 's/^key   //p' | sort
}
k0=$(row_keys)
want_rows="tools   am62p netboot TFTP server
tools   am62p netboot UART phases
tools   am62p-gles3-tree: generator self-test and table check
no table in the artefact
no Mesa reference in the artefact
no test knob in the artefact
gen     precompiled table gpu-powervr:
gen     precompiled table gpu-vivante:
gen     precompiled table gpu-powervr,report-es2:
gen     precompiled table gpu-vivante,report-es2:"
[ -n "${PCO_SHC-}" ] && want_rows="$want_rows
gen     tests/reference/blobs.rs"
while IFS= read -r r; do
  checks=$((checks + 1))
  printf '%s\n' "$k0" | grep -qF "$r" || { echo "FAIL row keys: no key for '$r'"; fails=$((fails + 1)); }
done <<< "$want_rows"
checks=$((checks + 1))
if printf '%s\n' "$k0" | grep -q 'not made'; then
  echo "FAIL row keys: a key was not made:"
  printf '%s\n' "$k0" | grep 'not made' | sed 's/^/    /'
  fails=$((fails + 1))
fi
[ -n "${PCO_SHC-}" ] || echo "check_testcache_selftest: no PCO_SHC; the Mesa table's key was not checked"
# moved <label> <regex of the rows whose key must move> <keys after>: every
# other row's key is the same as before.
moved() {
  local label=$1 want=$2 after=$3 name before_k after_k
  while IFS= read -r line; do
    name=${line% *}
    before_k=${line##* }
    after_k=$(printf '%s\n' "$after" | grep -F "$name " | sed 's/.* //')
    checks=$((checks + 1))
    if printf '%s\n' "$name" | grep -qE "$want"; then
      [ "$before_k" != "$after_k" ] || { echo "FAIL $label: the key of '$name' did not move"; fails=$((fails + 1)); }
    else
      [ "$before_k" = "$after_k" ] || { echo "FAIL $label: the key of '$name' moved"; fails=$((fails + 1)); }
    fi
  done <<< "$k0"
}
moved "nothing changed" '^$' "$(row_keys)"
probe() {
  probes+=("$1")
  echo probe > "$1"
  moved "$2" "$3" "$(row_keys)"
  rm -f "$1"
}
probe docs/.testcache-selftest-probe "a document" '^$'
probe tools/shaders/.testcache-selftest-probe "a shader source" '^(gen     precompiled table|gen     tests/reference|no )'
probe tests/fixtures/precompile_testcache_probe.txt "a precompile fixture" '^gen     precompiled table'
probe src/.testcache-selftest-probe "a library source" '^no '
probe third_party/mesa-26.2.1/.testcache-selftest-probe "a C source build.rs compiles" '^no '
probe tools/testdata/am62p-netboot-testcache-probe.uart.ts "a UART capture" 'netboot UART'
# A tracked file each Python row names, changed in place and restored.
change() {
  mkdir -p "$cache/restore"
  cp "$1" "$cache/restore/$(echo "$1" | tr / _)"
  restore+=("$1")
  echo "# probe" >> "$1"
  moved "$2" "$3" "$(row_keys)"
  cp "$cache/restore/$(echo "$1" | tr / _)" "$1"
}
change tools/tftp_oneshot.py "the TFTP server" 'TFTP server'
change tools/cts/am62p-gles3-tree.txt "the GLES3 tree table" 'gles3-tree'
change tests/reference/blobs.rs "the committed Mesa table" '^(gen     tests/reference|no )'
# The environment: the toolchain's variables, the interpreter's.
moved "RUSTFLAGS" '^(gen     precompiled table|no )' "$(row_keys RUSTFLAGS=--cfg=testcache_probe)"
moved "PYTHONPATH" '^tools ' "$(row_keys PYTHONPATH=/nonexistent)"
moved "PCO_DEBUG" '^gen     tests/reference' "$(row_keys PCO_DEBUG=probe)"
echo "check_testcache_selftest: $checks checks run (planned $checks), $fails failed"
[ "$fails" -eq 0 ]
