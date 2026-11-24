#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of tools/gate_prune.sh: a fake target tree under $TMPDIR, every
# mode and every refusal. It deletes only inside that tree.

set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
prune=$here/gate_prune.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/gate_prune_selftest.XXXXXX") || exit 1
trap 'rm -rf -- "$work"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
check() { # name, command...
  local n=$1
  shift
  if "$@"; then ok "$n"; else bad "$n"; fi
}
has() { [[ $1 == *$2* ]]; }
rc_is() { # expected code, command...
  local want=$1 got
  shift
  "$@" >/dev/null 2>&1
  got=$?
  [ "$got" -eq "$want" ]
}

mk() { # fresh fake target at $1
  local t=$1 p
  rm -rf -- "$t"
  mkdir -p "$t"
  : >"$t/CACHEDIR.TAG"
  for p in debug x86_64-unknown-linux-gnu/debug release; do
    mkdir -p "$t/$p/deps" "$t/$p/incremental/crate-abc" "$t/$p/examples" "$t/$p/build" "$t/$p/.fingerprint"
    head -c 100000 /dev/zero >"$t/$p/incremental/crate-abc/dep-graph.bin"
    head -c 50000 /dev/zero >"$t/$p/deps/t_one-0123456789abcdef"
    chmod +x "$t/$p/deps/t_one-0123456789abcdef"
    echo meta >"$t/$p/deps/libfoo-0123.rlib"
    echo meta >"$t/$p/deps/libfoo-0123.rmeta"
    echo dep >"$t/$p/deps/t_one-0123456789abcdef.d"
    echo ex >"$t/$p/examples/demo-0123"
    chmod +x "$t/$p/examples/demo-0123"
    echo keep >"$t/$p/build/out.txt"
    echo keep >"$t/$p/.fingerprint/fp"
    echo arch >"$t/$p/libyakogl_release.a"
  done
}

t=$work/target
outside=$work/outside
mkdir -p "$outside/incremental"
echo precious >"$outside/incremental/file"
echo precious >"$outside/keep"
exe=deps/t_one-0123456789abcdef

# --incremental
mk "$t"
out=$(bash "$prune" "$t" --incremental 2>&1)
rc=$?
check "incremental exits 0" test $rc -eq 0
check "incremental removes debug/incremental" test ! -e "$t/debug/incremental"
check "incremental removes triple incremental" test ! -e "$t/x86_64-unknown-linux-gnu/debug/incremental"
check "incremental keeps test executable" test -f "$t/debug/$exe"
check "incremental keeps CACHEDIR.TAG" test -f "$t/CACHEDIR.TAG"
check "incremental prints removal" has "$out" "removed debug/incremental"
check "incremental prints freed" has "$out" "freed"

# --artefacts
mk "$t"
out=$(bash "$prune" "$t" --artefacts 2>&1)
rc=$?
check "artefacts exits 0" test $rc -eq 0
check "artefacts removes test executable" test ! -e "$t/debug/$exe"
check "artefacts removes triple test executable" test ! -e "$t/x86_64-unknown-linux-gnu/debug/$exe"
check "artefacts removes examples" test ! -e "$t/debug/examples/demo-0123"
check "artefacts keeps rlib and rmeta" test -f "$t/debug/deps/libfoo-0123.rlib" -a -f "$t/debug/deps/libfoo-0123.rmeta"
check "artefacts keeps .d" test -f "$t/debug/$exe.d"
check "artefacts keeps build and fingerprint" test -f "$t/debug/build/out.txt" -a -f "$t/debug/.fingerprint/fp"
check "artefacts keeps release tree" test -f "$t/release/$exe" -a -f "$t/release/libyakogl_release.a"
check "artefacts keeps archives" test -f "$t/debug/libyakogl_release.a"
check "artefacts keeps incremental" test -d "$t/debug/incremental"
check "artefacts prints freed" has "$out" "freed"

# default = both
mk "$t"
bash "$prune" "$t" >/dev/null 2>&1
check "default removes incremental" test ! -e "$t/debug/incremental"
check "default removes test executable" test ! -e "$t/debug/$exe"

# --if-free-below
mk "$t"
out=$(bash "$prune" "$t" --incremental --if-free-below 0 2>&1)
check "if-free-below 0 does nothing" test -d "$t/debug/incremental"
check "if-free-below 0 says so" has "$out" "nothing removed"
bash "$prune" "$t" --incremental --if-free-below 99999999 >/dev/null 2>&1
check "if-free-below huge acts" test ! -e "$t/debug/incremental"
check "if-free-below rejects a non-number" rc_is 2 bash "$prune" "$t" --if-free-below abc

# symlinks are not followed
mk "$t"
rm -rf -- "$t/debug/incremental" "$t/debug/examples" "$t/debug/$exe"
ln -s "$outside/incremental" "$t/debug/incremental"
ln -s "$outside/keep" "$t/debug/deps/t_link-0123"
ln -s "$outside" "$t/debug/examples"
bash "$prune" "$t" >/dev/null 2>&1
check "symlinked content outside survives" test -f "$outside/incremental/file" -a -f "$outside/keep"
check "symlinked incremental is left alone" test -L "$t/debug/incremental" -a -d "$outside/incremental"
check "symlinked examples dir not entered" test -L "$t/debug/examples"

# refusals
mkdir -p "$work/plain/debug/incremental"
check "refuses a dir without CACHEDIR.TAG or .rustc_info.json" rc_is 2 bash "$prune" "$work/plain"
check "refused dir is untouched" test -d "$work/plain/debug/incremental"
mkdir -p "$work/rj/debug/incremental"
echo '{}' >"$work/rj/.rustc_info.json"
bash "$prune" "$work/rj" --incremental >/dev/null 2>&1
check "accepts .rustc_info.json" test ! -e "$work/rj/debug/incremental"
check "refuses a missing path" rc_is 2 bash "$prune" "$work/nonexistent"
check "refuses /" rc_is 2 bash "$prune" /
repo=$(git -C "$here" rev-parse --show-toplevel 2>/dev/null || dirname "$here")
check "refuses the repository root" rc_is 2 bash "$prune" "$repo"
fakehome=$work/home
mkdir -p "$fakehome/debug/incremental"
: >"$fakehome/CACHEDIR.TAG"
: >"$work/CACHEDIR.TAG"
check "refuses \$HOME" rc_is 2 env HOME="$fakehome" bash "$prune" "$fakehome"
check "refuses an ancestor of \$HOME" rc_is 2 env HOME="$fakehome" bash "$prune" "$work"
check "\$HOME is untouched" test -d "$fakehome/debug/incremental"
check "refuses no argument" rc_is 2 bash "$prune"

[ "$fail" -eq 0 ] && echo "gate_prune_selftest: all ok"
exit "$fail"
