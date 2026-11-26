#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of run-on-board.sh's cross compiler selection (panthor_cross_cc):
#   1. an aarch64-linux-gnu-gcc on PATH wins, PATH stays as it was;
#   2. with none on PATH, bin/ of MALI_DEQP_TOOLCHAIN (the real
#      tools/cts/deps-env.sh over a fake dependency layout) goes in front;
#   3. with neither, the function fails naming the build command, and
#      run-on-board.sh exits 2 before the board lock and without ssh or scp.
# No board, no network, no real toolchain: the commands run with a PATH of
# symlinks to a few host tools only (so a host cross gcc cannot leak in), and
# the compilers, ssh, scp and cargo are stubs that record their calls.
#
#   tools/mali/panthor/run-on-board-cc-selftest.sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/run-on-board-cc.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
EXPECT=13
n=0
fail=0
check() { # <description> <command...>
  local d=$1
  shift
  n=$((n + 1))
  if "$@"; then echo "ok $n: $d"; else fail=$((fail + 1)); echo "FAIL $n: $d" >&2; fi
}

# The sandbox PATH: run-on-board.sh up to its compiler selection, and
# deps-env.sh (python3) behind it.
sb=$tmp/sandbox
mkdir -p "$sb"
for t in bash sh env sed grep cat dirname mkdir id cut sha256sum python3; do
  p=$(command -v "$t") || { echo "run-on-board-cc-selftest: no $t on the host" >&2; exit 1; }
  ln -s "$p" "$sb/$t"
done
stub() { # <file>: a stub that appends its name and arguments to $FAKE_LOG
  mkdir -p "$(dirname "$1")"
  printf '#!/bin/sh\necho "%s $*" >> "$FAKE_LOG"\nexit 1\n' "${1##*/}" > "$1"
  chmod +x "$1"
}
stub "$tmp/onpath/aarch64-linux-gnu-gcc"
for t in ssh scp cargo flock timeout; do stub "$tmp/fakebin/$t"; done
# A layout with the toolchain (its marker and the gcc driver), and an empty one.
tc=$tmp/layout/debian-trixie-arm64-cross
stub "$tc/bin/aarch64-linux-gnu-g++"
stub "$tc/bin/aarch64-linux-gnu-gcc"
mkdir -p "$tmp/empty"

# sel <PATH dirs in front of the sandbox, or ""> <layout>: source the
# function, run it, and print its status, PATH and the gcc PATH resolves.
sel() {
  env -i PATH="${1:+$1:}$sb" CONNECTED_DEPS_DIR="$2" CCFARM_DEPS_DIR="$2" \
    FAKE_LOG="$tmp/calls.log" RUN_ON_BOARD_FUNCS_ONLY=1 bash -c '
      . "$1/run-on-board.sh"
      panthor_cross_cc "$2"
      echo "rc=$?"
      echo "PATH=$PATH"
      echo "gcc=$(command -v aarch64-linux-gnu-gcc)"' sel "$here" "$repo"
}

# 1. compiler on PATH (the toolchain is in the layout too: PATH still wins)
out=$(sel "$tmp/onpath" "$tmp/layout" 2> "$tmp/err")
check "on PATH: the selection succeeds" grep -qx 'rc=0' <<< "$out"
check "on PATH: PATH is unchanged" grep -qxF "PATH=$tmp/onpath:$sb" <<< "$out"
check "on PATH: PATH's gcc is the one used" grep -qxF "gcc=$tmp/onpath/aarch64-linux-gnu-gcc" <<< "$out"

# 2. toolchain only
out=$(sel "" "$tmp/layout" 2> "$tmp/err")
check "toolchain only: the selection succeeds" grep -qx 'rc=0' <<< "$out"
check "toolchain only: bin/ goes in front of PATH" grep -qxF "PATH=$tc/bin:$sb" <<< "$out"
check "toolchain only: the toolchain's gcc is the one used" grep -qxF "gcc=$tc/bin/aarch64-linux-gnu-gcc" <<< "$out"
check "toolchain only: the choice is printed on stderr" grep -qF "cross compiler $tc/bin/aarch64-linux-gnu-gcc (MALI_DEQP_TOOLCHAIN)" "$tmp/err"

# 3. neither
out=$(sel "" "$tmp/empty" 2> "$tmp/err")
check "neither: the selection fails" grep -qx 'rc=1' <<< "$out"
check "neither: the message names the build command" \
  grep -qF 'tools/cts/deps-env.sh --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN' "$tmp/err"
# ... and run-on-board.sh itself stops before the lock and the board
: > "$tmp/calls.log"
rc=0
env -i PATH="$tmp/fakebin:$sb" CONNECTED_DEPS_DIR="$tmp/empty" CCFARM_DEPS_DIR="$tmp/empty" \
  FAKE_LOG="$tmp/calls.log" BOARD_HOST=root@board.invalid BOARD_LOCK="$tmp/board.lock" \
  bash "$here/run-on-board.sh" panthor-hello > "$tmp/run.out" 2> "$tmp/run.err" || rc=$?
check "neither: run-on-board.sh exits 2" test "$rc" = 2
check "neither: run-on-board.sh names the build command" \
  grep -qF 'tools/cts/deps-env.sh --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN' "$tmp/run.err"
check "neither: no ssh, scp or cargo call" test ! -s "$tmp/calls.log"
check "neither: the board lock is not taken" \
  eval '[ ! -e "$tmp/board.lock" ] && ! grep -q "holding the board lock" "$tmp/run.out" "$tmp/run.err"'

if [ "$fail" -ne 0 ] || [ "$n" -ne "$EXPECT" ]; then
  echo "run-on-board-cc-selftest: $fail of $n checks failed (expected $EXPECT checks)" >&2
  for f in err run.out run.err calls.log; do [ -s "$tmp/$f" ] && { echo "--- $f" >&2; cat "$tmp/$f" >&2; }; done
  exit 1
fi
echo "run-on-board-cc-selftest: $n checks passed"
