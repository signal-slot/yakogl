#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of the compiler selection of run-on-board.sh (cross-cc.sh):
# a compiler on PATH is used as is; without one MALI_DEQP_TOOLCHAIN's bin/
# (from a fake deps-env.sh) is put first on PATH; with neither the selection
# fails naming the deps-env.sh command, and run-on-board.sh itself exits 2
# before the board lock and any ssh (a fake ssh on PATH records its calls).
#
#   tools/mali/panthor/run-on-board-cc-selftest.sh   prints "run-on-board-cc-selftest: N checks passed"
#
# Host only: no board, no network, no real toolchain.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/run-on-board-cc.XXXXXX")
trap 'rm -rf "$T"' EXIT
EXPECT=9
passed=0 failed=0
ok() { # <what> <status of the check>
  if [ "$2" -eq 0 ]; then
    passed=$((passed + 1)); echo "  ok    $1"
  else
    failed=$((failed + 1)); echo "  FAIL  $1"
  fi
}

SYS=/usr/bin:/bin
mkdir -p "$T/pathcc" "$T/tc/bin" "$T/fakebin"
for d in "$T/pathcc/aarch64-linux-gnu-gcc" "$T/tc/bin/aarch64-linux-gnu-gcc"; do
  printf '#!/bin/sh\nexit 0\n' > "$d"; chmod +x "$d"
done
# fake deps-env.sh: --present nanopc_t6 prints MALI_DEQP_TOOLCHAIN=$FAKE_TC (when set)
printf '#!/bin/sh\n[ -n "${FAKE_TC-}" ] && echo "MALI_DEQP_TOOLCHAIN=$FAKE_TC"\nexit 0\n' > "$T/deps-env.sh"
chmod +x "$T/deps-env.sh"
# fake ssh/scp: record the call
for c in ssh scp; do
  printf '#!/bin/sh\necho "%s $*" >> "%s/calls"\nexit 1\n' "$c" "$T" > "$T/fakebin/$c"
  chmod +x "$T/fakebin/$c"
done

sel() { # <PATH> <FAKE_TC>: run the selection, print the resolved gcc
  env -i PATH="$1" FAKE_TC="$2" DEPS_ENV="$T/deps-env.sh" bash -c \
    '. "$1" && panthor_cross_cc "$2" && command -v aarch64-linux-gnu-gcc' _ "$HERE/cross-cc.sh" "$ROOT"
}

# 1: a compiler on PATH wins over the toolchain
out=$(sel "$T/pathcc:$SYS" "$T/tc"); rc=$?
ok "compiler on PATH: selection succeeds" "$rc"
[ "$out" = "$T/pathcc/aarch64-linux-gnu-gcc" ]; ok "compiler on PATH: PATH's compiler is kept" $?

# 2: toolchain only
out=$(sel "$SYS" "$T/tc"); rc=$?
ok "toolchain only: selection succeeds" "$rc"
[ "$out" = "$T/tc/bin/aarch64-linux-gnu-gcc" ]; ok "toolchain only: its bin/ is on PATH" $?

# 3: neither
err=$(env -i PATH="$SYS" FAKE_TC= DEPS_ENV="$T/deps-env.sh" bash -c '. "$1" && panthor_cross_cc "$2"' _ "$HERE/cross-cc.sh" "$ROOT" 2>&1)
rc=$?
[ "$rc" -ne 0 ]; ok "neither: selection fails" $?
grep -q 'tools/cts/deps-env.sh --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN' <<< "$err"; ok "neither: the message names the build command" $?
# a toolchain path without the compiler is as good as none
mkdir -p "$T/empty/bin"
sel "$SYS" "$T/empty" > /dev/null 2>&1; [ $? -ne 0 ]; ok "toolchain without gcc: selection fails" $?

# run-on-board.sh itself, neither case: exit 2, no ssh, no lock
rm -f "$T/calls"
env -i PATH="$T/fakebin:$SYS" HOME="$T" BOARD_HOST=root@fake FAKE_TC= DEPS_ENV="$T/deps-env.sh" \
  bash "$HERE/run-on-board.sh" panthor-hello > "$T/out" 2>&1
rc=$?
[ "$rc" -eq 2 ] && grep -q 'deps-env.sh --build --board nanopc_t6' "$T/out"; ok "run-on-board.sh: exits 2 with the message" $?
[ ! -e "$T/calls" ] && ! grep -q 'holding the board lock' "$T/out"; ok "run-on-board.sh: no ssh and no board lock" $?

if [ "$failed" -ne 0 ] || [ "$passed" -ne "$EXPECT" ]; then
  echo "run-on-board-cc-selftest: $passed passed, $failed failed (expected $EXPECT)"
  exit 1
fi
echo "run-on-board-cc-selftest: $passed checks passed"
