#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Selftest of the cross compiler selection of run-on-board.sh (cross-cc.sh).
# No board, no network, no real toolchain: fake gcc, deps-env.sh, ssh, scp.
set -u
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/run-on-board-cc.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
EXPECT=11
n=0
fail=0
ok() { n=$((n + 1)); echo "ok $n: $1"; }
bad() { n=$((n + 1)); fail=$((fail + 1)); echo "FAIL $n: $1" >&2; }
check() { # description, command...
  local d=$1
  shift
  if "$@"; then ok "$d"; else bad "$d"; fi
}

# sandbox PATH: only these tools, so a host cross gcc cannot leak in
sb=$tmp/sandbox
mkdir -p "$sb"
for t in bash sh sed dirname cat grep mkdir env; do
  p=$(command -v "$t") && ln -s "$p" "$sb/$t"
done
# fake ssh/scp record their calls
mkdir -p "$tmp/fakebin"
for t in ssh scp; do
  printf '#!/bin/sh\necho "%s $*" >> "$FAKE_LOG"\nexit 1\n' "$t" > "$tmp/fakebin/$t"
  chmod +x "$tmp/fakebin/$t"
done
# a gcc on PATH
mkdir -p "$tmp/pathgcc"
printf '#!/bin/sh\necho path-gcc\n' > "$tmp/pathgcc/aarch64-linux-gnu-gcc"
chmod +x "$tmp/pathgcc/aarch64-linux-gnu-gcc"
# a toolchain with gcc, one without
mkdir -p "$tmp/tc/bin" "$tmp/tc-empty/bin"
printf '#!/bin/sh\necho toolchain-gcc\n' > "$tmp/tc/bin/aarch64-linux-gnu-gcc"
chmod +x "$tmp/tc/bin/aarch64-linux-gnu-gcc"
# fake deps-env.sh
printf '#!/bin/sh\necho "MALI_DEQP_TOOLCHAIN=$FAKE_TC"\n' > "$tmp/deps-env.sh"
chmod +x "$tmp/deps-env.sh"

: > "$tmp/ssh.log"
# sel <extra PATH dir or ""> <toolchain dir>: run panthor_cross_cc, print resolved gcc
sel() {
  env -i PATH="${1:+$1:}$sb" DEPS_ENV="$tmp/deps-env.sh" FAKE_TC="$2" \
    bash -c ". '$here/cross-cc.sh'; panthor_cross_cc '$repo' || exit 9; command -v aarch64-linux-gnu-gcc"
}

out=$(sel "$tmp/pathgcc" "$tmp/tc" 2> "$tmp/err"); rc=$?
check "compiler on PATH: selection succeeds" test "$rc" = 0
check "compiler on PATH: PATH's gcc is kept" test "$out" = "$tmp/pathgcc/aarch64-linux-gnu-gcc"

out=$(sel "" "$tmp/tc" 2> "$tmp/err"); rc=$?
check "toolchain only: selection succeeds" test "$rc" = 0
check "toolchain only: toolchain bin/ gcc resolved" test "$out" = "$tmp/tc/bin/aarch64-linux-gnu-gcc"

out=$(sel "" "$tmp/none" 2> "$tmp/err"); rc=$?
check "neither: selection fails" test "$rc" = 9
check "neither: message names the build command" \
  grep -q 'tools/cts/deps-env.sh --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN' "$tmp/err"

out=$(sel "" "$tmp/tc-empty" 2> "$tmp/err"); rc=$?
check "toolchain without gcc: selection fails" test "$rc" = 9

# run-on-board.sh itself, neither case
env -i PATH="$tmp/fakebin:$sb" DEPS_ENV="$tmp/deps-env.sh" FAKE_TC="$tmp/none" \
  FAKE_LOG="$tmp/ssh.log" BOARD_HOST=root@board.invalid BOARD_LOCK="$tmp/board.lock" \
  bash "$here/run-on-board.sh" panthor-hello > "$tmp/rob.out" 2> "$tmp/rob.err"
rc=$?
check "run-on-board.sh: exits 2" test "$rc" = 2
check "run-on-board.sh: message names the build command" \
  grep -q 'tools/cts/deps-env.sh --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN' "$tmp/rob.err"
check "run-on-board.sh: no ssh/scp call" test ! -s "$tmp/ssh.log"
check "run-on-board.sh: no board lock taken" \
  test -z "$(grep 'holding the board lock' "$tmp/rob.out" "$tmp/rob.err")"

if [ "$n" != "$EXPECT" ]; then
  echo "run-on-board-cc-selftest: ran $n checks, expected $EXPECT" >&2
  exit 1
fi
if [ "$fail" != 0 ]; then
  echo "run-on-board-cc-selftest: $fail of $n checks failed" >&2
  exit 1
fi
echo "run-on-board-cc-selftest: $n checks passed"
