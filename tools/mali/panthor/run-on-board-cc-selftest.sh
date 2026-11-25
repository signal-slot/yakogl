#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of run-on-board.sh's compiler selection (select_cross_cc):
# a compiler on PATH is left alone, a MALI_DEQP_TOOLCHAIN of the dependency
# layout puts its bin/ first, and with neither the script exits 2 naming the
# build command, before the board lock and before any ssh.
#
#   tools/mali/panthor/run-on-board-cc-selftest.sh   prints "run-on-board-cc-selftest: N checks passed"
#
# Host only, no board, no network, no real toolchain (fake PATH and layout).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SCRIPT=$HERE/run-on-board.sh
W=$(mktemp -d "${TMPDIR:-/tmp}/rob-cc-XXXXXX")
trap 'rm -rf "$W"' EXIT
passed=0 failed=0
ok() { passed=$((passed + 1)); }
bad() { failed=$((failed + 1)); echo "FAIL: $*" >&2; }
# a fake repo root whose deps-env.sh prints $FAKE_DEPS
R=$W/root
mkdir -p "$R/tools/cts" "$R/tools/mali/panthor" "$R/driver/tools/kexec-zephyr" "$W/sys" "$W/pathcc" "$W/tc/bin"
cat > "$R/tools/cts/deps-env.sh" <<'S'
#!/bin/sh
[ "$1" = --present ] && [ "$2" = nanopc_t6 ] || exit 1
[ -n "$FAKE_DEPS" ] && echo "MALI_DEQP_TOOLCHAIN=$FAKE_DEPS"
exit 0
S
chmod +x "$R/tools/cts/deps-env.sh"
for d in pathcc tc/bin; do
  printf '#!/bin/sh\n' > "$W/$d/aarch64-linux-gnu-gcc"
  chmod +x "$W/$d/aarch64-linux-gnu-gcc"
done
# the system PATH: only what the script needs, no compiler
for t in sed sh dirname; do ln -s "$(command -v $t)" "$W/sys/$t"; done
BASH=$(command -v bash)

sel() { # <PATH> <FAKE_DEPS>: function status, then the compiler it resolved
  (cd "$R" && PATH=$1 FAKE_DEPS=$2 RUN_ON_BOARD_FUNCS_ONLY=1 "$BASH" -c \
    ". '$SCRIPT'; select_cross_cc; rc=\$?; echo \"CC=\$(command -v aarch64-linux-gnu-gcc)\"; exit \$rc" 2>&1)
}

out=$(sel "$W/pathcc:$W/sys" "$W/tc"); rc=$?
if [ $rc -eq 0 ] && echo "$out" | grep -qx "CC=$W/pathcc/aarch64-linux-gnu-gcc"; then ok; else bad "compiler on PATH: rc=$rc $out"; fi

out=$(sel "$W/sys" "$W/tc"); rc=$?
if [ $rc -eq 0 ] && echo "$out" | grep -qx "CC=$W/tc/bin/aarch64-linux-gnu-gcc"; then ok; else bad "toolchain only: rc=$rc $out"; fi

out=$(sel "$W/sys" ""); rc=$?
if [ $rc -eq 2 ] && echo "$out" | grep -q 'deps-env.sh --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN'; then ok; else bad "neither: rc=$rc $out"; fi

# the whole script with neither: exit 2, no lock, no ssh
cp "$HERE/board-profile.sh" "$SCRIPT" "$R/tools/mali/panthor/"
echo 'echo LOCK-TAKEN >&2' > "$R/driver/tools/kexec-zephyr/board_lock.sh"
printf '#!/bin/sh\ntouch "%s/ssh-called"\nexit 9\n' "$W" > "$W/sys/ssh"
chmod +x "$W/sys/ssh"
out=$(PATH="$W/sys" FAKE_DEPS= BOARD_HOST=x@y "$BASH" "$R/tools/mali/panthor/run-on-board.sh" bin 2>&1); rc=$?
if [ $rc -eq 2 ] && ! echo "$out" | grep -q LOCK-TAKEN && [ ! -e "$W/ssh-called" ] &&
  echo "$out" | grep -q 'MALI_DEQP_TOOLCHAIN'; then ok; else bad "script, neither: rc=$rc $out"; fi

echo "run-on-board-cc-selftest: $passed checks passed"
[ $failed -eq 0 ]
